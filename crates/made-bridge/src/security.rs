//! Bridge hardening and token redaction.
//!
//! This module owns the per-instance secret, the loopback address rules, and
//! the bounded limits used by the bridge and state validation. A [`Token`] is
//! generated from 256 bits of operating-system randomness and compared in
//! constant time. Its `Debug` and `Display` implementations never reveal the
//! secret, so logging a token, a [`BridgeOptions`](crate::server::BridgeOptions),
//! or a composed config cannot leak it by accident.

use std::{
    fmt,
    net::{IpAddr, SocketAddr},
    str::FromStr,
    time::Duration,
};

use sha2::{Digest, Sha256};
use thiserror::Error;

/// Maximum size of a single WebSocket message, in bytes.
pub const MAX_MESSAGE_BYTES: usize = 256 * 1024;

/// Maximum number of sessions in one state snapshot.
pub const MAX_SESSIONS: usize = 64;

/// Maximum number of characters in a session title.
pub const MAX_TITLE_CHARS: usize = 512;

/// Maximum number of characters in a directory, subpath, or hello cwd.
pub const MAX_PATH_CHARS: usize = 4096;

/// Maximum number of characters in a session identifier.
pub const MAX_SESSION_ID_CHARS: usize = 128;

/// Maximum number of characters in an OpenCode project identifier.
pub const MAX_PROJECT_ID_CHARS: usize = 128;

/// Maximum number of characters in a pane identifier.
pub const MAX_PANE_ID_CHARS: usize = 128;

/// Maximum number of characters in a plugin or app version.
pub const MAX_VERSION_CHARS: usize = 128;

/// Maximum number of characters in the Made application version.
pub const MAX_APP_VERSION_CHARS: usize = 128;

/// Maximum number of characters in an OpenCode version string.
pub const MAX_OPENCODE_VERSION_CHARS: usize = 64;

/// Maximum number of characters in a focus error or plugin error message.
pub const MAX_ERROR_CHARS: usize = 512;

/// Maximum number of characters in a goodbye reason.
pub const MAX_GOODBYE_CHARS: usize = 256;

/// Maximum number of characters in a protocol error code.
pub const MAX_ERROR_CODE_CHARS: usize = 64;

/// The bridge requires a hello within this deadline, measured from the
/// accepted socket. The deadline covers the WebSocket handshake and the
/// `hello` message, so a byte-drip handshake cannot hold a connection thread
/// beyond it. Exceeding the deadline closes the socket with code `4403`.
/// Test harnesses may use a shorter timeout through
/// [`BridgeOptions::with_auth_timeout`](crate::server::BridgeOptions::with_auth_timeout).
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(1);

/// Default interval between bridge heartbeat pings.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// Number of unanswered heartbeat pings after which the bridge gives up.
pub const MISSED_PONG_LIMIT: u32 = 3;

/// Default deadline for a focus result before Made reports a non-blocking failure.
pub const FOCUS_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum number of focus requests awaiting a result on one connection.
pub const MAX_PENDING_FOCUS: usize = 64;

/// Capacity of the bounded per-connection command queue.
pub const COMMAND_QUEUE_CAPACITY: usize = 64;

/// Capacity of the bounded bridge event queue.
///
/// Events are never delivered by blocking a socket, accept, or connection
/// thread. The queue reserves headroom for lifecycle events
/// (`Connected`, `Disconnected`, authenticated `ProtocolError`,
/// `HeartbeatTimeout`, and `Goodbye`) and coalesces pending state snapshots to
/// the latest accepted snapshot per connection epoch. When the droppable part
/// of the queue is exhausted, low-priority events are dropped and counted by
/// [`BridgeHandle::dropped_events`](crate::server::BridgeHandle::dropped_events).
pub const EVENT_QUEUE_CAPACITY: usize = 256;

/// Maximum number of unauthenticated notices kept in the event queue.
///
/// Unauthenticated traffic has its own small budget so it cannot displace
/// authenticated lifecycle events or state.
pub const MAX_UNAUTH_EVENT_NOTICES: usize = 8;

