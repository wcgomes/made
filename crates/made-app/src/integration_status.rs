//! Integration status state machine, OpenCode version gate, and last-known-root
//! fallback.
//!
//! Status is one of `connected`, `degraded`, or `unsupported`. Every failure
//! keeps the terminal and the last usable explorer root; nothing here can block
//! input. The state machine consumes the bridge's normalized events and adds
//! app-level events for configuration, OpenCode availability, tabs, focus, and
//! unavailable directories.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use made_bridge::protocol::{opencode_major, PROTOCOL_VERSION};
use made_bridge::server::{
    BridgeEvent, ConnectionInfo, DisconnectedInfo, Goodbye, HeartbeatTimeout, ProtocolError,
};
use made_bridge::state::SessionSnapshot;

/// Minimum supported OpenCode major version.
pub const SUPPORTED_OPENCODE_MAJOR: u32 = 2;

/// Integration availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationStatus {
    /// Optional integration has not been started in this shell.
    Inactive,
    /// A valid plugin connection and compatible protocol are active.
    Connected,
    /// Integration is absent, waiting, disconnected, or temporarily unusable.
    Degraded,
    /// The plugin or OpenCode version is incompatible with this app.
    Unsupported,
}

impl IntegrationStatus {
    /// Stable lowercase label for diagnostics.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Inactive => "inactive",
            Self::Connected => "connected",
            Self::Degraded => "degraded",
            Self::Unsupported => "unsupported",
        }
    }

    /// Whether the integration is connected.
    pub const fn is_connected(self) -> bool {
        matches!(self, Self::Connected)
    }

    /// Whether the integration is degraded.
    pub const fn is_degraded(self) -> bool {
        matches!(self, Self::Degraded)
    }

    /// Whether the integration is unsupported.
    pub const fn is_unsupported(self) -> bool {
        matches!(self, Self::Unsupported)
    }
}

impl fmt::Display for IntegrationStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// Machine-readable reason codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReasonCode {
    /// Optional integration is inactive; no executable or hello is awaited.
    Inactive,
    /// A healthy connection.
    Connected,
    /// A connection returned after a failure.
    Reconnected,
    /// The OpenCode plugin is not loaded.
    PluginNotLoaded,
    /// OpenCode is not installed or not on PATH.
    OpenCodeMissing,
    /// OpenCode is older than v2.
    OpenCodeTooOld,
    /// OpenCode started with extension loading disabled.
    ExtensionLoadingDisabled,
    /// Per-launch integration configuration could not be composed.
    ConfigCompositionFailed,
    /// The plugin protocol major version is incompatible.
    ProtocolMismatch,
    /// The bridge rejected the connection credentials or pane.
    AuthenticationRejected,
    /// The bridge connection closed.
    BridgeClosed,
    /// Heartbeat pings went unanswered.
    HeartbeatTimeout,
    /// OpenCode exited or unloaded the plugin.
    OpenCodeExited,
    /// OpenCode session tabs are disabled.
    TabsUnavailable,
    /// The active session directory is missing or unreadable.
    ActiveDirectoryUnavailable,
    /// Read-only git metadata could not be read.
    WorktreeMetadataUnavailable,
    /// A focus request failed.
    FocusFailed,
    /// A focus request timed out.
    FocusTimedOut,
}

impl ReasonCode {
    /// Every reason code, useful for diagnostics tests and UI copy checks.
    pub const ALL: [Self; 18] = [
        Self::Inactive,
        Self::Connected,
        Self::Reconnected,
        Self::PluginNotLoaded,
        Self::OpenCodeMissing,
        Self::OpenCodeTooOld,
        Self::ExtensionLoadingDisabled,
        Self::ConfigCompositionFailed,
        Self::ProtocolMismatch,
        Self::AuthenticationRejected,
        Self::BridgeClosed,
        Self::HeartbeatTimeout,
        Self::OpenCodeExited,
        Self::TabsUnavailable,
        Self::ActiveDirectoryUnavailable,
        Self::WorktreeMetadataUnavailable,
        Self::FocusFailed,
        Self::FocusTimedOut,
    ];

