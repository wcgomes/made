//! The attached session's capability report shares the companion's grok
//! detection. Its own test binary: the supervisor reads the process environment,
//! so this file holds exactly one test that sets it.
#![cfg(unix)]

use made_remote::{
    protocol::*,
    supervisor::{
        capability_bytes, read_envelope, write_envelope, ChildLaunch, Endpoint, Supervisor,
    },
};
use std::{
    ffi::OsString,
    io::{ErrorKind, Write},
    net::TcpStream,
    os::unix::fs::PermissionsExt,
    thread,
    time::{Duration, Instant},
};

#[test]
fn supervisor_capabilities_report_grok_found_only_in_the_user_install_dir() {
    let temp = tempfile::tempdir().unwrap();
    let home = std::fs::canonicalize(temp.path()).unwrap().join("home");
    let bin = home.join(".grok/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("grok");
    std::fs::write(&fake, b"#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    // A non-interactive SSH command's environment: no ~/.grok/bin on PATH.
    std::env::set_var("HOME", &home);
    std::env::set_var("PATH", "/nonexistent");

    let workspace = temp.path().join("work");
    std::fs::create_dir_all(&workspace).unwrap();
    let runtime = temp.path().join("remote/runtime/sha256-0123456789abcdef");
    made_platform::ensure_private_directory(&runtime).unwrap();
    let identity = Identity {
        authority: AuthorityId::random().unwrap(),
        tab: TabCapability::random().unwrap(),
        epoch: 1,
        generation: 1,
    };
    let first = WireEnvelope::new(
        identity.clone(),
        1,
        Operation::Open {
            directory: RemotePath::new(workspace.as_os_str().as_encoded_bytes().to_vec()).unwrap(),
            target: LaunchTarget::Shell,
            rows: 24,
            cols: 80,
        },
    );
    let supervisor = Supervisor::create(
        &first,
        runtime.clone(),
        temp.path(),
        std::path::Path::new(env!("CARGO_BIN_EXE_made-remote")),
        ChildLaunch {
            program: "/bin/sh".into(),
            args: vec![OsString::from("-c"), OsString::from("sleep 30")],
            cwd: workspace,
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
        },
        None,
    )
    .expect("supervisor create");
    let runner = thread::spawn(move || supervisor.run());
    let limit = Instant::now() + Duration::from_secs(15);
    let endpoint: Endpoint = loop {
        assert!(Instant::now() < limit, "endpoint.json was never published");
        if let Ok(bytes) = std::fs::read(runtime.join("endpoint.json")) {
            break serde_json::from_slice(&bytes).expect("endpoint");
        }
        thread::sleep(Duration::from_millis(20));
    };
    let mut stream = TcpStream::connect(endpoint.addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    stream
        .write_all(&capability_bytes(&first.identity).unwrap())
        .unwrap();
    stream.write_all(&first.identity.authority.0).unwrap();
    stream
        .write_all(&first.identity.epoch.to_be_bytes())
        .unwrap();
    write_envelope(&mut stream, &first).expect("write first frame");
    write_envelope(
        &mut stream,
        &WireEnvelope::new(identity.clone(), 2, Operation::Capabilities),
    )
    .expect("request capabilities");
    let grok = loop {
        assert!(Instant::now() < limit, "no capabilities reply");
        match read_envelope(&mut stream) {
            Ok(frame) => {
                if let Operation::CapabilitiesResult { capabilities } = frame.operation {
                    break capabilities.grok;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(error) => panic!("attachment socket failed: {error}"),
        }
    };
    write_envelope(
        &mut stream,
        &WireEnvelope::new(identity, 9_999, Operation::Close),
    )
    .expect("send close");
    drop(stream);
    let _ = runner.join();
    assert!(grok, "grok in ~/.grok/bin must be reported as detected");
}
