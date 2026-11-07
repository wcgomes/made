//! OpenCode state of one tab: the loopback bridge, its credential, the
//! captured user configuration and the composed configuration of the launch.
//!
//! The tab starts the bridge and composes the configuration before the
//! terminal starts, and reads the bridge events at the session-channel
//! position of its tick; this state only owns what those steps produce.

use std::any::Any;

use made_bridge::config::CONFIG_CONTENT_ENV;
use made_bridge::config_guard::ConfigGuard;
use made_bridge::security::Token;
use made_bridge::server::BridgeHandle;

use crate::integrations::{
    EnvContribution, LaunchCtx, Presentation, PresentationCtx, TabState, OPENCODE,
};
use crate::session_map::SessionRow;
use crate::tui::TerminalTarget;

/// OpenCode state of one tab.
#[derive(Default)]
pub(crate) struct OpenCodeTabState {
    bridge: Option<BridgeHandle>,
    /// Credential of the bridge, kept to redact it from text shown or saved.
    token: Option<Token>,
    /// The user's configuration as captured before the session.
    config_guard: Option<ConfigGuard>,
    /// The composed configuration for the next launch, never persisted.
    composed: Option<String>,
}

impl OpenCodeTabState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The tab's bridge, once it started.
    pub(crate) fn bridge(&self) -> Option<&BridgeHandle> {
        self.bridge.as_ref()
    }

    /// The bridge credential, while the tab runs.
    pub(crate) fn token(&self) -> Option<&Token> {
        self.token.as_ref()
    }

    /// Keep the capture of the user's configuration taken before the session.
    pub(crate) fn set_config_guard(&mut self, guard: Option<ConfigGuard>) {
        self.config_guard = guard;
    }

    /// Keep the started bridge, its credential and the configuration composed
    /// for it.
    pub(crate) fn set_started(&mut self, token: Token, bridge: BridgeHandle, composed: String) {
        self.token = Some(token);
        self.bridge = Some(bridge);
        self.composed = Some(composed);
    }

    /// The bridge, handed over for shutdown.
    pub(crate) fn take_bridge(&mut self) -> Option<BridgeHandle> {
        self.bridge.take()
    }

    /// The capture of the user's configuration, handed over for verification.
    pub(crate) fn take_config_guard(&mut self) -> Option<ConfigGuard> {
        self.config_guard.take()
    }

    /// Forget the credential.
    pub(crate) fn clear_token(&mut self) {
        self.token = None;
    }
}

impl TabState for OpenCodeTabState {
    /// OpenCode brands a local tab while its bridge is connected or its direct
    /// TUI runs, with the sessions of the bridge. A remote tab's branding
    /// comes from its own server state.
    fn presentation(&self, ctx: &PresentationCtx<'_>) -> Option<Presentation> {
        if ctx.remote {
            return None;
        }
        crate::ui::workspace_opencode_active(ctx.target, ctx.connected, ctx.terminal_running).then(
            || Presentation {
                dots: ctx
                    .session_map
                    .rows()
                    .iter()
                    .map(SessionRow::activity)
                    .collect(),
                title: None,
            },
        )
    }

    /// The composed configuration, for the targets the policy covers. The
    /// content is handed over once.
    fn prepare_launch(
        &mut self,
        target: TerminalTarget,
        _ctx: &mut LaunchCtx<'_>,
    ) -> EnvContribution {
        let composed = self.composed.take();
        match composed {
            Some(content) if OPENCODE.env_policy().covers(OPENCODE, target) => EnvContribution {
                set: vec![(CONFIG_CONTENT_ENV.into(), content.into())],
                strip: Vec::new(),
            },
            _ => EnvContribution::default(),
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
