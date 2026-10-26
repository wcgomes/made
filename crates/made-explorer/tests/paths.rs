use std::fs;

use made_explorer::paths::{PathError, PathGuard};

fn make_root(temp: &tempfile::TempDir) -> std::path::PathBuf {
    let root = temp.path().join("root");
    fs::create_dir_all(&root).expect("create root");
    root
}

#[test]
fn new_canonicalizes_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = make_root(&temp);
    fs::write(root.join("file.txt"), "hello").unwrap();

    let guard = PathGuard::new(root.join(".")).unwrap();
    assert_eq!(guard.root(), fs::canonicalize(&root).unwrap());
    assert!(guard.root().is_absolute());
}

#[test]
fn resolve_existing_accepts_relative_and_absolute_inside_targets() {
    let temp = tempfile::tempdir().unwrap();
    let root = make_root(&temp);
    fs::create_dir(root.join("src")).unwrap();
    let file = root.join("src").join("main.rs");
    fs::write(&file, "fn main() {}").unwrap();

    let guard = PathGuard::new(&root).unwrap();
    let canonical = fs::canonicalize(&file).unwrap();
    assert_eq!(guard.resolve_existing("src/main.rs").unwrap(), canonical);
    assert_eq!(guard.resolve_existing(&file).unwrap(), canonical);
}

#[test]
fn traversal_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = make_root(&temp);
    fs::write(temp.path().join("outside.txt"), "outside").unwrap();

    let guard = PathGuard::new(&root).unwrap();
    for attempted in [
        root.join("../outside.txt"),
        std::path::PathBuf::from("../outside.txt"),
        std::path::PathBuf::from("src/../../outside.txt"),
    ] {
        let error = guard.resolve_existing(attempted).unwrap_err();
        assert!(
            matches!(error, PathError::OutsideRoot { .. }),
            "expected traversal rejection, got {error:?}"
        );
    }
}

#[test]
fn absolute_target_outside_root_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = make_root(&temp);
    let outside = temp.path().join("outside.txt");
    fs::write(&outside, "outside").unwrap();

    let guard = PathGuard::new(&root).unwrap();
    let error = guard.resolve_existing(&outside).unwrap_err();
    assert!(matches!(error, PathError::OutsideRoot { .. }));
}

#[test]
fn missing_target_is_not_found() {
    let temp = tempfile::tempdir().unwrap();
    let root = make_root(&temp);
    let guard = PathGuard::new(&root).unwrap();

    let error = guard
        .resolve_existing(root.join("missing.txt"))
        .unwrap_err();
    assert!(matches!(error, PathError::NotFound { .. }));
}

#[test]
fn invalid_roots_are_typed() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing-root");
    let error = PathGuard::new(&missing).unwrap_err();
    assert!(matches!(error, PathError::InvalidRoot { .. }));

    let file = temp.path().join("file-root.txt");
    fs::write(&file, "not a directory").unwrap();
    let error = PathGuard::new(&file).unwrap_err();
    assert!(matches!(error, PathError::InvalidRoot { .. }));
}

#[test]
fn resolve_for_create_accepts_missing_paths_inside_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = make_root(&temp);
    let guard = PathGuard::new(&root).unwrap();

    let target = guard.resolve_for_create("new-dir/new-file.txt").unwrap();
    let canonical_root = fs::canonicalize(&root).unwrap();
    assert_eq!(target, canonical_root.join("new-dir").join("new-file.txt"));
    assert!(!target.exists());
}

#[test]
fn resolve_for_create_canonicalizes_existing_targets() {
    let temp = tempfile::tempdir().unwrap();
    let root = make_root(&temp);
    let file = root.join("existing.txt");
    fs::write(&file, "content").unwrap();
    let guard = PathGuard::new(&root).unwrap();

    assert_eq!(
        guard.resolve_for_create("existing.txt").unwrap(),
        fs::canonicalize(&file).unwrap()
    );
}

#[test]
fn resolve_for_create_rejects_traversal() {
    let temp = tempfile::tempdir().unwrap();
    let root = make_root(&temp);
    let guard = PathGuard::new(&root).unwrap();

    let error = guard.resolve_for_create("../escaped.txt").unwrap_err();
    assert!(matches!(error, PathError::OutsideRoot { .. }));
}

#[test]
fn explicitly_granted_roots_extend_the_allowlist() {
    let temp = tempfile::tempdir().unwrap();
    let root = make_root(&temp);
    let extra = temp.path().join("extra");
    fs::create_dir(&extra).unwrap();
    let extra_file = extra.join("note.txt");
    fs::write(&extra_file, "note").unwrap();

    let mut guard = PathGuard::new(&root).unwrap();
    let error = guard.resolve_existing(&extra_file).unwrap_err();
    assert!(matches!(error, PathError::OutsideRoot { .. }));

    let granted = guard.add_granted_root(&extra).unwrap();
    assert_eq!(granted, fs::canonicalize(&extra).unwrap());
    assert_eq!(
        guard.resolve_existing(&extra_file).unwrap(),
        fs::canonicalize(&extra_file).unwrap()
    );
    assert_eq!(guard.granted_roots().len(), 1);

    // Granting the active root again does not duplicate it.
    guard.add_granted_root(&root).unwrap();
    assert_eq!(guard.granted_roots().len(), 1);
}

