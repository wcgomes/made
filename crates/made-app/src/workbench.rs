//! Native workbench runtime: launch composition, bridge events, session follow,
//! explorer, worktree metadata, diagnostics, and metrics.
//!
//! This is the composition root. It owns the privileged runtime side of the
//! workspace. Runtime queues are bounded and nonblocking. Construction still
//! performs synchronous config/PTY setup (including shell policy preflight);
//! callers must not mistake that remaining launch cost for background work.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui;
use made_bridge::config::{self, ComposeOptions, CONFIG_CONTENT_ENV};
use made_bridge::config_guard::{ConfigFileState, ConfigGuard, ConfigVerification};
use made_bridge::security::Token;
use made_bridge::server::{self, BridgeEvent, BridgeHandle, BridgeOptions};
use made_bridge::state::SessionSnapshot;
use made_explorer::root_switch::RootGeneration;
use made_platform::{
    host_platform, AppPaths, LogIssue, LogIssueSeverity, Platform, PlatformFamily, PtySize,
};
use made_terminal::PtyOptions;

use crate::diagnostics::{
    Diagnostics, DiagnosticsSnapshot, LogEntry, LogLevel, WorkspaceDiagnostics, DEFAULT_LOG_FILE,
    MAX_LOG_FILE_BYTES,
};
use crate::explorer_view::ExplorerPane;
use crate::integration_status::{IntegrationState, IntegrationStatus};
use crate::metrics::Metrics;
use crate::sanitize::{sanitize_copy_text, sanitize_display};
use crate::session_focus::{FocusErrorKind, FocusOutcome, FocusUpdate, SessionFocusController};
use crate::session_follow::{SessionFollowCoordinator, SessionFollowUpdate};
use crate::session_map::SessionMap;
use crate::terminal_view::{TerminalPane, DEFAULT_COLS, DEFAULT_ROWS};
use crate::theme::EXPLORER_DEFAULT_WIDTH_WIDE;
use crate::workspace::{fallback_terminal_cwd, LaunchPlan, WorkspaceError};
use crate::worktree::{GitRunner, MetadataStatus, WorktreeMetadata};
use crate::{APP_NAME, APP_VERSION};

/// Default application title.
const BASE_TITLE: &str = APP_NAME;

/// How many bridge events are handled in one frame.
const MAX_BRIDGE_EVENTS_PER_FRAME: usize = 128;

/// How long a transient terminal notice stays in the status area.
const NOTICE_TTL: Duration = Duration::from_secs(8);

/// Default low-frequency revalidation cadence for the shell-owned root.
///
/// The check runs on a private worker thread, never on the UI thread. It is
/// deliberately much slower than terminal pumping: a deleted/renamed directory
/// does not need 60 Hz polling to be reported honestly.
const SHELL_ROOT_REVALIDATION_INTERVAL: Duration = Duration::from_secs(1);

static NEXT_PANE: AtomicU64 = AtomicU64::new(1);

/// Which transition created an explorer root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootSwitchOrigin {
    /// A validated shell cwd report changed the root.
    ShellCwd,
    /// Session-follow state created or restored the root.
    Session,
}

/// A shell-owned root observed unavailable by the background revalidator.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellRootUnavailable {
    path: PathBuf,
    detail: String,
}

/// One availability transition for the shell-owned root.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ShellRootTransition {
    /// Availability did not change.
    Unchanged,
    /// The root became unavailable for the first time in this episode.
    BecameUnavailable(String),
    /// A previously unavailable root is usable again.
    Recovered,
}

/// Apply an availability observation, deduplicating repeated failures at the
/// same path. This is the state machine behind "mark once and recover once";
/// callers log only the two transition variants.
fn apply_shell_root_availability(
    state: &mut Option<ShellRootUnavailable>,
    path: &Path,
    result: &Result<(), String>,
) -> ShellRootTransition {
    match result {
        Err(detail) => {
            if state
                .as_ref()
                .is_some_and(|unavailable| unavailable.path == path)
            {
                ShellRootTransition::Unchanged
            } else {
                *state = Some(ShellRootUnavailable {
                    path: path.to_path_buf(),
                    detail: detail.clone(),
                });
                ShellRootTransition::BecameUnavailable(detail.clone())
            }
        }
        Ok(()) => {
            if state.take().is_some() {
                ShellRootTransition::Recovered
            } else {
                ShellRootTransition::Unchanged
            }
        }
    }
}

/// Per-frame instrumentation state. Every field is a scalar, an `Option`, or
/// one bounded callback slot; no per-frame collection is retained here.
#[derive(Debug)]
struct RuntimeInstrumentation {
    activation_started: Option<Instant>,
    layout_change_started: Option<Instant>,
    preview_entered_at: Option<Instant>,
    preview_open_latency: Option<Duration>,
    preview_return_started: Option<Instant>,
    cwd_root_started_at: Option<Instant>,
    root_switch_origin: Option<RootSwitchOrigin>,
    last_grid_paints: u64,
    /// End-of-pass timestamp written by egui after this workspace's UI has
    /// submitted all shapes. Metrics are finalized from this timestamp, not
    /// from `Workbench::ui` entry.
    paint_submitted_at: Arc<Mutex<Option<Instant>>>,
    /// One callback per workspace, registered on first paint.
    paint_callback_registered: bool,
    /// A visible pass is waiting for its end-of-pass timestamp.
    paint_pending: bool,
    /// Whether the explorer panel was expected to paint in the pending pass.
    paint_explorer_visible: bool,
}

impl RuntimeInstrumentation {
    fn new() -> Self {
        Self {
            activation_started: None,
            layout_change_started: None,
            preview_entered_at: None,
            preview_open_latency: None,
            preview_return_started: None,
            cwd_root_started_at: None,
            root_switch_origin: None,
            last_grid_paints: 0,
            paint_submitted_at: Arc::new(Mutex::new(None)),
            paint_callback_registered: false,
            paint_pending: false,
            paint_explorer_visible: false,
        }
    }
}

/// Exactly one left surface receives painting and input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftSurface {
    Terminal,
    Preview,
}

/// Maximum size of the user-owned OpenCode config Made is willing to read on
/// the workspace preparation path. Larger or special files are skipped rather
/// than read, so opening a workspace cannot stall behind an unbounded read.
pub const MAX_USER_CONFIG_BYTES: u64 = 1024 * 1024;

/// Worktree metadata state for the header and diagnostics.
#[derive(Debug, Clone, Default)]
pub struct WorktreeState {
    /// Latest metadata for the active root.
    pub metadata: Option<WorktreeMetadata>,
    /// Root generation the metadata belongs to.
    pub generation: RootGeneration,
    /// A metadata read is in flight.
    pub loading: bool,
}

/// One workspace owner; siblings share no process, bridge or load receiver.
pub struct Workbench {
    pub(crate) platform: made_platform::HostPlatform,
    pub(crate) plan: Option<LaunchPlan>,
    pub(crate) workspace_error: Option<WorkspaceError>,
    pub(crate) terminal_cwd: PathBuf,
    pub(crate) paths: Option<AppPaths>,
    pub(crate) token: Option<Token>,
    pub(crate) bridge: Option<BridgeHandle>,
    pub(crate) config_guard: Option<ConfigGuard>,
    pub(crate) config_failure: Option<String>,
    pub(crate) diag: Diagnostics,
    pub(crate) metrics: Metrics,
    pub(crate) integration: IntegrationState,
    pub(crate) follow: Option<SessionFollowCoordinator>,
    pub(crate) session_map: SessionMap,
    pub(crate) focus: SessionFocusController,
    pub(crate) terminal: Option<TerminalPane>,
    pub(crate) terminal_error: Option<String>,
    pub(crate) shell_cwd_error: Option<String>,
    pub(crate) explorer: Option<ExplorerPane>,
    pub(crate) worktree: WorktreeState,
    worktree_worker: WorktreeWorker,
    pending_worktree: PendingWorktreeRequest,
    shell_capability: Option<made_platform::ShellCwdCapability>,
    shell_root_worker: ShellRootWorker,
    shell_root_unavailable: Option<ShellRootUnavailable>,
    shell_root_check_in_flight: bool,
    shell_root_checked_at: Instant,
    shell_root_check_interval: Duration,
    instrumentation: RuntimeInstrumentation,
    pane_id: String,
    visible: bool,
    left_surface: LeftSurface,
    explorer_visible: bool,
    explorer_width: f32,
    sessions_visible: bool,
    tick_count: u64,
    paint_count: u64,
    last_osc7_cwd: Option<PathBuf>,
    log_path: Option<PathBuf>,
    pub(crate) log_write_issue: Option<LogIssue>,
    /// Preparation start for this workspace, not the application launch.
    pub(crate) started_at: Instant,
    startup_recorded: bool,
    shutdown_done: bool,
    /// UI context captured during tick so the one bounded end-of-pass callback
    /// can be registered without editing the UI module's entry point.
    frame_ctx: Option<egui::Context>,
    pub(crate) last_activity: Instant,
    last_frame: Instant,
    last_tick_active: bool,
    benchmark_frames: bool,
    pending_notice: Option<(String, Instant)>,
    last_title: Option<String>,
}

impl std::fmt::Debug for Workbench {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Workbench")
            .field("plan", &self.plan)
            .field("workspace_error", &self.workspace_error)
            .field("integration", &self.integration.status())
            .field("session_rows", &self.session_map.len())
            .field("terminal", &self.terminal)
            .field("explorer", &self.explorer)
            .field("worktree", &self.worktree)
            .finish_non_exhaustive()
    }
}

