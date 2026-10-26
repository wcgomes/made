//! Private log-file hardening tests.
//!
//! The application writes diagnostics through `made_platform::open_private_log`.
//! These tests verify the helper does not follow symlinks, rejects non-regular
//! paths, creates owner-only directories, and produces an owner-only file.

use std::fs;
use std::io::Write;

use made_app::diagnostics::Diagnostics;

#[test]
fn unavailable_diagnostics_never_points_at_a_shared_temporary_path() {
    let diagnostics = Diagnostics::unavailable("app paths unavailable");
    assert!(diagnostics.copyable_log_path().starts_with("unavailable:"));
    assert!(!diagnostics.copyable_log_path().contains("tmp"));
}

#[test]
fn private_log_append_creates_a_private_file_and_parent() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("logs");
    let path = parent.join("integration.log");

    let mut file = made_platform::open_private_log(&path).expect("open private log");
    file.write_all(b"first\n").unwrap();
    drop(file);

    let mut file = made_platform::open_private_log(&path).expect("reopen private log");
    file.write_all(b"second\n").unwrap();
    drop(file);

    assert_eq!(fs::read(&path).unwrap(), b"first\nsecond\n");
}

#[test]
fn private_log_rejects_a_directory() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("logs");
    fs::create_dir(&path).unwrap();
    assert!(made_platform::open_private_log(&path).is_err());
}

#[cfg(unix)]
#[test]
fn private_log_is_owner_only_and_parent_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("private-logs");
    let path = parent.join("integration.log");
    let file = made_platform::open_private_log(&path).expect("open private log");
    drop(file);

    let file_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    let dir_mode = fs::metadata(&parent).unwrap().permissions().mode() & 0o777;
    assert_eq!(file_mode, 0o600, "log file mode: {file_mode:o}");
    assert_eq!(dir_mode, 0o700, "log directory mode: {dir_mode:o}");
}

#[cfg(unix)]
#[test]
fn private_log_rejects_a_symlink() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target.log");
    fs::write(&target, b"target").unwrap();
    let link = temp.path().join("integration.log");
    symlink(&target, &link).unwrap();

    assert!(
        made_platform::open_private_log(&link).is_err(),
        "private log followed a symlink"
    );
    assert_eq!(fs::read(&target).unwrap(), b"target");
}