#[test]
fn invalid_granted_root_is_typed() {
    let temp = tempfile::tempdir().unwrap();
    let root = make_root(&temp);
    let mut guard = PathGuard::new(&root).unwrap();

    let error = guard
        .add_granted_root(temp.path().join("not-there"))
        .unwrap_err();
    assert!(matches!(error, PathError::InvalidRoot { .. }));
    assert!(guard.granted_roots().is_empty());
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    #[test]
    fn root_symlink_is_canonicalized() {
        let temp = tempfile::tempdir().unwrap();
        let root = make_root(&temp);
        let link = temp.path().join("root-link");
        symlink(&root, &link).unwrap();

        let guard = PathGuard::new(&link).unwrap();
        assert_eq!(guard.root(), fs::canonicalize(&root).unwrap());
    }

    #[test]
    fn symlink_inside_root_is_allowed_and_canonicalized() {
        let temp = tempfile::tempdir().unwrap();
        let root = make_root(&temp);
        let target = root.join("target.txt");
        fs::write(&target, "target").unwrap();
        let link = root.join("link.txt");
        symlink(&target, &link).unwrap();

        let guard = PathGuard::new(&root).unwrap();
        assert_eq!(
            guard.resolve_existing(&link).unwrap(),
            fs::canonicalize(&target).unwrap()
        );
    }

    #[test]
    fn symlink_file_escape_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = make_root(&temp);
        let outside = temp.path().join("outside.txt");
        fs::write(&outside, "secret").unwrap();
        let link = root.join("escape.txt");
        symlink(&outside, &link).unwrap();

        let guard = PathGuard::new(&root).unwrap();
        let error = guard.resolve_existing(&link).unwrap_err();
        assert!(matches!(error, PathError::OutsideRoot { .. }));
    }

    #[test]
    fn symlink_directory_escape_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = make_root(&temp);
        let outside = temp.path().join("outside-dir");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "secret").unwrap();
        symlink(&outside, root.join("escape-dir")).unwrap();

        let guard = PathGuard::new(&root).unwrap();
        let error = guard.resolve_existing("escape-dir/secret.txt").unwrap_err();
        assert!(matches!(error, PathError::OutsideRoot { .. }));
    }

    #[test]
    fn resolve_for_create_rejects_symlinked_ancestor_escape() {
        let temp = tempfile::tempdir().unwrap();
        let root = make_root(&temp);
        let outside = temp.path().join("outside-dir");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, root.join("escape-dir")).unwrap();

        let guard = PathGuard::new(&root).unwrap();
        let error = guard
            .resolve_for_create("escape-dir/new-file.txt")
            .unwrap_err();
        assert!(matches!(error, PathError::OutsideRoot { .. }));
    }

    #[test]
    fn unreadable_root_is_rejected_at_construction() {
        let temp = tempfile::tempdir().unwrap();
        let root = make_root(&temp);
        fs::set_permissions(&root, fs::Permissions::from_mode(0o000)).unwrap();

        let probe = fs::read_dir(&root);
        let result = PathGuard::new(&root);

        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();

        if probe.is_ok() {
            // Running as root bypasses the permission bits.
            eprintln!("skipping unreadable-root assertion when running as root");
            return;
        }
        let error = result.unwrap_err();
        assert!(
            matches!(error, PathError::InvalidRoot { .. }),
            "expected invalid root, got {error:?}"
        );
    }

    #[test]
    fn unreadable_component_is_typed_unreadable() {
        let temp = tempfile::tempdir().unwrap();
        let root = make_root(&temp);
        let private = root.join("private");
        fs::create_dir(&private).unwrap();
        fs::write(private.join("secret.txt"), "secret").unwrap();
        fs::set_permissions(&private, fs::Permissions::from_mode(0o000)).unwrap();

        // Root can read anything, so the permission test is not meaningful
        // there; still restore permissions before returning.
        if fs::read_dir(&private).is_ok() {
            fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
            eprintln!("skipping unreadable-component assertion when running as root");
            return;
        }

        let guard = PathGuard::new(&root).unwrap();
        let result = guard.resolve_existing("private/secret.txt");
        fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();

        let error = result.unwrap_err();
        assert!(
            matches!(error, PathError::Unreadable { .. }),
            "expected unreadable, got {error:?}"
        );
    }
}
