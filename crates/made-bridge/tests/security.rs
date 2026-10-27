mod common;

use std::{
    collections::HashSet,
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    thread,
    time::{Duration, Instant},
};

use common::*;
use made_bridge::{
    security::{
        is_loopback_ip, is_loopback_socket_addr, is_loopback_ws_url, loopback_ws_url,
        redact_secret, Redacted, Token, AUTH_TIMEOUT, FOCUS_TIMEOUT, HEARTBEAT_INTERVAL,
        MAX_MESSAGE_BYTES, MAX_PENDING_FOCUS, MAX_SESSIONS, MISSED_PONG_LIMIT, REDACTED,
        TOKEN_HEX_LEN,
    },
    server::{BridgeEvent, BridgeHandle, BridgeOptions},
};
use serde_json::json;
use tungstenite::http::{header, HeaderValue};

fn workspace_bridges() -> [(Token, BridgeHandle); 2] {
    ["workspace-a", "workspace-b"].map(|pane| {
        let token = Token::generate().expect("workspace token");
        let handle = made_bridge::server::start(BridgeOptions::new(token.clone(), pane))
            .expect("workspace bridge");
        (token, handle)
    })
}

fn authenticate_workspace(
    handle: &BridgeHandle,
    socket: &mut tungstenite::WebSocket<TcpStream>,
    token: &Token,
) -> u64 {
    send(
        socket,
        &hello_with(token, handle.pane_id(), 1, OPENCODE_VERSION),
    );
    assert_eq!(read_json(socket)["type"], "welcome");
    let event = wait_for(handle, |event| {
        assert!(
            !matches!(event, BridgeEvent::State(_)),
            "old-epoch state survived authentication: {event:?}"
        );
        matches!(event, BridgeEvent::Connected(_))
    });
    let BridgeEvent::Connected(info) = event else {
        unreachable!("waited for Connected");
    };
    assert_eq!(info.pane_id, handle.pane_id());
    info.connection_id
}