/// Number of queue slots reserved for priority events. The remaining capacity
/// is used by state snapshots and other droppable notifications.
pub const EVENT_QUEUE_RESERVED_SLOTS: usize = 32;

/// Maximum number of simultaneously pending connection threads.
///
/// This bounds slow-handshake (slowloris) resource use. Only one of these
/// sockets can be an authenticated connection; the rest are handshake or
/// pre-auth sockets waiting for their bounded deadline.
pub const MAX_PENDING_CONNECTIONS: usize = 8;

/// The placeholder used whenever a secret must be rendered.
pub const REDACTED: &str = "[redacted]";

/// Number of random bytes in a bridge token (256 bits).
pub const TOKEN_BYTES: usize = 32;

/// Number of lowercase hexadecimal characters in an encoded bridge token.
pub const TOKEN_HEX_LEN: usize = TOKEN_BYTES * 2;

/// The WebSocket path served by the bridge.
pub const MADE_PATH: &str = "/made";

/// Errors produced while constructing or validating a [`Token`].
#[derive(Debug, Error)]
pub enum TokenError {
    /// The supplied token did not contain exactly [`TOKEN_HEX_LEN`] characters.
    #[error("token must be {expected} hexadecimal characters, got {actual}")]
    WrongLength {
        /// Required number of characters.
        expected: usize,
        /// Supplied number of characters.
        actual: usize,
    },
    /// The supplied token contained a non-hexadecimal character.
    #[error("token contains a non-hexadecimal character")]
    InvalidHex,
    /// The operating system random source failed.
    #[error("could not obtain system randomness: {0}")]
    Random(#[source] getrandom::Error),
}

/// A 256-bit bridge secret.
///
/// The secret is stored as lowercase hexadecimal so it can be placed in the
/// plugin options without an encoding step. It is intentionally not
/// `Serialize`, and neither `Debug` nor `Display` reveals it.
#[derive(Clone)]
pub struct Token(String);

impl Token {
    /// Generate a fresh token from the operating system random source.
    pub fn generate() -> Result<Self, TokenError> {
        let mut bytes = [0_u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes).map_err(TokenError::Random)?;
        Ok(Self::from_bytes(bytes))
    }

    /// Build a token from raw random bytes.
    pub fn from_bytes(bytes: [u8; TOKEN_BYTES]) -> Self {
        Self(hex_lower(&bytes))
    }

    /// Parse a token from exactly [`TOKEN_HEX_LEN`] hexadecimal characters.
    ///
    /// Uppercase input is accepted and normalized to lowercase.
    pub fn from_hex(value: impl AsRef<str>) -> Result<Self, TokenError> {
        let value = value.as_ref();
        if value.len() != TOKEN_HEX_LEN {
            return Err(TokenError::WrongLength {
                expected: TOKEN_HEX_LEN,
                actual: value.len(),
            });
        }
        if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(TokenError::InvalidHex);
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    /// Return the secret for the one place that legitimately needs it: plugin
    /// option composition.
    ///
    /// Callers must never log, persist, or display the returned string.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    /// Compare a presented hexadecimal token in constant time.
    ///
    /// Only the length check is not constant time; all equal-length
    /// comparisons touch every byte.
    pub fn verify(&self, presented: &str) -> bool {
        let expected = self.0.as_bytes();
        let presented = presented.as_bytes();
        if expected.len() != presented.len() {
            return false;
        }
        let mut difference = 0_u8;
        for (left, right) in expected.iter().zip(presented) {
            difference |= left ^ right;
        }
        difference == 0
    }

    /// A short SHA-256 fingerprint suitable for diagnostics and logs.
    ///
    /// This never exposes the token and is safe to include in events.
    pub fn fingerprint(&self) -> String {
        let digest = Sha256::digest(self.0.as_bytes());
        format!("sha256:{}", &hex_lower(&digest)[..16])
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Token([redacted])")
    }
}

impl fmt::Display for Token {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED)
    }
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        self.verify(other.expose_secret())
    }
}

