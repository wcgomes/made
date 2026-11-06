//! Launch-environment and log-order matrix for every terminal target, written
//! against the code before the integration-module refactor
//! (agent-integration-modules, phase 0).
//!
//! Each scenario runs a real `Workbench` in an isolated child process with fake
//! `opencode`, `grok` and `claude` programs first on `PATH`, an isolated home,
//! and an install root of its own (a hard link of this test executable) so the
//! shipped Claude Code plugin is exactly present or absent. The transcript of
//! one target records the program, arguments, variables set (in order),
//! variables removed (in order) and the diagnostics log lines (in order), with
//! the run-specific parts replaced by placeholders. The expected transcripts
//! below do not change during the refactor; only the adapter block does.
#![cfg(unix)]

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Instant;

use made_app::Workbench;
use made_platform::Platform;
use made_remote::protocol::AuthorityId;

// BEGIN characterization adapter
use made_app::{CompatibleTui, TerminalTarget};

/// Local targets in the order they are exercised.
fn local_targets() -> Vec<(&'static str, TerminalTarget)> {
    vec![
        ("shell", TerminalTarget::Shell),
        ("opencode", TerminalTarget::Tui(CompatibleTui::OpenCode)),
        ("grok", TerminalTarget::Tui(CompatibleTui::Grok)),
        (
            "claude-code",
            TerminalTarget::Tui(CompatibleTui::ClaudeCode),
        ),
    ]
}

/// Every remote target for one saved server.
fn remote_targets(authority: AuthorityId) -> Vec<(&'static str, TerminalTarget)> {
    vec![
        ("remote-shell", TerminalTarget::RemoteShell { authority }),
        (
            "remote-opencode",
            TerminalTarget::RemoteOpenCode { authority },
        ),
        ("remote-grok", TerminalTarget::RemoteGrok { authority }),
        ("remote-claude", TerminalTarget::RemoteClaude { authority }),
    ]
}

/// The target a `+` menu launch of the Grok integration starts.
fn grok_target() -> TerminalTarget {
    TerminalTarget::Tui(CompatibleTui::Grok)
}

fn shell_target() -> TerminalTarget {
    TerminalTarget::Shell
}

const PLUGIN_DIRS: &str = "CLAUDE_CODE_PLUGIN_DIRS";
const SPOOL: &str = "MADE_CLAUDE_SPOOL";

/// Fake program names placed first on `PATH`.
const PROGRAMS: [&str; 3] = ["claude", "grok", "opencode"];

/// The program a user types in a shell tab to start the Grok Build integration.
const TYPED_PROGRAM: &str = "grok";

/// Write the integration files of the install root: the OpenCode plugin
/// package, and with `claude_plugin` the shipped Claude Code plugin with its
/// hook binary.
fn stage_integration_files(install: &Path, claude_plugin: bool) {
    let opencode_plugin = opencode_plugin_dir(install);
    fs::create_dir_all(&opencode_plugin).unwrap();
    let repository = made_platform::repository_root()
        .join("plugins")
        .join("opencode-made");
    for file in made_bridge::config::PLUGIN_REQUIRED_FILES {
        fs::copy(repository.join(file), opencode_plugin.join(file)).unwrap();
    }
    if claude_plugin {
        let hook_dir = install.join("plugins").join("claude-code-made").join("bin");
        fs::create_dir_all(&hook_dir).unwrap();
        make_executable(&hook_dir.join("made-claude-status"), "#!/bin/sh\nexit 0\n");
    }
}

/// The OpenCode plugin package the child is pointed at.
fn opencode_plugin_dir(install: &Path) -> PathBuf {
    install.join("plugins").join("opencode-made")
}

/// Variables the parent process exports and every tab must not inherit:
/// another tab's plugin list and spool.
fn inherited_variables() -> [(&'static str, &'static str); 2] {
    [
        (PLUGIN_DIRS, "/user/one:/user/two"),
        (SPOOL, "/inherited/other-tab/claude/1-x/tab.jsonl"),
    ]
}

/// Make the per-tab spool impossible to create under `temp_dir`.
fn block_spool_root(temp_dir: &Path) {
    fs::create_dir_all(temp_dir).unwrap();
    fs::write(temp_dir.join("claude"), "blocked").unwrap();
}

