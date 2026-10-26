mod common;

use std::time::{Duration, Instant};

use common::*;
use made_bridge::{
    security::MAX_MESSAGE_BYTES,
    server::{BridgeEvent, DisconnectedInfo},
};
use serde_json::{json, Value};
use tungstenite::{Error as WsError, Message};

fn connected_info(event: BridgeEvent) -> made_bridge::server::ConnectionInfo {
    match event {
        BridgeEvent::Connected(info) => info,
        other => panic!("expected Connected, got {other:?}"),
    }
}

#[test]
fn hello_authenticates_and_welcome_matches_protocol() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);

    let welcome = authenticate(&handle, &mut socket, &token);
    assert_eq!(welcome["appVersion"], APP_VERSION);

    let info = connected_info(wait_for(&handle, |event| {
        matches!(event, BridgeEvent::Connected(_))
    }));
    assert_eq!(info.pane_id, PANE);
    assert_eq!(info.plugin_version, PLUGIN_VERSION);
    assert_eq!(info.opencode_version, OPENCODE_VERSION);
    assert_eq!(info.protocol_version, 1);
    assert_eq!(info.pid, 4321);

    send(
        &mut socket,
        &state(
            1,
            Some("session-a"),
            vec![session("session-a", "Alpha", true, Some("/tmp/alpha"))],
        ),
    );
    let snapshot = match wait_for(&handle, |event| matches!(event, BridgeEvent::State(_))) {
        BridgeEvent::State(snapshot) => snapshot,
        _ => unreachable!(),
    };
    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.active_session_id.as_deref(), Some("session-a"));
    assert!(snapshot.tabs_enabled);
    assert_eq!(snapshot.sessions.len(), 1);
    assert_eq!(snapshot.sessions[0].title, "Alpha");
    assert_eq!(
        snapshot.sessions[0].directory.as_deref(),
        Some(std::path::Path::new("/tmp/alpha"))
    );

    let _ = socket.close(None);
    handle.shutdown().unwrap();
}

#[test]
fn wrong_token_is_rejected_with_4401_before_state() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    send(&mut socket, &hello(&other_token()));

    assert_eq!(read_close_code(&mut socket), Some(4401));

    let error = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::ProtocolError(_))
    });
    match error {
        BridgeEvent::ProtocolError(error) => {
            assert_eq!(error.code, "invalid_token");
            assert!(error.fatal);
        }
        _ => unreachable!(),
    }
    let disconnected = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::Disconnected(_))
    });
    match disconnected {
        BridgeEvent::Disconnected(info) => {
            assert!(!info.authenticated);
            assert_eq!(info.code, Some(4401));
        }
        _ => unreachable!(),
    }

    assert_no_matching(&handle, Duration::from_millis(50), |event| {
        matches!(event, BridgeEvent::Connected(_) | BridgeEvent::State(_))
    });
    assert!(!handle.is_connected());
    let _ = token;
    handle.shutdown().unwrap();
}

#[test]
fn unsupported_protocol_major_is_rejected_with_4402() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    send(&mut socket, &hello_with(&token, PANE, 2, OPENCODE_VERSION));

    assert_eq!(read_close_code(&mut socket), Some(4402));
    let error = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::ProtocolError(_))
    });
    match error {
        BridgeEvent::ProtocolError(error) => assert_eq!(error.code, "protocol_mismatch"),
        _ => unreachable!(),
    }
    handle.shutdown().unwrap();
}

#[test]
fn opencode_below_v2_is_rejected_with_4402() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    send(&mut socket, &hello_with(&token, PANE, 1, "1.9.4"));

    assert_eq!(read_close_code(&mut socket), Some(4402));
    let error = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::ProtocolError(_))
    });
    match error {
        BridgeEvent::ProtocolError(error) => {
            assert_eq!(error.code, "unsupported_opencode_version");
        }
        _ => unreachable!(),
    }
    handle.shutdown().unwrap();
}

