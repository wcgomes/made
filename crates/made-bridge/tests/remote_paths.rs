use made_bridge::state::{PathSemantics, SessionSnapshot, SnapshotError};

fn snapshot(view: &str, directory: &str, subpath: &str) -> SessionSnapshot {
    serde_json::from_value(serde_json::json!({
        "revision": 1, "activeSessionID": "s", "tabsEnabled": true,
        "viewDirectory": view,
        "sessions": [{"sessionID":"s", "title":"s", "active":true,
            "directory":directory, "subpath":subpath}]
    }))
    .unwrap()
}

#[test]
fn remote_posix_paths_do_not_use_host_syntax() {
    // Backslash and drive-looking text are ordinary POSIX name bytes, including
    // a backslash followed by '..'; Windows must not reinterpret either.
    let good = snapshot("/project/a\\..", "/project/C:\\child", "a\\..");
    assert!(good
        .validate_with_path_semantics(PathSemantics::RemotePosix)
        .is_ok());
    assert!(snapshot("/project", "/project", "./child")
        .validate_with_path_semantics(PathSemantics::RemotePosix)
        .is_err());
    for bad in ["relative", "C:\\project", "/a/../b", "/a/./b", "/nul\0"] {
        assert!(snapshot(bad, "/project", "child")
            .validate_with_path_semantics(PathSemantics::RemotePosix)
            .is_err());
        assert!(snapshot("/project", bad, "child")
            .validate_with_path_semantics(PathSemantics::RemotePosix)
            .is_err());
    }
    assert!(snapshot("/project", "/project", "../child")
        .validate_with_path_semantics(PathSemantics::RemotePosix)
        .is_err());
    assert!(snapshot("/project", "/project", "/child")
        .validate_with_path_semantics(PathSemantics::RemotePosix)
        .is_err());
}

#[test]
fn native_default_is_unchanged_and_identity_checks_are_shared() {
    let mut state = snapshot("/project", "/project", "./child");
    assert_eq!(
        state.validate(),
        state.validate_with_path_semantics(PathSemantics::NativeLocal)
    );
    if cfg!(windows) {
        assert!(state.validate().is_err());
    } else {
        assert!(state.validate().is_ok());
    }
    state.revision = 0;
    assert_eq!(
        state.validate_with_path_semantics(PathSemantics::RemotePosix),
        Err(SnapshotError::ZeroRevision)
    );
    state.revision = 1;
    state.sessions.push(state.sessions[0].clone());
    assert!(state
        .validate_with_path_semantics(PathSemantics::RemotePosix)
        .is_err());
}