/// The spool path a started tab bound, if any.
fn bound_spool(env: &[(OsString, OsString)]) -> Option<String> {
    env_value(env, SPOOL)
}
// END characterization adapter

const CHILD_ENV: &str = "MADE_LAUNCH_ENV_CHILD";
const LAYOUT_ENV: &str = "MADE_LAUNCH_ENV_LAYOUT";
const BIN_ENV: &str = "MADE_LAUNCH_ENV_BIN";
const ROOT_ENV: &str = "MADE_LAUNCH_ENV_ROOT";

/// Serializes the parents so the hard link creation and the child runs stay
/// readable in order.
static PARENTS: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    /// The shipped Claude Code plugin (with its hook binary) and a spool.
    PluginAndSpool,
    /// No shipped Claude Code plugin.
    NoPlugin,
    /// The shipped plugin is present but the spool cannot be created.
    PluginNoSpool,
}

impl Layout {
    fn name(self) -> &'static str {
        match self {
            Self::PluginAndSpool => "plugin-and-spool",
            Self::NoPlugin => "no-plugin",
            Self::PluginNoSpool => "plugin-no-spool",
        }
    }

    fn parse(name: &str) -> Self {
        [Self::PluginAndSpool, Self::NoPlugin, Self::PluginNoSpool]
            .into_iter()
            .find(|layout| layout.name() == name)
            .expect("known layout")
    }
}

fn make_executable(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Re-run `test` in a child process for `layout`. Returns `true` in the parent
/// after the child passed.
fn isolated(test: &str, layout: Layout) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        return false;
    }
    let _guard = PARENTS.lock().unwrap_or_else(|poison| poison.into_inner());
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let recorded = home.path().join("args");
    fs::create_dir(&recorded).unwrap();
    for program in PROGRAMS {
        // Each fake records its argument count and arguments, then idles.
        make_executable(
            &bin.join(program),
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$#\" \"$@\" > \"$MADE_FAKE_ARGS_DIR/{program}\"\nsleep 30\n"
            ),
        );
    }
    let tmp = home.path().join("tmp");
    let runtime = home.path().join("runtime");
    let roaming = home.path().join("AppData").join("Roaming");
    let local = home.path().join("AppData").join("Local");
    for directory in [&tmp, &runtime, &roaming, &local] {
        fs::create_dir_all(directory).unwrap();
    }
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();

    // An install root of its own: the test executable hard-linked into a fresh
    // directory, so the plugin layout beside it is exactly what this layout
    // says, whatever other tests staged beside the cargo `deps` directory.
    let install = home.path().join("install");
    fs::create_dir_all(&install).unwrap();
    let executable = install.join("launch-env-characterization");
    let current = std::env::current_exe().unwrap();
    if fs::hard_link(&current, &executable).is_err() {
        fs::copy(&current, &executable).unwrap();
    }
    stage_integration_files(&install, layout != Layout::NoPlugin);
    let opencode_plugin = opencode_plugin_dir(&install);

    let path = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(bin.clone()).chain(std::env::split_paths(&path));
    let output = Command::new(&executable)
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .current_dir(home.path())
        .env(CHILD_ENV, "1")
        .env("MADE_FAKE_ARGS_DIR", &recorded)
        .env(LAYOUT_ENV, layout.name())
        .env(BIN_ENV, &bin)
        .env(ROOT_ENV, &install)
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
        .env(made_platform::PLUGIN_DIR_ENV, &opencode_plugin)
        .env("SHELLOPTS", "errexit")
        .env("BASH_ENV", "/inherited/bash_env")
        .env("ENV", "/inherited/env")
        .env_remove(made_bridge::config::CONFIG_CONTENT_ENV)
        .env_remove(made_platform::SHELL_ENV)
        .envs(inherited_variables())
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