    /// Stable machine-readable code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Inactive => "inactive",
            Self::Connected => "connected",
            Self::Reconnected => "reconnected",
            Self::PluginNotLoaded => "plugin_not_loaded",
            Self::OpenCodeMissing => "opencode_missing",
            Self::OpenCodeTooOld => "opencode_too_old",
            Self::ExtensionLoadingDisabled => "extension_loading_disabled",
            Self::ConfigCompositionFailed => "config_composition_failed",
            Self::ProtocolMismatch => "protocol_mismatch",
            Self::AuthenticationRejected => "authentication_rejected",
            Self::BridgeClosed => "bridge_closed",
            Self::HeartbeatTimeout => "heartbeat_timeout",
            Self::OpenCodeExited => "opencode_exited",
            Self::TabsUnavailable => "tabs_unavailable",
            Self::ActiveDirectoryUnavailable => "active_directory_unavailable",
            Self::WorktreeMetadataUnavailable => "worktree_metadata_unavailable",
            Self::FocusFailed => "focus_failed",
            Self::FocusTimedOut => "focus_timed_out",
        }
    }

    /// Actionable human-readable message.
    pub const fn message(self) -> &'static str {
        match self {
            Self::Inactive => {
                "Optional OpenCode integration is inactive."
            }
            Self::Connected => "Connected to the OpenCode v2 integration.",
            Self::Reconnected => "Reconnected to the OpenCode v2 integration.",
            Self::PluginNotLoaded => {
                "The OpenCode v2 integration plugin did not load; restart OpenCode in the Made terminal."
            }
            Self::OpenCodeMissing => {
                "OpenCode v2 is not installed or not on PATH; install OpenCode v2 and run it in the terminal."
            }
            Self::OpenCodeTooOld => {
                "OpenCode v2 or newer is required; upgrade OpenCode and restart it in the terminal."
            }
            Self::ExtensionLoadingDisabled => {
                "OpenCode started with extension loading disabled; enable plugins for this launch."
            }
            Self::ConfigCompositionFailed => {
                "Made could not compose its per-launch OpenCode configuration; your files were not changed."
            }
            Self::ProtocolMismatch => {
                "The Made plugin protocol is incompatible with this app; update Made or the plugin."
            }
            Self::AuthenticationRejected => {
                "The integration connection was rejected; restart OpenCode in the Made terminal."
            }
            Self::BridgeClosed => {
                "The integration connection closed; waiting to reconnect while the terminal stays usable."
            }
            Self::HeartbeatTimeout => {
                "The integration stopped responding; waiting to reconnect while the terminal stays usable."
            }
            Self::OpenCodeExited => "OpenCode exited; the shell remains usable.",
            Self::TabsUnavailable => {
                "OpenCode session tabs are disabled; Made requires tabs to follow sessions."
            }
            Self::ActiveDirectoryUnavailable => {
                "The active session directory is unavailable; showing the last known folder."
            }
            Self::WorktreeMetadataUnavailable => {
                "Git metadata could not be read; showing the folder without branch information."
            }
            Self::FocusFailed => {
                "The session could not be focused in OpenCode; the current workspace was kept."
            }
            Self::FocusTimedOut => {
                "Focusing the session timed out; the current workspace was kept."
            }
        }
    }

    /// Whether this reason is actionable by the user.
    pub const fn is_actionable(self) -> bool {
        !matches!(
            self,
            Self::Inactive | Self::Connected | Self::Reconnected | Self::OpenCodeExited
        )
    }
}

/// A reason code plus bounded detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReason {
    /// Stable reason code.
    pub code: ReasonCode,
    /// Optional detail, bounded to a safe display length.
    pub detail: Option<String>,
}

impl StatusReason {
    /// Build a reason without detail.
    pub const fn new(code: ReasonCode) -> Self {
        Self { code, detail: None }
    }

    /// Build a reason with detail.
    pub fn with_detail(code: ReasonCode, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        let detail = if detail.is_empty() {
            None
        } else {
            Some(bounded_detail(&detail))
        };
        Self { code, detail }
    }

    /// Human-readable explanation used by the status area and diagnostics.
    pub fn human(&self) -> String {
        match &self.detail {
            Some(detail) => format!("{} ({detail})", self.code.message()),
            None => self.code.message().to_owned(),
        }
    }
}

