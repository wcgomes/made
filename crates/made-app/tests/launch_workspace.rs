//! Launch tests: explicit workspace argument, terminal cwd, and explorer root.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use made_app::workspace::{
    LaunchPlan, Surfaces, WorkspaceLaunch, WorkspaceSelection, WorkspaceSource,
};
use made_explorer::root_switch::RootGeneration;
use made_explorer::tree::{EntryKind, TreeModel};
use made_platform::{
    host_platform, resolve_user_home, validate_user_home, Platform, PlatformFamily,
};
use made_terminal::{PtyEvent, PtyOptions, PtyProcess};

/// Drain shell output while answering the console host's cursor-position
/// query. Returns `(saw_query, terminal_ended)`.
fn drain_console(pty: &mut PtyProcess, carry: &mut Vec<u8>) -> (bool, bool) {
    let mut saw_query = false;
    while let Some(event) = pty.poll() {
        match event {
            PtyEvent::Data(bytes) => {
                let mut scan = Vec::with_capacity(carry.len() + bytes.len());
                scan.extend_from_slice(carry);
                scan.extend_from_slice(&bytes);
                let requests = scan
                    .windows(4)
                    .filter(|window| *window == b"\x1b[6n")
                    .count();
                for _ in 0..requests {
                    let _ = pty.write(b"\x1b[1;1R");
                }
                saw_query |= requests > 0;
                *carry = scan[scan.len().saturating_sub(3)..].to_vec();
            }
            PtyEvent::Exited(_)
            | PtyEvent::Eof
            | PtyEvent::ReadError(_)
            | PtyEvent::MonitorError(_) => return (saw_query, true),
        }
    }
    (saw_query, false)
}

/// Answer the startup ConPTY query before writing a command. Best effort; a
/// console host that never asks simply proceeds after the deadline.
#[cfg(windows)]
fn settle_console(pty: &mut PtyProcess) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut carry = Vec::new();
    loop {
        let (saw_query, ended) = drain_console(pty, &mut carry);
        if saw_query || ended || Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(not(windows))]
fn settle_console(_pty: &mut PtyProcess) {}

fn fixture(temp: &tempfile::TempDir) -> PathBuf {
    let root = temp.path().join("workspace");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("src").join("main.rs"), "fn main() {}").unwrap();
    fs::write(root.join("README.md"), "# fixture\n").unwrap();
    root
}

#[test]
fn explicit_workspace_argument_produces_one_launch_plan() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let canonical = fs::canonicalize(&root).unwrap();

    let selection = WorkspaceSelection::from_arg(Some(OsStr::new(&root)));
    assert_eq!(selection.requested_path(), Some(root.as_path()));

    let launch = WorkspaceLaunch::from_selection(selection);
    assert_eq!(launch.surfaces(), Surfaces::full());
    assert!(launch.error().is_none());

    let plan = launch.plan().expect("valid workspace launches");
    assert_eq!(plan.workspace.path(), canonical.as_path());
    assert_eq!(plan.workspace.source(), WorkspaceSource::LaunchArgument);
    assert_eq!(plan.explorer_root, canonical);
    assert_eq!(plan.terminal_cwd(), canonical.as_path());
    assert_eq!(launch.terminal_cwd(), Some(canonical.as_path()));
    assert_eq!(launch.explorer_root(), Some(canonical.as_path()));
}

#[test]
fn explorer_tree_root_matches_the_launch_plan() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let canonical = fs::canonicalize(&root).unwrap();
    let plan = LaunchPlan::open(&root, WorkspaceSource::LaunchArgument).unwrap();

    let mut tree = TreeModel::new(&plan.explorer_root, RootGeneration::INITIAL).expect("tree root");
    assert_eq!(tree.root(), canonical.as_path());
    tree.expand_root().unwrap();

    let names: Vec<&str> = tree
        .children(tree.root())
        .unwrap()
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, vec!["docs", "src", "README.md"]);
    let src = tree
        .children(tree.root())
        .unwrap()
        .iter()
        .find(|entry| entry.name == "src")
        .unwrap();
    assert_eq!(src.kind, EntryKind::Directory);
}

