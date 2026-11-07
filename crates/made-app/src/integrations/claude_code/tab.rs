//! Claude Code state of one tab: the shipped plugin, the tab's spool, the
//! joined hook and protocol channels, and the directory bookkeeping that
//! retries a rejected directory at the next liveness check.

use std::any::Any;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::claude_follow::{ready_claude_plugin_dir, shared_instance, ClaudeTab, TabSpool};
use crate::diagnostics::{ClaudeDiagnosticsReport, LogLevel};
use crate::integration_status::{
    claude_diagnostics_row, remote_claude_diagnostics_row, ClaudeDiagnosticsInputs,
    ClaudeDiagnosticsPhase,
};
use crate::integrations::{
    EnvContribution, LaunchCtx, TabInit, TabIo, TabReport, TabState, CLAUDE_CODE,
};
use crate::session_follow::{SessionFollowOutcome, SessionFollowUpdate};
use crate::tui::{ClaudeLaunchEnv, TerminalTarget, CLAUDE_SPOOL_ENV};

/// How often owned Claude Code processes are checked for exit.
const CLAUDE_LIVENESS_INTERVAL: Duration = Duration::from_secs(1);

/// Claude Code state of one tab: the joined hook and protocol channels, and
/// the tab's spool.
struct ClaudeRuntime {
    tab: ClaudeTab,
    /// `None` when the spool could not be created; hooks are then unavailable.
    spool: Option<TabSpool>,
    liveness_checked_at: Instant,
    /// When Claude Code last became present in this tab.
    present_since: Option<Instant>,
    /// Directory last handed to the follow coordinator and accepted by it.
    applied_directory: Option<PathBuf>,
    /// Directory the coordinator rejected, retried on the liveness tick.
    rejected_directory: Option<PathBuf>,
}

/// A launch prepared for a target that loads the plugin, until the terminal
/// child starts or fails to.
struct Launching {
    direct: bool,
    spool: Option<TabSpool>,
}

/// Claude Code state of one tab.
pub(crate) struct ClaudeTabState {
    /// The shipped Claude Code plugin when it is usable, looked up once per
    /// launch so the per-frame diagnostics row never touches the filesystem.
    plugin: Option<PathBuf>,
    /// App-wide Claude Code detection state for the diagnostics row.
    phase: ClaudeDiagnosticsPhase,
    /// A remote tab's own `claude` detection: `None` while its probe is
    /// pending. The host PATH is never consulted for a remote tab.
    remote_detected: Option<bool>,
    /// Present for local shell and direct Claude Code tabs, and for remote
    /// tabs once they poll.
    runtime: Option<ClaudeRuntime>,
    launching: Option<Launching>,
    /// The directory the last poll handed to the follow coordinator, until
    /// the coordinator's answer arrives.
    reported: Option<Option<PathBuf>>,
}

impl ClaudeTabState {
    pub(crate) fn new(init: &TabInit<'_>) -> Self {
        Self {
            plugin: init.paths.and_then(|paths| {
                ready_claude_plugin_dir(paths.shipped_claude_plugin_dir().as_deref())
            }),
            phase: ClaudeDiagnosticsPhase::NotOnPath,
            remote_detected: None,
            runtime: None,
            launching: None,
            reported: None,
        }
    }

    /// The state of a tab whose list has no Claude Code entry, before any
    /// detection.
    pub(crate) fn absent() -> Self {
        Self::new(&TabInit {
            paths: None,
            remote: false,
        })
    }

    /// Claude Code state of this tab: presence, dots and title inputs. `None`
    /// for tabs that do not load Made's Claude Code plugin.
    pub(crate) fn tab(&self) -> Option<&ClaudeTab> {
        self.runtime.as_ref().map(|runtime| &runtime.tab)
    }

    /// Record the app-wide Claude Code detection state for the diagnostics row.
    pub(crate) fn set_phase(&mut self, phase: ClaudeDiagnosticsPhase) {
        self.phase = phase;
    }

    /// Record this remote tab's own `claude` detection: `None` while its probe
    /// is pending.
    pub(crate) fn set_remote_detection(&mut self, detected: Option<bool>) {
        self.remote_detected = detected;
    }