/// Snapshot of the integration state for views and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationReport {
    /// Current status.
    pub status: IntegrationStatus,
    /// Current reason.
    pub reason: StatusReason,
    /// Last root established by an active session.
    pub last_known_root: Option<PathBuf>,
    /// Active session identifier.
    pub active_session_id: Option<String>,
    /// Active session title.
    pub active_session_title: Option<String>,
    /// Plugin package version, when connected.
    pub plugin_version: Option<String>,
    /// OpenCode version, when connected.
    pub opencode_version: Option<String>,
    /// Negotiated bridge protocol version, when connected.
    pub protocol_version: Option<u32>,
    /// Bridge connection identifier, when connected.
    pub connection_id: Option<u64>,
    /// Last accepted state revision.
    pub last_revision: Option<u64>,
    /// Last heartbeat or state observation.
    pub last_heartbeat: Option<Instant>,
    /// Whether OpenCode reports session tabs as enabled.
    pub tabs_enabled: bool,
}

impl IntegrationReport {
    /// Age of the last heartbeat at `now`.
    pub fn heartbeat_age(&self, now: Instant) -> Option<Duration> {
        self.last_heartbeat
            .map(|last| now.saturating_duration_since(last))
    }

    /// Whether the last heartbeat is older than `threshold`.
    pub fn heartbeat_stale(&self, now: Instant, threshold: Duration) -> bool {
        self.heartbeat_age(now).is_some_and(|age| age > threshold)
    }
}

/// One status transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusChange {
    /// Status before the event.
    pub previous: IntegrationStatus,
    /// Status after the event.
    pub current: IntegrationStatus,
    /// Reason after the event.
    pub reason: StatusReason,
    /// Whether the status value changed.
    pub status_changed: bool,
    /// Whether the reason changed.
    pub reason_changed: bool,
}

impl StatusChange {
    /// Whether anything user-visible changed.
    pub const fn changed(&self) -> bool {
        self.status_changed || self.reason_changed
    }
}

/// Integration state consumed by the status area, diagnostics, and session
/// follow.
#[derive(Debug, Clone)]
pub struct IntegrationState {
    status: IntegrationStatus,
    reason: StatusReason,
    last_known_root: Option<PathBuf>,
    launch_root: Option<PathBuf>,
    terminal_cwd: PathBuf,
    active_session_id: Option<String>,
    active_session_title: Option<String>,
    connection: Option<ConnectionInfo>,
    last_revision: Option<u64>,
    last_heartbeat: Option<Instant>,
    tabs_enabled: bool,
    focus_error: Option<String>,
}

impl IntegrationState {
    /// Healthy shell use starts with optional integration inactive.
    pub fn new(launch_root: Option<PathBuf>, terminal_cwd: impl Into<PathBuf>) -> Self {
        Self {
            status: IntegrationStatus::Inactive,
            reason: StatusReason::new(ReasonCode::Inactive),
            last_known_root: None,
            launch_root,
            terminal_cwd: terminal_cwd.into(),
            active_session_id: None,
            active_session_title: None,
            connection: None,
            last_revision: None,
            last_heartbeat: None,
            tabs_enabled: false,
            focus_error: None,
        }
    }

    /// Current status.
    pub const fn status(&self) -> IntegrationStatus {
        self.status
    }

    /// Current reason.
    pub fn reason(&self) -> &StatusReason {
        &self.reason
    }

    /// Active session identifier.
    pub fn active_session_id(&self) -> Option<&str> {
        self.active_session_id.as_deref()
    }

    /// Active session title.
    pub fn active_session_title(&self) -> Option<&str> {
        self.active_session_title.as_deref()
    }

    /// Last accepted state revision.
    pub const fn last_revision(&self) -> Option<u64> {
        self.last_revision
    }

    /// Last heartbeat or state observation.
    pub const fn last_heartbeat(&self) -> Option<Instant> {
        self.last_heartbeat
    }

    /// Current plugin/OpenCode connection identity.
    pub fn connection(&self) -> Option<&ConnectionInfo> {
        self.connection.as_ref()
    }

    /// Last known active root.
    pub fn last_known_root(&self) -> Option<&Path> {
        self.last_known_root.as_deref()
    }

    /// Focus failure detail, when the last focus attempt failed.
    pub fn focus_error(&self) -> Option<&str> {
        self.focus_error.as_deref()
    }