#[test]
fn terminal_shell_starts_in_the_workspace_folder() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(&temp);
    let canonical = fs::canonicalize(&root).unwrap();
    let plan = LaunchPlan::open(&root, WorkspaceSource::LaunchArgument).unwrap();

    let platform = host_platform();
    let mut pty = PtyProcess::spawn(
        &platform,
        PtyOptions::new(plan.terminal_cwd())
            .with_channel_capacity(64)
            .with_size(made_platform::PtySize { rows: 24, cols: 80 }),
    )
    .expect("spawn workspace shell");

    let marker = temp.path().join("cwd-marker.txt");
    let command = match platform.family() {
        PlatformFamily::Linux | PlatformFamily::MacOs => {
            format!("pwd > {}\n", quote_posix(&marker))
        }
        PlatformFamily::Windows => format!("cd > \"{}\"\r\n", marker.display()),
    };
    settle_console(&mut pty);
    pty.write(command.as_bytes()).expect("write cwd probe");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut carry = Vec::new();
    while Instant::now() < deadline && !marker.is_file() {
        let _ = drain_console(&mut pty, &mut carry);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(marker.is_file(), "shell did not run the cwd probe");

    let reported = fs::read_to_string(&marker).unwrap();
    let reported = reported.trim();
    let reported = Path::new(reported);
    let reported = fs::canonicalize(reported).unwrap_or_else(|_| reported.to_path_buf());
    assert_eq!(
        reported, canonical,
        "shell did not start in the workspace folder"
    );

    let _ = pty.terminate();
}

#[test]
fn no_workspace_opens_the_platform_home_as_the_default_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let home = fixture(&temp);
    let canonical = fs::canonicalize(&home).unwrap();
    let launch = WorkspaceLaunch::from_selection_with_home(
        WorkspaceSelection::None,
        validate_user_home(&home),
    );

    assert_eq!(launch.surfaces(), Surfaces::full());
    assert_eq!(launch.source(), WorkspaceSource::DefaultHome);
    assert!(launch.error().is_none());
    let plan = launch.plan().expect("home workspace launches");
    assert_eq!(plan.workspace.source(), WorkspaceSource::DefaultHome);
    assert_eq!(plan.explorer_root, canonical);
    assert_eq!(plan.terminal_cwd(), canonical.as_path());
    assert_eq!(launch.explorer_root(), Some(canonical.as_path()));
    assert_eq!(launch.terminal_cwd(), Some(canonical.as_path()));
    assert!(launch.picker().is_none());
}

#[test]
fn an_explicit_workspace_overrides_the_default_home() {
    let temp = tempfile::tempdir().unwrap();
    let home = fixture(&temp);
    let explicit = temp.path().join("explicit");
    fs::create_dir(&explicit).unwrap();
    let canonical = fs::canonicalize(&explicit).unwrap();

    let launch = WorkspaceLaunch::from_selection_with_home(
        WorkspaceSelection::Explicit(explicit.clone()),
        validate_user_home(&home),
    );
    assert_eq!(launch.surfaces(), Surfaces::full());
    assert_eq!(launch.source(), WorkspaceSource::LaunchArgument);
    let plan = launch.plan().expect("explicit workspace launches");
    assert_eq!(plan.workspace.path(), canonical.as_path());
    assert_eq!(plan.workspace.source(), WorkspaceSource::LaunchArgument);
}

#[test]
fn picker_selection_shows_only_the_picker_until_confirmed() {
    let mut launch = WorkspaceLaunch::from_selection(WorkspaceSelection::Picker);
    assert_eq!(launch.surfaces(), Surfaces::Picker);
    assert_eq!(launch.source(), WorkspaceSource::Picker);
    assert!(launch.surfaces().has_picker());
    assert!(!launch.surfaces().has_terminal());
    assert!(!launch.surfaces().has_explorer());
    assert!(launch.plan().is_none());
    assert!(launch.terminal_cwd().is_none());
    assert!(launch.explorer_root().is_none());
    assert!(launch.picker().is_some());

    launch.confirm_picker().expect("confirm current folder");
    assert!(launch.surfaces().has_terminal());
    assert!(launch.surfaces().has_explorer());
    assert_eq!(launch.source(), WorkspaceSource::Picker);
    let plan = launch.plan().expect("confirmed plan");
    assert_eq!(plan.workspace.source(), WorkspaceSource::Picker);
    assert_eq!(plan.explorer_root, plan.terminal_cwd());
}

