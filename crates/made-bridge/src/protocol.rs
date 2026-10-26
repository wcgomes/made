//! WebSocket wire protocol shared with the OpenCode plugin.
//!
//! Every message is a JSON text frame with a `type` discriminator. The field
//! names and semantics match the production plugin and the Phase 0 spike:
//!
//! | Type | Direction | Purpose |
//! |---|---|---|
//! | `hello` | plugin → app | Authenticate and identify the connection |
//! | `welcome` | app → plugin | Confirm authentication and version |
//! | `state` | plugin → app | Full normalized session snapshot |
//! | `focus` | app → plugin | Focus a session in OpenCode |
//! | `focus_result` | plugin → app | Report a focus outcome |
//! | `request_state` | app → plugin | Ask for a fresh snapshot |
//! | `ping` | app → plugin | Liveness check |
//! | `pong` | plugin → app | Liveness response |
//! | `error` | plugin → app | Plugin-reported error |
//! | `goodbye` | plugin → app | Clean unload |
//!
//! Unknown fields are ignored, so additive fields remain compatible within a
//! protocol major version.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    security::{
        MAX_ERROR_CHARS, MAX_ERROR_CODE_CHARS, MAX_GOODBYE_CHARS, MAX_OPENCODE_VERSION_CHARS,
        MAX_PANE_ID_CHARS, MAX_PATH_CHARS, MAX_VERSION_CHARS,
    },
    state::SessionSnapshot,
};

/// The protocol major version implemented by this crate.
pub const PROTOCOL_VERSION: u32 = 1;

/// The only WebSocket path served by the bridge.
pub const MADE_PATH: &str = crate::security::MADE_PATH;

/// The JSON `type` discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    /// Plugin authentication message.
    Hello,
    /// Bridge authentication acknowledgement.
    Welcome,
    /// Full session snapshot.
    State,
    /// Focus request.
    Focus,
    /// Focus response.
    FocusResult,
    /// Snapshot refresh request.
    RequestState,
    /// Liveness request.
    Ping,
    /// Liveness response.
    Pong,
    /// Plugin error report.
    Error,
    /// Plugin unload notification.
    Goodbye,
}

/// Plugin authentication message. Must be the first message on a connection.
#[derive(Clone, Serialize, Deserialize)]
pub struct HelloMessage {
    /// Message discriminator (`hello`).
    #[serde(rename = "type")]
    pub message_type: MessageKind,
    /// Protocol major version.
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
    /// Per-instance secret. Never logged.
    pub token: String,
    /// Terminal pane identity.
    #[serde(rename = "paneId")]
    pub pane_id: String,
    /// Plugin package version.
    #[serde(rename = "pluginVersion")]
    pub plugin_version: String,
    /// OpenCode version reported by the TUI.
    #[serde(rename = "opencodeVersion")]
    pub opencode_version: String,
    /// Plugin process id.
    pub pid: i64,
    /// Plugin process working directory at launch.
    pub cwd: String,
}

impl HelloMessage {
    /// Construct a hello message. The token is stored as supplied.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        protocol_version: u32,
        token: impl Into<String>,
        pane_id: impl Into<String>,
        plugin_version: impl Into<String>,
        opencode_version: impl Into<String>,
        pid: i64,
        cwd: impl Into<String>,
    ) -> Self {
        Self {
            message_type: MessageKind::Hello,
            protocol_version,
            token: token.into(),
            pane_id: pane_id.into(),
            plugin_version: plugin_version.into(),
            opencode_version: opencode_version.into(),
            pid,
            cwd: cwd.into(),
        }
    }

    /// Validate shape and bounded string sizes before authentication.
    ///
    /// The token length and value are checked with
    /// [`Token::verify`](crate::security::Token::verify); this method only
    /// ensures the credentials are present.
    pub fn validate(&self) -> Result<(), String> {
        if self.message_type != MessageKind::Hello {
            return Err("expected hello".into());
        }
        if self.token.is_empty() {
            return Err("token is missing".into());
        }
        if self.pane_id.is_empty() || self.pane_id.chars().count() > MAX_PANE_ID_CHARS {
            return Err("paneId is missing or too long".into());
        }
        if self.plugin_version.is_empty() || self.plugin_version.chars().count() > MAX_VERSION_CHARS
        {
            return Err("pluginVersion is missing or too long".into());
        }
        if self.opencode_version.is_empty()
            || self.opencode_version.chars().count() > MAX_OPENCODE_VERSION_CHARS
        {
            return Err("opencodeVersion is missing or too long".into());
        }
        if self.pid < 0 {
            return Err("pid is invalid".into());
        }
        if self.cwd.chars().count() > MAX_PATH_CHARS {
            return Err("cwd is too long".into());
        }
        Ok(())
    }
}

