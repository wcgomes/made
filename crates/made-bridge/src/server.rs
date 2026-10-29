//! Loopback bridge server and protocol state machine.
//!
//! [`start`] binds an IPv4 loopback listener on an OS-assigned port and
//! returns a [`BridgeHandle`] immediately. All socket I/O runs on background
//! threads; the handle only enqueues commands and exposes a
//! [`std::sync::mpsc::Receiver`]-style event API of [`BridgeEvent`] values.
//!
//! Security properties enforced here:
//!
//! - the listener is always bound to `127.0.0.1:0`;
//! - the WebSocket path must be exactly `/made`;
//! - the `Host` header must name the loopback authority the socket was
//!   accepted on, and a request carrying an `Origin` header is rejected;
//! - a valid `hello` is required within the absolute authentication deadline,
//!   which starts when the socket is accepted and includes the WebSocket
//!   handshake;
//! - the token comparison is constant time and no state is emitted before
//!   authentication;
//! - messages larger than [`MAX_MESSAGE_BYTES`] are closed with code `1009`;
//! - malformed JSON is closed with a protocol error;
//! - at most one authenticated connection exists per bridge, and a newer
//!   authenticated connection replaces the older one with close code `4409`.
//!
//! Close codes for authentication failures are distinct: `4401` for an
//! invalid token, pane, or malformed hello; `4402` for a protocol or OpenCode
//! version mismatch; and `4403` when the handshake and hello deadline expires.
//!
//! Event delivery is non-blocking for every socket and accept thread. State
//! snapshots are coalesced to the latest snapshot per connection epoch, and
//! lifecycle events for authenticated connections are retained ahead of
//! best-effort notifications. See [`BridgeHandle`] for the overflow counters.
//!
//! The per-instance token is never stored in an event, included in an error,
//! or rendered by `Debug`.

use std::{
    collections::{HashMap, VecDeque},
    fmt,
    io::{self, Read, Write},
    net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError},
        Arc, Mutex, MutexGuard,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::Value;
use tungstenite::{
    handshake::{
        server::{ErrorResponse, Request, Response},
        HandshakeError,
    },
    http::{header, StatusCode},
    protocol::{
        frame::{coding::CloseCode, CloseFrame},
        WebSocket, WebSocketConfig,
    },
    Error as WsError, Message,
};

use crate::{
    events::EventQueue,
    protocol::{
        bounded_error_code, bounded_error_message, bounded_goodbye_reason, opencode_major,
        truncate_chars, ErrorMessage, FocusMessage, FocusResultMessage, GoodbyeMessage,
        HelloMessage, PingMessage, PongMessage, RequestStateMessage, StateMessage, WelcomeMessage,
        MADE_PATH, PROTOCOL_VERSION,
    },
    security::{
        is_loopback_socket_addr, loopback_ws_url, redact_secret, Token, AUTH_TIMEOUT,
        COMMAND_QUEUE_CAPACITY, FOCUS_TIMEOUT, HEARTBEAT_INTERVAL, MAX_APP_VERSION_CHARS,
        MAX_ERROR_CHARS, MAX_MESSAGE_BYTES, MAX_PANE_ID_CHARS, MAX_PENDING_CONNECTIONS,
        MAX_PENDING_FOCUS, MAX_SESSION_ID_CHARS, MISSED_PONG_LIMIT, REDACTED,
    },
    state::{SessionSnapshot, SnapshotError, SnapshotTracker},
};

const REPLACEMENT_WAIT: Duration = Duration::from_millis(500);
const SOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const CLOSE_DRAIN_TIMEOUT: Duration = Duration::from_millis(100);
const CLOSE_DRAIN_TOTAL: Duration = Duration::from_millis(250);
const REPLACEMENT_CLOSE_DRAIN_TIMEOUT: Duration = Duration::from_millis(100);
const REPLACEMENT_CLOSE_DRAIN_TOTAL: Duration = Duration::from_millis(750);
const ACCEPT_POLL: Duration = Duration::from_millis(10);
const AUTH_READ_SLICE: Duration = Duration::from_millis(100);
const HANDSHAKE_POLL: Duration = Duration::from_millis(2);

/// Options for [`start`].
#[derive(Clone)]
pub struct BridgeOptions {
    /// Per-instance bridge secret.
    pub token: Token,
    /// Terminal pane identity expected in `hello.paneId`.
    pub pane_id: String,
    /// Application version sent in `welcome.appVersion`.
    pub app_version: String,
    /// Interval between heartbeat pings. Defaults to five seconds.
    pub heartbeat_interval: Duration,
    /// Absolute deadline for the WebSocket handshake and a valid hello,
    /// measured from the accepted socket. Defaults to one second and may only
    /// be shortened for tests.
    pub auth_timeout: Duration,
    /// Deadline for a focus result before the bridge reports a failure.
    /// Defaults to five seconds.
    pub focus_timeout: Duration,
}

impl BridgeOptions {
    /// Build options with the production defaults.
    pub fn new(token: Token, pane_id: impl Into<String>) -> Self {
        Self {
            token,
            pane_id: pane_id.into(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            heartbeat_interval: HEARTBEAT_INTERVAL,
            auth_timeout: AUTH_TIMEOUT,
            focus_timeout: FOCUS_TIMEOUT,
        }
    }

    /// Override the version reported to the plugin.
    pub fn with_app_version(mut self, version: impl Into<String>) -> Self {
        self.app_version = version.into();
        self
    }

    /// Override the heartbeat interval. Intended for tests; the v1 default is
    /// [`HEARTBEAT_INTERVAL`].
    pub fn with_heartbeat_interval(mut self, interval: Duration) -> Self {
        self.heartbeat_interval = interval;
        self
    }

    /// Override the absolute handshake-and-hello deadline. The value must be
    /// greater than zero and no longer than [`AUTH_TIMEOUT`]; this method
    /// exists for tests.
    pub fn with_auth_timeout(mut self, timeout: Duration) -> Self {
        self.auth_timeout = timeout;
        self
    }

    /// Override the focus result deadline. Intended for tests; the v1 default
    /// is [`FOCUS_TIMEOUT`].
    pub fn with_focus_timeout(mut self, timeout: Duration) -> Self {
        self.focus_timeout = timeout;
        self
    }

    /// Borrow the bridge token.
    pub fn token(&self) -> &Token {
        &self.token
    }

    fn validate(&self) -> io::Result<()> {
        if self.pane_id.is_empty() || self.pane_id.chars().count() > MAX_PANE_ID_CHARS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "pane id must be non-empty and at most 128 characters",
            ));
        }
        if self.app_version.is_empty() || self.app_version.chars().count() > MAX_APP_VERSION_CHARS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "app version must be non-empty and at most 128 characters",
            ));
        }
        if self.heartbeat_interval.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "heartbeat interval must be greater than zero",
            ));
        }
        if self.auth_timeout.is_zero() || self.auth_timeout > AUTH_TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "authentication timeout must be in (0, 1s]",
            ));
        }
        if self.focus_timeout.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "focus timeout must be greater than zero",
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for BridgeOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BridgeOptions")
            .field("token", &REDACTED)
            .field("pane_id", &self.pane_id)
            .field("app_version", &self.app_version)
            .field("heartbeat_interval", &self.heartbeat_interval)
            .field("auth_timeout", &self.auth_timeout)
            .field("focus_timeout", &self.focus_timeout)
            .finish()
    }
}

