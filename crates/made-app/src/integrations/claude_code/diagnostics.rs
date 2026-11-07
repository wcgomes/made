//! Claude Code app-wide state: PATH detection for the diagnostics row.

use std::any::Any;

use crate::integration_status::ClaudeDiagnosticsPhase;
use crate::integrations::{AppCtx, AppInit, AppState, CLAUDE_CODE};

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

    /// The detection state every workspace's row shows.
    pub(crate) fn phase(&self) -> ClaudeDiagnosticsPhase {
        self.phase
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

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
