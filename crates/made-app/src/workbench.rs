//! Native workbench runtime: launch composition, bridge events, session follow,
//! explorer, worktree metadata, diagnostics, and metrics.
//!
//! This is the composition root. It owns the privileged runtime side of the
//! workspace. Runtime queues are bounded and nonblocking. Construction still
//! performs synchronous config/PTY setup (including shell policy preflight);
//! callers must not mistake that remaining launch cost for background work.

use std::collections::HashSet;
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
use made_explorer::highlight::Highlighter;
use made_explorer::root_switch::RootGeneration;
use made_platform::{
    host_platform, AppPaths, CompatibleTui, LogIssue, LogIssueSeverity, Platform, PlatformFamily,
    PtySize,
};

use crate::diagnostics::{
    Diagnostics, DiagnosticsSnapshot, GrokDiagnosticsRow, LogEntry, LogLevel, WorkspaceDiagnostics,
    DEFAULT_LOG_FILE, MAX_LOG_FILE_BYTES,
};
use crate::explorer_view::{
    ExplorerGitChrome, ExplorerPane, GitHeader, PreviewDocument, PreviewUiAction,
};
use crate::integration_status::{IntegrationState, IntegrationStatus};
use crate::metrics::Metrics;
use crate::sanitize::{sanitize_copy_text, sanitize_display};
use crate::session_focus::{FocusErrorKind, FocusOutcome, FocusUpdate, SessionFocusController};
use crate::session_follow::{SessionFollowCoordinator, SessionFollowUpdate};
use crate::session_map::{SessionActivity, SessionMap};
use crate::terminal_view::{TerminalPane, DEFAULT_COLS, DEFAULT_ROWS};
use crate::theme::EXPLORER_DEFAULT_WIDTH_WIDE;
use crate::tui::TerminalTarget;
use crate::workspace::{fallback_terminal_cwd, LaunchPlan, WorkspaceError};
use crate::worktree::{
    GitRunner, GitStatusRead, GitStatusSnapshot, MetadataStatus, WorktreeMetadata,
};
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

/// Coalesce explorer watcher bursts before reading git status again.
const GIT_STATUS_DEBOUNCE: Duration = Duration::from_millis(200);

static NEXT_PANE: AtomicU64 = AtomicU64::new(1);

/// One Grok session open in this terminal, with the latest hook status.
#[derive(Debug, Clone)]
struct GrokLiveSession {
    id: String,
    cwd: PathBuf,
    opened_at: String,
    activity: SessionActivity,
    touched: Instant,
}

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

#[derive(Debug)]
enum RemotePhase {
    Connecting,
    NeedsConsent,
    NeedsHostApproval,
    /// Same-version companion bytes differ. Waiting for this attempt's separate
    /// update approval; nothing has been written or deleted.
    NeedsUpdate,
    Staging,
    AwaitingAttach,
    Ready,
    Failed,
}

#[derive(Clone, Copy)]
enum OpenRetry {
    Probe,
    Stage,
    Companion,
    /// The approved same-version update stage failed; Retry stays on the
    /// update card instead of becoming a generic open failure.
    Update,
}

struct PendingUpload {
    upload_id: u64,
    bytes: Vec<u8>,
    offset: u64,
    begin_id: Option<u64>,
    begin_acked: bool,
    inflight_chunk: Option<u64>,
    commit_id: Option<u64>,
}

struct RemoteTab {
    snapshot: crate::ssh_servers::ServerSnapshot,
    workspace: crate::remote_workspace::RemoteWorkspace,
    phase: RemotePhase,
    detail: String,
    prompt: Option<made_platform::ssh_askpass::PromptRequest>,
    prompt_draft: String,
    opencode: bool,
    /// Whether the remote launch target is Grok: the first frame opens
    /// `grok dashboard` through LaunchTarget::Grok.
    grok: bool,
    /// Sanitized target triple for the update card. The version is the shipped
    /// app version; no path or hash is stored here.
    update_target: Option<String>,
    /// This attempt's explicit `Update companion` approval. Never persisted.
    update_approved: bool,
    /// The approved update attempt itself failed; Retry stays actionable.
    update_failed: bool,
    /// What the tab's own launch probe detected. Absence is undetected; the
    /// host PATH is never consulted for a remote tab.
    grok_detected: bool,
    probe: Option<crate::ssh_servers::ProbeAttempt>,
    stage: Option<crate::ssh_servers::StageAttempt>,
    client_rx: Option<
        Receiver<Result<crate::ssh_servers::CompanionSession, crate::ssh_servers::AttemptError>>,
    >,
    failures: crate::ssh_servers::SshFailureSlot,
    host_fingerprint: Option<String>,
    approved_fingerprint: Option<String>,
    host_retry: Option<OpenRetry>,
    snapshot_gate: crate::remote_session::SnapshotGate,
    companion_spawn_requested: bool,
    client: Option<made_remote::client::RemoteClient>,
    /// Whether this tab is on screen, read by the client's wake callback on
    /// the transport thread. A hidden tab's output must not repaint the window.
    wake_visible: Arc<std::sync::atomic::AtomicBool>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    /// Client request id -> workspace request id. RemoteClient assigns its own ids.
    id_map: std::collections::HashMap<u64, u64>,
    cleanup_unconfirmed: bool,
    /// Set when the user approves provisioning for this launch.
    consent_granted: bool,
    consent_persist: bool,
    /// The private companion was staged/verified successfully in this launch.
    /// Only then can the consented Grok plugin installer find its helper.
    companion_provisioned: bool,
    session_open: crate::remote_session::SessionOpen,
    /// Epoch stamped on the next client. The workspace epoch stays put until Attached.
    client_epoch: u64,
    attach_request: Option<u64>,
    attached_once: bool,
    holding_input: bool,
    remote_target: Option<String>,
    upload: Option<PendingUpload>,
    next_upload: u64,
}