/// The run-specific parts of one transcript.
struct Placeholders {
    pairs: Vec<(String, &'static str)>,
}

impl Placeholders {
    fn apply(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for (from, to) in &self.pairs {
            text = text.replace(from.as_str(), to);
        }
        text
    }
}

/// Replace the number after `marker` up to the next non-digit with `<what>`.
fn mask_number(text: &str, marker: &str, what: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(index) = rest.find(marker) {
        let (head, tail) = rest.split_at(index + marker.len());
        out.push_str(head);
        let digits = tail.chars().take_while(char::is_ascii_digit).count();
        out.push_str(what);
        rest = &tail[digits..];
    }
    out.push_str(rest);
    out
}

/// Replace the text between `start` and `end` with `what`.
fn mask_between(text: &str, start: &str, end: &str, what: &str) -> String {
    let Some(from) = text.find(start).map(|index| index + start.len()) else {
        return text.to_owned();
    };
    let Some(to) = text[from..].find(end).map(|index| index + from) else {
        return text.to_owned();
    };
    format!("{}{what}{}", &text[..from], &text[to..])
}

fn env_value(env: &[(OsString, OsString)], key: &str) -> Option<String> {
    env.iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.to_string_lossy().into_owned())
}

/// Transcript of one started target.
fn transcript(workbench: &mut Workbench, label: &str, placeholders: &Placeholders) -> String {
    let mut out = format!("== {label}\n");
    match workbench.terminal() {
        Some(pane) => {
            let options = pane.options();
            let program = options
                .shell
                .as_ref()
                .map_or("<platform shell>".to_owned(), |shell| {
                    placeholders.apply(&shell.to_string_lossy())
                });
            out.push_str(&format!("program: {program}\n"));
            let args: Vec<String> = options
                .args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            out.push_str(&format!("args: {args:?}\n"));
            let set: Vec<String> = options
                .env
                .iter()
                .map(|(key, value)| {
                    let key = key.to_string_lossy();
                    let value = if key == made_bridge::config::CONFIG_CONTENT_ENV {
                        "<composed configuration>".to_owned()
                    } else {
                        placeholders.apply(&value.to_string_lossy())
                    };
                    format!("{key}={value}")
                })
                .collect();
            out.push_str(&format!("set: {set:?}\n"));
            let removed: Vec<String> = options
                .env_remove
                .iter()
                .map(|key| key.to_string_lossy().into_owned())
                .collect();
            out.push_str(&format!("removed: {removed:?}\n"));
            out.push_str(&format!(
                "cwd: {}\n",
                placeholders.apply(&options.cwd.to_string_lossy())
            ));
        }
        None => out.push_str("terminal: none\n"),
    }
    out.push_str("log:\n");
    for entry in workbench.diagnostics().log().entries() {
        // The plugin source label depends on the build profile, not on the
        // launch target.
        if entry.message.starts_with("plugin source: ") {
            continue;
        }
        let message = placeholders.apply(&entry.message);
        // Where the user's own OpenCode configuration would be is a platform
        // convention, not a property of the launch.
        let message = mask_between(&message, "User config ", " was not found", "<user config>");
        let message = mask_number(&message, "(pid ", "<pid>");
        let message = mask_number(&message, "loopback port ", "<port>");
        let message = mask_number(&message, "(pane made-", "<pid>");
        out.push_str(&format!("  {}: {message}\n", entry.level.label()));
    }
    out
}

/// Child side: start every local target and return the combined transcript.
fn run_local(layout: Layout, cwd: &Path) -> String {
    let bin = PathBuf::from(std::env::var_os(BIN_ENV).unwrap());
    let root = PathBuf::from(std::env::var_os(ROOT_ENV).unwrap());
    if layout == Layout::PluginNoSpool {
        // The spool root cannot be created under a regular file.
        let temp = made_platform::host_platform().app_paths().unwrap().temp_dir;
        block_spool_root(&temp);
    }
    let mut out = String::new();
    for (label, target) in local_targets() {
        let mut workbench = Workbench::start(None, None, cwd.to_path_buf(), target, Instant::now());
        let spool = bound_spool(
            &workbench
                .terminal()
                .map(|pane| pane.options().env.clone())
                .unwrap_or_default(),
        );
        let mut pairs = vec![
            (bin.to_string_lossy().into_owned(), "<bin>"),
            (root.to_string_lossy().into_owned(), "<install>"),
            // The home as the filesystem names it and as the process was given
            // it (they differ below a symlinked temporary directory); the
            // longer spelling goes first.
            (home_dir().to_string_lossy().into_owned(), "<home>"),
            (raw_home(), "<home>"),
        ];
        if let Some(spool) = spool {
            assert!(Path::new(&spool).is_absolute() && Path::new(&spool).is_file());
            pairs.insert(0, (spool, "<spool>"));
        }
        let placeholders = Placeholders { pairs };
        out.push_str(&transcript(&mut workbench, label, &placeholders));
        workbench.shutdown();
    }
    out
}