#[test]
fn pane_mismatch_is_rejected_with_4401() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    send(
        &mut socket,
        &hello_with(&token, "some-other-pane", 1, OPENCODE_VERSION),
    );

    assert_eq!(read_close_code(&mut socket), Some(4401));
    let error = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::ProtocolError(_))
    });
    match error {
        BridgeEvent::ProtocolError(error) => assert_eq!(error.code, "pane_mismatch"),
        _ => unreachable!(),
    }
    handle.shutdown().unwrap();
}

#[test]
fn missing_hello_times_out_with_4403() {
    let (_token, handle) =
        start_bridge_with(|options| options.with_auth_timeout(Duration::from_millis(80))).unwrap();
    let mut socket = connect(&handle);

    assert_eq!(read_close_code(&mut socket), Some(4403));
    let error = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::ProtocolError(_))
    });
    match error {
        BridgeEvent::ProtocolError(error) => assert_eq!(error.code, "auth_timeout"),
        _ => unreachable!(),
    }
    handle.shutdown().unwrap();
}

#[test]
fn state_before_hello_is_rejected_with_4401() {
    let (_token, handle) = start_bridge();
    let mut socket = connect(&handle);
    send(
        &mut socket,
        &state(1, None, vec![session("s", "S", false, None)]),
    );

    assert_eq!(read_close_code(&mut socket), Some(4401));
    let error = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::ProtocolError(_))
    });
    match error {
        BridgeEvent::ProtocolError(error) => assert_eq!(error.code, "hello_required"),
        _ => unreachable!(),
    }
    assert_no_matching(&handle, Duration::from_millis(50), |event| {
        matches!(event, BridgeEvent::State(_))
    });
    handle.shutdown().unwrap();
}

#[test]
fn malformed_json_closes_with_protocol_error() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);

    send_text(&mut socket, "{ definitely not json");
    assert_eq!(read_close_code(&mut socket), Some(1003));
    let error = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::ProtocolError(_))
    });
    match error {
        BridgeEvent::ProtocolError(error) => {
            assert_eq!(error.code, "malformed_json");
            assert!(error.fatal);
        }
        _ => unreachable!(),
    }
    handle.shutdown().unwrap();
}

#[test]
fn oversized_message_closes_with_1009() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);

    let oversized = "x".repeat(MAX_MESSAGE_BYTES + 1024);
    let _ = socket.send(Message::Text(oversized.into()));
    assert_eq!(read_close_code(&mut socket), Some(1009));

    let error = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::ProtocolError(_))
    });
    match error {
        BridgeEvent::ProtocolError(error) => assert_eq!(error.code, "message_too_large"),
        _ => unreachable!(),
    }
    handle.shutdown().unwrap();
}

#[test]
fn invalid_state_closes_with_1008() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);

    send(
        &mut socket,
        &state(0, None, vec![session("session-a", "Alpha", false, None)]),
    );
    assert_eq!(read_close_code(&mut socket), Some(1008));
    let error = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::ProtocolError(_))
    });
    match error {
        BridgeEvent::ProtocolError(error) => assert_eq!(error.code, "invalid_state"),
        _ => unreachable!(),
    }
    handle.shutdown().unwrap();
}

#[test]
fn stale_revisions_are_ignored_without_closing() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);

    send(
        &mut socket,
        &state(1, None, vec![session("a", "A", false, None)]),
    );
    let first = match wait_for(&handle, |event| matches!(event, BridgeEvent::State(_))) {
        BridgeEvent::State(snapshot) => snapshot,
        _ => unreachable!(),
    };
    send(
        &mut socket,
        &state(2, None, vec![session("a", "A2", false, None)]),
    );
    let second = match wait_for(&handle, |event| matches!(event, BridgeEvent::State(_))) {
        BridgeEvent::State(snapshot) => snapshot,
        _ => unreachable!(),
    };
    assert_eq!((first.revision, second.revision), (1, 2));

    send(
        &mut socket,
        &state(2, None, vec![session("a", "stale-equal", false, None)]),
    );
    send(
        &mut socket,
        &state(1, None, vec![session("a", "stale-old", false, None)]),
    );
    send(
        &mut socket,
        &state(3, None, vec![session("a", "A3", false, None)]),
    );
    let third = match wait_for(&handle, |event| matches!(event, BridgeEvent::State(_))) {
        BridgeEvent::State(snapshot) => snapshot,
        _ => unreachable!(),
    };
    assert_eq!(third.revision, 3);
    assert_eq!(third.sessions[0].title, "A3");
    assert!(handle.is_connected());
    handle.shutdown().unwrap();
}

