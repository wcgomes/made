//! Invalid workspace tests: missing, non-directory, unreadable, and empty.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use made_app::workspace::{Surfaces, WorkspaceErrorKind, WorkspaceLaunch};

#[test]
fn missing_workspace_keeps_the_terminal_usable() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("does-not-exist");

    let launch = WorkspaceLaunch::from_arg(Some(OsStr::new(&missing)));
    assert_eq!(launch.surfaces(), Surfaces::terminal_only());
    assert!(launch.surfaces().has_terminal());
    assert!(!launch.surfaces().has_explorer());
    assert!(launch.plan().is_none());
    assert!(launch.terminal_cwd().is_some_and(Path::is_dir));
    assert!(launch.explorer_root().is_none());

    let error = launch.error().expect("missing workspace error");
    assert_eq!(error.kind(), WorkspaceErrorKind::Missing);
    assert!(!error.is_blocking());
    assert!(error.message().contains("does-not-exist"));
}

#[test]
fn file_workspace_is_not_a_directory() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("not-a-directory.txt");
    fs::write(&file, "content").unwrap();

    let launch = WorkspaceLaunch::from_arg(Some(OsStr::new(&file)));
    assert_eq!(launch.surfaces(), Surfaces::terminal_only());
    let error = launch.error().unwrap();
    assert_eq!(error.kind(), WorkspaceErrorKind::NotADirectory);
    assert!(launch.terminal_cwd().is_some_and(Path::is_dir));
}

#[test]
fn unreadable_workspace_is_reported_without_blocking() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("unreadable");
    fs::create_dir(&root).unwrap();
    if !chmod(&root, "000") {
        eprintln!("skipping unreadable assertion: chmod is unavailable");
        return;
    }

    let readable = fs::read_dir(&root).is_ok();
    let launch = WorkspaceLaunch::from_arg(Some(OsStr::new(&root)));
    let result = chmod(&root, "755");
    assert!(result, "could not restore permissions");

    if readable {
        eprintln!("skipping unreadable assertion: the current user can read a mode-000 directory");
        return;
    }

    let error = launch.error().expect("unreadable workspace error");
    assert_eq!(error.kind(), WorkspaceErrorKind::Unreadable);
    assert!(!error.is_blocking());
    assert_eq!(launch.surfaces(), Surfaces::terminal_only());
}

#[test]
fn invalid_explicit_path_has_an_actionable_message() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing").join("nested");
    let launch = WorkspaceLaunch::from_arg(Some(OsStr::new(&missing)));
    let error = launch.error().unwrap();
    assert!(error
        .message()
        .starts_with("the workspace folder does not exist"));
    assert!(error.message().contains("missing"));
    assert_eq!(error.kind().code(), "workspace_missing");
}

#[test]
fn no_workspace_argument_opens_home_without_a_picker() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    let launch = WorkspaceLaunch::from_selection_with_home(
        made_app::workspace::WorkspaceSelection::None,
        made_platform::validate_user_home(&home),
    );
    assert_eq!(launch.surfaces(), Surfaces::full());
    assert_eq!(
        launch.source(),
        made_app::workspace::WorkspaceSource::DefaultHome
    );
    assert!(launch.error().is_none());
    assert!(launch.picker().is_none());
    assert!(launch.terminal_cwd().is_some_and(Path::is_dir));
    assert_eq!(
        launch.explorer_root(),
        Some(fs::canonicalize(&home).unwrap().as_path())
    );
}

fn chmod(path: &Path, mode: &str) -> bool {
    std::process::Command::new("chmod")
        .arg(mode)
        .arg(path)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