#[test]
fn a_missing_default_home_falls_back_to_a_usable_terminal_without_a_picker() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing-home");
    let launch = WorkspaceLaunch::from_selection_with_home(
        WorkspaceSelection::None,
        validate_user_home(&missing),
    );

    assert_eq!(launch.surfaces(), Surfaces::terminal_only());
    assert_eq!(launch.source(), WorkspaceSource::DefaultHome);
    assert!(launch.picker().is_none());
    assert!(launch.plan().is_none());
    assert!(launch.terminal_cwd().is_some_and(Path::is_dir));
    assert!(launch.explorer_root().is_none());
    let error = launch.error().expect("missing home error");
    assert_eq!(
        error.kind(),
        made_app::workspace::WorkspaceErrorKind::Missing
    );
    assert!(!error.is_blocking());
    assert!(error.message().contains("missing-home"));
}

#[test]
fn an_unresolvable_home_falls_back_terminal_only() {
    let home = resolve_user_home(PlatformFamily::Linux, None, None);
    let launch = WorkspaceLaunch::from_selection_with_home(WorkspaceSelection::None, home);

    assert_eq!(launch.surfaces(), Surfaces::terminal_only());
    assert_eq!(launch.source(), WorkspaceSource::DefaultHome);
    assert!(launch.picker().is_none());
    assert!(launch.terminal_cwd().is_some_and(Path::is_dir));
    let error = launch.error().expect("unresolved home error");
    assert_eq!(
        error.kind(),
        made_app::workspace::WorkspaceErrorKind::InvalidPath
    );
    let message = error.message();
    assert!(
        message.contains("the workspace path is invalid"),
        "unresolved home should say the workspace path is invalid: {message}"
    );
    assert!(
        message.contains("HOME and USERPROFILE are unset or empty"),
        "unresolved home should name the missing environment: {message}"
    );
}

#[test]
fn a_non_directory_home_falls_back_terminal_only() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("not-a-home");
    fs::write(&file, "content").unwrap();
    let launch = WorkspaceLaunch::from_selection_with_home(
        WorkspaceSelection::None,
        validate_user_home(&file),
    );

    assert_eq!(launch.surfaces(), Surfaces::terminal_only());
    assert_eq!(launch.source(), WorkspaceSource::DefaultHome);
    assert_eq!(
        launch.error().expect("home error").kind(),
        made_app::workspace::WorkspaceErrorKind::NotADirectory
    );
}

#[test]
fn empty_argument_requests_the_default_home() {
    let selection = WorkspaceSelection::from_arg(Some(OsStr::new("")));
    assert_eq!(selection, WorkspaceSelection::None);
    assert_eq!(
        WorkspaceSelection::Picker.requested_path(),
        None,
        "the picker is not a filesystem path"
    );
}

#[test]
fn workspace_source_labels_are_stable_for_diagnostics() {
    assert_eq!(WorkspaceSource::DefaultHome.label(), "default-home");
    assert_eq!(WorkspaceSource::LaunchArgument.label(), "launch-argument");
    assert_eq!(WorkspaceSource::Picker.label(), "picker");
}

// ---- Claude Code launch target and session-only plugin environment ----------

use std::ffi::{OsStr as ClaudeOsStr, OsString};

use made_app::{CompatibleTui, TerminalTarget};

const PLUGIN_DIRS: &str = "CLAUDE_CODE_PLUGIN_DIRS";
const SPOOL: &str = "MADE_CLAUDE_SPOOL";

fn env_of(env: &[(OsString, OsString)], key: &str) -> Option<OsString> {
    env.iter()
        .rev()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
}

#[cfg(unix)]
#[test]
fn shell_and_direct_claude_code_tabs_get_the_composed_environment() {
    let shipped = Path::new("/opt/made/plugins/claude-code-made");
    let spool = Path::new("/run/user/1/made-1/claude/10-abc/tab.jsonl");
    for target in [
        TerminalTarget::Shell,
        TerminalTarget::Tui(CompatibleTui::ClaudeCode),
    ] {
        let launch = target.claude_launch_env(
            Some(shipped),
            Some(ClaudeOsStr::new("/user/one:/user/two")),
            Some(spool),
        );
        assert!(!launch.hooks_unavailable, "{target:?}");
        assert_eq!(
            env_of(&launch.env, PLUGIN_DIRS),
            Some(OsString::from(
                "/user/one:/user/two:/opt/made/plugins/claude-code-made"
            )),
            "user entries keep their order and Made's entry comes last: {target:?}"
        );
        assert_eq!(
            env_of(&launch.env, SPOOL),
            Some(spool.as_os_str().to_owned())
        );
    }
    let bare = TerminalTarget::Shell.claude_launch_env(Some(shipped), None, Some(spool));
    assert_eq!(
        env_of(&bare.env, PLUGIN_DIRS),
        Some(shipped.as_os_str().to_owned())
    );
    let again = TerminalTarget::Shell.claude_launch_env(
        Some(shipped),
        Some(shipped.as_os_str()),
        Some(spool),
    );
    assert_eq!(
        env_of(&again.env, PLUGIN_DIRS),
        Some(shipped.as_os_str().to_owned()),
        "Made's entry is never added twice"
    );
}