#[test]
fn pending_states_coalesce_to_the_latest_snapshot() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);
    wait_for(&handle, |event| matches!(event, BridgeEvent::Connected(_)));

    for revision in 1..=300 {
        send(
            &mut socket,
            &state(
                revision,
                None,
                vec![session("a", &format!("Title {revision}"), false, None)],
            ),
        );
    }

    assert!(
        wait_until(&handle, Duration::from_secs(3), |handle| {
            handle.coalesced_states() >= 299
        }),
        "expected the pending snapshots to coalesce"
    );

    let events = handle.drain_events();
    let states: Vec<_> = events
        .into_iter()
        .filter_map(|event| match event {
            BridgeEvent::State(snapshot) => Some(snapshot),
            _ => None,
        })
        .collect();
    assert_eq!(states.len(), 1, "pending state must remain bounded");
    assert_eq!(states[0].revision, 300);
    assert_eq!(states[0].sessions[0].title, "Title 300");
    handle.shutdown().unwrap();
}

#[test]
fn replacement_discards_old_epoch_state_and_delivers_the_new_state() {
    let (token, handle) = start_bridge();
    let mut old = connect(&handle);
    authenticate(&handle, &mut old, &token);
    let old_id = wait_for(&handle, |event| matches!(event, BridgeEvent::Connected(_)))
        .connection_id()
        .expect("old connection id");

    for revision in 1..=300 {
        send(
            &mut old,
            &state(
                revision,
                None,
                vec![session("old", &format!("Old {revision}"), false, None)],
            ),
        );
    }
    assert!(
        wait_until(&handle, Duration::from_secs(3), |handle| {
            handle.coalesced_states() >= 299
        }),
        "old connection states were not processed"
    );

    let mut new = connect(&handle);
    authenticate(&handle, &mut new, &token);
    let new_id = handle
        .current_connection_id()
        .expect("new connection registered");
    assert_ne!(new_id, old_id);

    send(
        &mut new,
        &state(
            1,
            Some("new"),
            vec![session("new", "New", true, Some("/tmp/new"))],
        ),
    );

    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let now = Instant::now();
        assert!(now < deadline, "timed out waiting for the new state");
        let event = handle
            .recv_timeout(deadline - now)
            .expect("new epoch event");
        let done = matches!(
            &event,
            BridgeEvent::State(snapshot)
                if snapshot.sessions.iter().any(|session| session.session_id == "new")
        );
        events.push(event);
        if done {
            break;
        }
    }

    let new_connect = events
        .iter()
        .position(
            |event| matches!(event, BridgeEvent::Connected(info) if info.connection_id == new_id),
        )
        .expect("new Connected event");
    let new_state = events
        .iter()
        .position(|event| {
            matches!(
                event,
                BridgeEvent::State(snapshot)
                    if snapshot.sessions.iter().any(|session| session.session_id == "new")
            )
        })
        .expect("new State event");
    assert!(new_connect < new_state, "Connected must precede its state");
    assert!(
        !events[new_connect..].iter().any(|event| matches!(
            event,
            BridgeEvent::State(snapshot)
                if snapshot.sessions.iter().any(|session| session.session_id == "old")
        )),
        "old-epoch state arrived after the new Connected: {events:?}"
    );
    assert!(handle.dropped_events() > 0, "old state should be discarded");
    handle.shutdown().unwrap();
}

