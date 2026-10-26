//! Focus request dispatch and failure handling for the session map.
//!
//! The controller sends one focus request at a time through a [`FocusSink`]
//! (the bridge in production), correlates the result by request id, enforces a
//! deadline, and turns failures into a non-blocking error plus a fresh session
//! state request.
//!
//! The module has no UI dependency and no timer thread. The app calls
//! [`SessionFocusController::tick`] from its event loop; tests can pass an
//! explicit instant to [`SessionFocusController::poll_timeout`].

use std::error::Error;
use std::fmt;
use std::time::{Duration, Instant};

use made_bridge::security::FOCUS_TIMEOUT;
use made_bridge::server::{BridgeHandle, FocusResult};

/// Default deadline for a focus result.
pub const DEFAULT_FOCUS_TIMEOUT: Duration = FOCUS_TIMEOUT;

/// Transport used to send focus and state-refresh commands.
pub trait FocusSink {
    /// Ask OpenCode to activate a session and return the request identifier.
    fn send_focus(&self, session_id: &str) -> Result<String, String>;

    /// Ask the plugin for a fresh full snapshot and return its request id.
    fn request_state(&self) -> Result<String, String>;
}

impl FocusSink for BridgeHandle {
    fn send_focus(&self, session_id: &str) -> Result<String, String> {
        BridgeHandle::focus(self, session_id).map_err(|error| error.to_string())
    }

    fn request_state(&self) -> Result<String, String> {
        BridgeHandle::request_state(self).map_err(|error| error.to_string())
    }
}

/// A focus request awaiting its result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingFocus {
    /// Correlation identifier returned by the sink.
    pub request_id: String,
    /// Session the request targets.
    pub session_id: String,
    /// Instant after which the request is timed out.
    pub deadline: Instant,
}

/// Focus failure category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusErrorKind {
    /// The command could not be queued or sent.
    SendFailed,
    /// The plugin reported a failure.
    Failed,
    /// No result arrived before the deadline.
    TimedOut,
    /// The connection closed while the request was pending.
    Cancelled,
}

impl FocusErrorKind {
    /// Stable machine-readable code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::SendFailed => "focus_send_failed",
            Self::Failed => "focus_failed",
            Self::TimedOut => "focus_timed_out",
            Self::Cancelled => "focus_cancelled",
        }
    }
}

/// A non-blocking focus failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusError {
    /// Failure category.
    pub kind: FocusErrorKind,
    /// Correlated request id, when one was issued.
    pub request_id: Option<String>,
    /// Session that was targeted.
    pub session_id: Option<String>,
    /// Human-readable detail.
    pub message: String,
}

impl FocusError {
    fn new(
        kind: FocusErrorKind,
        request_id: Option<String>,
        session_id: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            request_id,
            session_id,
            message: message.into(),
        }
    }
}

impl fmt::Display for FocusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.kind.code(), self.message)
    }
}

impl Error for FocusError {}

/// Outcome classification for one controller operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusOutcome {
    /// Nothing was pending and nothing happened.
    Idle,
    /// Another request is already pending.
    Busy,
    /// A request was accepted by the sink.
    Started,
    /// The focus request succeeded.
    Succeeded,
    /// The plugin reported a failure.
    Failed,
    /// The request deadline elapsed.
    TimedOut,
    /// The result did not match the pending request.
    Uncorrelated,
    /// The sink refused the request.
    SendFailed,
}

/// State refresh triggered by a focus failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateRefresh {
    /// No refresh was needed.
    NotNeeded,
    /// A state request was sent and returned this request id.
    Requested(String),
    /// A refresh was attempted but failed.
    Failed(String),
}

/// The result of one controller operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusUpdate {
    /// What happened.
    pub outcome: FocusOutcome,
    /// Correlation identifier involved, when known.
    pub request_id: Option<String>,
    /// Session involved, when known.
    pub session_id: Option<String>,
    /// Failure detail, when the operation failed.
    pub error: Option<FocusError>,
    /// State refresh side effect.
    pub state_refresh: StateRefresh,
    /// Request still pending after this operation.
    pub in_flight: Option<PendingFocus>,
}

impl FocusUpdate {
    /// Whether the operation created a pending request.
    pub const fn is_pending(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Whether a state refresh was triggered.
    pub const fn state_refresh_triggered(&self) -> bool {
        matches!(
            self.state_refresh,
            StateRefresh::Requested(_) | StateRefresh::Failed(_)
        )
    }
}

/// Correlates focus requests and their outcomes.
#[derive(Debug, Clone)]
pub struct SessionFocusController {
    pending: Option<PendingFocus>,
    last_error: Option<FocusError>,
    timeout: Duration,
}

impl Default for SessionFocusController {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionFocusController {
    /// Controller with the production timeout.
    pub fn new() -> Self {
        Self::with_timeout(DEFAULT_FOCUS_TIMEOUT)
    }

    /// Controller with an explicit timeout. Intended for tests and
    /// diagnostics.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            pending: None,
            last_error: None,
            timeout,
        }
    }

    /// Configured focus deadline.
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Request currently awaiting a result.
    pub fn in_flight(&self) -> Option<&PendingFocus> {
        self.pending.as_ref()
    }

    /// Whether a request is awaiting a result.
    pub fn is_in_flight(&self) -> bool {
        self.pending.is_some()
    }

    /// Last failure, cleared by a successful focus result.
    pub fn last_error(&self) -> Option<&FocusError> {
        self.last_error.as_ref()
    }

    /// Clear the last failure.
    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    /// Send a focus request now.
    pub fn request(&mut self, sink: &dyn FocusSink, session_id: impl Into<String>) -> FocusUpdate {
        self.request_at(sink, session_id, Instant::now())
    }