    /// The Claude Code diagnostics row for this tab, which is `remote` when it
    /// belongs to a remote workspace.
    pub(crate) fn diagnostics(&self, remote: bool) -> ClaudeDiagnosticsReport {
        let runtime = self.runtime.as_ref();
        let present = runtime.is_some_and(|runtime| runtime.tab.present());
        if remote {
            // A remote workspace reports its own server's detection and this
            // tab's presence; the host's plugin and PATH never apply.
            return remote_claude_diagnostics_row(&ClaudeDiagnosticsInputs {
                phase: match self.remote_detected {
                    None => ClaudeDiagnosticsPhase::Checking,
                    Some(false) => ClaudeDiagnosticsPhase::NotOnPath,
                    Some(true) => ClaudeDiagnosticsPhase::Available,
                },
                plugin_missing: false,
                present,
                present_for: runtime
                    .and_then(|runtime| runtime.present_since)
                    .map_or(Duration::ZERO, |since| {
                        Instant::now().saturating_duration_since(since)
                    }),
                live_session: false,
                protocol_observed: runtime
                    .is_some_and(|runtime| runtime.tab.protocol_status_seen()),
            });
        }
        claude_diagnostics_row(&ClaudeDiagnosticsInputs {
            phase: self.phase,
            plugin_missing: self.plugin.is_none(),
            present,
            present_for: runtime
                .and_then(|runtime| runtime.present_since)
                .map_or(Duration::ZERO, |since| {
                    Instant::now().saturating_duration_since(since)
                }),
            live_session: runtime.is_some_and(|runtime| runtime.tab.session_count() > 0),
            protocol_observed: runtime.is_some_and(|runtime| runtime.tab.protocol_status_seen()),
        })
    }

    /// Replace the cached plugin lookup.
    #[cfg(test)]
    pub(crate) fn set_plugin(&mut self, plugin: Option<PathBuf>) {
        self.plugin = plugin;
    }

    /// Replace when Claude Code last became present in this tab.
    #[cfg(test)]
    pub(crate) fn set_present_since(&mut self, since: Option<Instant>) {
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.present_since = since;
        }
    }

    fn open_spool(ctx: &mut LaunchCtx<'_>) -> Option<TabSpool> {
        let temp_dir = ctx.paths?.temp_dir.clone();
        let opened =
            shared_instance(&temp_dir).and_then(|instance| TabSpool::open(instance, ctx.pane_id));
        match opened {
            Ok(spool) => Some(spool),
            Err(error) => {
                ctx.log.record(
                    LogLevel::Warn,
                    format!("Claude Code spool could not be created: {error}"),
                );
                None
            }
        }
    }
}

/// What a Claude Code launch adds to a terminal child's environment. A launch
/// with no spool binding, including every target that does not load the
/// plugin, also removes an inherited one, so a copied variable can never point
/// a tab at another tab's spool.
pub(crate) fn launch_contribution(
    target: TerminalTarget,
    launch: &ClaudeLaunchEnv,
) -> EnvContribution {
    let policy = CLAUDE_CODE.env_policy();
    if !policy.covers(CLAUDE_CODE, target) {
        return EnvContribution {
            set: Vec::new(),
            strip: policy.strip.to_vec(),
        };
    }
    let binds_spool = launch.env.iter().any(|(key, _)| key == CLAUDE_SPOOL_ENV);
    EnvContribution {
        set: launch.env.clone(),
        strip: if binds_spool {
            Vec::new()
        } else {
            policy.strip.to_vec()
        },
    }
}

impl TabState for ClaudeTabState {
    /// Create this tab's spool and compose its Claude Code environment. Only
    /// local shell and direct Claude Code tabs get one.
    fn prepare_launch(
        &mut self,
        target: TerminalTarget,
        ctx: &mut LaunchCtx<'_>,
    ) -> EnvContribution {
        if !CLAUDE_CODE.env_policy().covers(CLAUDE_CODE, target) {
            return launch_contribution(target, &ClaudeLaunchEnv::default());
        }
        self.plugin = ctx.paths.and_then(|paths| {
            ready_claude_plugin_dir(paths.shipped_claude_plugin_dir().as_deref())
        });
        let shipped = self.plugin.clone();
        let spool = if shipped.is_some() {
            Self::open_spool(ctx)
        } else {
            None
        };
        let existing = std::env::var_os(made_platform::CLAUDE_CODE_PLUGIN_DIRS_ENV);
        let launch = target.claude_launch_env(
            shipped.as_deref(),
            existing.as_deref(),
            spool.as_ref().map(TabSpool::path),
        );
        if launch.hooks_unavailable {
            ctx.log.record(
                LogLevel::Warn,
                "Claude Code session-status hooks are unavailable for this tab",
            );
        }
        self.launching = Some(Launching {
            direct: target == TerminalTarget::Tui(CLAUDE_CODE),
            spool,
        });
        launch_contribution(target, &launch)
    }