#[test]
fn final_state_and_lifecycle_survive_event_queue_pressure() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);

    // Authenticated protocol errors flood the priority part of the queue
    // without blocking the connection thread. Older errors are discarded to
    // make room, but Connected and the final state must survive.
    for index in 0..400 {
        send(
            &mut socket,
            &json!({
                "type": "error",
                "code": "pressure",
                "message": format!("pressure {index}"),
                "fatal": false
            }),
        );
    }
    assert!(
        wait_until(&handle, Duration::from_secs(3), |handle| {
            handle.dropped_events() > 0
        }),
        "event queue did not report pressure"
    );

    send(
        &mut socket,
        &state(
            1,
            Some("final"),
            vec![session("final", "Final", true, Some("/tmp/final"))],
        ),
    );
    send(
        &mut socket,
        &json!({"type": "goodbye", "reason": "test unload"}),
    );
    assert_eq!(read_close_code(&mut socket), Some(1000));

    let events = handle.drain_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, BridgeEvent::Connected(_))),
        "Connected was dropped under pressure: {} events",
        events.len()
    );
    let state_position = events
        .iter()
        .position(|event| matches!(event, BridgeEvent::State(_)))
        .expect("final state");
    let disconnect_position = events
        .iter()
        .rposition(|event| matches!(event, BridgeEvent::Disconnected(_)))
        .expect("disconnect");
    assert!(
        state_position < disconnect_position,
        "final state must precede Disconnected"
    );
    match &events[state_position] {
        BridgeEvent::State(snapshot) => {
            assert_eq!(snapshot.revision, 1);
            assert_eq!(snapshot.sessions[0].session_id, "final");
        }
        _ => unreachable!(),
    }
    handle.shutdown().unwrap();
}

#[test]
fn shutdown_returns_promptly_while_the_event_queue_is_saturated() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);

    for index in 0..400 {
        send(
            &mut socket,
            &json!({
                "type": "error",
                "code": "pressure",
                "message": format!("pressure {index}"),
                "fatal": false
            }),
        );
    }
    assert!(
        wait_until(&handle, Duration::from_secs(3), |handle| {
            handle.dropped_events() > 0
        }),
        "event queue did not report pressure"
    );

    let started = Instant::now();
    handle.shutdown().unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "shutdown blocked for {elapsed:?}"
    );
}

#[test]
fn unauth_flood_cannot_fill_the_authenticated_event_queue() {
    let (token, handle) = start_bridge();

    for _ in 0..12 {
        let mut socket = connect(&handle);
        send(&mut socket, &hello(&other_token()));
        assert_eq!(read_close_code(&mut socket), Some(4401));
    }

    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);
    send(
        &mut socket,
        &state(
            1,
            Some("auth"),
            vec![session("auth", "Authenticated", true, Some("/tmp/auth"))],
        ),
    );

    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    let state = loop {
        let now = Instant::now();
        assert!(now < deadline, "timed out waiting for authenticated state");
        let event = handle.recv_timeout(deadline - now).expect("event");
        let state = match &event {
            BridgeEvent::State(snapshot) => Some(snapshot.clone()),
            _ => None,
        };
        events.push(event);
        if let Some(snapshot) = state {
            break snapshot;
        }
    };
    assert_eq!(state.sessions[0].session_id, "auth");
    assert!(events
        .iter()
        .any(|event| matches!(event, BridgeEvent::Connected(_))));
    let unauth_errors = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                BridgeEvent::ProtocolError(error) if error.code == "invalid_token"
            )
        })
        .count();
    assert!(
        unauth_errors <= made_bridge::security::MAX_UNAUTH_EVENT_NOTICES,
        "unauthenticated traffic exceeded its queue budget: {unauth_errors}"
    );
    assert!(handle.dropped_events() > 0);
    handle.shutdown().unwrap();
}

