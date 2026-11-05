//! Model/storage-spy tests, not SSH or filesystem permission acceptance.
use made_app::ssh_servers::*;
use made_remote::protocol::{Capabilities, ErrorCode, RemoteTarget};
use std::{cell::RefCell, rc::Rc};

fn probe_output(os: &str, arch: &str, opencode: &str, grok: &str) -> Vec<u8> {
    format!("MADE-PROBE-1\n{os}\n{arch}\n{opencode}\n{grok}\n").into_bytes()
}

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
        grok: false,
        python3: false,
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

#[test]
fn probe_parses_grok_detection_honestly_and_never_reports_python3() {
    let detected = parse_probe_output(&probe_output(
        "Linux",
        "aarch64",
        "/usr/local/bin/opencode",
        "/home/u/bin/grok",
    ))
    .unwrap();
    assert_eq!(detected.target, RemoteTarget::LinuxArm64);
    assert!(detected.opencode && detected.grok && !detected.python3);
    // A stale probe that still prints an interpreter line cannot set the
    // frozen field: it is never parsed.
    let stale =
        parse_probe_output(b"MADE-PROBE-1\nLinux\nx86_64\n\n/home/u/bin/grok\n/usr/bin/python3\n")
            .unwrap();
    assert!(stale.grok && !stale.python3);
    // Relative or option-like text is absence, never a host executable.
    let absent = parse_probe_output(&probe_output(
        "Darwin",
        "x86_64",
        "/usr/local/bin/opencode",
        "grok",
    ))
    .unwrap();
    assert!(absent.opencode && !absent.grok && !absent.python3);
    assert_eq!(absent.target, RemoteTarget::MacosX64);
    // Missing additive lines are honest absence, not a malformed probe.
    let short = parse_probe_output(b"MADE-PROBE-1\nLinux\nx86_64\n\n\n").unwrap();
    assert!(!short.opencode && !short.grok && !short.python3);
    // The banner is still required and controls still never claim tmux.
    assert_eq!(
        parse_probe_output(b"not-the-banner\n"),
        Err(ErrorCode::InvalidRequest)
    );
    assert!(!detected.tmux);
}

#[test]
fn grok_install_consent_is_distinct_persistent_and_edit_cleared() {
    let storage = Storage::default();
    let mut registry = ServerRegistry::load(storage.clone()).unwrap();
    let id = registry.add(metadata("server-a")).unwrap();
    assert!(!registry
        .snapshot(id)
        .unwrap()
        .server()
        .allows_grok_install("0.1.0"));
    registry.set_grok_consent(id, Some("0.1.0".into())).unwrap();
    // A distinct consent kind: granting Grok install never grants companion
    // provisioning, and version binding stays exact.
    assert!(!registry
        .snapshot(id)
        .unwrap()
        .server()
        .allows_provision("0.1.0"));
    assert!(registry
        .snapshot(id)
        .unwrap()
        .server()
        .allows_grok_install("0.1.0"));
    assert!(!registry
        .snapshot(id)
        .unwrap()
        .server()
        .allows_grok_install("0.2.0"));
    // Owned live snapshots keep their own copy across registry changes.
    let live = registry.snapshot(id).unwrap();
    // Refusal is the absence of consent; it never converts into approval.
    registry.set_grok_consent(id, None).unwrap();
    assert!(live.server().allows_grok_install("0.1.0"));
    assert!(!registry
        .snapshot(id)
        .unwrap()
        .server()
        .allows_grok_install("0.1.0"));
    // Re-granting persists across restart under the same schema version.
    registry.set_grok_consent(id, Some("0.1.0".into())).unwrap();
    assert!(ServerRegistry::load(storage.clone())
        .unwrap()
        .snapshot(id)
        .unwrap()
        .server()
        .allows_grok_install("0.1.0"));
    // Every record edit clears both consent kinds for the retargeted authority.
    registry.edit(id, metadata("server-b")).unwrap();
    let cleared = ServerRegistry::load(storage).unwrap();
    let snapshot = cleared.snapshot(id).unwrap();
    let server = snapshot.server();
    assert!(!server.allows_grok_install("0.1.0"));
    assert!(!server.allows_provision("0.1.0"));
}