/// Identity of a successfully authenticated plugin connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionInfo {
    /// Bridge-local connection number. Changes for every new socket.
    pub connection_id: u64,
    /// Pane reported by the plugin.
    pub pane_id: String,
    /// Plugin package version.
    pub plugin_version: String,
    /// OpenCode version.
    pub opencode_version: String,
    /// Plugin process id.
    pub pid: i64,
    /// Plugin launch working directory. Not a stable session location.
    pub cwd: PathBuf,
    /// Negotiated protocol version.
    pub protocol_version: u32,
}

/// A correlated focus outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusResult {
    /// Correlation identifier returned by [`BridgeHandle::focus`].
    pub request_id: String,
    /// Session the request targeted, when the request was bridge-initiated.
    pub session_id: Option<String>,
    /// Whether the plugin activated the session.
    pub ok: bool,
    /// Failure detail, when available.
    pub error: Option<String>,
}

/// Why an accepted socket disconnected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisconnectedInfo {
    /// Bridge-local connection number.
    pub connection_id: u64,
    /// Whether the connection completed authentication.
    pub authenticated: bool,
    /// WebSocket close code, when one was observed or sent.
    pub code: Option<u16>,
    /// Human-readable reason. Never contains the token.
    pub reason: String,
    /// True when a newer authenticated connection replaced this one.
    pub replaced: bool,
}

/// A bridge-side or plugin-side protocol failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError {
    /// Connection involved, when known.
    pub connection_id: Option<u64>,
    /// Stable machine-readable code.
    pub code: String,
    /// Bounded, token-redacted detail.
    pub message: String,
    /// Whether the connection was (or will be) closed.
    pub fatal: bool,
}

/// Heartbeat failure detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatTimeout {
    /// Connection that stopped answering.
    pub connection_id: u64,
    /// Number of unanswered heartbeat pings.
    pub missed_pongs: u32,
}

/// Clean plugin unload notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Goodbye {
    /// Connection that said goodbye.
    pub connection_id: u64,
    /// Reason supplied by the plugin.
    pub reason: String,
}

/// Event delivered to the application event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeEvent {
    /// A plugin authenticated and received `welcome`.
    Connected(ConnectionInfo),
    /// A validated, non-stale session snapshot.
    State(SessionSnapshot),
    /// A focus outcome, correlated by `request_id`.
    FocusResult(FocusResult),
    /// An accepted socket disconnected.
    Disconnected(DisconnectedInfo),
    /// A protocol, authentication, or plugin-reported error.
    ProtocolError(ProtocolError),
    /// Heartbeat pings went unanswered.
    HeartbeatTimeout(HeartbeatTimeout),
    /// The plugin reported a clean unload.
    Goodbye(Goodbye),
}

impl BridgeEvent {
    /// The connection associated with this event, when it has one.
    pub fn connection_id(&self) -> Option<u64> {
        match self {
            Self::Connected(info) => Some(info.connection_id),
            Self::State(_) => None,
            Self::FocusResult(_) => None,
            Self::Disconnected(info) => Some(info.connection_id),
            Self::ProtocolError(error) => error.connection_id,
            Self::HeartbeatTimeout(timeout) => Some(timeout.connection_id),
            Self::Goodbye(goodbye) => Some(goodbye.connection_id),
        }
    }
}

/// Handle to a running bridge.
///
/// The handle is `Send + Sync`. It never blocks on network I/O:
/// [`focus`](Self::focus), [`request_state`](Self::request_state), and
/// [`ping`](Self::ping) use a bounded non-blocking command queue, and events
/// are read with [`try_recv`](Self::try_recv) or
/// [`recv_timeout`](Self::recv_timeout).
///
/// Event delivery is bounded and non-blocking for the bridge threads. Pending
/// state snapshots are coalesced to the latest accepted snapshot per
/// connection epoch, lifecycle events for authenticated connections are
/// retained ahead of best-effort notifications, and unauthenticated notices
/// have a small budget of their own. Drain the queue every frame; if the
/// consumer stalls, [`dropped_events`](Self::dropped_events) reports how many
/// low-priority events were discarded and
/// [`coalesced_states`](Self::coalesced_states) reports how many snapshots
/// were superseded.
pub struct BridgeHandle {
    local_addr: SocketAddr,
    pane_id: String,
    token_fingerprint: String,
    shared: Arc<Shared>,
    accept_thread: Mutex<Option<JoinHandle<()>>>,
    shutdown_requested: AtomicBool,
}

impl BridgeHandle {
    /// The bound loopback address.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The bound port.
    pub fn port(&self) -> u16 {
        self.local_addr.port()
    }

    /// The canonical bridge URL (`ws://127.0.0.1:<port>/made`).
    pub fn url(&self) -> String {
        loopback_ws_url(self.local_addr.port())
    }

    /// The pane identity this bridge accepts.
    pub fn pane_id(&self) -> &str {
        &self.pane_id
    }

    /// A safe SHA-256 fingerprint of the token for diagnostics.
    pub fn token_fingerprint(&self) -> &str {
        &self.token_fingerprint
    }

    /// Whether an authenticated plugin connection is currently registered and
    /// accepting commands.
    pub fn is_connected(&self) -> bool {
        !self.shutdown_requested.load(Ordering::Acquire)
            && lock(&self.shared.current)
                .as_ref()
                .is_some_and(|slot| slot.accepting.load(Ordering::Acquire))
    }

    /// The current authenticated connection number, if any.
    pub fn current_connection_id(&self) -> Option<u64> {
        lock(&self.shared.current)
            .as_ref()
            .filter(|slot| slot.accepting.load(Ordering::Acquire))
            .map(|slot| slot.connection_id)
    }

    /// Non-blocking event poll for the application loop.
    ///
    /// Returns [`TryRecvError::Disconnected`] only after [`shutdown`](Self::shutdown)
    /// has closed the event queue.
    pub fn try_recv(&self) -> Result<BridgeEvent, TryRecvError> {
        self.shared.events.try_recv()
    }