    /// Send a focus request with an explicit start instant.
    pub fn request_at(
        &mut self,
        sink: &dyn FocusSink,
        session_id: impl Into<String>,
        now: Instant,
    ) -> FocusUpdate {
        let session_id = session_id.into();
        if let Some(pending) = &self.pending {
            return FocusUpdate {
                outcome: FocusOutcome::Busy,
                request_id: Some(pending.request_id.clone()),
                session_id: Some(pending.session_id.clone()),
                error: None,
                state_refresh: StateRefresh::NotNeeded,
                in_flight: Some(pending.clone()),
            };
        }
        if session_id.is_empty() {
            let error = FocusError::new(
                FocusErrorKind::SendFailed,
                None,
                Some(session_id.clone()),
                "session id is empty",
            );
            self.last_error = Some(error.clone());
            return self.failure_update(FocusOutcome::SendFailed, error, sink);
        }

        match sink.send_focus(&session_id) {
            Ok(request_id) => {
                let pending = PendingFocus {
                    request_id: request_id.clone(),
                    session_id: session_id.clone(),
                    deadline: now.checked_add(self.timeout).unwrap_or(now),
                };
                self.pending = Some(pending.clone());
                self.last_error = None;
                FocusUpdate {
                    outcome: FocusOutcome::Started,
                    request_id: Some(request_id),
                    session_id: Some(session_id),
                    error: None,
                    state_refresh: StateRefresh::NotNeeded,
                    in_flight: Some(pending),
                }
            }
            Err(message) => {
                let error =
                    FocusError::new(FocusErrorKind::SendFailed, None, Some(session_id), message);
                self.last_error = Some(error.clone());
                self.failure_update(FocusOutcome::SendFailed, error, sink)
            }
        }
    }

    /// Apply a bridge focus result.
    pub fn handle_result(&mut self, sink: &dyn FocusSink, result: FocusResult) -> FocusUpdate {
        let Some(pending) = self.pending.clone() else {
            return FocusUpdate {
                outcome: FocusOutcome::Uncorrelated,
                request_id: Some(result.request_id),
                session_id: result.session_id,
                error: None,
                state_refresh: StateRefresh::NotNeeded,
                in_flight: None,
            };
        };
        if pending.request_id != result.request_id {
            return FocusUpdate {
                outcome: FocusOutcome::Uncorrelated,
                request_id: Some(result.request_id),
                session_id: result.session_id,
                error: None,
                state_refresh: StateRefresh::NotNeeded,
                in_flight: Some(pending),
            };
        }

        self.pending = None;
        if result.ok {
            self.last_error = None;
            return FocusUpdate {
                outcome: FocusOutcome::Succeeded,
                request_id: Some(pending.request_id),
                session_id: Some(pending.session_id),
                error: None,
                state_refresh: StateRefresh::NotNeeded,
                in_flight: None,
            };
        }

        let error = FocusError::new(
            FocusErrorKind::Failed,
            Some(pending.request_id),
            Some(pending.session_id),
            result
                .error
                .unwrap_or_else(|| "focus request failed".to_owned()),
        );
        self.last_error = Some(error.clone());
        self.failure_update(FocusOutcome::Failed, error, sink)
    }

    /// Expire the pending request if its deadline has passed.
    pub fn poll_timeout(&mut self, sink: &dyn FocusSink, now: Instant) -> FocusUpdate {
        let Some(pending) = self.pending.clone() else {
            return FocusUpdate {
                outcome: FocusOutcome::Idle,
                request_id: None,
                session_id: None,
                error: None,
                state_refresh: StateRefresh::NotNeeded,
                in_flight: None,
            };
        };
        if now < pending.deadline {
            return FocusUpdate {
                outcome: FocusOutcome::Idle,
                request_id: Some(pending.request_id.clone()),
                session_id: Some(pending.session_id.clone()),
                error: None,
                state_refresh: StateRefresh::NotNeeded,
                in_flight: Some(pending),
            };
        }

        self.pending = None;
        let error = FocusError::new(
            FocusErrorKind::TimedOut,
            Some(pending.request_id),
            Some(pending.session_id),
            "focus request timed out",
        );
        self.last_error = Some(error.clone());
        self.failure_update(FocusOutcome::TimedOut, error, sink)
    }

    /// Convenience wrapper around [`Self::poll_timeout`] using the current
    /// instant.
    pub fn tick(&mut self, sink: &dyn FocusSink) -> FocusUpdate {
        self.poll_timeout(sink, Instant::now())
    }

    /// Cancel the pending request because the connection closed.
    ///
    /// No state refresh is attempted on a closed bridge.
    pub fn cancel(&mut self) -> Option<FocusError> {
        let pending = self.pending.take()?;
        let error = FocusError::new(
            FocusErrorKind::Cancelled,
            Some(pending.request_id),
            Some(pending.session_id),
            "connection closed before focus completed",
        );
        self.last_error = Some(error.clone());
        Some(error)
    }

    fn failure_update(
        &self,
        outcome: FocusOutcome,
        error: FocusError,
        sink: &dyn FocusSink,
    ) -> FocusUpdate {
        let state_refresh = match sink.request_state() {
            Ok(request_id) => StateRefresh::Requested(request_id),
            Err(message) => StateRefresh::Failed(message),
        };
        FocusUpdate {
            outcome,
            request_id: error.request_id.clone(),
            session_id: error.session_id.clone(),
            error: Some(error),
            state_refresh,
            in_flight: self.pending.clone(),
        }
    }
}
