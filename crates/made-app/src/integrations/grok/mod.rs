//! Grok Build integration.

pub(crate) mod diagnostics;
pub(crate) mod follow;
pub(crate) mod plugin;
pub(crate) mod tab;

use super::{
    AppInit, AppState, Descriptor, DiagnosticsDecl, FollowKind, FollowPolicy, Integration,
    IntegrationList, Mark, OnUnusable, Rank, RowDecl, Slots, TabInit, TabState, WhileOutranked,
    GROK,
};
use crate::app::{AppModel, WorkspaceId};
use crate::tui::TerminalTarget;
use diagnostics::{GrokAppState, GrokDiagnosticsRow};
pub use diagnostics::{GrokDiagnosticsHold, GrokRunningHold};
pub(crate) use tab::GrokTabState;

/// Grok Build, started directly as `grok dashboard`.
pub(super) struct Grok;

static DESCRIPTOR: Descriptor = Descriptor {
    key: "grok",
    program: "grok",
    label: "Grok Build",
    menu_item_id: "grok",
    scope_id: "grok-build",
    scope_name: "Grok Build",
    root_source_id: Some("grok"),
    mark: Mark::Image {
        uri: "bytes://grok.svg",
        bytes: include_bytes!("../../../assets/grok.svg"),
    },
    rank: Rank(300),
};

impl Integration for Grok {
    fn descriptor(&self) -> &'static Descriptor {
        &DESCRIPTOR
    }

    fn follow(&self) -> FollowKind {
        FollowKind::Directory(FollowPolicy {
            on_unusable: OnUnusable::Ignore,
            under_session_channel: WhileOutranked::Ignore,
            under_higher_directory: WhileOutranked::Retain,
        })
    }

    fn launch_args(&self) -> &'static [&'static str] {
        &["dashboard"]
    }

    fn new_app_state(&self, init: &AppInit<'_>) -> Option<Box<dyn AppState>> {
        Some(Box::new(GrokAppState::new(init)))
    }

    fn new_tab_state(
        &self,
        _target: TerminalTarget,
        _init: &TabInit<'_>,
    ) -> Option<Box<dyn TabState>> {
        Some(Box::new(GrokTabState::new()))
    }

    fn diagnostics(&self) -> DiagnosticsDecl {
        DiagnosticsDecl::Row(RowDecl {
            status_key: "grok status",
            sentence_key: "grok",
            initial: GrokDiagnosticsRow::NOT_ON_PATH.into(),
        })
    }
}

/// Whether a non-OpenCode tab shows the Grok Build label and mark.
///
/// A directly launched Grok TUI counts while its process is alive, even
/// before a session exists. A shell tab counts while that terminal still has
/// open Grok sessions. A remote tab counts the same way from its own server
/// state: a directly launched remote grok (`TerminalTarget::RemoteGrok`) or
/// any owned open Grok session in the tab's projection. An active OpenCode
/// tab never does: its label, icon, and dots stay OpenCode's.
pub(crate) fn workspace_grok_branded(
    opencode: bool,
    target: TerminalTarget,
    running: bool,
    open_grok_sessions: usize,
) -> bool {
    !opencode
        && (open_grok_sessions > 0
            || ((target == TerminalTarget::Tui(GROK)
                || matches!(target, TerminalTarget::RemoteGrok { .. }))
                && running))
}

/// Official Grok mark from `assets/grok.svg`, outside the Material icon
/// inventory.
pub(crate) fn mark() -> Mark {
    DESCRIPTOR.mark
}

/// The Grok state of a tab built over `list`.
#[cfg(test)]
pub(crate) fn tab_state(
    list: IntegrationList,
    slots: &Slots<Box<dyn TabState>>,
) -> Option<&GrokTabState> {
    super::tab_state::<GrokTabState>(list, slots, GROK)
}

/// The Grok state of a tab built over `list`, mutably.
pub(crate) fn tab_state_mut(
    list: IntegrationList,
    slots: &mut Slots<Box<dyn TabState>>,
) -> Option<&mut GrokTabState> {
    super::tab_state_mut::<GrokTabState>(list, slots, GROK)
}

/// Hold the Grok Build diagnostics row without a grok process.
///
/// The hold replaces detection and install until the model is dropped. It
/// does not spawn grok, wait on `PATH`, or start the shipped-plugin install.
/// Background detection and install still run when no hold is set.
/// [`GrokDiagnosticsHold::Installed`] and [`GrokDiagnosticsHold::AlreadyCurrent`]
/// are ready only while that workspace's Grok is running.
#[doc(hidden)]
pub fn hold_diagnostics(model: &mut AppModel, hold: GrokDiagnosticsHold) {
    let list = model.integrations();
    if let Some(state) = super::app_state_mut::<GrokAppState>(list, model.app_states_mut(), GROK) {
        state.set_hold(hold);
    }
    model.push_integration_diagnostics();
}

/// Mark whether Grok is running in one workspace, without a grok process.
///
/// The mark stays until replaced. It does not spawn grok, look up `PATH`,
/// or install the plugin, and it does not change the tab label. Checking,
/// not on PATH, installing, hooks unavailable, and the explicit-catalog
/// sentence ignore it.
#[doc(hidden)]
pub fn hold_workspace_running(model: &mut AppModel, id: WorkspaceId, running: GrokRunningHold) {
    let list = model.integrations();
    if let Some(state) = super::app_state_mut::<GrokAppState>(list, model.app_states_mut(), GROK) {
        state.set_running_hold(id, running);
    }
    model.push_integration_diagnostics();
}