impl fmt::Debug for HelloMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HelloMessage")
            .field("message_type", &self.message_type)
            .field("protocol_version", &self.protocol_version)
            .field("token", &crate::security::REDACTED)
            .field("pane_id", &self.pane_id)
            .field("plugin_version", &self.plugin_version)
            .field("opencode_version", &self.opencode_version)
            .field("pid", &self.pid)
            .field("cwd", &self.cwd)
            .finish()
    }
}

/// Bridge authentication acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WelcomeMessage {
    /// Message discriminator (`welcome`).
    #[serde(rename = "type")]
    pub message_type: MessageKind,
    /// Protocol major version.
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
    /// Made application version.
    #[serde(rename = "appVersion")]
    pub app_version: String,
    /// Heartbeat period the plugin should expect.
    #[serde(rename = "heartbeatIntervalMs")]
    pub heartbeat_interval_ms: u64,
}

impl WelcomeMessage {
    /// Construct a welcome message.
    pub fn new(
        protocol_version: u32,
        app_version: impl Into<String>,
        heartbeat_interval_ms: u64,
    ) -> Self {
        Self {
            message_type: MessageKind::Welcome,
            protocol_version,
            app_version: app_version.into(),
            heartbeat_interval_ms,
        }
    }
}

/// A full normalized session snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateMessage {
    /// Message discriminator (`state`).
    #[serde(rename = "type")]
    pub message_type: MessageKind,
    /// The snapshot fields, flattened into the same JSON object.
    #[serde(flatten)]
    pub snapshot: SessionSnapshot,
}

impl StateMessage {
    /// Wrap a snapshot in a wire message.
    pub fn new(snapshot: SessionSnapshot) -> Self {
        Self {
            message_type: MessageKind::State,
            snapshot,
        }
    }

    /// Consume the message and return its snapshot.
    pub fn into_snapshot(self) -> SessionSnapshot {
        self.snapshot
    }
}

/// Focus a session in the OpenCode TUI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusMessage {
    /// Message discriminator (`focus`).
    #[serde(rename = "type")]
    pub message_type: MessageKind,
    /// Correlation identifier chosen by Made.
    #[serde(rename = "requestID")]
    pub request_id: String,
    /// Session to activate.
    #[serde(rename = "sessionID")]
    pub session_id: String,
}

impl FocusMessage {
    /// Construct a focus request.
    pub fn new(request_id: impl Into<String>, session_id: impl Into<String>) -> Self {
        Self {
            message_type: MessageKind::Focus,
            request_id: request_id.into(),
            session_id: session_id.into(),
        }
    }
}

/// Result of a focus request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusResultMessage {
    /// Message discriminator (`focus_result`).
    #[serde(rename = "type")]
    pub message_type: MessageKind,
    /// Correlation identifier from the focus request.
    #[serde(rename = "requestID")]
    pub request_id: String,
    /// Whether OpenCode activated the session.
    pub ok: bool,
    /// Failure detail when `ok` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl FocusResultMessage {
    /// Construct a focus result.
    pub fn new(request_id: impl Into<String>, ok: bool, error: Option<impl Into<String>>) -> Self {
        Self {
            message_type: MessageKind::FocusResult,
            request_id: request_id.into(),
            ok,
            error: error.map(Into::into),
        }
    }
}