    /// Snapshot of the state.
    pub fn report(&self) -> IntegrationReport {
        IntegrationReport {
            status: self.status,
            reason: self.reason.clone(),
            last_known_root: self.last_known_root.clone(),
            active_session_id: self.active_session_id.clone(),
            active_session_title: self.active_session_title.clone(),
            plugin_version: self
                .connection
                .as_ref()
                .map(|info| info.plugin_version.clone()),
            opencode_version: self
                .connection
                .as_ref()
                .map(|info| info.opencode_version.clone()),
            protocol_version: self.connection.as_ref().map(|info| info.protocol_version),
            connection_id: self.connection.as_ref().map(|info| info.connection_id),
            last_revision: self.last_revision,
            last_heartbeat: self.last_heartbeat,
            tabs_enabled: self.tabs_enabled,
        }
    }

    /// Record the last root established by a successful session follow.
    pub fn note_root(&mut self, root: impl Into<PathBuf>) {
        self.last_known_root = Some(root.into());
    }

    /// Update the launch workspace used as fallback.
    pub fn set_launch_root(&mut self, root: impl Into<PathBuf>) {
        self.launch_root = Some(root.into());
    }

    /// Update the terminal working directory used as fallback.
    pub fn set_terminal_cwd(&mut self, cwd: impl Into<PathBuf>) {
        self.terminal_cwd = cwd.into();
    }

    /// Latest validated shell cwd; old agent roots never outrank it.
    pub fn fallback_root(&self) -> Option<&Path> {
        Some(self.terminal_cwd.as_path())
    }

    /// Record a heartbeat observation.
    pub fn note_heartbeat(&mut self, now: Instant) {
        self.last_heartbeat = Some(now);
    }

    /// Whether `event` belongs to the currently authenticated connection.
    ///
    /// A bridge accepts unauthenticated sockets long enough to reject them.
    /// Their protocol errors and disconnects must never affect a healthy
    /// workspace, so lifecycle events are applied only when their connection
    /// identifier matches the current authenticated connection. Events without
    /// an identifier (or with a foreign identifier) are treated as noise while
    /// a connection exists. With no established connection, only
    /// pre-authentication failures can change the status; unsupported states
    /// are never overwritten by unrelated client noise.
    pub fn accepts_event(&self, event: &BridgeEvent) -> bool {
        match event {
            BridgeEvent::Connected(_) | BridgeEvent::State(_) | BridgeEvent::FocusResult(_) => true,
            BridgeEvent::ProtocolError(error) => match error.connection_id {
                Some(connection_id) => self.connection.as_ref().map_or_else(
                    || !self.status.is_unsupported(),
                    |current| current.connection_id == connection_id,
                ),
                None => self.connection.is_none() && !self.status.is_unsupported(),
            },
            BridgeEvent::Disconnected(info) => self
                .connection
                .as_ref()
                .is_some_and(|current| current.connection_id == info.connection_id),
            BridgeEvent::HeartbeatTimeout(timeout) => self
                .connection
                .as_ref()
                .is_some_and(|current| current.connection_id == timeout.connection_id),
            BridgeEvent::Goodbye(goodbye) => self
                .connection
                .as_ref()
                .is_some_and(|current| current.connection_id == goodbye.connection_id),
        }
    }

    /// Apply a bridge event to the state machine.
    ///
    /// Events from foreign or rejected connections are ignored so a healthy
    /// authenticated workspace cannot be reset by unrelated local clients.
    pub fn apply_bridge_event(&mut self, event: &BridgeEvent) -> StatusChange {
        if !self.accepts_event(event) {
            return self.unchanged();
        }
        match event {
            BridgeEvent::Connected(info) => self.connected(info),
            BridgeEvent::State(snapshot) => self.state(snapshot),
            BridgeEvent::FocusResult(result) => {
                if result.ok {
                    self.note_focus_succeeded()
                } else {
                    self.note_focus_failed(
                        result
                            .error
                            .clone()
                            .unwrap_or_else(|| "focus request failed".to_owned()),
                    )
                }
            }
            BridgeEvent::Disconnected(info) => self.disconnected(info),
            BridgeEvent::ProtocolError(error) => self.protocol_error(error),
            BridgeEvent::HeartbeatTimeout(timeout) => self.heartbeat_timeout(timeout),
            BridgeEvent::Goodbye(goodbye) => self.goodbye(goodbye),
        }
    }

