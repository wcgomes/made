//! OpenCode state of one tab: the loopback bridge, its credential, the
//! captured user configuration and the composed configuration of the launch.
//!
//! The tab starts the bridge and composes the configuration before the
//! terminal starts, and reads the bridge events at the session-channel
//! position of its tick; this state owns what those steps produce.

use std::any::Any;
use std::path::Path;

use made_bridge::config::{self, ComposeOptions};
use made_bridge::config_guard::{ConfigFileState, ConfigGuard, ConfigVerification};
use made_bridge::security::Token;
use made_bridge::server::{self, BridgeHandle, BridgeOptions};

use super::launch;
use crate::diagnostics::{DiagnosticLog, LogLevel};
use crate::integrations::{
    EnvContribution, LaunchCtx, Presentation, PresentationCtx, StartCtx, TabState,
};
use crate::session_map::SessionRow;
use crate::tui::TerminalTarget;
use crate::APP_VERSION;

/// Maximum size of the user-owned OpenCode config Made is willing to read on
/// the workspace preparation path. Larger or special files are skipped rather
/// than read, so opening a workspace cannot stall behind an unbounded read.
pub const MAX_USER_CONFIG_BYTES: u64 = 1024 * 1024;

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

    /// Keep the capture of the user's configuration taken before the session.
    fn set_config_guard(&mut self, guard: Option<ConfigGuard>) {
        self.config_guard = guard;
    }

    /// Keep the started bridge, its credential and the configuration composed
    /// for it.
    fn set_started(&mut self, token: Token, bridge: BridgeHandle, composed: String) {
        self.token = Some(token);
        self.bridge = Some(bridge);
        self.composed = Some(composed);
    }

    /// Forget the credential and the composed configuration that carries it.
    fn clear_token(&mut self) {
        self.token = None;
        self.composed = None;
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
        super::active(ctx.target, ctx.connected, ctx.terminal_running).then(|| Presentation {
            dots: ctx
                .session_map
                .rows()
                .iter()
                .map(SessionRow::activity)
                .collect(),
            title: None,
        })
    }

    /// The composed configuration, for the targets the policy covers. The
    /// content is handed over once.
    fn prepare_launch(
        &mut self,
        _target: TerminalTarget,
        covered: bool,
        _ctx: &mut LaunchCtx<'_>,
    ) -> EnvContribution {
        launch::contribution(self.composed.take(), covered)
    }

    /// Start the bridge and compose the per-launch OpenCode configuration.
    ///
    /// The user's configuration is read only. A composition failure leaves the
    /// child environment untouched and reports a degraded status. The composed
    /// content waits in this state for the launch preparation and is never
    /// persisted.
    fn start(&mut self, ctx: &mut StartCtx<'_>) {
        let Some(paths) = ctx.paths else {
            *ctx.failure = Some("app-owned paths are unavailable".to_owned());
            ctx.session
                .note_config_composition_failed("app-owned paths are unavailable");
            ctx.diag.log_mut().record(
                LogLevel::Error,
                "configuration not composed: app-owned paths are unavailable",
            );
            return;
        };

        // Inspect the user's config before any read. Only a regular file within
        // the bounded limit is captured or merged; FIFOs, devices, directories,
        // and oversized files are skipped so workspace preparation cannot
        // block on an unbounded or blocking read.
        let user_config = match bounded_user_config(&paths.opencode_config_path) {
            Ok(()) => Some(paths.opencode_config_path.clone()),
            Err(detail) => {
                *ctx.failure = Some(detail.clone());
                ctx.session.note_config_composition_failed(detail.clone());
                ctx.diag.log_mut().record(LogLevel::Error, detail);
                None
            }
        };

        // Capture the user's config before the session, even if composition
        // later fails. Verification never rewrites the file.
        let guard = if let Some(path) = user_config.as_deref() {
            match ConfigGuard::before_session(path) {
                Ok(guard) => {
                    let state = guard.before().state();
                    ctx.diag.log_mut().record(
                        LogLevel::Info,
                        format!(
                            "user OpenCode config captured ({})",
                            match state {
                                ConfigFileState::Present => "present",
                                ConfigFileState::Missing => "missing",
                            }
                        ),
                    );
                    Some(guard)
                }
                Err(error) => {
                    ctx.diag.log_mut().record(
                        LogLevel::Warn,
                        format!("could not capture user OpenCode config: {error}"),
                    );
                    None
                }
            }
        } else {
            None
        };
        self.set_config_guard(guard);

        let pane_id = ctx.pane_id.to_owned();
        let token = match Token::generate() {
            Ok(token) => token,
            Err(error) => {
                let detail = format!("bridge token generation failed: {error}");
                *ctx.failure = Some(detail.clone());
                ctx.session.note_config_composition_failed(detail);
                ctx.diag
                    .log_mut()
                    .record(LogLevel::Error, "bridge token generation failed");
                return;
            }
        };
        *ctx.diag = ctx.diag.clone().with_token_fingerprint(token.fingerprint());

        let bridge = match server::start(
            BridgeOptions::new(token.clone(), pane_id.clone()).with_app_version(APP_VERSION),
        ) {
            Ok(bridge) => bridge,
            Err(error) => {
                let detail = format!("bridge could not start: {error}");
                ctx.session.note_plugin_unavailable(detail.clone());
                ctx.diag
                    .log_mut()
                    .record(LogLevel::Error, "loopback bridge could not start");
                *ctx.failure = Some(detail);
                return;
            }
        };

        let options = ComposeOptions {
            plugin_dir: paths.plugin_dir.clone(),
            app_owned_root: paths.plugin_root.clone(),
            user_config_path: user_config,
            inline_config: None,
            bridge_url: bridge.url(),
            token: token.clone(),
            pane_id: pane_id.clone(),
            app_version: APP_VERSION.to_owned(),
        }
        .with_inline_config_from_env();

        let composed = match config::compose(&options) {
            Ok(composed) => composed,
            Err(error) => {
                let detail = format!("configuration could not be composed: {error}");
                *ctx.failure = Some(detail.clone());
                ctx.session.note_config_composition_failed(detail);
                ctx.diag.log_mut().record(
                    LogLevel::Error,
                    "configuration could not be composed; launching with the original environment",
                );
                let _ = bridge.shutdown();
                return;
            }
        };

        for warning in &composed.warnings {
            ctx.diag
                .log_mut()
                .record(LogLevel::Warn, format!("configuration warning: {warning}"));
        }
        ctx.diag.log_mut().record(
            LogLevel::Info,
            format!(
                "integration ready on loopback port {} (pane {pane_id})",
                bridge.port()
            ),
        );
        self.set_started(token, bridge, composed.content);
    }

    /// The composed configuration holds the bridge credential, so a tab that
    /// closes without launching does not keep it.
    fn teardown(&mut self) {
        self.composed = None;
    }

    /// Stop the bridge and compare the user's configuration with its capture.
    fn shutdown(&mut self, log: &mut DiagnosticLog) {
        if let Some(bridge) = self.bridge.take() {
            if let Err(error) = bridge.shutdown() {
                log.record(LogLevel::Warn, format!("bridge shutdown failed: {error}"));
            }
        }
        if let Some(guard) = self.config_guard.take() {
            match guard.after_session() {
                Ok(ConfigVerification::Unchanged { state, .. }) => {
                    log.record(
                        LogLevel::Info,
                        format!(
                            "user OpenCode config unchanged ({})",
                            match state {
                                ConfigFileState::Present => "present",
                                ConfigFileState::Missing => "missing",
                            }
                        ),
                    );
                }
                Ok(ConfigVerification::Changed { .. }) => {
                    log.record(
                        LogLevel::Warn,
                        "user OpenCode config changed during the session (Made never writes it)",
                    );
                }
                Err(error) => {
                    log.record(
                        LogLevel::Warn,
                        format!("could not verify user OpenCode config: {error}"),
                    );
                }
            }
        }
    }

    /// The credential stays readable until the diagnostics are written, so
    /// they can be checked for it.
    fn after_shutdown(&mut self) {
        self.clear_token();
    }

    fn bridge(&self) -> Option<&BridgeHandle> {
        self.bridge.as_ref()
    }

    fn token(&self) -> Option<&Token> {
        self.token.as_ref()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Reject special or oversized user config before any read.
///
/// Only a regular file at or below [`MAX_USER_CONFIG_BYTES`] is safe to read on
/// the preparation path. A missing file is valid (composition starts empty).
fn bounded_user_config(path: &Path) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err(format!(
                    "user OpenCode config {} is not a regular file; skipped without reading",
                    path.display()
                ));
            }
            if metadata.len() > MAX_USER_CONFIG_BYTES {
                return Err(format!(
                    "user OpenCode config {} is {} bytes, above the {} byte limit; skipped without reading",
                    path.display(),
                    metadata.len(),
                    MAX_USER_CONFIG_BYTES
                ));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "user OpenCode config {} could not be inspected; skipped without reading: {error}",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teardown_drops_the_composed_configuration_that_holds_the_credential() {
        let mut state = OpenCodeTabState::new();
        state.composed = Some("{\"token\":\"secret\"}".to_owned());
        state.teardown();
        assert!(state.composed.is_none());
        // Idempotent.
        state.teardown();
        assert!(state.composed.is_none());
    }

    #[test]
    fn clearing_the_token_drops_the_composed_configuration_too() {
        let mut state = OpenCodeTabState::new();
        state.composed = Some("{\"token\":\"secret\"}".to_owned());
        state.clear_token();
        assert!(state.composed.is_none());
        assert!(state.token().is_none());
    }

    #[test]
    fn oversized_user_config_is_rejected_before_any_read() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cli.json");
        std::fs::write(&path, vec![b'{'; (MAX_USER_CONFIG_BYTES + 1) as usize]).unwrap();
        let error = bounded_user_config(&path).expect_err("oversized config must be skipped");
        assert!(error.contains("above"), "{error}");
        assert!(error.contains("skipped without reading"), "{error}");
    }

    #[test]
    fn special_user_config_is_rejected_before_any_read() {
        let temp = tempfile::tempdir().unwrap();
        let error = bounded_user_config(temp.path()).expect_err("directory is not a file");
        assert!(error.contains("not a regular file"), "{error}");
    }

    #[test]
    fn regular_user_config_within_the_cap_is_accepted() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cli.json");
        std::fs::write(&path, b"{}").unwrap();
        assert!(bounded_user_config(&path).is_ok());
        assert!(bounded_user_config(&temp.path().join("missing.json")).is_ok());
    }
}
