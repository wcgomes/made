use made_platform::{
    artifact_permissions_match, private_dacl_is_owner_only, private_dacl_refusal,
    read_private_metadata, write_private_metadata_atomic, PrivateAce, PrivateAceTrustee,
    PrivateDacl,
};
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
fn artifact_permissions_match_requires_exact_remote_stage_modes() {
    use std::os::unix::fs::PermissionsExt;
    let root = Scratch::new();
    std::fs::create_dir_all(&root.0).unwrap();
    let file = root.0.join("artifact");
    std::fs::write(&file, b"data").unwrap();
    for (mode, executable, expected) in [
        (0o600, false, true),
        (0o700, true, true),
        (0o600, true, false),
        (0o700, false, false),
        (0o644, false, false),
        (0o755, true, false),
        (0o400, false, false),
    ] {
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
        let metadata = std::fs::symlink_metadata(&file).unwrap();
        assert_eq!(
            artifact_permissions_match(&metadata, executable),
            expected,
            "mode {mode:o}, executable {executable}"
        );
    }
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

fn ace(trustee: PrivateAceTrustee, allow: bool, full_access: bool, inherited: bool) -> PrivateAce {
    PrivateAce {
        inherited,
        allow,
        full_access,
        trustee,
    }
}

fn dacl(protected: bool, owner_is_current: bool, aces: Vec<PrivateAce>) -> PrivateDacl {
    PrivateDacl {
        protected,
        owner_is_current,
        aces,
    }
}

#[test]
fn owner_only_dacl_accepts_canonical_forms_and_rejects_weaker_grants() {
    let owner = ace(PrivateAceTrustee::Owner, true, true, false);
    let rights = ace(PrivateAceTrustee::OwnerRights, true, true, false);
    assert!(private_dacl_is_owner_only(&dacl(true, true, vec![owner])));
    assert!(private_dacl_is_owner_only(&dacl(true, true, vec![rights])));
    assert!(private_dacl_is_owner_only(&dacl(
        true,
        true,
        vec![owner, rights]
    )));
    assert_eq!(
        private_dacl_refusal(&dacl(false, true, vec![owner])),
        Some("DACL is not protected")
    );
    assert_eq!(
        private_dacl_refusal(&dacl(true, false, vec![owner])),
        Some("owner is not the current user")
    );
    assert_eq!(
        private_dacl_refusal(&dacl(true, true, Vec::new())),
        Some("DACL has no ACE")
    );
    assert_eq!(
        private_dacl_refusal(&dacl(
            true,
            true,
            vec![ace(PrivateAceTrustee::Owner, true, true, true)]
        )),
        Some("ACE is inherited")
    );
    assert_eq!(
        private_dacl_refusal(&dacl(
            true,
            true,
            vec![ace(PrivateAceTrustee::Other, true, true, false)]
        )),
        Some("ACE trustee is not the owner")
    );
    assert_eq!(
        private_dacl_refusal(&dacl(
            true,
            true,
            vec![owner, ace(PrivateAceTrustee::Other, true, false, false)]
        )),
        Some("ACE trustee is not the owner")
    );
    assert_eq!(
        private_dacl_refusal(&dacl(
            true,
            true,
            vec![ace(PrivateAceTrustee::Owner, true, false, false)]
        )),
        Some("ACE mask is not full access")
    );
    assert_eq!(
        private_dacl_refusal(&dacl(
            true,
            true,
            vec![ace(PrivateAceTrustee::Owner, false, true, false)]
        )),
        Some("ACE is not an allow")
    );
}

/// Windows CI creates this directory under an isolated APPDATA. Creation must
/// assign the current user as owner and a protected full-allow DACL, then the
/// following read must accept that canonical ACL. Not executed here.
#[cfg(windows)]
#[test]
fn metadata_write_accepts_canonical_owner_only_dacl() {
    let root = Scratch::new();
    write_private_metadata_atomic(&root.0, "servers.json", b"registry", 32)
        .expect("canonicalized owner-only DACL must still be private");
    assert_eq!(
        read_private_metadata(&root.0, "servers.json", 32)
            .unwrap()
            .unwrap(),
        b"registry"
    );
}