#[test]
fn active_session_disagreement_is_rejected_on_the_wire() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);

    send(
        &mut socket,
        &state(
            1,
            Some("a"),
            vec![
                session("a", "A", false, None),
                session("b", "B", true, None),
            ],
        ),
    );
    assert_eq!(read_close_code(&mut socket), Some(1008));
    let error = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::ProtocolError(_))
    });
    match error {
        BridgeEvent::ProtocolError(error) => assert_eq!(error.code, "invalid_state"),
        _ => unreachable!(),
    }
    handle.shutdown().unwrap();
}

#[test]
fn invalid_paths_and_project_ids_are_rejected_on_the_wire() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);

    send(
        &mut socket,
        &state(
            1,
            None,
            vec![session("a", "A", false, Some("/tmp/../escape"))],
        ),
    );
    assert_eq!(read_close_code(&mut socket), Some(1008));
    let error = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::ProtocolError(_))
    });
    match error {
        BridgeEvent::ProtocolError(error) => assert_eq!(error.code, "invalid_state"),
        _ => unreachable!(),
    }

    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);
    let mut session = session("a", "A", false, Some("/tmp/ok"));
    session["projectID"] = json!("p".repeat(129));
    send(&mut socket, &state(1, None, vec![session]));
    assert_eq!(read_close_code(&mut socket), Some(1008));
    handle.shutdown().unwrap();
}

#[test]
fn focus_result_and_control_commands_are_correlated() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);

    let request_id = handle.focus("session-a").expect("focus queued");
    let focus = read_json(&mut socket);
    assert_eq!(focus["type"], "focus");
    assert_eq!(focus["requestID"], request_id.as_str());
    assert_eq!(focus["sessionID"], "session-a");

    send(
        &mut socket,
        &json!({
            "type": "focus_result",
            "requestID": request_id,
            "ok": true
        }),
    );
    let result = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::FocusResult(_))
    });
    match result {
        BridgeEvent::FocusResult(result) => {
            assert_eq!(result.request_id, request_id);
            assert_eq!(result.session_id.as_deref(), Some("session-a"));
            assert!(result.ok);
            assert!(result.error.is_none());
        }
        _ => unreachable!(),
    }

    let failed_id = handle.focus("missing-session").expect("focus queued");
    let focus = read_json(&mut socket);
    assert_eq!(focus["requestID"], failed_id.as_str());
    send(
        &mut socket,
        &json!({
            "type": "focus_result",
            "requestID": failed_id,
            "ok": false,
            "error": "session is not open"
        }),
    );
    let result = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::FocusResult(_))
    });
    match result {
        BridgeEvent::FocusResult(result) => {
            assert_eq!(result.request_id, failed_id);
            assert!(!result.ok);
            assert_eq!(result.error.as_deref(), Some("session is not open"));
        }
        _ => unreachable!(),
    }

    let state_request = handle.request_state().expect("state request queued");
    let request = read_json(&mut socket);
    assert_eq!(request["type"], "request_state");
    assert_eq!(request["requestID"], state_request.as_str());

    let nonce = handle.ping().expect("ping queued");
    let ping = read_json(&mut socket);
    assert_eq!(ping["type"], "ping");
    assert_eq!(ping["nonce"], nonce.as_str());

    handle.shutdown().unwrap();
}