    /// Wait for an event up to `timeout`. Intended for tests and diagnostics.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<BridgeEvent, RecvTimeoutError> {
        self.shared.events.recv_timeout(timeout)
    }

    /// Drain every currently queued event.
    pub fn drain_events(&self) -> Vec<BridgeEvent> {
        self.shared.events.drain()
    }

    /// Number of events discarded because the bounded queue was under pressure
    /// or superseded by a newer connection epoch.
    ///
    /// This counter is diagnostic: low-priority events are dropped rather than
    /// blocking a socket, accept, or connection thread.
    pub fn dropped_events(&self) -> u64 {
        self.shared.events.dropped()
    }

    /// Number of pending state snapshots superseded by a newer snapshot.
    ///
    /// State is coalesced per connection epoch, so this counts snapshots that
    /// were replaced before the application read them.
    pub fn coalesced_states(&self) -> u64 {
        self.shared.events.coalesced()
    }

    /// Ask the plugin to activate `session_id`.
    ///
    /// Returns the request identifier that will appear in
    /// [`BridgeEvent::FocusResult`]. The bridge also fails the request after
    /// [`FOCUS_TIMEOUT`] or when the connection closes.
    pub fn focus(&self, session_id: impl Into<String>) -> io::Result<String> {
        let session_id = session_id.into();
        if session_id.is_empty() || session_id.chars().count() > MAX_SESSION_ID_CHARS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "session id must be non-empty and at most 128 characters",
            ));
        }
        let request_id = self.next_request_id("focus");
        self.send_command(ConnectionCommand::Focus {
            request_id: request_id.clone(),
            session_id,
        })?;
        Ok(request_id)
    }

    /// Ask the plugin for a fresh full snapshot.
    ///
    /// Returns the request identifier sent to the plugin. The resulting state
    /// arrives as [`BridgeEvent::State`], keyed by its revision rather than by
    /// this identifier.
    pub fn request_state(&self) -> io::Result<String> {
        let request_id = self.next_request_id("state");
        self.send_command(ConnectionCommand::RequestState {
            request_id: request_id.clone(),
        })?;
        Ok(request_id)
    }

    /// Send a manual protocol-level ping.
    ///
    /// Returns the nonce. Heartbeat timeout tracking is based only on
    /// automatic heartbeat pings.
    pub fn ping(&self) -> io::Result<String> {
        let nonce = self.next_request_id("ping");
        self.send_command(ConnectionCommand::Ping {
            nonce: nonce.clone(),
        })?;
        Ok(nonce)
    }

    /// Stop the listener and the current connection.
    ///
    /// This is idempotent and is also called from `Drop`. All delivery paths
    /// are non-blocking, so this returns promptly even when the event queue is
    /// saturated or the application is not draining it.
    pub fn shutdown(&self) -> io::Result<()> {
        let already_requested = self.shutdown_requested.swap(true, Ordering::AcqRel);
        if !already_requested {
            let _registration = lock(&self.shared.registration);
            self.shared.shutdown.store(true, Ordering::Release);
            self.shared.events.close();

            let current = lock(&self.shared.current).take();
            if let Some(slot) = current {
                let _ = slot.commands.try_send(ConnectionCommand::Shutdown {
                    code: 1001,
                    reason: "bridge shutting down".to_owned(),
                    replaced: false,
                });
                let _ = slot.done.recv_timeout(REPLACEMENT_WAIT);
            }
        }

        self.join_accept_thread()
    }

    fn join_accept_thread(&self) -> io::Result<()> {
        let handle = lock(&self.accept_thread).take();
        if let Some(handle) = handle {
            handle
                .join()
                .map_err(|_| io::Error::other("bridge accept thread panicked"))?;
        }
        Ok(())
    }

    fn next_request_id(&self, prefix: &str) -> String {
        let number = self
            .shared
            .next_request_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        format!("{prefix}-{number}")
    }

    fn send_command(&self, command: ConnectionCommand) -> io::Result<()> {
        if self.shutdown_requested.load(Ordering::Acquire)
            || self.shared.shutdown.load(Ordering::Acquire)
        {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "bridge is shut down",
            ));
        }

        let current = lock(&self.shared.current);
        let Some(slot) = current.as_ref() else {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "no authenticated plugin connection",
            ));
        };
        if !slot.accepting.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "plugin connection is closing",
            ));
        }

        match slot.commands.try_send(command) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "bridge command queue is full",
            )),
            Err(TrySendError::Disconnected(_)) => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "plugin connection is closing",
            )),
        }
    }
}

impl fmt::Debug for BridgeHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BridgeHandle")
            .field("local_addr", &self.local_addr)
            .field("url", &self.url())
            .field("pane_id", &self.pane_id)
            .field("token_fingerprint", &self.token_fingerprint)
            .finish_non_exhaustive()
    }
}

impl Drop for BridgeHandle {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct Shared {
    options: BridgeOptions,
    events: EventQueue,
    current: Mutex<Option<ConnectionSlot>>,
    registration: Mutex<()>,
    shutdown: AtomicBool,
    pending_connections: AtomicUsize,
    next_connection_id: AtomicU64,
    next_request_id: AtomicU64,
}

struct ConnectionSlot {
    connection_id: u64,
    accepting: AtomicBool,
    commands: SyncSender<ConnectionCommand>,
    done: Receiver<()>,
}

struct PendingConnectionGuard(Arc<Shared>);

impl Drop for PendingConnectionGuard {
    fn drop(&mut self) {
        self.0.pending_connections.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug)]
enum ConnectionCommand {
    Focus {
        request_id: String,
        session_id: String,
    },
    RequestState {
        request_id: String,
    },
    Ping {
        nonce: String,
    },
    Shutdown {
        code: u16,
        reason: String,
        replaced: bool,
    },
}

struct PendingFocus {
    session_id: String,
    deadline: Instant,
}

struct Outcome {
    authenticated: bool,
    code: Option<u16>,
    reason: String,
    replaced: bool,
}

/// Bind a loopback bridge and start its background threads.
///
/// Returns as soon as the listener is bound. The URL is available through
/// [`BridgeHandle::url`] and is always `ws://127.0.0.1:<port>/made`.
pub fn start(options: BridgeOptions) -> io::Result<BridgeHandle> {
    options.validate()?;

    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
    let local_addr = listener.local_addr()?;
    if !is_loopback_socket_addr(local_addr) {
        return Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("bridge bound to non-loopback address {local_addr}"),
        ));
    }
    listener.set_nonblocking(true)?;

    let token_fingerprint = options.token.fingerprint();
    let pane_id = options.pane_id.clone();

    let shared = Arc::new(Shared {
        options,
        events: EventQueue::new(),
        current: Mutex::new(None),
        registration: Mutex::new(()),
        shutdown: AtomicBool::new(false),
        pending_connections: AtomicUsize::new(0),
        next_connection_id: AtomicU64::new(0),
        next_request_id: AtomicU64::new(0),
    });

    let accept_shared = Arc::clone(&shared);
    let accept_thread = thread::Builder::new()
        .name("made-bridge-accept".to_owned())
        .spawn(move || accept_loop(listener, accept_shared))?;

    Ok(BridgeHandle {
        local_addr,
        pane_id,
        token_fingerprint,
        shared,
        accept_thread: Mutex::new(Some(accept_thread)),
        shutdown_requested: AtomicBool::new(false),
    })
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    loop {
        if shared.shutdown.load(Ordering::Acquire) {
            break;
        }

        match listener.accept() {
            Ok((stream, peer)) => {
                let accepted_at = Instant::now();
                if shared.shutdown.load(Ordering::Acquire) {
                    drop(stream);
                    break;
                }
                if !peer.ip().is_loopback() {
                    emit_protocol_error(
                        &shared,
                        None,
                        false,
                        "non_loopback_peer",
                        "rejected a non-loopback peer",
                        true,
                    );
                    drop(stream);
                    continue;
                }

                let pending = shared
                    .pending_connections
                    .fetch_add(1, Ordering::AcqRel)
                    .saturating_add(1);
                if pending > MAX_PENDING_CONNECTIONS {
                    shared.pending_connections.fetch_sub(1, Ordering::AcqRel);
                    emit_protocol_error(
                        &shared,
                        None,
                        false,
                        "connection_limit",
                        "too many pending bridge connections",
                        true,
                    );
                    drop(stream);
                    continue;
                }

                let connection_id = shared
                    .next_connection_id
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1);
                let connection_shared = Arc::clone(&shared);
                if let Err(error) = thread::Builder::new()
                    .name(format!("made-bridge-{connection_id}"))
                    .spawn(move || {
                        connection_thread(stream, connection_id, accepted_at, connection_shared);
                    })
                {
                    shared.pending_connections.fetch_sub(1, Ordering::AcqRel);
                    emit_protocol_error(
                        &shared,
                        Some(connection_id),
                        false,
                        "connection_thread_failed",
                        &format!("could not start connection thread: {error}"),
                        true,
                    );
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                thread::sleep(ACCEPT_POLL);
            }
            Err(error) => {
                emit_protocol_error(
                    &shared,
                    None,
                    false,
                    "accept_failed",
                    &format!("bridge listener failed: {error}"),
                    true,
                );
                break;
            }
        }
    }
}