    /// App-level event: the plugin is known not to be loaded.
    pub fn note_plugin_unavailable(&mut self, detail: impl Into<String>) -> StatusChange {
        self.transition(
            IntegrationStatus::Degraded,
            StatusReason::with_detail(ReasonCode::PluginNotLoaded, detail),
        )
    }

    /// App-level event: OpenCode started with extension loading disabled.
    pub fn note_extension_loading_disabled(&mut self) -> StatusChange {
        self.transition(
            IntegrationStatus::Degraded,
            StatusReason::new(ReasonCode::ExtensionLoadingDisabled),
        )
    }

    /// App-level event: per-launch configuration composition failed.
    pub fn note_config_composition_failed(&mut self, detail: impl Into<String>) -> StatusChange {
        self.transition(
            IntegrationStatus::Degraded,
            StatusReason::with_detail(ReasonCode::ConfigCompositionFailed, detail),
        )
    }

    /// App-level event: OpenCode is not installed or not on PATH.
    pub fn note_opencode_missing(&mut self) -> StatusChange {
        self.transition(
            IntegrationStatus::Unsupported,
            StatusReason::new(ReasonCode::OpenCodeMissing),
        )
    }

    /// Apply the OpenCode major-version gate.
    pub fn note_opencode_version(&mut self, version: &str) -> StatusChange {
        if version.trim().is_empty() {
            return self.note_opencode_missing();
        }
        if !is_supported_opencode_version(version) {
            return self.transition(
                IntegrationStatus::Unsupported,
                StatusReason::with_detail(ReasonCode::OpenCodeTooOld, version),
            );
        }

        if self.status.is_unsupported()
            && matches!(
                self.reason.code,
                ReasonCode::OpenCodeMissing | ReasonCode::OpenCodeTooOld
            )
        {
            return self.transition(
                IntegrationStatus::Inactive,
                StatusReason::new(ReasonCode::Inactive),
            );
        }
        self.unchanged()
    }

    /// App-level event: OpenCode exited normally.
    pub fn note_opencode_exited(&mut self) -> StatusChange {
        self.connection = None;
        self.transition(
            IntegrationStatus::Inactive,
            StatusReason::new(ReasonCode::OpenCodeExited),
        )
    }

    /// OpenCode reports that session tabs are disabled.
    pub fn note_tabs_unavailable(&mut self) -> StatusChange {
        self.tabs_enabled = false;
        self.transition(
            IntegrationStatus::Degraded,
            StatusReason::new(ReasonCode::TabsUnavailable),
        )
    }

    /// The active session directory is missing or unreadable.
    ///
    /// Integration remains connected; the root is retained.
    pub fn note_active_directory_unavailable(
        &mut self,
        session_id: impl Into<String>,
        detail: impl Into<String>,
    ) -> StatusChange {
        self.active_session_id = Some(session_id.into());
        self.active_session_title = None;
        let status = self.status;
        self.transition(
            status,
            StatusReason::with_detail(ReasonCode::ActiveDirectoryUnavailable, detail),
        )
    }

    /// Read-only git metadata could not be read.
    pub fn note_worktree_metadata_unavailable(
        &mut self,
        detail: impl Into<String>,
    ) -> StatusChange {
        let status = self.status;
        self.transition(
            status,
            StatusReason::with_detail(ReasonCode::WorktreeMetadataUnavailable, detail),
        )
    }

    /// A focus request failed. The workspace and connection are retained.
    pub fn note_focus_failed(&mut self, detail: impl Into<String>) -> StatusChange {
        let detail = bounded_detail(&detail.into());
        self.focus_error = Some(detail.clone());
        let status = self.status;
        let reason = if status.is_connected() {
            StatusReason::with_detail(ReasonCode::FocusFailed, detail)
        } else {
            self.reason.clone()
        };
        self.transition(status, reason)
    }

    /// A focus request timed out. The workspace and connection are retained.
    pub fn note_focus_timed_out(&mut self) -> StatusChange {
        self.focus_error = Some("focus request timed out".to_owned());
        let status = self.status;
        let reason = if status.is_connected() {
            StatusReason::new(ReasonCode::FocusTimedOut)
        } else {
            self.reason.clone()
        };
        self.transition(status, reason)
    }