#[test]
fn workspace_endpoints_credentials_state_and_focus_are_independent() {
    let [(token_a, a), (token_b, b)] = workspace_bridges();
    assert_ne!(a.local_addr(), b.local_addr());
    assert_ne!(a.url(), b.url());
    assert_ne!(a.pane_id(), b.pane_id());
    assert!(!token_a.verify(token_b.expose_secret()));
    assert!(!token_b.verify(token_a.expose_secret()));
    for handle in [&a, &b] {
        assert_eq!(handle.local_addr().ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_ne!(handle.port(), 0);
    }

    let mut socket_a = connect(&a);
    let mut socket_b = connect(&b);
    let id_a = authenticate_workspace(&a, &mut socket_a, &token_a);
    let id_b = authenticate_workspace(&b, &mut socket_b, &token_b);
    // Session and request ids are local to an endpoint, not global routing keys.
    for (handle, socket, directory) in [
        (&a, &mut socket_a, "/workspace-a"),
        (&b, &mut socket_b, "/workspace-b"),
    ] {
        send(
            socket,
            &state(
                1,
                Some("shared"),
                vec![session("shared", "Session", true, Some(directory))],
            ),
        );
        let event = wait_for(handle, |event| matches!(event, BridgeEvent::State(_)));
        let BridgeEvent::State(snapshot) = event else {
            unreachable!()
        };
        assert_eq!(
            snapshot.sessions[0].directory.as_deref(),
            Some(std::path::Path::new(directory))
        );
    }
    let request_a = a.focus("shared").unwrap();
    let request_b = b.focus("shared").unwrap();
    assert_eq!(read_json(&mut socket_a)["requestID"], request_a);
    assert_eq!(read_json(&mut socket_b)["requestID"], request_b);
    send(
        &mut socket_a,
        &json!({"type": "focus_result", "requestID": request_a, "ok": true}),
    );
    wait_for(
        &a,
        |event| matches!(event, BridgeEvent::FocusResult(result) if result.ok),
    );
    assert_no_matching(&b, Duration::from_millis(100), |event| {
        matches!(event, BridgeEvent::FocusResult(_))
    });
    send(
        &mut socket_b,
        &json!({"type": "focus_result", "requestID": request_b, "ok": false, "error": "workspace-b only"}),
    );
    wait_for(
        &b,
        |event| matches!(event, BridgeEvent::FocusResult(result) if !result.ok && result.error.as_deref() == Some("workspace-b only")),
    );
    assert_eq!(a.current_connection_id(), Some(id_a));
    assert_eq!(b.current_connection_id(), Some(id_b));
    a.shutdown().unwrap();
    let nonce = b.ping().unwrap();
    assert_eq!(
        read_json(&mut socket_b)["nonce"],
        nonce,
        "closing A must leave B live"
    );
    b.shutdown().unwrap();
}

#[test]
fn cross_workspace_hello_is_rejected_before_replacement_or_disclosure() {
    let [(token_a, a), (token_b, b)] = workspace_bridges();
    let mut socket_a = connect(&a);
    let mut socket_b = connect(&b);
    let id_a = authenticate_workspace(&a, &mut socket_a, &token_a);
    let id_b = authenticate_workspace(&b, &mut socket_b, &token_b);

    for (handle, local_token, foreign_token, foreign_pane) in [
        (&a, &token_a, &token_b, b.pane_id()),
        (&b, &token_b, &token_a, a.pane_id()),
    ] {
        for (credential, pane, expected_error) in [
            (local_token, foreign_pane, "pane_mismatch"),
            (foreign_token, handle.pane_id(), "invalid_token"),
            (foreign_token, foreign_pane, "invalid_token"),
        ] {
            let mut intruder = connect(handle);
            send(
                &mut intruder,
                &hello_with(credential, pane, 1, OPENCODE_VERSION),
            );
            // Do not use read_close_code: it skips JSON and could hide a leaked welcome/state.
            match intruder.read().expect("rejection frame") {
                tungstenite::Message::Close(Some(frame)) => assert_eq!(u16::from(frame.code), 4401),
                message => panic!("cross-workspace hello disclosed data before close: {message:?}"),
            }
            let event = wait_for(handle, |event| {
                assert!(
                    !matches!(event, BridgeEvent::Connected(_) | BridgeEvent::State(_)),
                    "rejected hello changed workspace state: {event:?}"
                );
                matches!(event, BridgeEvent::ProtocolError(_))
            });
            assert!(
                matches!(event, BridgeEvent::ProtocolError(error) if error.code == expected_error)
            );
            assert_eq!(a.current_connection_id(), Some(id_a));
            assert_eq!(b.current_connection_id(), Some(id_b));
        }
    }
    for (handle, socket) in [(&a, &mut socket_a), (&b, &mut socket_b)] {
        assert_no_matching(handle, Duration::from_millis(100), |event| {
            matches!(event, BridgeEvent::Connected(_) | BridgeEvent::State(_))
        });
        let nonce = handle.ping().unwrap();
        assert_eq!(
            read_json(socket)["nonce"],
            nonce,
            "original connection was replaced"
        );
    }
    a.shutdown().unwrap();
    b.shutdown().unwrap();
}

#[test]
fn workspace_reconnect_discards_only_its_old_epoch_and_resets_revision() {
    let [(token_a, a), (token_b, b)] = workspace_bridges();
    let mut old_a = connect(&a);
    let mut socket_b = connect(&b);
    let old_id = authenticate_workspace(&a, &mut old_a, &token_a);
    let id_b = authenticate_workspace(&b, &mut socket_b, &token_b);
    for (handle, socket, name) in [(&a, &mut old_a, "old-a"), (&b, &mut socket_b, "b")] {
        for revision in [99, 100] {
            send(
                socket,
                &state(revision, Some(name), vec![session(name, name, true, None)]),
            );
        }
        assert!(wait_until(handle, Duration::from_secs(3), |handle| handle
            .coalesced_states()
            == 1));
    }
    let mut new_a = connect(&a);
    let new_id = authenticate_workspace(&a, &mut new_a, &token_a);
    assert_ne!(old_id, new_id);
    // A client can still attempt a late write on the replaced socket; it must not win.
    let _ = old_a.send(tungstenite::Message::Text(
        state(
            101,
            Some("stale-a"),
            vec![session("stale-a", "Stale", true, None)],
        )
        .to_string()
        .into(),
    ));
    assert_eq!(read_close_code(&mut old_a), Some(4409));
    send(
        &mut new_a,
        &state(1, Some("new-a"), vec![session("new-a", "New", true, None)]),
    );
    let event = wait_for(&a, |event| matches!(event, BridgeEvent::State(_)));
    assert!(
        matches!(event, BridgeEvent::State(snapshot) if snapshot.revision == 1 && snapshot.active_session_id.as_deref() == Some("new-a"))
    );
    assert_no_matching(&a, Duration::from_millis(100), |event| {
        matches!(event, BridgeEvent::State(_))
    });
    let event = wait_for(&b, |event| matches!(event, BridgeEvent::State(_)));
    assert!(
        matches!(event, BridgeEvent::State(snapshot) if snapshot.revision == 100 && snapshot.active_session_id.as_deref() == Some("b"))
    );
    assert_eq!(b.current_connection_id(), Some(id_b));
    let nonce = b.ping().unwrap();
    assert_eq!(read_json(&mut socket_b)["nonce"], nonce);
    a.shutdown().unwrap();
    b.shutdown().unwrap();
}

#[test]
fn generated_tokens_are_256_bits_of_hexadecimal() {
    let tokens: Vec<Token> = (0..64).map(|_| Token::generate().unwrap()).collect();
    let mut unique = HashSet::new();
    for token in &tokens {
        let secret = token.expose_secret();
        assert_eq!(secret.len(), TOKEN_HEX_LEN);
        assert!(secret.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(secret.bytes().all(|byte| !byte.is_ascii_uppercase()));
        assert!(unique.insert(secret.to_owned()));
    }
}

#[test]
fn token_comparison_accepts_only_the_exact_secret() {
    let token = token();
    assert!(token.verify(token.expose_secret()));
    assert!(!token.verify(&token.expose_secret().to_ascii_uppercase()));
    assert!(!token.verify(&"0".repeat(TOKEN_HEX_LEN)));
    assert!(!token.verify(&"0".repeat(TOKEN_HEX_LEN - 1)));
    assert!(!token.verify(&"0".repeat(TOKEN_HEX_LEN + 1)));
    assert!(!token.verify(""));
    assert_eq!(token, token.clone());
    assert_ne!(token, other_token());
}

#[test]
fn token_parsing_rejects_wrong_lengths_and_non_hex() {
    assert!(Token::from_hex("abc").is_err());
    assert!(Token::from_hex("z".repeat(TOKEN_HEX_LEN)).is_err());
    let uppercase = Token::from_hex("AB".repeat(32)).expect("uppercase hex accepted");
    assert_eq!(uppercase.expose_secret(), "ab".repeat(32));
    let from_bytes = Token::from_bytes([0xab; 32]);
    assert_eq!(from_bytes.expose_secret(), "ab".repeat(32));
    let parsed: Token = "01".repeat(32).parse().expect("FromStr");
    assert_eq!(parsed.expose_secret(), "01".repeat(32));
}

#[test]
fn token_debug_and_display_are_redacted() {
    let token = token();
    let secret = token.expose_secret();
    let debug = format!("{token:?}");
    let display = format!("{token}");
    assert!(!debug.contains(secret));
    assert!(!display.contains(secret));
    assert!(debug.contains(REDACTED));
    assert_eq!(display, REDACTED);
}

#[test]
fn token_fingerprint_is_stable_and_non_secret() {
    let token = token();
    let fingerprint = token.fingerprint();
    assert!(fingerprint.starts_with("sha256:"));
    assert_eq!(fingerprint.len(), "sha256:".len() + 16);
    assert!(!fingerprint.contains(token.expose_secret()));
    assert_eq!(fingerprint, token.fingerprint());
    assert_ne!(fingerprint, other_token().fingerprint());
}

#[test]
fn log_redaction_removes_every_occurrence() {
    let token = token();
    let secret = token.expose_secret();
    let text = format!("bridge token={secret} again={secret}");
    let redacted = redact_secret(&text, &token);
    assert!(!redacted.contains(secret));
    assert_eq!(redacted.matches(REDACTED).count(), 2);

    let wrapped = Redacted::new(&text, &token);
    let shown = format!("{wrapped}");
    let debug = format!("{wrapped:?}");
    assert!(!shown.contains(secret));
    assert!(!debug.contains(secret));
}

#[test]
fn loopback_url_validation_accepts_only_the_bridge_shape() {
    let accepted = [
        "ws://127.0.0.1:1234/made",
        "ws://127.0.0.2:1/made",
        "ws://127.255.255.255:65535/made",
        "WS://127.0.0.1:43210/made",
        "ws://[::1]:1234/made",
    ];
    for url in accepted {
        assert!(is_loopback_ws_url(url), "should accept {url}");
    }

    let rejected = [
        "",
        "wss://127.0.0.1:1234/made",
        "http://127.0.0.1:1234/made",
        "ws://localhost:1234/made",
        "ws://0.0.0.0:1234/made",
        "ws://192.168.1.2:1234/made",
        "ws://128.0.0.1:1234/made",
        "ws://127.0.0.256:1234/made",
        "ws://127.0.0.1/made",
        "ws://127.0.0.1:0/made",
        "ws://127.0.0.1:65536/made",
        "ws://127.0.0.1:12x/made",
        "ws://127.0.0.1:1234/",
        "ws://127.0.0.1:1234/made/",
        "ws://127.0.0.1:1234/made?x=1",
        "ws://127.0.0.1:1234/made#frag",
        "ws://user@127.0.0.1:1234/made",
        "ws://[::2]:1234/made",
        "ws://[::1]/made",
        "ws://[0:0:0:0:0:0:0:1]:1234/made",
    ];
    for url in rejected {
        assert!(!is_loopback_ws_url(url), "should reject {url}");
    }
}

#[test]
fn loopback_helpers_cover_ipv4_and_ipv6() {
    let url = loopback_ws_url(47123);
    assert_eq!(url, "ws://127.0.0.1:47123/made");
    assert!(is_loopback_ws_url(&url));

    assert!(is_loopback_socket_addr(SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        1234
    ))));
    assert!(is_loopback_socket_addr(SocketAddr::from((
        IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        1234
    ))));
    assert!(!is_loopback_socket_addr(SocketAddr::from((
        Ipv4Addr::new(192, 168, 1, 10),
        1234
    ))));
    assert!(!is_loopback_ip(IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
}

#[test]
fn bridge_binds_only_ipv4_loopback_with_an_ephemeral_port() {
    let (_token, handle) = start_bridge();
    let address = handle.local_addr();
    assert_eq!(address.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert!(address.ip().is_loopback());
    assert!(address.port() > 0);
    assert_eq!(handle.port(), address.port());
    assert!(is_loopback_socket_addr(address));
    assert!(is_loopback_ws_url(&handle.url()));
    assert_eq!(handle.url(), loopback_ws_url(address.port()));
    handle.shutdown().unwrap();
}

#[test]
fn options_defaults_and_limits_match_the_v1_contract() {
    let options = BridgeOptions::new(token(), "pane");
    assert_eq!(options.heartbeat_interval, HEARTBEAT_INTERVAL);
    assert_eq!(options.auth_timeout, AUTH_TIMEOUT);
    assert_eq!(options.focus_timeout, FOCUS_TIMEOUT);
    assert!(HEARTBEAT_INTERVAL >= Duration::from_secs(5));
    assert_eq!(MISSED_PONG_LIMIT, 3);
    assert_eq!(MAX_SESSIONS, 64);
    assert_eq!(MAX_MESSAGE_BYTES, 256 * 1024);
    assert_eq!(MAX_PENDING_FOCUS, 64);
}

#[test]
fn public_bridge_types_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<BridgeHandle>();
    assert_send_sync::<BridgeOptions>();
    assert_send_sync::<BridgeEvent>();
    assert_send_sync::<made_bridge::config::ComposedConfig>();
    assert_send_sync::<made_bridge::config_guard::ConfigSnapshot>();
    assert_send_sync::<made_bridge::state::SessionSnapshot>();
}

#[test]
fn invalid_bridge_options_are_rejected() {
    let error =
        made_bridge::server::start(BridgeOptions::new(token(), "")).expect_err("empty pane");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);

    let error = made_bridge::server::start(
        BridgeOptions::new(token(), "pane").with_auth_timeout(Duration::from_secs(2)),
    )
    .expect_err("long auth timeout");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);

    let error = made_bridge::server::start(
        BridgeOptions::new(token(), "pane").with_heartbeat_interval(Duration::ZERO),
    )
    .expect_err("zero heartbeat");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn malformed_hello_is_rejected_before_state_is_disclosed() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    send(
        &mut socket,
        &json!({
            "type": "hello",
            "protocolVersion": 1,
            "token": token.expose_secret(),
            "paneId": PANE
        }),
    );
    assert_eq!(read_close_code(&mut socket), Some(4401));

    let events = handle.drain_events();
    assert!(events.iter().any(|event| matches!(
        event,
        BridgeEvent::ProtocolError(error) if error.code == "invalid_hello"
    )));
    assert!(!events
        .iter()
        .any(|event| matches!(event, BridgeEvent::State(_) | BridgeEvent::Connected(_))));
    handle.shutdown().unwrap();
}