fn connection_thread(
    stream: TcpStream,
    connection_id: u64,
    started_at: Instant,
    shared: Arc<Shared>,
) {
    let _pending_guard = PendingConnectionGuard(Arc::clone(&shared));
    let deadline = started_at + shared.options.auth_timeout;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_write_timeout(Some(SOCKET_WRITE_TIMEOUT));

    let (done_sender, done_receiver) = mpsc::channel();
    let authority = match stream.local_addr() {
        Ok(address) => address.to_string(),
        Err(error) => {
            emit_protocol_error(
                &shared,
                Some(connection_id),
                false,
                "handshake_failed",
                &format!("could not read the local address: {error}"),
                true,
            );
            let _ = done_sender.send(());
            return;
        }
    };

    let socket = match accept_websocket(stream, deadline, &authority) {
        Ok(socket) => socket,
        Err(error) => {
            emit_protocol_error(
                &shared,
                Some(connection_id),
                false,
                "handshake_failed",
                &error,
                true,
            );
            let _ = done_sender.send(());
            return;
        }
    };

    let (command_sender, command_receiver) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
    let mut session = Session::new(
        socket,
        connection_id,
        Arc::clone(&shared),
        command_sender,
        command_receiver,
        done_receiver,
        started_at,
    );

    let outcome = session.run();

    // Stop accepting new commands while the connection is still registered, so
    // every command queued before this point is drained and failed below.
    {
        let current = lock(&shared.current);
        if let Some(slot) = current.as_ref() {
            if slot.connection_id == connection_id {
                slot.accepting.store(false, Ordering::Release);
            }
        }
    }

    session.fail_queued_focus("connection closed before focus_result");
    session.fail_pending_focus("connection closed before focus_result");

    let disconnected = BridgeEvent::Disconnected(DisconnectedInfo {
        connection_id,
        authenticated: outcome.authenticated,
        code: outcome.code,
        reason: outcome.reason,
        replaced: outcome.replaced,
    });
    if outcome.authenticated {
        shared.events.push_lifecycle(disconnected);
    } else {
        shared.events.push_unauth(disconnected);
    }

    // Remove the registration only after the disconnect event. A replacement
    // connection that observes this slot waits for the done signal, which keeps
    // Disconnected before the replacement's Connected in the event stream.
    {
        let mut current = lock(&shared.current);
        if current
            .as_ref()
            .is_some_and(|slot| slot.connection_id == connection_id)
        {
            *current = None;
        }
    }

    // Let the replacement proceed only after Disconnected is queued, but keep
    // this socket alive for the close drain so a late peer write is still read
    // and the flushed close frame is not replaced by a reset.
    let _ = done_sender.send(());
    session.finish_close();
    drop(session);
}

/// A TCP stream that bounds every read and write by the absolute
/// authentication deadline.
///
/// Tungstenite's frame reader can keep reading a slowly arriving frame within
/// a single `read` call. Setting a socket timeout once is therefore not enough
/// to bound a byte-drip hello. This wrapper recalculates the timeout on every
/// underlying read from the shared absolute deadline. When the deadline has
/// passed it returns [`io::ErrorKind::TimedOut`] without terminating the
/// WebSocket state, so the session can still write the `4403` close frame.
struct DeadlineStream {
    stream: TcpStream,
    deadline: Arc<Mutex<Option<Instant>>>,
}

impl DeadlineStream {
    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            deadline: Arc::new(Mutex::new(None)),
        }
    }

    fn set_auth_deadline(&self, deadline: Instant) {
        *lock(&self.deadline) = Some(deadline);
    }

    fn clear_auth_deadline(&self) {
        *lock(&self.deadline) = None;
    }

    fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        self.stream.set_nonblocking(nonblocking)
    }

    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.stream.set_read_timeout(timeout)
    }

    fn shutdown_write(&self) -> io::Result<()> {
        self.stream.shutdown(Shutdown::Write)
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.stream.set_write_timeout(timeout)
    }
}

fn authentication_deadline_error() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "authentication deadline exceeded")
}

impl Read for DeadlineStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let deadline = *lock(&self.deadline);
        if let Some(deadline) = deadline {
            let now = Instant::now();
            if now >= deadline {
                return Err(authentication_deadline_error());
            }
            let wait = (deadline - now).min(AUTH_READ_SLICE);
            self.stream
                .set_read_timeout(Some(wait.max(Duration::from_millis(1))))?;
        }
        let result = self.stream.read(buffer);
        if let Some(deadline) = deadline {
            if Instant::now() >= deadline {
                return Err(authentication_deadline_error());
            }
        }
        result
    }
}

impl Write for DeadlineStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let deadline = *lock(&self.deadline);
        if let Some(deadline) = deadline {
            let now = Instant::now();
            if now >= deadline {
                return Err(authentication_deadline_error());
            }
            self.stream
                .set_write_timeout(Some((deadline - now).min(SOCKET_WRITE_TIMEOUT)))?;
        }
        self.stream.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