#[test]
fn newer_connection_replaces_older_with_4409_in_event_order() {
    let (token, handle) = start_bridge();
    let mut first_socket = connect(&handle);
    authenticate(&handle, &mut first_socket, &token);
    let first = connected_info(wait_for(&handle, |event| {
        matches!(event, BridgeEvent::Connected(_))
    }));

    let mut second_socket = connect(&handle);
    authenticate(&handle, &mut second_socket, &token);

    assert_eq!(read_close_code(&mut first_socket), Some(4409));

    let mut events = Vec::new();
    let second_id = loop {
        let event = handle
            .recv_timeout(Duration::from_secs(3))
            .expect("replacement events");
        let connected = match &event {
            BridgeEvent::Connected(info) if info.connection_id != first.connection_id => {
                Some(info.connection_id)
            }
            _ => None,
        };
        events.push(event);
        if let Some(id) = connected {
            break id;
        }
    };

    let old_disconnect = events.iter().position(|event| {
        matches!(
            event,
            BridgeEvent::Disconnected(DisconnectedInfo {
                connection_id,
                replaced: true,
                ..
            }) if *connection_id == first.connection_id
        )
    });
    let new_connect = events.iter().position(|event| {
        matches!(
            event,
            BridgeEvent::Connected(info) if info.connection_id == second_id
        )
    });
    assert!(
        old_disconnect.is_some(),
        "missing replaced disconnect: {events:?}"
    );
    assert!(
        new_connect.is_some(),
        "missing replacement connect: {events:?}"
    );
    assert!(old_disconnect < new_connect);

    send(
        &mut second_socket,
        &state(1, None, vec![session("b", "B", false, None)]),
    );
    let snapshot = match wait_for(&handle, |event| matches!(event, BridgeEvent::State(_))) {
        BridgeEvent::State(snapshot) => snapshot,
        _ => unreachable!(),
    };
    assert_eq!(snapshot.revision, 1);
    assert_eq!(handle.current_connection_id(), Some(second_id));
    handle.shutdown().unwrap();
}

#[test]
fn goodbye_is_emitted_then_connection_closes() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);

    send(
        &mut socket,
        &json!({"type": "goodbye", "reason": "plugin unloaded"}),
    );
    assert_eq!(read_close_code(&mut socket), Some(1000));

    let goodbye = wait_for(&handle, |event| matches!(event, BridgeEvent::Goodbye(_)));
    match goodbye {
        BridgeEvent::Goodbye(goodbye) => assert_eq!(goodbye.reason, "plugin unloaded"),
        _ => unreachable!(),
    }
    let disconnected = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::Disconnected(_))
    });
    match disconnected {
        BridgeEvent::Disconnected(info) => {
            assert!(info.authenticated);
            assert_eq!(info.code, Some(1000));
        }
        _ => unreachable!(),
    }
    handle.shutdown().unwrap();
}

#[test]
fn unknown_fields_and_message_types_are_ignored() {
    let (token, handle) = start_bridge();
    let mut socket = connect(&handle);

    let mut hello = hello(&token);
    hello["futureField"] = json!({"nested": true});
    send(&mut socket, &hello);
    let welcome = read_json(&mut socket);
    assert_eq!(welcome["type"], "welcome");

    send_text(&mut socket, r#"{"type":"future_message","payload":42}"#);

    let mut session = session("s", "S", false, None);
    session["futureSessionField"] = json!([1, 2, 3]);
    let mut state = state(1, None, vec![session]);
    state["futureTopLevel"] = json!("ignored");
    send(&mut socket, &state);

    let snapshot = match wait_for(&handle, |event| matches!(event, BridgeEvent::State(_))) {
        BridgeEvent::State(snapshot) => snapshot,
        _ => unreachable!(),
    };
    assert_eq!(snapshot.revision, 1);
    assert!(handle.is_connected());
    handle.shutdown().unwrap();
}

#[test]
fn non_made_websocket_path_is_rejected() {
    let (_token, handle) = start_bridge();
    let error = expect_handshake_rejected(&handle, "/not-made");
    assert!(
        error.contains("404") || error.to_ascii_lowercase().contains("http"),
        "unexpected handshake error: {error}"
    );
    handle.shutdown().unwrap();
}

#[test]
fn heartbeat_times_out_after_three_missed_pongs() {
    let (_token, handle) =
        start_bridge_with(|options| options.with_heartbeat_interval(Duration::from_millis(40)))
            .unwrap();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &_token);

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut pings = 0_usize;
    let close_code = loop {
        match socket.read() {
            Ok(Message::Text(text)) => {
                let value: Value = serde_json::from_str(text.as_str()).expect("json");
                if value["type"] == "ping" {
                    pings += 1;
                }
            }
            Ok(Message::Close(frame)) => break frame.map(|frame| u16::from(frame.code)),
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
            Ok(Message::Binary(_)) => panic!("unexpected binary frame"),
            Err(WsError::Io(error)) if is_timeout(&error) => {
                assert!(Instant::now() < deadline, "heartbeat timeout not observed");
            }
            Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => break None,
            Err(error) => panic!("read failed: {error}"),
        }
    };

    assert_eq!(close_code, Some(1001));
    assert!(pings >= 3, "expected at least three pings, got {pings}");
    let timeout = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::HeartbeatTimeout(_))
    });
    match timeout {
        BridgeEvent::HeartbeatTimeout(timeout) => {
            assert_eq!(timeout.missed_pongs, 3);
            assert_eq!(timeout.connection_id, 1);
        }
        _ => unreachable!(),
    }
    handle.shutdown().unwrap();
}

