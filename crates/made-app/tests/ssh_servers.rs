//! Model/storage-spy tests, not SSH or filesystem permission acceptance.
use made_app::ssh_servers::*;
use made_remote::protocol::{Capabilities, ErrorCode, RemoteTarget};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
struct Memory {
    bytes: Option<Vec<u8>>,
    writes: usize,
    fail: bool,
}
#[derive(Clone, Default)]
struct Storage(Rc<RefCell<Memory>>);
impl ServerStorage for Storage {
    fn read(&mut self, _: usize) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(self.0.borrow().bytes.clone())
    }
    fn replace(&mut self, bytes: &[u8]) -> Result<(), StorageError> {
        let mut state = self.0.borrow_mut();
        if state.fail {
            return Err(StorageError::WriteFailed);
        }
        state.bytes = Some(bytes.to_vec());
        state.writes += 1;
        Ok(())
    }
}
fn metadata(host: &str) -> ServerMetadata {
    ServerMetadata {
        display_name: "Development".into(),
        host: host.into(),
        user: None,
        port: Some(22),
        key_file: Some("/keys/reference-only".into()),
        initial_directory: None,
    }
}
fn capabilities() -> Capabilities {
    Capabilities {
        target: RemoteTarget::LinuxX64,
        tmux: true,
        opencode: true,
        shell_cwd: true,
        helper_version: "1".into(),
    }
}

#[test]
fn restart_crud_and_live_snapshot_are_independent() {
    let storage = Storage::default();
    let mut registry = ServerRegistry::load(storage.clone()).unwrap();
    let id = registry.add(metadata("server-a")).unwrap();
    registry.set_consent(id, Some("1".into())).unwrap();
    let live = registry.snapshot(id).unwrap();
    registry.edit(id, metadata("server-b")).unwrap();
    assert_eq!(live.server().metadata().host, "server-a");
    assert!(live.server().allows_provision("1"));
    assert!(!registry
        .snapshot(id)
        .unwrap()
        .server()
        .allows_provision("1"));
    let mut restarted = ServerRegistry::load(storage.clone()).unwrap();
    assert_eq!(
        restarted.snapshot(id).unwrap().server().metadata().host,
        "server-b"
    );
    assert_eq!(restarted.probe_state(id), ProbeState::Idle);
    restarted.remove(id).unwrap();
    assert!(ServerRegistry::load(storage).unwrap().servers().is_empty());
    assert_eq!(live.server().metadata().host, "server-a");
}

#[test]
fn corrupt_version_and_secret_fields_never_overwrite_original() {
    for (bytes, expected) in [
        (b"not-json".to_vec(), RegistryError::Corrupt),
        (
            br#"{"schema_version":2,"future":true}"#.to_vec(),
            RegistryError::Version(2),
        ),
        (
            br#"{"schema_version":1,"servers":[],"password":"do-not-store"}"#.to_vec(),
            RegistryError::Corrupt,
        ),
    ] {
        let storage = Storage::default();
        storage.0.borrow_mut().bytes = Some(bytes.clone());
        assert!(matches!(ServerRegistry::load(storage.clone()), Err(error) if error == expected));
        assert_eq!(storage.0.borrow().bytes, Some(bytes));
        assert_eq!(storage.0.borrow().writes, 0);
    }
}

#[test]
fn failure_does_not_commit_model_or_destroy_saved_metadata() {
    let storage = Storage::default();
    let mut registry = ServerRegistry::load(storage.clone()).unwrap();
    let id = registry.add(metadata("server-a")).unwrap();
    let before = storage.0.borrow().bytes.clone();
    storage.0.borrow_mut().fail = true;
    assert!(registry.edit(id, metadata("server-b")).is_err());
    assert_eq!(
        registry.snapshot(id).unwrap().server().metadata().host,
        "server-a"
    );
    assert_eq!(storage.0.borrow().bytes, before);
    let encoded = String::from_utf8(before.unwrap()).unwrap();
    for forbidden in [
        "password",
        "token",
        "private_key",
        "prompt",
        "capabilities",
        "tab",
    ] {
        assert!(!encoded.contains(forbidden));
    }
}

#[test]
fn probe_is_per_server_cancelled_or_edited_attempt_cannot_complete() {
    let mut registry = ServerRegistry::load(Storage::default()).unwrap();
    let a = registry.add(metadata("a")).unwrap();
    let b = registry.add(metadata("b")).unwrap();
    let (first, _) = registry.begin_probe(a).unwrap();
    let (other, _) = registry.begin_probe(b).unwrap();
    let (replacement, cancel) = registry.begin_probe(a).unwrap();
    assert_eq!(cancel, Some(first.ticket));
    assert_eq!(
        registry.finish_probe(first.ticket, Ok(capabilities())),
        Err(RegistryError::StaleProbe)
    );
    assert!(registry.cancel_probe(replacement.ticket));
    assert_eq!(
        registry.finish_probe(replacement.ticket, Ok(capabilities())),
        Err(RegistryError::StaleProbe)
    );
    registry
        .finish_probe(other.ticket, Ok(capabilities()))
        .unwrap();
    assert!(registry.can_launch_opencode(b));
    assert!(!registry.can_launch_opencode(a));
    let (probe, _) = registry.begin_probe(b).unwrap();
    assert_eq!(
        registry.edit(b, metadata("new-b")).unwrap(),
        Some(probe.ticket)
    );
    assert_eq!(
        registry.finish_probe(probe.ticket, Err(ErrorCode::Io)),
        Err(RegistryError::StaleProbe)
    );
    assert!(!registry.can_launch_opencode(b));
}

#[test]
fn rejects_option_targets_controls_and_zero_ports_before_storage() {
    let storage = Storage::default();
    let mut registry = ServerRegistry::load(storage.clone()).unwrap();
    for host in [
        "-oProxyCommand=bad",
        "a\nb",
        "user@host",
        "",
        "host argument",
    ] {
        assert!(registry.add(metadata(host)).is_err());
    }
    let mut invalid = metadata("host");
    invalid.port = Some(0);
    assert!(registry.add(invalid).is_err());
    assert_eq!(storage.0.borrow().writes, 0);
}