#[allow(clippy::result_large_err)] // tungstenite's server callback requires this error type
fn accept_websocket(
    stream: TcpStream,
    deadline: Instant,
    expected_authority: &str,
) -> Result<WebSocket<DeadlineStream>, String> {
    let config = WebSocketConfig::default()
        .read_buffer_size(32 * 1024)
        .write_buffer_size(8 * 1024)
        .max_write_buffer_size(MAX_MESSAGE_BYTES)
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));

    let expected_authority = expected_authority.to_owned();
    let callback = move |request: &Request, response: Response| {
        let valid_path = request.uri().path() == MADE_PATH && request.uri().query().is_none();
        if !valid_path {
            let mut error = ErrorResponse::new(Some("not found".to_owned()));
            *error.status_mut() = StatusCode::NOT_FOUND;
            return Err(error);
        }

        let mut hosts = request.headers().get_all(header::HOST).iter();
        let valid_host = match (hosts.next(), hosts.next()) {
            (Some(value), None) => value.to_str().ok() == Some(expected_authority.as_str()),
            _ => false,
        };
        if !valid_host {
            let mut error =
                ErrorResponse::new(Some("host is not the loopback authority".to_owned()));
            *error.status_mut() = StatusCode::FORBIDDEN;
            return Err(error);
        }
        if request.headers().contains_key(header::ORIGIN) {
            let mut error = ErrorResponse::new(Some("origin is not allowed".to_owned()));
            *error.status_mut() = StatusCode::FORBIDDEN;
            return Err(error);
        }

        Ok(response)
    };

    let stream = DeadlineStream::new(stream);
    if stream.set_nonblocking(true).is_err() {
        return Err("could not configure the handshake deadline".to_owned());
    }

    let mut handshake = tungstenite::accept_hdr_with_config(stream, callback, Some(config));
    loop {
        match handshake {
            Ok(mut socket) => {
                if Instant::now() >= deadline {
                    return Err("websocket handshake deadline exceeded".to_owned());
                }
                if socket.get_mut().set_nonblocking(false).is_err() {
                    return Err("could not restore blocking socket mode".to_owned());
                }
                socket.get_mut().set_auth_deadline(deadline);
                return Ok(socket);
            }
            Err(HandshakeError::Interrupted(mid)) => {
                let now = Instant::now();
                if now >= deadline {
                    return Err("websocket handshake deadline exceeded".to_owned());
                }
                thread::sleep((deadline - now).min(HANDSHAKE_POLL));
                handshake = mid.handshake();
            }
            Err(HandshakeError::Failure(error)) => return Err(error.to_string()),
        }
    }
}

struct Session {
    socket: WebSocket<DeadlineStream>,
    shared: Arc<Shared>,
    connection_id: u64,
    epoch: u64,
    commands: Option<SyncSender<ConnectionCommand>>,
    command_receiver: Receiver<ConnectionCommand>,
    done_receiver: Option<Receiver<()>>,
    tracker: SnapshotTracker,
    pending_focus: HashMap<String, PendingFocus>,
    heartbeat_pings: VecDeque<String>,
    last_heartbeat: Instant,
    started_at: Instant,
    authenticated: bool,
    closed: bool,
    close_code: Option<u16>,
    close_reason: String,
    replaced: bool,
}

impl Session {
    fn new(
        socket: WebSocket<DeadlineStream>,
        connection_id: u64,
        shared: Arc<Shared>,
        commands: SyncSender<ConnectionCommand>,
        command_receiver: Receiver<ConnectionCommand>,
        done_receiver: Receiver<()>,
        started_at: Instant,
    ) -> Self {
        Self {
            socket,
            shared,
            connection_id,
            epoch: connection_id,
            commands: Some(commands),
            command_receiver,
            done_receiver: Some(done_receiver),
            tracker: SnapshotTracker::new(),
            pending_focus: HashMap::new(),
            heartbeat_pings: VecDeque::new(),
            last_heartbeat: Instant::now(),
            started_at,
            authenticated: false,
            closed: false,
            close_code: None,
            close_reason: "connection closed".to_owned(),
            replaced: false,
        }
    }

    fn run(&mut self) -> Outcome {
        if self.shared.shutdown.load(Ordering::Acquire) {
            self.close_now(1001, "bridge shutting down");
            return self.outcome();
        }

        if !self.authenticate() {
            return self.outcome();
        }

        self.loop_authenticated();
        self.outcome()
    }

    fn outcome(&self) -> Outcome {
        Outcome {
            authenticated: self.authenticated,
            code: self.close_code,
            reason: self.close_reason.clone(),
            replaced: self.replaced,
        }
    }