#[test]
fn an_authenticated_connection_is_unique_per_pane() {
    let (token, handle) = start_bridge();
    let mut first = connect(&handle);
    authenticate(&handle, &mut first, &token);
    let first_id = wait_for(&handle, |event| matches!(event, BridgeEvent::Connected(_)))
        .connection_id()
        .expect("first connection id");

    let mut second = connect(&handle);
    authenticate(&handle, &mut second, &token);
    assert_eq!(read_close_code(&mut first), Some(4409));

    let current = wait_for(
        &handle,
        |event| matches!(event, BridgeEvent::Connected(info) if info.connection_id != first_id),
    );
    let current_id = current.connection_id();
    assert_eq!(handle.current_connection_id(), current_id);
    handle.shutdown().unwrap();
}

#[test]
fn bridge_debug_output_and_events_never_contain_the_token() {
    let (token, handle) = start_bridge();
    let secret = token.expose_secret().to_owned();

    let handle_debug = format!("{handle:?}");
    assert!(!handle_debug.contains(&secret));
    assert!(handle_debug.contains(handle.token_fingerprint()));

    let options = BridgeOptions::new(token.clone(), PANE);
    let options_debug = format!("{options:?}");
    assert!(!options_debug.contains(&secret));
    assert!(options_debug.contains(REDACTED));

    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);
    let title = format!("session {secret}");
    let directory = test_path(&secret);
    send(
        &mut socket,
        &state(1, None, vec![session("a", &title, false, Some(&directory))]),
    );
    wait_for(&handle, |event| matches!(event, BridgeEvent::State(_)));
    for event in handle.drain_events() {
        let rendered = format!("{event:?}");
        assert!(!rendered.contains(&secret), "leaked token in {rendered}");
    }
    handle.shutdown().unwrap();
}

