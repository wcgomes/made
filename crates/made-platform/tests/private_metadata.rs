use made_platform::{read_private_metadata, write_private_metadata_atomic};
use std::path::PathBuf;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let call = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let name = format!("made-metadata-test-{}-{nanos}-{call}", std::process::id());
        Self(std::env::temp_dir().join(name))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn metadata_is_bounded_atomic_and_preserves_original_on_invalid_write() {
    let root = Scratch::new();
    assert!(read_private_metadata(&root.0, "servers.json", 32)
        .unwrap()
        .is_none());
    write_private_metadata_atomic(&root.0, "servers.json", b"original", 32).unwrap();
    assert_eq!(
        read_private_metadata(&root.0, "servers.json", 32)
            .unwrap()
            .unwrap(),
        b"original"
    );
    assert!(write_private_metadata_atomic(&root.0, "servers.json", &[0; 33], 32).is_err());
    assert_eq!(
        read_private_metadata(&root.0, "servers.json", 32)
            .unwrap()
            .unwrap(),
        b"original"
    );
    write_private_metadata_atomic(&root.0, "servers.json", b"replacement", 32).unwrap();
    assert_eq!(
        read_private_metadata(&root.0, "servers.json", 32)
            .unwrap()
            .unwrap(),
        b"replacement"
    );
    assert!(read_private_metadata(&root.0, "servers.json", 3).is_err());
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 1);
    for name in [
        "../escape",
        "sub/file",
        "..",
        "",
        "a:b",
        "a\\b",
        "CON.json",
        "lpt1",
        "servers.",
    ] {
        assert!(write_private_metadata_atomic(&root.0, name, b"bad", 32).is_err());
        assert!(read_private_metadata(&root.0, name, 32).is_err());
    }
    std::fs::create_dir(root.0.join("not-file")).unwrap();
    assert!(read_private_metadata(&root.0, "not-file", 32).is_err());
    assert!(write_private_metadata_atomic(&root.0, "not-file", b"bad", 32).is_err());
}

#[cfg(unix)]
#[test]
fn metadata_rejects_symlinks_and_uses_private_modes() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let root = Scratch::new();
    write_private_metadata_atomic(&root.0, "servers.json", b"original", 32).unwrap();
    assert_eq!(
        std::fs::metadata(&root.0).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(root.0.join("servers.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    symlink("servers.json", root.0.join("link.json")).unwrap();
    assert!(read_private_metadata(&root.0, "link.json", 32).is_err());
    assert!(write_private_metadata_atomic(&root.0, "link.json", b"bad", 32).is_err());
    assert_eq!(
        std::fs::read(root.0.join("servers.json")).unwrap(),
        b"original"
    );
    let alias = Scratch::new();
    symlink(&root.0, &alias.0).unwrap();
    assert!(read_private_metadata(&alias.0, "servers.json", 32).is_err());
    assert!(write_private_metadata_atomic(&alias.0, "servers.json", b"bad", 32).is_err());
    std::fs::set_permissions(
        root.0.join("servers.json"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(read_private_metadata(&root.0, "servers.json", 32).is_err());
}

#[cfg(unix)]
#[test]
fn metadata_refuses_fifo_without_blocking() {
    use std::os::unix::ffi::OsStrExt;
    let root = Scratch::new();
    write_private_metadata_atomic(&root.0, "servers.json", b"original", 32).unwrap();
    let fifo = std::ffi::CString::new(root.0.join("fifo").as_os_str().as_bytes()).unwrap();
    // SAFETY: terminated path in the owned scratch directory; FIFO mode is private.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(read_private_metadata(&root.0, "fifo", 32).is_err());
    assert!(write_private_metadata_atomic(&root.0, "fifo", b"bad", 32).is_err());
}