    fn authenticate(&mut self) -> bool {
        let deadline = self.started_at + self.shared.options.auth_timeout;

        loop {
            if self.shared.shutdown.load(Ordering::Acquire) {
                self.close_now(1001, "bridge shutting down");
                return false;
            }

            let now = Instant::now();
            if now >= deadline {
                self.reject(
                    4403,
                    "hello required",
                    "auth_timeout",
                    "hello was not received before the authentication deadline",
                );
                return false;
            }

            let read = self.socket.read();
            if Instant::now() >= deadline {
                self.reject(
                    4403,
                    "hello required",
                    "auth_timeout",
                    "hello was not received before the authentication deadline",
                );
                return false;
            }

            match read {
                Ok(Message::Text(text)) => {
                    if text.len() > MAX_MESSAGE_BYTES {
                        self.reject(
                            1009,
                            "message too large",
                            "message_too_large",
                            "hello exceeded the maximum message size",
                        );
                        return false;
                    }
                    if self.handle_hello(text.as_str()) {
                        self.socket.get_mut().clear_auth_deadline();
                        return true;
                    }
                    return false;
                }
                Ok(Message::Binary(_)) => {
                    self.reject(
                        1003,
                        "text messages only",
                        "binary_message",
                        "binary messages are not part of the bridge protocol",
                    );
                    return false;
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {
                    let _ = self.socket.flush();
                }
                Ok(Message::Close(frame)) => {
                    let code = frame.as_ref().map(|frame| u16::from(frame.code));
                    let reason = frame
                        .as_ref()
                        .map(|frame| frame.reason.to_string())
                        .unwrap_or_else(|| "no close reason".to_owned());
                    let reason = redact_secret(&reason, &self.shared.options.token);
                    self.set_close(code, format!("client closed before hello: {reason}"));
                    let _ = self.socket.close(None);
                    let _ = self.socket.flush();
                    return false;
                }
                Ok(Message::Frame(_)) => {}
                Err(WsError::Io(error)) if is_would_block(&error) => {}
                Err(WsError::Capacity(_)) => {
                    self.reject(
                        1009,
                        "message too large",
                        "message_too_large",
                        "hello exceeded the maximum message size",
                    );
                    return false;
                }
                Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => {
                    self.set_close(None, "connection closed before hello");
                    return false;
                }
                Err(error) => {
                    self.reject(
                        1002,
                        "protocol error",
                        "websocket_protocol_error",
                        &format!("websocket error before hello: {error}"),
                    );
                    return false;
                }
            }
        }
    }

    fn handle_hello(&mut self, text: &str) -> bool {
        let Some(value) = parse_json_object(text, self) else {
            return false;
        };
        let Some(kind) = message_kind(&value) else {
            self.reject(
                1003,
                "malformed message",
                "malformed_message",
                "message is not an object with a string type field",
            );
            return false;
        };
        if kind != "hello" {
            self.reject(
                4401,
                "hello required",
                "hello_required",
                "the first message must be hello",
            );
            return false;
        }

        let hello: HelloMessage = match serde_json::from_value(value) {
            Ok(hello) => hello,
            Err(error) => {
                self.reject(
                    4401,
                    "invalid hello",
                    "invalid_hello",
                    &format!("hello is malformed: {error}"),
                );
                return false;
            }
        };
        if let Err(error) = hello.validate() {
            self.reject(4401, "invalid hello", "invalid_hello", &error);
            return false;
        }
        if !self.shared.options.token.verify(&hello.token) {
            self.reject(4401, "invalid token", "invalid_token", "token was rejected");
            return false;
        }
        if hello.protocol_version != PROTOCOL_VERSION {
            self.reject(
                4402,
                "unsupported protocol version",
                "protocol_mismatch",
                &format!(
                    "protocol version {} is unsupported; expected {PROTOCOL_VERSION}",
                    hello.protocol_version
                ),
            );
            return false;
        }
        if hello.pane_id != self.shared.options.pane_id {
            self.reject(
                4401,
                "pane mismatch",
                "pane_mismatch",
                "pane id does not match this bridge",
            );
            return false;
        }
        let Some(opencode_major) = opencode_major(&hello.opencode_version) else {
            self.reject(
                4402,
                "unsupported OpenCode version",
                "unsupported_opencode_version",
                "OpenCode version could not be parsed",
            );
            return false;
        };
        if opencode_major < 2 {
            self.reject(
                4402,
                "unsupported OpenCode version",
                "unsupported_opencode_version",
                "Made requires OpenCode major version 2 or newer",
            );
            return false;
        }

        if !self.register() {
            return false;
        }

        let welcome = WelcomeMessage::new(
            PROTOCOL_VERSION,
            self.shared.options.app_version.clone(),
            duration_millis(self.shared.options.heartbeat_interval),
        );
        if !self.send_json(&welcome) {
            self.set_close(None, "could not send welcome");
            return false;
        }

        self.authenticated = true;
        let tick = tick_interval(self.shared.options.heartbeat_interval);
        let _ = self.socket.get_mut().set_read_timeout(Some(tick));
        let _ = self
            .socket
            .get_mut()
            .set_write_timeout(Some(SOCKET_WRITE_TIMEOUT));
        self.last_heartbeat = Instant::now();

        let token = &self.shared.options.token;
        self.shared
            .events
            .push_lifecycle(BridgeEvent::Connected(ConnectionInfo {
                connection_id: self.connection_id,
                pane_id: redact_secret(&hello.pane_id, token),
                plugin_version: redact_secret(&hello.plugin_version, token),
                opencode_version: redact_secret(&hello.opencode_version, token),
                pid: hello.pid,
                cwd: PathBuf::from(redact_secret(&hello.cwd, token)),
                protocol_version: hello.protocol_version,
            }));

        true
    }

    fn register(&mut self) -> bool {
        let shared = Arc::clone(&self.shared);
        let _registration = lock(&shared.registration);

        if self.shared.shutdown.load(Ordering::Acquire) {
            self.close_now(1001, "bridge shutting down");
            return false;
        }

        let old = lock(&self.shared.current).take();
        if let Some(old) = old {
            if old.connection_id != self.connection_id {
                let _ = old.commands.try_send(ConnectionCommand::Shutdown {
                    code: 4409,
                    reason: "replaced by newer connection".to_owned(),
                    replaced: true,
                });
                let _ = old.done.recv_timeout(REPLACEMENT_WAIT);
            }
        }

        if self.shared.shutdown.load(Ordering::Acquire) {
            self.close_now(1001, "bridge shutting down");
            return false;
        }

        let (Some(commands), Some(done)) = (self.commands.take(), self.done_receiver.take()) else {
            self.set_close(None, "connection registration failed");
            return false;
        };

        // A new authenticated connection is a new snapshot epoch. Queued state
        // from older epochs is discarded before this connection can emit its
        // own Connected event, so a stale snapshot can never follow it.
        self.shared.events.begin_epoch(self.epoch);

        *lock(&self.shared.current) = Some(ConnectionSlot {
            connection_id: self.connection_id,
            accepting: AtomicBool::new(true),
            commands,
            done,
        });
        true
    }

    fn loop_authenticated(&mut self) {
        let tick = tick_interval(self.shared.options.heartbeat_interval);

        loop {
            if self.shared.shutdown.load(Ordering::Acquire) {
                self.close_now(1001, "bridge shutting down");
                break;
            }

            if !self.ensure_current() {
                break;
            }

            let mut keep_running = true;
            loop {
                match self.command_receiver.try_recv() {
                    Ok(command) => {
                        if !self.handle_command(command) {
                            keep_running = false;
                            break;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.close_now(1001, "bridge connection closed");
                        keep_running = false;
                        break;
                    }
                }
            }
            if !keep_running || self.closed {
                break;
            }

            self.heartbeat();
            if self.closed {
                break;
            }
            self.expire_focus();

            if self.socket.get_mut().set_read_timeout(Some(tick)).is_err() {
                self.set_close(None, "could not configure socket read timeout");
                break;
            }

            let mut stop = false;
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    if !self.handle_text(text.as_str()) {
                        stop = true;
                    }
                }
                Ok(Message::Binary(_)) => {
                    self.reject(
                        1003,
                        "text messages only",
                        "binary_message",
                        "binary messages are not part of the bridge protocol",
                    );
                    stop = true;
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                Ok(Message::Close(frame)) => {
                    let code = frame.as_ref().map(|frame| u16::from(frame.code));
                    let reason = frame
                        .as_ref()
                        .map(|frame| frame.reason.to_string())
                        .unwrap_or_else(|| "no close reason".to_owned());
                    let reason = redact_secret(&reason, &self.shared.options.token);
                    self.set_close(code, format!("client closed: {reason}"));
                    let _ = self.socket.close(None);
                    stop = true;
                }
                Ok(Message::Frame(_)) => {}
                Err(WsError::Io(error)) if is_would_block(&error) => {}
                Err(WsError::Capacity(_)) => {
                    self.reject(
                        1009,
                        "message too large",
                        "message_too_large",
                        "message exceeded the maximum message size",
                    );
                    stop = true;
                }
                Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => {
                    self.set_close(None, "connection closed");
                    stop = true;
                }
                Err(error) => {
                    self.reject(
                        1002,
                        "protocol error",
                        "websocket_protocol_error",
                        &format!("websocket error: {error}"),
                    );
                    stop = true;
                }
            }

            if self.socket.flush().is_err() {
                if !self.closed {
                    self.set_close(None, "socket flush failed");
                }
                stop = true;
            }
            if stop {
                break;
            }
        }
    }

    fn heartbeat(&mut self) {
        if self.last_heartbeat.elapsed() < self.shared.options.heartbeat_interval {
            return;
        }

        if self.heartbeat_pings.len() as u32 >= MISSED_PONG_LIMIT {
            self.shared
                .events
                .push_lifecycle(BridgeEvent::HeartbeatTimeout(HeartbeatTimeout {
                    connection_id: self.connection_id,
                    missed_pongs: self.heartbeat_pings.len() as u32,
                }));
            self.close_now(1001, "heartbeat timeout");
            return;
        }

        let nonce = self.next_nonce("hb");
        if self.send_json(&PingMessage::new(nonce.clone())) {
            self.heartbeat_pings.push_back(nonce);
            self.last_heartbeat = Instant::now();
        } else {
            self.set_close(None, "could not send heartbeat ping");
        }
    }

    fn expire_focus(&mut self) {
        let now = Instant::now();
        let expired: Vec<String> = self
            .pending_focus
            .iter()
            .filter(|(_, pending)| pending.deadline <= now)
            .map(|(request_id, _)| request_id.clone())
            .collect();

        for request_id in expired {
            if let Some(pending) = self.pending_focus.remove(&request_id) {
                self.shared
                    .events
                    .push_droppable(BridgeEvent::FocusResult(FocusResult {
                        request_id,
                        session_id: Some(pending.session_id),
                        ok: false,
                        error: Some("focus request timed out".to_owned()),
                    }));
            }
        }
    }

    fn fail_queued_focus(&mut self, reason: &str) {
        while let Ok(command) = self.command_receiver.try_recv() {
            if let ConnectionCommand::Focus {
                request_id,
                session_id,
            } = command
            {
                self.shared
                    .events
                    .push_droppable(BridgeEvent::FocusResult(FocusResult {
                        request_id,
                        session_id: Some(session_id),
                        ok: false,
                        error: Some(reason.to_owned()),
                    }));
            }
        }
    }

    fn fail_pending_focus(&mut self, reason: &str) {
        for (request_id, pending) in self.pending_focus.drain() {
            self.shared
                .events
                .push_droppable(BridgeEvent::FocusResult(FocusResult {
                    request_id,
                    session_id: Some(pending.session_id),
                    ok: false,
                    error: Some(reason.to_owned()),
                }));
        }
    }

    fn handle_command(&mut self, command: ConnectionCommand) -> bool {
        match command {
            ConnectionCommand::Focus {
                request_id,
                session_id,
            } => {
                if self.pending_focus.len() >= MAX_PENDING_FOCUS {
                    self.shared
                        .events
                        .push_droppable(BridgeEvent::FocusResult(FocusResult {
                            request_id,
                            session_id: Some(session_id),
                            ok: false,
                            error: Some("too many pending focus requests".to_owned()),
                        }));
                    return true;
                }

                let deadline = Instant::now()
                    .checked_add(self.shared.options.focus_timeout)
                    .unwrap_or_else(Instant::now);
                self.pending_focus.insert(
                    request_id.clone(),
                    PendingFocus {
                        session_id: session_id.clone(),
                        deadline,
                    },
                );
                if !self.send_json(&FocusMessage::new(request_id, session_id)) {
                    self.set_close(None, "could not send focus request");
                    return false;
                }
                true
            }
            ConnectionCommand::RequestState { request_id } => {
                if !self.send_json(&RequestStateMessage::new(request_id)) {
                    self.set_close(None, "could not send state request");
                    return false;
                }
                true
            }
            ConnectionCommand::Ping { nonce } => {
                if !self.send_json(&PingMessage::new(nonce)) {
                    self.set_close(None, "could not send ping");
                    return false;
                }
                true
            }
            ConnectionCommand::Shutdown {
                code,
                reason,
                replaced,
            } => {
                self.replaced = replaced;
                self.close_now(code, &reason);
                false
            }
        }
    }

    fn handle_text(&mut self, text: &str) -> bool {
        if !self.ensure_current() {
            return false;
        }
        if text.len() > MAX_MESSAGE_BYTES {
            self.reject(
                1009,
                "message too large",
                "message_too_large",
                "message exceeded the maximum message size",
            );
            return false;
        }

        let Some(value) = parse_json_object(text, self) else {
            return false;
        };
        let Some(kind) = message_kind(&value).map(str::to_owned) else {
            self.reject(
                1003,
                "malformed message",
                "malformed_message",
                "message is not an object with a string type field",
            );
            return false;
        };

        match kind.as_str() {
            "state" => self.handle_state(value),
            "focus_result" => self.handle_focus_result(value),
            "pong" => {
                self.handle_pong(value);
                true
            }
            "goodbye" => {
                let goodbye: GoodbyeMessage = match serde_json::from_value(value) {
                    Ok(goodbye) => goodbye,
                    Err(error) => {
                        self.reject(
                            1008,
                            "invalid goodbye",
                            "invalid_goodbye",
                            &format!("goodbye is malformed: {error}"),
                        );
                        return false;
                    }
                };
                let reason = bounded_goodbye_reason(&redact_secret(
                    &goodbye.reason,
                    &self.shared.options.token,
                ));
                self.shared
                    .events
                    .push_lifecycle(BridgeEvent::Goodbye(Goodbye {
                        connection_id: self.connection_id,
                        reason,
                    }));
                self.close_now(1000, "goodbye");
                false
            }
            "error" => self.handle_plugin_error(value),
            // A duplicate hello or an app-to-plugin message is not fatal;
            // unknown message types are ignored for additive compatibility.
            _ => true,
        }
    }

    fn handle_state(&mut self, value: Value) -> bool {
        let message: StateMessage = match serde_json::from_value(value) {
            Ok(message) => message,
            Err(error) => {
                self.reject(
                    1008,
                    "invalid state",
                    "invalid_state",
                    &format!("state is malformed: {error}"),
                );
                return false;
            }
        };

        let snapshot = match self.tracker.accept(message.into_snapshot()) {
            Ok(snapshot) => snapshot,
            Err(SnapshotError::StaleRevision { .. }) => return true,
            Err(error) => {
                self.reject(1008, "invalid state", "invalid_state", &error.to_string());
                return false;
            }
        };

        self.shared.events.push_state(
            self.epoch,
            redact_snapshot(snapshot, &self.shared.options.token),
        );
        true
    }

    fn handle_focus_result(&mut self, value: Value) -> bool {
        let message: FocusResultMessage = match serde_json::from_value(value) {
            Ok(message) => message,
            Err(error) => {
                self.reject(
                    1008,
                    "invalid focus result",
                    "invalid_focus_result",
                    &format!("focus_result is malformed: {error}"),
                );
                return false;
            }
        };

        let Some(pending) = self.pending_focus.remove(&message.request_id) else {
            self.protocol_error(
                "uncorrelated_focus_result",
                "focus result did not match a pending request",
                false,
            );
            return true;
        };

        let error = if message.ok {
            None
        } else {
            message.error.as_deref().map(|error| {
                bounded_error_message(&redact_secret(error, &self.shared.options.token))
            })
        };
        self.shared
            .events
            .push_droppable(BridgeEvent::FocusResult(FocusResult {
                request_id: message.request_id,
                session_id: Some(pending.session_id),
                ok: message.ok,
                error,
            }));
        true
    }

    fn handle_pong(&mut self, value: Value) {
        let message: PongMessage = match serde_json::from_value(value) {
            Ok(message) => message,
            Err(error) => {
                self.protocol_error(
                    "invalid_pong",
                    &format!("pong is malformed: {error}"),
                    false,
                );
                return;
            }
        };
        if let Some(index) = self
            .heartbeat_pings
            .iter()
            .position(|nonce| nonce == &message.nonce)
        {
            self.heartbeat_pings.remove(index);
        }
    }

    fn handle_plugin_error(&mut self, value: Value) -> bool {
        let message: ErrorMessage = match serde_json::from_value(value) {
            Ok(message) => message,
            Err(error) => {
                self.reject(
                    1008,
                    "invalid plugin error",
                    "invalid_plugin_error",
                    &format!("plugin error message is malformed: {error}"),
                );
                return false;
            }
        };

        let code = bounded_error_code(&redact_secret(&message.code, &self.shared.options.token));
        let detail =
            bounded_error_message(&redact_secret(&message.message, &self.shared.options.token));
        self.shared
            .events
            .push_protocol_error(BridgeEvent::ProtocolError(ProtocolError {
                connection_id: Some(self.connection_id),
                code,
                message: detail,
                fatal: message.fatal,
            }));

        if message.fatal {
            self.close_now(1011, "plugin reported a fatal error");
            false
        } else {
            true
        }
    }

    fn send_json<T: Serialize>(&mut self, message: &T) -> bool {
        let Ok(text) = serde_json::to_string(message) else {
            return false;
        };
        if text.len() > MAX_MESSAGE_BYTES {
            return false;
        }
        self.socket.send(Message::Text(text.into())).is_ok()
    }

    fn next_nonce(&self, prefix: &str) -> String {
        let number = self
            .shared
            .next_request_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        format!("{prefix}-{number}")
    }

    fn connection_is_current(&self) -> bool {
        lock(&self.shared.current)
            .as_ref()
            .is_some_and(|slot| slot.connection_id == self.connection_id)
    }

    fn ensure_current(&mut self) -> bool {
        if self.shared.shutdown.load(Ordering::Acquire) {
            self.close_now(1001, "bridge shutting down");
            return false;
        }
        if self.connection_is_current() {
            return true;
        }
        self.replaced = true;
        self.close_now(4409, "replaced by newer connection");
        false
    }

    fn protocol_error(&self, code: &str, message: &str, fatal: bool) {
        emit_protocol_error(
            &self.shared,
            Some(self.connection_id),
            self.authenticated,
            code,
            message,
            fatal,
        );
    }

    fn reject(&mut self, close_code: u16, close_reason: &str, error_code: &str, message: &str) {
        self.protocol_error(error_code, message, true);
        self.close_now(close_code, close_reason);
    }

    fn set_close(&mut self, code: Option<u16>, reason: impl Into<String>) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.close_code = code;
        let reason = reason.into();
        self.close_reason = truncate_chars(
            if reason.is_empty() {
                "connection closed"
            } else {
                &reason
            },
            120,
        );
    }

