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

fn quote_posix(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("'{}'", text.replace('\'', "'\\''"))
}
