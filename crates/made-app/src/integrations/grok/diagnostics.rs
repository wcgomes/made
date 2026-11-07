//! Grok Build app-wide state: PATH detection, the shipped-plugin install and
//! the test holds for the diagnostics row.

use std::any::Any;
use std::borrow::Cow;
use std::sync::mpsc::{Receiver, TryRecvError};

use super::plugin::{self, EnsureOutcome};
use crate::app::WorkspaceId;
use crate::diagnostics::{DiagnosticsPill, LogLevel, RemoteGrokReport};
use crate::integrations::{
    AppCtx, AppInit, AppState, RowReport, WorkspaceNotices, WorkspaceTabs, GROK,
};
use made_platform::AppPaths;

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
            self.plugin_receiver = plugin::spawn_ensure();
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

    /// The tab's row: the app-wide phase, and whether Grok runs in the tab.
    fn publish(&self, workspace: &mut WorkspaceTabs<'_>) {
        let id = workspace.id;
        let target = workspace.target;
        let terminal_running = workspace.terminal_running;
        let Some(tab) = super::tab_state_mut(workspace.list, workspace.tab_states) else {
            return;
        };
        let running = tab.follow_running(target, terminal_running);
        let row = grok_row_for(self.effective_phase(), self.running_mark(id), running);
        tab.set_diagnostics(row, false);
    }

    /// A test hold replaces detection and install.
    fn holds_detection(&self) -> bool {
        self.hold.is_some()
    }

    fn on_workspace_closed(&mut self, id: WorkspaceId) {
        self.running_hold.retain(|(key, _)| *key != id);
    }

    /// A failed install still has to be reported to the open workspaces.
    fn take_pending_notice(&mut self) -> bool {
        std::mem::take(&mut self.unavailable_notice)
    }

    /// A workspace records that Grok session follow has no status hooks, once.
    ///
    /// Called when the shipped plugin could not be installed. Repeated calls
    /// keep a single warning.
    fn notify_workspace(&self, workspace: &mut WorkspaceNotices<'_>) {
        const MESSAGE: &str = "Grok session follow: session-status hooks are unavailable";
        if !self.hooks_unavailable {
            return;
        }
        if workspace.log.entries().any(|entry| {
            entry
                .message
                .contains("session-status hooks are unavailable")
        }) {
            return;
        }
        workspace.log.record(LogLevel::Warn, MESSAGE);
    }

    /// Whether the shipped `made-grok-status` hook is staged beside the
    /// plugin manifest.
    fn smoke_marker(&self, paths: Option<&AppPaths>) -> Option<(&'static str, String)> {
        let hook = format!("made-grok-status{}", std::env::consts::EXE_SUFFIX);
        let status = paths
            .and_then(|paths| paths.plugin_package_dir("grok-made"))
            .map(|dir| dir.join("hooks").join("bin").join(&hook).is_file())
            .map(|found| if found { "present" } else { "missing" })
            .unwrap_or("unresolved");
        Some(("grok_status", status.to_owned()))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Grok Build row while PATH detection and the shipped-plugin install run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GrokDiagnosticsPhase {
    /// Background PATH detection has not finished.
    Detecting,
    /// Detection finished without grok, or the catalog omits Grok.
    NotOnPath,
    /// grok was detected and the install worker has not finished.
    Installing,
    /// Install succeeded, or the installed version was already current.
    Ready,
    /// The install failed.
    Unavailable,
    /// An explicit catalog includes Grok and detection did not run.
    NotInstalledForLaunch,
}

impl GrokDiagnosticsPhase {
    const fn row(self) -> GrokDiagnosticsRow {
        match self {
            Self::Detecting => GrokDiagnosticsRow::CHECKING,
            Self::NotOnPath => GrokDiagnosticsRow::NOT_ON_PATH,
            Self::Installing => GrokDiagnosticsRow::INSTALLING,
            // Callers with a workspace use [`grok_row_for`]. This arm is only
            // the success sentence; a quiet workspace uses `NOT_RUNNING`.
            Self::Ready => GrokDiagnosticsRow::READY,
            Self::Unavailable => GrokDiagnosticsRow::HOOKS_UNAVAILABLE,
            Self::NotInstalledForLaunch => GrokDiagnosticsRow::NOT_INSTALLED,
        }
    }
}

/// Test-only hold for the Grok Build diagnostics row.
///
/// Applying a hold does not spawn grok, look up `PATH`, or install the plugin.
/// Product startup leaves it unset and keeps background detection and install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrokDiagnosticsHold {
    /// PATH detection is still running.
    Detecting,
    /// Detection finished and grok is not on PATH.
    NotOnPath,
    /// grok was detected and the shipped-plugin install is still running.
    Installing,
    /// The install command succeeded.
    Installed,
    /// The installed version is already current.
    AlreadyCurrent,
    /// The install failed.
    InstallFailed,
}

impl GrokDiagnosticsHold {
    pub(crate) const fn phase(self) -> GrokDiagnosticsPhase {
        match self {
            Self::Detecting => GrokDiagnosticsPhase::Detecting,
            Self::NotOnPath => GrokDiagnosticsPhase::NotOnPath,
            Self::Installing => GrokDiagnosticsPhase::Installing,
            Self::Installed | Self::AlreadyCurrent => GrokDiagnosticsPhase::Ready,
            Self::InstallFailed => GrokDiagnosticsPhase::Unavailable,
        }
    }
}