fn child_layout() -> Layout {
    Layout::parse(&std::env::var(LAYOUT_ENV).unwrap())
}

/// `HOME` as the process was given it.
fn raw_home() -> String {
    std::env::var("HOME").expect("isolated home")
}

fn home_dir() -> PathBuf {
    made_platform::host_platform()
        .home_dir()
        .expect("test home")
        .canonicalize()
        .expect("canonical test home")
}

fn scenario(name: &str, layout: Layout, retry: bool) {
    if isolated(name, layout) {
        return;
    }
    let cwd = if retry {
        // A directory the child cannot enter fails the first spawn; a missing
        // one would be ignored by the PTY layer and never reach the retry.
        let locked = home_dir().join("locked-workspace");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read_dir(&locked).is_ok() {
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
            eprintln!("permissions are not enforced for this user; fallback retry not exercised");
            return;
        }
        locked
    } else {
        home_dir()
    };
    let out = run_local(child_layout(), &cwd);
    if retry {
        fs::set_permissions(&cwd, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let expected = if retry {
        retry_golden(layout)
    } else {
        golden(layout)
    };
    assert_eq!(
        out,
        expected.replace("<version>", env!("CARGO_PKG_VERSION"))
    );
}

fn golden(layout: Layout) -> &'static str {
    match layout {
        Layout::PluginAndSpool => GOLDEN_PLUGIN_AND_SPOOL,
        Layout::NoPlugin => GOLDEN_NO_PLUGIN,
        Layout::PluginNoSpool => GOLDEN_PLUGIN_NO_SPOOL,
    }
}

fn retry_golden(layout: Layout) -> &'static str {
    match layout {
        Layout::PluginAndSpool => GOLDEN_RETRY_PLUGIN_AND_SPOOL,
        Layout::NoPlugin => GOLDEN_RETRY_NO_PLUGIN,
        Layout::PluginNoSpool => unreachable!("no retry scenario for this layout"),
    }
}

#[test]
fn local_targets_with_the_plugin_and_a_spool() {
    scenario(
        "local_targets_with_the_plugin_and_a_spool",
        Layout::PluginAndSpool,
        false,
    );
}

#[test]
fn local_targets_without_the_plugin() {
    scenario("local_targets_without_the_plugin", Layout::NoPlugin, false);
}

#[test]
fn local_targets_with_the_plugin_and_no_spool() {
    scenario(
        "local_targets_with_the_plugin_and_no_spool",
        Layout::PluginNoSpool,
        false,
    );
}

#[test]
fn fallback_directory_retry_with_the_plugin_and_a_spool() {
    scenario(
        "fallback_directory_retry_with_the_plugin_and_a_spool",
        Layout::PluginAndSpool,
        true,
    );
}

#[test]
fn fallback_directory_retry_without_the_plugin() {
    scenario(
        "fallback_directory_retry_without_the_plugin",
        Layout::NoPlugin,
        true,
    );
}

