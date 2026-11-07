//! Grok Build integration.

pub(crate) mod diagnostics;
pub(crate) mod tab;

use super::{
    AppInit, AppState, Descriptor, DiagnosticsDecl, FollowKind, FollowPolicy, Integration,
    IntegrationList, Mark, OnUnusable, Rank, RowDecl, Slots, TabInit, TabState, WhileOutranked,
    GROK,
};
use crate::diagnostics::GrokDiagnosticsRow;
use crate::tui::TerminalTarget;
use diagnostics::GrokAppState;
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

/// The Grok state of an app model built over `list`.
pub(crate) fn app_state(
    list: IntegrationList,
    slots: &Slots<Box<dyn AppState>>,
) -> Option<&GrokAppState> {
    super::app_state::<GrokAppState>(list, slots, GROK)
}

/// The Grok state of an app model built over `list`, mutably.
pub(crate) fn app_state_mut(
    list: IntegrationList,
    slots: &mut Slots<Box<dyn AppState>>,
) -> Option<&mut GrokAppState> {
    super::app_state_mut::<GrokAppState>(list, slots, GROK)
}