/// Test-only running mark for one workspace's Grok Build row.
///
/// Applying a mark does not spawn grok, look up `PATH`, or install the plugin.
/// Product startup leaves every workspace unmarked and reads that workspace's
/// terminal target and open Grok sessions. The tab label is not a mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrokRunningHold {
    /// This workspace's terminal target is Grok and that process is still alive.
    LivingTarget,
    /// This workspace's shell has at least one open Grok session.
    ///
    /// The target is not a living Grok process.
    OpenSession,
    /// Neither a living Grok target nor an open Grok session.
    NotRunning,
}

/// Row for one workspace. Install success is ready only while Grok is running
/// there, as the workspace's own state (`running`) or a test mark says.
pub(crate) fn grok_row_for(
    phase: GrokDiagnosticsPhase,
    mark: Option<GrokRunningHold>,
    running: bool,
) -> GrokDiagnosticsRow {
    if phase != GrokDiagnosticsPhase::Ready {
        return phase.row();
    }
    let running = match mark {
        Some(GrokRunningHold::LivingTarget | GrokRunningHold::OpenSession) => true,
        Some(GrokRunningHold::NotRunning) => false,
        None => running,
    };
    if running {
        GrokDiagnosticsRow::READY
    } else {
        GrokDiagnosticsRow::NOT_RUNNING
    }
}

/// Current Grok Build pill and the one sentence shown beside it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct GrokDiagnosticsRow {
    pub(crate) pill: DiagnosticsPill,
    /// Fixed product wording, or the owning tab's bounded failure detail.
    pub(crate) sentence: Cow<'static, str>,
}

impl GrokDiagnosticsRow {
    pub(crate) const CHECKING: Self = Self {
        pill: DiagnosticsPill::Inactive,
        sentence: Cow::Borrowed("Checking for Grok Build."),
    };
    pub(crate) const NOT_ON_PATH: Self = Self {
        pill: DiagnosticsPill::Inactive,
        sentence: Cow::Borrowed("Grok Build is not on PATH."),
    };
    pub(crate) const INSTALLING: Self = Self {
        pill: DiagnosticsPill::Inactive,
        sentence: Cow::Borrowed("Installing the Grok Build plugin."),
    };
    pub(crate) const READY: Self = Self {
        pill: DiagnosticsPill::Ready,
        sentence: Cow::Borrowed("Grok Build session follow is ready."),
    };
    pub(crate) const NOT_RUNNING: Self = Self {
        pill: DiagnosticsPill::Inactive,
        sentence: Cow::Borrowed("Grok Build is not running."),
    };
    pub(crate) const HOOKS_UNAVAILABLE: Self = Self {
        pill: DiagnosticsPill::Degraded,
        sentence: Cow::Borrowed("Grok Build session-status hooks are unavailable."),
    };
    pub(crate) const NOT_INSTALLED: Self = Self {
        pill: DiagnosticsPill::Inactive,
        sentence: Cow::Borrowed("Grok Build plugin was not installed for this launch."),
    };
    pub(crate) const REMOTE_NOT_DETECTED: Self = Self {
        pill: DiagnosticsPill::Inactive,
        sentence: Cow::Borrowed("Grok Build is not detected on this server."),
    };
    pub(crate) const REMOTE_INSTALLING: Self = Self {
        pill: DiagnosticsPill::Inactive,
        sentence: Cow::Borrowed("Installing the Grok Build plugin on this server."),
    };
    pub(crate) const REMOTE_HOOKS_NOT_APPROVED: Self = Self {
        pill: DiagnosticsPill::Degraded,
        sentence: Cow::Borrowed("Remote Grok Build session-status hooks are unavailable: the plugin install was not approved."),
    };
    pub(crate) const REMOTE_INSTALL_PENDING: Self = Self {
        pill: DiagnosticsPill::Degraded,
        sentence: Cow::Borrowed("Remote Grok Build session-status hooks are unavailable: the plugin install has not run on this server yet."),
    };
    pub(crate) const REMOTE_INSTALL_FAILED: Self = Self {
        pill: DiagnosticsPill::Degraded,
        sentence: Cow::Borrowed("Remote Grok Build session-status hooks are unavailable: the plugin install failed on this server."),
    };
    pub(crate) const REMOTE_TRANSPORT_DOWN: Self = Self {
        pill: DiagnosticsPill::Degraded,
        sentence: Cow::Borrowed("The SSH transport is disconnected; the remote Grok Build state is unknown until reattach."),
    };
    pub(crate) const REMOTE_UNKNOWN: Self = Self {
        pill: DiagnosticsPill::Inactive,
        sentence: Cow::Borrowed("The remote Grok Build state is not known yet."),
    };
}

impl From<GrokDiagnosticsRow> for RowReport {
    fn from(row: GrokDiagnosticsRow) -> Self {
        Self {
            pill: row.pill,
            sentence: row.sentence,
        }
    }
}

impl From<GrokDiagnosticsRow> for RemoteGrokReport {
    fn from(row: GrokDiagnosticsRow) -> Self {
        Self {
            pill: row.pill,
            sentence: row.sentence.into_owned(),
        }
    }
}