    fn on_spawned(&mut self, pid: Option<u32>) {
        let Some(launching) = self.launching.take() else {
            return;
        };
        let mut tab = ClaudeTab::new(pid.unwrap_or(0), launching.direct);
        tab.set_child_running(true);
        self.runtime = Some(ClaudeRuntime {
            tab,
            spool: launching.spool,
            liveness_checked_at: Instant::now(),
            present_since: None,
            applied_directory: None,
            rejected_directory: None,
        });
    }

    fn on_launch_failed(&mut self) {
        self.launching = None;
    }

    /// Apply this tab's Claude Code channels: terminal protocol events and the
    /// shell-prompt report, spool records read off the UI thread, the
    /// one-second liveness check and the session directory.
    fn poll(&mut self, now: Instant, io: &mut TabIo<'_>) -> TabReport {
        // A remote tab has no spool, no hook sessions and no process anchor: its
        // runtime exists to carry the status protocol of its remote terminal.
        if self.runtime.is_none() && io.remote {
            let direct = matches!(io.target, TerminalTarget::RemoteClaude { .. });
            self.runtime = Some(ClaudeRuntime {
                tab: ClaudeTab::new(0, direct),
                spool: None,
                liveness_checked_at: now,
                present_since: None,
                applied_directory: None,
                rejected_directory: None,
            });
        }
        let Some(runtime) = self.runtime.as_mut() else {
            return TabReport::default();
        };
        let Some(terminal) = io.terminal.as_deref_mut() else {
            return TabReport::default();
        };
        if let Some(pid) = terminal.pid() {
            runtime.tab.set_root(pid);
        }
        let prompt_reported = terminal.take_cwd_report();
        runtime
            .tab
            .apply_terminal_reports(terminal.take_protocol_events(), prompt_reported);
        runtime.tab.set_child_running(terminal.is_running());
        // One process map serves every ancestry and liveness check of this
        // poll. It is built only when a check needs it, and records stay
        // queued until the terminal process is known.
        let tick_due =
            now.saturating_duration_since(runtime.liveness_checked_at) >= CLAUDE_LIVENESS_INTERVAL;
        let records = if runtime.tab.root_known() {
            runtime
                .spool
                .as_ref()
                .map(TabSpool::drain)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut parents = None;
        let mut parent_of = |pid: u32| -> Option<u32> {
            parents
                .get_or_insert_with(made_platform::process_parent_map)
                .get(&pid)
                .copied()
        };
        if !records.is_empty() {
            runtime.tab.ingest(records, &mut parent_of);
        }
        if tick_due {
            runtime.liveness_checked_at = now;
            runtime.tab.check_liveness(&mut parent_of);
        }
        if runtime.tab.present() {
            runtime.present_since.get_or_insert(now);
        } else if runtime.present_since.take().is_some() {
            runtime.tab.reset_observations();
        }
        let directory = runtime.tab.directory().map(PathBuf::from);
        if directory == runtime.applied_directory {
            return TabReport::default();
        }
        if !tick_due && directory.is_some() && directory == runtime.rejected_directory {
            return TabReport::default();
        }
        self.reported = Some(directory.clone());
        TabReport {
            directory: Some(directory),
        }
    }

    /// Remember whether the coordinator accepted the directory this tab
    /// reported; a rejected directory is not remembered as applied.
    fn on_follow_outcome(&mut self, outcome: Option<&SessionFollowOutcome>) {
        let Some(directory) = self.reported.take() else {
            return;
        };
        let accepted = outcome.is_none_or(|outcome| {
            !matches!(outcome.update, SessionFollowUpdate::RejectedSnapshot { .. })
        });
        if let Some(runtime) = self.runtime.as_mut() {
            if accepted {
                runtime.applied_directory.clone_from(&directory);
                runtime.rejected_directory = None;
            } else {
                runtime.rejected_directory = directory;
            }
        }
    }

    fn teardown(&mut self) {
        self.runtime = None;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
