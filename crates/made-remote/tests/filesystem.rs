//! Unix path and private-root operations. Windows remotes are unsupported.
#![cfg(unix)]
use made_remote::{
    filesystem::{remote, Filesystem},
    protocol::*,
};
use std::fs;

#[test]
fn guarded_list_preview_and_generation_are_server_scoped() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("unicode.txt"), "é".repeat(MAX_CHUNK_BYTES)).unwrap();
    fs::write(temp.path().join("binary"), b"hello\0world").unwrap();
    let root = remote(&fs::canonicalize(temp.path()).unwrap()).unwrap();
    let mut service = Filesystem::new(&root, 1).unwrap();
    let listings = service.list(&root).unwrap();
    assert!(matches!(
        listings.last(),
        Some(Operation::Listed { done: true, .. })
    ));
    for op in &listings {
        op.validate().unwrap();
    }
    let path = remote(&service.root().join("unicode.txt")).unwrap();
    let previews = service.preview(&path).unwrap();
    assert!(previews.len() > 1);
    for op in &previews {
        op.validate().unwrap();
    }
    let binary = remote(&service.root().join("binary")).unwrap();
    assert!(matches!(
        service.preview(&binary).unwrap()[0],
        Operation::PreviewResult {
            kind: PreviewKind::Binary,
            ..
        }
    ));
    service.select(&root).unwrap();
    assert_eq!(service.check(1), Err(ErrorCode::Stale));
}
#[test]
fn outside_root_never_reaches_a_preview() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    fs::write(temp.path().join("secret"), "not allowed").unwrap();
    let root = remote(&fs::canonicalize(root).unwrap()).unwrap();
    let service = Filesystem::new(&root, 1).unwrap();
    let outside = remote(&fs::canonicalize(temp.path()).unwrap().join("secret")).unwrap();
    assert_eq!(service.preview(&outside), Err(ErrorCode::Unauthorized));
}
#[test]
fn watch_changes_are_coalesced_to_authorized_subscription() {
    let temp = tempfile::tempdir().unwrap();
    let root = remote(&fs::canonicalize(temp.path()).unwrap()).unwrap();
    let mut service = Filesystem::new(&root, 1).unwrap();
    service.watch(&root).unwrap();
    fs::write(temp.path().join("new"), "new").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Some(op) = service.poll_change() {
            assert_eq!(op, Operation::Changed { path: root });
            break;
        }
        assert!(std::time::Instant::now() < deadline, "watch deadline");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