impl Workbench {
    /// Start the runtime for a validated plan or an invalid-workspace fallback.
    ///
    /// `started_at` is this workspace's preparation start captured by the app
    /// when it schedules the work, not the application launch instant. A tab
    /// opened minutes later must not inherit a multi-second startup metric.
    /// Every failure becomes visible state; this constructor does not panic and
    /// does not prevent the terminal from starting.
    pub fn start(
        plan: Option<LaunchPlan>,
        mut workspace_error: Option<WorkspaceError>,
        terminal_cwd: PathBuf,
        started_at: Instant,
    ) -> Self {
        let platform = host_platform();
        let mut app_paths = None;
        let mut path_error = None;
        match platform.app_paths() {
            Ok(paths) => {
                if let Err(error) = paths.ensure_directories() {
                    path_error = Some(format!("app directories: {error}"));
                }
                app_paths = Some(paths);
            }
            Err(error) => path_error = Some(format!("app paths: {error}")),
        }
        let log_path = diagnostics_log_path(app_paths.as_ref());
        #[cfg(test)]
        if let Some(path) = &log_path {
            let temp = std::env::temp_dir();
            if !path.starts_with(&temp) {
                panic!(
                    "unit tests must isolate app paths before Workbench::start; resolved log path {} is not under TMPDIR {}",
                    path.display(),
                    temp.display()
                );
            }
        }
        let diag = match &log_path {
            Some(path) => Diagnostics::new(path.clone()),
            None => Diagnostics::unavailable(
                "app paths are unavailable; no shared temporary log is used",
            ),
        };

        let launch_root = plan.as_ref().map(|plan| plan.explorer_root.clone());
        let integration = IntegrationState::new(launch_root.clone(), terminal_cwd.clone());
        let follow = plan
            .as_ref()
            .and_then(|plan| SessionFollowCoordinator::from_launch_plan(plan).ok());
        if plan.is_some() && follow.is_none() {
            let error = WorkspaceError::new(
                crate::workspace::WorkspaceErrorKind::Unreadable,
                terminal_cwd.clone(),
                "the workspace cannot be followed",
            );
            workspace_error = Some(error);
        }

        let mut workbench = Self {
            platform,
            plan,
            workspace_error,
            terminal_cwd,
            paths: app_paths,
            token: None,
            bridge: None,
            config_guard: None,
            config_failure: None,
            diag,
            metrics: Metrics::new(),
            integration,
            follow,
            session_map: SessionMap::new(),
            focus: SessionFocusController::new(),
            terminal: None,
            terminal_error: None,
            shell_cwd_error: None,
            explorer: None,
            worktree: WorktreeState::default(),
            worktree_worker: WorktreeWorker::start(),
            pending_worktree: PendingWorktreeRequest::default(),
            shell_capability: None,
            shell_root_worker: ShellRootWorker::start(),
            shell_root_unavailable: None,
            shell_root_check_in_flight: false,
            shell_root_checked_at: Instant::now(),
            shell_root_check_interval: SHELL_ROOT_REVALIDATION_INTERVAL,
            instrumentation: RuntimeInstrumentation::new(),
            pane_id: format!(
                "made-{}-{}",
                std::process::id(),
                NEXT_PANE.fetch_add(1, Ordering::Relaxed)
            ),
            visible: true,
            left_surface: LeftSurface::Terminal,
            explorer_visible: true,
            explorer_width: EXPLORER_DEFAULT_WIDTH_WIDE,
            sessions_visible: false,
            tick_count: 0,
            paint_count: 0,
            last_osc7_cwd: None,
            log_path,
            log_write_issue: None,
            started_at,
            startup_recorded: false,
            shutdown_done: false,
            frame_ctx: None,
            last_activity: Instant::now(),
            last_frame: Instant::now(),
            last_tick_active: false,
            benchmark_frames: false,
            pending_notice: None,
            last_title: None,
        };

        if let Some(error) = path_error {
            workbench.log_write_issue = Some(LogIssue::error(format!(
                "app path resolution failed: {error}"
            )));
        } else if workbench.log_path.is_none() {
            workbench.log_write_issue = Some(LogIssue::error(
                "diagnostics log unavailable: app paths are unavailable (fail-closed)",
            ));
        } else if let Some(path) = &workbench.log_path {
            // Preflight the private log so a permissions/symlink failure is
            // visible in diagnostics while the window is still open. A legacy
            // file above the threshold is not refused: the next clean shutdown
            // keeps it as the previous log and starts a fresh active log.
            if let Err(error) = made_platform::open_private_log(path) {
                workbench.log_write_issue = Some(LogIssue::error(format!(
                    "diagnostics log unavailable: {error}"
                )));
            } else {
                let mut warnings = Vec::new();
                if let Ok(metadata) = std::fs::symlink_metadata(path) {
                    if metadata.len() > MAX_LOG_FILE_BYTES {
                        warnings.push(format!(
                            "diagnostics log is {} bytes, above the 1 MiB threshold; it will be kept as {} on the next shutdown write",
                            metadata.len(),
                            made_platform::previous_private_log_path(path).display()
                        ));
                    }
                }
                let backup = made_platform::previous_private_log_path(path);
                match std::fs::symlink_metadata(&backup) {
                    Ok(metadata) if metadata.is_file() => {
                        if let Err(error) = made_platform::tighten_private_log_permissions(&backup)
                        {
                            warnings.push(format!(
                                "previous diagnostics log permissions could not be tightened: {error}"
                            ));
                        }
                    }
                    Ok(_) => warnings.push(format!(
                        "previous diagnostics log path is not a regular file ({}); rotation will be refused if required",
                        backup.display()
                    )),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => warnings.push(format!(
                        "previous diagnostics log path could not be inspected ({}): {error}",
                        backup.display()
                    )),
                }
                let lock = made_platform::private_log_lock_path(path);
                match std::fs::symlink_metadata(&lock) {
                    Ok(metadata) if metadata.is_file() => {}
                    Ok(_) => warnings.push(format!(
                        "diagnostics log lock path is not a regular file ({}); writes will be refused",
                        lock.display()
                    )),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => warnings.push(format!(
                        "diagnostics log lock path could not be inspected ({}): {error}",
                        lock.display()
                    )),
                }
                workbench.log_write_issue = if warnings.is_empty() {
                    None
                } else {
                    Some(LogIssue::warning(warnings.join("; ")))
                };
            }
        }
        if let Some(issue) = &workbench.log_write_issue {
            let level = match issue.severity {
                LogIssueSeverity::Warning => LogLevel::Warn,
                LogIssueSeverity::Error => LogLevel::Error,
            };
            workbench
                .diag
                .log_mut()
                .record(level, issue.message.clone());
        }
        workbench.log_launch_context();
        let composed_content = workbench.start_integration();
        workbench.start_terminal(composed_content);
        workbench.refresh_shell_capability();

        if let Some(root) = workbench.explorer_root().map(Path::to_path_buf) {
            let generation = workbench
                .follow
                .as_ref()
                .map(SessionFollowCoordinator::generation)
                .unwrap_or(RootGeneration::INITIAL);
            workbench.explorer = Some(ExplorerPane::new(&root, generation));
            workbench.request_worktree(root, generation);
        }
        workbench
    }