impl RemoteTab {
    fn connecting(
        snapshot: crate::ssh_servers::ServerSnapshot,
        directory: made_remote::protocol::RemotePath,
    ) -> Result<Self, String> {
        let identity = made_remote::protocol::Identity {
            authority: snapshot.server().id(),
            tab: made_remote::protocol::TabCapability::random()
                .map_err(|_| "could not allocate a remote tab capability".to_owned())?,
            epoch: 1,
            generation: 1,
        };
        let workspace = crate::remote_workspace::RemoteWorkspace::new(identity, directory)
            .map_err(|_| "remote workspace identity was rejected".to_owned())?;
        let mut workspace = workspace;
        workspace.disconnect();
        Ok(Self {
            snapshot,
            workspace,
            phase: RemotePhase::Connecting,
            detail: "Connecting. No local shell was started.".to_owned(),
            prompt: None,
            prompt_draft: String::new(),
            opencode: false,
            grok: false,
            update_target: None,
            update_approved: false,
            update_failed: false,
            grok_detected: false,
            probe: None,
            stage: None,
            client_rx: None,
            failures: crate::ssh_servers::SshFailureSlot::default(),
            host_fingerprint: None,
            approved_fingerprint: None,
            host_retry: None,
            snapshot_gate: crate::remote_session::SnapshotGate::default(),
            companion_spawn_requested: false,
            client: None,
            wake_visible: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            id_map: std::collections::HashMap::new(),
            cleanup_unconfirmed: false,
            consent_granted: false,
            consent_persist: false,
            companion_provisioned: false,
            session_open: crate::remote_session::SessionOpen::New,
            client_epoch: 1,
            attach_request: None,
            attached_once: false,
            holding_input: false,
            remote_target: None,
            upload: None,
            next_upload: 1,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemotePreviewKey {
    path: made_remote::protocol::RemotePath,
    freshness: crate::remote_workspace::Freshness,
    len: usize,
    revision: u64,
}

struct RemotePreviewJob {
    key: RemotePreviewKey,
    path: made_remote::protocol::RemotePath,
    kind: made_remote::protocol::PreviewKind,
    data: Vec<u8>,
}

struct RemotePreviewResult {
    key: RemotePreviewKey,
    document: PreviewDocument,
}

struct RemotePreviewWorker {
    jobs: SyncSender<RemotePreviewJob>,
    results: Receiver<RemotePreviewResult>,
}

impl RemotePreviewWorker {
    fn start() -> Self {
        let (jobs, job_rx) = mpsc::sync_channel::<RemotePreviewJob>(2);
        let (result_tx, results) = mpsc::sync_channel::<RemotePreviewResult>(8);
        thread::Builder::new()
            .name("made-remote-preview".to_owned())
            .spawn(move || {
                let mut highlighter: Option<Highlighter> = None;
                while let Ok(job) = job_rx.recv() {
                    let highlighter = highlighter.get_or_insert_with(Highlighter::new);
                    let document =
                        PreviewDocument::from_remote(&job.path, job.kind, &job.data, highlighter);
                    if result_tx
                        .send(RemotePreviewResult {
                            key: job.key,
                            document,
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

    fn request(&self, job: RemotePreviewJob) -> bool {
        self.jobs.try_send(job).is_ok()
    }

    fn try_recv(&self) -> Option<RemotePreviewResult> {
        self.results.try_recv().ok()
    }
}

#[derive(Default)]
struct RemotePreviewView {
    key: Option<RemotePreviewKey>,
    document: Option<PreviewDocument>,
    document_key: Option<RemotePreviewKey>,
    refreshed_at: Option<Instant>,
    worker: Option<RemotePreviewWorker>,
    pending: Option<RemotePreviewKey>,
}

fn remote_preview_loading_and_error(
    preview: &crate::remote_workspace::PreviewProjection,
    has_document: bool,
    pending: bool,
) -> (bool, Option<String>) {
    use crate::remote_workspace::Freshness;
    let error = match preview.freshness {
        Freshness::Unavailable(code) => Some(format!(
            "Preview unavailable: {}. Refresh to try again.",
            remote_error_reason(code)
        )),
        Freshness::Stale if !has_document => {
            Some("Preview is stale. Refresh to try again.".to_owned())
        }
        _ => None,
    };
    let loading = error.is_none() && (preview.freshness == Freshness::Loading || pending);
    (loading, error)
}

fn remote_error_reason(code: made_remote::protocol::ErrorCode) -> &'static str {
    use made_remote::protocol::ErrorCode;
    match code {
        ErrorCode::Unsupported => "unsupported request",
        ErrorCode::Unavailable => "the remote path is unavailable",
        ErrorCode::Unauthorized => "the remote path is not authorized",
        ErrorCode::InvalidRequest => "the remote request was rejected",
        ErrorCode::Stale => "the remote state is stale",
        ErrorCode::Cancelled => "the remote request was cancelled",
        ErrorCode::Timeout => "the remote request timed out",
        ErrorCode::Limit => "the remote limit was reached",
        ErrorCode::SessionLost => "the remote session was lost",
        ErrorCode::CleanupUnconfirmed => "remote cleanup was not confirmed",
        ErrorCode::Io => "the remote I/O failed",
    }
}

impl RemotePreviewView {
    fn clear(&mut self) {
        self.key = None;
        self.document = None;
        self.document_key = None;
        self.refreshed_at = None;
        self.pending = None;
    }

    fn request(
        &mut self,
        key: RemotePreviewKey,
        path: made_remote::protocol::RemotePath,
        kind: made_remote::protocol::PreviewKind,
        data: Vec<u8>,
    ) -> bool {
        if self.pending.as_ref() == Some(&key) {
            return true;
        }
        let worker = self.worker.get_or_insert_with(RemotePreviewWorker::start);
        if worker.request(RemotePreviewJob {
            key: key.clone(),
            path,
            kind,
            data,
        }) {
            self.pending = Some(key);
            true
        } else {
            false
        }
    }

    fn poll(&mut self) {
        let mut results = Vec::new();
        if let Some(worker) = self.worker.as_ref() {
            while let Some(result) = worker.try_recv() {
                results.push(result);
            }
        }
        for result in results {
            if self.key.as_ref() == Some(&result.key) {
                self.document = Some(result.document);
                self.document_key = Some(result.key);
                self.refreshed_at = Some(Instant::now());
                self.pending = None;
            }
        }
    }
}

/// One workspace owner; siblings share no process, bridge or load receiver.
pub struct Workbench {
    pub(crate) platform: made_platform::HostPlatform,
    pub(crate) plan: Option<LaunchPlan>,
    pub(crate) workspace_error: Option<WorkspaceError>,
    pub(crate) terminal_cwd: PathBuf,
    pub(crate) terminal_target: TerminalTarget,
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
    git_status_worker: GitStatusWorker,
    pending_git_status: PendingGitStatusRequest,
    git_status: GitStatusRuntime,
    shell_capability: Option<made_platform::ShellCwdCapability>,
    shell_root_worker: ShellRootWorker,
    shell_root_unavailable: Option<ShellRootUnavailable>,
    shell_root_check_in_flight: bool,
    shell_root_checked_at: Instant,
    shell_root_check_interval: Duration,
    grok_checked_at: Instant,
    grok_log_offset: u64,
    grok_sessions: Vec<GrokLiveSession>,
    /// Session ids removed by a session-end hook. Inventory must not put the
    /// dot back while Grok's file still lists that row.
    grok_closed: HashSet<String>,
    /// Hook session moved before the inventory could prove it belongs here.
    grok_provisional_id: Option<String>,
    /// Explorer root after that provisional move. A later release restores the
    /// saved directory only while the explorer is still here.
    grok_provisional_cwd: Option<PathBuf>,
    /// Grok directory to restore if that provisional session is not ours.
    /// `None` while a provisional move is active means the shell owned the root.
    grok_restore_root: Option<PathBuf>,
    grok_file_newest_id: Option<String>,
    instrumentation: RuntimeInstrumentation,
    pane_id: String,
    visible: bool,
    left_surface: LeftSurface,
    explorer_visible: bool,
    explorer_width: f32,
    sessions_visible: bool,
    /// Whether the About card is revealing its diagnostics detail.
    pub(crate) about_diagnostics_visible: bool,
    /// Increments when About opens so its reveal animation starts hidden.
    pub(crate) about_open_generation: u64,
    /// Grok Build pill and sentence attached to the diagnostics snapshot.
    grok_diagnostics: GrokDiagnosticsRow,
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
    remote: Option<RemoteTab>,
    remote_preview: RemotePreviewView,
    remote_explorer_cache: crate::explorer_view::RemoteExplorerCache,
    pub(crate) remote_preview_focus_pending: bool,
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
        workspace_error: Option<WorkspaceError>,
        terminal_cwd: PathBuf,
        terminal_target: TerminalTarget,
        started_at: Instant,
    ) -> Self {
        Self::start_with(
            plan,
            workspace_error,
            terminal_cwd,
            terminal_target,
            started_at,
            None,
        )
    }

    /// Open a remote tab without starting a host shell, bridge, or explorer.
    ///
    /// Connection continues on a worker. Failure stays on this tab.
    pub fn start_remote(
        snapshot: crate::ssh_servers::ServerSnapshot,
        directory: made_remote::protocol::RemotePath,
        target: TerminalTarget,
        started_at: Instant,
    ) -> Result<Self, String> {
        let mut remote = RemoteTab::connecting(snapshot.clone(), directory)?;
        remote.opencode = matches!(target, TerminalTarget::RemoteOpenCode { .. });
        remote.grok = matches!(target, TerminalTarget::RemoteGrok { .. });
        remote.grok_detected = false;
        remote.probe = Some(crate::ssh_servers::spawn_probe(
            crate::ssh_servers::ProbeRequest {
                ticket: crate::ssh_servers::ProbeTicket {
                    authority: snapshot.server().id(),
                    attempt: 1,
                },
                server: snapshot,
            },
        ));
        Ok(Self::start_with(
            None,
            None,
            PathBuf::new(),
            target,
            started_at,
            Some(remote),
        ))
    }

    /// Insert an already-built remote model. Used by UI tests and by a caller
    /// that has a projection but no live SSH process. It does not claim a
    /// successful connection.
    pub fn bind_remote_model(
        snapshot: crate::ssh_servers::ServerSnapshot,
        workspace: crate::remote_workspace::RemoteWorkspace,
        target: TerminalTarget,
        detail: impl Into<String>,
    ) -> Self {
        let remote = RemoteTab {
            snapshot,
            workspace,
            phase: RemotePhase::Ready,
            detail: detail.into(),
            prompt: None,
            prompt_draft: String::new(),
            opencode: matches!(target, TerminalTarget::RemoteOpenCode { .. }),
            grok: matches!(target, TerminalTarget::RemoteGrok { .. }),
            update_target: None,
            update_approved: false,
            update_failed: false,
            grok_detected: false,
            probe: None,
            stage: None,
            client_rx: None,
            failures: crate::ssh_servers::SshFailureSlot::default(),
            host_fingerprint: None,
            approved_fingerprint: None,
            host_retry: None,
            snapshot_gate: crate::remote_session::SnapshotGate::default(),
            companion_spawn_requested: false,
            client: None,
            wake_visible: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            id_map: std::collections::HashMap::new(),
            cleanup_unconfirmed: false,
            consent_granted: false,
            consent_persist: false,
            companion_provisioned: false,
            session_open: crate::remote_session::SessionOpen::New,
            client_epoch: 1,
            attach_request: None,
            attached_once: false,
            holding_input: false,
            remote_target: None,
            upload: None,
            next_upload: 1,
        };
        Self::start_with(
            None,
            None,
            PathBuf::new(),
            target,
            Instant::now(),
            Some(remote),
        )
    }

    fn start_with(
        plan: Option<LaunchPlan>,
        mut workspace_error: Option<WorkspaceError>,
        terminal_cwd: PathBuf,
        terminal_target: TerminalTarget,
        started_at: Instant,
        remote: Option<RemoteTab>,
    ) -> Self {
        let platform = host_platform();
        let mut app_paths = None;
        let mut path_error = None;
        match platform.app_paths() {
            Ok(paths) => {
                if let Err(error) = paths.ensure_directories() {
                    path_error = Some(format!("app directories: {error}"));
                }
                // Opportunistic retention: a new tab or app launch bounds the
                // clipboard image directory even before the next image paste.
                let _ = made_platform::prune_clipboard_images(&paths.temp_dir);
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
            terminal_target,
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
            git_status_worker: GitStatusWorker::start(),
            pending_git_status: PendingGitStatusRequest::default(),
            git_status: GitStatusRuntime::default(),
            shell_capability: None,
            shell_root_worker: ShellRootWorker::start(),
            shell_root_unavailable: None,
            shell_root_check_in_flight: false,
            shell_root_checked_at: Instant::now(),
            grok_checked_at: Instant::now(),
            grok_log_offset: u64::MAX,
            grok_sessions: Vec::new(),
            grok_closed: HashSet::new(),
            grok_provisional_id: None,
            grok_provisional_cwd: None,
            grok_restore_root: None,
            grok_file_newest_id: None,
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
            about_diagnostics_visible: false,
            about_open_generation: 0,
            grok_diagnostics: GrokDiagnosticsRow::NOT_ON_PATH,
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
            remote,
            remote_preview: RemotePreviewView::default(),
            remote_explorer_cache: crate::explorer_view::RemoteExplorerCache::default(),
            remote_preview_focus_pending: false,
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
        if workbench.remote.is_some() {
            workbench.follow = None;
            workbench.explorer = None;
            workbench.plan = None;
            workbench.diag.log_mut().record(
                LogLevel::Info,
                "remote workspace opened without a host shell",
            );
            return workbench;
        }
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
        // Grok is started as-is. OpenCode and the shell keep the composed
        // plugin environment; Grok must not receive it or a written config.
        let composed_content = match self.terminal_target {
            TerminalTarget::Tui(CompatibleTui::Grok) => None,
            TerminalTarget::Shell | TerminalTarget::Tui(CompatibleTui::OpenCode) => {
                composed_content
            }
            // Remote tabs already return before this function. If it is reached,
            // do not start a host PTY or inject the local OpenCode plugin.
            TerminalTarget::RemoteShell { .. }
            | TerminalTarget::RemoteOpenCode { .. }
            | TerminalTarget::RemoteGrok { .. } => {
                return;
            }
        };
        // One PATH snapshot serves the primary spawn and the fallback retry, so
        // a TUI resolves to the same executable for both.
        let path = std::env::var_os("PATH").unwrap_or_default();
        let cwd = self.terminal_cwd.clone();
        let Some(options) = self.terminal_target.pty_options(cwd, &path) else {
            let detail = format!(
                "{} is not available on PATH",
                self.terminal_target.program_label()
            );
            self.diag.log_mut().record(LogLevel::Error, detail.clone());
            self.terminal_error = Some(detail);
            return;
        };
        // A parent script (CI bash -e, for example) exports SHELLOPTS=errexit.
        // An interactive terminal must not inherit that, or the first failed
        // prompt hook exits the shell while the workspace is still open.
        let mut options = options
            .with_size(PtySize {
                rows: DEFAULT_ROWS as u16,
                cols: DEFAULT_COLS as u16,
            })
            .with_channel_capacity(256)
            .without_env("SHELLOPTS")
            .without_env("BASH_ENV")
            .without_env("ENV");

        if let Some(content) = &composed_content {
            options = options.with_env(CONFIG_CONTENT_ENV, content.clone());
        }

        match TerminalPane::spawn(&self.platform, options.clone()) {
            Ok(pane) => {
                let pane = pane.with_target(self.terminal_target);
                if let Some(pid) = pane.pid() {
                    self.diag.log_mut().record(
                        LogLevel::Info,
                        format!(
                            "terminal {} started (pid {pid})",
                            self.terminal_target.log_label()
                        ),
                    );
                }
                self.terminal = Some(pane);
                self.terminal_error = None;
                if composed_content.is_none() {
                    self.diag.log_mut().record(
                        LogLevel::Warn,
                        format!(
                            "terminal {} started with the original environment (no Made configuration)",
                            self.terminal_target.log_label()
                        ),
                    );
                }
            }
            Err(first_error) => {
                // The workspace may have disappeared between validation and
                // spawn. Retry once in the documented safe fallback directory.
                let fallback = fallback_terminal_cwd();
                if fallback != self.terminal_cwd {
                    if let Some(retry) = self.terminal_target.pty_options(fallback.clone(), &path) {
                        let mut retry = retry
                            .with_size(options.size)
                            .with_channel_capacity(256)
                            .without_env("SHELLOPTS")
                            .without_env("BASH_ENV")
                            .without_env("ENV");
                        if let Some(content) = &composed_content {
                            retry = retry.with_env(CONFIG_CONTENT_ENV, content.clone());
                        }
                        if let Ok(pane) = TerminalPane::spawn(&self.platform, retry) {
                            let pane = pane.with_target(self.terminal_target);
                            self.diag.log_mut().record(
                                LogLevel::Warn,
                                format!(
                                    "workspace {} failed; terminal started in the fallback directory",
                                    self.terminal_target.log_label()
                                ),
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
                }
                let detail = format!(
                    "terminal {} could not start: {first_error}",
                    self.terminal_target.log_label()
                );
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

    /// Record that OpenCode is not installed or not on PATH.
    pub fn note_opencode_missing(&mut self) {
        self.integration.note_opencode_missing();
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

    /// Keep the Grok Build diagnostics row for the next snapshot.
    pub(crate) fn set_grok_diagnostics(&mut self, row: GrokDiagnosticsRow) {
        // Consent authorizes Made's installer, not hooks a user installed.
        // Keep transport, detection, install and not-running rows unchanged.
        let hooks_ready = self.remote_grok_running()
            && self.remote_grok_hooks_ready() == Some(true)
            && (row == GrokDiagnosticsRow::REMOTE_HOOKS_NOT_APPROVED
                || row == GrokDiagnosticsRow::REMOTE_INSTALL_PENDING);
        self.grok_diagnostics = if hooks_ready {
            GrokDiagnosticsRow::READY
        } else {
            row
        };
    }

    /// Grok Build pill and sentence currently shown in About.
    pub(crate) fn grok_diagnostics(&self) -> GrokDiagnosticsRow {
        self.grok_diagnostics.clone()
    }

    /// Record that Grok session follow has no status hooks.
    ///
    /// Called when the shipped plugin could not be installed. Repeated calls
    /// keep a single warning.
    pub(crate) fn note_grok_session_hooks_unavailable(&mut self) {
        const MESSAGE: &str = "Grok session follow: session-status hooks are unavailable";
        if self.diag.log().entries().any(|entry| {
            entry
                .message
                .contains("session-status hooks are unavailable")
        }) {
            return;
        }
        self.diag.log_mut().record(LogLevel::Warn, MESSAGE);
    }

    /// Terminal pane, when the shell started.
    pub fn terminal(&self) -> Option<&TerminalPane> {
        self.terminal.as_ref()
    }

    /// Mutable terminal for explicit input and lifecycle actions.
    pub fn terminal_mut(&mut self) -> Option<&mut TerminalPane> {
        self.terminal.as_mut()
    }
    /// Which child process this workspace's terminal starts.
    pub fn terminal_target(&self) -> TerminalTarget {
        self.terminal_target
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
    /// Whether the effective tree is currently owned by OpenCode.
    pub fn root_source(&self) -> &'static str {
        self.follow
            .as_ref()
            .map_or("shell", |follow| follow.effective_root_source().as_str())
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
        if self.remote.is_some() {
            let update = self
                .remote
                .as_mut()
                .and_then(|remote| remote.workspace.focus_session(session_id).ok());
            if update.is_none() {
                self.set_notice("That session is not in the remote map.");
            }
            return;
        }
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
    pub fn is_remote(&self) -> bool {
        self.remote.is_some()
    }

    pub fn remote_label(&self) -> Option<String> {
        self.remote
            .as_ref()
            .map(|remote| remote.snapshot.server().metadata().display_name.clone())
    }

    pub fn remote_path_label(&self) -> Option<String> {
        self.remote
            .as_ref()
            .and_then(|remote| remote.workspace.root().to_utf8().ok().map(str::to_owned))
    }

    pub fn remote_detail(&self) -> Option<&str> {
        self.remote.as_ref().map(|remote| remote.detail.as_str())
    }

    pub fn remote_authority(&self) -> Option<made_remote::protocol::AuthorityId> {
        self.remote
            .as_ref()
            .map(|remote| remote.snapshot.server().id())
    }

    pub fn remote_needs_consent(&self) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|remote| matches!(remote.phase, RemotePhase::NeedsConsent))
    }

    /// A same-version companion mismatch is waiting for this attempt's own
    /// approval. Initial staging consent never authorizes changed bytes.
    pub fn remote_needs_update(&self) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|remote| matches!(remote.phase, RemotePhase::NeedsUpdate))
    }

    pub fn remote_update_failed(&self) -> bool {
        self.remote.as_ref().is_some_and(|remote| {
            remote.update_failed && matches!(remote.phase, RemotePhase::NeedsUpdate)
        })
    }

    /// Sanitized target triple for the update card.
    pub fn remote_update_target(&self) -> Option<&str> {
        self.remote
            .as_ref()
            .and_then(|remote| remote.update_target.as_deref())
    }

    /// Whether approval of this attempt's update has started the fixed stage.
    pub fn remote_update_approved(&self) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|remote| remote.update_approved)
    }

    pub fn remote_update_in_progress(&self) -> bool {
        self.remote.as_ref().is_some_and(|remote| {
            remote.update_approved && matches!(remote.phase, RemotePhase::Staging)
        })
    }

    /// Show a sanitized open failure on this tab. Does not start SSH.
    pub fn present_open_failure(
        &mut self,
        class: crate::remote_session::SshFailureClass,
        detail: impl Into<String>,
        fingerprint: Option<&str>,
    ) -> bool {
        if self.remote.is_none() {
            return false;
        }
        let shown = crate::remote_session::shown_failure(class, detail, fingerprint);
        self.apply_attempt_error(
            crate::ssh_servers::AttemptError {
                class: shown.class,
                detail: shown.detail,
                fingerprint: shown.fingerprint,
            },
            OpenRetry::Probe,
        );
        true
    }

    pub fn remote_needs_host_approval(&self) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|remote| matches!(remote.phase, RemotePhase::NeedsHostApproval))
    }

    pub fn remote_host_fingerprint(&self) -> Option<&str> {
        self.remote
            .as_ref()
            .and_then(|remote| remote.host_fingerprint.as_deref())
    }

    pub fn approve_unknown_host(&mut self) {
        let retry = {
            let Some(remote) = self.remote.as_mut() else {
                return;
            };
            if !matches!(remote.phase, RemotePhase::NeedsHostApproval) {
                return;
            }
            remote.approved_fingerprint = remote.host_fingerprint.clone();
            remote.host_retry.take()
        };
        match retry {
            Some(OpenRetry::Probe) => self.retry_remote_probe(),
            Some(OpenRetry::Stage) => self.start_remote_stage(),
            Some(OpenRetry::Update) => {
                if let Some(remote) = self.remote.as_mut() {
                    remote.update_approved = true;
                    remote.update_failed = false;
                }
                self.start_remote_stage_with(true);
            }
            Some(OpenRetry::Companion) => self.spawn_remote_companion(),
            None => {}
        }
    }

    pub fn deny_unknown_host(&mut self) {
        self.cancel_remote_attempt("the remote host key is not in the app trust file");
    }

    pub fn remote_can_reconnect(&self) -> bool {
        self.remote.as_ref().is_some_and(|remote| {
            remote.attached_once && matches!(remote.phase, RemotePhase::Failed)
        })
    }

    /// Approve only this attempt's same-version companion update. This is
    /// separate from initial staging consent and is never persisted. Approval
    /// starts the fixed update-mode stage; the companion still waits for a
    /// successful stage result.
    pub fn approve_remote_update(&mut self) -> bool {
        let approved = self
            .remote
            .as_ref()
            .is_some_and(|remote| matches!(remote.phase, RemotePhase::NeedsUpdate));
        if !approved {
            return false;
        }
        if let Some(remote) = self.remote.as_mut() {
            remote.update_approved = true;
            remote.update_failed = false;
            remote.prompt = None;
            remote.prompt_draft.clear();
        }
        self.start_remote_stage_with(true);
        true
    }

    /// Retry a failed approved update. Refusal and cancellation never enter
    /// update mode.
    pub fn retry_remote_update(&mut self) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|remote| remote.update_failed)
            && self.approve_remote_update()
    }

    /// Refuse or cancel the update. No update-mode stage is started, so no
    /// remote file is written or deleted by this action.
    pub fn refuse_remote_update(&mut self) {
        let pending = self
            .remote
            .as_ref()
            .is_some_and(|remote| matches!(remote.phase, RemotePhase::NeedsUpdate));
        if !pending {
            return;
        }
        if let Some(remote) = self.remote.as_mut() {
            remote.update_approved = false;
            remote.update_failed = false;
        }
        self.cancel_remote_attempt(
            "The companion update was not approved. Nothing was written or deleted, and no local shell was started.",
        );
    }

    /// Show an authentication card on an existing remote tab. Does not start SSH.
    pub fn present_remote_prompt(&mut self, text: impl Into<String>) -> bool {
        let Some(remote) = self.remote.as_mut() else {
            return false;
        };
        remote.prompt = Some(made_platform::ssh_askpass::PromptRequest {
            id: 1,
            text: text.into(),
            expires_at: Instant::now() + Duration::from_secs(60),
        });
        true
    }

    pub fn remote_auth(&self) -> Option<crate::remote_session::SshPromptClass> {
        self.remote.as_ref().and_then(|remote| {
            remote
                .prompt
                .as_ref()
                .map(|prompt| crate::remote_session::classify_prompt(&prompt.text))
        })
    }

    pub fn remote_prompt_id(&self) -> Option<u64> {
        self.remote
            .as_ref()
            .and_then(|remote| remote.prompt.as_ref().map(|prompt| prompt.id))
    }

    pub fn remote_prompt_draft(&mut self) -> Option<&mut String> {
        self.remote
            .as_mut()
            .and_then(|remote| remote.prompt.is_some().then_some(&mut remote.prompt_draft))
    }

    /// Persist consent outside this method. This only continues the launch.
    pub fn grant_remote_consent(&mut self) {
        let allowed = self
            .remote
            .as_ref()
            .is_some_and(|remote| matches!(remote.phase, RemotePhase::NeedsConsent));
        if !allowed {
            return;
        }
        if let Some(remote) = self.remote.as_mut() {
            remote.consent_granted = true;
            remote.consent_persist = true;
            remote.prompt = None;
        }
        self.start_remote_stage();
    }

    pub fn take_consent_persist(&mut self) -> Option<made_remote::protocol::AuthorityId> {
        let remote = self.remote.as_mut()?;
        if !remote.consent_persist {
            return None;
        }
        remote.consent_persist = false;
        Some(remote.snapshot.server().id())
    }

    pub fn deny_remote_consent(&mut self) {
        self.cancel_remote_attempt(
            "Provisioning was not approved. No remote files were written and no local shell was started.",
        );
    }

    pub fn submit_remote_auth(&mut self, accept: bool) {
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        let Some(prompt) = remote.prompt.take() else {
            return;
        };
        let class = crate::remote_session::classify_prompt(&prompt.text);
        let draft = std::mem::take(&mut remote.prompt_draft);
        match class {
            crate::remote_session::SshPromptClass::ChangedHostKey { .. } => {
                self.cancel_remote_attempt("the remote host key changed and was refused");
            }
            crate::remote_session::SshPromptClass::UnknownHost { .. } => {
                if accept {
                    let answer =
                        made_platform::ssh_askpass::PromptAnswer::new(b"yes".to_vec()).ok();
                    self.deliver_remote_answer(prompt.id, answer);
                } else {
                    self.cancel_remote_attempt(
                        crate::remote_session::SshFailureClass::Cancelled.detail(),
                    );
                }
            }
            crate::remote_session::SshPromptClass::Secret { .. } => {
                if accept {
                    if crate::credential_cache::is_password_prompt(&prompt.text) {
                        if let Some(authority) = self.remote_authority() {
                            crate::credential_cache::remember(authority, &draft);
                        }
                    }
                    match made_platform::ssh_askpass::PromptAnswer::new(draft.into_bytes()) {
                        Ok(answer) => self.deliver_remote_answer(prompt.id, Some(answer)),
                        Err(_) => {
                            if let Some(remote) = self.remote.as_mut() {
                                remote.prompt = Some(prompt);
                                remote.detail =
                                    "That answer was rejected locally. Nothing was sent."
                                        .to_owned();
                            }
                        }
                    }
                } else {
                    self.deliver_remote_answer(prompt.id, None);
                    self.cancel_remote_attempt(
                        crate::remote_session::SshFailureClass::Cancelled.detail(),
                    );
                }
            }
        }
    }

    pub fn reconnect_remote(&mut self) -> bool {
        let Some(remote) = self.remote.as_mut() else {
            return false;
        };
        if !remote.attached_once || !matches!(remote.phase, RemotePhase::Failed) {
            return false;
        }
        remote.session_open = crate::remote_session::SessionOpen::Reattach;
        remote.client_epoch = remote.workspace.identity().epoch.saturating_add(1);
        if remote.client_epoch == remote.workspace.identity().epoch {
            return false;
        }
        remote.phase = RemotePhase::Connecting;
        remote.detail =
            "Reconnecting. The previous session is not replaced until attach succeeds.".to_owned();
        remote.prompt = None;
        remote.prompt_draft.clear();
        remote.attach_request = None;
        remote.workspace.disconnect();
        let snapshot = remote.snapshot.clone();
        remote.probe = Some(crate::ssh_servers::spawn_probe(
            crate::ssh_servers::ProbeRequest {
                ticket: crate::ssh_servers::ProbeTicket {
                    authority: snapshot.server().id(),
                    attempt: remote.client_epoch,
                },
                server: snapshot,
            },
        ));
        true
    }

    /// Hand the live client to a worker that sends Close and waits at most 2s.
    /// The UI thread does not wait. Call this before [`Self::shutdown`].
    pub fn take_remote_close(&mut self) -> Option<Receiver<crate::remote_session::CloseReport>> {
        let remote = self.remote.as_mut()?;
        remote.cancel.store(true, Ordering::Release);
        if let Some(probe) = remote.probe.as_ref() {
            probe.cancel();
        }
        if let Some(stage) = remote.stage.as_ref() {
            stage.cancel();
        }
        let (tx, rx) = mpsc::channel();
        if let Some(client) = remote.client.take() {
            let _ = thread::Builder::new()
                .name("made-ssh-close".to_owned())
                .spawn(move || {
                    let report = crate::remote_session::await_close(client);
                    let _ = tx.send(report);
                });
            return Some(rx);
        }
        if remote.attached_once
            && remote.workspace.close_state() != crate::remote_workspace::CloseState::Confirmed
        {
            let _ = tx.send(crate::remote_session::CloseReport::Unconfirmed);
            return Some(rx);
        }
        None
    }

    fn deliver_remote_answer(
        &mut self,
        id: u64,
        answer: Option<made_platform::ssh_askpass::PromptAnswer>,
    ) {
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        if let Some(probe) = remote.probe.as_ref() {
            let _ = probe.answer(id, answer);
            return;
        }
        if let Some(stage) = remote.stage.as_ref() {
            let _ = stage.answer(id, answer);
            return;
        }
        if let Some(client) = remote.client.as_ref() {
            let _ = client.answer_prompt(id, answer);
        }
    }

    fn cancel_remote_attempt(&mut self, detail: &str) {
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        remote.cancel.store(true, Ordering::Release);
        if let Some(probe) = remote.probe.as_ref() {
            probe.cancel();
        }
        if let Some(stage) = remote.stage.as_ref() {
            stage.cancel();
        }
        if let Some(client) = remote.client.take() {
            client.disconnect();
        }
        remote.probe = None;
        remote.stage = None;
        remote.client_rx = None;
        remote.prompt = None;
        remote.prompt_draft.clear();
        remote.phase = RemotePhase::Failed;
        remote.detail = detail.to_owned();
        remote.update_approved = false;
        remote.update_failed = false;
        remote.update_target = None;
    }

    pub fn remote_connection(&self) -> Option<crate::remote_workspace::ConnectionState> {
        self.remote
            .as_ref()
            .map(|remote| remote.workspace.connection())
    }

    pub fn remote_integration(&self) -> Option<crate::remote_workspace::IntegrationState> {
        self.remote
            .as_ref()
            .map(|remote| remote.workspace.integration())
    }

    /// Whether this remote tab presents OpenCode: the owning remote
    /// integration is connected through the attached session, or a directly
    /// launched remote OpenCode process is still alive before its first
    /// plugin handshake. A reported remote integration exit ends the direct
    /// launch's branding. Never reads host integration or process state.
    pub(crate) fn remote_opencode_active(&self) -> bool {
        let Some(remote) = self.remote.as_ref() else {
            return false;
        };
        if remote.workspace.connection() != crate::remote_workspace::ConnectionState::Connected {
            return false;
        }
        let integration = remote.workspace.integration();
        integration == crate::remote_workspace::IntegrationState::Connected
            || (remote.opencode
                && integration != crate::remote_workspace::IntegrationState::Unavailable)
    }

    /// Whether the tab's remote Grok state currently justifies Grok Build
    /// branding: the tab's managed child is a living grok process, or at
    /// least one open Grok session is owned by this tab's process ancestry.
    /// OpenCode branding is decided by the caller; this only reports the
    /// remote Grok projection. Never reads host process state.
    pub(crate) fn remote_grok_running(&self) -> bool {
        self.remote_projection()
            .and_then(|projection| projection.grok.as_ref())
            .is_some_and(|snapshot| snapshot.running || !snapshot.sessions.is_empty())
    }

    /// The companion's explicit hook tier, independent of session activity.
    fn remote_grok_hooks_ready(&self) -> Option<bool> {
        self.remote
            .as_ref()
            .and_then(|remote| remote.workspace.grok_hooks_ready())
    }

    /// The owning tab's own launch-probe Grok detection. The host PATH is
    /// never consulted; a tab without a finished probe reports undetected.
    pub(crate) fn remote_grok_detected(&self) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|remote| remote.grok_detected)
    }

    /// Record the tab's own remote detection after its launch probe finishes.
    /// Test-only seam: record the resolved remote target without a live probe.
    /// Production sets this from the server's own capabilities.
    pub fn hold_remote_target(&mut self, target: impl Into<String>) {
        if let Some(remote) = self.remote.as_mut() {
            remote.remote_target = Some(target.into());
        }
    }

    pub(crate) fn set_remote_grok_detection(&mut self, detected: bool) {
        if let Some(remote) = self.remote.as_mut() {
            remote.grok_detected = detected;
        }
    }

    /// Whether the tab's SSH transport is down: the attempt failed or the
    /// attached session is disconnected. Transport failure outranks every
    /// remote Grok claim in diagnostics.
    pub(crate) fn remote_transport_failed(&self) -> bool {
        self.remote.as_ref().is_some_and(|remote| {
            matches!(remote.phase, RemotePhase::Failed | RemotePhase::NeedsUpdate)
                || remote.workspace.connection()
                    == crate::remote_workspace::ConnectionState::Disconnected
        })
    }

    /// The owning tab's own bounded failure sentence when opening failed.
    /// About reports this instead of the generic transport row so a vanished
    /// grok, stage, companion or launch failure keeps its actual reason.
    pub(crate) fn remote_failure_detail(&self) -> Option<&str> {
        self.remote
            .as_ref()
            .filter(|remote| matches!(remote.phase, RemotePhase::Failed | RemotePhase::NeedsUpdate))
            .map(|remote| remote.detail.as_str())
    }

    /// Whether this tab's private companion was staged or verified in this
    /// launch. The consented Grok installer needs that helper on the server.
    pub(crate) fn remote_companion_provisioned(&self) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|remote| remote.companion_provisioned)
    }

    /// Whether the tab's SSH session is attached and current.
    pub(crate) fn remote_attached(&self) -> bool {
        self.remote.as_ref().is_some_and(|remote| {
            matches!(remote.phase, RemotePhase::Ready)
                && remote.workspace.connection()
                    == crate::remote_workspace::ConnectionState::Connected
        })
    }

    /// Remote Grok session dots in attachment order, mapped with the local
    /// idle/working/attention treatment (`error` shows as attention). Empty
    /// when no owned Grok projection is current for this tab.
    pub(crate) fn remote_grok_activities(&self) -> Vec<SessionActivity> {
        let Some(snapshot) = self
            .remote_projection()
            .and_then(|projection| projection.grok.as_ref())
            .cloned()
        else {
            return Vec::new();
        };
        let mut sessions = snapshot.sessions;
        sessions.sort_by_key(|session| session.opened_at);
        sessions
            .into_iter()
            .map(|session| match session.activity {
                made_remote::protocol::GrokActivity::Idle => SessionActivity::Idle,
                made_remote::protocol::GrokActivity::Working => SessionActivity::Busy,
                made_remote::protocol::GrokActivity::Attention
                | made_remote::protocol::GrokActivity::Error => SessionActivity::Attention,
            })
            .collect()
    }

    pub fn remote_projection(&self) -> Option<&crate::remote_workspace::WorkspaceProjection> {
        self.remote
            .as_ref()
            .map(|remote| remote.workspace.projection())
    }

    pub(crate) fn remote_explorer_ui(
        &mut self,
        ui: &mut egui::Ui,
        root: &made_remote::protocol::RemotePath,
        layout: &crate::theme::Layout,
        scope: egui::Id,
        native_backdrop: bool,
    ) -> Vec<crate::explorer_view::RemoteExplorerAction> {
        let Some(remote) = self.remote.as_ref() else {
            return Vec::new();
        };
        crate::explorer_view::remote_explorer_ui(
            ui,
            root,
            remote.workspace.projection(),
            layout,
            crate::explorer_view::RemoteExplorerOptions {
                scope,
                native_backdrop,
                allow_root_retry: remote.workspace.connection()
                    == crate::remote_workspace::ConnectionState::Connected,
            },
            &mut self.remote_explorer_cache,
        )
    }

    pub fn remote_has_preview(&self) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|remote| remote.workspace.projection().preview.is_some())
    }

    fn sync_remote_preview(&mut self) {
        use crate::remote_workspace::Freshness;
        let Some(remote) = self.remote.as_ref() else {
            self.remote_preview.clear();
            return;
        };
        let projection = remote.workspace.projection();
        let Some(preview) = projection.preview.as_ref() else {
            self.remote_preview.clear();
            return;
        };
        let key = RemotePreviewKey {
            path: preview.path.clone(),
            freshness: preview.freshness,
            len: preview.data.len(),
            revision: projection.preview_revision,
        };
        let key_changed = self.remote_preview.key.as_ref() != Some(&key);
        if key_changed {
            let same_path = self
                .remote_preview
                .key
                .as_ref()
                .is_some_and(|old| old.path == preview.path);
            if !same_path {
                self.remote_preview.document = None;
                self.remote_preview.document_key = None;
                self.remote_preview.refreshed_at = None;
            }
            self.remote_preview.key = Some(key.clone());
        }
        if preview.freshness != Freshness::Current {
            return;
        }
        let Some(kind) = preview.kind else {
            return;
        };
        if !key_changed
            && (self.remote_preview.pending.as_ref() == Some(&key)
                || self.remote_preview.document_key.as_ref() == Some(&key))
        {
            return;
        }
        let _ = self
            .remote_preview
            .request(key, preview.path.clone(), kind, preview.data.clone());
    }

    pub(crate) fn remote_preview_ui(
        &mut self,
        ui: &mut egui::Ui,
        layout: &crate::theme::Layout,
    ) -> PreviewUiAction {
        self.remote_preview.poll();
        self.sync_remote_preview();
        let Some(remote) = self.remote.as_ref() else {
            return PreviewUiAction::Close;
        };
        let Some(preview) = remote.workspace.projection().preview.as_ref() else {
            return PreviewUiAction::Close;
        };
        let full = crate::explorer_view::remote_path_text(&preview.path);
        let name = crate::explorer_view::remote_file_name(&preview.path);
        let has_document = self.remote_preview.document.is_some();
        let (loading, error) = remote_preview_loading_and_error(
            preview,
            has_document,
            self.remote_preview.pending.is_some(),
        );
        let scope = egui::Id::unique(("made-remote-preview", self.pane_id()));
        let result = crate::explorer_view::preview_card_ui(
            ui,
            crate::explorer_view::PreviewCardState {
                path_name: Some(&name),
                path_full: Some(&full),
                document: self.remote_preview.document.as_ref(),
                loading,
                error: error.as_deref(),
                refreshed_at: self.remote_preview.refreshed_at,
                focus_pending: self.remote_preview_focus_pending,
            },
            crate::explorer_view::PreviewCardIds {
                focus: scope.with("preview-focus"),
                close: scope.with(("preview-action", "Close")),
                terminal: scope.with(("preview-action", "Terminal")),
                refresh: scope.with(("preview-action", "Refresh")),
                copy: scope.with(("preview-action", "Copy path")),
            },
            layout,
        );
        self.remote_preview_focus_pending = false;
        if result.copy_path {
            ui.ctx().copy_text(sanitize_display(&full, 4096));
            self.diag
                .log_mut()
                .record(LogLevel::Info, "preview path copied");
        }
        if result.refresh {
            if let Some(remote) = self.remote.as_mut() {
                let _ = remote.workspace.refresh_preview();
            }
        }
        if matches!(
            result.surface,
            PreviewUiAction::Close | PreviewUiAction::ShowTerminal
        ) {
            ui.ctx()
                .memory_mut(|memory| memory.surrender_focus(scope.with("preview-focus")));
        }
        if result.surface == PreviewUiAction::Close {
            if let Some(remote) = self.remote.as_mut() {
                let _ = remote.workspace.close_preview();
            }
            self.remote_preview.clear();
        }
        result.surface
    }

    pub fn remote_sessions_available(&self) -> bool {
        self.remote.as_ref().is_some_and(|remote| {
            remote.workspace.integration() == crate::remote_workspace::IntegrationState::Connected
                || remote
                    .workspace
                    .projection()
                    .sessions
                    .as_ref()
                    .is_some_and(|snapshot| !snapshot.sessions.is_empty())
        })
    }

    pub fn remote_workspace(&self) -> Option<&crate::remote_workspace::RemoteWorkspace> {
        self.remote.as_ref().map(|remote| &remote.workspace)
    }

    pub fn remote_workspace_mut(
        &mut self,
    ) -> Option<&mut crate::remote_workspace::RemoteWorkspace> {
        self.remote.as_mut().map(|remote| &mut remote.workspace)
    }

    pub fn answer_remote_prompt(&mut self, text: Option<String>) {
        if let Some(text) = text {
            if let Some(remote) = self.remote.as_mut() {
                remote.prompt_draft = text;
            }
            self.submit_remote_auth(true);
        } else {
            self.submit_remote_auth(false);
        }
    }

    pub fn apply_remote_actions(
        &mut self,
        ctx: &egui::Context,
        actions: Vec<crate::explorer_view::RemoteExplorerAction>,
    ) {
        let mut open_preview = false;
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        for action in actions {
            match action {
                crate::explorer_view::RemoteExplorerAction::SetMode(mode) => {
                    remote.workspace.set_mode(mode);
                    if mode == crate::remote_workspace::ExplorerMode::Git {
                        let _ = remote.workspace.refresh_git_if_idle();
                    }
                }
                crate::explorer_view::RemoteExplorerAction::Expand(path) => {
                    let _ = remote.workspace.expand(path);
                }
                crate::explorer_view::RemoteExplorerAction::Collapse(path) => {
                    remote.workspace.collapse(&path);
                }
                crate::explorer_view::RemoteExplorerAction::Select(path) => {
                    let _ = remote.workspace.select(path);
                }
                crate::explorer_view::RemoteExplorerAction::SelectGit(path) => {
                    let _ = remote.workspace.select_git(path);
                }
                crate::explorer_view::RemoteExplorerAction::OpenPreview(path) => {
                    if remote.workspace.open_preview(path).is_ok() {
                        open_preview = true;
                    }
                }
                crate::explorer_view::RemoteExplorerAction::RetryRoot => {
                    let root = remote
                        .workspace
                        .projection()
                        .requested_root
                        .clone()
                        .unwrap_or_else(|| remote.workspace.root().clone());
                    let _ = remote.workspace.load_directory(root);
                }
            }
        }
        if open_preview {
            self.set_left_surface(LeftSurface::Preview);
            self.sync_terminal_visibility(ctx);
            self.remote_preview.clear();
            self.remote_preview_focus_pending = true;
        }
    }

    fn tick_remote(&mut self, ctx: &egui::Context) {
        if let Some(remote) = self.remote.as_ref() {
            remote.wake_visible.store(self.visible, Ordering::Relaxed);
        }
        self.poll_remote_prompt();
        self.take_observed_failure();
        let probe_done = self
            .remote
            .as_ref()
            .and_then(|remote| remote.probe.as_ref())
            .and_then(|probe| probe.poll());
        if let Some(result) = probe_done {
            self.finish_remote_probe(result);
        }
        let stage_done = self
            .remote
            .as_ref()
            .and_then(|remote| remote.stage.as_ref())
            .and_then(|stage| stage.poll());
        if let Some(result) = stage_done {
            self.finish_remote_stage(result);
        }
        if let Some(remote) = self.remote.as_mut() {
            if let Some(rx) = remote.client_rx.as_ref() {
                if let Ok(client) = rx.try_recv() {
                    remote.client_rx = None;
                    match client {
                        Ok(session) => {
                            if let Some(remote) = self.remote.as_mut() {
                                remote.failures = session.failures;
                            }
                            self.arm_remote_client(session.client);
                        }
                        Err(error) => self.apply_attempt_error(error, OpenRetry::Companion),
                    }
                }
            }
        }
        self.pump_remote_client();
        self.sync_remote_sessions();
        let terminal_changed = self.pump_remote_terminal();
        self.pump_remote_upload();
        // Remote output belongs to this tab, not the window. A hidden tab
        // must not drive an immediate repaint of the active local tab: that
        // turned every remote output frame into a full-window repaint at the
        // display refresh rate. Match the local terminal's output cadence;
        // keyboard input still requests its own immediate repaint in the
        // terminal view.
        if terminal_changed {
            if let Some(delay) = Self::remote_output_repaint_delay(self.visible) {
                ctx.request_repaint_after(delay);
            }
        }
        // The wake callback covers output; this bounded poll covers whatever it
        // cannot signal, and replaces the idle cadence the local tick ends with,
        // which a remote tab never reaches.
        if self.visible
            && self
                .remote
                .as_ref()
                .is_some_and(|remote| remote.client.is_some())
        {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if self.visible
            && self.remote.as_ref().is_some_and(|remote| {
                remote.prompt.is_some()
                    || remote.holding_input
                    || remote.upload.is_some()
                    || matches!(
                        remote.phase,
                        RemotePhase::Connecting
                            | RemotePhase::NeedsConsent
                            | RemotePhase::NeedsHostApproval
                            | RemotePhase::Staging
                            | RemotePhase::AwaitingAttach
                    )
            })
        {
            ctx.request_repaint_after(crate::metrics::ACTIVE_REPAINT_INTERVAL);
        }
    }

    /// A hidden remote tab must not repaint the active local tab. Its output
    /// is still pumped; the next visible frame shows it.
    fn remote_output_repaint_delay(visible: bool) -> Option<Duration> {
        visible.then_some(crate::metrics::ACTIVE_REPAINT_INTERVAL)
    }

    fn poll_remote_prompt(&mut self) {
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        if remote.prompt.is_some() {
            return;
        }
        let approved = remote.approved_fingerprint.clone();
        let prompt = remote
            .probe
            .as_ref()
            .and_then(|probe| probe.poll_prompt())
            .or_else(|| remote.stage.as_ref().and_then(|stage| stage.poll_prompt()))
            .or_else(|| {
                remote
                    .client
                    .as_ref()
                    .and_then(|client| client.poll_prompt())
            });
        let Some(prompt) = prompt else {
            return;
        };
        let class = crate::remote_session::classify_prompt(&prompt.text);
        if matches!(
            class,
            crate::remote_session::SshPromptClass::ChangedHostKey { .. }
        ) {
            return self.cancel_remote_attempt("the remote host key changed and was refused");
        }
        if crate::remote_session::may_answer_yes(&class, approved.as_deref(), &prompt.text) {
            let answer = made_platform::ssh_askpass::PromptAnswer::new(b"yes".to_vec()).ok();
            if let Some(probe) = remote.probe.as_ref() {
                let _ = probe.answer(prompt.id, answer);
            } else if let Some(stage) = remote.stage.as_ref() {
                let _ = stage.answer(prompt.id, answer);
            } else if let Some(client) = remote.client.as_ref() {
                let _ = client.answer_prompt(prompt.id, answer);
            }
            return;
        }
        if matches!(class, crate::remote_session::SshPromptClass::Secret { .. })
            && crate::credential_cache::is_password_prompt(&prompt.text)
        {
            if let Some(answer) = crate::credential_cache::answer(remote.snapshot.server().id()) {
                if let Some(probe) = remote.probe.as_ref() {
                    let _ = probe.answer(prompt.id, Some(answer));
                    return;
                }
                if let Some(stage) = remote.stage.as_ref() {
                    let _ = stage.answer(prompt.id, Some(answer));
                    return;
                }
                if let Some(client) = remote.client.as_ref() {
                    let _ = client.answer_prompt(prompt.id, Some(answer));
                    return;
                }
            }
        }
        remote.prompt = Some(prompt);
    }

    fn take_observed_failure(&mut self) {
        let failure = self
            .remote
            .as_ref()
            .and_then(|remote| remote.failures.take());
        let Some(failure) = failure else {
            return;
        };
        if let Some(client) = self.remote.as_mut().and_then(|remote| remote.client.take()) {
            client.disconnect();
        }
        self.apply_attempt_error(
            crate::ssh_servers::AttemptError::from_failure(&failure),
            OpenRetry::Companion,
        );
    }

    fn finish_remote_probe(
        &mut self,
        result: Result<made_remote::protocol::Capabilities, crate::ssh_servers::AttemptError>,
    ) {
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        remote.probe = None;
        remote.prompt = None;
        remote.prompt_draft.clear();
        let capabilities = match result {
            Ok(capabilities) => capabilities,
            Err(error) => return self.apply_attempt_error(error, OpenRetry::Probe),
        };
        // Presence is recorded; the field itself is not a gate.
        let _ = capabilities.tmux;
        if remote.opencode && !capabilities.opencode {
            remote.phase = RemotePhase::Failed;
            remote.detail =
                "OpenCode was not detected on this server. Nothing ran locally.".to_owned();
            remote
                .workspace
                .project_unavailable(made_remote::protocol::ErrorCode::Unavailable);
            return;
        }
        if remote.grok && !capabilities.grok {
            remote.phase = RemotePhase::Failed;
            remote.detail = "Grok was not detected on this server. Nothing ran locally.".to_owned();
            remote
                .workspace
                .project_unavailable(made_remote::protocol::ErrorCode::Unavailable);
            return;
        }
        // The owning tab's own detection feeds its Grok Build diagnostics row.
        remote.grok_detected = capabilities.grok;
        let target = match capabilities.target {
            made_remote::protocol::RemoteTarget::LinuxX64 => "x86_64-unknown-linux-gnu",
            made_remote::protocol::RemoteTarget::LinuxArm64 => "aarch64-unknown-linux-gnu",
            made_remote::protocol::RemoteTarget::MacosX64 => "x86_64-apple-darwin",
            made_remote::protocol::RemoteTarget::MacosArm64 => "aarch64-apple-darwin",
        };
        remote.remote_target = Some(target.to_owned());
        if crate::ssh_servers::companion_artifact(target).is_none() {
            remote.phase = RemotePhase::Failed;
            remote.detail = "The remote companion is not packaged with this build. Made did not start a local shell.".to_owned();
            remote
                .workspace
                .project_unavailable(made_remote::protocol::ErrorCode::Unavailable);
            return;
        }
        let allowed =
            remote.consent_granted || remote.snapshot.server().allows_provision(APP_VERSION);
        if !allowed {
            remote.phase = RemotePhase::NeedsConsent;
            remote.detail = format!(
                "Approve installing or reusing the private companion {APP_VERSION} on this server. No files are written until you approve."
            );
            return;
        }
        self.start_remote_stage();
    }

    fn start_remote_stage(&mut self) {
        self.start_remote_stage_with(false);
    }

    /// Initial staging stays on `consent-v1`; only an approved update builds
    /// the platform's `consent-v2-update` payload.
    fn start_remote_stage_with(&mut self, update: bool) {
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        // A new attempt never inherits the previous attempt's update outcome.
        // Initial/consented staging also never inherits an update approval.
        remote.update_failed = false;
        if !update {
            remote.update_approved = false;
            remote.update_target = None;
        }
        let Some(target) = remote.remote_target.clone() else {
            remote.phase = RemotePhase::Failed;
            remote.detail =
                "The remote target is unknown. Made did not start a local shell.".to_owned();
            remote.update_approved = false;
            remote.update_failed = false;
            remote.update_target = None;
            remote
                .workspace
                .project_unavailable(made_remote::protocol::ErrorCode::Unavailable);
            return;
        };
        let metadata = remote.snapshot.server().metadata().clone();
        remote.stage = Some(if update {
            crate::ssh_servers::spawn_approved_update_stage(
                metadata,
                APP_VERSION.to_owned(),
                target,
            )
        } else {
            crate::ssh_servers::spawn_consented_stage(metadata, APP_VERSION.to_owned(), target)
        });
        remote.phase = RemotePhase::Staging;
        remote.detail = if update {
            "Updating the private companion only after your approval. A running session is not signalled."
                .to_owned()
        } else {
            "Checking the private companion. Existing matching files are reused; nothing runs locally."
                .to_owned()
        };
    }

    fn finish_remote_stage(&mut self, result: Result<(), crate::ssh_servers::AttemptError>) {
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        remote.stage = None;
        remote.prompt = None;
        remote.prompt_draft.clear();
        let update = remote.update_approved;
        let Err(error) = result else {
            remote.update_approved = false;
            remote.update_failed = false;
            remote.update_target = None;
            remote.companion_provisioned = true;
            return self.spawn_remote_companion();
        };
        if error.class == crate::remote_session::SshFailureClass::UpdateRequired {
            // The platform confirmed a same-version mismatch. Nothing was
            // written or deleted; keep this attempt's own approval pending.
            remote.companion_provisioned = false;
            remote.update_approved = false;
            remote.update_failed = false;
            remote.update_target = remote.remote_target.clone();
            remote.phase = RemotePhase::NeedsUpdate;
            remote.detail = error.detail.clone();
            if !remote.attached_once {
                remote
                    .workspace
                    .project_unavailable(made_remote::protocol::ErrorCode::Unavailable);
            }
            self.diag.log_mut().record(
                LogLevel::Warn,
                format!("remote companion update required: {}", error.detail),
            );
            return;
        }
        // A failed stage must not open another SSH session or the companion.
        self.apply_attempt_error(
            error,
            if update {
                OpenRetry::Update
            } else {
                OpenRetry::Stage
            },
        );
    }

    fn apply_attempt_error(&mut self, error: crate::ssh_servers::AttemptError, retry: OpenRetry) {
        self.diag.log_mut().record(
            LogLevel::Error,
            format!("remote attempt failed: {}", error.detail),
        );
        if error.class == crate::remote_session::SshFailureClass::AuthFailed {
            if let Some(authority) = self.remote_authority() {
                crate::credential_cache::forget(authority);
            }
        }
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        remote.prompt = None;
        remote.prompt_draft.clear();
        // A pre-attach failure cannot supply a tree. Project a bounded
        // unavailable Explorer state instead of leaving it loading forever.
        if !remote.attached_once {
            remote
                .workspace
                .project_unavailable(made_remote::protocol::ErrorCode::Unavailable);
        }
        if error.class == crate::remote_session::SshFailureClass::UnknownHost {
            remote.phase = RemotePhase::NeedsHostApproval;
            remote.detail = error.detail;
            remote.host_fingerprint = error.fingerprint;
            remote.host_retry = Some(retry);
            return;
        }
        // Only update-required itself, or a retryable failure of an approved
        // update attempt, keeps the update card. Changed keys, auth failures,
        // cancellation and missing packaged capabilities stay terminal.
        let retryable_update = matches!(retry, OpenRetry::Update)
            && matches!(
                error.class,
                crate::remote_session::SshFailureClass::Timeout
                    | crate::remote_session::SshFailureClass::Other
            );
        if error.class == crate::remote_session::SshFailureClass::UpdateRequired || retryable_update
        {
            remote.companion_provisioned = false;
            remote.update_approved = false;
            remote.update_target = remote.remote_target.clone();
            remote.update_failed =
                error.class != crate::remote_session::SshFailureClass::UpdateRequired;
            remote.phase = RemotePhase::NeedsUpdate;
            remote.detail = error.detail;
            remote.host_fingerprint = None;
            remote.host_retry = None;
            return;
        }
        remote.host_fingerprint = None;
        remote.host_retry = None;
        remote.phase = RemotePhase::Failed;
        remote.detail = error.detail;
        remote.companion_provisioned = false;
        // Terminal failures end this update attempt. A later normal stage
        // must not inherit its consent or its retry card.
        remote.update_approved = false;
        remote.update_failed = false;
        remote.update_target = None;
    }

    fn retry_remote_probe(&mut self) {
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        let snapshot = remote.snapshot.clone();
        remote.phase = RemotePhase::Connecting;
        remote.detail = "Retrying after host approval. No local shell was started.".to_owned();
        remote.prompt = None;
        remote.probe = Some(crate::ssh_servers::spawn_probe(
            crate::ssh_servers::ProbeRequest {
                ticket: crate::ssh_servers::ProbeTicket {
                    authority: snapshot.server().id(),
                    attempt: remote.client_epoch.saturating_add(1),
                },
                server: snapshot,
            },
        ));
    }

    /// Apply a finished Stage result. Success is the only path that starts the companion.
    pub fn deliver_stage_result(&mut self, result: Result<(), crate::ssh_servers::AttemptError>) {
        self.finish_remote_stage(result);
    }

    pub fn companion_spawn_requested(&self) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|remote| remote.companion_spawn_requested)
    }

    fn spawn_remote_companion(&mut self) {
        if let Some(remote) = self.remote.as_mut() {
            remote.companion_spawn_requested = true;
        }
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        let Some(target) = remote.remote_target.clone() else {
            remote.phase = RemotePhase::Failed;
            remote.detail =
                "The remote target is unknown. Made did not start a local shell.".to_owned();
            remote.companion_provisioned = false;
            return;
        };
        let metadata = remote.snapshot.server().metadata().clone();
        let mut identity = remote.workspace.identity().clone();
        identity.epoch = remote.client_epoch;
        let root = remote.workspace.launch_directory().clone();
        let version = APP_VERSION.to_owned();
        let (tx, rx) = mpsc::channel();
        remote.client_rx = Some(rx);
        remote.phase = RemotePhase::Connecting;
        remote.detail = "Companion staged. Opening the authenticated session.".to_owned();
        let _ = thread::Builder::new()
            .name("made-ssh-attach".to_owned())
            .spawn(move || {
                let result = crate::ssh_servers::connect_companion(
                    &metadata, &version, &target, identity, root,
                );
                let _ = tx.send(result);
            });
    }

    /// The first authenticated frame is Open or Attach. Nothing else is queued.
    fn arm_remote_client(&mut self, client: made_remote::client::RemoteClient) {
        let Some(remote) = self.remote.as_mut() else {
            client.disconnect();
            return;
        };
        let directory = remote.workspace.launch_directory().clone();
        let launch = if remote.opencode {
            made_remote::protocol::LaunchTarget::OpenCode
        } else if remote.grok {
            made_remote::protocol::LaunchTarget::Grok
        } else {
            made_remote::protocol::LaunchTarget::Shell
        };
        let operation = crate::remote_session::first_operation(
            remote.session_open,
            directory,
            launch,
            DEFAULT_ROWS as u16,
            DEFAULT_COLS as u16,
        );
        debug_assert!(crate::remote_session::allow_before_attached(&operation));
        // Remote output arrives on the transport thread. Without a wake the
        // window repaints only on the next poll, so an animation frame that
        // lands after an idle gap waits for it.
        if let Some(ctx) = self.frame_ctx.clone() {
            let visible = remote.wake_visible.clone();
            client.set_wake(move || {
                if visible.load(Ordering::Relaxed) {
                    ctx.request_repaint_after(crate::metrics::ACTIVE_REPAINT_INTERVAL);
                }
            });
        }
        self.diag.log_mut().record(
            LogLevel::Info,
            format!("remote opening: {:?} epoch {}", launch, remote.client_epoch),
        );
        match client.request(operation, None) {
            Ok(id) => {
                remote.client = Some(client);
                remote.attach_request = Some(id);
                remote.phase = RemotePhase::AwaitingAttach;
                remote.detail =
                    "Session opening. Root checks wait until the remote session is attached."
                        .to_owned();
            }
            Err(_) => {
                client.disconnect();
                remote.phase = RemotePhase::Failed;
                remote.detail =
                    "The remote session rejected the opening frame. No local shell was started."
                        .to_owned();
                remote.companion_provisioned = false;
                remote
                    .workspace
                    .project_unavailable(made_remote::protocol::ErrorCode::Unavailable);
            }
        }
    }

    fn pump_remote_client(&mut self) {
        let (attach_reply, upload_replies, disconnected) =
            {
                let Some(remote) = self.remote.as_mut() else {
                    return;
                };
                let Some(client) = remote.client.clone() else {
                    return;
                };
                if !client.is_connected() {
                    (None, Vec::new(), true)
                } else {
                    let waiting_attach = remote.attach_request;
                    let mut attach_reply = None;
                    let mut upload_replies = Vec::new();
                    while let Some(reply) = client.poll_reply() {
                        if waiting_attach == Some(reply.request_id) {
                            attach_reply = Some(reply);
                            continue;
                        }
                        if remote.upload.as_ref().is_some_and(|upload| {
                            upload.begin_id == Some(reply.request_id)
                                || upload.inflight_chunk == Some(reply.request_id)
                                || upload.commit_id == Some(reply.request_id)
                        }) {
                            upload_replies.push(reply);
                            continue;
                        }
                        if remote.attach_request.is_some() {
                            continue;
                        }
                        // The shell's cwd report is an unsolicited, untrusted
                        // event, not a reply to a pending request. It enters
                        // the root precedence only through server validation.
                        // The report is not a filesystem result, so a root
                        // generation that moved while it was in flight must
                        // not drop it; transport authority and epoch still
                        // have to match.
                        if let made_remote::protocol::Operation::ShellCwd { path } = reply.operation
                        {
                            let mut identity = reply.identity;
                            identity.generation = remote.workspace.identity().generation;
                            let _ = remote.workspace.report_shell_cwd(&identity, path);
                            continue;
                        }
                        let listed = matches!(
                            reply.operation,
                            made_remote::protocol::Operation::Listed { .. }
                        );
                        let client_id = reply.request_id;
                        // A streamed reply (directory, preview, git, watch)
                        // keeps the same client request id across batches.
                        // Remove the mapping only when that stream is done.
                        let terminal = match &reply.operation {
                            made_remote::protocol::Operation::Listed { done, .. }
                            | made_remote::protocol::Operation::PreviewResult { done, .. }
                            | made_remote::protocol::Operation::GitResult { done, .. } => *done,
                            made_remote::protocol::Operation::Changed { .. } => false,
                            _ => true,
                        };
                        let mut reply = reply;
                        if let Some(workspace_id) = remote.id_map.get(&client_id).copied() {
                            reply.request_id = workspace_id;
                            if terminal {
                                remote.id_map.remove(&client_id);
                            }
                        }
                        let accepted = remote.workspace.accept(reply);
                        if listed {
                            let root = remote.workspace.root().clone();
                            let entries = remote
                                .workspace
                                .projection()
                                .directories
                                .get(&root)
                                .map(|directory| directory.entries.len());
                            self.diag.log_mut().record(
                                LogLevel::Info,
                                format!(
                                "remote listed applied: ok={} root_present={} entries={entries:?}",
                                accepted.is_ok(),
                                remote.workspace.projection().directories.contains_key(&root)
                            ),
                            );
                        }
                    }
                    // Root and session results use dedicated client ports. They
                    // still belong to workspace request ids and must be applied
                    // through the same id map as the generic queue.
                    use made_remote::protocol::{RootResolver, SessionCommandSink};
                    let mut resolver = client.root_resolver();
                    while let Some(mut resolution) = resolver.poll() {
                        self.diag.log_mut().record(
                            LogLevel::Info,
                            format!(
                                "remote root reply: generation {} ok={}",
                                resolution.generation,
                                resolution.result.is_ok()
                            ),
                        );
                        if let Some(workspace_id) = remote.id_map.remove(&resolution.request_id) {
                            resolution.request_id = workspace_id;
                        } else {
                            continue;
                        }
                        let generation = resolution.generation;
                        if let Ok(Some(crate::remote_workspace::RemoteResult::Root(path))) =
                            remote.workspace.accept_root(resolution)
                        {
                            if generation > client.identity().generation {
                                let _ = client.set_root(path, generation);
                            }
                        }
                    }
                    let mut sessions = client.session_sink();
                    while let Some(reply) = sessions.poll() {
                        self.diag.log_mut().record(
                            LogLevel::Info,
                            format!("remote session reply: {}", operation_name(&reply.operation)),
                        );
                        // The client classifies Grok snapshots and events as
                        // unsolicited pushes riding the current attach id.
                        // They never consume a pending workspace request
                        // mapping, even by numeric id equality; a correlated
                        // GrokState reply arrives on the generic queue above.
                        if matches!(
                            reply.operation,
                            made_remote::protocol::Operation::GrokSnapshot { .. }
                                | made_remote::protocol::Operation::GrokEvent { .. }
                        ) {
                            let _ = remote.workspace.accept_grok_push(reply);
                            continue;
                        }
                        if let Some(workspace_id) = remote.id_map.remove(&reply.request_id) {
                            let mut reply = reply;
                            reply.request_id = workspace_id;
                            let _ = remote.workspace.accept(reply);
                        } else {
                            // Unsolicited plugin state uses the attach request id,
                            // which can collide with a workspace request id.
                            let _ = remote.workspace.accept_session_event(reply);
                        }
                    }
                    (attach_reply, upload_replies, false)
                }
            };
        if disconnected {
            self.fail_remote_transport();
            return;
        }
        if attach_reply.is_some() || !upload_replies.is_empty() {
            self.finish_remote_replies(attach_reply, upload_replies);
        }
        {
            let Some(remote) = self.remote.as_mut() else {
                return;
            };
            if remote.attach_request.is_some() {
                return;
            }
            let Some(client) = remote.client.clone() else {
                return;
            };
            while let Some(command) = remote.workspace.next_command() {
                if remote.workspace.connection()
                    != crate::remote_workspace::ConnectionState::Connected
                    && !crate::remote_session::allow_before_attached(&command.operation)
                {
                    continue;
                }
                let workspace_id = command.request_id;
                let operation = command.operation;
                let root_select = matches!(
                    &operation,
                    made_remote::protocol::Operation::RootSelect { .. }
                );
                let grant = match &operation {
                    made_remote::protocol::Operation::RootSelect { path }
                    | made_remote::protocol::Operation::RootValidate { path } => Some(path.clone()),
                    _ => None,
                };
                if root_select {
                    // The transition already dropped the workspace's old-root
                    // work silently. Reset the client's bounded queues and id
                    // map too, so old pending requests cannot consume the
                    // in-flight slots the RootSelect needs.
                    remote.id_map.clear();
                    if client.reset_pending().is_err() {
                        remote.workspace.disconnect();
                        break;
                    }
                }
                match client.request(operation, grant.as_ref()) {
                    Ok(client_id) => {
                        remote.id_map.insert(client_id, workspace_id);
                    }
                    Err(_) => {
                        remote.workspace.disconnect();
                        break;
                    }
                }
            }
            let _ = remote.workspace.expire(Instant::now());
            let _ = remote.workspace.reconcile();
        };
        self.sync_remote_sessions();
    }

    /// Mirror the remote projection's session snapshot into the shared session
    /// map and drop stale rows once the remote integration no longer carries
    /// sessions. The projection is the only remote session channel; the
    /// shared map feeds the sessions panel and the workspace tab dots.
    fn sync_remote_sessions(&mut self) {
        let Some(remote) = self.remote.as_ref() else {
            return;
        };
        match remote.workspace.projection().sessions.clone() {
            Some(snapshot) => {
                self.session_map.apply_snapshot(&snapshot);
            }
            None => {
                if remote.workspace.integration()
                    != crate::remote_workspace::IntegrationState::Connected
                {
                    self.session_map.clear();
                }
            }
        }
    }

    fn fail_remote_transport(&mut self) {
        self.diag
            .log_mut()
            .record(LogLevel::Error, "remote transport disconnected");
        if let Some(remote) = self.remote.as_mut() {
            remote.workspace.disconnect();
            remote.phase = RemotePhase::Failed;
            remote.detail =
                "SSH disconnected. Queued input was discarded and will not be replayed.".to_owned();
            remote.companion_provisioned = false;
            remote.client = None;
            remote.attach_request = None;
            remote.holding_input = false;
            remote.upload = None;
            // Stale remote session rows must not outlive the transport.
            self.session_map.clear();
        }
        if let Some(terminal) = self.terminal.as_mut() {
            terminal.fail_remote_image_paste();
        }
    }

    fn finish_remote_replies(
        &mut self,
        attach_reply: Option<made_remote::protocol::WireEnvelope>,
        upload_replies: Vec<made_remote::protocol::WireEnvelope>,
    ) {
        if let Some(reply) = attach_reply {
            self.note_attach_reply(reply);
        }
        for reply in upload_replies {
            self.apply_upload_reply(reply);
        }
    }

    fn apply_upload_reply(&mut self, reply: made_remote::protocol::WireEnvelope) {
        let outcome = {
            let Some(remote) = self.remote.as_mut() else {
                return;
            };
            let Some(upload) = remote.upload.as_mut() else {
                return;
            };
            use made_remote::protocol::Operation;
            match &reply.operation {
                Operation::Ack if upload.begin_id == Some(reply.request_id) => {
                    upload.begin_acked = true;
                    upload.begin_id = None;
                    None
                }
                Operation::Ack if upload.inflight_chunk == Some(reply.request_id) => {
                    upload.inflight_chunk = None;
                    None
                }
                Operation::UploadResult { path, .. } => {
                    let pasted = path.to_utf8().ok().map(str::to_owned);
                    remote.upload = None;
                    Some(pasted)
                }
                Operation::Error { .. } => {
                    remote.upload = None;
                    Some(None)
                }
                _ => None,
            }
        };
        let Some(pasted) = outcome else {
            return;
        };
        let Some(terminal) = self.terminal.as_mut() else {
            return;
        };
        match pasted {
            Some(path) => {
                let _ = terminal.finish_remote_image_paste(&path, self.diag.log_mut());
            }
            None => terminal.fail_remote_image_paste(),
        }
    }

    fn note_attach_reply(&mut self, reply: made_remote::protocol::WireEnvelope) {
        self.diag.log_mut().record(
            LogLevel::Info,
            format!("remote attach reply: {}", operation_name(&reply.operation)),
        );
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        remote.attach_request = None;
        match reply.operation {
            made_remote::protocol::Operation::Attached => {}
            made_remote::protocol::Operation::Error { code } => {
                self.diag
                    .log_mut()
                    .record(LogLevel::Error, format!("remote attach error: {code:?}"));
                remote.phase = RemotePhase::Failed;
                remote.detail = crate::remote_session::classify_error(code)
                    .detail()
                    .to_owned();
                remote.companion_provisioned = false;
                remote
                    .workspace
                    .project_unavailable(made_remote::protocol::ErrorCode::Unavailable);
                if let Some(client) = remote.client.take() {
                    client.disconnect();
                }
                return;
            }
            _ => {
                remote.phase = RemotePhase::Failed;
                remote.detail =
                    "The remote session did not attach. No local shell was started.".to_owned();
                remote.companion_provisioned = false;
                remote
                    .workspace
                    .project_unavailable(made_remote::protocol::ErrorCode::Unavailable);
                return;
            }
        }
        let open = remote.session_open;
        let epoch = remote.client.as_ref().map(|client| client.identity().epoch);
        let attached = match open {
            crate::remote_session::SessionOpen::New => remote
                .workspace
                .mark_attached()
                .and_then(|()| remote.workspace.begin_resnapshot()),
            crate::remote_session::SessionOpen::Reattach => {
                remote.workspace.reattach(epoch.unwrap_or(0))
            }
        };
        if attached.is_err() {
            remote.phase = RemotePhase::Failed;
            remote.detail =
                "The remote session attached, but this tab could not accept it. Input was not sent."
                    .to_owned();
            remote.companion_provisioned = false;
            remote
                .workspace
                .project_unavailable(made_remote::protocol::ErrorCode::Unavailable);
            return;
        }
        remote.attached_once = true;
        remote.phase = RemotePhase::Ready;
        remote.holding_input = true;
        remote.detail =
            "SSH attached. Applying the remote snapshot before accepting input.".to_owned();
        let Some(client) = remote.client.clone() else {
            return;
        };
        let target = self.terminal_target;
        let mut pane = TerminalPane::open_remote(target, DEFAULT_ROWS as u16, DEFAULT_COLS as u16);
        pane.attach_remote_transport(Box::new(client.terminal_transport()));
        self.terminal = Some(pane);
        self.terminal_error = None;
        if let Some(remote) = self.remote.as_mut() {
            remote.snapshot_gate = crate::remote_session::SnapshotGate::default();
            remote.holding_input = true;
        }
        // An empty queue here is not completion. Input stays paused until done.
        self.pull_resnapshot();
    }

    fn pump_remote_terminal(&mut self) -> bool {
        let before = self.terminal.as_ref().map(TerminalPane::output_bytes);
        let holding = self
            .remote
            .as_ref()
            .is_some_and(|remote| remote.holding_input);
        if !holding {
            if let Some(terminal) = self.terminal.as_mut() {
                let _ = terminal.pump(self.diag.log_mut());
            }
        } else {
            self.pull_resnapshot();
        }
        before != self.terminal.as_ref().map(TerminalPane::output_bytes)
    }

    /// Apply queued `TerminalOutput` frames. `poll_output` returning `None` does
    /// not resume input. Only the first `done: true` frame does, after it is fed.
    fn pull_resnapshot(&mut self) {
        let Some(client) = self
            .remote
            .as_ref()
            .and_then(|remote| remote.client.clone())
        else {
            return;
        };
        if !self
            .remote
            .as_ref()
            .is_some_and(|remote| remote.holding_input)
        {
            return;
        }
        let mut transport = client.terminal_transport();
        loop {
            let frame = transport.poll_output();
            let gate = self
                .remote
                .as_ref()
                .map(|remote| remote.snapshot_gate)
                .unwrap_or_default();
            let (gate, step) = match &frame {
                None => gate.observe_empty(),
                Some(frame) => gate.observe_frame(frame.done),
            };
            if let Some(remote) = self.remote.as_mut() {
                remote.snapshot_gate = gate;
            }
            let Some(frame) = frame else {
                break;
            };
            self.apply_snapshot_step(step, &frame.bytes);
            if matches!(step, crate::remote_session::SnapshotStep::Complete { .. }) {
                if let Some(terminal) = self.terminal.as_mut() {
                    terminal.resume_input();
                }
                if let Some(remote) = self.remote.as_mut() {
                    remote.holding_input = false;
                    remote.detail = "SSH attached.".to_owned();
                }
            }
        }
    }

    fn apply_snapshot_step(&mut self, step: crate::remote_session::SnapshotStep, bytes: &[u8]) {
        let Some(terminal) = self.terminal.as_mut() else {
            return;
        };
        match step {
            crate::remote_session::SnapshotStep::StillWaiting => {}
            crate::remote_session::SnapshotStep::Feed { reset: true }
            | crate::remote_session::SnapshotStep::Complete { reset: true } => {
                terminal.apply_resnapshot(bytes);
            }
            crate::remote_session::SnapshotStep::Feed { reset: false }
            | crate::remote_session::SnapshotStep::Complete { reset: false }
            | crate::remote_session::SnapshotStep::Live => {
                if !bytes.is_empty() {
                    terminal.feed_remote_output(bytes);
                }
            }
        }
    }

    fn pump_remote_upload(&mut self) {
        let pending = self
            .terminal
            .as_mut()
            .and_then(TerminalPane::take_pending_image);
        if let Some(bytes) = pending {
            let ready = self.remote.as_ref().is_some_and(|remote| {
                remote.attach_request.is_none()
                    && remote.client.is_some()
                    && !remote.holding_input
                    && remote.upload.is_none()
            });
            if !ready {
                if let Some(terminal) = self.terminal.as_mut() {
                    terminal.fail_remote_image_paste();
                }
            } else if let Some(remote) = self.remote.as_mut() {
                let upload_id = remote.next_upload;
                remote.next_upload = remote.next_upload.saturating_add(1);
                remote.upload = Some(PendingUpload {
                    upload_id,
                    bytes,
                    offset: 0,
                    begin_id: None,
                    begin_acked: false,
                    inflight_chunk: None,
                    commit_id: None,
                });
            }
        }
        let mut failed = false;
        {
            let Some(remote) = self.remote.as_mut() else {
                return;
            };
            let Some(client) = remote.client.clone() else {
                return;
            };
            let Some(upload) = remote.upload.as_mut() else {
                return;
            };
            if upload.begin_id.is_none() && !upload.begin_acked {
                match client.request(
                    made_remote::protocol::Operation::UploadBegin {
                        upload_id: upload.upload_id,
                        total: upload.bytes.len() as u64,
                    },
                    None,
                ) {
                    Ok(id) => upload.begin_id = Some(id),
                    Err(_) => {
                        remote.upload = None;
                        failed = true;
                    }
                }
            } else if upload.begin_acked
                && upload.inflight_chunk.is_none()
                && upload.commit_id.is_none()
            {
                if upload.offset < upload.bytes.len() as u64 {
                    let start = upload.offset as usize;
                    let end =
                        (start + made_remote::protocol::MAX_CHUNK_BYTES).min(upload.bytes.len());
                    let chunk = upload.bytes[start..end].to_vec();
                    let offset = upload.offset;
                    let upload_id = upload.upload_id;
                    match made_remote::protocol::Chunk::new(chunk) {
                        Ok(data) => match client.request(
                            made_remote::protocol::Operation::UploadChunk {
                                upload_id,
                                offset,
                                data,
                            },
                            None,
                        ) {
                            Ok(id) => {
                                if let Some(upload) = remote.upload.as_mut() {
                                    upload.inflight_chunk = Some(id);
                                    upload.offset = end as u64;
                                }
                            }
                            Err(_) => {
                                remote.upload = None;
                                failed = true;
                            }
                        },
                        Err(_) => {
                            remote.upload = None;
                            failed = true;
                        }
                    }
                } else {
                    let upload_id = upload.upload_id;
                    match client.request(
                        made_remote::protocol::Operation::UploadCommit { upload_id },
                        None,
                    ) {
                        Ok(id) => {
                            if let Some(upload) = remote.upload.as_mut() {
                                upload.commit_id = Some(id);
                            }
                        }
                        Err(_) => {
                            remote.upload = None;
                            failed = true;
                        }
                    }
                }
            }
        }
        if failed {
            self.fail_remote_paste();
        }
    }

    fn fail_remote_paste(&mut self) {
        if let Some(terminal) = self.terminal.as_mut() {
            terminal.fail_remote_image_paste();
        }
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let frame_time = now.saturating_duration_since(self.last_frame);
        self.last_frame = now;
        self.frame_ctx = Some(ctx.clone());
        self.finalize_paint_metrics();

        self.tick_count = self.tick_count.saturating_add(1);
        if self.remote.is_some() {
            self.tick_remote(ctx);
            return;
        }
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
                            .is_none_or(|follow| !follow.is_agent_owned());
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

        let fs_activity = if let Some(explorer) = self.explorer.as_mut() {
            explorer.tick(self.diag.log_mut());
            let fs_activity = explorer.take_fs_activity();
            if let Some(latency) = explorer.take_switch_latency() {
                self.metrics.record_session_switch(latency);
                self.last_activity = now;
                active = true;
            }
            fs_activity
        } else {
            false
        };
        if fs_activity {
            self.note_git_fs_activity(now);
        }

        self.poll_shell_root_revalidation(now);
        self.maybe_schedule_shell_root_revalidation(now);
        self.poll_grok_hooks(now);
        self.poll_grok_directory(now);

        self.try_send_pending_worktree();
        if self.poll_worktree() {
            active = true;
        }
        self.sync_git_status_scope();
        self.try_send_pending_git_status();
        if self.poll_git_status() {
            active = true;
        }
        if self.maybe_debounce_git_status(now) {
            active = true;
        }
        self.push_git_chrome();

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
            !follow.is_agent_owned() && follow.generation() == result.generation
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

    /// Whether Grok is running in this workspace.
    ///
    /// Running means the terminal target is Grok and that process is still
    /// alive, or the inventory has confirmed at least one open Grok session
    /// for this terminal. A hook row starts with an empty `opened_at` and does
    /// not count until that confirmation. The tab label is not part of the
    /// check: OpenCode can own the label while Grok runs.
    pub(crate) fn grok_follow_running(&self) -> bool {
        let living_target = matches!(
            self.terminal_target,
            TerminalTarget::Tui(CompatibleTui::Grok)
        ) && self.terminal.as_ref().is_some_and(TerminalPane::is_running);
        let confirmed = self
            .grok_sessions
            .iter()
            .any(|session| !session.opened_at.is_empty());
        living_target || confirmed
    }

    /// Status dots for Grok sessions open in this terminal.
    pub(crate) fn grok_tab_activities(&self) -> Vec<SessionActivity> {
        let mut sessions = self.grok_sessions.clone();
        sessions.sort_by(|left, right| left.opened_at.cmp(&right.opened_at));
        sessions
            .into_iter()
            .map(|session| session.activity)
            .collect()
    }

    /// Apply hook lines as soon as they are appended.
    fn poll_grok_hooks(&mut self, now: Instant) {
        let (offset, events) = crate::grok_follow::read_hook_events(self.grok_log_offset);
        self.grok_log_offset = offset;
        for event in events {
            let moves_explorer = matches!(
                event.event.to_ascii_lowercase().as_str(),
                "session_start" | "sessionstart" | "user_prompt_submit" | "userpromptsubmit"
            );
            if event.event.eq_ignore_ascii_case("session_end")
                || event.event.eq_ignore_ascii_case("sessionend")
            {
                let ended = event.session_id.clone();
                let was_provisional = self.grok_provisional_id.as_deref() == Some(ended.as_str());
                self.grok_closed.insert(ended.clone());
                self.grok_sessions.retain(|session| session.id != ended);
                if was_provisional {
                    self.release_provisional_move();
                } else if self.grok_sessions.is_empty() {
                    self.grok_file_newest_id = None;
                    self.follow_grok_path(None);
                }
                continue;
            }
            // The log is global. Re-read inventory before moving so a session
            // this tab owns is not delayed, and a foreign pid is not followed.
            let ownership = self.hook_session_ownership(&event.session_id);
            if ownership == crate::grok_follow::SessionOwnership::Foreign {
                let id = event.session_id.clone();
                let was_provisional = self.grok_provisional_id.as_deref() == Some(id.as_str());
                self.grok_sessions.retain(|session| session.id != id);
                if was_provisional {
                    self.release_provisional_move();
                }
                continue;
            }
            self.grok_closed.remove(&event.session_id);
            let activity = match event.status.as_str() {
                "working" => SessionActivity::Busy,
                "attention" | "error" => SessionActivity::Attention,
                _ => SessionActivity::Idle,
            };
            let cwd = PathBuf::from(&event.cwd);
            if let Some(session) = self
                .grok_sessions
                .iter_mut()
                .find(|session| session.id == event.session_id)
            {
                session.activity = activity;
                session.touched = now;
                if cwd.is_absolute() {
                    session.cwd = cwd.clone();
                }
            } else if !event.session_id.is_empty() {
                self.grok_sessions.push(GrokLiveSession {
                    id: event.session_id.clone(),
                    cwd: cwd.clone(),
                    opened_at: String::new(),
                    activity,
                    touched: now,
                });
            }
            if moves_explorer && cwd.is_absolute() {
                if ownership == crate::grok_follow::SessionOwnership::Owned {
                    // A prompt in a session this tab owns replaces any earlier
                    // provisional move, including one for a different session.
                    self.clear_provisional_slot();
                    self.follow_grok_path(Some(cwd.as_path()));
                } else {
                    self.begin_provisional_move(&event.session_id, cwd.as_path());
                }
            }
            self.last_activity = now;
        }
    }

    fn hook_session_ownership(&self, session_id: &str) -> crate::grok_follow::SessionOwnership {
        let Some(pid) = self.terminal.as_ref().and_then(TerminalPane::pid) else {
            return crate::grok_follow::SessionOwnership::Unknown;
        };
        crate::grok_follow::session_ownership(pid, session_id)
    }

    /// Remember the directory this tab was following, then follow `cwd` until
    /// inventory confirms the session.
    fn begin_provisional_move(&mut self, session_id: &str, cwd: &Path) {
        if self.grok_provisional_id.is_none() {
            self.grok_restore_root = self.follow.as_ref().and_then(|follow| {
                (follow.effective_root_source() == crate::session_follow::RootSource::Grok)
                    .then(|| follow.current_root().to_path_buf())
            });
        }
        self.grok_provisional_id = Some(session_id.to_owned());
        self.follow_grok_path(Some(cwd));
        self.grok_provisional_cwd = self.explorer_root().map(Path::to_path_buf);
    }

    fn clear_provisional_slot(&mut self) {
        self.grok_provisional_id = None;
        self.grok_provisional_cwd = None;
        self.grok_restore_root = None;
    }

    fn release_provisional_move(&mut self) {
        if self.grok_provisional_id.is_none() {
            return;
        }
        let restore = self.grok_restore_root.take();
        let provisional_cwd = self.grok_provisional_cwd.take();
        self.grok_provisional_id = None;
        // An owned prompt after this move already left the provisional
        // directory. Restoring would undo that prompt.
        let still_there = provisional_cwd
            .as_ref()
            .is_some_and(|cwd| self.explorer_root() == Some(cwd.as_path()));
        if !still_there {
            return;
        }
        match restore {
            Some(path) => self.follow_grok_path(Some(&path)),
            None => self.follow_grok_path(None),
        }
    }

    /// Refresh the open-session inventory and follow a newly attached session.
    fn poll_grok_directory(&mut self, now: Instant) {
        if now.saturating_duration_since(self.grok_checked_at) < Duration::from_secs(1) {
            return;
        }
        self.grok_checked_at = now;
        let Some(pid) = self.terminal.as_ref().and_then(TerminalPane::pid) else {
            return;
        };
        let open = crate::grok_follow::open_sessions(pid);
        let open_ids: Vec<&str> = open.iter().map(|row| row.session_id.as_str()).collect();
        self.grok_closed
            .retain(|id| open_ids.contains(&id.as_str()));
        for row in &open {
            if self.grok_closed.contains(&row.session_id) {
                continue;
            }
            let cwd = PathBuf::from(&row.cwd);
            if let Some(session) = self
                .grok_sessions
                .iter_mut()
                .find(|session| session.id == row.session_id)
            {
                session.opened_at = row.opened_at.clone();
                if cwd.is_absolute() {
                    session.cwd = cwd;
                }
            } else {
                self.grok_sessions.push(GrokLiveSession {
                    id: row.session_id.clone(),
                    cwd,
                    opened_at: row.opened_at.clone(),
                    activity: SessionActivity::Idle,
                    touched: now,
                });
            }
        }
        let mut reverted_provisional = false;
        if let Some(id) = self.grok_provisional_id.clone() {
            let confirmed = open_ids.contains(&id.as_str());
            if confirmed {
                self.clear_provisional_slot();
            } else {
                // Pid is missing, or the row belongs to another process and was
                // filtered out. Drop the dot and undo only that session's move.
                self.grok_sessions.retain(|session| session.id != id);
                self.release_provisional_move();
                reverted_provisional = true;
            }
        }
        self.grok_sessions.retain(|session| {
            !self.grok_closed.contains(&session.id) && open_ids.contains(&session.id.as_str())
        });
        if reverted_provisional {
            return;
        }
        let newest = open
            .iter()
            .rev()
            .find(|row| !self.grok_closed.contains(&row.session_id));
        match newest {
            Some(row) if self.grok_file_newest_id.as_deref() != Some(row.session_id.as_str()) => {
                self.grok_file_newest_id = Some(row.session_id.clone());
                let path = PathBuf::from(&row.cwd);
                if path.is_absolute() {
                    self.follow_grok_path(Some(path.as_path()));
                }
            }
            None if open.is_empty() && self.grok_file_newest_id.is_some() => {
                self.grok_file_newest_id = None;
                self.follow_grok_path(None);
            }
            _ => {}
        }
    }

    fn follow_grok_path(&mut self, directory: Option<&Path>) {
        let Some(outcome) = self
            .follow
            .as_mut()
            .map(|follow| follow.follow_grok_directory(directory))
        else {
            return;
        };
        if outcome.root_switched || self.explorer_generation_mismatch(outcome.generation) {
            self.switch_root(outcome.root, outcome.generation, RootSwitchOrigin::Session);
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
        if follow.is_agent_owned() {
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
            SessionFollowUpdate::ViewDirectory => {
                self.shell_root_unavailable = None;
                if let Some(explorer) = self.explorer.as_mut() {
                    explorer.clear_root_unavailable();
                }
                self.integration.note_root(&outcome.root);
                if outcome.root_switched || self.explorer_generation_mismatch(outcome.generation) {
                    let view = self
                        .follow
                        .as_ref()
                        .and_then(SessionFollowCoordinator::view_directory)
                        .map(|view| view.directory().display().to_string());
                    self.switch_root(
                        outcome.root.clone(),
                        outcome.generation,
                        RootSwitchOrigin::Session,
                    );
                    if let Some(view) = view {
                        self.diag.log_mut().push(LogEntry::new(
                            LogLevel::Info,
                            format!("view directory followed: {view}"),
                        ));
                    }
                }
            }
            SessionFollowUpdate::ViewDirectoryUnavailable { directory, detail } => {
                self.diag.log_mut().push(LogEntry::new(
                    LogLevel::Warn,
                    format!(
                        "view directory unavailable at {}: {detail}",
                        directory.display()
                    ),
                ));
                if outcome.root_switched || self.explorer_generation_mismatch(outcome.generation) {
                    self.switch_root(
                        outcome.root.clone(),
                        outcome.generation,
                        RootSwitchOrigin::Session,
                    );
                }
                // A reported but unusable view directory falls back to the
                // shell root without presenting an unavailable-root tree.
                self.settle_no_active_root(&outcome.root);
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

    fn is_git_repo(&self) -> bool {
        self.worktree
            .metadata
            .as_ref()
            .is_some_and(|metadata| metadata.is_known() && metadata.is_worktree)
    }

    fn note_git_fs_activity(&mut self, now: Instant) {
        if self.is_git_repo() {
            self.git_status.dirty_at = Some(now);
        }
    }

    fn sync_git_status_scope(&mut self) {
        if !self.is_git_repo() {
            self.git_status.clear();
            self.pending_git_status.clear();
            return;
        }
        let generation = self.worktree.generation;
        if self.git_status.generation != generation {
            self.git_status.clear();
            self.git_status.generation = generation;
        }
        if !self.git_status.attempted && !self.git_status.loading {
            self.request_git_status();
        }
    }

    fn request_git_status(&mut self) {
        if !self.is_git_repo() {
            return;
        }
        let Some(root) = self
            .explorer
            .as_ref()
            .and_then(ExplorerPane::root)
            .map(Path::to_path_buf)
        else {
            return;
        };
        let toplevel = self
            .worktree
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.toplevel.clone());
        let generation = self.worktree.generation;
        self.git_status.generation = generation;
        self.git_status.loading = true;
        self.git_status.attempted = true;
        self.git_status.request_started_at = Some(Instant::now());
        self.pending_git_status.offer(GitStatusJob {
            root,
            toplevel,
            generation,
        });
        self.try_send_pending_git_status();
    }

    /// Refresh when the user opens source control and the snapshot is stale.
    pub(crate) fn refresh_git_status_if_stale(&mut self) {
        if !self.is_git_repo() {
            return;
        }
        let stale = self.git_status.snapshot.is_none()
            || self.git_status.failed
            || self.git_status.dirty_at.is_some();
        if !stale {
            return;
        }
        if self.git_status.loading {
            return;
        }
        self.git_status.dirty_at = None;
        self.request_git_status();
    }

    fn try_send_pending_git_status(&mut self) {
        let worker = &self.git_status_worker;
        if self
            .pending_git_status
            .try_send(|job| worker.request(job.clone()))
        {
            self.git_status.loading = true;
        }
    }

    fn poll_git_status(&mut self) -> bool {
        let mut changed = false;
        while let Some(result) = self.git_status_worker.try_recv() {
            if result.generation != self.worktree.generation || !self.is_git_repo() {
                continue;
            }
            self.git_status.loading = false;
            changed = true;
            match result.read {
                GitStatusRead::Ready(snapshot) => {
                    let count: usize = snapshot.groups.iter().map(|group| group.rows.len()).sum();
                    self.git_status.snapshot = Some(snapshot);
                    self.git_status.failed = false;
                    self.diag
                        .log_mut()
                        .record(LogLevel::Info, format!("git status: {count} change(s)"));
                }
                GitStatusRead::Failed { reason } | GitStatusRead::Unavailable { reason } => {
                    if self.git_status.snapshot.is_none() {
                        self.git_status.failed = true;
                    }
                    self.diag.log_mut().record(
                        LogLevel::Warn,
                        format!("git status unavailable; explorer continues ({reason})"),
                    );
                }
            }
            let watch_after_request = self.git_status.dirty_at.is_some_and(|dirty| {
                self.git_status
                    .request_started_at
                    .is_some_and(|started| dirty > started)
            });
            if !watch_after_request {
                self.git_status.dirty_at = None;
            }
        }
        changed
    }

    fn maybe_debounce_git_status(&mut self, now: Instant) -> bool {
        let Some(dirty_at) = self.git_status.dirty_at else {
            return false;
        };
        if !self.is_git_repo() {
            self.git_status.dirty_at = None;
            return false;
        }
        if now.saturating_duration_since(dirty_at) < GIT_STATUS_DEBOUNCE {
            return false;
        }
        if self.git_status.loading {
            if self
                .git_status
                .request_started_at
                .is_some_and(|started| started >= dirty_at)
            {
                self.git_status.dirty_at = None;
            }
            return false;
        }
        self.git_status.dirty_at = None;
        self.request_git_status();
        true
    }

    fn push_git_chrome(&mut self) {
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        let metadata = self.worktree.metadata.as_ref();
        let show_icon =
            metadata.is_some_and(|metadata| metadata.is_known() && metadata.is_worktree);
        let header = GitHeader {
            show_icon,
            branch: metadata.and_then(|metadata| metadata.branch.clone()),
            detached: metadata.is_some_and(|metadata| metadata.detached),
            commit_abbrev: metadata.and_then(|metadata| metadata.commit_abbrev.clone()),
            commit_subject: metadata.and_then(|metadata| metadata.commit_subject.clone()),
            is_linked_worktree: metadata.is_some_and(|metadata| metadata.is_linked_worktree),
            directory_name: metadata.and_then(|metadata| metadata.directory_name.clone()),
        };
        let loading = self.git_status.loading && self.git_status.snapshot.is_none();
        explorer.apply_git_chrome(ExplorerGitChrome {
            header,
            status: self.git_status.snapshot.clone(),
            failed: self.git_status.failed && self.git_status.snapshot.is_none(),
            loading,
        });
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
        // Online close is a worker. Dropping the watch does not cancel the Close.
        let _close = self.take_remote_close();
        if let Some(remote) = self.remote.as_mut() {
            remote.cancel.store(true, Ordering::Release);
            if let Some(probe) = remote.probe.as_ref() {
                probe.cancel();
            }
            if let Some(stage) = remote.stage.as_ref() {
                stage.cancel();
            }
            // A close worker may already own the client. Do not disconnect first:
            // that drops Operation::Close before it is written.
            if remote.client.is_some() && remote.attached_once {
                remote.cleanup_unconfirmed = true;
                self.diag.log_mut().record(
                    LogLevel::Warn,
                    "remote cleanup was not confirmed before local release",
                );
            }
        }

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
        let (view_directory, view_directory_available, view_directory_reason) = self
            .follow
            .as_ref()
            .and_then(|follow| follow.view_directory())
            .map(|view| {
                (
                    Some(view.directory().to_path_buf()),
                    Some(view.available),
                    view.reason.clone(),
                )
            })
            .unwrap_or((None, None, None));
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
            view_directory,
            view_directory_available,
            view_directory_reason,
            ssh_authority: self
                .remote
                .as_ref()
                .map(|remote| remote.snapshot.server().metadata().display_name.clone()),
            remote_root: self.remote_path_label(),
            ssh_transport: self.remote.as_ref().map(|remote| {
                match remote.workspace.connection() {
                    crate::remote_workspace::ConnectionState::Connected => "connected",
                    crate::remote_workspace::ConnectionState::Disconnected => "disconnected",
                }
                .to_owned()
            }),
            ssh_integration: self.remote.as_ref().map(|remote| {
                match remote.workspace.integration() {
                    crate::remote_workspace::IntegrationState::Connected => "connected",
                    crate::remote_workspace::IntegrationState::Unavailable => "unavailable",
                    crate::remote_workspace::IntegrationState::Unknown => "unknown",
                }
                .to_owned()
            }),
            remote_version: self.remote.as_ref().and_then(|remote| {
                if matches!(remote.phase, RemotePhase::NeedsUpdate) {
                    return Some(APP_VERSION.to_owned());
                }
                remote
                    .workspace
                    .projection()
                    .sessions
                    .as_ref()
                    .map(|_| APP_VERSION.to_owned())
            }),
            cleanup_unconfirmed: self
                .remote
                .as_ref()
                .map(|remote| remote.cleanup_unconfirmed),
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
        if let Some(report) = self.remote_opencode_diagnostics() {
            snapshot.set_remote_opencode(report);
        }
        snapshot.set_grok_diagnostics(self.grok_diagnostics.clone());
        snapshot
    }

    /// The owning remote tab's OpenCode integration for the diagnostics pill.
    /// Transport loss and integration loss stay distinguishable; the
    /// host-local integration state is never claimed for a remote tab.
    pub(crate) fn remote_opencode_diagnostics(
        &self,
    ) -> Option<crate::diagnostics::RemoteOpenCodeReport> {
        let remote = self.remote.as_ref()?;
        let attached = matches!(remote.phase, RemotePhase::Ready);
        let failed = matches!(remote.phase, RemotePhase::Failed | RemotePhase::NeedsUpdate)
            .then(|| remote.detail.as_str());
        Some(crate::diagnostics::remote_opencode_report(
            attached,
            failed,
            remote.workspace.connection(),
            remote.workspace.integration(),
            remote.workspace.projection().integration_error,
        ))
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

struct GitStatusRuntime {
    generation: RootGeneration,
    snapshot: Option<GitStatusSnapshot>,
    failed: bool,
    loading: bool,
    attempted: bool,
    dirty_at: Option<Instant>,
    request_started_at: Option<Instant>,
}

impl Default for GitStatusRuntime {
    fn default() -> Self {
        Self {
            generation: RootGeneration::INITIAL,
            snapshot: None,
            failed: false,
            loading: false,
            attempted: false,
            dirty_at: None,
            request_started_at: None,
        }
    }
}

impl GitStatusRuntime {
    fn clear(&mut self) {
        *self = Self {
            generation: self.generation,
            ..Self::default()
        };
    }
}

#[derive(Clone)]
struct GitStatusJob {
    root: PathBuf,
    toplevel: Option<PathBuf>,
    generation: RootGeneration,
}

struct GitStatusResult {
    generation: RootGeneration,
    read: GitStatusRead,
}

struct GitStatusWorker {
    jobs: SyncSender<GitStatusJob>,
    results: Receiver<GitStatusResult>,
}

impl GitStatusWorker {
    fn start() -> Self {
        let (jobs, job_rx) = mpsc::sync_channel::<GitStatusJob>(1);
        let (result_tx, results) = mpsc::sync_channel::<GitStatusResult>(4);
        thread::Builder::new()
            .name("made-git-status".to_owned())
            .spawn(move || {
                let runner = GitRunner::from_env();
                while let Ok(job) = job_rx.recv() {
                    let read = runner.read_status(&job.root, job.toplevel.as_deref());
                    if result_tx
                        .send(GitStatusResult {
                            generation: job.generation,
                            read,
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

    fn request(&self, job: GitStatusJob) -> bool {
        self.jobs.try_send(job).is_ok()
    }

    fn try_recv(&self) -> Option<GitStatusResult> {
        self.results.try_recv().ok()
    }
}

#[derive(Default)]
struct PendingGitStatusRequest {
    pending: Option<GitStatusJob>,
}

impl PendingGitStatusRequest {
    fn offer(&mut self, job: GitStatusJob) {
        self.pending = Some(job);
    }

    fn clear(&mut self) {
        self.pending = None;
    }

    fn try_send(&mut self, mut send: impl FnMut(&GitStatusJob) -> bool) -> bool {
        let Some(job) = self.pending.as_ref() else {
            return false;
        };
        if send(job) {
            self.pending = None;
            true
        } else {
            false
        }
    }
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

/// Stable operation name for diagnostics. `Operation` Debug is redacted.
fn operation_name(operation: &made_remote::protocol::Operation) -> &'static str {
    use made_remote::protocol::Operation;
    match operation {
        Operation::Attached => "Attached",
        Operation::Error { .. } => "Error",
        Operation::RootResult { .. } => "RootResult",
        Operation::TerminalOutput { .. } => "TerminalOutput",
        Operation::TerminalExit { .. } => "TerminalExit",
        Operation::SessionSnapshot { .. } => "SessionSnapshot",
        Operation::Changed { .. } => "Changed",
        Operation::Listed { .. } => "Listed",
        Operation::PreviewResult { .. } => "PreviewResult",
        Operation::GitResult { .. } => "GitResult",
        Operation::Ack => "Ack",
        Operation::Closed { .. } => "Closed",
        Operation::FocusResult { .. } => "FocusResult",
        Operation::UploadResult { .. } => "UploadResult",
        Operation::CapabilitiesResult { .. } => "CapabilitiesResult",
        Operation::RootValidate { .. } => "RootValidate",
        Operation::RootSelect { .. } => "RootSelect",
        Operation::List { .. } => "List",
        Operation::Preview { .. } => "Preview",
        Operation::Refresh { .. } => "Refresh",
        Operation::Watch { .. } => "Watch",
        Operation::Rescan { .. } => "Rescan",
        Operation::GitRead { .. } => "GitRead",
        Operation::SessionState => "SessionState",
        Operation::Heartbeat => "Heartbeat",
        Operation::Cancel { .. } => "Cancel",
        Operation::TerminalResize { .. } => "TerminalResize",
        _ => "other",
    }
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

    /// Terminates the stand-in child and skips the diagnostics-log write.
    struct GrokRunningProbe {
        workbench: Workbench,
    }

    impl Drop for GrokRunningProbe {
        fn drop(&mut self) {
            if let Some(mut terminal) = self.workbench.terminal.take() {
                terminal.terminate(self.workbench.diag.log_mut());
            }
            self.workbench.shutdown_done = true;
        }
    }

    impl GrokRunningProbe {
        /// Shell child, not grok. The running check only reads the target and
        /// whether that child is alive.
        fn with_living_shell() -> Self {
            let cwd = std::env::temp_dir();
            let platform = host_platform();
            let terminal = TerminalPane::spawn(&platform, made_terminal::PtyOptions::new(&cwd))
                .expect("stand-in shell");
            Self {
                workbench: idle_workbench(TerminalTarget::Shell, Some(terminal)),
            }
        }
    }

    impl std::ops::Deref for GrokRunningProbe {
        type Target = Workbench;

        fn deref(&self) -> &Workbench {
            &self.workbench
        }
    }

    impl std::ops::DerefMut for GrokRunningProbe {
        fn deref_mut(&mut self) -> &mut Workbench {
            &mut self.workbench
        }
    }

    fn idle_workbench(target: TerminalTarget, terminal: Option<TerminalPane>) -> Workbench {
        let cwd = std::env::temp_dir();
        let now = Instant::now();
        Workbench {
            platform: host_platform(),
            plan: None,
            workspace_error: None,
            terminal_cwd: cwd.clone(),
            terminal_target: target,
            paths: None,
            token: None,
            bridge: None,
            config_guard: None,
            config_failure: None,
            diag: Diagnostics::unavailable("grok running probe"),
            metrics: Metrics::new(),
            integration: IntegrationState::new(None, cwd),
            follow: None,
            session_map: SessionMap::new(),
            focus: SessionFocusController::new(),
            terminal,
            terminal_error: None,
            shell_cwd_error: None,
            explorer: None,
            worktree: WorktreeState::default(),
            worktree_worker: WorktreeWorker::start(),
            pending_worktree: PendingWorktreeRequest::default(),
            git_status_worker: GitStatusWorker::start(),
            pending_git_status: PendingGitStatusRequest::default(),
            git_status: GitStatusRuntime::default(),
            shell_capability: None,
            shell_root_worker: ShellRootWorker::start(),
            shell_root_unavailable: None,
            shell_root_check_in_flight: false,
            shell_root_checked_at: now,
            shell_root_check_interval: SHELL_ROOT_REVALIDATION_INTERVAL,
            grok_checked_at: now,
            grok_log_offset: 0,
            grok_sessions: Vec::new(),
            grok_closed: HashSet::new(),
            grok_provisional_id: None,
            grok_provisional_cwd: None,
            grok_restore_root: None,
            grok_file_newest_id: None,
            instrumentation: RuntimeInstrumentation::new(),
            pane_id: "grok-running-probe".to_owned(),
            visible: false,
            left_surface: LeftSurface::Terminal,
            explorer_visible: false,
            explorer_width: 0.0,
            sessions_visible: false,
            about_diagnostics_visible: false,
            about_open_generation: 0,
            grok_diagnostics: GrokDiagnosticsRow::NOT_ON_PATH,
            tick_count: 0,
            paint_count: 0,
            last_osc7_cwd: None,
            log_path: None,
            log_write_issue: None,
            started_at: now,
            startup_recorded: false,
            shutdown_done: false,
            frame_ctx: None,
            last_activity: now,
            last_frame: now,
            last_tick_active: false,
            benchmark_frames: false,
            pending_notice: None,
            last_title: None,
            remote: None,
            remote_preview: RemotePreviewView::default(),
            remote_explorer_cache: crate::explorer_view::RemoteExplorerCache::default(),
            remote_preview_focus_pending: false,
        }
    }

    fn hook_or_inventory_session(id: &str, opened_at: &str) -> GrokLiveSession {
        GrokLiveSession {
            id: id.to_owned(),
            cwd: PathBuf::from("/tmp/made-grok-other-workspace"),
            opened_at: opened_at.to_owned(),
            activity: SessionActivity::Idle,
            touched: Instant::now(),
        }
    }

    #[test]
    fn unconfirmed_hook_session_does_not_make_grok_follow_running() {
        // The signal is the living Grok target or an inventory-confirmed
        // session. It does not read the tab label or `workspace_grok_branded`.
        let mut probe = GrokRunningProbe::with_living_shell();
        assert!(
            probe
                .terminal
                .as_ref()
                .is_some_and(TerminalPane::is_running),
            "the stand-in child must be alive"
        );
        assert!(
            !probe.grok_follow_running(),
            "a shell with no Grok session is not running Grok"
        );

        probe.terminal_target = TerminalTarget::Tui(CompatibleTui::Grok);
        probe.last_title = Some("OpenCode".to_owned());
        assert!(
            probe.grok_follow_running(),
            "a living Grok target counts even when another label owns the title"
        );

        probe.terminal_target = TerminalTarget::Shell;
        probe
            .grok_sessions
            .push(hook_or_inventory_session("other-workspace", ""));
        assert!(
            !probe.grok_follow_running(),
            "a hook-only session with an empty opened_at is not confirmed for this terminal"
        );

        probe.grok_sessions.clear();
        if let Some(mut terminal) = probe.terminal.take() {
            terminal.terminate(probe.diag.log_mut());
        }
        assert!(probe
            .terminal
            .as_ref()
            .and_then(TerminalPane::pid)
            .is_none());
        probe
            .grok_sessions
            .push(hook_or_inventory_session("recorded-without-pid", ""));
        probe.terminal_target = TerminalTarget::Tui(CompatibleTui::Grok);
        assert!(
            !probe.grok_follow_running(),
            "a hook row recorded while this terminal has no pid must not count as running"
        );

        probe.grok_sessions.push(hook_or_inventory_session(
            "this-terminal",
            "2024-01-01T00:00:01Z",
        ));
        assert!(
            probe.grok_follow_running(),
            "inventory confirmation is a non-empty opened_at on this terminal's session"
        );

        probe
            .grok_sessions
            .retain(|session| session.opened_at.is_empty());
        probe.terminal_target = TerminalTarget::Shell;
        assert!(
            !probe.grok_follow_running(),
            "dropping the confirmed session must leave the hook-only row not running"
        );
    }

    #[test]
    fn workbench_drains_root_and_listing_ports() {
        use made_platform::ssh::SshEvent;
        use made_remote::protocol::{
            encode, DirectoryEntry, EntryKind, Identity, Operation, RemotePath, TabCapability,
            WireEnvelope,
        };
        use std::collections::VecDeque;
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct FakeStream {
            events: Arc<Mutex<VecDeque<SshEvent>>>,
            written: Arc<Mutex<Vec<u8>>>,
        }
        impl made_remote::client::ClientStream for FakeStream {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
                self.written.lock().unwrap().extend_from_slice(bytes);
                Ok(())
            }
            fn poll(&mut self) -> Option<SshEvent> {
                self.events.lock().unwrap().pop_front()
            }
            fn detach(&mut self) {}
        }
        fn take_frames(buffer: &mut Vec<u8>) -> Vec<WireEnvelope> {
            let mut frames = Vec::new();
            loop {
                if buffer.len() < 4 {
                    break;
                }
                let size = u32::from_be_bytes(buffer[..4].try_into().unwrap()) as usize;
                if buffer.len() < 4 + size {
                    break;
                }
                frames.push(serde_json::from_slice(&buffer[4..4 + size]).unwrap());
                buffer.drain(..4 + size);
            }
            frames
        }

        let directory = std::env::temp_dir().join(format!("made-wb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .unwrap();
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(&directory).unwrap();
        // Workbench::start refuses the real app log path in unit tests. Point
        // every app path at this temporary directory for the test lifetime.
        struct EnvGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                for (key, value) in self.0.drain(..) {
                    match value {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
        let home = directory.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let keys = [
            "HOME",
            "XDG_DATA_HOME",
            "XDG_CONFIG_HOME",
            "XDG_STATE_HOME",
            "XDG_CACHE_HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
        ];
        let _env = EnvGuard(
            keys.iter()
                .map(|key| (*key, std::env::var_os(key)))
                .collect(),
        );
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
        std::env::set_var("APPDATA", home.join("AppData").join("Roaming"));
        std::env::set_var("LOCALAPPDATA", home.join("AppData").join("Local"));
        std::env::set_var("XDG_DATA_HOME", home.join(".local/share"));
        std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
        std::env::set_var("XDG_STATE_HOME", home.join(".local/state"));
        std::env::set_var("XDG_CACHE_HOME", home.join(".cache"));
        let mut registry = crate::ssh_servers::ServerRegistry::load(
            // A Windows registry directory must carry the protected owner-only
            // DACL, which the store applies when it creates the directory.
            crate::ssh_servers::PrivateMetadataStorage::new(directory.join("registry")),
        )
        .unwrap();
        let authority = registry
            .add(crate::ssh_servers::ServerMetadata {
                display_name: "test".to_owned(),
                host: "example".to_owned(),
                user: Some("vscode".to_owned()),
                port: None,
                key_file: None,
                initial_directory: None,
            })
            .unwrap();
        let snapshot = registry.snapshot(authority).unwrap();
        let root = RemotePath::new(b"/home/vscode".to_vec()).unwrap();
        let identity = Identity {
            authority,
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        };
        let workspace =
            crate::remote_workspace::RemoteWorkspace::new(identity.clone(), root.clone()).unwrap();
        let mut workbench = Workbench::bind_remote_model(
            snapshot,
            workspace,
            TerminalTarget::RemoteShell { authority },
            "test",
        );
        let fake = FakeStream {
            events: Arc::new(Mutex::new(VecDeque::new())),
            written: Arc::new(Mutex::new(Vec::new())),
        };
        let client =
            made_remote::client::RemoteClient::new(fake.clone(), identity.clone(), root.clone())
                .unwrap();
        {
            let remote = workbench.remote.as_mut().unwrap();
            remote.client = Some(client);
            remote.attach_request = None;
            remote.phase = RemotePhase::Ready;
            remote.workspace.begin_resnapshot().unwrap();
        }

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut sent_root = false;
        let mut sent_list = false;
        while Instant::now() < deadline {
            workbench.pump_remote_client();
            let frames = {
                let mut buffer = fake.written.lock().unwrap();
                take_frames(&mut buffer)
            };
            for frame in frames {
                match frame.operation {
                    Operation::RootValidate { path } => {
                        let reply = WireEnvelope::new(
                            frame.identity.clone(),
                            frame.request_id,
                            Operation::RootResult {
                                requested: path.clone(),
                                path,
                                generation: 1,
                            },
                        );
                        fake.events
                            .lock()
                            .unwrap()
                            .push_back(SshEvent::Data(encode(&reply).unwrap()));
                        sent_root = true;
                    }
                    Operation::List { path } => {
                        let entry = DirectoryEntry {
                            path: RemotePath::new(b"/home/vscode/README.md".to_vec()).unwrap(),
                            kind: EntryKind::File,
                        };
                        let reply = WireEnvelope::new(
                            frame.identity.clone(),
                            frame.request_id,
                            Operation::Listed {
                                path,
                                entries: vec![entry],
                                truncated: false,
                                done: true,
                            },
                        );
                        fake.events
                            .lock()
                            .unwrap()
                            .push_back(SshEvent::Data(encode(&reply).unwrap()));
                        sent_list = true;
                    }
                    _ => {}
                }
            }
            if sent_root && sent_list {
                let remote = workbench.remote.as_ref().unwrap();
                let listed = remote
                    .workspace
                    .projection()
                    .directories
                    .get(&root)
                    .is_some_and(|directory| {
                        directory.entries.iter().any(|entry| {
                            entry.path
                                == RemotePath::new(b"/home/vscode/README.md".to_vec()).unwrap()
                        })
                    });
                if listed {
                    let _ = std::fs::remove_dir_all(&directory);
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = std::fs::remove_dir_all(&directory);
        panic!(
            "root/list port replies were not applied: sent_root={sent_root} sent_list={sent_list}"
        );
    }

    #[test]
    #[ignore = "requires MADE_E2E_PASSWORD and the local plantsuite fixture"]
    fn real_workbench_populates_the_remote_explorer() {
        use made_remote::protocol::{Identity, RemotePath, TabCapability};
        use std::time::Duration;

        let password = std::env::var("MADE_E2E_PASSWORD").expect("MADE_E2E_PASSWORD");
        let real_home = std::env::var_os("HOME").expect("HOME");
        let real_known = std::path::PathBuf::from(&real_home)
            .join("Library/Application Support/Made/ssh-known-hosts");
        let directory = std::env::temp_dir().join(format!("made-real-wb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .unwrap();
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(&directory).unwrap();
        let home = directory.join("home");
        let app_support = home.join("Library/Application Support/Made");
        std::fs::create_dir_all(&app_support).unwrap();
        if real_known.exists() {
            std::fs::copy(&real_known, app_support.join("ssh-known-hosts")).unwrap();
        }
        struct EnvGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                for (key, value) in self.0.drain(..) {
                    match value {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
        let keys = [
            "HOME",
            "XDG_DATA_HOME",
            "XDG_CONFIG_HOME",
            "XDG_STATE_HOME",
            "XDG_CACHE_HOME",
        ];
        let _env = EnvGuard(
            keys.iter()
                .map(|key| (*key, std::env::var_os(key)))
                .collect(),
        );
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", home.join(".local/share"));
        std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
        std::env::set_var("XDG_STATE_HOME", home.join(".local/state"));
        std::env::set_var("XDG_CACHE_HOME", home.join(".cache"));

        let mut registry = crate::ssh_servers::ServerRegistry::load(
            crate::ssh_servers::PrivateMetadataStorage::new(&directory),
        )
        .unwrap();
        let root = RemotePath::new(b"/home/vscode".to_vec()).unwrap();
        let authority = registry
            .add(crate::ssh_servers::ServerMetadata {
                display_name: "e2e".to_owned(),
                host: "plantsuite.container".to_owned(),
                user: Some("vscode".to_owned()),
                port: Some(2222),
                key_file: None,
                initial_directory: Some(root.clone()),
            })
            .unwrap();
        let snapshot = registry.snapshot(authority).unwrap();
        let identity = Identity {
            authority,
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        };
        let workspace =
            crate::remote_workspace::RemoteWorkspace::new(identity.clone(), root.clone()).unwrap();
        let mut workbench = Workbench::bind_remote_model(
            snapshot.clone(),
            workspace,
            TerminalTarget::RemoteShell { authority },
            "e2e",
        );
        let session = crate::ssh_servers::connect_companion(
            snapshot.server().metadata(),
            crate::APP_VERSION,
            "aarch64-unknown-linux-gnu",
            identity,
            root.clone(),
        )
        .expect("companion");
        let client = session.client.clone();
        {
            let remote = workbench.remote.as_mut().unwrap();
            remote.client = Some(client.clone());
            remote.failures = session.failures;
        }
        let open = client
            .request(
                crate::remote_session::first_operation(
                    crate::remote_session::SessionOpen::New,
                    root.clone(),
                    made_remote::protocol::LaunchTarget::Shell,
                    24,
                    80,
                ),
                None,
            )
            .unwrap();
        workbench.remote.as_mut().unwrap().attach_request = Some(open);

        let deadline = Instant::now() + Duration::from_secs(20);
        let mut attached = false;
        let mut listed = false;
        while Instant::now() < deadline {
            if let Some(prompt) = client.poll_prompt() {
                let answer =
                    made_platform::ssh_askpass::PromptAnswer::new(password.as_bytes().to_vec())
                        .ok();
                let _ = client.answer_prompt(prompt.id, answer);
            }
            workbench.pump_remote_client();
            workbench.pump_remote_terminal();
            let remote = workbench.remote.as_ref().unwrap();
            attached |= remote.attached_once;
            if attached {
                if let Some(directory) = remote.workspace.projection().directories.get(&root) {
                    if !directory.entries.is_empty() {
                        listed = true;
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = client.request(made_remote::protocol::Operation::Close, None);
        std::thread::sleep(Duration::from_millis(100));
        client.disconnect();
        let _ = std::fs::remove_dir_all(&directory);
        assert!(attached, "workbench never attached");
        assert!(listed, "workbench never applied the root listing");
    }

    #[test]
    fn hidden_remote_output_does_not_schedule_a_repaint() {
        assert_eq!(Workbench::remote_output_repaint_delay(false), None);
        assert_eq!(
            Workbench::remote_output_repaint_delay(true),
            Some(crate::metrics::ACTIVE_REPAINT_INTERVAL)
        );
    }

    fn ui_text(output: &egui::FullOutput) -> String {
        fn visit(shape: &egui::Shape, text: &mut String) {
            match shape {
                egui::Shape::Text(text_shape) => {
                    text.push_str(&text_shape.galley.job.text);
                    text.push('\n');
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        visit(shape, text);
                    }
                }
                _ => {}
            }
        }
        let mut text = String::new();
        for clipped in &output.shapes {
            visit(&clipped.shape, &mut text);
        }
        text
    }

    #[test]
    #[ignore = "requires the plantsuite container fixture"]
    fn real_container_remote_ui_uses_local_surfaces() {
        use made_remote::protocol::{Identity, Operation, RemotePath, TabCapability};
        use std::time::Duration;

        let password =
            std::env::var("MADE_FIXTURE_PASSWORD").unwrap_or_else(|_| "plantsuite".to_owned());
        let real_home = std::env::var_os("HOME").expect("HOME");
        let real_known = std::path::PathBuf::from(&real_home)
            .join("Library/Application Support/Made/ssh-known-hosts");
        let directory =
            std::env::temp_dir().join(format!("made-real-remote-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .unwrap();
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(&directory).unwrap();
        let home = directory.join("home");
        let app_support = home.join("Library/Application Support/Made");
        std::fs::create_dir_all(&app_support).unwrap();
        if real_known.exists() {
            std::fs::copy(&real_known, app_support.join("ssh-known-hosts")).unwrap();
        }
        struct EnvGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                for (key, value) in self.0.drain(..) {
                    match value {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
        let keys = [
            "HOME",
            "XDG_DATA_HOME",
            "XDG_CONFIG_HOME",
            "XDG_STATE_HOME",
            "XDG_CACHE_HOME",
        ];
        let _env = EnvGuard(
            keys.iter()
                .map(|key| (*key, std::env::var_os(key)))
                .collect(),
        );
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", home.join(".local/share"));
        std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
        std::env::set_var("XDG_STATE_HOME", home.join(".local/state"));
        std::env::set_var("XDG_CACHE_HOME", home.join(".cache"));

        let mut registry = crate::ssh_servers::ServerRegistry::load(
            crate::ssh_servers::PrivateMetadataStorage::new(&directory),
        )
        .unwrap();
        let root = RemotePath::new(b"/home/vscode".to_vec()).unwrap();
        let plant = RemotePath::new(b"/workspaces/PlantSuite".to_vec()).unwrap();
        let readme = RemotePath::new(b"/workspaces/PlantSuite/README.md".to_vec()).unwrap();
        let authority = registry
            .add(crate::ssh_servers::ServerMetadata {
                display_name: "e2e".to_owned(),
                host: "plantsuite.container".to_owned(),
                user: Some("vscode".to_owned()),
                port: Some(2222),
                key_file: None,
                initial_directory: Some(root.clone()),
            })
            .unwrap();
        let snapshot = registry.snapshot(authority).unwrap();
        let identity = Identity {
            authority,
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        };
        let workspace =
            crate::remote_workspace::RemoteWorkspace::new(identity.clone(), root.clone()).unwrap();
        let mut workbench = Workbench::bind_remote_model(
            snapshot.clone(),
            workspace,
            TerminalTarget::RemoteShell { authority },
            "e2e",
        );
        let session = crate::ssh_servers::connect_companion(
            snapshot.server().metadata(),
            crate::APP_VERSION,
            "aarch64-unknown-linux-gnu",
            identity,
            root.clone(),
        )
        .expect("companion");
        let client = session.client.clone();
        {
            let remote = workbench.remote.as_mut().unwrap();
            remote.client = Some(client.clone());
            remote.failures = session.failures;
        }
        let open = client
            .request(
                crate::remote_session::first_operation(
                    crate::remote_session::SessionOpen::New,
                    root.clone(),
                    made_remote::protocol::LaunchTarget::Shell,
                    24,
                    80,
                ),
                None,
            )
            .unwrap();
        workbench.remote.as_mut().unwrap().attach_request = Some(open);

        let deadline = Instant::now() + Duration::from_secs(30);
        let mut attached = false;
        let mut listed = false;
        while Instant::now() < deadline {
            if let Some(prompt) = client.poll_prompt() {
                let answer =
                    made_platform::ssh_askpass::PromptAnswer::new(password.as_bytes().to_vec())
                        .ok();
                let _ = client.answer_prompt(prompt.id, answer);
            }
            workbench.pump_remote_client();
            workbench.pump_remote_terminal();
            let remote = workbench.remote.as_ref().unwrap();
            attached |= remote.attached_once;
            if attached {
                if let Some(directory) = remote.workspace.projection().directories.get(&root) {
                    if !directory.entries.is_empty() {
                        listed = true;
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(attached, "workbench never attached");
        assert!(listed, "workbench never applied the root listing");

        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let layout = crate::theme::Layout::for_window(1280.0, 800.0);
        let scope = egui::Id::unique("real-remote-explorer");
        let projection = workbench
            .remote
            .as_ref()
            .unwrap()
            .workspace
            .projection()
            .clone();
        let entries = projection
            .directories
            .get(&root)
            .map(|directory| directory.entries.len())
            .unwrap_or(0);
        assert!(
            entries >= 20,
            "expected a populated /home/vscode, observed {entries} entries"
        );
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let mut explorer_cache = crate::explorer_view::RemoteExplorerCache::default();
        let mut output = ctx.run_ui(raw, |ui| {
            let _ = crate::explorer_view::remote_explorer_ui(
                ui,
                &root,
                &projection,
                &layout,
                crate::explorer_view::RemoteExplorerOptions {
                    scope,
                    native_backdrop: false,
                    allow_root_retry: true,
                },
                &mut explorer_cache,
            );
        });
        output.textures_delta.clear();
        let text = ui_text(&output);
        assert!(text.contains("EXPLORER"), "missing local explorer title");
        assert!(text.contains("vscode"), "missing root name");
        for forbidden in [
            "SSH connected",
            "SSH disconnected",
            "integration unknown",
            "Files",
            "Source Control",
            "Preview",
        ] {
            assert!(
                !text.contains(forbidden),
                "remote explorer painted SSH chrome {forbidden:?}: {text:?}"
            );
        }
        let painted_rows = (0..entries)
            .filter(|index| {
                ctx.read_response(scope.with(("tree-row", *index)))
                    .is_some()
            })
            .count();
        assert!(
            painted_rows >= 20 && painted_rows <= entries,
            "expected a populated virtual row window, painted {painted_rows} of {entries}"
        );
        eprintln!(
            "real container: explorer root={} entries={entries} painted_rows={painted_rows}",
            root.to_utf8().unwrap_or("<non-utf8>")
        );

        workbench
            .remote
            .as_mut()
            .unwrap()
            .workspace
            .select_root(plant.clone())
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut plant_listed = false;
        let mut git_ready = false;
        while Instant::now() < deadline {
            if let Some(prompt) = client.poll_prompt() {
                let answer =
                    made_platform::ssh_askpass::PromptAnswer::new(password.as_bytes().to_vec())
                        .ok();
                let _ = client.answer_prompt(prompt.id, answer);
            }
            workbench.pump_remote_client();
            workbench.pump_remote_terminal();
            let remote = workbench.remote.as_ref().unwrap();
            plant_listed |= remote
                .workspace
                .projection()
                .directories
                .get(&plant)
                .is_some_and(|directory| !directory.entries.is_empty());
            git_ready |= remote
                .workspace
                .projection()
                .git_view
                .as_ref()
                .is_some_and(|git| git.freshness == crate::remote_workspace::Freshness::Current);
            if plant_listed && git_ready {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            plant_listed,
            "workbench never listed /workspaces/PlantSuite"
        );
        assert!(git_ready, "workbench never read PlantSuite git status");
        let remote = workbench.remote.as_ref().unwrap();
        let git = remote
            .workspace
            .projection()
            .git_view
            .as_ref()
            .expect("git view");
        let branch = git.branch.as_deref().unwrap_or("<detached>");
        assert!(!branch.is_empty(), "git view must show a head line");
        let status = if git.groups.is_empty() {
            "No changes".to_owned()
        } else {
            git.groups
                .iter()
                .map(|group| format!("{} ({})", group.title, group.rows.len()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        eprintln!(
            "real container: git root={} branch={branch} status={status}",
            plant.to_utf8().unwrap_or("<non-utf8>"),
        );
        let mut git_projection = remote.workspace.projection().clone();
        git_projection.mode = crate::remote_workspace::ExplorerMode::Git;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let mut git_explorer_cache = crate::explorer_view::RemoteExplorerCache::default();
        let mut output = ctx.run_ui(raw, |ui| {
            let _ = crate::explorer_view::remote_explorer_ui(
                ui,
                &plant,
                &git_projection,
                &layout,
                crate::explorer_view::RemoteExplorerOptions {
                    scope,
                    native_backdrop: false,
                    allow_root_retry: true,
                },
                &mut git_explorer_cache,
            );
        });
        output.textures_delta.clear();
        let text = ui_text(&output);
        assert!(text.contains("REPOS"), "missing local REPOS title");
        assert!(text.contains(branch), "missing branch line");
        assert!(
            text.contains("No changes") || text.contains("Changes"),
            "missing git status state: {text:?}"
        );
        for forbidden in ["SSH", "integration unknown", "Files", "Source Control"] {
            assert!(
                !text.contains(forbidden),
                "remote source control painted SSH chrome {forbidden:?}: {text:?}"
            );
        }

        workbench.apply_remote_actions(
            &egui::Context::default(),
            vec![crate::explorer_view::RemoteExplorerAction::OpenPreview(
                readme.clone(),
            )],
        );
        assert_eq!(
            workbench.left_surface(),
            LeftSurface::Preview,
            "opening a remote file must use the full-left preview surface"
        );
        assert!(workbench.remote_has_preview());
        assert!(workbench.remote_preview_focus_pending);
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut preview_ready = false;
        while Instant::now() < deadline {
            if let Some(prompt) = client.poll_prompt() {
                let answer =
                    made_platform::ssh_askpass::PromptAnswer::new(password.as_bytes().to_vec())
                        .ok();
                let _ = client.answer_prompt(prompt.id, answer);
            }
            workbench.pump_remote_client();
            workbench.pump_remote_terminal();
            preview_ready |= workbench
                .remote
                .as_ref()
                .unwrap()
                .workspace
                .projection()
                .preview
                .as_ref()
                .is_some_and(|preview| {
                    preview.freshness == crate::remote_workspace::Freshness::Current
                });
            if preview_ready {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(preview_ready, "workbench never loaded README.md preview");
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut last_output = None;
        while Instant::now() < deadline {
            let mut output = ctx.run_ui(raw.clone(), |ui| {
                let action = workbench.remote_preview_ui(ui, &layout);
                assert_eq!(action, PreviewUiAction::None);
            });
            output.textures_delta.clear();
            if workbench.remote_preview.document.is_some() {
                last_output = Some(output);
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let output = last_output.expect("remote preview document worker");
        let text = ui_text(&output);
        for expected in ["PREVIEW", "README.md", "read-only", "Copy path", "Close"] {
            assert!(
                text.contains(expected),
                "remote preview missing {expected:?}: {text:?}"
            );
        }
        assert!(!text.contains("SSH"), "remote preview painted SSH chrome");
        assert_eq!(
            workbench.left_surface(),
            LeftSurface::Preview,
            "rendering the preview must keep the left surface"
        );
        let document = workbench
            .remote_preview
            .document
            .as_ref()
            .expect("remote preview document");
        assert_eq!(document.size, Some(7824), "unexpected README.md size");
        let preview_path = workbench
            .remote
            .as_ref()
            .unwrap()
            .workspace
            .projection()
            .preview
            .as_ref()
            .expect("preview")
            .path
            .clone();
        eprintln!(
            "real container: preview path={} bytes={} lines={} surface=full-left",
            preview_path.to_utf8().unwrap_or("<non-utf8>"),
            document.size.unwrap_or(0),
            document.line_count()
        );

        let _ = client.request(Operation::Close, None);
        std::thread::sleep(Duration::from_millis(100));
        client.disconnect();
        let _ = std::fs::remove_dir_all(&directory);
    }
    #[test]
    fn remote_preview_worker_keeps_highlighting_off_the_ui_path() {
        use made_remote::protocol::{PreviewKind, RemotePath};
        use std::time::Duration;

        let path = RemotePath::new(b"/project/large.txt".to_vec()).unwrap();
        let data = b"line of remote preview text\n".repeat(40_000);
        let key = RemotePreviewKey {
            path: path.clone(),
            freshness: crate::remote_workspace::Freshness::Current,
            len: data.len(),
            revision: 1,
        };
        let mut view = RemotePreviewView {
            key: Some(key.clone()),
            ..RemotePreviewView::default()
        };
        assert!(view.request(key, path, PreviewKind::Text, data.clone()));
        assert!(
            view.document.is_none(),
            "requesting a remote preview must not build it on the UI path"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while view.document.is_none() && Instant::now() < deadline {
            view.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        let document = view.document.expect("worker document");
        assert_eq!(document.size, Some(data.len() as u64));
        assert!(!document.lines.is_empty());
    }

    #[test]
    fn remote_preview_revision_invalidates_same_length_content_while_hidden() {
        use made_remote::protocol::{PreviewKind, RemotePath};
        use std::time::Duration;

        let path = RemotePath::new(b"/project/file.txt".to_vec()).unwrap();
        let key = |revision| RemotePreviewKey {
            path: path.clone(),
            freshness: crate::remote_workspace::Freshness::Current,
            len: 4,
            revision,
        };
        let mut view = RemotePreviewView {
            key: Some(key(1)),
            ..RemotePreviewView::default()
        };
        assert!(view.request(key(1), path.clone(), PreviewKind::Text, b"aaaa".to_vec()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while view.document.is_none() && Instant::now() < deadline {
            view.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            crate::explorer_view::document_text(view.document.as_ref().unwrap()),
            "aaaa"
        );

        // A hidden same-length refresh bumps the revision; the old document
        // stays until the replacement finishes.
        view.key = Some(key(2));
        assert!(view.request(key(2), path, PreviewKind::Text, b"bbbb".to_vec()));
        assert_eq!(
            crate::explorer_view::document_text(view.document.as_ref().unwrap()),
            "aaaa"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while view
            .document
            .as_ref()
            .is_some_and(|document| crate::explorer_view::document_text(document) != "bbbb")
            && Instant::now() < deadline
        {
            view.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            crate::explorer_view::document_text(view.document.as_ref().unwrap()),
            "bbbb"
        );
    }
    mod unsolicited_remote_snapshots {
        use super::*;
        use crate::remote_workspace::{IntegrationState as RemoteIntegrationState, RootSource};
        use made_platform::ssh::SshEvent;
        use made_remote::protocol::{
            encode, Chunk, DirectoryEntry, EntryKind, ErrorCode, GrokActivity, GrokEventKind,
            GrokSessionEvent, GrokSessionInfo, GrokSessionSnapshot, Identity, LaunchTarget,
            Operation, RemotePath, WireEnvelope,
        };
        use std::collections::VecDeque;

        #[derive(Clone)]
        struct FakeRemoteStream {
            events: Arc<Mutex<VecDeque<SshEvent>>>,
            written: Arc<Mutex<Vec<u8>>>,
        }

        impl made_remote::client::ClientStream for FakeRemoteStream {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
                self.written.lock().unwrap().extend_from_slice(bytes);
                Ok(())
            }
            fn poll(&mut self) -> Option<SshEvent> {
                self.events.lock().unwrap().pop_front()
            }
            fn detach(&mut self) {}
        }

        struct MemoryServerStorage(Option<Vec<u8>>);

        impl crate::ssh_servers::ServerStorage for MemoryServerStorage {
            fn read(
                &mut self,
                _limit: usize,
            ) -> Result<Option<Vec<u8>>, crate::ssh_servers::StorageError> {
                Ok(self.0.clone())
            }
            fn replace(&mut self, bytes: &[u8]) -> Result<(), crate::ssh_servers::StorageError> {
                self.0 = Some(bytes.to_vec());
                Ok(())
            }
        }

        struct RemoteBench {
            workbench: Workbench,
            stream: FakeRemoteStream,
            identity: Identity,
            root: RemotePath,
        }

        impl RemoteBench {
            fn send(&self, frame: WireEnvelope) {
                self.stream
                    .events
                    .lock()
                    .unwrap()
                    .push_back(SshEvent::Data(encode(&frame).unwrap()));
            }
            fn reply(&self, request: &WireEnvelope, operation: Operation) {
                self.send(WireEnvelope::new(
                    request.identity.clone(),
                    request.request_id,
                    operation,
                ));
            }
            fn pump(&mut self) {
                self.workbench.pump_remote_client();
                self.workbench.pump_remote_terminal();
            }
            fn take_written(&self) -> Vec<WireEnvelope> {
                let mut buffer = self.stream.written.lock().unwrap();
                let mut frames = Vec::new();
                loop {
                    if buffer.len() < 4 {
                        break;
                    }
                    let size = u32::from_be_bytes(buffer[..4].try_into().unwrap()) as usize;
                    if buffer.len() < 4 + size {
                        break;
                    }
                    frames.push(serde_json::from_slice(&buffer[4..4 + size]).unwrap());
                    buffer.drain(..4 + size);
                }
                frames
            }
            fn attach(&mut self) -> u64 {
                let client = self
                    .workbench
                    .remote
                    .as_ref()
                    .unwrap()
                    .client
                    .clone()
                    .unwrap();
                let id = client
                    .request(
                        Operation::Open {
                            directory: self.root.clone(),
                            target: LaunchTarget::Shell,
                            rows: 24,
                            cols: 80,
                        },
                        None,
                    )
                    .unwrap();
                self.workbench.remote.as_mut().unwrap().attach_request = Some(id);
                id
            }
        }

        fn remote_path(value: &str) -> RemotePath {
            RemotePath::new(value.as_bytes().to_vec()).unwrap()
        }

        fn file_entry(path: &RemotePath) -> DirectoryEntry {
            DirectoryEntry {
                path: path.clone(),
                kind: EntryKind::File,
            }
        }

        fn active_snapshot(revision: u64, directory: &str) -> SessionSnapshot {
            serde_json::from_value(serde_json::json!({
                "revision": revision,
                "activeSessionID": "one",
                "tabsEnabled": true,
                "sessions": [{
                    "sessionID": "one",
                    "title": "Remote",
                    "active": true,
                    "directory": directory
                }]
            }))
            .unwrap()
        }

        fn empty_snapshot(revision: u64) -> SessionSnapshot {
            SessionSnapshot {
                revision,
                active_session_id: None,
                tabs_enabled: true,
                view_directory: None,
                sessions: Vec::new(),
            }
        }

        fn remote_bench() -> RemoteBench {
            let mut registry =
                crate::ssh_servers::ServerRegistry::load(MemoryServerStorage(None)).unwrap();
            let authority = registry
                .add(crate::ssh_servers::ServerMetadata {
                    display_name: "test".to_owned(),
                    host: "example".to_owned(),
                    user: Some("vscode".to_owned()),
                    port: None,
                    key_file: None,
                    initial_directory: None,
                })
                .unwrap();
            let snapshot = registry.snapshot(authority).unwrap();
            let root = remote_path("/home/vscode");
            let identity = Identity {
                authority,
                tab: made_remote::protocol::TabCapability::random().unwrap(),
                epoch: 1,
                generation: 1,
            };
            let workspace =
                crate::remote_workspace::RemoteWorkspace::new(identity.clone(), root.clone())
                    .unwrap();
            let mut workbench = idle_workbench(TerminalTarget::RemoteShell { authority }, None);
            let mut remote = RemoteTab::connecting(snapshot, root.clone()).unwrap();
            remote.workspace = workspace;
            remote.phase = RemotePhase::Ready;
            workbench.remote = Some(remote);
            let stream = FakeRemoteStream {
                events: Arc::new(Mutex::new(VecDeque::new())),
                written: Arc::new(Mutex::new(Vec::new())),
            };
            let client = made_remote::client::RemoteClient::new(
                stream.clone(),
                identity.clone(),
                root.clone(),
            )
            .unwrap();
            workbench.remote.as_mut().unwrap().client = Some(client);
            RemoteBench {
                workbench,
                stream,
                identity,
                root,
            }
        }

        fn pump_until(
            bench: &mut RemoteBench,
            mut handle: impl FnMut(&mut RemoteBench, Option<WireEnvelope>) -> bool,
        ) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                bench.pump();
                for frame in bench.take_written() {
                    if handle(bench, Some(frame)) {
                        return;
                    }
                }
                if handle(bench, None) {
                    return;
                }
                thread::sleep(Duration::from_millis(5));
            }
            panic!("remote workbench did not reach the expected state");
        }

        fn listed(path: &RemotePath) -> Operation {
            let child = RemotePath::new([path.as_bytes(), b"/README.md"].concat()).unwrap();
            Operation::Listed {
                path: path.clone(),
                entries: vec![file_entry(&child)],
                truncated: false,
                done: true,
            }
        }

        fn root_result(frame: &WireEnvelope, path: &RemotePath) -> Operation {
            Operation::RootResult {
                requested: path.clone(),
                path: path.clone(),
                generation: frame.identity.generation,
            }
        }

        /// Pump until the attach reply has been processed. Session snapshots
        /// are sent only after this, matching the companion's post-attach
        /// ordering. The bench workspace starts pre-connected, so an event
        /// arriving in the same batch as the attach reply would otherwise be
        /// wiped by the attach resnapshot; a real tab starts disconnected and
        /// rejects pre-attach events outright.
        fn wait_for_attach(bench: &mut RemoteBench) {
            pump_until(bench, |bench, _| {
                bench
                    .workbench
                    .remote
                    .as_ref()
                    .is_some_and(|remote| remote.attach_request.is_none() && remote.attached_once)
            });
        }

        fn assert_terminal_continues(bench: &RemoteBench, output: u64) {
            let remote = bench.workbench.remote.as_ref().unwrap();
            assert!(remote.attached_once, "attach marker must survive a switch");
            assert!(
                matches!(remote.phase, RemotePhase::Ready),
                "remote phase must stay Ready across a switch"
            );
            assert!(
                remote.attach_request.is_none(),
                "switching must not start a new attach"
            );
            assert_eq!(
                bench.workbench.terminal.as_ref().unwrap().output_bytes(),
                output,
                "the attached terminal must retain its output state"
            );
        }

        fn follow_snapshot(
            bench: &mut RemoteBench,
            attach: u64,
            snapshot: SessionSnapshot,
            session_root: &RemotePath,
        ) {
            let identity = bench
                .workbench
                .remote
                .as_ref()
                .unwrap()
                .client
                .as_ref()
                .unwrap()
                .identity();
            bench.send(WireEnvelope::new(
                identity,
                attach,
                Operation::SessionSnapshot { snapshot },
            ));
            let mut selected = false;
            pump_until(bench, |bench, frame| {
                if let Some(frame) = frame {
                    match &frame.operation {
                        Operation::RootSelect { path } => {
                            assert_eq!(path, session_root);
                            selected = true;
                            let mut result = root_result(&frame, path);
                            if let Operation::RootResult { generation, .. } = &mut result {
                                *generation += 1;
                            }
                            bench.reply(&frame, result);
                        }
                        Operation::List { path } if path == session_root => {
                            bench.reply(&frame, listed(path))
                        }
                        _ => {}
                    }
                }
                let remote = bench.workbench.remote.as_ref().unwrap();
                selected
                    && remote.workspace.root() == session_root
                    && remote
                        .workspace
                        .projection()
                        .directories
                        .contains_key(session_root)
            });
        }

        #[test]
        fn unsolicited_snapshot_after_unavailable_session_connects_and_follows_session_root() {
            let mut bench = remote_bench();
            let attach = bench.attach();
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::Attached,
            ));
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::TerminalOutput {
                    data: Chunk::new(b"terminal-live".to_vec()).unwrap(),
                    done: true,
                },
            ));

            let root = bench.root.clone();
            let mut session_unavailable = false;
            pump_until(&mut bench, |bench, frame| {
                if let Some(frame) = frame {
                    match &frame.operation {
                        Operation::RootValidate { path } => {
                            bench.reply(&frame, root_result(&frame, path))
                        }
                        Operation::List { path } => bench.reply(&frame, listed(path)),
                        Operation::SessionState => {
                            session_unavailable = true;
                            bench.reply(
                                &frame,
                                Operation::Error {
                                    code: ErrorCode::Unavailable,
                                },
                            );
                        }
                        _ => {}
                    }
                }
                let remote = bench.workbench.remote.as_ref().unwrap();
                session_unavailable
                    && remote.workspace.integration() == RemoteIntegrationState::Unavailable
                    && remote
                        .workspace
                        .projection()
                        .directories
                        .contains_key(&root)
                    && bench
                        .workbench
                        .terminal
                        .as_ref()
                        .is_some_and(|terminal| terminal.output_bytes() > 0)
            });
            let terminal_output = bench.workbench.terminal.as_ref().unwrap().output_bytes();

            let session_root = remote_path("/session");
            follow_snapshot(
                &mut bench,
                attach,
                active_snapshot(1, "/session"),
                &session_root,
            );

            let remote = bench.workbench.remote.as_ref().unwrap();
            assert_eq!(
                remote.workspace.integration(),
                RemoteIntegrationState::Connected
            );
            assert_eq!(
                remote.workspace.projection().source,
                crate::remote_workspace::RootSource::Session
            );
            assert!(remote.workspace.projection().sessions.is_some());
            assert_eq!(remote.workspace.root(), &session_root);
            assert_terminal_continues(&bench, terminal_output);

            // A later unsolicited snapshot reuses the original attach id even
            // after the first RootSelect advanced the client discard window.
            let switched_root = remote_path("/session-two");
            follow_snapshot(
                &mut bench,
                attach,
                active_snapshot(2, "/session-two"),
                &switched_root,
            );
            let remote = bench.workbench.remote.as_ref().unwrap();
            assert_eq!(remote.workspace.root(), &switched_root);
            assert_eq!(
                remote
                    .workspace
                    .projection()
                    .sessions
                    .as_ref()
                    .unwrap()
                    .revision,
                2
            );
            assert_terminal_continues(&bench, terminal_output);

            let final_root = remote_path("/session-three");
            follow_snapshot(
                &mut bench,
                attach,
                active_snapshot(3, "/session-three"),
                &final_root,
            );
            let remote = bench.workbench.remote.as_ref().unwrap();
            assert_eq!(remote.workspace.root(), &final_root);
            assert_eq!(
                remote
                    .workspace
                    .projection()
                    .sessions
                    .as_ref()
                    .unwrap()
                    .revision,
                3
            );
            assert_terminal_continues(&bench, terminal_output);
        }

        #[test]
        fn shell_cwd_report_moves_the_remote_root_after_cd() {
            let mut bench = remote_bench();
            let attach = bench.attach();
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::Attached,
            ));
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::TerminalOutput {
                    data: Chunk::new(b"$ ".to_vec()).unwrap(),
                    done: true,
                },
            ));
            let root = bench.root.clone();
            pump_until(&mut bench, |bench, frame| {
                if let Some(frame) = frame {
                    match &frame.operation {
                        Operation::RootValidate { path } => {
                            bench.reply(&frame, root_result(&frame, path))
                        }
                        Operation::List { path } => bench.reply(&frame, listed(path)),
                        Operation::SessionState => bench.reply(
                            &frame,
                            Operation::Error {
                                code: ErrorCode::Unavailable,
                            },
                        ),
                        _ => {}
                    }
                }
                bench
                    .workbench
                    .remote
                    .as_ref()
                    .unwrap()
                    .workspace
                    .projection()
                    .directories
                    .contains_key(&root)
            });

            // The companion reports the shell's cwd after `cd`, as raw POSIX
            // bytes: spaces and non-ASCII must survive untouched.
            let target = remote_path("/home/vscode/dir com espaço/ação 日本");
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::ShellCwd {
                    path: target.clone(),
                },
            ));
            let mut validated = false;
            pump_until(&mut bench, |bench, frame| {
                if let Some(frame) = frame {
                    match &frame.operation {
                        Operation::RootValidate { path } | Operation::RootSelect { path } => {
                            assert_eq!(path, &target);
                            validated = true;
                            let mut result = root_result(&frame, path);
                            if let Operation::RootResult { generation, .. } = &mut result {
                                if matches!(frame.operation, Operation::RootSelect { .. }) {
                                    *generation += 1;
                                }
                            }
                            bench.reply(&frame, result);
                        }
                        Operation::List { path } => bench.reply(&frame, listed(path)),
                        _ => {}
                    }
                }
                let remote = bench.workbench.remote.as_ref().unwrap();
                validated
                    && remote.workspace.root() == &target
                    && remote.workspace.projection().source
                        == crate::remote_workspace::RootSource::Shell
                    && remote
                        .workspace
                        .projection()
                        .directories
                        .contains_key(&target)
            });
        }

        #[test]
        fn attach_id_colliding_with_pending_root_validate_is_applied_as_event() {
            let mut bench = remote_bench();
            let attach = bench.attach();
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::Attached,
            ));

            let mut root_validate_pending = false;
            pump_until(&mut bench, |bench, frame| {
                if let Some(frame) = frame {
                    root_validate_pending |=
                        matches!(frame.operation, Operation::RootValidate { .. });
                }
                root_validate_pending
                    && bench
                        .workbench
                        .remote
                        .as_ref()
                        .unwrap()
                        .workspace
                        .pending_count()
                        == 1
            });
            assert_eq!(attach, 1, "attach request id");
            assert_eq!(
                bench
                    .workbench
                    .remote
                    .as_ref()
                    .unwrap()
                    .workspace
                    .pending_count(),
                1,
                "RootValidate must still be pending when the event arrives"
            );

            let session_root = remote_path("/session");
            follow_snapshot(
                &mut bench,
                attach,
                active_snapshot(1, "/session"),
                &session_root,
            );

            let remote = bench.workbench.remote.as_ref().unwrap();
            assert_eq!(
                remote.workspace.integration(),
                RemoteIntegrationState::Connected
            );
            assert!(remote.workspace.projection().sessions.is_some());
        }

        #[test]
        fn unsolicited_snapshot_does_not_consume_numeric_equal_pending_request() {
            let mut bench = remote_bench();
            let client = bench
                .workbench
                .remote
                .as_ref()
                .unwrap()
                .client
                .clone()
                .unwrap();
            // Synthetic id alignment: production's first request is Open/Attach.
            // Consume client id 1 here so the attach id can collide with the
            // pending List workspace id in this adversarial test.
            let _ = client.request(Operation::Capabilities, None).unwrap();
            let attach = bench.attach();
            assert_eq!(attach, 2, "attach request id");
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::Attached,
            ));

            let mut list_request = None;
            pump_until(&mut bench, |bench, frame| {
                if let Some(frame) = frame {
                    match &frame.operation {
                        Operation::RootValidate { path } => {
                            bench.reply(&frame, root_result(&frame, path))
                        }
                        Operation::List { .. } if list_request.is_none() => {
                            list_request = Some(frame.clone());
                        }
                        _ => {}
                    }
                }
                list_request.is_some()
            });
            let list_request = list_request.unwrap();
            let pending_before = bench
                .workbench
                .remote
                .as_ref()
                .unwrap()
                .workspace
                .pending_count();
            assert!(pending_before > 0);

            // The attach id is 2 and the pending List workspace id is also 2.
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::SessionSnapshot {
                    snapshot: empty_snapshot(1),
                },
            ));
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                bench.pump();
                if bench
                    .workbench
                    .remote
                    .as_ref()
                    .unwrap()
                    .workspace
                    .integration()
                    == RemoteIntegrationState::Connected
                {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            let remote = bench.workbench.remote.as_ref().unwrap();
            assert_eq!(
                remote.workspace.integration(),
                RemoteIntegrationState::Connected,
                "unsolicited snapshot must be applied as an event"
            );
            assert_eq!(
                remote.workspace.pending_count(),
                pending_before,
                "an event must not consume a numeric-equal pending request"
            );

            bench.reply(&list_request, listed(&bench.root));
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                bench.pump();
                if bench
                    .workbench
                    .remote
                    .as_ref()
                    .unwrap()
                    .workspace
                    .projection()
                    .directories
                    .contains_key(&bench.root)
                {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            assert!(
                bench
                    .workbench
                    .remote
                    .as_ref()
                    .unwrap()
                    .workspace
                    .projection()
                    .directories
                    .contains_key(&bench.root),
                "the correlated List reply must still apply"
            );
        }

        #[test]
        fn remote_integration_exit_clears_the_shared_session_map() {
            let mut bench = remote_bench();
            let attach = bench.attach();
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::Attached,
            ));
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::TerminalOutput {
                    data: Chunk::new(b"terminal-live".to_vec()).unwrap(),
                    done: true,
                },
            ));
            wait_for_attach(&mut bench);
            let session_root = remote_path("/session");
            follow_snapshot(
                &mut bench,
                attach,
                active_snapshot(1, "/session"),
                &session_root,
            );
            assert!(
                !bench.workbench.session_map().rows().is_empty(),
                "the current-attach snapshot populates the shared session map"
            );
            assert_eq!(
                bench.workbench.remote_integration(),
                Some(RemoteIntegrationState::Connected)
            );
            let terminal_output = bench.workbench.terminal.as_ref().unwrap().output_bytes();

            // The remote OpenCode exits: the next session-state poll reports it
            // unavailable while the SSH transport stays healthy.
            bench
                .workbench
                .remote_workspace_mut()
                .unwrap()
                .refresh_sessions()
                .unwrap();
            let mut session_poll_sent = false;
            pump_until(&mut bench, |bench, frame| {
                if let Some(frame) = frame {
                    if matches!(frame.operation, Operation::SessionState) {
                        session_poll_sent = true;
                        bench.reply(
                            &frame,
                            Operation::Error {
                                code: ErrorCode::Unavailable,
                            },
                        );
                    }
                }
                session_poll_sent
                    && bench.workbench.remote_integration()
                        == Some(RemoteIntegrationState::Unavailable)
            });
            assert!(
                bench.workbench.session_map().rows().is_empty(),
                "stale remote session rows must not outlive the remote integration"
            );
            assert!(
                matches!(
                    bench.workbench.remote.as_ref().unwrap().phase,
                    RemotePhase::Ready
                ),
                "an integration-only failure must not fail the attached session"
            );
            assert_eq!(
                bench.workbench.terminal.as_ref().unwrap().output_bytes(),
                terminal_output,
                "the attached terminal must keep its output state"
            );
        }

        #[test]
        fn remote_transport_loss_clears_the_shared_session_map() {
            let mut bench = remote_bench();
            let attach = bench.attach();
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::Attached,
            ));
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::TerminalOutput {
                    data: Chunk::new(b"terminal-live".to_vec()).unwrap(),
                    done: true,
                },
            ));
            wait_for_attach(&mut bench);
            let session_root = remote_path("/session");
            follow_snapshot(
                &mut bench,
                attach,
                active_snapshot(1, "/session"),
                &session_root,
            );
            assert!(
                !bench.workbench.session_map().rows().is_empty(),
                "the current-attach snapshot populates the shared session map"
            );
            let terminal_output = bench.workbench.terminal.as_ref().unwrap().output_bytes();

            // The SSH transport drops: the client reports disconnected and the
            // tab gates its remote session state without touching the terminal.
            bench
                .workbench
                .remote
                .as_ref()
                .unwrap()
                .client
                .as_ref()
                .unwrap()
                .disconnect();
            bench.pump();
            assert_eq!(
                bench.workbench.remote_connection(),
                Some(crate::remote_workspace::ConnectionState::Disconnected),
            );
            assert!(
                bench.workbench.session_map().rows().is_empty(),
                "stale remote session rows must not outlive the SSH transport"
            );
            assert!(
                matches!(
                    bench.workbench.remote.as_ref().unwrap().phase,
                    RemotePhase::Failed
                ),
                "the tab must report the transport failure"
            );
            assert_eq!(
                bench.workbench.terminal.as_ref().unwrap().output_bytes(),
                terminal_output,
                "the attached terminal keeps its output state"
            );
        }

        #[test]
        fn remote_diagnostics_report_the_owning_tab_not_the_host() {
            let mut bench = remote_bench();

            let attach = bench.attach();
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::Attached,
            ));
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::TerminalOutput {
                    data: Chunk::new(b"terminal-live".to_vec()).unwrap(),
                    done: true,
                },
            ));
            wait_for_attach(&mut bench);
            let session_root = remote_path("/session");
            follow_snapshot(
                &mut bench,
                attach,
                active_snapshot(1, "/session"),
                &session_root,
            );
            let snapshot = bench.workbench.diagnostics_snapshot();
            assert_eq!(snapshot.status_label, "connected");
            assert_eq!(snapshot.reason_code, "connected");
            assert_eq!(snapshot.reason, "OpenCode session follow is ready.");
            let text = snapshot.to_text();
            assert!(text.contains("status: connected"), "{text}");
            assert!(text.contains("ssh integration: connected"), "{text}");
            assert!(text.contains("ssh transport: connected"), "{text}");

            // Transport loss reports the SSH transport, not an integration
            // failure and not the host-local inactive state.
            bench
                .workbench
                .remote
                .as_ref()
                .unwrap()
                .client
                .as_ref()
                .unwrap()
                .disconnect();
            bench.pump();
            let snapshot = bench.workbench.diagnostics_snapshot();
            assert_eq!(snapshot.status_label, "degraded");
            assert!(
                snapshot.reason.contains("SSH disconnected"),
                "the transport reason must stay honest: {}",
                snapshot.reason
            );
            assert!(
                snapshot.to_text().contains("ssh transport: disconnected"),
                "{}",
                snapshot.to_text()
            );
        }

        fn grok_session(
            id: &str,
            cwd: Option<&str>,
            opened_at: u64,
            activity: GrokActivity,
        ) -> GrokSessionInfo {
            GrokSessionInfo {
                session_id: id.to_owned(),
                cwd: cwd.map(|value| RemotePath::new(value.as_bytes().to_vec()).unwrap()),
                opened_at,
                activity,
            }
        }

        fn grok_snapshot(
            revision: u64,
            running: bool,
            sessions: Vec<GrokSessionInfo>,
        ) -> GrokSessionSnapshot {
            GrokSessionSnapshot {
                revision,
                running,
                hooks_ready: false,
                sessions,
            }
        }

        fn grok_snapshot_envelope(
            identity: &Identity,
            request_id: u64,
            snapshot: GrokSessionSnapshot,
        ) -> WireEnvelope {
            WireEnvelope::new(
                identity.clone(),
                request_id,
                Operation::GrokSnapshot { snapshot },
            )
        }

        fn grok_event_envelope(
            identity: &Identity,
            request_id: u64,
            kind: GrokEventKind,
            session: &str,
            cwd: Option<&str>,
            activity: GrokActivity,
        ) -> WireEnvelope {
            WireEnvelope::new(
                identity.clone(),
                request_id,
                Operation::GrokEvent {
                    event: GrokSessionEvent {
                        kind,
                        session_id: session.to_owned(),
                        cwd: cwd.map(|value| RemotePath::new(value.as_bytes().to_vec()).unwrap()),
                        activity,
                    },
                },
            )
        }

        /// Drive a Grok-established root change to completion: reply RootSelect
        /// with an advanced generation and then serve the new root's List.
        fn settle_grok_root(bench: &mut RemoteBench, target: &RemotePath) {
            let mut selected = false;
            pump_until(bench, |bench, frame| {
                if let Some(frame) = frame {
                    match &frame.operation {
                        Operation::RootSelect { path } => {
                            assert_eq!(path, target, "Grok follow must select the session cwd");
                            selected = true;
                            let mut result = root_result(&frame, path);
                            if let Operation::RootResult { generation, .. } = &mut result {
                                *generation += 1;
                            }
                            bench.reply(&frame, result);
                        }
                        Operation::List { path } if path == target => {
                            bench.reply(&frame, listed(path))
                        }
                        _ => {}
                    }
                }
                let remote = bench.workbench.remote.as_ref().unwrap();
                selected && remote.workspace.root() == target
            });
        }

        fn current_identity(bench: &RemoteBench) -> Identity {
            bench
                .workbench
                .remote
                .as_ref()
                .unwrap()
                .client
                .as_ref()
                .unwrap()
                .identity()
        }

        #[test]
        fn remote_grok_launch_sends_the_grok_dashboard_open() {
            use made_remote::protocol::LaunchTarget;

            let mut bench = remote_bench();
            // The Grok SSH launch target routes the first frame through
            // LaunchTarget::Grok; the companion then starts the detected
            // absolute grok with the single argument `dashboard`.
            bench.workbench.remote.as_mut().unwrap().grok = true;
            bench.workbench.remote.as_mut().unwrap().client = None;
            bench.workbench.remote.as_mut().unwrap().attach_request = None;
            let stream = bench.stream.clone();
            let client = made_remote::client::RemoteClient::new(
                stream,
                bench.identity.clone(),
                bench.root.clone(),
            )
            .unwrap();
            bench.workbench.arm_remote_client(client);
            let mut open_frame = None;
            pump_until(&mut bench, |_bench, frame| {
                if let Some(frame) = frame {
                    if matches!(frame.operation, Operation::Open { .. }) {
                        open_frame = Some(frame.operation.clone());
                    }
                }
                open_frame.is_some()
            });
            match open_frame.expect("the first authenticated frame") {
                Operation::Open {
                    target, directory, ..
                } => {
                    assert_eq!(target, LaunchTarget::Grok);
                    assert_eq!(directory, bench.root);
                }
                other => panic!("expected an Open frame, got {other:?}"),
            }
        }

        #[test]
        fn remote_grok_state_reply_is_correlated_and_refreshes_after_attach() {
            let mut bench = remote_bench();
            let attach = bench.attach();
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::Attached,
            ));
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::TerminalOutput {
                    data: Chunk::new(b"terminal-live".to_vec()).unwrap(),
                    done: true,
                },
            ));
            wait_for_attach(&mut bench);

            // Completing the resnapshot's RootValidate is what asks the
            // companion for a fresh Grok snapshot on the attached root.
            let mut grok_request = None;
            pump_until(&mut bench, |bench, frame| {
                if let Some(frame) = frame {
                    match &frame.operation {
                        Operation::RootValidate { path } => {
                            bench.reply(&frame, root_result(&frame, path))
                        }
                        Operation::GrokState => grok_request = Some(frame.clone()),
                        _ => {}
                    }
                }
                grok_request.is_some()
            });
            let grok_request = grok_request.unwrap();

            // The correlated reply carries the same client request id and the
            // current identity; the projection and the root follow from it.
            let inventory = grok_snapshot(
                1,
                true,
                vec![grok_session("a", Some("/grok-a"), 1, GrokActivity::Idle)],
            );
            bench.send(grok_snapshot_envelope(
                &current_identity(&bench),
                grok_request.request_id,
                inventory,
            ));
            settle_grok_root(&mut bench, &remote_path("/grok-a"));
            let remote = bench.workbench.remote.as_ref().unwrap();
            let snapshot = remote.workspace.projection().grok.clone().unwrap();
            assert_eq!(snapshot.revision, 1);
            assert!(snapshot.running);
            assert_eq!(remote.workspace.projection().source, RootSource::Grok);
        }

        #[test]
        fn remote_grok_pushes_follow_sessions_and_keep_terminal_identity() {
            let mut bench = remote_bench();
            let attach = bench.attach();
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::Attached,
            ));
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::TerminalOutput {
                    data: Chunk::new(b"terminal-live".to_vec()).unwrap(),
                    done: true,
                },
            ));
            wait_for_attach(&mut bench);
            let output = bench.workbench.terminal.as_ref().unwrap().output_bytes();

            // A genuine current-attach snapshot rides the attach id with no
            // workspace mapping; it must still apply and move the root.
            let identity = current_identity(&bench);
            bench.send(grok_snapshot_envelope(
                &identity,
                attach,
                grok_snapshot(
                    1,
                    true,
                    vec![grok_session("a", Some("/grok-a"), 1, GrokActivity::Idle)],
                ),
            ));
            settle_grok_root(&mut bench, &remote_path("/grok-a"));
            assert_terminal_continues(&bench, output);

            // A prompt event moves immediately, without waiting for the next
            // inventory refresh.
            bench.send(grok_event_envelope(
                &current_identity(&bench),
                attach,
                GrokEventKind::Prompt,
                "b",
                Some("/grok-b"),
                GrokActivity::Working,
            ));
            settle_grok_root(&mut bench, &remote_path("/grok-b"));
            assert_terminal_continues(&bench, output);

            // Repeated A→B→C switches land on the newest owned attachment and
            // never lose the terminal's accumulated output.
            bench.send(grok_snapshot_envelope(
                &current_identity(&bench),
                attach,
                grok_snapshot(
                    2,
                    true,
                    vec![
                        grok_session("a", Some("/grok-a"), 1, GrokActivity::Idle),
                        grok_session("b", Some("/grok-b"), 2, GrokActivity::Working),
                        grok_session("c", Some("/grok-c"), 3, GrokActivity::Attention),
                        grok_session("d", Some("/grok-d"), 4, GrokActivity::Error),
                    ],
                ),
            ));
            settle_grok_root(&mut bench, &remote_path("/grok-d"));
            assert_terminal_continues(&bench, output);

            let activities = bench
                .workbench
                .remote
                .as_ref()
                .unwrap()
                .workspace
                .projection()
                .grok
                .as_ref()
                .unwrap()
                .sessions
                .iter()
                .map(|session| session.activity)
                .collect::<Vec<_>>();
            assert_eq!(
                activities,
                vec![
                    GrokActivity::Idle,
                    GrokActivity::Working,
                    GrokActivity::Attention,
                    GrokActivity::Error
                ],
                "inventory order must stay attachment order"
            );
            let dots = bench.workbench.remote_grok_activities();
            assert_eq!(
                dots,
                vec![
                    SessionActivity::Idle,
                    SessionActivity::Busy,
                    SessionActivity::Attention,
                    SessionActivity::Attention
                ],
                "remote dots must map with the local idle/working/attention treatment"
            );

            // When no owned session remains the shell fallback returns.
            bench.send(grok_snapshot_envelope(
                &current_identity(&bench),
                attach,
                grok_snapshot(3, true, vec![]),
            ));
            let launch_root = bench.root.clone();
            settle_grok_root(&mut bench, &launch_root);
            assert_terminal_continues(&bench, output);
        }

        #[test]
        fn idle_remote_hook_marks_diagnostics_ready_without_install_consent() {
            let mut bench = remote_bench();
            let attach = bench.attach();
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::Attached,
            ));
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::TerminalOutput {
                    data: Chunk::new(b"terminal-live".to_vec()).unwrap(),
                    done: true,
                },
            ));
            wait_for_attach(&mut bench);
            bench.send(grok_snapshot_envelope(
                &current_identity(&bench),
                attach,
                grok_snapshot(
                    1,
                    true,
                    vec![grok_session("idle", None, 1, GrokActivity::Idle)],
                ),
            ));
            pump_until(&mut bench, |bench, _| {
                bench
                    .workbench
                    .remote
                    .as_ref()
                    .unwrap()
                    .workspace
                    .projection()
                    .grok
                    .is_some()
            });
            let revision = bench
                .workbench
                .remote
                .as_ref()
                .unwrap()
                .workspace
                .projection()
                .revision;
            bench.send(grok_event_envelope(
                &current_identity(&bench),
                attach,
                GrokEventKind::SessionStart,
                "idle",
                None,
                GrokActivity::Idle,
            ));
            pump_until(&mut bench, |bench, _| {
                bench
                    .workbench
                    .remote
                    .as_ref()
                    .unwrap()
                    .workspace
                    .projection()
                    .revision
                    > revision
            });

            bench
                .workbench
                .set_grok_diagnostics(GrokDiagnosticsRow::REMOTE_HOOKS_NOT_APPROVED);
            assert_eq!(
                bench.workbench.grok_diagnostics(),
                GrokDiagnosticsRow::READY
            );
            assert_eq!(
                bench.workbench.diagnostics_snapshot().grok_sentence,
                GrokDiagnosticsRow::READY.sentence
            );
        }

        #[test]
        fn remote_snapshot_hook_tier_drives_sessionless_and_idle_diagnostics() {
            let mut bench = remote_bench();
            let push = |bench: &mut RemoteBench, snapshot: GrokSessionSnapshot| {
                let identity = bench.identity.clone();
                bench
                    .workbench
                    .remote_workspace_mut()
                    .unwrap()
                    .accept_grok_push(WireEnvelope::new(
                        identity,
                        1,
                        Operation::GrokSnapshot { snapshot },
                    ))
                    .unwrap();
            };

            // A user-self-installed equal/newer plugin is ready from the first
            // snapshot, without consent, an install attempt, sessions or dots.
            let mut ready = grok_snapshot(1, true, vec![]);
            ready.hooks_ready = true;
            push(&mut bench, ready);
            bench
                .workbench
                .set_grok_diagnostics(GrokDiagnosticsRow::REMOTE_HOOKS_NOT_APPROVED);
            assert_eq!(
                bench.workbench.diagnostics_snapshot().grok_sentence,
                GrokDiagnosticsRow::READY.sentence
            );
            bench
                .workbench
                .set_grok_diagnostics(GrokDiagnosticsRow::REMOTE_INSTALL_PENDING);
            assert_eq!(
                bench.workbench.grok_diagnostics(),
                GrokDiagnosticsRow::READY,
                "a consented install is unnecessary when hooks are already armed"
            );
            bench
                .workbench
                .set_grok_diagnostics(GrokDiagnosticsRow::REMOTE_TRANSPORT_DOWN);
            assert_eq!(
                bench.workbench.grok_diagnostics(),
                GrokDiagnosticsRow::REMOTE_TRANSPORT_DOWN
            );

            // An explicit inventory-only tier keeps refusal and install
            // failures honest instead of inferring readiness from `running`.
            push(&mut bench, grok_snapshot(2, true, vec![]));
            for row in [
                GrokDiagnosticsRow::REMOTE_HOOKS_NOT_APPROVED,
                GrokDiagnosticsRow::REMOTE_INSTALL_PENDING,
                GrokDiagnosticsRow::REMOTE_INSTALL_FAILED,
            ] {
                bench.workbench.set_grok_diagnostics(row.clone());
                assert_eq!(bench.workbench.grok_diagnostics(), row);
            }

            // Idle inventory is sufficient to report an explicitly ready tier.
            let mut idle = grok_snapshot(
                3,
                false,
                vec![grok_session("idle", None, 1, GrokActivity::Idle)],
            );
            idle.hooks_ready = true;
            push(&mut bench, idle);
            bench
                .workbench
                .set_grok_diagnostics(GrokDiagnosticsRow::REMOTE_HOOKS_NOT_APPROVED);
            assert_eq!(
                bench.workbench.grok_diagnostics(),
                GrokDiagnosticsRow::READY
            );
        }

        #[test]
        fn remote_grok_push_never_consumes_a_numeric_equal_pending_request() {
            let mut bench = remote_bench();
            let client = bench
                .workbench
                .remote
                .as_ref()
                .unwrap()
                .client
                .clone()
                .unwrap();
            // Synthetic id alignment: production's first request is Open/Attach.
            // Consume client id 1 here so the attach id can collide with the
            // pending List workspace id in this adversarial test.
            let _ = client.request(Operation::Capabilities, None).unwrap();
            let attach = bench.attach();
            assert_eq!(attach, 2, "attach request id");
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::Attached,
            ));

            let mut list_request = None;
            pump_until(&mut bench, |bench, frame| {
                if let Some(frame) = frame {
                    match &frame.operation {
                        Operation::RootValidate { path } => {
                            bench.reply(&frame, root_result(&frame, path))
                        }
                        Operation::List { .. } if list_request.is_none() => {
                            list_request = Some(frame.clone());
                        }
                        _ => {}
                    }
                }
                list_request.is_some()
            });
            let list_request = list_request.unwrap();
            let pending_before = bench
                .workbench
                .remote
                .as_ref()
                .unwrap()
                .workspace
                .pending_count();
            assert!(pending_before > 0);

            // The push rides the attach id, whose number equals the pending
            // List workspace id. It must be applied as an event and must not
            // complete, poison, or remove that pending request.
            bench.send(grok_snapshot_envelope(
                &bench.identity.clone(),
                attach,
                grok_snapshot(1, true, vec![]),
            ));
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                bench.pump();
                if bench
                    .workbench
                    .remote
                    .as_ref()
                    .unwrap()
                    .workspace
                    .projection()
                    .grok
                    .is_some()
                {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            let remote = bench.workbench.remote.as_ref().unwrap();
            assert!(
                remote.workspace.projection().grok.is_some(),
                "the current-attach Grok snapshot must apply without a workspace mapping"
            );
            assert_eq!(
                remote.workspace.pending_count(),
                pending_before,
                "a Grok push must not consume a numeric-equal pending request"
            );

            // The real correlated reply must still apply afterwards.
            bench.reply(&list_request, listed(&bench.root));
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                bench.pump();
                if bench
                    .workbench
                    .remote
                    .as_ref()
                    .unwrap()
                    .workspace
                    .projection()
                    .directories
                    .contains_key(&bench.root)
                {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            assert!(
                bench
                    .workbench
                    .remote
                    .as_ref()
                    .unwrap()
                    .workspace
                    .projection()
                    .directories
                    .contains_key(&bench.root),
                "the correlated List reply must still apply"
            );
        }

        #[test]
        fn late_grok_state_reply_after_a_root_transition_is_discarded() {
            let mut bench = remote_bench();
            let attach = bench.attach();
            bench.send(WireEnvelope::new(
                bench.identity.clone(),
                attach,
                Operation::Attached,
            ));
            wait_for_attach(&mut bench);

            // Complete the resnapshot so a correlated GrokState request is in
            // flight before the transition.
            let mut grok_request = None;
            pump_until(&mut bench, |bench, frame| {
                if let Some(frame) = frame {
                    match &frame.operation {
                        Operation::RootValidate { path } => {
                            bench.reply(&frame, root_result(&frame, path))
                        }
                        Operation::GrokState => grok_request = Some(frame.clone()),
                        _ => {}
                    }
                }
                grok_request.is_some()
            });
            let grok_request = grok_request.unwrap();

            // A session-root transition resets the client's bounded queues and
            // discards every old-root request id.
            let session_root = remote_path("/session");
            follow_snapshot(
                &mut bench,
                attach,
                active_snapshot(1, "/session"),
                &session_root,
            );

            // The late correlated reply from before the transition arrives on
            // the old request id. It must stay discarded: no stale Grok state
            // may repopulate the projection and no forged-reply disconnect.
            bench.send(grok_snapshot_envelope(
                &bench.identity.clone(),
                grok_request.request_id,
                grok_snapshot(
                    1,
                    true,
                    vec![grok_session("a", Some("/grok-a"), 1, GrokActivity::Idle)],
                ),
            ));
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                bench.pump();
                thread::sleep(Duration::from_millis(5));
            }
            let remote = bench.workbench.remote.as_ref().unwrap();
            assert!(
                remote.workspace.projection().grok.is_none(),
                "a late pre-transition Grok reply must stay discarded"
            );
            assert!(
                remote.client.as_ref().unwrap().is_connected(),
                "a discarded late reply is not a forged reply"
            );
            assert_eq!(remote.workspace.root(), &session_root);
        }
    }

    #[test]
    fn remote_preview_failure_and_stale_states_do_not_spin() {
        use crate::remote_workspace::{Freshness, PreviewProjection};
        use made_remote::protocol::{ErrorCode, RemotePath};

        let path = RemotePath::new(b"/project/file.txt".to_vec()).unwrap();
        let preview = |freshness| PreviewProjection {
            path: path.clone(),
            kind: None,
            data: Vec::new(),
            freshness,
        };
        let (loading, error) = remote_preview_loading_and_error(
            &preview(Freshness::Unavailable(ErrorCode::Timeout)),
            false,
            false,
        );
        assert!(!loading, "failure must not paint a loading spinner");
        assert!(error
            .as_deref()
            .is_some_and(|error| error.contains("timed out")));

        let (loading, error) = remote_preview_loading_and_error(
            &preview(Freshness::Unavailable(ErrorCode::Timeout)),
            true,
            true,
        );
        assert!(!loading, "error must suppress the loading spinner");
        assert!(error.is_some());

        let (loading, error) =
            remote_preview_loading_and_error(&preview(Freshness::Stale), false, false);
        assert!(!loading, "stale without a document must not spin forever");
        assert!(error
            .as_deref()
            .is_some_and(|error| error.contains("stale")));

        let (loading, error) =
            remote_preview_loading_and_error(&preview(Freshness::Stale), true, false);
        assert!(!loading);
        assert!(error.is_none(), "a retained document is shown while stale");

        let (loading, error) =
            remote_preview_loading_and_error(&preview(Freshness::Loading), false, true);
        assert!(loading);
        assert!(error.is_none());

        let (loading, error) =
            remote_preview_loading_and_error(&preview(Freshness::Current), false, false);
        assert!(!loading);
        assert!(error.is_none());
    }
}