impl Eq for Token {}

impl FromStr for Token {
    type Err = TokenError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_hex(value)
    }
}

/// A borrowed string with a token redacted for safe logging.
///
/// ```ignore
/// tracing::warn!("{}", Redacted::new(detail, &token));
/// ```
#[derive(Clone, Copy)]
pub struct Redacted<'a> {
    text: &'a str,
    secret: &'a str,
}

impl<'a> Redacted<'a> {
    /// Wrap `text`, replacing every occurrence of `token` with [`REDACTED`].
    pub fn new(text: &'a str, token: &'a Token) -> Self {
        Self {
            text,
            secret: token.expose_secret(),
        }
    }
}

impl fmt::Display for Redacted<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&redact_secret_impl(self.text, self.secret))
    }
}

impl fmt::Debug for Redacted<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

/// Replace every occurrence of `token` in `text` with [`REDACTED`].
pub fn redact_secret(text: &str, token: &Token) -> String {
    redact_secret_impl(text, token.expose_secret())
}

fn redact_secret_impl(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_owned();
    }
    text.replace(secret, REDACTED)
}

/// Return true when `raw` is a loopback WebSocket URL accepted by the plugin.
///
/// Accepted forms are exactly:
///
/// - `ws://127.<0-255>.<0-255>.<0-255>:<port>/made`
/// - `ws://[::1]:<port>/made`
///
/// The host must be an IP literal in the IPv4 loopback /8 or IPv6 loopback,
/// the port must be explicit and non-zero, the path must be exactly `/made`,
/// and user info, queries, fragments, TLS, and hostnames are rejected.
pub fn is_loopback_ws_url(raw: &str) -> bool {
    if !raw.is_ascii() || raw.len() < "ws://x/made".len() {
        return false;
    }
    let Some(scheme) = raw.get(..5) else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("ws://") {
        return false;
    }
    let rest = &raw[5..];
    if rest.contains('@') || rest.contains('?') || rest.contains('#') {
        return false;
    }
    let Some(slash) = rest.find('/') else {
        return false;
    };
    let authority = &rest[..slash];
    let path = &rest[slash..];
    if path != MADE_PATH {
        return false;
    }

    let port = if let Some(inner) = authority.strip_prefix('[') {
        let Some((host, after)) = inner.split_once(']') else {
            return false;
        };
        if host != "::1" {
            return false;
        }
        let Some(port) = after.strip_prefix(':') else {
            return false;
        };
        port
    } else {
        let Some((host, port)) = authority.rsplit_once(':') else {
            return false;
        };
        if host.contains(':') || !is_loopback_ipv4(host) {
            return false;
        }
        port
    };

    parse_explicit_port(port)
}

fn is_loopback_ipv4(host: &str) -> bool {
    let mut parts = host.split('.');
    let Some(first) = parts.next() else {
        return false;
    };
    if first != "127" {
        return false;
    }
    let mut count = 1;
    for part in parts {
        count += 1;
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
        let Ok(octet) = part.parse::<u16>() else {
            return false;
        };
        if octet > 255 {
            return false;
        }
    }
    count == 4
}

fn parse_explicit_port(port: &str) -> bool {
    if port.is_empty() || port.len() > 5 || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    port.parse::<u16>().is_ok_and(|port| port > 0)
}

/// Build the canonical bridge URL for a local port.
pub fn loopback_ws_url(port: u16) -> String {
    format!("ws://127.0.0.1:{port}{MADE_PATH}")
}

/// Return true when a socket address is loopback.
pub fn is_loopback_socket_addr(address: SocketAddr) -> bool {
    address.ip().is_loopback()
}

/// Return true when an IP address is loopback.
pub fn is_loopback_ip(address: IpAddr) -> bool {
    address.is_loopback()
}

pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}