    /// Start the bridge and compose the per-launch OpenCode configuration.
    ///
    /// The user's configuration is read only. A composition failure leaves the
    /// child environment untouched and reports a degraded status. Returns the
    /// composed content for the child environment, never persisted.
    fn start_integration(&mut self) -> Option<String> {
        let Some(paths) = self.paths.clone() else {
            self.config_failure = Some("app-owned paths are unavailable".to_owned());
            self.integration
                .note_config_composition_failed("app-owned paths are unavailable");
            self.diag.log_mut().record(
                LogLevel::Error,
                "configuration not composed: app-owned paths are unavailable",
            );
            return None;
        };

        // Inspect the user's config before any read. Only a regular file within
        // the bounded limit is captured or merged; FIFOs, devices, directories,
        // and oversized files are skipped so workspace preparation cannot
        // block on an unbounded or blocking read.
        let user_config = match bounded_user_config(&paths.opencode_config_path) {
            Ok(()) => Some(paths.opencode_config_path.clone()),
            Err(detail) => {
                self.config_failure = Some(detail.clone());
                self.integration
                    .note_config_composition_failed(detail.clone());
                self.diag.log_mut().record(LogLevel::Error, detail);
                None
            }
        };

        // Capture the user's config before the session, even if composition
        // later fails. Verification never rewrites the file.
        self.config_guard = if let Some(path) = user_config.as_deref() {
            match ConfigGuard::before_session(path) {
                Ok(guard) => {
                    let state = guard.before().state();
                    self.diag.log_mut().record(
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
                    self.diag.log_mut().record(
                        LogLevel::Warn,
                        format!("could not capture user OpenCode config: {error}"),
                    );
                    None
                }
            }
        } else {
            None
        };

        let pane_id = self.pane_id.clone();
        let token = match Token::generate() {
            Ok(token) => token,
            Err(error) => {
                let detail = format!("bridge token generation failed: {error}");
                self.config_failure = Some(detail.clone());
                self.integration.note_config_composition_failed(detail);
                self.diag
                    .log_mut()
                    .record(LogLevel::Error, "bridge token generation failed");
                return None;
            }
        };
        self.diag = self
            .diag
            .clone()
            .with_token_fingerprint(token.fingerprint());

        let bridge = match server::start(
            BridgeOptions::new(token.clone(), pane_id.clone()).with_app_version(APP_VERSION),
        ) {
            Ok(bridge) => bridge,
            Err(error) => {
                let detail = format!("bridge could not start: {error}");
                self.integration.note_plugin_unavailable(detail.clone());
                self.diag
                    .log_mut()
                    .record(LogLevel::Error, "loopback bridge could not start");
                self.config_failure = Some(detail);
                return None;
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
                self.config_failure = Some(detail.clone());
                self.integration.note_config_composition_failed(detail);
                self.diag.log_mut().record(
                    LogLevel::Error,
                    "configuration could not be composed; launching with the original environment",
                );
                let _ = bridge.shutdown();
                return None;
            }
        };

        for warning in &composed.warnings {
            self.diag
                .log_mut()
                .record(LogLevel::Warn, format!("configuration warning: {warning}"));
        }
        self.diag.log_mut().record(
            LogLevel::Info,
            format!(
                "integration ready on loopback port {} (pane {pane_id})",
                bridge.port()
            ),
        );
        self.token = Some(token);
        self.bridge = Some(bridge);
        Some(composed.content)
    }

    fn start_terminal(&mut self, composed_content: Option<String>) {
        let mut options = PtyOptions::new(self.terminal_cwd.clone())
            .with_size(PtySize {
                rows: DEFAULT_ROWS as u16,
                cols: DEFAULT_COLS as u16,
            })
            .with_channel_capacity(256);

        if let Some(content) = &composed_content {
            options = options.with_env(CONFIG_CONTENT_ENV, content.clone());
        }

        match TerminalPane::spawn(&self.platform, options.clone()) {
            Ok(pane) => {
                if let Some(pid) = pane.pid() {
                    self.diag.log_mut().record(
                        LogLevel::Info,
                        format!("terminal shell started (pid {pid})"),
                    );
                }
                self.terminal = Some(pane);
                self.terminal_error = None;
                if composed_content.is_none() {
                    self.diag.log_mut().record(
                        LogLevel::Warn,
                        "terminal shell started with the original environment (no Made configuration)",
                    );
                }
            }
            Err(first_error) => {
                // The workspace may have disappeared between validation and
                // spawn. Retry once in the documented safe fallback directory.
                let fallback = fallback_terminal_cwd();
                if fallback != self.terminal_cwd {
                    let mut retry = PtyOptions::new(fallback.clone())
                        .with_size(options.size)
                        .with_channel_capacity(256);
                    if let Some(content) = &composed_content {
                        retry = retry.with_env(CONFIG_CONTENT_ENV, content.clone());
                    }
                    if let Ok(pane) = TerminalPane::spawn(&self.platform, retry) {
                        self.diag.log_mut().record(
                            LogLevel::Warn,
                            "workspace shell failed; terminal started in the fallback directory",
                        );
                        self.terminal_cwd = fallback;
                        self.integration.set_terminal_cwd(&self.terminal_cwd);
                        if let Some(follow) = self.follow.as_mut() {
                            follow.set_terminal_cwd(&self.terminal_cwd);
                        }
                        self.terminal = Some(pane);
                        self.terminal_error = None;
                        return;
                    }
                }
                let detail = format!("terminal shell could not start: {first_error}");
                self.diag.log_mut().record(LogLevel::Error, detail.clone());
                self.terminal_error = Some(detail);
            }
        }
    }

    fn log_launch_context(&mut self) {
        let source = self
            .plan
            .as_ref()
            .map(|plan| plan.workspace.source().label())
            .unwrap_or("invalid-workspace");
        let root = self.terminal_cwd.display().to_string();
        self.diag.log_mut().record(
            LogLevel::Info,
            format!("{BASE_TITLE} {APP_VERSION} starting ({source})"),
        );
        self.diag.log_mut().record(
            LogLevel::Info,
            format!("terminal working directory: {root}"),
        );
        if let Some(paths) = &self.paths {
            self.diag.log_mut().record(
                LogLevel::Info,
                format!(
                    "plugin source: {} ({})",
                    paths.plugin_source.name(),
                    paths.plugin_dir.display()
                ),
            );
        }
        if let Some(error) = &self.workspace_error {
            self.diag
                .log_mut()
                .record(LogLevel::Warn, format!("workspace unavailable: {error}"));
        }
    }

    /// Active explorer root.
    pub fn explorer_root(&self) -> Option<&Path> {
        self.follow
            .as_ref()
            .map(SessionFollowCoordinator::current_root)
            .or_else(|| self.plan.as_ref().map(|plan| plan.explorer_root.as_path()))
    }

    /// Integration state.
    pub fn integration(&self) -> &IntegrationState {
        &self.integration
    }

    /// Session map.
    pub fn session_map(&self) -> &SessionMap {
        &self.session_map
    }

    /// Focus controller.
    pub fn focus_controller(&self) -> &SessionFocusController {
        &self.focus
    }

    /// Metrics.
    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    /// Diagnostics.
    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diag
    }

    /// Terminal pane, when the shell started.
    pub fn terminal(&self) -> Option<&TerminalPane> {
        self.terminal.as_ref()
    }