#[test]
fn legacy_grok_field_absent_and_corrupt_grok_consent_preserve_originals() {
    fn with_bytes(bytes: Vec<u8>) -> Storage {
        let storage = Storage::default();
        storage.0.borrow_mut().bytes = Some(bytes);
        storage
    }
    let storage = Storage::default();
    let mut registry = ServerRegistry::load(storage.clone()).unwrap();
    let id = registry.add(metadata("server-a")).unwrap();
    registry.set_grok_consent(id, Some("0.1.0".into())).unwrap();
    let bytes = storage.0.borrow().bytes.clone().unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // A schema-v1 document written before the Grok consent kind loads with
    // that consent absent.
    let mut legacy = value.clone();
    for server in legacy["servers"].as_array_mut().unwrap() {
        server.as_object_mut().unwrap().remove("grok_consent");
    }
    let legacy = serde_json::to_vec(&legacy).unwrap();
    let restarted = ServerRegistry::load(with_bytes(legacy)).unwrap();
    assert!(!restarted
        .snapshot(id)
        .unwrap()
        .server()
        .allows_grok_install("0.1.0"));
    // A corrupt grok consent version is rejected without overwriting.
    value["servers"][0]["grok_consent"] = serde_json::json!({ "version": "" });
    let corrupt = serde_json::to_vec(&value).unwrap();
    let keeper = with_bytes(corrupt.clone());
    assert!(matches!(
        ServerRegistry::load(keeper.clone()),
        Err(RegistryError::Corrupt)
    ));
    assert_eq!(keeper.0.borrow().bytes.clone(), Some(corrupt));
    assert_eq!(keeper.0.borrow().writes, 0);
    // Unknown fields stay rejected under the closed schema.
    value["servers"][0]["made_up"] = serde_json::json!(1);
    let unknown = serde_json::to_vec(&value).unwrap();
    assert!(matches!(
        ServerRegistry::load(with_bytes(unknown)),
        Err(RegistryError::Corrupt)
    ));
    let _ = id;
}

#[test]
fn companion_artifact_finds_unix_made_remote_not_exe() {
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    let parent = std::env::current_exe()
        .expect("test executable")
        .parent()
        .expect("executable directory")
        .to_path_buf();
    let dir = parent.join("remote").join("test-companion-unix-name");
    let _cleanup = Cleanup(dir.clone());
    std::fs::create_dir_all(&dir).unwrap();

    let unix = dir.join("made-remote");
    std::fs::write(&unix, b"companion").unwrap();
    let found = companion_artifact("test-companion-unix-name").expect("unix companion");
    assert_eq!(found, unix);
    assert_eq!(
        found.file_name().and_then(|name| name.to_str()),
        Some("made-remote")
    );

    std::fs::remove_file(&unix).unwrap();
    std::fs::write(dir.join("made-remote.exe"), b"wrong").unwrap();
    assert!(
        companion_artifact("test-companion-unix-name").is_none(),
        "clients must not search remote/<target>/ for made-remote.exe"
    );
}

#[test]
fn update_required_classification_is_specific_and_sanitized() {
    use made_app::remote_session::{class_from_kind, SshFailureClass};

    let class = class_from_kind(made_platform::ssh::SshFailureKind::UpdateRequired);
    assert_eq!(class, SshFailureClass::UpdateRequired);
    let detail = SshFailureClass::UpdateRequired.detail();
    assert!(
        detail.to_ascii_lowercase().contains("companion"),
        "the update reason must name the companion: {detail}"
    );
    assert!(
        !detail.contains("ssh connection failed") && !detail.contains("Io"),
        "a same-version mismatch must never surface as generic transport failure: {detail}"
    );
    assert!(
        !detail.contains("/") && !detail.contains('\\'),
        "the reason carries no remote path: {detail}"
    );
    assert!(
        !detail
            .split_whitespace()
            .any(|word| word.len() >= 32 && word.chars().all(|ch| ch.is_ascii_hexdigit())),
        "the reason carries no hash: {detail}"
    );
}