#[test]
fn remote_targets_start_no_host_process() {
    if isolated(
        "remote_targets_start_no_host_process",
        Layout::PluginAndSpool,
    ) {
        return;
    }
    let authority = AuthorityId([7; 16]);
    let mut out = String::new();
    for (label, target) in remote_targets(authority) {
        let mut workbench = Workbench::start(None, None, home_dir(), target, Instant::now());
        let placeholders = Placeholders {
            pairs: vec![
                (home_dir().to_string_lossy().into_owned(), "<home>"),
                (raw_home(), "<home>"),
            ],
        };
        out.push_str(&transcript(&mut workbench, label, &placeholders));
        workbench.shutdown();
    }
    assert_eq!(
        out,
        GOLDEN_REMOTE.replace("<version>", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn the_shell_target_has_no_program_arguments_and_typing_grok_adds_no_dashboard() {
    if isolated(
        "the_shell_target_has_no_program_arguments_and_typing_grok_adds_no_dashboard",
        Layout::PluginAndSpool,
    ) {
        return;
    }
    // The shell target starts the platform shell with no program arguments
    // and no explicit program, whatever is on PATH.
    let path = std::env::var_os("PATH").unwrap_or_default();
    let options = shell_target()
        .pty_options(home_dir(), &path)
        .expect("the shell target always resolves");
    assert!(options.shell.is_none());
    assert!(options.args.is_empty(), "{:?}", options.args);
    // The Grok integration alone starts `grok dashboard`.
    let grok = grok_target()
        .pty_options(home_dir(), &path)
        .expect("grok is on PATH");
    assert_eq!(grok.args, vec![OsString::from("dashboard")]);

    // Typing `grok` in a shell tab runs the program exactly as typed.
    let captured =
        PathBuf::from(std::env::var_os("MADE_FAKE_ARGS_DIR").unwrap()).join(TYPED_PROGRAM);
    let mut workbench = Workbench::start(None, None, home_dir(), shell_target(), Instant::now());
    let mut log = made_app::DiagnosticLog::new();
    let terminal = workbench.terminal_mut().expect("shell tab");
    for ch in TYPED_PROGRAM.chars() {
        terminal.send_key(
            made_terminal::Key::Char(ch),
            made_terminal::Modifiers::default(),
            &mut log,
        );
    }
    terminal.send_key(
        made_terminal::Key::Enter,
        made_terminal::Modifiers::default(),
        &mut log,
    );
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    let content = loop {
        match fs::read_to_string(&captured) {
            Ok(content) if !content.is_empty() => break content,
            _ if Instant::now() >= deadline => panic!("the typed grok never ran"),
            _ => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    };
    workbench.shutdown();
    assert_eq!(content, "0\n", "no argument, in particular no `dashboard`");
}

const GOLDEN_PLUGIN_AND_SPOOL: &str = r#"== shell
program: <platform shell>
args: []
set: ["CLAUDE_CODE_PLUGIN_DIRS=/user/one:/user/two:<install>/plugins/claude-code-made", "MADE_CLAUDE_SPOOL=<spool>", "OPENCODE_CLI_CONFIG_CONTENT=<composed configuration>"]
removed: ["SHELLOPTS", "BASH_ENV", "ENV"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-1)
  info: terminal shell started (pid <pid>)
== opencode
program: <bin>/opencode
args: []
set: ["OPENCODE_CLI_CONFIG_CONTENT=<composed configuration>"]
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-2)
  info: terminal OpenCode started (pid <pid>)
== grok
program: <bin>/grok
args: ["dashboard"]
set: []
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-3)
  info: terminal Grok Build started (pid <pid>)
  warn: terminal Grok Build started with the original environment (no Made configuration)
== claude-code
program: <bin>/claude
args: []
set: ["CLAUDE_CODE_PLUGIN_DIRS=/user/one:/user/two:<install>/plugins/claude-code-made", "MADE_CLAUDE_SPOOL=<spool>"]
removed: ["SHELLOPTS", "BASH_ENV", "ENV"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-4)
  info: terminal Claude Code started (pid <pid>)
  warn: terminal Claude Code started with the original environment (no Made configuration)
"#;

const GOLDEN_NO_PLUGIN: &str = r#"== shell
program: <platform shell>
args: []
set: ["OPENCODE_CLI_CONFIG_CONTENT=<composed configuration>"]
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-1)
  warn: Claude Code session-status hooks are unavailable for this tab
  info: terminal shell started (pid <pid>)
== opencode
program: <bin>/opencode
args: []
set: ["OPENCODE_CLI_CONFIG_CONTENT=<composed configuration>"]
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-2)
  info: terminal OpenCode started (pid <pid>)
== grok
program: <bin>/grok
args: ["dashboard"]
set: []
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-3)
  info: terminal Grok Build started (pid <pid>)
  warn: terminal Grok Build started with the original environment (no Made configuration)
== claude-code
program: <bin>/claude
args: []
set: []
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-4)
  warn: Claude Code session-status hooks are unavailable for this tab
  info: terminal Claude Code started (pid <pid>)
  warn: terminal Claude Code started with the original environment (no Made configuration)
"#;

const GOLDEN_PLUGIN_NO_SPOOL: &str = r#"== shell
program: <platform shell>
args: []
set: ["CLAUDE_CODE_PLUGIN_DIRS=/user/one:/user/two:<install>/plugins/claude-code-made", "OPENCODE_CLI_CONFIG_CONTENT=<composed configuration>"]
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-1)
  warn: Claude Code spool could not be created: private runtime: private directory path is not a directory
  warn: Claude Code session-status hooks are unavailable for this tab
  info: terminal shell started (pid <pid>)
