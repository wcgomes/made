//! OpenCode v2 integration.

pub mod diagnostics;
pub(crate) mod launch;
pub(crate) mod tab;

use super::{
    AppInit, AppState, Descriptor, DiagnosticsDecl, EnvContribution, EnvPolicy, FollowKind,
    Integration, Mark, Rank, TabInit, TabState, OPENCODE,
};
use crate::tui::TerminalTarget;
use diagnostics::OpenCodeAppState;
pub(crate) use tab::OpenCodeTabState;
pub use tab::MAX_USER_CONFIG_BYTES;

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

    fn configures_launch(&self, contribution: &EnvContribution) -> bool {
        launch::composes(contribution)
    }

    fn supports_peer_version(&self, version: &str) -> bool {
        diagnostics::is_supported_version(version)
    }
}

/// Whether OpenCode is active in a workspace tab.
///
/// A connected bridge covers both a directly launched TUI and a shell where
/// the user ran `opencode` manually. A directly launched TUI also counts while
/// its process is alive, even before the bridge handshakes.
pub(crate) fn active(target: TerminalTarget, connected: bool, running: bool) -> bool {
    connected || (target == TerminalTarget::Tui(OPENCODE) && running)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_tab_icon_follows_connection_or_a_live_direct_tui() {
        let opencode = TerminalTarget::Tui(OPENCODE);
        assert!(
            !active(TerminalTarget::Shell, false, true),
            "a running shell is not OpenCode"
        );
        assert!(
            active(TerminalTarget::Shell, true, false),
            "a connected integration covers a manually launched OpenCode"
        );
        assert!(
            active(opencode, false, true),
            "a directly launched TUI shows the icon before the bridge connects"
        );
        assert!(
            !active(opencode, false, false),
            "the icon disappears when a direct TUI exits or the integration disconnects"
        );
    }
}