    fn close_now(&mut self, code: u16, reason: &str) {
        // A close can happen at the authentication deadline, when reads are
        // already refused. Clear the deadline so the close frame can be written.
        self.socket.get_mut().clear_auth_deadline();
        let was_closed = self.closed;
        self.set_close(Some(code), reason);
        if !was_closed {
            close_socket(&mut self.socket, code, reason);
        }
    }

    /// Finish a close without letting the operating system replace the close
    /// frame with a TCP reset. Flush the queued frame, drain unread input for a
    /// bounded time, then half-close the write side. Windows resets a socket
    /// that is dropped with unread receive data; draining lets the peer read the
    /// close frame first. A replaced connection keeps the socket readable a
    /// little longer so the peer's late write is drained instead of aborting.
    fn finish_close(&mut self) {
        self.socket.get_mut().clear_auth_deadline();
        let _ = self.socket.flush();
        let replacement = self.replaced || self.close_code == Some(4409);
        let read_timeout = if replacement {
            REPLACEMENT_CLOSE_DRAIN_TIMEOUT
        } else {
            CLOSE_DRAIN_TIMEOUT
        };
        let total = if replacement {
            REPLACEMENT_CLOSE_DRAIN_TOTAL
        } else {
            CLOSE_DRAIN_TOTAL
        };
        let _ = self.socket.get_mut().set_read_timeout(Some(read_timeout));
        let deadline = Instant::now() + total;
        let mut buffer = [0u8; 4096];
        loop {
            match self.socket.get_mut().read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) if is_would_block(&error) && !replacement => break,
                Err(error) if is_would_block(&error) => {}
                Err(_) => break,
            }
            if Instant::now() >= deadline {
                break;
            }
        }
        let _ = self.socket.get_mut().shutdown_write();
    }
}