    /// Mutable terminal for explicit input and lifecycle actions.
    pub fn terminal_mut(&mut self) -> Option<&mut TerminalPane> {
        self.terminal.as_mut()
    }
    /// Validated launch plan for picker cancellation and folder labels.
    pub fn launch_plan(&self) -> Option<&LaunchPlan> {
        self.plan.as_ref()
    }
    /// Authenticated pane identity, distinct from all sibling workspaces.
    pub fn pane_id(&self) -> &str {
        &self.pane_id
    }
    /// Latest validated shell cwd, tracked even while OpenCode overrides it.
    pub fn shell_cwd(&self) -> &Path {
        &self.terminal_cwd
    }
    /// Last rejected shell cwd report, when the shell is not authoritative.
    ///
    /// This is tracked separately from the effective root: a bad shell report
    /// must not mark a healthy agent-owned root unavailable.
    pub fn shell_cwd_error(&self) -> Option<&str> {
        self.shell_cwd_error.as_deref()
    }
    /// Configuration composition failure or unsafe-config detail, if any.
    pub fn config_failure(&self) -> Option<&str> {
        self.config_failure.as_deref()
    }
    /// Whether the effective tree is currently owned by an active session.
    pub fn root_source(&self) -> &'static str {
        if self.active_session().is_some() {
            "opencode"
        } else {
            "shell"
        }
    }
    /// Reporting capability delegated from the owning live PTY.
    ///
    /// The capability is also cached at launch and before shutdown so
    /// diagnostics can still describe the selected workspace after the terminal
    /// has been terminated.
    pub fn shell_cwd_capability(&self) -> Option<&made_platform::ShellCwdCapability> {
        self.terminal
            .as_ref()
            .and_then(TerminalPane::shell_cwd_capability)
            .or(self.shell_capability.as_ref())
    }

    /// Cache the live terminal's capability before it can be dropped.
    fn refresh_shell_capability(&mut self) {
        if let Some(capability) = self
            .terminal
            .as_ref()
            .and_then(TerminalPane::shell_cwd_capability)
        {
            self.shell_capability = Some(capability.clone());
        }
    }

    /// Latest shell-owned root unavailability detail, when the background
    /// revalidator has observed a deletion/rename/permission failure.
    pub fn shell_root_unavailable(&self) -> Option<&str> {
        self.shell_root_unavailable
            .as_ref()
            .map(|unavailable| unavailable.detail.as_str())
    }

    /// Override the low-frequency shell-root revalidation cadence (tests use a
    /// short interval; production keeps the default).
    pub fn set_shell_root_revalidation_interval(&mut self, interval: Duration) {
        self.shell_root_check_interval = interval;
    }
    /// Current visible left surface.
    pub fn left_surface(&self) -> LeftSurface {
        self.left_surface
    }
    /// Whether the explorer should allocate a visible panel.
    pub fn explorer_visible(&self) -> bool {
        self.explorer_visible
    }
    /// Retained explorer width, clamped again by the layout owner.
    pub fn explorer_width(&self) -> f32 {
        self.explorer_width
    }
    /// Optional session map is initially hidden and retained per workspace.
    pub fn sessions_visible(&self) -> bool {
        self.sessions_visible
    }
    /// Change optional session map visibility.
    pub fn set_sessions_visible(&mut self, visible: bool) {
        self.sessions_visible = visible;
    }
    /// Change explorer visibility without discarding its model.
    pub fn set_explorer_visible(&mut self, visible: bool) {
        if self.explorer_visible == visible {
            return;
        }
        self.explorer_visible = visible;
        self.instrumentation
            .layout_change_started
            .get_or_insert_with(Instant::now);
    }
    /// Store a finite explorer width; actual viewport bounds belong to layout.
    pub fn set_explorer_width(&mut self, width: f32) {
        if width.is_finite() {
            let width = width.clamp(160.0, 1200.0);
            if (width - self.explorer_width).abs() > f32::EPSILON {
                self.explorer_width = width;
                self.instrumentation
                    .layout_change_started
                    .get_or_insert_with(Instant::now);
            }
        }
    }
    /// Switch left surfaces without replacing or stopping the shell.
    pub fn set_left_surface(&mut self, surface: LeftSurface) {
        let previous = self.left_surface;
        if previous != surface {
            let now = Instant::now();
            match (previous, surface) {
                (LeftSurface::Terminal, LeftSurface::Preview) => {
                    self.instrumentation.preview_entered_at = Some(now);
                    self.instrumentation.preview_open_latency = None;
                    self.instrumentation.preview_return_started = None;
                }
                (LeftSurface::Preview, LeftSurface::Terminal) => {
                    let open = self
                        .instrumentation
                        .preview_open_latency
                        .take()
                        .or_else(|| {
                            self.instrumentation
                                .preview_entered_at
                                .map(|started| now.saturating_duration_since(started))
                        })
                        .unwrap_or_default();
                    self.instrumentation.preview_open_latency = Some(open);
                    self.instrumentation.preview_return_started = Some(now);
                }
                _ => {}
            }
            self.left_surface = surface;
        }
        if let Some(terminal) = self.terminal.as_mut() {
            terminal.set_visible(self.visible && surface == LeftSurface::Terminal);
        }
    }
    /// Return from preview to the same live terminal with fresh focus.
    pub fn close_preview(&mut self) {
        self.set_left_surface(LeftSurface::Terminal);
    }

    /// Drop any in-flight preview timing when the preview is invalidated by a
    /// root transition instead of a user close. An auto-close is not a
    /// user-visible preview roundtrip and must not be reported as one.
    fn clear_preview_instrumentation(&mut self) {
        self.instrumentation.preview_entered_at = None;
        self.instrumentation.preview_open_latency = None;
        self.instrumentation.preview_return_started = None;
    }

    /// A root is no longer a safe base for the loaded preview: return the left
    /// surface to the terminal, clear the preview document and pending read,
    /// and mark the retained tree unavailable.
    fn invalidate_preview_for_unavailable_root(&mut self, detail: impl Into<String>) {
        self.close_preview();
        self.clear_preview_instrumentation();
        if let Some(explorer) = self.explorer.as_mut() {
            // Drain a queued open event as well as the loaded document so the
            // next UI pass cannot resurrect the stale preview.
            let _ = explorer.take_preview_opened();
            explorer.clear_preview();
            explorer.mark_root_unavailable(detail);
        }
    }
    /// Explicit workspace activation resets focus and IME, not PTY state.
    pub fn set_visible(&mut self, visible: bool) {
        let was_visible = self.visible;
        if was_visible != visible {
            self.visible = visible;
            if visible {
                self.instrumentation
                    .activation_started
                    .get_or_insert_with(Instant::now);
            } else {
                self.instrumentation.activation_started = None;
            }
        }
        if let Some(terminal) = self.terminal.as_mut() {
            terminal.set_visible(visible && self.left_surface == LeftSurface::Terminal);
        }
    }
    /// Whether this workspace can paint or receive UI input.
    pub fn is_visible(&self) -> bool {
        self.visible
    }
    /// Bounded runtime passes, including inactive workspaces.
    pub fn tick_count(&self) -> u64 {
        self.tick_count
    }
    /// Visible workspace paint passes (hidden polling never increments this).
    pub fn paint_count(&self) -> u64 {
        self.paint_count
    }

    /// Arm a paint-pass measurement from the UI entry point.
    ///
    /// This is called before the UI code runs, but it does not record any
    /// measurement yet. It registers one bounded egui end-of-pass callback (on
    /// first use) and marks the pass pending. The callback timestamps the point
    /// where this workspace has submitted all shapes; the next tick finalizes
    /// activation, explorer-layout, preview-return, hidden-grid, and
    /// cwd-to-root metrics from that timestamp. That keeps the numbers tied to
    /// paint submission rather than layout-entry time.
    pub(crate) fn record_paint(&mut self) {
        self.paint_count = self.paint_count.saturating_add(1);
        self.instrumentation.paint_explorer_visible =
            self.explorer_visible && self.explorer.is_some();
        self.instrumentation.paint_pending = true;
        self.register_paint_callback();
        if let Ok(mut submitted) = self.instrumentation.paint_submitted_at.lock() {
            *submitted = None;
        }
    }

    fn register_paint_callback(&mut self) {
        if self.instrumentation.paint_callback_registered {
            return;
        }
        let Some(ctx) = self.frame_ctx.as_ref() else {
            return;
        };
        let submitted = Arc::clone(&self.instrumentation.paint_submitted_at);
        // One callback per workspace for the process lifetime. Registering per
        // frame would grow egui's callback list without bound.
        ctx.on_end_pass(
            "made-workbench-paint",
            Arc::new(move |_ui| {
                if let Ok(mut slot) = submitted.lock() {
                    *slot = Some(Instant::now());
                }
            }),
        );
        self.instrumentation.paint_callback_registered = true;
    }

    /// Complete metrics for the pass armed by [`Self::record_paint`].
    ///
    /// Runs at the start of the next tick (after egui has submitted the frame)
    /// and in shutdown. Timestamps come from the end-of-pass callback.
    fn finalize_paint_metrics(&mut self) {
        if !self.instrumentation.paint_pending {
            return;
        }
        let Some(submitted_at) = self
            .instrumentation
            .paint_submitted_at
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
        else {
            // The callback has not run yet; keep the pass pending.
            return;
        };
        self.instrumentation.paint_pending = false;

        if let Some(terminal) = self.terminal.as_ref() {
            let grid_paints = terminal.grid_paint_count();
            let delta = grid_paints.saturating_sub(self.instrumentation.last_grid_paints);
            if delta > 0 {
                // The terminal reports whether each grid paint itself happened
                // while visible. A surface transition after a visible paint
                // (for example File -> preview in the same pass) is therefore
                // not misclassified as hidden work.
                if !terminal.last_grid_paint_visible() {
                    self.metrics.record_hidden_grid_paints(delta);
                }
                self.instrumentation.last_grid_paints = grid_paints;
            }
        }

        if let Some(started) = self.instrumentation.activation_started.take() {
            self.metrics
                .record_activation(submitted_at.saturating_duration_since(started));
        }
        if let Some(started) = self.instrumentation.layout_change_started.take() {
            self.metrics
                .record_explorer_layout(submitted_at.saturating_duration_since(started));
        }
        if self.instrumentation.preview_open_latency.is_none()
            && self.left_surface == LeftSurface::Preview
            && self
                .explorer
                .as_ref()
                .is_some_and(|explorer| explorer.preview_document().is_some())
        {
            if let Some(started) = self.instrumentation.preview_entered_at {
                // The first pass that actually paints the loaded preview.
                self.instrumentation.preview_open_latency =
                    Some(submitted_at.saturating_duration_since(started));
            }
        }
        if let Some(return_started) = self.instrumentation.preview_return_started.take() {
            let open = self
                .instrumentation
                .preview_open_latency
                .take()
                .unwrap_or_default();
            self.metrics.record_preview_roundtrip(
                open.saturating_add(submitted_at.saturating_duration_since(return_started)),
            );
        }
        if self.instrumentation.root_switch_origin == Some(RootSwitchOrigin::ShellCwd) {
            if self.instrumentation.paint_explorer_visible {
                if let Some(started) = self.instrumentation.cwd_root_started_at.take() {
                    self.metrics
                        .record_cwd_to_root(submitted_at.saturating_duration_since(started));
                }
            }
            // Keep a pending measurement while the explorer is hidden; the
            // first pass that actually paints it completes the metric.
            if self.instrumentation.cwd_root_started_at.is_none() {
                self.instrumentation.root_switch_origin = None;
            }
        }
    }

    /// Explorer pane, when the workspace is valid.
    pub fn explorer(&self) -> Option<&ExplorerPane> {
        self.explorer.as_ref()
    }

    /// Mutable explorer pane for explicit user actions and tests.
    pub fn explorer_mut(&mut self) -> Option<&mut ExplorerPane> {
        self.explorer.as_mut()
    }

    /// Workspace validation error from launch, if any.
    pub fn workspace_error(&self) -> Option<&WorkspaceError> {
        self.workspace_error.as_ref()
    }

    /// Terminal spawn error, if any.
    pub fn terminal_error(&self) -> Option<&str> {
        self.terminal_error.as_deref()
    }

    /// Latest worktree metadata.
    pub fn worktree(&self) -> Option<&WorktreeMetadata> {
        self.worktree.metadata.as_ref()
    }

    /// Worktree header label.
    pub fn worktree_label(&self) -> String {
        worktree_indicator(self.worktree.metadata.as_ref(), self.worktree.loading)
    }

    /// Active session identity from the follow coordinator.
    pub fn active_session(&self) -> Option<&crate::session_follow::FollowedSession> {
        self.follow
            .as_ref()
            .and_then(SessionFollowCoordinator::active_session)
    }

    /// Current task notice, if one is still fresh.
    pub fn notice(&self) -> Option<&str> {
        self.pending_notice
            .as_ref()
            .filter(|(_, until)| Instant::now() < *until)
            .map(|(text, _)| text.as_str())
    }

    /// Record every frame regardless of activity. Used by the smoke run so its
    /// p95 describes the real render loop rather than the idle polling cadence.
    pub fn set_benchmark_frames(&mut self, enabled: bool) {
        self.benchmark_frames = enabled;
    }

    /// Set a transient status notice.
    pub fn set_notice(&mut self, notice: impl Into<String>) {
        let notice = sanitize_copy_text(&notice.into(), self.token.as_ref(), 240);
        self.pending_notice = Some((notice, Instant::now() + NOTICE_TTL));
    }

    /// Request focus for a session from the session map.
    pub fn focus_session(&mut self, session_id: &str) {
        let Some(bridge) = self.bridge.as_ref() else {
            self.set_notice("Focus is unavailable: the integration is not connected.");
            return;
        };
        let update = self.focus.request(bridge, session_id.to_owned());
        self.handle_focus_update(update);
    }

    /// Dismiss the visible focus error.
    pub fn dismiss_focus_error(&mut self) {
        self.focus.clear_error();
    }

    fn handle_focus_update(&mut self, update: FocusUpdate) {
        match update.outcome {
            FocusOutcome::Started => {
                self.diag.log_mut().record(
                    LogLevel::Info,
                    format!(
                        "focus requested{}",
                        update
                            .session_id
                            .as_ref()
                            .map(|session| format!(" for {session}"))
                            .unwrap_or_default()
                    ),
                );
                self.last_activity = Instant::now();
            }
            FocusOutcome::Succeeded => {
                self.integration.note_focus_succeeded();
                self.diag
                    .log_mut()
                    .record(LogLevel::Info, "focus request succeeded");
            }
            FocusOutcome::Failed | FocusOutcome::SendFailed => {
                if let Some(error) = &update.error {
                    if error.kind == FocusErrorKind::TimedOut {
                        self.integration.note_focus_timed_out();
                    } else {
                        self.integration.note_focus_failed(error.message.clone());
                    }
                    self.diag.log_mut().record(
                        LogLevel::Warn,
                        format!("focus request failed: {}", error.message),
                    );
                    self.set_notice(format!("Focus failed: {}", error.message));
                }
            }
            FocusOutcome::TimedOut => {
                self.integration.note_focus_timed_out();
                self.diag
                    .log_mut()
                    .record(LogLevel::Warn, "focus request timed out");
                self.set_notice("Focus failed: the request timed out.");
            }
            FocusOutcome::Busy => {
                self.set_notice("A focus request is already pending.");
            }
            FocusOutcome::Idle | FocusOutcome::Uncorrelated => {}
        }
        if update.state_refresh_triggered() {
            self.diag
                .log_mut()
                .record(LogLevel::Info, "requested a fresh session state");
        }
        self.last_activity = Instant::now();
    }

    /// Poll PTY, bridge, explorer, focus, and worktree work.
    pub fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let frame_time = now.saturating_duration_since(self.last_frame);
        self.last_frame = now;
        self.frame_ctx = Some(ctx.clone());
        self.finalize_paint_metrics();

        self.tick_count = self.tick_count.saturating_add(1);
        let mut active = false;

        let terminal_visible = self.visible && self.left_surface == LeftSurface::Terminal;
        let mut hidden_output_bytes = 0_u64;
        if let Some(terminal) = self.terminal.as_mut() {
            terminal.sync_visibility(ctx, self.diag.log_mut());
            let output_before = terminal.output_bytes();
            if terminal.pump(self.diag.log_mut()) || terminal.has_pending_input() {
                active = true;
                self.last_activity = now;
            }
            if !terminal_visible {
                hidden_output_bytes = terminal.output_bytes().saturating_sub(output_before);
            }
        }
        self.metrics.record_hidden_output(hidden_output_bytes);

        let cwd_change = self
            .terminal
            .as_mut()
            .and_then(TerminalPane::take_cwd_change);
        if let Some((cwd, cwd_observed_at)) = cwd_change {
            if note_terminal_cwd_once(&mut self.last_osc7_cwd, &cwd) {
                let outcome = self
                    .follow
                    .as_mut()
                    .map(|follow| follow.follow_shell_cwd(&cwd));
                match outcome {
                    Some(Ok(outcome)) => {
                        self.shell_cwd_error = None;
                        if outcome.active_session.is_none() {
                            self.shell_root_unavailable = None;
                            if let Some(explorer) = self.explorer.as_mut() {
                                explorer.clear_root_unavailable();
                            }
                        }
                        if let Some(validated) = self
                            .follow
                            .as_ref()
                            .and_then(SessionFollowCoordinator::terminal_cwd)
                        {
                            self.integration.set_terminal_cwd(validated);
                            self.terminal_cwd = validated.to_path_buf();
                        }
                        if outcome.root_switched
                            || self.explorer_generation_mismatch(outcome.generation)
                        {
                            self.instrumentation.cwd_root_started_at = Some(cwd_observed_at);
                            self.switch_root(
                                outcome.root,
                                outcome.generation,
                                RootSwitchOrigin::ShellCwd,
                            );
                            // Completed by the first pass that actually
                            // paints the explorer, using the end-of-pass
                            // timestamp. Recorded in `finalize_paint_metrics`.
                        } else if self.instrumentation.root_switch_origin
                            != Some(RootSwitchOrigin::ShellCwd)
                        {
                            self.instrumentation.cwd_root_started_at = None;
                        }
                    }
                    Some(Err(error)) => {
                        let detail = error.message();
                        self.shell_cwd_error = Some(detail.clone());
                        self.diag.log_mut().record(
                            LogLevel::Warn,
                            format!("shell directory unavailable: {detail}"),
                        );
                        // Shell errors are tracked independently. They mark the
                        // effective root unavailable only while the shell owns
                        // precedence; an authoritative agent root is untouched.
                        let shell_owns_root = self
                            .follow
                            .as_ref()
                            .is_none_or(|follow| follow.active_session().is_none());
                        if shell_owns_root {
                            self.invalidate_preview_for_unavailable_root(detail);
                        }
                    }
                    None => {}
                }
            }
        }

        if self.drain_bridge_events() {
            active = true;
            self.last_activity = now;
        }

        if let Some(explorer) = self.explorer.as_mut() {
            explorer.tick(self.diag.log_mut());
            if let Some(latency) = explorer.take_switch_latency() {
                self.metrics.record_session_switch(latency);
                self.last_activity = now;
                active = true;
            }
        }

        self.poll_shell_root_revalidation(now);
        self.maybe_schedule_shell_root_revalidation(now);

        self.try_send_pending_worktree();
        if self.poll_worktree() {
            active = true;
        }

        let bridge_counters = self
            .bridge
            .as_ref()
            .map(|bridge| (bridge.dropped_events(), bridge.coalesced_states()));
        if let Some((dropped, coalesced)) = bridge_counters {
            self.metrics.set_bridge_counters(dropped, coalesced);
        }
        if let Some(bridge) = self.bridge.as_ref() {
            let update = self.focus.tick(bridge);
            if update.outcome != FocusOutcome::Idle {
                self.handle_focus_update(update);
            }
        }

        let explorer_ready = self
            .explorer
            .as_ref()
            .is_none_or(|explorer| explorer.is_root_ready());
        if !self.startup_recorded && self.terminal.is_some() && explorer_ready {
            self.metrics.record_startup(self.started_at.elapsed());
            self.startup_recorded = true;
            self.diag.log_mut().record(
                LogLevel::Info,
                format!(
                    "startup to interactive shell: {:.0}ms",
                    self.started_at.elapsed().as_secs_f64() * 1000.0
                ),
            );
        }

        if self.visible {
            self.update_window_title(ctx);
        }

        let recent_activity =
            now.saturating_duration_since(self.last_activity) < Duration::from_millis(250);
        let sampleable = self.last_tick_active;
        if self.benchmark_frames || ((active || recent_activity) && sampleable) {
            self.metrics.record_frame(frame_time);
        }
        self.last_tick_active = active || recent_activity;

        if active || recent_activity {
            ctx.request_repaint_after(crate::metrics::ACTIVE_REPAINT_INTERVAL);
        } else if self.bridge.as_ref().is_some_and(BridgeHandle::is_connected) {
            ctx.request_repaint_after(Duration::from_millis(50));
        } else {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
    }

    /// Apply a background availability observation for the shell-owned root.
    ///
    /// Results are gated on both authority (no active agent session) and the
    /// exact root generation/path observed. A late result from a replaced root
    /// or from before an agent took precedence is discarded.
    fn poll_shell_root_revalidation(&mut self, now: Instant) {
        let mut latest = None;
        while let Some(result) = self.shell_root_worker.try_recv() {
            latest = Some(result);
        }
        let Some(result) = latest else {
            return;
        };
        self.shell_root_check_in_flight = false;

        let shell_authoritative = self.follow.as_ref().is_some_and(|follow| {
            follow.active_session().is_none() && follow.generation() == result.generation
        });
        if !shell_authoritative || self.explorer_root() != Some(result.path.as_path()) {
            return;
        }

        match apply_shell_root_availability(
            &mut self.shell_root_unavailable,
            &result.path,
            &result.result,
        ) {
            ShellRootTransition::Unchanged => {}
            ShellRootTransition::BecameUnavailable(detail) => {
                self.diag.log_mut().record(
                    LogLevel::Warn,
                    format!(
                        "shell directory unavailable at {}: {}; keeping the last usable tree",
                        result.path.display(),
                        detail
                    ),
                );
                self.invalidate_preview_for_unavailable_root(detail);
                self.last_activity = now;
            }
            ShellRootTransition::Recovered => {
                self.diag.log_mut().record(
                    LogLevel::Info,
                    format!(
                        "shell directory available again at {}",
                        result.path.display()
                    ),
                );
                self.close_preview();
                if let Some(explorer) = self.explorer.as_mut() {
                    explorer.clear_root_unavailable();
                    // The directory may have a new inode (delete + recreate).
                    // Reinstall the root so the watcher and tree include changes
                    // made while it was absent.
                    explorer.retry();
                }
                self.last_activity = now;
            }
        }
    }

    /// Offer a low-frequency check only while the shell owns the root. The
    /// worker has a bounded one-slot queue and never blocks a frame.
    fn maybe_schedule_shell_root_revalidation(&mut self, now: Instant) {
        if self.shutdown_done || self.shell_root_check_in_flight {
            return;
        }
        let Some(follow) = self.follow.as_ref() else {
            return;
        };
        if follow.active_session().is_some() {
            return;
        }
        if now.saturating_duration_since(self.shell_root_checked_at)
            < self.shell_root_check_interval
        {
            return;
        }
        let Some(root) = self.explorer_root().map(Path::to_path_buf) else {
            return;
        };
        let generation = follow.generation();
        if self.shell_root_worker.request(root, generation) {
            self.shell_root_check_in_flight = true;
            self.shell_root_checked_at = now;
        }
    }

    fn drain_bridge_events(&mut self) -> bool {
        let Some(bridge) = self.bridge.as_ref() else {
            return false;
        };
        let mut events = Vec::new();
        for _ in 0..MAX_BRIDGE_EVENTS_PER_FRAME {
            match bridge.try_recv() {
                Ok(event) => events.push(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        if events.is_empty() {
            return false;
        }
        for event in events {
            self.handle_bridge_event(event);
        }
        true
    }

    fn handle_bridge_event(&mut self, event: BridgeEvent) {
        // Rejected clients can reach the bridge long enough to emit protocol
        // errors and disconnects. Only events owned by the current
        // authenticated connection may affect integration, follow, focus, or
        // session state.
        if !self.integration.accepts_event(&event) {
            self.log_ignored_connection_event(&event);
            return;
        }

        // Focus results are correlated by the focus controller; applying them
        // here first could turn an uncorrelated/stale result into a false
        // integration failure.
        if !matches!(event, BridgeEvent::FocusResult(_)) {
            let change = self.integration.apply_bridge_event(&event);
            if change.changed() {
                self.diag.record_status(&change);
            }
        }

        if !matches!(&event, BridgeEvent::State(_)) {
            let outcome = self
                .follow
                .as_mut()
                .and_then(|follow| follow.apply_bridge_event(&event));
            if let Some(outcome) = outcome {
                if outcome.root_switched || self.explorer_generation_mismatch(outcome.generation) {
                    self.switch_root(
                        outcome.root.clone(),
                        outcome.generation,
                        RootSwitchOrigin::Session,
                    );
                }
                if matches!(outcome.update, SessionFollowUpdate::NoActiveSession) {
                    self.settle_no_active_root(&outcome.root);
                }
            }
        }

        match event {
            BridgeEvent::Connected(info) => {
                self.focus.cancel();
                self.focus.clear_error();
                self.session_map.clear();
                if self.integration.status().is_connected() {
                    self.diag.log_mut().push(
                        LogEntry::new(
                            LogLevel::Info,
                            format!(
                                "OpenCode integration connected (plugin {}, OpenCode {}, protocol {})",
                                info.plugin_version, info.opencode_version, info.protocol_version
                            ),
                        )
                        .with_connection(info.connection_id),
                    );
                } else {
                    self.diag.log_mut().push(
                        LogEntry::new(
                            LogLevel::Warn,
                            format!(
                                "OpenCode integration rejected: {}",
                                self.integration.reason().human()
                            ),
                        )
                        .with_connection(info.connection_id),
                    );
                }
            }
            BridgeEvent::State(snapshot) => self.handle_state(snapshot),
            BridgeEvent::FocusResult(result) => {
                if let Some(bridge) = self.bridge.as_ref() {
                    let update = self.focus.handle_result(bridge, result);
                    self.handle_focus_update(update);
                }
            }
            BridgeEvent::Disconnected(info) => {
                if !info.replaced {
                    self.focus.cancel();
                    self.session_map.clear();
                    self.diag.log_mut().push(
                        LogEntry::new(
                            LogLevel::Warn,
                            format!("integration disconnected: {}", info.reason),
                        )
                        .with_connection(info.connection_id),
                    );
                }
            }
            BridgeEvent::ProtocolError(error) => {
                if error.fatal {
                    self.focus.cancel();
                }
                self.diag.log_mut().push(
                    LogEntry::new(
                        LogLevel::Warn,
                        format!("integration protocol error: {}", error.message),
                    )
                    .with_reason(error.code),
                );
            }
            BridgeEvent::HeartbeatTimeout(timeout) => {
                self.focus.cancel();
                self.session_map.clear();
                self.diag.log_mut().push(
                    LogEntry::new(
                        LogLevel::Warn,
                        format!(
                            "integration heartbeat timed out after {} missed pongs",
                            timeout.missed_pongs
                        ),
                    )
                    .with_connection(timeout.connection_id),
                );
            }
            BridgeEvent::Goodbye(goodbye) => {
                self.focus.cancel();
                self.session_map.clear();
                self.diag.log_mut().push(
                    LogEntry::new(
                        LogLevel::Info,
                        format!("OpenCode integration unloaded: {}", goodbye.reason),
                    )
                    .with_connection(goodbye.connection_id),
                );
            }
        }
    }

    fn handle_state(&mut self, snapshot: SessionSnapshot) {
        let received_at = Instant::now();
        if self.integration.connection().is_none() || self.integration.status().is_unsupported() {
            self.diag.log_mut().record(
                LogLevel::Warn,
                "ignored session state from an unsupported integration",
            );
            return;
        }
        let outcome = self
            .follow
            .as_mut()
            .map(|follow| follow.apply_snapshot(snapshot.clone()));
        let accepted = outcome.as_ref().is_none_or(|outcome| {
            !matches!(
                outcome.update,
                SessionFollowUpdate::StaleRevision { .. }
                    | SessionFollowUpdate::RejectedSnapshot { .. }
            )
        });
        if !accepted {
            if let Some(outcome) = &outcome {
                match &outcome.update {
                    SessionFollowUpdate::StaleRevision {
                        revision,
                        last_accepted,
                    } => {
                        self.diag.log_mut().record(
                            LogLevel::Info,
                            format!(
                                "discarded stale session state revision {revision} (last accepted {last_accepted})"
                            ),
                        );
                    }
                    SessionFollowUpdate::RejectedSnapshot { detail } => {
                        self.diag.log_mut().record(
                            LogLevel::Warn,
                            format!("discarded invalid session state: {detail}"),
                        );
                    }
                    _ => {}
                }
            }
            return;
        }

        let map_update = self.session_map.apply_snapshot(&snapshot);
        let active_changed = map_update.active_changed;
        if !map_update.is_empty() {
            self.last_activity = Instant::now();
        }

        let Some(outcome) = outcome else {
            return;
        };
        match &outcome.update {
            SessionFollowUpdate::ActiveSession => {
                self.shell_root_unavailable = None;
                if let Some(explorer) = self.explorer.as_mut() {
                    explorer.clear_root_unavailable();
                }
                self.integration.note_root(&outcome.root);
                if let Some(active) = &outcome.active_session {
                    self.diag.log_mut().push(
                        LogEntry::new(
                            LogLevel::Info,
                            format!("active session {} ({})", active.session_id, active.title),
                        )
                        .with_session(active.session_id.clone()),
                    );
                }
                if outcome.root_switched || self.explorer_generation_mismatch(outcome.generation) {
                    self.switch_root(
                        outcome.root.clone(),
                        outcome.generation,
                        RootSwitchOrigin::Session,
                    );
                } else if active_changed {
                    // The explorer root did not change; the session map/active
                    // identity updated in this frame.
                    self.metrics.record_session_switch(received_at.elapsed());
                    // The branch may have changed under the same worktree root;
                    // refresh the read-only header metadata.
                    self.request_worktree(outcome.root.clone(), outcome.generation);
                }
            }
            SessionFollowUpdate::ActiveDirectoryUnavailable {
                session_id, detail, ..
            } => {
                self.integration
                    .note_active_directory_unavailable(session_id.clone(), detail.clone());
                self.present_unavailable_root(
                    outcome.root.clone(),
                    outcome.generation,
                    detail.clone(),
                );
                self.diag.log_mut().push(
                    LogEntry::new(
                        LogLevel::Warn,
                        format!("active session directory unavailable: {detail}"),
                    )
                    .with_session(session_id.clone()),
                );
            }
            SessionFollowUpdate::NoActiveSession => {
                if outcome.root_switched || self.explorer_generation_mismatch(outcome.generation) {
                    self.switch_root(
                        outcome.root.clone(),
                        outcome.generation,
                        RootSwitchOrigin::Session,
                    );
                }
                self.settle_no_active_root(&outcome.root);
            }
            _ => {}
        }
    }

    fn explorer_generation_mismatch(&self, generation: RootGeneration) -> bool {
        self.explorer
            .as_ref()
            .is_some_and(|explorer| explorer.generation() != generation)
    }

    /// No active session released the agent root. If the resulting fallback is
    /// itself gone, do not leave the previous preview/tree presented as usable:
    /// clear the preview and return to the explicit unavailable state.
    fn settle_no_active_root(&mut self, root: &Path) {
        if usable_directory(root) {
            self.shell_root_unavailable = None;
            if let Some(explorer) = self.explorer.as_mut() {
                explorer.clear_root_unavailable();
            }
        } else {
            self.invalidate_preview_for_unavailable_root(format!(
                "the effective root {} is unavailable",
                root.display()
            ));
        }
    }

    /// Present the last usable tree under an unavailable marker without
    /// treating the unavailable directory as current.
    ///
    /// When the generation advanced, the explorer is rebuilt at the last usable
    /// root so tree, watcher, and preview work started for the previous root
    /// identity cannot be applied. The preview surface and its pending reads
    /// are closed before the rebuild. If the last usable root itself no longer
    /// exists, the retained pane is kept and marked unavailable.
    fn present_unavailable_root(
        &mut self,
        root: PathBuf,
        generation: RootGeneration,
        detail: String,
    ) {
        // This covers both the rebuilt-tree and `keep_cached_tree` branches,
        // including the case where the last usable root itself was deleted.
        self.invalidate_preview_for_unavailable_root(detail.clone());
        self.shell_root_unavailable = None;
        let needs_rebuild = self
            .explorer
            .as_ref()
            .is_none_or(|explorer| explorer.generation() != generation);
        let keep_cached_tree = self.explorer.is_some() && !usable_directory(&root);
        if needs_rebuild && !keep_cached_tree {
            let mut explorer = ExplorerPane::new(&root, generation);
            explorer.mark_root_unavailable(detail);
            self.explorer = Some(explorer);
            self.request_worktree(root, generation);
            self.diag.log_mut().record(
                LogLevel::Info,
                format!(
                    "explorer root invalidated for an unavailable directory (generation {})",
                    generation
                ),
            );
        }
        self.last_activity = Instant::now();
    }

    fn log_ignored_connection_event(&mut self, event: &BridgeEvent) {
        let kind = match event {
            BridgeEvent::ProtocolError(_) => "protocol error",
            BridgeEvent::Disconnected(_) => "disconnect",
            BridgeEvent::HeartbeatTimeout(_) => "heartbeat timeout",
            BridgeEvent::Goodbye(_) => "goodbye",
            _ => "event",
        };
        let connection = event
            .connection_id()
            .map_or_else(|| "unauthenticated".to_owned(), |id| id.to_string());
        let current = self
            .integration
            .connection()
            .map_or_else(|| "none".to_owned(), |info| info.connection_id.to_string());
        let code = match event {
            BridgeEvent::ProtocolError(error) => format!(" ({})", error.code),
            _ => String::new(),
        };
        self.diag.log_mut().record(
            LogLevel::Info,
            format!(
                "ignored {kind}{code} from non-current connection {connection} (current {current})"
            ),
        );
    }

    fn switch_root(&mut self, root: PathBuf, generation: RootGeneration, origin: RootSwitchOrigin) {
        self.close_preview();
        self.clear_preview_instrumentation();
        if origin != RootSwitchOrigin::ShellCwd {
            self.instrumentation.cwd_root_started_at = None;
        }
        self.instrumentation.root_switch_origin = Some(origin);
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        *explorer = ExplorerPane::new(&root, generation);
        explorer.begin_switch_measurement();
        self.diag.log_mut().record(
            LogLevel::Info,
            format!(
                "explorer root switched to {} (generation {})",
                root.display(),
                generation
            ),
        );
        self.request_worktree(root, generation);
        self.last_activity = Instant::now();
    }

    fn request_worktree(&mut self, root: PathBuf, generation: RootGeneration) {
        if self.worktree.generation != generation {
            self.worktree.metadata = None;
        }
        self.worktree.generation = generation;
        self.worktree.loading = true;
        self.pending_worktree.offer(root, generation);
        self.try_send_pending_worktree();
    }

    fn try_send_pending_worktree(&mut self) {
        let worker = &self.worktree_worker;
        if self
            .pending_worktree
            .try_send(|root, generation| worker.request(root.to_path_buf(), generation))
        {
            self.worktree.loading = true;
        }
    }

    fn poll_worktree(&mut self) -> bool {
        let mut changed = false;
        while let Some(result) = self.worktree_worker.try_recv() {
            let current = self
                .follow
                .as_ref()
                .map(SessionFollowCoordinator::generation)
                .unwrap_or(self.worktree.generation);
            if result.generation != current {
                continue;
            }
            self.worktree.generation = result.generation;
            self.worktree.loading = false;
            self.worktree.metadata = Some(result.metadata.clone());
            changed = true;
            let status = result.metadata.status;
            match status {
                MetadataStatus::Known => {
                    self.diag.log_mut().record(
                        LogLevel::Info,
                        format!("worktree metadata: {}", result.metadata.header_label()),
                    );
                }
                MetadataStatus::Unknown => {
                    self.diag.log_mut().record(
                        LogLevel::Info,
                        "worktree metadata unknown (plain folder or non-git root)",
                    );
                }
                MetadataStatus::Unavailable => {
                    self.diag.log_mut().record(
                        LogLevel::Warn,
                        "git metadata unavailable; explorer continues as a plain folder",
                    );
                }
            }
        }
        changed
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let workspace = self
            .plan
            .as_ref()
            .map(|plan| plan.workspace.display_name().to_owned())
            .unwrap_or_else(|| "workspace unavailable".to_owned());
        let terminal_title = self
            .terminal
            .as_ref()
            .and_then(TerminalPane::title)
            .map(|title| sanitize_display(title.trim(), 160))
            .filter(|title| !title.is_empty());
        let title = match terminal_title {
            Some(title) => format!("{title} — {BASE_TITLE}"),
            None => format!("{BASE_TITLE} — {workspace}"),
        };
        let title = sanitize_copy_text(&title, self.token.as_ref(), 220);
        if self.last_title.as_deref() != Some(title.as_str()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = Some(title);
        }
    }

    /// Terminate the child, stop the bridge, verify config immutability, and
    /// write diagnostics. Idempotent.
    pub fn shutdown(&mut self) {
        if self.shutdown_done {
            return;
        }
        self.shutdown_done = true;

        // Preserve the selected workspace's capability/reason before the
        // terminal (the live capability owner) is dropped. The persisted
        // diagnostics are written after termination, so the snapshot must not
        // depend on the terminal still existing.
        self.refresh_shell_capability();
        self.finalize_paint_metrics();
        self.shell_root_worker.shutdown();
        self.shell_root_check_in_flight = false;

        if let Some(mut terminal) = self.terminal.take() {
            terminal.terminate(self.diag.log_mut());
        }
        if let Some(bridge) = self.bridge.take() {
            if let Err(error) = bridge.shutdown() {
                self.diag
                    .log_mut()
                    .record(LogLevel::Warn, format!("bridge shutdown failed: {error}"));
            }
        }
        if let Some(guard) = self.config_guard.take() {
            match guard.after_session() {
                Ok(ConfigVerification::Unchanged { state, .. }) => {
                    self.diag.log_mut().record(
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
                    self.diag.log_mut().record(
                        LogLevel::Warn,
                        "user OpenCode config changed during the session (Made never writes it)",
                    );
                }
                Err(error) => {
                    self.diag.log_mut().record(
                        LogLevel::Warn,
                        format!("could not verify user OpenCode config: {error}"),
                    );
                }
            }
        }

        self.diag.log_mut().record(
            LogLevel::Info,
            format!(
                "shutting down; {} session row(s), status {}",
                self.session_map.len(),
                self.integration.status().label()
            ),
        );
        let performance = self.metrics.report();
        for line in performance.to_text().lines() {
            self.diag.log_mut().record(LogLevel::Info, line);
        }
        self.write_diagnostics_log();
        self.token = None;
    }

    fn write_diagnostics_log(&mut self) {
        let Some(path) = self.log_path.clone() else {
            let message = "diagnostics log unavailable: app paths are unavailable (fail-closed)";
            self.log_write_issue = Some(LogIssue::error(message));
            eprintln!("made: {message}");
            return;
        };

        let snapshot = self.diagnostics_snapshot();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        // Build and scan the exact record that is written; no second render.
        let mut record = format!("--- made diagnostics (unix {timestamp}) ---\n");
        record.push_str(&snapshot.to_text());
        if let Some(token) = &self.token {
            let secret = token.expose_secret();
            if !secret.is_empty() && record.contains(secret) {
                let message = "refused to write diagnostics containing the bridge secret";
                self.log_write_issue = Some(LogIssue::error(message));
                eprintln!("made: {message}");
                return;
            }
        }

        match made_platform::append_private_log_rotated(
            &path,
            record.as_bytes(),
            MAX_LOG_FILE_BYTES,
        ) {
            Ok(made_platform::LogAppendOutcome::Written { rotated: _, issue }) => {
                self.log_write_issue = issue;
                if let Some(issue) = &self.log_write_issue {
                    eprintln!("made: {}", issue.message);
                }
            }
            Ok(made_platform::LogAppendOutcome::Refused { issue }) => {
                eprintln!("made: {}", issue.message);
                self.log_write_issue = Some(issue);
            }
            Err(error) => {
                let message = format!("diagnostics log write failed: {error}");
                self.log_write_issue = Some(LogIssue::error(message.clone()));
                eprintln!("made: {message}");
            }
        }
    }

    /// Selected-workspace shell/root context for diagnostics.
    ///
    /// This is available even while the optional integration is inactive and
    /// keeps the same field names consumed by the UI.
    pub fn workspace_diagnostics_context(&self) -> WorkspaceDiagnostics {
        let label = self
            .plan
            .as_ref()
            .map(|plan| plan.workspace.display_name().to_owned())
            .unwrap_or_else(|| "Workspace unavailable".to_owned());
        let path = self
            .plan
            .as_ref()
            .map(|plan| plan.workspace.path().to_path_buf());
        let capability = self.shell_cwd_capability();
        WorkspaceDiagnostics {
            identity: self.pane_id.clone(),
            label,
            path,
            shell_cwd: Some(self.terminal_cwd.clone()),
            cwd_supported: capability.map(made_platform::ShellCwdCapability::is_supported),
            cwd_reason: capability.map(|capability| capability.reason.clone()),
            root_source: Some(self.root_source().to_owned()),
            watcher: self.explorer.as_ref().map(|explorer| {
                if explorer.has_watcher() {
                    "live".to_owned()
                } else if let Some(error) = explorer.watcher_error() {
                    format!("unavailable: {}", sanitize_display(error, 300))
                } else if explorer.watcher_loading() {
                    "starting".to_owned()
                } else {
                    "unavailable".to_owned()
                }
            }),
        }
    }

    /// Build the immutable diagnostics view for the selected workspace.
    pub fn diagnostics_snapshot(&self) -> DiagnosticsSnapshot {
        let mut snapshot = self.diag.snapshot(
            &self.integration,
            self.active_session(),
            self.explorer_root(),
            self.worktree(),
        );
        snapshot.attach_workspace(self.workspace_diagnostics_context());
        snapshot
    }

    /// Sanitized and token-redacted log path for the clipboard.
    pub fn sanitized_log_path(&self) -> String {
        sanitize_copy_text(self.diag.copyable_log_path(), self.token.as_ref(), 2048)
    }

    /// Sanitized and token-redacted full diagnostics text for the clipboard.
    pub fn sanitized_diagnostics_text(&self) -> String {
        let mut text = self.diagnostics_snapshot().to_text();
        text.push_str("performance:\n");
        text.push_str(&self.metrics.report().to_text());
        sanitize_copy_text(&text, self.token.as_ref(), 1024 * 1024)
    }

    /// Whether the integration is connected.
    pub fn is_connected(&self) -> bool {
        self.integration.status() == IntegrationStatus::Connected
    }

    /// Short smoke summary for the CLI smoke marker. Reports observed state,
    /// never a fabricated success.
    pub fn smoke_summary(&self) -> String {
        let terminal = match &self.terminal {
            Some(pane) => format!(
                "running={} exit={}",
                pane.is_running(),
                pane.exit_status()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "none".to_owned())
            ),
            None => "absent".to_owned(),
        };
        let explorer = match &self.explorer {
            Some(pane) => format!(
                "root={} error={}",
                pane.root()
                    .map(|root| root.display().to_string())
                    .unwrap_or_else(|| "none".to_owned()),
                pane.root_error().unwrap_or("none")
            ),
            None => "absent".to_owned(),
        };
        format!(
            "status={} reason={} terminal=[{}] explorer=[{}]",
            self.integration.status().label(),
            self.integration.reason().code.code(),
            sanitize_display(&terminal, 240),
            sanitize_display(&explorer, 360)
        )
    }
}

impl Drop for Workbench {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Default)]
struct PendingWorktreeRequest {
    pending: Option<(PathBuf, RootGeneration)>,
}

impl PendingWorktreeRequest {
    fn offer(&mut self, root: PathBuf, generation: RootGeneration) {
        // Latest request wins; older pending generations are coalesced away.
        self.pending = Some((root, generation));
    }

    #[cfg(test)]
    fn peek(&self) -> Option<&(PathBuf, RootGeneration)> {
        self.pending.as_ref()
    }

    fn try_send(&mut self, mut send: impl FnMut(&Path, RootGeneration) -> bool) -> bool {
        let Some((root, generation)) = self.pending.as_ref() else {
            return false;
        };
        if send(root, *generation) {
            self.pending = None;
            true
        } else {
            false
        }
    }

    #[cfg(test)]
    fn take(&mut self) -> Option<(PathBuf, RootGeneration)> {
        self.pending.take()
    }

    #[cfg(test)]
    fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
}

struct WorktreeRequest {
    root: PathBuf,
    generation: RootGeneration,
}
struct WorktreeResult {
    generation: RootGeneration,
    metadata: WorktreeMetadata,
}

struct WorktreeWorker {
    jobs: SyncSender<WorktreeRequest>,
    results: Receiver<WorktreeResult>,
}

impl WorktreeWorker {
    fn start() -> Self {
        let (jobs, job_rx) = mpsc::sync_channel::<WorktreeRequest>(4);
        let (result_tx, results) = mpsc::sync_channel::<WorktreeResult>(8);
        thread::Builder::new()
            .name("made-worktree-metadata".to_owned())
            .spawn(move || {
                let runner = GitRunner::from_env();
                while let Ok(job) = job_rx.recv() {
                    let metadata = runner.read_metadata(&job.root);
                    if result_tx
                        .send(WorktreeResult {
                            generation: job.generation,
                            metadata,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .ok();
        Self { jobs, results }
    }

    fn request(&self, root: PathBuf, generation: RootGeneration) -> bool {
        self.jobs
            .try_send(WorktreeRequest { root, generation })
            .is_ok()
    }

    fn try_recv(&self) -> Option<WorktreeResult> {
        self.results.try_recv().ok()
    }
}

struct ShellRootCheckRequest {
    path: PathBuf,
    generation: RootGeneration,
}

struct ShellRootCheckResult {
    path: PathBuf,
    generation: RootGeneration,
    result: Result<(), String>,
}

/// Private one-slot worker for low-frequency shell-root availability checks.
///
/// The UI thread only offers a path with `try_send` and polls results with
/// `try_recv`; directory canonicalization and `read_dir` never run on the frame
/// path. Dropping the workbench closes the job channel and joins the thread,
/// releasing the check on tab close.
struct ShellRootWorker {
    jobs: Option<SyncSender<ShellRootCheckRequest>>,
    results: Receiver<ShellRootCheckResult>,
    handle: Option<thread::JoinHandle<()>>,
}

impl ShellRootWorker {
    fn start() -> Self {
        let (jobs, job_rx) = mpsc::sync_channel::<ShellRootCheckRequest>(1);
        let (result_tx, results) = mpsc::sync_channel::<ShellRootCheckResult>(4);
        let handle = thread::Builder::new()
            .name("made-shell-root-check".to_owned())
            .spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    let result = probe_directory(&job.path);
                    if result_tx
                        .send(ShellRootCheckResult {
                            path: job.path,
                            generation: job.generation,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .ok();
        Self {
            jobs: Some(jobs),
            results,
            handle,
        }
    }

    fn request(&self, path: PathBuf, generation: RootGeneration) -> bool {
        self.jobs.as_ref().is_some_and(|jobs| {
            jobs.try_send(ShellRootCheckRequest { path, generation })
                .is_ok()
        })
    }

    fn try_recv(&self) -> Option<ShellRootCheckResult> {
        self.results.try_recv().ok()
    }

    fn shutdown(&mut self) {
        self.jobs.take();
        // Drain before joining so a completed check cannot block on a full
        // result channel while shutdown waits for the thread.
        while self.results.try_recv().is_ok() {}
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ShellRootWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn diagnostics_log_path(paths: Option<&AppPaths>) -> Option<PathBuf> {
    // If app paths are unavailable, fail closed. A shared $TMPDIR path would be
    // predictable and symlink-attackable.
    paths.map(|paths| paths.log_dir.join(DEFAULT_LOG_FILE))
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

fn usable_directory(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

/// Validate that a path is still a readable, local directory without changing
/// the caller's UI. Used only by the background revalidator.
fn probe_directory(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return Err("directory path is empty or not absolute".to_owned());
    }
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("directory cannot be resolved: {error}"))?;
    let metadata = std::fs::metadata(&canonical)
        .map_err(|error| format!("directory cannot be inspected: {error}"))?;
    if !metadata.is_dir() {
        return Err("path is not a directory".to_owned());
    }
    let mut entries = std::fs::read_dir(&canonical)
        .map_err(|error| format!("directory cannot be read: {error}"))?;
    if let Some(Err(error)) = entries.next() {
        return Err(format!("directory cannot be read: {error}"));
    }
    Ok(())
}

/// Update the cached OSC 7 cwd and report whether it changed.
pub(crate) fn note_terminal_cwd_once(last: &mut Option<PathBuf>, cwd: &Path) -> bool {
    if last.as_deref() == Some(cwd) {
        return false;
    }
    *last = Some(cwd.to_path_buf());
    true
}

/// Worktree header text, including the unknown/unavailable indicator.
pub fn worktree_indicator(metadata: Option<&WorktreeMetadata>, loading: bool) -> String {
    match metadata {
        Some(metadata) => metadata.header_label(),
        None if loading => "reading git metadata".to_owned(),
        None => "unknown git metadata".to_owned(),
    }
}

/// The platform family is surfaced for diagnostics and smoke evidence without
/// any per-OS UI branch.
pub fn platform_family_label() -> &'static str {
    PlatformFamily::current().name()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_root_availability_transitions_once_and_recovers() {
        let mut state = None;
        let path = Path::new("/tmp/made-gone");
        assert_eq!(
            apply_shell_root_availability(&mut state, path, &Ok(())),
            ShellRootTransition::Unchanged
        );
        assert!(state.is_none());

        assert_eq!(
            apply_shell_root_availability(&mut state, path, &Err("removed".to_owned())),
            ShellRootTransition::BecameUnavailable("removed".to_owned())
        );
        assert_eq!(
            apply_shell_root_availability(&mut state, path, &Err("removed again".to_owned())),
            ShellRootTransition::Unchanged,
            "repeated failures at the same path must not log again"
        );
        assert_eq!(
            state.as_ref().map(|state| state.detail.as_str()),
            Some("removed")
        );

        assert_eq!(
            apply_shell_root_availability(&mut state, path, &Ok(())),
            ShellRootTransition::Recovered
        );
        assert!(state.is_none());
        assert_eq!(
            apply_shell_root_availability(&mut state, path, &Ok(())),
            ShellRootTransition::Unchanged
        );
    }

    #[test]
    fn background_shell_root_check_detects_delete_and_recovery_and_stops() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();

        let mut worker = ShellRootWorker::start();
        assert!(worker.request(root.clone(), RootGeneration::INITIAL));
        let available = wait_for_shell_root_result(&worker);
        assert!(available.result.is_ok(), "{:?}", available.result);

        let renamed = temp.path().join("renamed");
        std::fs::rename(&root, &renamed).unwrap();
        assert!(worker.request(root.clone(), RootGeneration::INITIAL));
        let renamed_away = wait_for_shell_root_result(&worker);
        assert!(renamed_away.result.is_err(), "rename removes the old path");
        std::fs::rename(&renamed, &root).unwrap();
        assert!(worker.request(root.clone(), RootGeneration::INITIAL));
        let back = wait_for_shell_root_result(&worker);
        assert!(back.result.is_ok(), "{:?}", back.result);

        std::fs::remove_dir_all(&root).unwrap();
        assert!(worker.request(root.clone(), RootGeneration::INITIAL));
        let unavailable = wait_for_shell_root_result(&worker);
        assert!(unavailable.result.is_err());

        std::fs::create_dir(&root).unwrap();
        assert!(worker.request(root.clone(), RootGeneration::INITIAL));
        let recovered = wait_for_shell_root_result(&worker);
        assert!(recovered.result.is_ok(), "{:?}", recovered.result);

        worker.shutdown();
        assert!(!worker.request(root, RootGeneration::INITIAL));
    }

    fn wait_for_shell_root_result(worker: &ShellRootWorker) -> ShellRootCheckResult {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(result) = worker.try_recv() {
                return result;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("background shell-root check did not complete");
    }

    #[test]
    fn pending_worktree_request_keeps_only_the_latest_generation_and_retries() {
        let mut pending = PendingWorktreeRequest::default();
        pending.offer(PathBuf::from("/tmp/a"), RootGeneration::new(1));
        pending.offer(PathBuf::from("/tmp/b"), RootGeneration::new(2));
        assert_eq!(
            pending.peek(),
            Some(&(PathBuf::from("/tmp/b"), RootGeneration::new(2)))
        );
        let mut attempts = 0;
        assert!(!pending.try_send(|_, _| {
            attempts += 1;
            false
        }));
        assert_eq!(attempts, 1);
        assert!(pending.is_pending());
        assert!(pending.try_send(|root, generation| {
            assert_eq!(root, Path::new("/tmp/b"));
            assert_eq!(generation, RootGeneration::new(2));
            true
        }));
        assert!(!pending.is_pending());
        assert_eq!(pending.take(), None);
    }

    #[test]
    fn diagnostics_fail_closed_without_app_paths() {
        assert_eq!(diagnostics_log_path(None), None);
    }

    #[test]
    fn osc7_cwd_is_revalidated_only_when_it_changes() {
        let mut last = None;
        let first = Path::new("/tmp/one");
        assert!(note_terminal_cwd_once(&mut last, first));
        assert!(!note_terminal_cwd_once(&mut last, first));
        assert!(note_terminal_cwd_once(&mut last, Path::new("/tmp/two")));
    }

    #[test]
    fn worktree_unavailable_has_a_visible_indicator() {
        let unavailable = WorktreeMetadata::unavailable("git missing");
        assert_eq!(
            worktree_indicator(Some(&unavailable), false),
            "git unavailable"
        );
        let unknown = WorktreeMetadata::unknown("not a repo");
        assert_eq!(
            worktree_indicator(Some(&unknown), false),
            "unknown git metadata"
        );
        assert_eq!(worktree_indicator(None, false), "unknown git metadata");
    }

    #[test]
    fn optional_integration_starts_inactive_without_a_probe_or_deadline() {
        let integration = IntegrationState::new(Some(PathBuf::from("/tmp")), PathBuf::from("/tmp"));
        assert_eq!(integration.status(), IntegrationStatus::Inactive);
        assert!(!integration.status().is_degraded());
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
