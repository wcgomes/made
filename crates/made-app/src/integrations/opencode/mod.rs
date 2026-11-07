//! OpenCode v2 integration.

pub(crate) mod diagnostics;
pub(crate) mod tab;

use super::{
    AppInit, AppState, Descriptor, DiagnosticsDecl, EnvPolicy, FollowKind, Integration,
    IntegrationList, Mark, Rank, Slots, TabInit, TabState, OPENCODE,
};
use crate::tui::TerminalTarget;
use diagnostics::OpenCodeAppState;
use made_bridge::security::Token;
use made_bridge::server::BridgeHandle;
pub(crate) use tab::OpenCodeTabState;

/// OpenCode v2, the owner of the bridge session channel.
pub(super) struct OpenCode;

static DESCRIPTOR: Descriptor = Descriptor {
    key: "opencode",
    program: "opencode",
    label: "OpenCode",
    menu_item_id: "opencode",
    scope_id: "opencode-v2",
    scope_name: "OpenCode v2",
    root_source_id: Some("opencode"),
    mark: Mark::Vendored("opencode.svg"),
    rank: Rank(100),
};

impl Integration for OpenCode {
    fn descriptor(&self) -> &'static Descriptor {
        &DESCRIPTOR
    }

    fn follow(&self) -> FollowKind {
        FollowKind::SessionChannel
    }

    fn env_policy(&self) -> EnvPolicy {
        EnvPolicy {
            shell: true,
            own_direct: true,
            strip: &[],
        }
    }

    fn new_app_state(&self, init: &AppInit<'_>) -> Option<Box<dyn AppState>> {
        Some(Box::new(OpenCodeAppState::new(init)))
    }

    fn new_tab_state(
        &self,
        _target: TerminalTarget,
        _init: &TabInit<'_>,
    ) -> Option<Box<dyn TabState>> {
        Some(Box::new(OpenCodeTabState::new()))
    }

    fn diagnostics(&self) -> DiagnosticsDecl {
        DiagnosticsDecl::Primary
    }
}

/// The OpenCode state of a tab built over `list`.
pub(crate) fn tab_state(
    list: IntegrationList,
    slots: &Slots<Box<dyn TabState>>,
) -> Option<&OpenCodeTabState> {
    super::tab_state::<OpenCodeTabState>(list, slots, OPENCODE)
}

/// The OpenCode state of a tab built over `list`, mutably.
pub(crate) fn tab_state_mut(
    list: IntegrationList,
    slots: &mut Slots<Box<dyn TabState>>,
) -> Option<&mut OpenCodeTabState> {
    super::tab_state_mut::<OpenCodeTabState>(list, slots, OPENCODE)
}

/// The bridge of a tab built over `list`, once it started.
pub(crate) fn bridge(
    list: IntegrationList,
    slots: &Slots<Box<dyn TabState>>,
) -> Option<&BridgeHandle> {
    tab_state(list, slots).and_then(OpenCodeTabState::bridge)
}

/// The bridge credential of a tab built over `list`, while it runs.
pub(crate) fn token(list: IntegrationList, slots: &Slots<Box<dyn TabState>>) -> Option<&Token> {
    tab_state(list, slots).and_then(OpenCodeTabState::token)
}

/// The OpenCode state of an app model built over `list`.
pub(crate) fn app_state(
    list: IntegrationList,
    slots: &Slots<Box<dyn AppState>>,
) -> Option<&OpenCodeAppState> {
    super::app_state::<OpenCodeAppState>(list, slots, OPENCODE)
}

/// The OpenCode state of an app model built over `list`, mutably.
pub(crate) fn app_state_mut(
    list: IntegrationList,
    slots: &mut Slots<Box<dyn AppState>>,
) -> Option<&mut OpenCodeAppState> {
    super::app_state_mut::<OpenCodeAppState>(list, slots, OPENCODE)
}