#[test]
fn unauthorized_connection_gets_no_state() {
    let (_token, handle) = start_bridge();
    let mut socket = connect(&handle);
    send(
        &mut socket,
        &json!({
            "type": "hello",
            "protocolVersion": 1,
            "token": other_token().expose_secret(),
            "paneId": PANE,
            "pluginVersion": PLUGIN_VERSION,
            "opencodeVersion": OPENCODE_VERSION,
            "pid": 1,
            "cwd": test_path("workspace")
        }),
    );
    assert_eq!(read_close_code(&mut socket), Some(4401));

    assert_no_matching(&handle, Duration::from_millis(100), |event| {
        matches!(event, BridgeEvent::State(_) | BridgeEvent::Connected(_))
    });
    handle.shutdown().unwrap();
}

#[test]
fn client_supplied_error_and_goodbye_text_is_redacted() {
    let (token, handle) = start_bridge();
    let secret = token.expose_secret().to_owned();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);

    send(
        &mut socket,
        &json!({
            "type": "error",
            "code": "plugin_error",
            "message": format!("detail {secret}"),
            "fatal": false
        }),
    );
    let error = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::ProtocolError(_))
    });
    let rendered = format!("{error:?}");
    assert!(!rendered.contains(&secret), "leaked token in {rendered}");

    send(
        &mut socket,
        &json!({"type": "goodbye", "reason": format!("unload {secret}")}),
    );
    let goodbye = wait_for(&handle, |event| matches!(event, BridgeEvent::Goodbye(_)));
    let rendered = format!("{goodbye:?}");
    assert!(!rendered.contains(&secret), "leaked token in {rendered}");
    assert_eq!(read_close_code(&mut socket), Some(1000));
    handle.shutdown().unwrap();
}