#[cfg(unix)]
#[test]
fn other_launches_get_no_claude_environment() {
    let shipped = Path::new("/opt/made/plugins/claude-code-made");
    let spool = Path::new("/tmp/claude/10-abc/tab.jsonl");
    let authority = made_remote::protocol::AuthorityId([7; 16]);
    for target in [
        TerminalTarget::Tui(CompatibleTui::OpenCode),
        TerminalTarget::Tui(CompatibleTui::Grok),
        TerminalTarget::RemoteShell { authority },
        TerminalTarget::RemoteOpenCode { authority },
        TerminalTarget::RemoteGrok { authority },
    ] {
        let launch = target.claude_launch_env(
            Some(shipped),
            Some(ClaudeOsStr::new("/user/one")),
            Some(spool),
        );
        assert!(
            launch.env.is_empty(),
            "{target:?} must not be given {launch:?}"
        );
        assert!(!launch.hooks_unavailable, "{target:?}");
    }
}

#[cfg(unix)]
#[test]
fn an_unsafe_shipped_path_never_sets_an_empty_plugin_list() {
    let spool = Path::new("/tmp/claude/10-abc/tab.jsonl");
    for target in [
        TerminalTarget::Shell,
        TerminalTarget::Tui(CompatibleTui::ClaudeCode),
    ] {
        for shipped in [
            "/opt/a:b/claude-code-made",
            "/opt/a\"b/claude-code-made",
            "relative/claude-code-made",
        ] {
            let launch = target.claude_launch_env(Some(Path::new(shipped)), None, Some(spool));
            assert!(
                env_of(&launch.env, PLUGIN_DIRS).is_none() && launch.env.is_empty(),
                "{target:?} {shipped}: no empty or partial list, no spool binding: {launch:?}"
            );
            assert!(launch.hooks_unavailable, "{target:?} {shipped}");
        }
    }
}

#[test]
fn a_missing_shipped_plugin_adds_no_entry_and_records_hooks_unavailable() {
    let spool = Path::new("/tmp/claude/10-abc/tab.jsonl");
    for target in [
        TerminalTarget::Shell,
        TerminalTarget::Tui(CompatibleTui::ClaudeCode),
    ] {
        let launch =
            target.claude_launch_env(None, Some(ClaudeOsStr::new("/user/one")), Some(spool));
        assert!(
            launch.env.is_empty(),
            "no entry, no spool binding: {launch:?}"
        );
        assert!(launch.hooks_unavailable);
    }
    let shipped = Path::new("/opt/made/plugins/claude-code-made");
    let unwritable = TerminalTarget::Shell.claude_launch_env(Some(shipped), None, None);
    assert!(unwritable.hooks_unavailable, "no spool means no hooks");
    assert!(env_of(&unwritable.env, SPOOL).is_none());
}

