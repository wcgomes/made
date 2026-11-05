use made_bridge::{config_guard::ConfigSnapshot, security::Token};
use made_remote::integration::{resolve_config, Integration};
use made_remote::protocol::{ErrorCode, Operation};
use serde_json::{json, Value};
use std::{
    fs,
    net::TcpStream,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tungstenite::{client::IntoClientRequest, http::header, Error as WsError, Message, WebSocket};
fn plugin_fixture(root: &Path, name: &str) -> PathBuf {
    let plugin = root.join(name);
    fs::create_dir_all(&plugin).unwrap();
    fs::write(plugin.join("package.json"), "{\"name\":\"opencode-made\"}").unwrap();
    fs::write(plugin.join("tui.ts"), "export default {}").unwrap();
    plugin
}

fn user_config(root: &Path) -> PathBuf {
    let config = root.join("cli.json");
    fs::write(
        &config,
        r#"{"theme":"remote","plugins":["existing"],"tabs":{"mode":"off","scope":"tab","layout":"wide"},"agent":{"model":"x"}}"#,
    )
    .unwrap();
    config
}

fn session(id: &str, active: bool, directory: &str) -> Value {
    json!({
        "sessionID": id,
        "title": id,
        "active": active,
        "directory": directory,
    })
}

fn state(revision: u64, active: Option<&str>, sessions: Vec<Value>) -> Value {
    json!({
        "type": "state",
        "revision": revision,
        "activeSessionID": active,
        "tabsEnabled": true,
        "sessions": sessions,
    })
}

struct FakePlugin {
    socket: WebSocket<TcpStream>,
}

fn hello(token: &Token, pane: &str) -> Value {
    json!({
        "type": "hello",
        "protocolVersion": 1,
        "token": token.expose_secret(),
        "paneId": pane,
        "pluginVersion": "0.1.0-test",
        "opencodeVersion": "2.1.4",
        "pid": 1,
        "cwd": "/tmp",
    })
}

fn bridge_details(service: &Integration) -> (String, Token, String) {
    let (_, content) = service.environment();
    let value: Value = serde_json::from_str(content).unwrap();
    let made = &value["plugins"].as_array().unwrap().last().unwrap()["options"]["made"];
    let url = made["bridgeUrl"].as_str().unwrap().to_owned();
    let token = Token::from_hex(made["token"].as_str().unwrap()).unwrap();
    let pane = made["paneId"].as_str().unwrap().to_owned();
    (url, token, pane)
}

/// Perform the WebSocket upgrade with a generous timeout and an optional Host
/// override. The bridge must reject a non-authority Host before disclosing any
/// handshake state.
fn connect_socket(url: &str, host: Option<&str>) -> Result<WebSocket<TcpStream>, String> {
    use tungstenite::http::HeaderValue;

    let mut request = url.into_client_request().unwrap();
    if let Some(host) = host {
        request
            .headers_mut()
            .insert(header::HOST, HeaderValue::from_str(host).unwrap());
    }
    let authority = url
        .strip_prefix("ws://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    let stream = TcpStream::connect(authority).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    tungstenite::client(request, stream)
        .map(|(socket, _)| socket)
        .map_err(|error| error.to_string())
}

impl FakePlugin {
    fn connect(service: &Integration) -> Self {
        let (url, token, pane) = bridge_details(service);
        let mut socket = connect_socket(&url, None).unwrap();
        // Short reads keep poll loops responsive; apply them only after the
        // upgrade, never to the handshake itself.
        socket
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let mut plugin = Self { socket };
        plugin.send(&hello(&token, &pane));
        let welcome = plugin.next_json();
        assert_eq!(welcome["type"], "welcome");
        assert_eq!(welcome["protocolVersion"], 1);
        plugin
    }

    fn send(&mut self, value: &Value) {
        self.socket
            .send(Message::Text(value.to_string().into()))
            .unwrap();
    }

    fn next_json(&mut self) -> Value {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    return serde_json::from_str(text.as_str()).unwrap();
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {
                    let _ = self.socket.flush();
                }
                Ok(Message::Binary(_)) => panic!("bridge sent binary data"),
                Ok(Message::Close(frame)) => panic!("bridge closed: {frame:?}"),
                Err(WsError::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    assert!(
                        Instant::now() < deadline,
                        "timed out waiting for a bridge message"
                    );
                }
                Err(error) => panic!("bridge read failed: {error}"),
            }
        }
    }

    fn read_until(&mut self, kind: &str) -> Value {
        loop {
            let value = self.next_json();
            if value["type"] == kind {
                return value;
            }
        }
    }
}