== opencode
program: <bin>/opencode
args: []
set: ["OPENCODE_CLI_CONFIG_CONTENT=<composed configuration>"]
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-2)
  info: terminal OpenCode started (pid <pid>)
== grok
program: <bin>/grok
args: ["dashboard"]
set: []
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-3)
  info: terminal Grok Build started (pid <pid>)
  warn: terminal Grok Build started with the original environment (no Made configuration)
== claude-code
program: <bin>/claude
args: []
set: ["CLAUDE_CODE_PLUGIN_DIRS=/user/one:/user/two:<install>/plugins/claude-code-made"]
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-4)
  warn: Claude Code spool could not be created: private runtime: private directory path is not a directory
  warn: Claude Code session-status hooks are unavailable for this tab
  info: terminal Claude Code started (pid <pid>)
  warn: terminal Claude Code started with the original environment (no Made configuration)
"#;

const GOLDEN_RETRY_PLUGIN_AND_SPOOL: &str = r#"== shell
program: <platform shell>
args: []
set: ["CLAUDE_CODE_PLUGIN_DIRS=/user/one:/user/two:<install>/plugins/claude-code-made", "MADE_CLAUDE_SPOOL=<spool>", "OPENCODE_CLI_CONFIG_CONTENT=<composed configuration>"]
removed: ["SHELLOPTS", "BASH_ENV", "ENV"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>/locked-workspace
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-1)
  warn: workspace shell failed; terminal started in the fallback directory
== opencode
program: <bin>/opencode
args: []
set: ["OPENCODE_CLI_CONFIG_CONTENT=<composed configuration>"]
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>/locked-workspace
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-2)
  warn: workspace OpenCode failed; terminal started in the fallback directory
== grok
program: <bin>/grok
args: ["dashboard"]
set: []
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>/locked-workspace
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-3)
  warn: workspace Grok Build failed; terminal started in the fallback directory
== claude-code
program: <bin>/claude
args: []
set: ["CLAUDE_CODE_PLUGIN_DIRS=/user/one:/user/two:<install>/plugins/claude-code-made", "MADE_CLAUDE_SPOOL=<spool>"]
removed: ["SHELLOPTS", "BASH_ENV", "ENV"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>/locked-workspace
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-4)
  warn: workspace Claude Code failed; terminal started in the fallback directory
"#;

const GOLDEN_RETRY_NO_PLUGIN: &str = r#"== shell
program: <platform shell>
args: []
set: ["OPENCODE_CLI_CONFIG_CONTENT=<composed configuration>"]
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>/locked-workspace
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-1)
  warn: Claude Code session-status hooks are unavailable for this tab
  warn: workspace shell failed; terminal started in the fallback directory
== opencode
program: <bin>/opencode
args: []
set: ["OPENCODE_CLI_CONFIG_CONTENT=<composed configuration>"]
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>/locked-workspace
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-2)
  warn: workspace OpenCode failed; terminal started in the fallback directory
== grok
program: <bin>/grok
args: ["dashboard"]
set: []
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>/locked-workspace
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-3)
  warn: workspace Grok Build failed; terminal started in the fallback directory
== claude-code
program: <bin>/claude
args: []
set: []
removed: ["SHELLOPTS", "BASH_ENV", "ENV", "MADE_CLAUDE_SPOOL"]
cwd: <home>
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>/locked-workspace
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-4)
  warn: Claude Code session-status hooks are unavailable for this tab
  warn: workspace Claude Code failed; terminal started in the fallback directory
"#;

const GOLDEN_REMOTE: &str = r#"== remote-shell
terminal: none
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-1)
== remote-opencode
terminal: none
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-2)
== remote-grok
terminal: none
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-3)
== remote-claude
terminal: none
log:
  info: Made <version> starting (invalid-workspace)
  info: terminal working directory: <home>
  info: user OpenCode config captured (missing)
  warn: configuration warning: User config <user config> was not found; composed from other inputs.
  warn: configuration warning: Forced tabs.mode to "on" for this Made launch.
  warn: configuration warning: Forced tabs.scope to "global" for this Made launch.
  info: integration ready on loopback port <port> (pane made-<pid>-4)
"#;
