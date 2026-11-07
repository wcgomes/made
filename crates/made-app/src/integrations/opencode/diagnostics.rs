//! OpenCode app-wide state: whether OpenCode is known to be missing, and the
//! app-level events and version gate of the OpenCode status.

use std::any::Any;

use made_bridge::protocol::opencode_major;
use made_platform::AppPaths;

use crate::integration_status::{
    IntegrationState, IntegrationStatus, ReasonCode, StatusChange, StatusReason,
};
use crate::integrations::{AppCtx, AppInit, AppState, WorkspaceNotices, OPENCODE};

/// Minimum supported OpenCode major version.
pub const SUPPORTED_OPENCODE_MAJOR: u32 = 2;

/// Whether an OpenCode version string is v2 or newer.
pub fn is_supported_version(version: &str) -> bool {
    opencode_major(version).is_some_and(|major| major >= SUPPORTED_OPENCODE_MAJOR)
}

/// App-level event: OpenCode is not installed or not on PATH.
pub fn note_missing(state: &mut IntegrationState) -> StatusChange {
    state.transition(
        IntegrationStatus::Unsupported,
        StatusReason::new(ReasonCode::OpenCodeMissing),
    )
}

/// Apply the OpenCode major-version gate.
pub fn note_version(state: &mut IntegrationState, version: &str) -> StatusChange {
    if version.trim().is_empty() {
        return note_missing(state);
    }
    if !is_supported_version(version) {
        return state.transition(
            IntegrationStatus::Unsupported,
            StatusReason::with_detail(ReasonCode::OpenCodeTooOld, version),
        );
    }

    if state.status().is_unsupported()
        && matches!(
            state.reason().code,
            ReasonCode::OpenCodeMissing | ReasonCode::OpenCodeTooOld
        )
    {
        return state.transition(
            IntegrationStatus::Inactive,
            StatusReason::new(ReasonCode::Inactive),
        );
    }
    state.unchanged()
}

/// App-level event: OpenCode exited normally.
pub fn note_exited(state: &mut IntegrationState) -> StatusChange {
    state.drop_connection();
    state.transition(
        IntegrationStatus::Inactive,
        StatusReason::new(ReasonCode::OpenCodeExited),
    )
}

/// OpenCode was omitted by an explicit catalog, or PATH detection finished
/// without it.
pub(crate) struct OpenCodeAppState {
    /// Stays false while detection is still running, so the row keeps the
    /// inactive not-running sentence.
    missing: bool,
    /// Every workspace open when OpenCode became missing was told.
    announced: bool,
}

impl OpenCodeAppState {
    pub(crate) fn new(init: &AppInit<'_>) -> Self {
        // A live worker has not answered yet. A thread that never started is
        // the same as detection finishing without OpenCode.
        let missing = if init.detecting {
            !init.detection_started
        } else {
            !init.catalog.contains(&OPENCODE)
        };
        Self {
            missing,
            // No workspace exists while the model is built.
            announced: missing,
        }
    }
}

impl AppState for OpenCodeAppState {
    fn on_detection(&mut self, detected: bool, _ctx: &mut AppCtx) {
        if !detected {
            self.missing = true;
        }
    }

    fn on_detection_failed(&mut self, _ctx: &mut AppCtx) {
        self.missing = true;
    }

    /// Every workspace open when OpenCode became missing is told.
    fn take_pending_notice(&mut self) -> bool {
        let pending = self.missing && !self.announced;
        self.announced = self.missing;
        pending
    }

    /// A workspace records that OpenCode is not on PATH.
    fn notify_workspace(&self, workspace: &mut WorkspaceNotices<'_>) {
        if self.missing {
            note_missing(workspace.session);
        }
    }

    /// Where the OpenCode plugin source resolves from, and whether its package
    /// is there.
    fn smoke_marker(&self, paths: Option<&AppPaths>) -> Option<(&'static str, String)> {
        let plugin = paths
            .map(|paths| {
                let found = paths.plugin_dir.join("package.json").is_file();
                format!(
                    "{}{}",
                    paths.plugin_source.name(),
                    if found { "" } else { ":missing" }
                )
            })
            .unwrap_or_else(|| "unresolved".to_owned());
        Some(("opencode_plugin", plugin))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