    /// A focus request succeeded; clears a focus failure reason.
    pub fn note_focus_succeeded(&mut self) -> StatusChange {
        self.focus_error = None;
        if self.status.is_connected()
            && matches!(
                self.reason.code,
                ReasonCode::FocusFailed | ReasonCode::FocusTimedOut
            )
        {
            return self.transition(
                IntegrationStatus::Connected,
                StatusReason::new(ReasonCode::Connected),
            );
        }
        self.unchanged()
    }

    /// Return the current state without changing it.
    pub fn unchanged(&self) -> StatusChange {
        StatusChange {
            previous: self.status,
            current: self.status,
            reason: self.reason.clone(),
            status_changed: false,
            reason_changed: false,
        }
    }

    fn connected(&mut self, info: &ConnectionInfo) -> StatusChange {
        let was_connected = self.status.is_connected();
        let reconnected = !was_connected
            && matches!(
                self.reason.code,
                ReasonCode::BridgeClosed
                    | ReasonCode::HeartbeatTimeout
                    | ReasonCode::OpenCodeExited
                    | ReasonCode::AuthenticationRejected
                    | ReasonCode::Reconnected
                    | ReasonCode::ProtocolMismatch
                    | ReasonCode::OpenCodeTooOld
            );
        self.connection = Some(info.clone());
        self.last_revision = None;
        self.tabs_enabled = false;
        self.active_session_id = None;
        self.active_session_title = None;
        self.focus_error = None;
        self.last_heartbeat = Some(Instant::now());

        if info.protocol_version != PROTOCOL_VERSION {
            return self.transition(
                IntegrationStatus::Unsupported,
                StatusReason::with_detail(
                    ReasonCode::ProtocolMismatch,
                    format!(
                        "plugin protocol {} is unsupported; expected {}",
                        info.protocol_version, PROTOCOL_VERSION
                    ),
                ),
            );
        }
        if !is_supported_opencode_version(&info.opencode_version) {
            return self.transition(
                IntegrationStatus::Unsupported,
                StatusReason::with_detail(
                    ReasonCode::OpenCodeTooOld,
                    info.opencode_version.clone(),
                ),
            );
        }

        self.transition(
            IntegrationStatus::Connected,
            StatusReason::new(if reconnected {
                ReasonCode::Reconnected
            } else {
                ReasonCode::Connected
            }),
        )
    }

    fn state(&mut self, snapshot: &SessionSnapshot) -> StatusChange {
        if self.connection.is_none() || self.status.is_unsupported() {
            return self.unchanged();
        }
        if self
            .last_revision
            .is_some_and(|last| snapshot.revision <= last)
        {
            return self.unchanged();
        }

        self.last_revision = Some(snapshot.revision);
        self.last_heartbeat = Some(Instant::now());
        self.focus_error = None;

        let active_id = snapshot.active_session_id.clone().or_else(|| {
            snapshot
                .sessions
                .iter()
                .find(|session| session.active)
                .map(|session| session.session_id.clone())
        });
        self.active_session_id = active_id.clone();
        self.active_session_title = active_id.as_ref().and_then(|id| {
            snapshot
                .sessions
                .iter()
                .find(|session| &session.session_id == id)
                .map(|session| session.title.clone())
        });
        self.tabs_enabled = snapshot.tabs_enabled;

        if !snapshot.tabs_enabled {
            return self.transition(
                IntegrationStatus::Degraded,
                StatusReason::new(ReasonCode::TabsUnavailable),
            );
        }
        self.transition(
            IntegrationStatus::Connected,
            StatusReason::new(ReasonCode::Connected),
        )
    }

    fn disconnected(&mut self, info: &DisconnectedInfo) -> StatusChange {
        if info.replaced {
            return self.unchanged();
        }
        self.connection = None;

        // The bridge emits a specific failure event before closing the socket.
        // A following `Disconnected` must not overwrite that more useful
        // reason (for example an incompatible protocol or a normal OpenCode
        // exit).
        if self.status.is_unsupported()
            || matches!(
                self.reason.code,
                ReasonCode::ProtocolMismatch
                    | ReasonCode::AuthenticationRejected
                    | ReasonCode::OpenCodeTooOld
                    | ReasonCode::HeartbeatTimeout
                    | ReasonCode::OpenCodeExited
            )
        {
            return self.unchanged();
        }

        self.transition(
            IntegrationStatus::Degraded,
            StatusReason::with_detail(ReasonCode::BridgeClosed, info.reason.clone()),
        )
    }

