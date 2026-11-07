//! Claude Code app-wide state: PATH detection for the diagnostics row.

use std::any::Any;

use crate::diagnostics::ClaudeDiagnosticsReport;
use crate::integrations::{AppCtx, AppInit, AppState, WorkspaceTabs, CLAUDE_CODE};

/// How long after Claude Code becomes present a missing channel still reads as
/// starting rather than unavailable.
pub const CLAUDE_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// What PATH detection says about Claude Code for the whole app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeDiagnosticsPhase {
    /// PATH detection is still running.
    Checking,
    /// Detection finished without finding `claude`, or an explicit catalog
    /// omits it.
    NotOnPath,
    /// `claude` was detected, or an explicit catalog lists it without
    /// detection. Plugin loading does not depend on detection.
    Available,
}

/// Inputs of the Claude Code diagnostics row for one workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaudeDiagnosticsInputs {
    /// App-wide detection state.
    pub phase: ClaudeDiagnosticsPhase,
    /// The shipped plugin is missing or unreadable.
    pub plugin_missing: bool,
    /// Claude Code is present in this workspace, whoever owns the tab label.
    pub present: bool,
    /// How long it has been present.
    pub present_for: std::time::Duration,
    /// This workspace has at least one live owned Claude Code session.
    pub live_session: bool,
    /// A session-level `app=claude-code` emission arrived since that Claude
    /// Code started. A capability probe alone does not count.
    pub protocol_observed: bool,
}

/// The Claude Code row: the first state that applies.
pub fn claude_diagnostics_row(inputs: &ClaudeDiagnosticsInputs) -> ClaudeDiagnosticsReport {
    use crate::diagnostics::DiagnosticsPill::{Degraded, Inactive, Ready};
    const HOOKS: &str = "Claude Code session-status hooks are unavailable.";
    let waiting = !inputs.live_session || !inputs.protocol_observed;
    let (pill, sentence) = match inputs.phase {
        ClaudeDiagnosticsPhase::Checking => (Inactive, "Checking for Claude Code."),
        ClaudeDiagnosticsPhase::NotOnPath => (Inactive, "Claude Code is not on PATH."),
        ClaudeDiagnosticsPhase::Available if inputs.plugin_missing => (Degraded, HOOKS),
        ClaudeDiagnosticsPhase::Available if !inputs.present => {
            (Inactive, "Claude Code is not running.")
        }
        ClaudeDiagnosticsPhase::Available if waiting && inputs.present_for < CLAUDE_GRACE => {
            (Inactive, "Claude Code is starting.")
        }
        ClaudeDiagnosticsPhase::Available if !inputs.live_session => (Degraded, HOOKS),
        ClaudeDiagnosticsPhase::Available if !inputs.protocol_observed => {
            (Degraded, "Claude Code status protocol is unavailable.")
        }
        ClaudeDiagnosticsPhase::Available => (Ready, "Claude Code session follow is ready."),
    };
    ClaudeDiagnosticsReport::new(pill, sentence)
}

/// The Claude Code row of a remote workspace: the same state machine with
/// hooks permanently unavailable over SSH, so it never reports ready. The
/// plugin and live-session inputs describe local hooks and are ignored; the
/// phase is the owning tab's own remote detection.
pub fn remote_claude_diagnostics_row(inputs: &ClaudeDiagnosticsInputs) -> ClaudeDiagnosticsReport {
    use crate::diagnostics::DiagnosticsPill::{Degraded, Inactive};
    let (pill, sentence) = match inputs.phase {
        ClaudeDiagnosticsPhase::Checking => (Inactive, "Checking for Claude Code."),
        ClaudeDiagnosticsPhase::NotOnPath => (Inactive, "Claude Code is not on PATH."),
        ClaudeDiagnosticsPhase::Available if !inputs.present => {
            (Inactive, "Claude Code is not running.")
        }
        ClaudeDiagnosticsPhase::Available
            if !inputs.protocol_observed && inputs.present_for < CLAUDE_GRACE =>
        {
            (Inactive, "Claude Code is starting.")
        }
        ClaudeDiagnosticsPhase::Available if !inputs.protocol_observed => {
            (Degraded, "Claude Code status protocol is unavailable.")
        }
        ClaudeDiagnosticsPhase::Available => (
            Degraded,
            "Claude Code session-status hooks are unavailable over SSH.",
        ),
    };
    ClaudeDiagnosticsReport::new(pill, sentence)
}

/// Claude Code PATH detection state for every workspace's diagnostics row.
pub(crate) struct ClaudeAppState {
    phase: ClaudeDiagnosticsPhase,
}

impl ClaudeAppState {
    pub(crate) fn new(init: &AppInit<'_>) -> Self {
        Self {
            phase: initial_claude_phase(
                init.detecting,
                init.detection_started,
                init.catalog.contains(&CLAUDE_CODE),
            ),
        }
    }
}

fn initial_claude_phase(
    detecting: bool,
    detection_started: bool,
    catalog_has_claude: bool,
) -> ClaudeDiagnosticsPhase {
    if detecting {
        if detection_started {
            ClaudeDiagnosticsPhase::Checking
        } else {
            ClaudeDiagnosticsPhase::NotOnPath
        }
    } else if catalog_has_claude {
        // Plugin loading does not depend on detection.
        ClaudeDiagnosticsPhase::Available
    } else {
        ClaudeDiagnosticsPhase::NotOnPath
    }
}

impl AppState for ClaudeAppState {
    fn on_detection(&mut self, detected: bool, _ctx: &mut AppCtx) {
        self.phase = if detected {
            ClaudeDiagnosticsPhase::Available
        } else {
            ClaudeDiagnosticsPhase::NotOnPath
        };
    }

    fn on_detection_failed(&mut self, ctx: &mut AppCtx) {
        if self.phase == ClaudeDiagnosticsPhase::Checking {
            self.phase = ClaudeDiagnosticsPhase::NotOnPath;
            ctx.mark_diagnostics_changed();
        }
    }

    fn publish(&self, workspace: &mut WorkspaceTabs<'_>) {
        if let Some(state) = super::tab_state_mut(workspace.list, workspace.tab_states) {
            state.set_phase(self.phase);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