#[test]
fn client_close_reason_is_redacted_in_disconnect_event() {
    use tungstenite::{
        protocol::frame::{coding::CloseCode, CloseFrame},
        Message,
    };

    let (token, handle) = start_bridge();
    let secret = token.expose_secret().to_owned();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);

    socket
        .send(Message::Close(Some(CloseFrame {
            code: CloseCode::Normal,
            reason: format!("bye {secret}").into(),
        })))
        .expect("send close");

    let disconnected = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::Disconnected(_))
    });
    let rendered = format!("{disconnected:?}");
    assert!(!rendered.contains(&secret), "leaked token in {rendered}");
    handle.shutdown().unwrap();
}

#[test]
fn focus_failure_text_is_redacted() {
    let (token, handle) = start_bridge();
    let secret = token.expose_secret().to_owned();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);

    let request_id = handle.focus("session-a").expect("focus queued");
    let focus = read_json(&mut socket);
    assert_eq!(focus["requestID"], request_id.as_str());
    send(
        &mut socket,
        &json!({
            "type": "focus_result",
            "requestID": request_id,
            "ok": false,
            "error": format!("focus failed {secret}")
        }),
    );

    let result = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::FocusResult(_))
    });
    let rendered = format!("{result:?}");
    assert!(!rendered.contains(&secret), "leaked token in {rendered}");
    handle.shutdown().unwrap();
}