fn redact_snapshot(mut snapshot: SessionSnapshot, token: &Token) -> SessionSnapshot {
    if let Some(active_id) = &mut snapshot.active_session_id {
        redact_in_place(active_id, token);
    }
    if let Some(view_directory) = &mut snapshot.view_directory {
        redact_path(view_directory, token);
    }
    for session in &mut snapshot.sessions {
        redact_in_place(&mut session.session_id, token);
        redact_in_place(&mut session.title, token);

        if let Some(directory) = &mut session.directory {
            redact_path(directory, token);
        }
        if let Some(subpath) = &mut session.subpath {
            redact_in_place(subpath, token);
        }
        if let Some(project_id) = &mut session.project_id {
            redact_in_place(project_id, token);
        }
    }
    snapshot
}

fn redact_path(path: &mut PathBuf, token: &Token) {
    let display = path.to_string_lossy();
    let secret = token.expose_secret();
    if !secret.is_empty() && display.contains(secret) {
        *path = PathBuf::from(display.replace(secret, REDACTED));
    }
}

fn redact_in_place(value: &mut String, token: &Token) {
    let secret = token.expose_secret();
    if !secret.is_empty() && value.contains(secret) {
        *value = value.replace(secret, REDACTED);
    }
}

fn parse_json_object(text: &str, session: &mut Session) -> Option<Value> {
    match serde_json::from_str::<Value>(text) {
        Ok(value @ Value::Object(_)) => Some(value),
        Ok(_) => {
            session.reject(
                1003,
                "malformed message",
                "malformed_message",
                "message must be a JSON object",
            );
            None
        }
        Err(error) => {
            session.reject(
                1003,
                "malformed JSON",
                "malformed_json",
                &format!("message was not valid JSON: {error}"),
            );
            None
        }
    }
}

fn message_kind(value: &Value) -> Option<&str> {
    value.get("type").and_then(Value::as_str)
}

fn emit_protocol_error(
    shared: &Shared,
    connection_id: Option<u64>,
    authenticated: bool,
    code: &str,
    message: &str,
    fatal: bool,
) {
    let token = &shared.options.token;
    let code = bounded_error_code(&redact_secret(code, token));
    let message = bounded_error_message(&redact_secret(message, token));
    let message = truncate_chars(&message, MAX_ERROR_CHARS);
    let event = BridgeEvent::ProtocolError(ProtocolError {
        connection_id,
        code,
        message,
        fatal,
    });
    if authenticated {
        shared.events.push_protocol_error(event);
    } else {
        shared.events.push_unauth(event);
    }
}

fn close_socket(socket: &mut WebSocket<DeadlineStream>, code: u16, reason: &str) {
    let reason = truncate_chars(reason, 100);
    let frame = CloseFrame {
        code: close_code(code),
        reason: reason.into(),
    };
    match socket.close(Some(frame)) {
        Ok(()) => {}
        Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => {}
        Err(_) => {
            let _ = socket.flush();
        }
    }
}

fn close_code(code: u16) -> CloseCode {
    match code {
        1000 => CloseCode::Normal,
        1001 => CloseCode::Away,
        1002 => CloseCode::Protocol,
        1003 => CloseCode::Unsupported,
        1007 => CloseCode::Invalid,
        1008 => CloseCode::Policy,
        1009 => CloseCode::Size,
        1011 => CloseCode::Error,
        other => CloseCode::Library(other),
    }
}

fn is_would_block(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}

fn tick_interval(heartbeat: Duration) -> Duration {
    (heartbeat / 4).clamp(Duration::from_millis(5), Duration::from_millis(50))
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