/// Ask the plugin for a fresh snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestStateMessage {
    /// Message discriminator (`request_state`).
    #[serde(rename = "type")]
    pub message_type: MessageKind,
    /// Correlation identifier chosen by Made.
    #[serde(rename = "requestID")]
    pub request_id: String,
}

impl RequestStateMessage {
    /// Construct a refresh request.
    pub fn new(request_id: impl Into<String>) -> Self {
        Self {
            message_type: MessageKind::RequestState,
            request_id: request_id.into(),
        }
    }
}

/// Liveness request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PingMessage {
    /// Message discriminator (`ping`).
    #[serde(rename = "type")]
    pub message_type: MessageKind,
    /// Opaque nonce echoed by the plugin.
    pub nonce: String,
}

impl PingMessage {
    /// Construct a ping.
    pub fn new(nonce: impl Into<String>) -> Self {
        Self {
            message_type: MessageKind::Ping,
            nonce: nonce.into(),
        }
    }
}

/// Liveness response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PongMessage {
    /// Message discriminator (`pong`).
    #[serde(rename = "type")]
    pub message_type: MessageKind,
    /// Nonce copied from the ping.
    pub nonce: String,
}

impl PongMessage {
    /// Construct a pong.
    pub fn new(nonce: impl Into<String>) -> Self {
        Self {
            message_type: MessageKind::Pong,
            nonce: nonce.into(),
        }
    }
}

/// Plugin-reported error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorMessage {
    /// Message discriminator (`error`).
    #[serde(rename = "type")]
    pub message_type: MessageKind,
    /// Stable machine-readable code.
    pub code: String,
    /// Human-readable detail.
    pub message: String,
    /// Whether the plugin considers the connection unusable.
    pub fatal: bool,
}

impl ErrorMessage {
    /// Construct a plugin error message.
    pub fn new(code: impl Into<String>, message: impl Into<String>, fatal: bool) -> Self {
        Self {
            message_type: MessageKind::Error,
            code: code.into(),
            message: message.into(),
            fatal,
        }
    }
}

/// Plugin unload notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoodbyeMessage {
    /// Message discriminator (`goodbye`).
    #[serde(rename = "type")]
    pub message_type: MessageKind,
    /// Reason for unloading.
    pub reason: String,
}

impl GoodbyeMessage {
    /// Construct a goodbye message.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            message_type: MessageKind::Goodbye,
            reason: reason.into(),
        }
    }
}

/// Parse the major version from an OpenCode version string.
///
/// Accepts values such as `"2"`, `"2.1.4"`, `"v2.1.4"`, and
/// `"2.1.4-beta.1"`. Returns `None` when no leading numeric major is present.
pub fn opencode_major(version: &str) -> Option<u32> {
    let version = version.trim();
    let version = version.strip_prefix('v').unwrap_or(version);
    let major = version
        .split(['.', '-', '+'])
        .next()
        .filter(|part| !part.is_empty())?;
    if !major.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    major.parse().ok()
}

/// Truncate untrusted text to a bounded number of characters.
pub(crate) fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        text.chars().take(max).collect()
    }
}

/// Bound a protocol error code from a plugin message.
pub(crate) fn bounded_error_code(code: &str) -> String {
    truncate_chars(code, MAX_ERROR_CODE_CHARS)
}

/// Bound a protocol error message from a plugin message.
pub(crate) fn bounded_error_message(message: &str) -> String {
    truncate_chars(message, MAX_ERROR_CHARS)
}

/// Bound a goodbye reason from a plugin message.
pub(crate) fn bounded_goodbye_reason(reason: &str) -> String {
    truncate_chars(reason, MAX_GOODBYE_CHARS)
}