#[test]
fn handshake_requires_the_loopback_host_authority() {
    let (token, handle) = start_bridge();
    let expected = handle.local_addr().to_string();

    for host in ["evil.example", "localhost", "127.0.0.2:1", "127.0.0.1"] {
        let error = handshake_with(&handle, "/made", |request| {
            request.headers_mut().insert(
                header::HOST,
                HeaderValue::from_str(host).expect("host header"),
            );
        })
        .expect_err("non-authority host must be rejected");
        assert!(
            error.contains("403") || error.to_ascii_lowercase().contains("forbidden"),
            "unexpected rejection for {host}: {error}"
        );
    }

    let error = handshake_with(&handle, "/made", |request| {
        request.headers_mut().remove(header::HOST);
    })
    .expect_err("missing host must be rejected");
    assert!(
        error.to_ascii_lowercase().contains("host")
            || error.contains("403")
            || error.to_ascii_lowercase().contains("forbidden"),
        "unexpected missing-host rejection: {error}"
    );

    let mut socket = handshake_with(&handle, "/made", |request| {
        request.headers_mut().insert(
            header::HOST,
            HeaderValue::from_str(&expected).expect("host header"),
        );
    })
    .expect("the loopback authority is accepted");
    authenticate(&handle, &mut socket, &token);
    handle.shutdown().unwrap();
}