#[test]
fn pongs_keep_the_heartbeat_alive() {
    let (token, handle) =
        start_bridge_with(|options| options.with_heartbeat_interval(Duration::from_millis(100)))
            .unwrap();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);

    for _ in 0..4 {
        let ping = read_json(&mut socket);
        assert_eq!(ping["type"], "ping");
        send(
            &mut socket,
            &json!({"type": "pong", "nonce": ping["nonce"].clone()}),
        );
    }
    assert!(handle.is_connected());
    assert_no_matching(&handle, Duration::from_millis(150), |event| {
        matches!(event, BridgeEvent::HeartbeatTimeout(_))
    });
    handle.shutdown().unwrap();
}

#[test]
fn rapid_replacement_chain_leaves_only_the_newest_connection() {
    let (token, handle) = start_bridge();
    let mut first = connect(&handle);
    authenticate(&handle, &mut first, &token);
    let first_id = connected_info(wait_for(&handle, |event| {
        matches!(event, BridgeEvent::Connected(_))
    }))
    .connection_id;

    let mut second = connect(&handle);
    authenticate(&handle, &mut second, &token);

    let mut third = connect(&handle);
    authenticate(&handle, &mut third, &token);

    assert_eq!(read_close_code(&mut first), Some(4409));
    assert_eq!(read_close_code(&mut second), Some(4409));

    let third_id = handle.current_connection_id().expect("third connection id");
    assert_ne!(third_id, first_id);
    wait_for(
        &handle,
        |event| matches!(event, BridgeEvent::Connected(info) if info.connection_id == third_id),
    );

    send(
        &mut third,
        &state(1, None, vec![session("newest", "Newest", false, None)]),
    );
    let snapshot = match wait_for(&handle, |event| matches!(event, BridgeEvent::State(_))) {
        BridgeEvent::State(snapshot) => snapshot,
        _ => unreachable!(),
    };
    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.sessions[0].session_id, "newest");
    handle.shutdown().unwrap();
}

#[test]
fn unanswered_focus_request_times_out_with_a_correlated_failure() {
    let (token, handle) =
        start_bridge_with(|options| options.with_focus_timeout(Duration::from_millis(80))).unwrap();
    let mut socket = connect(&handle);
    authenticate(&handle, &mut socket, &token);

    let request_id = handle.focus("session-a").expect("focus queued");
    let focus = read_json(&mut socket);
    assert_eq!(focus["requestID"], request_id.as_str());

    let result = wait_for(&handle, |event| {
        matches!(event, BridgeEvent::FocusResult(_))
    });
    match result {
        BridgeEvent::FocusResult(result) => {
            assert_eq!(result.request_id, request_id);
            assert_eq!(result.session_id.as_deref(), Some("session-a"));
            assert!(!result.ok);
            assert!(result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("timed out")));
        }
        _ => unreachable!(),
    }
    assert!(handle.is_connected());
    handle.shutdown().unwrap();
}
