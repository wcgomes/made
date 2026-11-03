use made_remote::{
    protocol::*,
    supervisor::{read_envelope, runtime_directory, write_envelope},
};
#[test]
fn frontend_handshake_is_bounded_versioned_and_secret_is_not_a_runtime_locator() {
    let identity = Identity {
        authority: AuthorityId::random().unwrap(),
        tab: TabCapability::random().unwrap(),
        epoch: 1,
        generation: 1,
    };
    let request = WireEnvelope::new(
        identity.clone(),
        1,
        Operation::Open {
            directory: RemotePath::new(b"/project".to_vec()).unwrap(),
            target: LaunchTarget::Shell,
            rows: 24,
            cols: 80,
        },
    );
    let mut bytes = Vec::new();
    write_envelope(&mut bytes, &request).unwrap();
    assert_eq!(read_envelope(&mut bytes.as_slice()).unwrap(), request);
    let path = runtime_directory(std::path::Path::new("/private/made"), &identity).unwrap();
    assert!(path.starts_with("/private/made/remote/runtime"));
    assert!(!path
        .to_string_lossy()
        .contains(&serde_json::to_string(&identity.tab).unwrap()));
    let oversized = ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes();
    assert!(read_envelope(&mut oversized.as_slice()).is_err());
}