#[test]
fn handshake_rejects_a_request_carrying_origin() {
    let (_token, handle) = start_bridge();
    let error = handshake_with(&handle, "/made", |request| {
        request.headers_mut().insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:1234"),
        );
    })
    .expect_err("origin must be rejected");
    assert!(
        error.contains("403") || error.to_ascii_lowercase().contains("forbidden"),
        "unexpected origin rejection: {error}"
    );
    handle.shutdown().unwrap();
}

#[test]
fn byte_drip_handshake_is_closed_near_the_absolute_deadline() {
    let (_token, handle) =
        start_bridge_with(|options| options.with_auth_timeout(Duration::from_millis(250)))
            .expect("bridge starts");

    let authority = handle.local_addr().to_string();
    let request = format!("GET /made HTTP/1.1\r\nHost: {authority}\r\n");
    let mut stream = TcpStream::connect(handle.local_addr()).expect("connect to bridge");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .expect("write timeout");

    let started = Instant::now();
    for byte in request.bytes() {
        if stream.write_all(&[byte]).is_err() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
        if started.elapsed() > Duration::from_secs(2) {
            break;
        }
    }

    let mut buffer = [0_u8; 64];
    let closed = loop {
        match stream.read(&mut buffer) {
            Ok(0) => break true,
            Ok(_) => {}
            Err(error) if is_timeout(&error) => {
                if started.elapsed() > Duration::from_secs(2) {
                    break false;
                }
            }
            Err(_) => break true,
        }
    };

    let elapsed = started.elapsed();
    assert!(closed, "the byte-drip handshake was not closed");
    assert!(
        elapsed < Duration::from_secs(1),
        "handshake exceeded its deadline: {elapsed:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(100),
        "handshake closed before the deadline: {elapsed:?}"
    );
    assert!(!handle.is_connected());
    handle.shutdown().unwrap();
}

#[test]
fn byte_drip_hello_frame_is_closed_at_the_absolute_deadline() {
    let (_token, handle) =
        start_bridge_with(|options| options.with_auth_timeout(Duration::from_millis(250)))
            .expect("bridge starts");
    let mut socket = connect(&handle);
    let stream = socket.get_mut();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .expect("write timeout");

    // A masked text frame with a 100-byte payload. Only the header and a few
    // payload bytes are sent, so tungstenite keeps waiting inside `read`.
    let mut frame = vec![0x81, 0xE4, 0x01, 0x02, 0x03, 0x04];
    frame.extend(std::iter::repeat_n(0_u8, 100));

    let started = Instant::now();
    {
        let stream = socket.get_mut();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("read timeout");
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .expect("write timeout");
        for byte in frame.iter().take(20) {
            if stream.write_all(&[*byte]).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    assert_eq!(read_close_code(&mut socket), Some(4403));
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "hello frame exceeded its deadline: {elapsed:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(100),
        "hello frame closed before the deadline: {elapsed:?}"
    );
    assert!(!handle.is_connected());
    handle.shutdown().unwrap();
}

#[test]
fn hello_fields_are_redacted_in_connected_event() {
    let (token, handle) = start_bridge();
    let secret = token.expose_secret().to_owned();
    let mut socket = connect(&handle);

    let mut hello = hello(&token);
    hello["pluginVersion"] = json!(format!("0.1.0-{secret}"));
    hello["cwd"] = json!(test_path(&secret));
    send(&mut socket, &hello);
    assert_eq!(read_json(&mut socket)["type"], "welcome");

    let connected = wait_for(&handle, |event| matches!(event, BridgeEvent::Connected(_)));
    let rendered = format!("{connected:?}");
    assert!(!rendered.contains(&secret), "leaked token in {rendered}");
    handle.shutdown().unwrap();
}