#[cfg(unix)]
fn fake_executable(directory: &Path, name: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join(name);
    fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn the_claude_code_target_starts_the_detected_absolute_path_with_no_arguments() {
    let root = tempfile::tempdir().unwrap();
    let claude = fake_executable(root.path(), "claude");
    fake_executable(root.path(), "grok");
    let path = std::env::join_paths([root.path()]).unwrap();

    let options = TerminalTarget::Tui(CompatibleTui::ClaudeCode)
        .pty_options("/tmp", &path)
        .expect("a detected claude resolves");
    assert_eq!(options.shell.as_deref(), Some(claude.as_os_str()));
    assert!(options.args.is_empty(), "Made adds no argument to claude");
    assert!(
        TerminalTarget::Tui(CompatibleTui::ClaudeCode)
            .pty_options("/tmp", ClaudeOsStr::new(""))
            .is_none(),
        "nothing is spawned when claude is not on PATH"
    );
    let grok = TerminalTarget::Tui(CompatibleTui::Grok)
        .pty_options("/tmp", &path)
        .unwrap();
    assert_eq!(grok.args, vec![OsString::from("dashboard")]);
    assert_eq!(
        TerminalTarget::Tui(CompatibleTui::ClaudeCode).program_label(),
        "Claude Code"
    );
}

/// Re-run `name` in a child process with an isolated home and a fake `claude`,
/// `grok` and `opencode` first on PATH, and with the shipped plugin staged
/// beside the test executable. Returns `true` in the parent.
#[cfg(unix)]
fn isolated_claude_launch(name: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if std::env::var_os("MADE_CLAUDE_LAUNCH_CHILD").is_some() {
        return false;
    }
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join("bin");
    fs::create_dir(&bin).unwrap();
    for program in ["claude", "grok", "opencode"] {
        let path = bin.join(program);
        fs::write(&path, "#!/bin/sh\nsleep 30\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let tmp = home.path().join("tmp");
    let runtime = home.path().join("runtime");
    let roaming = home.path().join("AppData").join("Roaming");
    let local = home.path().join("AppData").join("Local");
    for directory in [&tmp, &runtime, &roaming, &local] {
        fs::create_dir_all(directory).unwrap();
    }
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    let shipped = made_platform::install_root_from_current_executable(PlatformFamily::current())
        .expect("test executable install root")
        .join("plugins")
        .join("claude-code-made");
    // The plugin counts as present only with its hook binary in place.
    fs::create_dir_all(shipped.join("bin")).unwrap();
    let hook = shipped.join("bin").join("made-claude-status");
    fs::write(&hook, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(bin).chain(std::env::split_paths(&path));
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("MADE_CLAUDE_LAUNCH_CHILD", "1")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env("TMPDIR", &tmp)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("APPDATA", &roaming)
        .env("LOCALAPPDATA", &local)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("SHELL", "/bin/bash")
        .env(PLUGIN_DIRS, "/user/one:/user/two")
        .env(SPOOL, "/inherited/other-tab/claude/1-x/tab.jsonl")
        .env_remove(made_bridge::config::CONFIG_CONTENT_ENV)
        .env_remove(made_platform::SHELL_ENV)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

#[cfg(unix)]
#[test]
fn real_workbenches_carry_the_plugin_environment_only_for_shell_and_claude_code() {
    if isolated_claude_launch(
        "real_workbenches_carry_the_plugin_environment_only_for_shell_and_claude_code",
    ) {
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    let home = host_platform().home_dir().expect("test home");
    let shipped = made_platform::install_root_from_current_executable(PlatformFamily::current())
        .unwrap()
        .join("plugins")
        .join("claude-code-made");
    let expected = format!("/user/one:/user/two:{}", shipped.display());
    let start =
        |target| made_app::Workbench::start(None, None, home.clone(), target, Instant::now());

    let mut claude = start(TerminalTarget::Tui(CompatibleTui::ClaudeCode));
    let options = claude.terminal().expect("claude child").options().clone();
    let program = options.shell.clone().expect("explicit program");
    assert!(
        Path::new(&program).is_absolute() && Path::new(&program).ends_with("claude"),
        "{program:?}"
    );
    assert!(options.args.is_empty());
    assert_eq!(
        env_of(&options.env, PLUGIN_DIRS),
        Some(OsString::from(&expected))
    );
    assert!(
        env_of(&options.env, made_bridge::config::CONFIG_CONTENT_ENV).is_none(),
        "OpenCode's plugin environment is not added to the Claude launch"
    );
    let spool = PathBuf::from(env_of(&options.env, SPOOL).expect("per-tab spool binding"));
    assert!(spool.is_absolute() && spool.is_file(), "{spool:?}");
    assert_eq!(
        fs::metadata(&spool).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(spool.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    let mut shell = start(TerminalTarget::Shell);
    let shell_options = shell.terminal().expect("shell").options().clone();
    assert_eq!(
        env_of(&shell_options.env, PLUGIN_DIRS),
        Some(OsString::from(&expected))
    );
    let shell_spool = PathBuf::from(env_of(&shell_options.env, SPOOL).unwrap());
    assert!(
        !shell_spool.starts_with("/inherited"),
        "the tab binds its own spool over an inherited one"
    );
    assert_ne!(shell_spool, spool, "each tab has its own spool");

    for target in [
        TerminalTarget::Tui(CompatibleTui::Grok),
        TerminalTarget::Tui(CompatibleTui::OpenCode),
    ] {
        let mut other = start(target);
        let options = other.terminal().expect("child").options().clone();
        assert!(env_of(&options.env, PLUGIN_DIRS).is_none(), "{target:?}");
        assert!(env_of(&options.env, SPOOL).is_none(), "{target:?}");
        assert!(
            options.env_remove.iter().any(|key| key == SPOOL),
            "an inherited spool binding is stripped from {target:?}"
        );
        if target == TerminalTarget::Tui(CompatibleTui::Grok) {
            assert_eq!(options.args, vec![OsString::from("dashboard")]);
            assert!(env_of(&options.env, made_bridge::config::CONFIG_CONTENT_ENV).is_none());
        }
        other.shutdown();
    }

    claude.shutdown();
    drop(claude);
    assert!(!spool.exists(), "closing the tab deletes its spool");
    assert!(shell_spool.exists(), "other tabs' spools are unchanged");
    shell.shutdown();
    drop(shell);
    assert!(!shell_spool.exists());
}

#[cfg(unix)]
#[test]
fn spool_records_of_the_tab_move_its_explorer_and_foreign_ones_do_not() {
    if isolated_claude_launch("spool_records_of_the_tab_move_its_explorer_and_foreign_ones_do_not")
    {
        return;
    }
    use std::io::Write;
    use std::time::Instant;

    let workspace = tempfile::tempdir().unwrap();
    let mk = |name: &str| {
        let path = workspace.path().join(name);
        fs::create_dir_all(&path).unwrap();
        path.canonicalize().unwrap()
    };
    let (root, session, foreign, worktree) =
        (mk("root"), mk("session"), mk("foreign"), mk("worktree"));
    let plan = LaunchPlan::open(&root, WorkspaceSource::Picker).unwrap();
    let mut workbench = made_app::Workbench::start(
        Some(plan),
        None,
        root.clone(),
        TerminalTarget::Tui(CompatibleTui::ClaudeCode),
        Instant::now(),
    );
    let pane_pid = workbench
        .terminal()
        .and_then(made_app::TerminalPane::pid)
        .unwrap();
    let spool = PathBuf::from(
        env_of(&workbench.terminal().unwrap().options().env, SPOOL).expect("spool binding"),
    );
    let ctx = eframe::egui::Context::default();
    let append = |cwd: &Path, event: &str, pids: &[u32]| {
        let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
        let line = format!(
            "{{\"at\":\"t\",\"sessionId\":\"s\",\"cwd\":{},\"event\":\"{event}\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[{}]}}\n",
            serde_json::to_string(&cwd.display().to_string()).unwrap(),
            pids.join(",")
        );
        let mut file = fs::OpenOptions::new().append(true).open(&spool).unwrap();
        file.write_all(line.as_bytes()).unwrap();
    };
    let tick_until = |workbench: &mut made_app::Workbench, expected: &Path| {
        let deadline = Instant::now() + Duration::from_secs(3);
        while workbench.explorer_root() != Some(expected) && Instant::now() < deadline {
            workbench.tick(&ctx);
            std::thread::sleep(Duration::from_millis(20));
        }
        workbench.explorer_root().map(Path::to_path_buf)
    };

    assert!(
        workbench.claude_tab().unwrap().present(),
        "a direct tab is Claude Code while its child runs"
    );
    // A record whose ancestry never reaches this tab's terminal is not owned.
    append(&foreign, "SessionStart", &[4_000_000, 4_000_001]);
    for _ in 0..30 {
        workbench.tick(&ctx);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(workbench.explorer_root(), Some(root.as_path()));
    assert!(workbench.claude_tab().unwrap().dots().is_empty());

    // The hook shell (already exited) then the terminal child: owned.
    append(&session, "SessionStart", &[4_000_000, pane_pid]);
    assert_eq!(
        tick_until(&mut workbench, &session).as_deref(),
        Some(session.as_path()),
        "an owned record moves the explorer"
    );
    append(&worktree, "CwdChanged", &[4_000_000, pane_pid]);
    assert_eq!(
        tick_until(&mut workbench, &worktree).as_deref(),
        Some(worktree.as_path()),
        "a later record of any event follows"
    );
    append(&root.join("missing"), "Stop", &[4_000_000, pane_pid]);
    for _ in 0..30 {
        workbench.tick(&ctx);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        workbench.explorer_root(),
        Some(worktree.as_path()),
        "a missing directory does not move the explorer"
    );
    workbench.shutdown();
}

fn quote_posix(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("'{}'", text.replace('\'', "'\\''"))
}