#[test]
fn approved_update_stage_uses_the_update_header_and_initial_staging_is_unchanged() {
    use made_app::remote_session::{stage_body, update_stage_body, STAGE_FILES};

    let files: Vec<Vec<u8>> = STAGE_FILES
        .iter()
        .enumerate()
        .map(|(index, _)| vec![b'a' + index as u8])
        .collect();
    let initial = stage_body(true, "0.1.0", "x86_64-unknown-linux-gnu", &files).unwrap();
    assert!(initial.starts_with(b"consent-v1\n0.1.0\nx86_64-unknown-linux-gnu\n"));

    let update = update_stage_body(true, "0.1.0", "x86_64-unknown-linux-gnu", &files).unwrap();
    let text = String::from_utf8_lossy(&update);
    assert!(
        text.starts_with("consent-v2-update\n0.1.0\nx86_64-unknown-linux-gnu\n"),
        "only an approved update may use update mode: {text}"
    );
    // Every artifact is length-delimited after the header; no path or raw
    // secret is added by update mode.
    assert!(text.len() > "consent-v2-update\n1\n1\n".len());
    assert!(!text.contains("/home/"));
    // Refusal is not approval: no update payload can be built without it.
    assert!(update_stage_body(false, "0.1.0", "x86_64-unknown-linux-gnu", &files).is_err());
    assert!(stage_body(false, "0.1.0", "x86_64-unknown-linux-gnu", &files).is_err());
}

#[test]
fn grok_install_invocation_and_report_parsing_are_bounded_and_exact() {
    // The stdin body is exactly the version and target lines the fixed script
    // reads before execing the staged helper; nothing else is sent.
    let body = grok_install_body("0.1.0", "x86_64-unknown-linux-gnu");
    assert_eq!(
        String::from_utf8(body).unwrap(),
        "0.1.0\nx86_64-unknown-linux-gnu\n"
    );

    // The companion's honest outcomes parse, including the new-sessions-only
    // disclosure.
    let report =
        parse_grok_install_report(br#"{"outcome":"installed","new_sessions_only":true}"#).unwrap();
    assert_eq!(report.outcome, GrokInstallOutcome::Installed);
    assert!(report.new_sessions_only);
    assert!(report.reason.is_none());

    let report =
        parse_grok_install_report(br#"{"outcome":"skipped","new_sessions_only":true}"#).unwrap();
    assert_eq!(report.outcome, GrokInstallOutcome::Skipped);

    let report = parse_grok_install_report(
        br#"{"outcome":"failed","reason":"install-failed","new_sessions_only":false}"#,
    )
    .unwrap();
    assert_eq!(report.outcome, GrokInstallOutcome::Failed);
    assert_eq!(report.reason.as_deref(), Some("install-failed"));

    // Oversized, non-JSON, multi-line, unknown-outcome and over-long reason
    // replies fail bounded and honest.
    assert_eq!(
        parse_grok_install_report(&vec![b'a'; 4097]).unwrap_err(),
        ErrorCode::Limit
    );
    assert!(parse_grok_install_report(b"not json").is_err());
    assert!(
        parse_grok_install_report(b"{\"outcome\":\"installed\"}\n{\"outcome\":\"installed\"}")
            .is_err()
    );
    assert!(parse_grok_install_report(br#"{"outcome":"mysterious"}"#).is_err());
    let long_reason = format!("{}-reason", "x".repeat(60));
    assert!(parse_grok_install_report(
        format!(r#"{{"outcome":"failed","reason":"{long_reason}"}}"#)
            .into_bytes()
            .as_slice()
    )
    .is_err());
}