    fn protocol_error(&mut self, error: &ProtocolError) -> StatusChange {
        if error.fatal {
            self.connection = None;
        }
        match error.code.as_str() {
            "protocol_mismatch" | "unsupported_protocol_version" => self.transition(
                IntegrationStatus::Unsupported,
                StatusReason::with_detail(ReasonCode::ProtocolMismatch, error.message.clone()),
            ),
            "unsupported_opencode_version" => self.transition(
                IntegrationStatus::Unsupported,
                StatusReason::with_detail(ReasonCode::OpenCodeTooOld, error.message.clone()),
            ),
            "invalid_token" | "pane_mismatch" | "auth_timeout" | "hello_required"
            | "invalid_hello" => self.transition(
                IntegrationStatus::Degraded,
                StatusReason::with_detail(
                    ReasonCode::AuthenticationRejected,
                    error.message.clone(),
                ),
            ),
            _ if error.fatal => self.transition(
                IntegrationStatus::Degraded,
                StatusReason::with_detail(ReasonCode::BridgeClosed, error.message.clone()),
            ),
            _ => self.unchanged(),
        }
    }

    fn heartbeat_timeout(&mut self, timeout: &HeartbeatTimeout) -> StatusChange {
        self.connection = None;
        self.transition(
            IntegrationStatus::Degraded,
            StatusReason::with_detail(
                ReasonCode::HeartbeatTimeout,
                format!("{} missed heartbeats", timeout.missed_pongs),
            ),
        )
    }

    fn goodbye(&mut self, goodbye: &Goodbye) -> StatusChange {
        self.connection = None;
        self.transition(
            IntegrationStatus::Inactive,
            StatusReason::with_detail(ReasonCode::OpenCodeExited, goodbye.reason.clone()),
        )
    }

    fn transition(&mut self, status: IntegrationStatus, reason: StatusReason) -> StatusChange {
        let previous = self.status;
        let previous_reason = self.reason.clone();
        let status_changed = previous != status;
        let reason_changed = previous_reason != reason;
        self.status = status;
        if !status.is_connected() {
            self.active_session_id = None;
            self.active_session_title = None;
            self.tabs_enabled = false;
        }
        self.reason = reason.clone();
        StatusChange {
            previous,
            current: status,
            reason,
            status_changed,
            reason_changed,
        }
    }
}

/// Whether an OpenCode version string is v2 or newer.
pub fn is_supported_opencode_version(version: &str) -> bool {
    opencode_major(version).is_some_and(|major| major >= SUPPORTED_OPENCODE_MAJOR)
}

/// The only agent integration Made v1 offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentIntegration {
    /// OpenCode v2 through its TUI plugin.
    OpenCodeV2,
}

impl AgentIntegration {
    /// Stable identifier.
    pub const fn id(self) -> &'static str {
        match self {
            Self::OpenCodeV2 => "opencode-v2",
        }
    }

    /// Human-readable name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::OpenCodeV2 => "OpenCode v2",
        }
    }
}

/// Compile-time v1 scope declaration used by the scope-boundary test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeBoundary {
    /// Agent integrations offered by the app.
    pub agent_integrations: &'static [AgentIntegration],
    /// Whether a replacement AI/chat surface exists.
    pub chat_ui: bool,
    /// Whether worktree lifecycle management is exposed.
    pub worktree_management: bool,
    /// Whether a separate OpenCode server process is required.
    pub separate_opencode_server: bool,
}

/// The v1 scope boundary.
pub const V1_SCOPE: ScopeBoundary = ScopeBoundary {
    agent_integrations: &[AgentIntegration::OpenCodeV2],
    chat_ui: false,
    worktree_management: false,
    separate_opencode_server: false,
};

fn bounded_detail(detail: &str) -> String {
    const MAX_CHARS: usize = 512;
    if detail.chars().count() <= MAX_CHARS {
        return detail.to_owned();
    }
    let mut bounded: String = detail.chars().take(MAX_CHARS - 1).collect();
    bounded.push('…');
    bounded
}
