//! The detached runtime directory keeps `ensure_private_directory`'s
//! guarantees: a symlink or a non-directory at the path is refused before
//! anything is spawned, and an existing directory of this user is tightened
//! to owner-only. (A directory owned by another user is refused because it
//! cannot be tightened; that needs a second account, so it is not tested.)
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use made_platform::spawn_detached_command;

fn spawn(runtime: &Path) -> Result<(), made_platform::PlatformError> {
    spawn_detached_command(Path::new("/usr/bin/true"), &[], b"", runtime)
}

#[test]
fn a_symlink_at_the_runtime_path_is_refused() {
    let base = tempfile::tempdir().unwrap();
    let target = base.path().join("target");
    std::fs::create_dir(&target).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = base.path().join("detach");
    std::os::unix::fs::symlink(&target, &runtime).unwrap();
    spawn(&runtime).expect_err("a symlinked runtime directory is refused");
    assert!(std::fs::symlink_metadata(&runtime)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn a_file_at_the_runtime_path_is_refused() {
    let base = tempfile::tempdir().unwrap();
    let runtime = base.path().join("detach");
    std::fs::write(&runtime, b"").unwrap();
    spawn(&runtime).expect_err("a file at the runtime path is refused");
    assert!(std::fs::symlink_metadata(&runtime).unwrap().is_file());
}

#[test]
fn a_relative_runtime_path_is_refused() {
    spawn(Path::new("relative/detach")).expect_err("a relative runtime path is refused");
    assert!(!Path::new("relative").exists());
}

#[test]
fn an_existing_loose_directory_of_this_user_is_tightened_to_owner_only() {
    let base = tempfile::tempdir().unwrap();
    let runtime = base.path().join("detach");
    std::fs::create_dir(&runtime).unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o755)).unwrap();
    spawn(&runtime).expect("an existing directory of this user is reused");
    assert_eq!(
        std::fs::symlink_metadata(&runtime)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}
