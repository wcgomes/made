//! Remote-only config composition and persistent plugin bridge ownership.
use crate::protocol::{ErrorCode, Operation};
use made_bridge::{
    config::{self, ComposeOptions, ComposedConfig},
    config_guard::{ConfigGuard, ConfigVerification},
    server::{self, BridgeEvent, BridgeHandle, BridgeOptions},
    state::SessionSnapshot,
    Token,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// Prefer remote V2/XDG config when it exists; legacy is fallback only.
/// All arguments come from the remote platform resolver, never the desktop.
pub fn resolve_config(v2: PathBuf, legacy: PathBuf) -> PathBuf {
    if v2.exists() {
        v2
    } else if legacy.exists() {
        legacy
    } else {
        v2
    }
}
pub struct Integration {
    bridge: BridgeHandle,
    config: ComposedConfig,
    guard: ConfigGuard,
    snapshot: Option<SessionSnapshot>,
    focus: HashMap<String, (u64, String)>,
}
impl Integration {
    pub fn start(
        plugin: &Path,
        install: &Path,
        user_config: PathBuf,
        inline: Option<String>,
        pane: String,
    ) -> Result<Self, ErrorCode> {
        let token = Token::generate().map_err(|_| ErrorCode::Io)?;
        let bridge =
            server::start(BridgeOptions::new(token.clone(), &pane)).map_err(|_| ErrorCode::Io)?;
        let guard =
            ConfigGuard::before_session(&user_config).map_err(|_| ErrorCode::Unavailable)?;
        let mut options = ComposeOptions::new(
            plugin,
            install,
            bridge.url(),
            token,
            pane,
            env!("CARGO_PKG_VERSION"),
        )
        .with_user_config_path(user_config);
        options.inline_config = inline;
        let config = config::compose(&options).map_err(|_| ErrorCode::Unavailable)?;
        Ok(Self {
            bridge,
            config,
            guard,
            snapshot: None,
            focus: HashMap::new(),
        })
    }
    /// Inject this one remote-composed pair into BOTH shell and direct TUI launch.
    pub fn environment(&self) -> (&'static str, &str) {
        self.config.env_pair()
    }
    pub fn url(&self) -> String {
        self.bridge.url()
    }
    pub fn state(&self) -> Result<Operation, ErrorCode> {
        self.snapshot
            .clone()
            .map(|snapshot| Operation::SessionSnapshot { snapshot })
            .ok_or(ErrorCode::Unavailable)
    }
    pub fn focus(&mut self, request: u64, session: &str) -> Result<(), ErrorCode> {
        if self.focus.len() >= 16 {
            return Err(ErrorCode::Limit);
        }
        let id = self
            .bridge
            .focus(session)
            .map_err(|_| ErrorCode::Unavailable)?;
        self.focus.insert(id, (request, session.to_owned()));
        Ok(())
    }
    /// Epoch changes drop uncertain focus commands; never retransmit on attach.
    pub fn reset_attachment(&mut self) {
        self.focus.clear();
        self.snapshot = None;
        let _ = self.bridge.request_state();
    }
    pub fn cancel(&mut self, request: u64) {
        self.focus.retain(|_, (id, _)| *id != request);
    }
    pub fn poll(&mut self) -> Vec<(Option<u64>, Operation)> {
        let mut result = Vec::new();
        for _ in 0..256 {
            let Ok(event) = self.bridge.try_recv() else {
                break;
            };
            match event {
                BridgeEvent::State(snapshot) => {
                    if snapshot
                        .validate_with_path_semantics(
                            made_bridge::state::PathSemantics::RemotePosix,
                        )
                        .is_ok()
                    {
                        self.snapshot = Some(snapshot.clone());
                        result.push((None, Operation::SessionSnapshot { snapshot }));
                    }
                }
                BridgeEvent::FocusResult(outcome) => {
                    if let Some((id, session_id)) = self.focus.remove(&outcome.request_id) {
                        result.push((
                            Some(id),
                            Operation::FocusResult {
                                session_id,
                                accepted: outcome.ok,
                            },
                        ));
                    }
                }
                BridgeEvent::Disconnected(_)
                | BridgeEvent::Goodbye(_)
                | BridgeEvent::HeartbeatTimeout(_) => {
                    self.snapshot = None;
                    self.focus.clear();
                }
                BridgeEvent::Connected(_) => {
                    let _ = self.bridge.request_state();
                }
                BridgeEvent::ProtocolError(_) => {}
            }
        }
        result
    }
    pub fn verify_config(&self) -> Result<ConfigVerification, ErrorCode> {
        self.guard.after_session().map_err(|_| ErrorCode::Io)
    }
}
