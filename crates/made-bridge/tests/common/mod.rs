#![allow(dead_code)]

use std::{
    net::TcpStream,
    thread,
    time::{Duration, Instant},
};

use made_bridge::{
    security::Token,
    server::{start, BridgeEvent, BridgeHandle, BridgeOptions},
};
use serde_json::{json, Value};
use tungstenite::{Error as WsError, Message, WebSocket};

pub const PANE: &str = "pane-test";
pub const PLUGIN_VERSION: &str = "0.1.0-test";
pub const OPENCODE_VERSION: &str = "2.1.4";
pub const APP_VERSION: &str = "0.1.0-test";

pub fn token() -> Token {
    // 64 lowercase hex characters = 256 bits.
    Token::from_hex("0123456789abcdef".repeat(4)).expect("fixture token")
}

pub fn other_token() -> Token {
    Token::from_hex("fedcba9876543210".repeat(4)).expect("fixture token")
}

pub fn start_bridge() -> (Token, BridgeHandle) {
    start_bridge_with(|options| options).expect("bridge starts")
}

pub fn start_bridge_with(
    configure: impl FnOnce(BridgeOptions) -> BridgeOptions,
) -> Result<(Token, BridgeHandle), std::io::Error> {
    let token = token();
    let options = configure(BridgeOptions::new(token.clone(), PANE).with_app_version(APP_VERSION));
    let handle = start(options)?;
    Ok((token, handle))
}

pub fn connect(handle: &BridgeHandle) -> WebSocket<TcpStream> {
    connect_path(handle, "/made")
}

pub fn connect_path(handle: &BridgeHandle, path: &str) -> WebSocket<TcpStream> {
    let stream = TcpStream::connect(handle.local_addr()).expect("connect to bridge");
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .expect("write timeout");
    let url = format!("ws://{}{}", handle.local_addr(), path);
    let (mut socket, _) = tungstenite::client(url, stream).expect("websocket handshake");
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("read timeout");
    socket
}

pub fn expect_handshake_rejected(handle: &BridgeHandle, path: &str) -> String {
    match handshake_with(handle, path, |_| {}) {
        Ok(_) => panic!("handshake unexpectedly succeeded"),
        Err(error) => error,
    }
}

/// Perform a client WebSocket handshake after letting `configure` adjust the
/// request headers.
pub fn handshake_with(
    handle: &BridgeHandle,
    path: &str,
    configure: impl FnOnce(&mut tungstenite::http::Request<()>),
) -> Result<WebSocket<TcpStream>, String> {
    use tungstenite::client::IntoClientRequest;

    let url = format!("ws://{}{}", handle.local_addr(), path);
    let mut request = url.into_client_request().expect("client request");
    configure(&mut request);

    let stream = TcpStream::connect(handle.local_addr()).expect("connect to bridge");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .expect("write timeout");
    tungstenite::client(request, stream)
        .map(|(socket, _response)| socket)
        .map_err(|error| error.to_string())
}

pub fn hello(token: &Token) -> Value {
    hello_with(token, PANE, 1, OPENCODE_VERSION)
}

pub fn hello_with(
    token: &Token,
    pane: &str,
    protocol_version: u32,
    opencode_version: &str,
) -> Value {
    json!({
        "type": "hello",
        "protocolVersion": protocol_version,
        "token": token.expose_secret(),
        "paneId": pane,
        "pluginVersion": PLUGIN_VERSION,
        "opencodeVersion": opencode_version,
        "pid": 4321,
        "cwd": "/tmp/made-test-workspace"
    })
}

pub fn send(socket: &mut WebSocket<TcpStream>, value: &Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .expect("send json");
}

pub fn send_text(socket: &mut WebSocket<TcpStream>, text: &str) {
    socket
        .send(Message::Text(text.to_owned().into()))
        .expect("send text");
}

pub fn authenticate(
    handle: &BridgeHandle,
    socket: &mut WebSocket<TcpStream>,
    token: &Token,
) -> Value {
    send(socket, &hello(token));
    let welcome = read_json(socket);
    assert_eq!(welcome["type"], "welcome");
    assert_eq!(welcome["protocolVersion"], 1);
    assert_eq!(welcome["appVersion"], APP_VERSION);
    assert!(welcome["heartbeatIntervalMs"].as_u64().unwrap_or(0) > 0);
    let _ = handle;
    welcome
}

pub fn session(id: &str, title: &str, active: bool, directory: Option<&str>) -> Value {
    json!({
        "sessionID": id,
        "title": title,
        "active": active,
        "busy": false,
        "attention": false,
        "unread": false,
        "promptPulse": 0,
        "renaming": false,
        "directory": directory,
        "subpath": null,
        "projectID": "project-1"
    })
}

pub fn state(revision: u64, active: Option<&str>, sessions: Vec<Value>) -> Value {
    json!({
        "type": "state",
        "revision": revision,
        "activeSessionID": active,
        "tabsEnabled": true,
        "sessions": sessions
    })
}

pub fn read_json(socket: &mut WebSocket<TcpStream>) -> Value {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => {
                return serde_json::from_str(text.as_str()).expect("server sent JSON");
            }
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {
                let _ = socket.flush();
            }
            Ok(Message::Binary(_)) => panic!("server sent an unexpected binary message"),
            Ok(Message::Close(frame)) => panic!("server closed while awaiting JSON: {frame:?}"),
            Err(WsError::Io(error)) if is_timeout(&error) => {
                if Instant::now() >= deadline {
                    panic!("timed out waiting for a JSON message");
                }
            }
            Err(error) => panic!("read failed while awaiting JSON: {error}"),
        }
    }
}

pub fn read_close_code(socket: &mut WebSocket<TcpStream>) -> Option<u16> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match socket.read() {
            Ok(Message::Close(frame)) => {
                return frame.map(|frame| u16::from(frame.code));
            }
            Ok(Message::Text(_))
            | Ok(Message::Binary(_))
            | Ok(Message::Ping(_))
            | Ok(Message::Pong(_))
            | Ok(Message::Frame(_)) => {}
            Err(WsError::Io(error)) if is_timeout(&error) => {
                if Instant::now() >= deadline {
                    panic!("timed out waiting for a close frame");
                }
            }
            Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => {
                return None;
            }
            Err(error) => panic!("read failed while awaiting close: {error}"),
        }
    }
}

pub fn wait_for(handle: &BridgeHandle, predicate: impl Fn(&BridgeEvent) -> bool) -> BridgeEvent {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let now = Instant::now();
        assert!(now < deadline, "timed out waiting for bridge event");
        match handle.recv_timeout(deadline - now) {
            Ok(event) => {
                if predicate(&event) {
                    return event;
                }
            }
            Err(error) => panic!("bridge event wait failed: {error}"),
        }
    }
}

/// Poll `predicate` until it is true or `timeout` expires.
pub fn wait_until(
    handle: &BridgeHandle,
    timeout: Duration,
    predicate: impl Fn(&BridgeHandle) -> bool,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if predicate(handle) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(2));
    }
}

pub fn assert_no_matching(
    handle: &BridgeHandle,
    duration: Duration,
    predicate: impl Fn(&BridgeEvent) -> bool,
) {
    let deadline = Instant::now() + duration;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        match handle.recv_timeout(deadline - now) {
            Ok(event) => {
                if predicate(&event) {
                    panic!("unexpected bridge event: {event:?}");
                }
            }
            Err(_) => return,
        }
    }
}

pub fn is_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::Interrupted
    )
}
