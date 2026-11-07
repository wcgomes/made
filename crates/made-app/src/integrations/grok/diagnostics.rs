//! Grok Build app-wide state: PATH detection, the shipped-plugin install and
//! the test holds for the diagnostics row.

use std::any::Any;
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::app::{GrokDiagnosticsHold, GrokDiagnosticsPhase, GrokRunningHold, WorkspaceId};
use crate::grok_plugin::{self, EnsureOutcome};
use crate::integrations::{AppCtx, AppInit, AppState, GROK};

/// Grok Build detection, install and hold state for every workspace's row.
pub(crate) struct GrokAppState {
    /// One-shot shipped grok-made install, started only after PATH detection
    /// reports grok. `None` once that worker has finished or was never started.
    plugin_receiver: Option<Receiver<EnsureOutcome>>,
    /// Install failed, so Grok session follow has no status hooks.
    hooks_unavailable: bool,
    /// The failure was found after the workspaces opened and none was told yet.
    unavailable_notice: bool,
    /// Row derived from detection and install, unless a test hold is set.
    phase: GrokDiagnosticsPhase,
    /// Set only by the test hold.
    hold: Option<GrokDiagnosticsHold>,
    /// Per-workspace running marks set only by the test hold.
    running_hold: Vec<(WorkspaceId, GrokRunningHold)>,
}

impl GrokAppState {
    pub(crate) fn new(init: &AppInit<'_>) -> Self {
        Self {
            plugin_receiver: None,
            hooks_unavailable: false,
            unavailable_notice: false,
            phase: initial_grok_phase(
                init.detecting,
                init.detection_started,
                init.catalog.contains(&GROK),
            ),
            hold: None,
            running_hold: Vec::new(),
        }
    }

    /// The phase every workspace's row shows: the hold's, when one is set.
    pub(crate) fn effective_phase(&self) -> GrokDiagnosticsPhase {
        self.hold
            .map(GrokDiagnosticsHold::phase)
            .unwrap_or(self.phase)
    }

    /// Whether a test hold replaces detection and install.
    pub(crate) fn hold_active(&self) -> bool {
        self.hold.is_some()
    }

    /// Whether the shipped plugin could not be installed.
    pub(crate) fn hooks_unavailable(&self) -> bool {
        self.hooks_unavailable
    }

    /// Whether a failed install still has to be reported to the open
    /// workspaces. Asking clears it.
    pub(crate) fn take_unavailable_notice(&mut self) -> bool {
        std::mem::take(&mut self.unavailable_notice)
    }

    /// The running mark held for one workspace.
    pub(crate) fn running_mark(&self, id: WorkspaceId) -> Option<GrokRunningHold> {
        self.running_hold
            .iter()
            .find(|(key, _)| *key == id)
            .map(|(_, hold)| *hold)
    }

    /// Hold the diagnostics row without a grok process.
    pub(crate) fn set_hold(&mut self, hold: GrokDiagnosticsHold) {
        self.hold = Some(hold);
        self.phase = hold.phase();
        self.hooks_unavailable = matches!(hold, GrokDiagnosticsHold::InstallFailed);
    }

    /// Mark whether Grok is running in one workspace.
    pub(crate) fn set_running_hold(&mut self, id: WorkspaceId, running: GrokRunningHold) {
        if let Some((_, slot)) = self.running_hold.iter_mut().find(|(key, _)| *key == id) {
            *slot = running;
        } else {
            self.running_hold.push((id, running));
        }
    }

    /// Forget the running mark of a closed workspace.
    pub(crate) fn forget_workspace(&mut self, id: WorkspaceId) {
        self.running_hold.retain(|(key, _)| *key != id);
    }
}

fn initial_grok_phase(
    detecting: bool,
    detection_started: bool,
    catalog_has_grok: bool,
) -> GrokDiagnosticsPhase {
    if detecting {
        if detection_started {
            GrokDiagnosticsPhase::Detecting
        } else {
            GrokDiagnosticsPhase::NotOnPath
        }
    } else if catalog_has_grok {
        GrokDiagnosticsPhase::NotInstalledForLaunch
    } else {
        GrokDiagnosticsPhase::NotOnPath
    }
}

impl AppState for GrokAppState {
    fn on_detection(&mut self, detected: bool, _ctx: &mut AppCtx) {
        if detected {
            self.plugin_receiver = grok_plugin::spawn_ensure();
            // `None` means this process already owns the worker. This
            // launch must not claim an install is running or ready.
            self.phase = if self.plugin_receiver.is_some() {
                GrokDiagnosticsPhase::Installing
            } else {
                GrokDiagnosticsPhase::NotInstalledForLaunch
            };
        } else {
            self.phase = GrokDiagnosticsPhase::NotOnPath;
        }
    }

    fn on_detection_failed(&mut self, ctx: &mut AppCtx) {
        if self.phase == GrokDiagnosticsPhase::Detecting {
            self.phase = GrokDiagnosticsPhase::NotOnPath;
            ctx.mark_diagnostics_changed();
        }
    }

    /// Poll the shipped grok-made install without waiting for it.
    fn poll(&mut self, ctx: &mut AppCtx) {
        if self.hold.is_some() {
            return;
        }
        let received = self
            .plugin_receiver
            .as_ref()
            .map(|receiver| receiver.try_recv());
        match received {
            Some(Ok(EnsureOutcome::Unavailable) | Err(TryRecvError::Disconnected)) => {
                self.plugin_receiver = None;
                self.hooks_unavailable = true;
                self.unavailable_notice = true;
                self.phase = GrokDiagnosticsPhase::Unavailable;
                ctx.mark_diagnostics_changed();
            }
            Some(Ok(EnsureOutcome::Installed | EnsureOutcome::Skipped)) => {
                self.plugin_receiver = None;
                self.phase = GrokDiagnosticsPhase::Ready;
                ctx.mark_diagnostics_changed();
            }
            Some(Err(TryRecvError::Empty)) => {
                ctx.request_repaint_after(crate::metrics::ACTIVE_REPAINT_INTERVAL);
            }
            None => {}
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