fn read_close_code(socket: &mut WebSocket<TcpStream>) -> Option<u16> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match socket.read() {
            Ok(Message::Close(frame)) => return frame.map(|frame| u16::from(frame.code)),
            Ok(Message::Text(text)) => {
                panic!("bridge disclosed data before the expected close: {text}")
            }
            Ok(Message::Binary(_)) => panic!("bridge sent binary data before close"),
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
            Err(WsError::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for a close frame"
                );
            }
            Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => return None,
            Err(error) => panic!("bridge read failed while awaiting close: {error}"),
        }
    }
}

fn poll_until(
    service: &mut Integration,
    timeout: Duration,
    predicate: impl Fn(&(Option<u64>, Operation)) -> bool,
) -> (Option<u64>, Operation) {
    let deadline = Instant::now() + timeout;
    loop {
        for event in service.poll() {
            if predicate(&event) {
                return event;
            }
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for relay event"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn quiet_for(service: &mut Integration, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        assert!(
            service.poll().is_empty(),
            "unexpected relay event while quiet"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn unauthorized_host_and_token_handshakes_never_disclose_state() {
    let temp = tempfile::tempdir().unwrap();
    let plugin = plugin_fixture(temp.path(), "plugin");
    let config = user_config(temp.path());
    let mut service =
        Integration::start(&plugin, temp.path(), config, None, "pane".into()).unwrap();
    let (url, _token, pane) = bridge_details(&service);

    // A non-authority Host is rejected during the HTTP upgrade, before any
    // WebSocket or session state exists.
    let error = connect_socket(&url, Some("attacker.example")).unwrap_err();
    let rendered = error.to_ascii_lowercase();
    assert!(
        rendered.contains("403") || rendered.contains("forbidden"),
        "unexpected Host rejection: {error}"
    );
    assert!(service.poll().is_empty());
    assert_eq!(service.state(), Err(ErrorCode::Unavailable));

    // A valid Host with the wrong token is rejected before state disclosure;
    // the close frame must not be preceded by a state message.
    let wrong = Token::from_hex("f".repeat(64)).unwrap();
    let mut socket = connect_socket(&url, None).unwrap();
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    socket
        .send(Message::Text(hello(&wrong, &pane).to_string().into()))
        .unwrap();
    assert_eq!(read_close_code(&mut socket), Some(4401));
    assert!(service.poll().is_empty());
    assert_eq!(service.state(), Err(ErrorCode::Unavailable));

    // Positive control: only the authorized plugin can cause a disclosure.
    let mut plugin = FakePlugin::connect(&service);
    assert!(
        service.poll().is_empty(),
        "only connection lifecycle so far"
    );
    plugin.read_until("request_state");
    plugin.send(&state(
        7,
        Some("session-a"),
        vec![session("session-a", true, "/tmp/a")],
    ));
    poll_until(&mut service, Duration::from_secs(2), |(_, operation)| {
        matches!(operation, Operation::SessionSnapshot { .. })
    });
    let Operation::SessionSnapshot { snapshot } = service.state().unwrap() else {
        panic!("state must return the accepted snapshot");
    };
    assert_eq!(snapshot.revision, 7);
    assert_eq!(snapshot.active_session_id.as_deref(), Some("session-a"));
}

#[test]
fn bridge_is_actual_remote_loopback_and_launch_config_is_immutable() {
    let temp = tempfile::tempdir().unwrap();
    let plugin = temp.path().join("plugin");
    std::fs::create_dir(&plugin).unwrap();
    std::fs::write(plugin.join("package.json"), "{\"name\":\"opencode-made\"}").unwrap();
    std::fs::write(plugin.join("tui.ts"), "export default {}").unwrap();
    let config = temp.path().join("cli.json");
    std::fs::write(&config, "{\"theme\":\"remote\",\"plugins\":[\"existing\"]}").unwrap();
    let service = Integration::start(&plugin, temp.path(), config, None, "pane".into()).unwrap();
    assert!(
        service.url().starts_with("ws://127.0.0.1:"),
        "the bridge must be actual remote loopback: {}",
        service.url()
    );
    assert_eq!(service.environment().0, "OPENCODE_CLI_CONFIG_CONTENT");
    let value: Value = serde_json::from_str(service.environment().1).unwrap();
    assert_eq!(value["theme"], "remote");
    assert_eq!(value["plugins"][0], "existing");
    assert!(service.verify_config().unwrap().is_unchanged());
}

#[test]
fn focus_state_and_attachment_epochs_relay_through_the_real_bridge() {
    let temp = tempfile::tempdir().unwrap();
    let plugin = plugin_fixture(temp.path(), "plugin");
    let config = user_config(temp.path());
    let mut service =
        Integration::start(&plugin, temp.path(), config, None, "pane".into()).unwrap();
    assert_eq!(service.state(), Err(ErrorCode::Unavailable));
    assert_eq!(
        service.focus(1, "session-a"),
        Err(ErrorCode::Unavailable),
        "no plugin connection yet"
    );

    let mut plugin = FakePlugin::connect(&service);
    assert!(
        service.poll().is_empty(),
        "only connection lifecycle so far"
    );
    plugin.read_until("request_state");
    plugin.send(&state(
        1,
        Some("session-a"),
        vec![session("session-a", true, "/tmp/a")],
    ));
    poll_until(&mut service, Duration::from_secs(2), |(_, operation)| {
        matches!(operation, Operation::SessionSnapshot { .. })
    });
    let Operation::SessionSnapshot { snapshot } = service.state().unwrap() else {
        panic!("state must return the accepted snapshot");
    };
    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.active_session_id.as_deref(), Some("session-a"));

    assert_eq!(
        service.focus(40, ""),
        Err(ErrorCode::Unavailable),
        "an invalid focus session never enters the map"
    );
    service.focus(41, "session-a").unwrap();
    let focus = plugin.read_until("focus");
    assert_eq!(focus["sessionID"], "session-a");
    plugin.send(&json!({
        "type": "focus_result",
        "requestID": focus["requestID"],
        "ok": true,
    }));
    let (caller, operation) = poll_until(&mut service, Duration::from_secs(2), |(caller, _)| {
        *caller == Some(41)
    });
    assert_eq!(caller, Some(41));
    match operation {
        Operation::FocusResult {
            session_id,
            accepted,
        } => {
            assert_eq!(session_id, "session-a");
            assert!(accepted);
        }
        _ => panic!("focus relay returned another operation"),
    }

    service.focus(42, "session-b").unwrap();
    let second = plugin.read_until("focus");
    plugin.send(&json!({
        "type": "focus_result",
        "requestID": "uncorrelated",
        "ok": true,
    }));
    quiet_for(&mut service, Duration::from_millis(200));
    plugin.send(&json!({
        "type": "focus_result",
        "requestID": second["requestID"],
        "ok": false,
        "error": "session is gone",
    }));
    let (caller, operation) = poll_until(&mut service, Duration::from_secs(2), |(caller, _)| {
        *caller == Some(42)
    });
    assert_eq!(caller, Some(42));
    match operation {
        Operation::FocusResult {
            session_id,
            accepted,
        } => {
            assert_eq!(session_id, "session-b");
            assert!(!accepted, "the plugin's error is relayed");
        }
        _ => panic!("focus relay returned another operation"),
    }

    plugin.send(&state(
        1,
        Some("session-a"),
        vec![session("session-a", true, "/tmp/a")],
    ));
    quiet_for(&mut service, Duration::from_millis(200));
    let Operation::SessionSnapshot { snapshot } = service.state().unwrap() else {
        panic!("state must keep the accepted snapshot");
    };
    assert_eq!(snapshot.revision, 1, "a stale revision is ignored");

    service.focus(43, "session-c").unwrap();
    let late = plugin.read_until("focus");
    service.reset_attachment();
    plugin.read_until("request_state");
    assert_eq!(
        service.state(),
        Err(ErrorCode::Unavailable),
        "attachment reset drops uncertain snapshots"
    );
    plugin.send(&json!({
        "type": "focus_result",
        "requestID": late["requestID"],
        "ok": true,
    }));
    quiet_for(&mut service, Duration::from_millis(200));
    plugin.send(&state(
        2,
        Some("session-b"),
        vec![
            session("session-a", false, "/tmp/a"),
            session("session-b", true, "/tmp/b"),
        ],
    ));
    poll_until(&mut service, Duration::from_secs(2), |(_, operation)| {
        matches!(operation, Operation::SessionSnapshot { .. })
    });
    let Operation::SessionSnapshot { snapshot } = service.state().unwrap() else {
        panic!("state must return the new epoch snapshot");
    };
    assert_eq!(snapshot.revision, 2);
    assert_eq!(snapshot.active_session_id.as_deref(), Some("session-b"));

    for caller in 100..116 {
        service.focus(caller, "session-b").unwrap();
    }
    assert_eq!(
        service.focus(116, "session-b"),
        Err(ErrorCode::Limit),
        "outstanding focus commands are bounded"
    );
    service.reset_attachment();
    plugin.read_until("request_state");
    plugin.send(&state(
        3,
        Some("session-b"),
        vec![session("session-b", true, "/tmp/b")],
    ));
    poll_until(&mut service, Duration::from_secs(2), |(_, operation)| {
        matches!(operation, Operation::SessionSnapshot { .. })
    });

    drop(plugin);
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.state().is_ok() {
        assert!(Instant::now() < deadline, "disconnect was not relayed");
        let _ = service.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(service.state(), Err(ErrorCode::Unavailable));
    assert_eq!(
        service.focus(200, "session-b"),
        Err(ErrorCode::Unavailable),
        "focus is unavailable after disconnect"
    );
}

#[test]
fn launch_environment_is_the_single_stable_injection_and_preserves_config() {
    let temp = tempfile::tempdir().unwrap();
    let plugin = plugin_fixture(temp.path(), "plugin");
    let config = user_config(temp.path());
    let inline = r#"{"theme":"inline","plugins":["inline-plugin"],"tabs":{"mode":"on"},"editor":{"fontSize":14}}"#;
    let before = fs::read(&config).unwrap();
    let process_before = std::env::var_os("OPENCODE_CLI_CONFIG_CONTENT");
    let service = Integration::start(
        &plugin,
        temp.path(),
        config.clone(),
        Some(inline.to_owned()),
        "pane".into(),
    )
    .unwrap();
    assert_eq!(
        std::env::var_os("OPENCODE_CLI_CONFIG_CONTENT"),
        process_before,
        "composition never mutates the companion process environment"
    );
    let first = service.environment();
    let second = service.environment();
    assert_eq!(first, second);
    assert_eq!(first.0, "OPENCODE_CLI_CONFIG_CONTENT");

    let value: Value = serde_json::from_str(first.1).unwrap();
    assert_eq!(value["theme"], "inline");
    assert_eq!(value["agent"]["model"], "x");
    assert_eq!(value["editor"]["fontSize"], 14);
    assert_eq!(value["tabs"]["mode"], "on");
    assert_eq!(value["tabs"]["scope"], "global");
    assert_eq!(value["tabs"]["layout"], "wide");
    let plugins = value["plugins"].as_array().unwrap();
    assert_eq!(plugins.len(), 3);
    assert_eq!(plugins[0], "existing");
    assert_eq!(plugins[1], "inline-plugin");
    let made = &plugins[2]["options"]["made"];
    assert_eq!(
        plugins[2]["package"],
        fs::canonicalize(&plugin)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    );
    assert_eq!(made["bridgeUrl"], service.url());
    assert_eq!(made["paneId"], "pane");
    assert_eq!(made["protocolVersion"], 1);
    assert!(!made["appVersion"].as_str().unwrap().is_empty());
    let token = made["token"].as_str().unwrap();
    assert_eq!(token.len(), 64);
    assert!(Token::from_hex(token).is_ok());

    assert!(service.verify_config().unwrap().is_unchanged());
    assert_eq!(fs::read(&config).unwrap(), before);
}

#[test]
fn config_resolution_and_composition_failures_never_rewrite_user_files() {
    let temp = tempfile::tempdir().unwrap();
    let v2 = temp.path().join("config.json");
    let legacy = temp.path().join("legacy.json");
    // Host location selection for the default V2/legacy paths (including
    // macOS `~/Library/Application Support/opencode/cli.json`) is covered by
    // made-platform's contract.rs::macos_paths_use_library_directories; this
    // test covers only ordering and the guarantee that resolution/composition
    // never writes.
    assert_eq!(resolve_config(v2.clone(), legacy.clone()), v2);
    assert!(!v2.exists() && !legacy.exists());
    fs::write(&legacy, "{}").unwrap();
    assert_eq!(resolve_config(v2.clone(), legacy.clone()), legacy);
    fs::write(&v2, "{\"theme\":\"v2\"}").unwrap();
    let resolved_before = ConfigSnapshot::capture(&v2).unwrap();
    assert_eq!(resolve_config(v2.clone(), legacy.clone()), v2);
    assert!(resolved_before.verify().unwrap().is_unchanged());

    let plugin = plugin_fixture(temp.path(), "plugin");
    fs::write(&v2, "not json").unwrap();
    let v2_before = fs::read(&v2).unwrap();
    let legacy_before = fs::read(&legacy).unwrap();
    assert!(matches!(
        Integration::start(
            &plugin,
            temp.path(),
            resolve_config(v2.clone(), legacy.clone()),
            None,
            "pane".into()
        ),
        Err(ErrorCode::Unavailable)
    ));
    assert_eq!(fs::read(&v2).unwrap(), v2_before);
    assert_eq!(fs::read(&legacy).unwrap(), legacy_before);

    let config = user_config(temp.path());
    let before = fs::read(&config).unwrap();
    assert!(matches!(
        Integration::start(
            &plugin,
            temp.path(),
            config.clone(),
            Some("not json".to_owned()),
            "pane".into()
        ),
        Err(ErrorCode::Unavailable)
    ));
    assert_eq!(fs::read(&config).unwrap(), before);

    let incomplete = temp.path().join("incomplete-plugin");
    fs::create_dir(&incomplete).unwrap();
    fs::write(incomplete.join("package.json"), "{}").unwrap();
    assert!(matches!(
        Integration::start(
            &incomplete,
            temp.path(),
            config.clone(),
            None,
            "pane".into()
        ),
        Err(ErrorCode::Unavailable)
    ));
    assert_eq!(fs::read(&config).unwrap(), before);
}
