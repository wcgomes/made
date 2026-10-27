//! Explorer tree and full-left read-only preview surfaces.
//!
//! The right-hand surface is a full-height lazy tree backed by [`TreeModel`].
//! The preview replaces the terminal across the left pane and is never stacked
//! below the tree. Directory reads, preview reads, and syntax highlighting run
//! on background workers; the egui thread only drains bounded results and
//! applies them through generation-checked model APIs.
//!
//! There is no editor, save command, dirty state, or write path.
//! [`PREVIEW_ACTIONS`] contains only read-only actions. Preview shortcuts are
//! scoped to preview focus; the terminal keeps Ctrl+S as XOFF.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::text::LayoutJob;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Label, RichText, ScrollArea, Sense, Stroke,
    StrokeKind, TextFormat, TextWrapMode, Ui, UiBuilder,
};
use made_explorer::highlight::Highlighter;
use made_explorer::paths::{PathError, PathGuard};
use made_explorer::preview::{Preview, PreviewKind, PreviewLoader};
use made_explorer::preview_refresh::{
    PreviewRefresher, PreviewSelection, DEFAULT_PREVIEW_DEBOUNCE,
};
use made_explorer::root_switch::RootGeneration;
use made_explorer::tree::{DirListing, DirRequest, EntryKind, TreeModel, TreeRow};
use made_explorer::watch::FsWatcher;

use crate::diagnostics::{DiagnosticLog, LogLevel};
use crate::sanitize::sanitize_display;
use crate::theme;

const MAX_TREE_RESULTS_PER_FRAME: usize = 64;
const MAX_WATCH_EVENTS_PER_FRAME: usize = 512;
const MAX_RELOAD_DIRS_PER_EVENT: usize = 64;
const MAX_PREVIEW_RESULTS_PER_FRAME: usize = 8;
const MAX_PENDING_TREE_LOADS: usize = 64;
const MAX_PENDING_TREE_RETRIES_PER_TICK: usize = 8;

/// Safe, read-only preview actions exposed by the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewAction {
    /// Re-read the selected file.
    Refresh,
    /// Copy the selected file's path.
    CopyPath,
    /// Close the preview.
    Close,
}

impl PreviewAction {
    /// Every action the preview surface exposes.
    pub const ALL: [Self; 3] = [Self::Refresh, Self::CopyPath, Self::Close];

    /// Label used by the UI and by the read-only UI test.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Refresh => "Refresh",
            Self::CopyPath => "Copy path",
            Self::Close => "Close",
        }
    }
}

/// The complete read-only preview action set.
pub const PREVIEW_ACTIONS: [PreviewAction; 3] = PreviewAction::ALL;

/// Resolve a keyboard shortcut for the read-only preview.
///
/// `Ctrl`/`Cmd+S` is deliberately inert while the preview has focus: there is no
/// save command to invoke and the preview never writes a file. The terminal has
/// its own focus scope and still sends Ctrl+S to the child as XOFF.
pub fn preview_shortcut_action(
    key: egui::Key,
    modifiers: egui::Modifiers,
) -> Option<PreviewAction> {
    let command = modifiers.command || modifiers.ctrl;
    if key == egui::Key::S && command {
        return None;
    }
    if key == egui::Key::F5 {
        return Some(PreviewAction::Refresh);
    }
    if command && modifiers.shift && key == egui::Key::C {
        return Some(PreviewAction::CopyPath);
    }
    None
}

/// One styled span in the read-only preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewSpan {
    /// Text in the span.
    pub text: String,
    /// RGB foreground.
    pub color: [u8; 3],
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Underlined.
    pub underline: bool,
}

/// One preview source line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreviewLine {
    /// Styled spans in source order.
    pub spans: Vec<PreviewSpan>,
}

impl PreviewLine {
    /// Reconstruct the plain text of the line.
    pub fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }
}

/// Owned, display-ready result of one read-only preview load.
///
/// Binary, oversized, invalid-UTF-8, missing, and forbidden files contain only
/// a safe message and no content lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewDocument {
    /// Canonical path that was requested.
    pub path: PathBuf,
    /// Classification from the loader.
    pub kind: PreviewKind,
    /// File size when known.
    pub size: Option<u64>,
    /// Safe message for non-text results.
    pub message: Option<String>,
    /// Syntax name when highlighting was applied.
    pub syntax: Option<String>,
    /// `true` when the plain-text fallback was used.
    pub plain_text: bool,
    /// Display lines. Empty for non-text results.
    pub lines: Vec<PreviewLine>,
}

impl PreviewDocument {
    /// Convert a loader result into display data.
    ///
    /// The highlighter is consulted only for text within
    /// [`HIGHLIGHT_SIZE_LIMIT`]; everything else falls back to plain spans.
    pub fn from_preview(preview: Preview, highlighter: &Highlighter) -> Self {
        let path = preview.path().to_path_buf();
        let kind = preview.kind();
        let size = preview.size();
        match preview {
            Preview::Text {
                content,
                size: content_size,
                ..
            } => {
                let highlighted = highlighter.highlight(&path, &content);
                let lines = highlighted
                    .lines
                    .into_iter()
                    .map(|line| PreviewLine {
                        spans: line
                            .spans
                            .into_iter()
                            .map(|span| PreviewSpan {
                                text: span.text,
                                color: ensure_preview_contrast(
                                    [
                                        span.style.foreground.r,
                                        span.style.foreground.g,
                                        span.style.foreground.b,
                                    ],
                                    [theme::BG_CARD.r(), theme::BG_CARD.g(), theme::BG_CARD.b()],
                                ),
                                bold: span.style.bold,
                                italic: span.style.italic,
                                underline: span.style.underline,
                            })
                            .collect(),
                    })
                    .collect();
                Self {
                    path,
                    kind,
                    size: Some(content_size),
                    message: None,
                    syntax: highlighted.syntax,
                    plain_text: highlighted.plain_text,
                    lines,
                }
            }
            other => {
                let message = other
                    .message()
                    .map(str::to_owned)
                    .unwrap_or_else(|| "Preview is unavailable.".to_owned());
                Self {
                    path,
                    kind,
                    size,
                    message: Some(message),
                    syntax: None,
                    plain_text: true,
                    lines: Vec::new(),
                }
            }
        }
    }

    /// Whether this document has renderable text lines.
    pub fn is_text(&self) -> bool {
        self.kind.is_text()
    }

    /// Number of display lines.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
}

/// Blend a syntax color toward the primary text color until it reaches AA
/// contrast against the preview card. This runs once when a document is built
/// on the worker thread, never per frame.
fn ensure_preview_contrast(color: [u8; 3], background: [u8; 3]) -> [u8; 3] {
    const MIN_RATIO: f32 = 4.5;
    let target = [
        theme::TEXT_PRIMARY.r(),
        theme::TEXT_PRIMARY.g(),
        theme::TEXT_PRIMARY.b(),
    ];
    if contrast_ratio(color, background) >= MIN_RATIO {
        return color;
    }
    for step in 1..=20 {
        let amount = step as f32 / 20.0;
        let mixed = [
            blend_channel(color[0], target[0], amount),
            blend_channel(color[1], target[1], amount),
            blend_channel(color[2], target[2], amount),
        ];
        if contrast_ratio(mixed, background) >= MIN_RATIO {
            return mixed;
        }
    }
    target
}

fn blend_channel(from: u8, to: u8, amount: f32) -> u8 {
    (from as f32 + (to as f32 - from as f32) * amount)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f32 {
    let a = relative_luminance(a);
    let b = relative_luminance(b);
    let (lighter, darker) = if a > b { (a, b) } else { (b, a) };
    (lighter + 0.05) / (darker + 0.05)
}

fn relative_luminance(color: [u8; 3]) -> f32 {
    let linear = |channel: u8| {
        let value = channel as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2])
}

/// A tree row action chosen by one UI click or keyboard command.
#[derive(Debug, Clone)]
enum TreeAction {
    Select(TreeRow),
    Toggle(PathBuf),
    OpenFile(PathBuf),
}

/// Search the visible rows for a path.
fn row_index(rows: &[TreeRow], path: &Path) -> Option<usize> {
    rows.iter().position(|row| row.path == path)
}

struct TreeJob {
    request: DirRequest,
    guard: PathGuard,
}

struct TreeResult {
    request: DirRequest,
    listing: Result<DirListing, String>,
}

struct TreeWorker {
    jobs: SyncSender<TreeJob>,
    results: Receiver<TreeResult>,
}

impl TreeWorker {
    fn start() -> Self {
        let (jobs, job_rx) = mpsc::sync_channel::<TreeJob>(64);
        let (result_tx, results) = mpsc::sync_channel::<TreeResult>(256);
        thread::Builder::new()
            .name("made-tree-loader".to_owned())
            .spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    let listing = job
                        .request
                        .load(&job.guard)
                        .map_err(|error| error.to_string());
                    if result_tx
                        .send(TreeResult {
                            request: job.request,
                            listing,
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

    fn request(&self, request: DirRequest, guard: PathGuard) -> bool {
        self.jobs.try_send(TreeJob { request, guard }).is_ok()
    }

    fn try_recv(&self) -> Option<TreeResult> {
        self.results.try_recv().ok()
    }

    #[cfg(test)]
    fn blocked() -> Self {
        // A dropped job receiver makes every try_send fail without spawning a
        // consumer, so saturation can be exercised deterministically.
        let (jobs, job_rx) = mpsc::sync_channel::<TreeJob>(1);
        drop(job_rx);
        let (_result_tx, results) = mpsc::sync_channel::<TreeResult>(1);
        Self { jobs, results }
    }
}

#[derive(Default)]
struct PendingTreeLoads {
    jobs: Vec<(DirRequest, PathGuard)>,
}

impl PendingTreeLoads {
    fn offer(&mut self, request: DirRequest, guard: PathGuard) {
        if let Some(existing) = self
            .jobs
            .iter_mut()
            .find(|(queued, _)| queued.dir() == request.dir())
        {
            *existing = (request, guard);
            return;
        }
        if self.jobs.len() >= MAX_PENDING_TREE_LOADS {
            // Keep the newest work; a dropped request leaves its tree row in a
            // loading state, but the cap is far above normal user activity.
            self.jobs.remove(0);
        }
        self.jobs.push((request, guard));
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.jobs.len()
    }

    fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    fn take(&mut self) -> Vec<(DirRequest, PathGuard)> {
        std::mem::take(&mut self.jobs)
    }
}

struct WatcherJob {
    root: PathBuf,
    generation: RootGeneration,
}

struct WatcherResult {
    generation: RootGeneration,
    watcher: Result<FsWatcher, String>,
}

struct WatcherInstaller {
    jobs: SyncSender<WatcherJob>,
    results: Receiver<WatcherResult>,
}

impl WatcherInstaller {
    fn start() -> Self {
        let (jobs, job_rx) = mpsc::sync_channel::<WatcherJob>(4);
        let (result_tx, results) = mpsc::sync_channel::<WatcherResult>(8);
        thread::Builder::new()
            .name("made-watcher-installer".to_owned())
            .spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    let watcher = FsWatcher::new(&job.root, job.generation)
                        .map_err(|error| error.to_string());
                    if result_tx
                        .send(WatcherResult {
                            generation: job.generation,
                            watcher,
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
        self.jobs.try_send(WatcherJob { root, generation }).is_ok()
    }

    fn try_recv(&self) -> Option<WatcherResult> {
        self.results.try_recv().ok()
    }
}

struct PreviewJob {
    request: u64,
    token: PreviewSelection,
    loader: PreviewLoader,
    path: PathBuf,
}

struct PreviewResult {
    request: u64,
    token: PreviewSelection,
    document: PreviewDocument,
}

struct PreviewWorker {
    jobs: SyncSender<PreviewJob>,
    results: Receiver<PreviewResult>,
}

impl PreviewWorker {
    fn start() -> Self {
        let (jobs, job_rx) = mpsc::sync_channel::<PreviewJob>(8);
        let (result_tx, results) = mpsc::sync_channel::<PreviewResult>(16);
        thread::Builder::new()
            .name("made-preview-loader".to_owned())
            .spawn(move || {
                let mut highlighter: Option<Highlighter> = None;
                while let Ok(job) = job_rx.recv() {
                    let preview = match job.loader.load_for(&job.path, job.token.generation()) {
                        Ok(preview) => preview,
                        Err(_) => continue,
                    };
                    let highlighter = highlighter.get_or_insert_with(Highlighter::new);
                    let document = PreviewDocument::from_preview(preview, highlighter);
                    if result_tx
                        .send(PreviewResult {
                            request: job.request,
                            token: job.token,
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

    fn request(&self, job: PreviewJob) -> bool {
        self.jobs.try_send(job).is_ok()
    }

    fn try_recv(&self) -> Option<PreviewResult> {
        self.results.try_recv().ok()
    }
}

#[derive(Default)]
struct PreviewState {
    path: Option<PathBuf>,
    document: Option<PreviewDocument>,
    loading: bool,
    error: Option<String>,
    pending_request: Option<u64>,
    last_applied: u64,
    next_request: u64,
    refreshed_at: Option<Instant>,
}

impl PreviewState {
    fn begin_request(&mut self, path: &Path, request: u64) {
        if self.path.as_deref() != Some(path) {
            // A new selection must never display the previous file's content.
            self.document = None;
            self.refreshed_at = None;
        }
        self.path = Some(path.to_path_buf());
        self.loading = true;
        self.error = None;
        self.pending_request = Some(request);
    }

    fn begin_error(&mut self, path: PathBuf, error: impl Into<String>) {
        if self.path.as_deref() != Some(path.as_path()) {
            self.document = None;
            self.refreshed_at = None;
        }
        self.path = Some(path);
        self.loading = false;
        self.error = Some(error.into());
        self.pending_request = None;
    }
}

/// Result requested from the full-left preview surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewUiAction {
    /// Stay on the preview.
    #[default]
    None,
    /// Close the preview and return to the terminal.
    Close,
    /// Keep the loaded preview and return to the live terminal.
    ShowTerminal,
}

/// Explorer + preview surface for one active root.
pub struct ExplorerPane {
    root: Option<PathBuf>,
    generation: RootGeneration,
    tree: Option<TreeModel>,
    watcher: Option<FsWatcher>,
    watcher_installer: WatcherInstaller,
    watcher_loading: bool,
    refresher: Option<PreviewRefresher>,
    watcher_error: Option<String>,
    root_error: Option<String>,
    root_unavailable: Option<String>,
    tree_worker: TreeWorker,
    preview_worker: PreviewWorker,
    pending_tree_loads: PendingTreeLoads,
    rows: Vec<TreeRow>,
    rows_dirty: bool,
    row_rebuilds: u64,
    pending_expand: Vec<PathBuf>,
    preview: PreviewState,
    preview_opened: bool,
    preview_focus_pending: bool,
    ui_scope: egui::Id,
    switched_at: Option<(Instant, RootGeneration)>,
    switch_latency: Option<Duration>,
    root_ready: bool,
    scroll_to_selection: bool,
}

impl std::fmt::Debug for ExplorerPane {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExplorerPane")
            .field("root", &self.root)
            .field("generation", &self.generation)
            .field("root_error", &self.root_error)
            .field("watcher_error", &self.watcher_error)
            .field("preview_path", &self.preview.path)
            .finish_non_exhaustive()
    }
}

impl ExplorerPane {
    /// Install a root and begin loading its first listing in the background.
    pub fn new(root: impl AsRef<Path>, generation: RootGeneration) -> Self {
        let mut pane = Self {
            root: None,
            generation,
            tree: None,
            watcher: None,
            watcher_installer: WatcherInstaller::start(),
            watcher_loading: false,
            refresher: None,
            watcher_error: None,
            root_error: None,
            root_unavailable: None,
            tree_worker: TreeWorker::start(),
            preview_worker: PreviewWorker::start(),
            pending_tree_loads: PendingTreeLoads::default(),
            rows: Vec::new(),
            rows_dirty: true,
            row_rebuilds: 0,
            pending_expand: Vec::new(),
            preview: PreviewState::default(),
            preview_opened: false,
            preview_focus_pending: false,
            ui_scope: egui::Id::unique("made-explorer"),
            switched_at: None,
            switch_latency: None,
            root_ready: false,
            scroll_to_selection: false,
        };
        pane.install_root(root.as_ref());
        pane
    }

    /// Canonical active root, when it could be installed.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Set the workspace-scoped egui id used by every custom explorer widget.
    pub fn set_ui_scope(&mut self, id: egui::Id) {
        self.ui_scope = id;
    }

    /// Stable widget id for one visible tree row.
    pub fn row_widget_id(&self, index: usize) -> egui::Id {
        self.ui_scope.with(("tree-row", index))
    }

    /// The single focusable tree container.
    pub fn tree_focus_id(&self) -> egui::Id {
        self.ui_scope.with("tree-focus")
    }

    /// Number of full visible-row rebuilds. Selection movement must not change
    /// this while the tree contents are unchanged.
    pub fn row_rebuilds(&self) -> u64 {
        self.row_rebuilds
    }

    /// Stable widget id for a preview header action.
    pub fn preview_action_id(&self, action: PreviewAction) -> egui::Id {
        self.ui_scope.with(("preview-action", action.label()))
    }

    /// Stable widget id for the preview's return-to-terminal action.
    pub fn preview_terminal_id(&self) -> egui::Id {
        self.ui_scope.with(("preview-action", "Terminal"))
    }

    /// The preview card's focus id.
    pub fn preview_focus_id(&self) -> egui::Id {
        self.ui_scope.with("preview-focus")
    }

    /// Whether a file-open event is waiting to select the preview surface.
    pub fn take_preview_opened(&mut self) -> bool {
        std::mem::take(&mut self.preview_opened)
    }

    /// Mark the preview for explicit keyboard focus on its next frame.
    pub fn focus_preview(&mut self) {
        self.preview_focus_pending = true;
    }

    /// Surrender preview focus before returning to the terminal.
    pub fn release_preview_focus(&mut self, ctx: &egui::Context) {
        let id = self.preview_focus_id();
        ctx.memory_mut(|memory| {
            if memory.has_focus(id) {
                memory.surrender_focus(id);
            }
        });
    }

    /// Surrender focus owned by the explorer when its panel is hidden.
    pub fn release_focus(&self, ctx: &egui::Context) {
        let focused = ctx.memory(|memory| memory.focused());
        let Some(focused) = focused else {
            return;
        };
        if focused == self.tree_focus_id() {
            ctx.memory_mut(|memory| memory.surrender_focus(focused));
        }
    }

    /// Loaded preview document, if any. Never exposes a write path.
    pub fn preview_document(&self) -> Option<&PreviewDocument> {
        self.preview.document.as_ref()
    }

    /// Active root generation.
    pub fn generation(&self) -> RootGeneration {
        self.generation
    }

    /// Root-level failure message. The rest of the app stays usable.
    pub fn root_error(&self) -> Option<&str> {
        self.root_error.as_deref()
    }

    /// Whether the first root listing has been applied (or failed visibly).
    pub fn is_root_ready(&self) -> bool {
        self.root_ready
    }

    /// Watcher failure, when automatic tree refresh is unavailable.
    pub fn watcher_error(&self) -> Option<&str> {
        self.watcher_error.as_deref()
    }

    /// Whether a live watcher is installed.
    pub fn has_watcher(&self) -> bool {
        self.watcher.is_some()
    }

    /// Whether the watcher installation worker is still running.
    pub fn watcher_loading(&self) -> bool {
        self.watcher_loading
    }

    /// Active-session root unavailable banner, while the last tree is retained.
    pub fn root_unavailable(&self) -> Option<&str> {
        self.root_unavailable.as_deref()
    }

    /// Mark the active session root unavailable without discarding the tree.
    pub fn mark_root_unavailable(&mut self, detail: impl Into<String>) {
        self.root_unavailable = Some(detail.into());
    }

    /// Clear the active-session root unavailable banner after recovery.
    pub fn clear_root_unavailable(&mut self) {
        self.root_unavailable = None;
    }

    /// Visible (flattened) row count, for diagnostics and UI tests.
    pub fn visible_row_count(&mut self) -> usize {
        self.ensure_rows();
        self.rows.len()
    }

    /// Selected path in the tree.
    pub fn selected_path(&self) -> Option<&Path> {
        self.tree.as_ref().and_then(TreeModel::selection)
    }

    /// Start measuring active-session switch latency for this root. Called when
    /// a session-follow root switch creates the pane, not on initial startup.
    pub fn begin_switch_measurement(&mut self) {
        self.switched_at = Some((Instant::now(), self.generation));
    }

    /// Take the measured active-session switch latency once the new root's
    /// first listing has been applied.
    pub fn take_switch_latency(&mut self) -> Option<Duration> {
        self.switch_latency.take()
    }

    /// Retry installing the current root after a failure.
    pub fn retry(&mut self) {
        if let Some(root) = self.root.clone() {
            self.install_root(&root);
        }
    }

    fn install_root(&mut self, requested: &Path) {
        self.tree = None;
        self.watcher = None;
        self.watcher_loading = false;
        self.refresher = None;
        self.watcher_error = None;
        self.root_error = None;
        self.root_unavailable = None;
        self.pending_tree_loads = PendingTreeLoads::default();
        self.rows.clear();
        self.rows_dirty = true;
        self.pending_expand.clear();
        self.preview = PreviewState::default();
        self.preview_opened = false;
        self.preview_focus_pending = false;
        self.root_ready = false;

        match TreeModel::new(requested, self.generation) {
            Ok(tree) => {
                self.root = Some(tree.root().to_path_buf());
                // Reuse the tree's already-validated guard; do not canonicalize
                // and read the root a second time on the UI thread.
                self.refresher = Some(PreviewRefresher::from_guard(
                    tree.path_guard().clone(),
                    self.generation,
                    DEFAULT_PREVIEW_DEBOUNCE,
                ));
                let root = tree.root().to_path_buf();
                self.tree = Some(tree);
                self.pending_expand.push(root.clone());
                self.queue_load(root.clone());
                self.watcher_loading = true;
                if !self.watcher_installer.request(root, self.generation) {
                    self.watcher_loading = false;
                    self.watcher_error = Some("the live-update watcher queue is busy".to_owned());
                }
            }
            Err(error) => {
                self.root = Some(requested.to_path_buf());
                self.root_error = Some(root_error_message(requested, &error));
                self.root_ready = true;
            }
        }
    }

    fn queue_load(&mut self, dir: PathBuf) {
        let Some((request, guard)) = self.begin_load(dir, false) else {
            return;
        };
        self.queue_tree_request(request, guard);
    }

    fn queue_tree_request(&mut self, request: DirRequest, guard: PathGuard) {
        if self.tree_worker.request(request.clone(), guard.clone()) {
            return;
        }
        // Queue saturation must not fabricate error rows. Keep the request
        // (coalesced by directory) and retry without blocking the UI.
        self.pending_tree_loads.offer(request, guard);
    }

    fn retry_pending_tree_loads(&mut self) {
        if self.pending_tree_loads.is_empty() {
            return;
        }
        let pending = self.pending_tree_loads.take();
        let mut remaining = Vec::new();
        let mut attempted = 0;
        for (request, guard) in pending {
            if request.generation() != self.generation {
                continue;
            }
            let still_loading = self
                .tree
                .as_ref()
                .is_some_and(|tree| tree.is_loading(request.dir()));
            if !still_loading {
                continue;
            }
            if attempted < MAX_PENDING_TREE_RETRIES_PER_TICK
                && self.tree_worker.request(request.clone(), guard.clone())
            {
                attempted += 1;
                continue;
            }
            remaining.push((request, guard));
        }
        for job in remaining {
            self.pending_tree_loads.offer(job.0, job.1);
        }
    }

    /// Start a background read even when a cached listing exists.
    fn queue_reload(&mut self, dir: PathBuf) {
        let Some((request, guard)) = self.begin_load(dir, true) else {
            return;
        };
        self.queue_tree_request(request, guard);
    }

    fn begin_load(&mut self, dir: PathBuf, force: bool) -> Option<(DirRequest, PathGuard)> {
        let tree = self.tree.as_mut()?;
        if !force && (tree.is_loaded(&dir) || tree.is_loading(&dir)) {
            return None;
        }
        if force && tree.is_loading(&dir) {
            return None;
        }
        match tree.begin_load(&dir) {
            Ok(request) => Some((request, tree.path_guard().clone())),
            Err(error) => {
                // Do not perform a synchronous read on the UI thread. An
                // outside-root load is rejected by apply_load; a missing or
                // unreadable child becomes a visible error row.
                let listing = DirListing::error_listing(dir, self.generation, error.to_string());
                let _ = tree.apply_load(listing);
                self.rows_dirty = true;
                self.preview.error = Some(error.to_string());
                None
            }
        }
    }

    fn toggle_dir(&mut self, dir: PathBuf) {
        enum Toggle {
            Collapse,
            ExpandLoaded,
            Load,
        }
        let action = {
            let Some(tree) = self.tree.as_ref() else {
                return;
            };
            if tree.is_expanded(&dir) {
                Toggle::Collapse
            } else if tree.is_loaded(&dir) {
                Toggle::ExpandLoaded
            } else {
                Toggle::Load
            }
        };
        match action {
            Toggle::Collapse => {
                if let Some(tree) = self.tree.as_mut() {
                    tree.collapse(&dir);
                }
                self.pending_expand.retain(|path| path != &dir);
                self.rows_dirty = true;
            }
            Toggle::ExpandLoaded => {
                if let Some(tree) = self.tree.as_mut() {
                    tree.expand(&dir).ok();
                }
                self.rows_dirty = true;
            }
            Toggle::Load => {
                if !self.pending_expand.contains(&dir) {
                    self.pending_expand.push(dir.clone());
                }
                self.queue_load(dir);
            }
        }
    }

    /// Update the tree selection without activating the row.
    fn apply_selection_path(&mut self, path: &Path) {
        let Some(tree) = self.tree.as_mut() else {
            return;
        };
        tree.select(path).ok();
        let mut found = false;
        for visible in &mut self.rows {
            let selected = visible.path == path;
            visible.selected = selected;
            found |= selected;
        }
        if !found {
            // The path is not in the current flattened view (for example a
            // parent revealed by collapsing a subtree). Fall back to a single
            // rebuild rather than leaving a stale selected flag.
            self.rows_dirty = true;
        }
        self.scroll_to_selection = true;
    }

    /// Activate a row: Enter/Space opens files and toggles directories.
    fn activate_row(&mut self, row: &TreeRow) {
        match row.kind {
            EntryKind::File => self.open_preview(row.path.clone()),
            EntryKind::Directory => {
                self.apply_selection_path(&row.path);
                self.toggle_dir(row.path.clone());
                if let Some(tree) = self.tree.as_mut() {
                    tree.select(&row.path).ok();
                }
                self.rows_dirty = true;
            }
            // Error/symlink/other rows are inspectable but never become a
            // preview or a directory action.
            EntryKind::Error | EntryKind::Symlink | EntryKind::Other => {}
        }
    }

    /// Click behavior: select immediately and open files as before.
    fn select_row_click(&mut self, row: &TreeRow) {
        self.apply_selection_path(&row.path);
        self.activate_row(row);
    }

    /// Open a real file in the read-only preview surface.
    pub fn open_preview(&mut self, path: PathBuf) {
        self.preview_opened = true;
        self.focus_preview();
        let Some(refresher) = self.refresher.as_mut() else {
            self.preview
                .begin_error(path, "Automatic preview is unavailable for this root.");
            return;
        };
        let token = match refresher.select(&path) {
            Ok(token) => token,
            Err(error) => {
                self.preview.begin_error(path, error.to_string());
                return;
            }
        };
        let loader = refresher.loader().clone();
        self.request_preview(path, token, loader);
    }

    fn request_preview(&mut self, path: PathBuf, token: PreviewSelection, loader: PreviewLoader) {
        self.preview.next_request = self.preview.next_request.saturating_add(1);
        let request = self.preview.next_request;
        self.preview.begin_request(&path, request);
        if !self.preview_worker.request(PreviewJob {
            request,
            token,
            loader,
            path,
        }) {
            self.preview.loading = false;
            self.preview.pending_request = None;
            self.preview.error = Some("The preview queue is busy; retry in a moment.".to_owned());
        }
    }

    fn refresh_preview(&mut self) {
        let Some(path) = self.preview.path.clone() else {
            return;
        };
        let Some(refresher) = self.refresher.as_ref() else {
            return;
        };
        let Some(token) = refresher.current_selection() else {
            return;
        };
        let loader = refresher.loader().clone();
        self.request_preview(path, token, loader);
    }

    pub fn clear_preview(&mut self) {
        self.preview = PreviewState::default();
        self.preview_focus_pending = false;
        if let Some(refresher) = self.refresher.as_mut() {
            refresher.clear_selection();
        }
    }

    /// Drain background results and watcher events. The UI thread never reads a
    /// directory or a preview file itself.
    pub fn tick(&mut self, log: &mut DiagnosticLog) {
        self.poll_watcher_install();
        self.retry_pending_tree_loads();
        self.poll_tree_results(log);
        self.poll_watch_events(log);
        self.poll_preview_events(log);
        self.poll_preview_results(log);
    }

    fn poll_watcher_install(&mut self) {
        while let Some(result) = self.watcher_installer.try_recv() {
            self.apply_watcher_result(result);
        }
    }

    fn apply_watcher_result(&mut self, result: WatcherResult) {
        if result.generation != self.generation {
            // A worker installed for an older root must not leak into the
            // current tree. Dropping it stops its watcher threads.
            return;
        }
        self.watcher_loading = false;
        match result.watcher {
            Ok(watcher) => {
                self.watcher = Some(watcher);
                self.watcher_error = None;
                // Catch a create/delete that raced the watcher installation.
                if let Some(root) = self.root.clone() {
                    self.queue_reload(root);
                }
            }
            Err(error) => {
                self.watcher = None;
                self.watcher_error = Some(error);
            }
        }
    }

    fn poll_tree_results(&mut self, log: &mut DiagnosticLog) {
        let mut count = 0;
        while count < MAX_TREE_RESULTS_PER_FRAME {
            let Some(result) = self.tree_worker.try_recv() else {
                break;
            };
            count += 1;
            let TreeResult { request, listing } = result;
            let listing = match listing {
                Ok(listing) => listing,
                Err(error) => DirListing::error_listing(
                    request.dir().to_path_buf(),
                    request.generation(),
                    error,
                ),
            };
            let dir = request.dir().to_path_buf();
            let expanded = self.pending_expand.iter().any(|path| path == &dir);
            let applied = {
                let Some(tree) = self.tree.as_mut() else {
                    continue;
                };
                match tree.finish_load(&request, listing) {
                    Ok(()) => {
                        if expanded {
                            tree.expand(&dir).ok();
                        }
                        true
                    }
                    Err(error) => {
                        log.record(
                            LogLevel::Warn,
                            format!(
                                "discarded stale tree load for {}: {error}",
                                request.dir().display()
                            ),
                        );
                        false
                    }
                }
            };
            if !applied {
                continue;
            }
            if expanded {
                self.pending_expand.retain(|path| path != &dir);
            }
            self.rows_dirty = true;
            self.note_root_loaded(&dir);
        }
    }

    fn note_root_loaded(&mut self, dir: &Path) {
        let is_root = self.root.as_deref() == Some(dir);
        if !is_root {
            return;
        }
        self.root_ready = true;
        if let Some((started, generation)) = self.switched_at {
            if generation == self.generation {
                self.switch_latency = Some(started.elapsed());
                self.switched_at = None;
            }
        }
    }

    fn poll_watch_events(&mut self, log: &mut DiagnosticLog) {
        let mut events = Vec::new();
        if let Some(watcher) = self.watcher.as_mut() {
            for _ in 0..MAX_WATCH_EVENTS_PER_FRAME {
                let Some(event) = watcher.poll() else {
                    break;
                };
                if event.is_current(self.generation) {
                    events.push(event);
                }
            }
        }
        if events.is_empty() {
            return;
        }
        let reload_dirs = {
            let Some(tree) = self.tree.as_mut() else {
                return;
            };
            match tree.apply_watch_events(&events) {
                Ok(update) => {
                    if !update.is_empty() {
                        self.rows_dirty = true;
                    }
                    update
                        .reload_dirs
                        .into_iter()
                        .take(MAX_RELOAD_DIRS_PER_EVENT)
                        .collect::<Vec<_>>()
                }
                Err(error) => {
                    log.record(
                        LogLevel::Warn,
                        format!("filesystem update rejected: {error}"),
                    );
                    Vec::new()
                }
            }
        };
        for dir in reload_dirs {
            self.queue_reload(dir);
        }
    }

    fn poll_preview_events(&mut self, log: &mut DiagnosticLog) {
        let mut pending = Vec::new();
        if let Some(refresher) = self.refresher.as_mut() {
            while let Some(event) = refresher.poll_event() {
                let current = refresher.current_selection();
                if current != Some(event.selection) {
                    log.record(LogLevel::Info, "discarded stale preview change event");
                    continue;
                }
                let loader = refresher.loader().clone();
                pending.push((event.path, event.selection, loader));
            }
        }
        for (path, token, loader) in pending {
            self.request_preview(path, token, loader);
        }
    }

    fn poll_preview_results(&mut self, log: &mut DiagnosticLog) {
        let mut count = 0;
        while count < MAX_PREVIEW_RESULTS_PER_FRAME {
            let Some(result) = self.preview_worker.try_recv() else {
                break;
            };
            count += 1;
            let current = self
                .refresher
                .as_ref()
                .and_then(PreviewRefresher::current_selection);
            if current != Some(result.token) {
                log.record(LogLevel::Info, "discarded stale preview result");
                continue;
            }
            if result.request < self.preview.last_applied {
                continue;
            }
            self.preview.last_applied = result.request;
            self.preview.document = Some(result.document);
            self.preview.loading = self
                .preview
                .pending_request
                .is_some_and(|pending| pending > result.request);
            self.preview.error = None;
            self.preview.refreshed_at = Some(Instant::now());
        }
    }

    fn ensure_rows(&mut self) {
        if !self.rows_dirty {
            return;
        }
        self.rows = self.tree.as_ref().map(TreeModel::rows).unwrap_or_default();
        self.rows_dirty = false;
        self.row_rebuilds = self.row_rebuilds.saturating_add(1);
    }

    /// Move the visible selection by `delta` without opening a preview.
    ///
    /// Visible selection flags are updated in place; the full row list is only
    /// rebuilt when the tree structure changes.
    pub fn move_selection(&mut self, delta: isize) {
        self.ensure_rows();
        if self.rows.is_empty() {
            return;
        }
        let current = self
            .tree
            .as_ref()
            .and_then(TreeModel::selection)
            .and_then(|path| row_index(&self.rows, path));
        let len = self.rows.len() as isize;
        let next = match current {
            Some(index) => (index as isize + delta).clamp(0, len - 1),
            None if delta < 0 => len - 1,
            None => 0,
        } as usize;
        let path = self.rows[next].path.clone();
        self.apply_selection_path(&path);
    }

    fn move_selection_to(&mut self, index: usize) {
        self.ensure_rows();
        if self.rows.is_empty() {
            return;
        }
        let index = index.min(self.rows.len() - 1);
        let path = self.rows[index].path.clone();
        self.apply_selection_path(&path);
    }

    fn selected_row(&mut self) -> Option<TreeRow> {
        self.ensure_rows();
        let path = self.selected_path()?.to_path_buf();
        self.rows.iter().find(|row| row.path == path).cloned()
    }

    fn handle_tree_keys(&mut self, ui: &Ui, tree_id: egui::Id) {
        if !ui.memory(|memory| memory.has_focus(tree_id)) {
            return;
        }
        let events = ui.ctx().input(|input| input.events.clone());
        for event in events {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            if key == egui::Key::S && (modifiers.command || modifiers.ctrl) {
                continue;
            }
            match key {
                egui::Key::ArrowDown => self.move_selection(1),
                egui::Key::ArrowUp => self.move_selection(-1),
                egui::Key::Home => self.move_selection_to(0),
                egui::Key::End => {
                    self.ensure_rows();
                    self.move_selection_to(self.rows.len().saturating_sub(1));
                }
                egui::Key::PageUp => self.move_selection(-10),
                egui::Key::PageDown => self.move_selection(10),
                egui::Key::ArrowRight => {
                    if let Some(row) = self.selected_row() {
                        if row.kind == EntryKind::Directory {
                            self.apply_selection_path(&row.path);
                            self.toggle_dir(row.path);
                            self.rows_dirty = true;
                        }
                    }
                }
                egui::Key::ArrowLeft => {
                    let Some(path) = self.selected_path().map(Path::to_path_buf) else {
                        continue;
                    };
                    let expanded = self
                        .tree
                        .as_ref()
                        .is_some_and(|tree| tree.is_expanded(&path));
                    if expanded {
                        if let Some(tree) = self.tree.as_mut() {
                            tree.collapse(&path);
                        }
                        self.rows_dirty = true;
                    } else if let Some(parent) = path.parent().map(Path::to_path_buf) {
                        self.ensure_rows();
                        if let Some(index) = row_index(&self.rows, &parent) {
                            let path = self.rows[index].path.clone();
                            self.apply_selection_path(&path);
                        }
                    }
                }
                egui::Key::Enter | egui::Key::Space => {
                    if let Some(row) = self.selected_row() {
                        self.activate_row(&row);
                    }
                }
                _ => {}
            }
        }
    }

    /// Draw the Explorer card only. The preview lives on the left surface.
    pub fn ui(&mut self, ui: &mut Ui, log: &mut DiagnosticLog, layout: &theme::Layout) {
        self.ensure_rows();
        self.explorer_card(ui, log, layout);
    }

    /// Full-height tree surface, with no embedded preview.
    pub fn tree_ui(&mut self, ui: &mut Ui, log: &mut DiagnosticLog, layout: &theme::Layout) {
        self.ensure_rows();
        self.explorer_card(ui, log, layout);
    }

    /// Selected preview path, including loading and guarded error states.
    pub fn preview_path(&self) -> Option<&Path> {
        self.preview.path.as_deref()
    }

    /// Paint the read-only file surface independently of the explorer.
    pub fn preview_ui(
        &mut self,
        ui: &mut Ui,
        log: &mut DiagnosticLog,
        layout: &theme::Layout,
    ) -> PreviewUiAction {
        self.preview_card(ui, log, layout)
    }

    fn explorer_card(&mut self, ui: &mut Ui, log: &mut DiagnosticLog, layout: &theme::Layout) {
        // Allocate the whole card so the resizable split panel does not
        // shrink-wrap to the tree content on the second frame.
        let (card_rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        theme::paint_glass_card(ui.painter(), card_rect, layout);
        let header_rect = egui::Rect::from_min_size(
            card_rect.min,
            egui::vec2(card_rect.width(), layout.card_header_height),
        );
        theme::paint_glass_card_header(ui, header_rect, layout);
        let content = header_rect.shrink2(egui::vec2(layout.header_padding_x, 0.0));
        let mut left = ui.new_child(
            UiBuilder::new()
                .max_rect(content)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        theme::section_label(&mut left, "EXPLORER");
        if let Some(root) = &self.root {
            let full = root.display().to_string();
            let name = root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| full.clone());
            let remaining = (content.right() - left.min_rect().right() - 6.0).max(0.0);
            let chars = ((remaining / 6.0).floor() as usize).min(layout.root_name_chars);
            if chars >= 4 {
                left.label(theme::mono(
                    theme::truncate_end(&sanitize_display(&name, 160), chars),
                    10.5,
                    theme::TEXT_TERTIARY,
                ))
                .on_hover_text(sanitize_display(&full, 600));
            }
        }
        theme::paint_card_border(ui, card_rect, layout);

        let body_rect = egui::Rect::from_min_max(
            egui::pos2(card_rect.left(), header_rect.bottom()),
            card_rect.max,
        );
        let content_rect = body_rect.shrink2(egui::vec2(8.0, 4.0));
        let mut body = ui.new_child(UiBuilder::new().max_rect(content_rect));
        if let Some(unavailable) = self.root_unavailable.clone() {
            tone_strip(
                &mut body,
                layout,
                26.0,
                theme::WARNING_TINT,
                theme::WARNING,
                "Active root unavailable — showing the last folder.",
                sanitize_display(&unavailable, 600),
            );
            body.add_space(4.0);
        }
        self.tree_rows_ui(&mut body, log, layout);
    }

    fn tree_rows_ui(&mut self, ui: &mut Ui, _log: &mut DiagnosticLog, layout: &theme::Layout) {
        if let Some(error) = self.root_error.clone() {
            self.root_error_ui(ui, &error, layout);
            return;
        }

        if self.rows.is_empty() {
            if self
                .tree
                .as_ref()
                .is_some_and(|tree| tree.loading_dirs().next().is_some())
            {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(12.0).color(theme::ACCENT));
                    ui.label(theme::body("Loading folder…"));
                });
            } else {
                ui.add_space(6.0);
                ui.label(theme::body("This folder is empty."));
            }
            return;
        }

        let tree_id = self.tree_focus_id();
        let ui_scope = self.ui_scope;
        let scroll_to_selection = self.scroll_to_selection;
        let mut action = None;
        let mut clicked_tree = false;
        let tree_rect = ui.available_rect_before_wrap();
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                tree_id,
                egui::EventFilter {
                    tab: false,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: false,
                },
            );
        });
        let container = ui.interact(tree_rect, tree_id, Sense::click());
        if container.clicked() {
            container.request_focus();
        }
        // Keyboard/programmatic entry into the tree must have a visible cue.
        // Select the first visible row without activating it (no preview).
        if container.has_focus()
            && self.tree.as_ref().and_then(TreeModel::selection).is_none()
            && !self.rows.is_empty()
        {
            let first = self.rows[0].path.clone();
            self.apply_selection_path(&first);
        }
        let rows = &self.rows;

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, 24.0, rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for index in range {
                    let Some(row) = rows.get(index) else {
                        continue;
                    };
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 24.0),
                        Sense::hover(),
                    );
                    let response =
                        ui.interact(rect, ui_scope.with(("tree-row", index)), Sense::CLICK);
                    let fill = if row.selected {
                        theme::BG_SELECTED
                    } else if response.hovered() {
                        theme::BG_HOVER
                    } else {
                        Color32::TRANSPARENT
                    };
                    let row_rect = rect.shrink2(egui::vec2(4.0, 0.0));
                    if fill != Color32::TRANSPARENT {
                        ui.painter()
                            .rect_filled(row_rect, CornerRadius::same(6), fill);
                    }

                    let indent = 4.0 + row.depth as f32 * 14.0;
                    let expanded = row.kind == EntryKind::Directory && row.expanded;
                    let color = if row.kind == EntryKind::Error {
                        theme::ERROR
                    } else if row.selected || expanded {
                        theme::TEXT_PRIMARY
                    } else {
                        theme::TEXT_SECONDARY
                    };
                    let icon_color = if row.kind == EntryKind::Error {
                        theme::ERROR
                    } else if row.selected || expanded {
                        theme::TEXT_PRIMARY
                    } else {
                        theme::TEXT_TERTIARY
                    };
                    let name = sanitize_display(&row.name, 200);
                    let text_left = rect.left() + 24.0 + row.depth as f32 * 14.0;
                    let width = (rect.right() - text_left - 6.0).max(1.0);
                    theme::paint_ellipsized_centered(
                        ui.painter(),
                        text_left,
                        rect.center().y,
                        name,
                        FontId::proportional(12.5),
                        color,
                        width,
                    );
                    // The label helper centers its galley on the row center, so
                    // the markers use that same optical center.
                    let marker_center = egui::pos2(rect.left() + indent + 8.0, rect.center().y);
                    match row.kind {
                        EntryKind::Directory => theme::paint_disclosure(
                            ui.painter(),
                            marker_center,
                            row.expanded,
                            icon_color,
                        ),
                        EntryKind::File => {
                            theme::paint_bullet(ui.painter(), marker_center, icon_color)
                        }
                        EntryKind::Symlink => {
                            ui.painter().text(
                                marker_center,
                                Align2::CENTER_CENTER,
                                "@",
                                FontId::monospace(10.0),
                                icon_color,
                            );
                        }
                        EntryKind::Other => {
                            ui.painter().text(
                                marker_center,
                                Align2::CENTER_CENTER,
                                "?",
                                FontId::monospace(10.0),
                                icon_color,
                            );
                        }
                        EntryKind::Error => {
                            ui.painter().text(
                                marker_center,
                                Align2::CENTER_CENTER,
                                "!",
                                FontId::monospace(10.0),
                                icon_color,
                            );
                        }
                    }

                    let tooltip = if let Some(error) = &row.error {
                        sanitize_display(error, 600)
                    } else {
                        sanitize_display(&row.path.display().to_string(), 600)
                    };
                    let response = response.on_hover_text(tooltip);
                    if scroll_to_selection && row.selected {
                        response.scroll_to_me(Some(Align::Center));
                    }
                    if response.clicked() {
                        clicked_tree = true;
                        action = Some(match row.kind {
                            EntryKind::Directory => TreeAction::Toggle(row.path.clone()),
                            EntryKind::File => TreeAction::OpenFile(row.path.clone()),
                            _ => TreeAction::Select(row.clone()),
                        });
                    }
                }
            });
        self.scroll_to_selection = false;
        if clicked_tree {
            ui.memory_mut(|memory| memory.request_focus(tree_id));
        }
        if let Some(action) = action {
            match action {
                TreeAction::Select(row) => self.select_row_click(&row),
                TreeAction::Toggle(path) => {
                    self.toggle_dir(path.clone());
                    if let Some(tree) = self.tree.as_mut() {
                        tree.select(&path).ok();
                    }
                    self.rows_dirty = true;
                }
                TreeAction::OpenFile(path) => {
                    self.apply_selection_path(&path);
                    self.open_preview(path);
                }
            }
        }
        self.handle_tree_keys(ui, tree_id);
    }

    fn root_error_ui(&mut self, ui: &mut Ui, error: &str, layout: &theme::Layout) {
        egui::Frame::new()
            .fill(theme::ERROR_TINT)
            .stroke(Stroke::new(1.0, theme::ERROR))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Explorer unavailable")
                        .font(FontId::proportional(13.0))
                        .color(theme::ERROR),
                );
                ui.label(theme::body(sanitize_display(error, 700)));
                if theme::secondary_button(ui, "Retry", layout.button_height).clicked() {
                    self.retry();
                }
            });
    }

    fn preview_card(
        &mut self,
        ui: &mut Ui,
        log: &mut DiagnosticLog,
        layout: &theme::Layout,
    ) -> PreviewUiAction {
        if self.preview.path.is_none() {
            // No default preview placeholder: the terminal owns the left pane
            // until the user opens a real file.
            return PreviewUiAction::None;
        }
        let card_rect = ui.available_rect_before_wrap();
        theme::paint_card(ui.painter(), card_rect, layout, theme::BG_CARD);

        let focus_id = self.preview_focus_id();
        let focus_response = ui.interact(card_rect, focus_id, Sense::click());
        let pressed_here = ui.ctx().input(|input| {
            input.pointer.any_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|pos| card_rect.contains(pos))
        });
        if self.preview_focus_pending || pressed_here {
            self.preview_focus_pending = false;
            focus_response.request_focus();
        }
        if ui.memory(|memory| memory.has_focus(focus_id)) {
            if let Some(action) = preview_shortcut_action_from_input(ui.ctx()) {
                if let Some(outcome) = self.apply_preview_action(action, ui, log) {
                    return outcome;
                }
            }
        }

        let header_rect = egui::Rect::from_min_size(
            card_rect.min,
            egui::vec2(card_rect.width(), layout.card_header_height),
        );
        paint_card_header(ui, header_rect, layout, theme::BG_HEADER);
        let has_path = self.preview.path.is_some();
        let filename_chars = if card_rect.width() < 480.0 { 10 } else { 20 };
        let mut copy_path = false;
        let mut refresh = false;
        let mut close = false;
        let content = header_rect.shrink2(egui::vec2(layout.header_padding_x, 0.0));
        let mut right = ui.new_child(
            UiBuilder::new()
                .max_rect(content)
                .layout(egui::Layout::right_to_left(Align::Center)),
        );
        let close_action = theme::ghost_action_with(
            &mut right,
            self.preview_action_id(PreviewAction::Close),
            PreviewAction::Close.label(),
            false,
            layout.button_height,
            theme::BG_HEADER,
            has_path,
        );
        close_action
            .response
            .on_hover_text("Close the preview and return to the live terminal");
        if close_action.activated {
            close = true;
        }
        let terminal_action = theme::ghost_action(
            &mut right,
            self.preview_terminal_id(),
            "Terminal",
            false,
            layout.button_height,
            theme::BG_HEADER,
        );
        terminal_action
            .response
            .on_hover_text("Return to the live terminal; the preview stays loaded");
        let show_terminal = terminal_action.activated;
        if layout.compact {
            let more = theme::ghost_action(
                &mut right,
                self.preview_action_id(PreviewAction::Refresh).with("menu"),
                "…",
                false,
                layout.button_height,
                theme::BG_HEADER,
            );
            let more_response = more.response;
            egui::Popup::menu(&more_response).show(|ui| {
                if ui.button(PreviewAction::Refresh.label()).clicked() {
                    refresh = true;
                    ui.close();
                }
                if ui.button(PreviewAction::CopyPath.label()).clicked() {
                    copy_path = true;
                    ui.close();
                }
            });
            more_response.on_hover_text("Preview actions");
        } else {
            let refresh_action = theme::ghost_action_with(
                &mut right,
                self.preview_action_id(PreviewAction::Refresh),
                PreviewAction::Refresh.label(),
                false,
                layout.button_height,
                theme::BG_HEADER,
                has_path,
            );
            refresh_action
                .response
                .on_hover_text("Re-read the selected file");
            if refresh_action.activated {
                refresh = true;
            }
            let copy_action = theme::ghost_action_with(
                &mut right,
                self.preview_action_id(PreviewAction::CopyPath),
                PreviewAction::CopyPath.label(),
                false,
                layout.button_height,
                theme::BG_HEADER,
                has_path,
            );
            copy_action
                .response
                .on_hover_text("Copy the selected file path");
            if copy_action.activated {
                copy_path = true;
            }
        }
        let right_rect = right.min_rect();
        let left_rect = egui::Rect::from_min_max(
            content.min,
            egui::pos2(
                (right_rect.left() - 8.0).max(content.left()),
                content.bottom(),
            ),
        );
        let mut left = ui.new_child(
            UiBuilder::new()
                .max_rect(left_rect)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        theme::section_label(&mut left, "PREVIEW");
        if let Some(path) = &self.preview.path {
            let full = path.display().to_string();
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| full.clone());
            let remaining = (left_rect.right() - left.min_rect().right() - 6.0).max(0.0);
            let chars = ((remaining / 6.2).floor() as usize).min(filename_chars);
            if chars >= 4 {
                left.label(
                    RichText::new(theme::truncate_middle(&sanitize_display(&name, 160), chars))
                        .font(FontId::proportional(12.5))
                        .color(theme::TEXT_PRIMARY),
                )
                .on_hover_text(sanitize_display(&full, 900));
            }
        }
        let pill_space = (left_rect.right() - left.min_rect().right() - 6.0).max(0.0);
        if pill_space > 74.0 {
            theme::pill(
                &mut left,
                "read-only",
                theme::SUCCESS,
                theme::SUCCESS_TINT,
                18.0,
            );
        }
        if close {
            self.clear_preview();
            return PreviewUiAction::Close;
        }
        if show_terminal {
            return PreviewUiAction::ShowTerminal;
        }
        if refresh {
            self.refresh_preview();
        }
        if copy_path {
            if let Some(outcome) = self.apply_preview_action(PreviewAction::CopyPath, ui, log) {
                return outcome;
            }
        }
        theme::paint_card_border(ui, card_rect, layout);

        let meta_height = if layout.hide_preview_meta { 0.0 } else { 18.0 };
        let meta_rect = egui::Rect::from_min_size(
            egui::pos2(card_rect.left(), header_rect.bottom()),
            egui::vec2(card_rect.width(), meta_height),
        );
        let body_top = meta_rect.bottom();
        let body_rect =
            egui::Rect::from_min_max(egui::pos2(card_rect.left(), body_top), card_rect.max);
        let mut body =
            ui.new_child(UiBuilder::new().max_rect(body_rect.shrink2(egui::vec2(10.0, 6.0))));
        if !layout.hide_preview_meta {
            if let Some(document) = &self.preview.document {
                let meta = preview_meta(document, self.preview.refreshed_at, layout.compact);
                let mut meta_ui = ui.new_child(
                    UiBuilder::new()
                        .max_rect(meta_rect.shrink2(egui::vec2(layout.header_padding_x, 0.0)))
                        .layout(egui::Layout::left_to_right(Align::Center)),
                );
                let full_meta = meta.clone();
                meta_ui
                    .add(egui::Label::new(theme::mono(meta, 10.5, theme::TEXT_TERTIARY)).truncate())
                    .on_hover_text(full_meta);
            }
        }

        if self.preview.loading {
            body.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0).color(theme::ACCENT));
                ui.label(theme::body("Loading preview…"));
            });
        }
        if let Some(error) = self.preview.error.clone() {
            tone_strip(
                &mut body,
                layout,
                26.0,
                theme::ERROR_TINT,
                theme::ERROR,
                &sanitize_display(&error, 400),
                sanitize_display(&error, 700),
            );
            body.add_space(4.0);
        }
        let Some(document) = self.preview.document.as_ref() else {
            return PreviewUiAction::None;
        };
        if !document.is_text() {
            preview_state_card(&mut body, document, layout);
            return PreviewUiAction::None;
        }
        if document.lines.is_empty() {
            preview_empty_card(&mut body, document, layout);
            return PreviewUiAction::None;
        }
        preview_code_ui(&mut body, document);
        PreviewUiAction::None
    }

    fn apply_preview_action(
        &mut self,
        action: PreviewAction,
        ui: &Ui,
        log: &mut DiagnosticLog,
    ) -> Option<PreviewUiAction> {
        match action {
            PreviewAction::Refresh => {
                self.refresh_preview();
                None
            }
            PreviewAction::CopyPath => {
                if let Some(path) = &self.preview.path {
                    let text = sanitize_display(&path.display().to_string(), 4096);
                    ui.ctx().copy_text(text);
                    log.record(LogLevel::Info, "preview path copied");
                }
                None
            }
            PreviewAction::Close => {
                self.clear_preview();
                Some(PreviewUiAction::Close)
            }
        }
    }
}

fn paint_card_header(ui: &Ui, rect: egui::Rect, layout: &theme::Layout, fill: Color32) {
    let radius = CornerRadius {
        nw: layout.card_radius,
        ne: layout.card_radius,
        sw: 0,
        se: 0,
    };
    ui.painter().rect_filled(rect, radius, fill);
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.bottom() - 0.5),
            egui::pos2(rect.right(), rect.bottom() - 0.5),
        ],
        Stroke::new(1.0, theme::BORDER_HEADER),
    );
}

fn tone_strip(
    ui: &mut Ui,
    layout: &theme::Layout,
    height: f32,
    fill: Color32,
    tone: Color32,
    text: &str,
    tooltip: String,
) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height.max(24.0)),
        Sense::hover(),
    );
    ui.painter().rect(
        rect,
        CornerRadius::same(4),
        fill,
        Stroke::NONE,
        StrokeKind::Inside,
    );
    ui.painter().rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
        CornerRadius::same(2),
        tone,
    );
    let _ = layout;
    ui.painter().text(
        egui::pos2(rect.left() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        "!",
        FontId::proportional(10.5),
        tone,
    );
    theme::paint_ellipsized(
        ui.painter(),
        egui::pos2(rect.left() + 20.0, rect.center().y - 7.0),
        text,
        FontId::proportional(10.5),
        tone,
        (rect.width() - 28.0).max(20.0),
    );
    response.on_hover_text(tooltip);
}

fn preview_meta(
    document: &PreviewDocument,
    refreshed_at: Option<Instant>,
    compact: bool,
) -> String {
    let size = document
        .size
        .map(theme::format_bytes)
        .unwrap_or_else(|| "size unknown".to_owned());
    let line_count = document.line_count();
    let lines = if line_count == 1 {
        "1 line".to_owned()
    } else {
        format!("{line_count} lines")
    };
    if compact {
        return format!("{size} · {lines}");
    }
    let syntax = document
        .syntax
        .as_deref()
        .map(|syntax| {
            let mut characters = syntax.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
                None => "Plain text".to_owned(),
            }
        })
        .unwrap_or_else(|| "Plain text".to_owned());
    let updated = refreshed_at
        .map(|instant| format!(" · updated {:.1}s ago", instant.elapsed().as_secs_f32()))
        .unwrap_or_default();
    format!("{size} · {syntax} · {lines}{updated}")
}

fn preview_state_card(ui: &mut Ui, document: &PreviewDocument, layout: &theme::Layout) {
    let (fill, tone) = match document.kind {
        PreviewKind::Binary | PreviewKind::TooLarge => (theme::WARNING_TINT, theme::WARNING),
        _ => (theme::ERROR_TINT, theme::ERROR),
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, tone))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.label(
                RichText::new(preview_kind_heading(document.kind))
                    .font(FontId::proportional(13.0))
                    .color(tone),
            );
            if let Some(message) = &document.message {
                ui.label(theme::body(sanitize_display(message, 600)));
            }
            if let Some(size) = document.size {
                ui.label(theme::micro(theme::format_bytes(size)));
            }
            let _ = layout;
        });
}

fn preview_empty_card(ui: &mut Ui, document: &PreviewDocument, _layout: &theme::Layout) {
    egui::Frame::new()
        .fill(theme::BG_DEEP)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.label(
                RichText::new("Empty file")
                    .font(FontId::proportional(13.0))
                    .color(theme::TEXT_PRIMARY),
            );
            ui.label(theme::micro(theme::format_bytes(
                document.size.unwrap_or(0),
            )));
        });
}

fn preview_code_ui(ui: &mut Ui, document: &PreviewDocument) {
    const ROW_HEIGHT: f32 = 18.0;
    const GUTTER_WIDTH: f32 = 44.0;
    let body_rect = ui.available_rect_before_wrap();
    if body_rect.width() <= GUTTER_WIDTH || body_rect.height() <= 0.0 {
        return;
    }
    let gutter_rect = egui::Rect::from_min_max(
        body_rect.min,
        egui::pos2(body_rect.left() + GUTTER_WIDTH, body_rect.bottom()),
    );
    let code_rect = egui::Rect::from_min_max(
        egui::pos2(gutter_rect.right(), body_rect.top()),
        body_rect.max,
    );
    let mut code_ui = ui.new_child(UiBuilder::new().max_rect(code_rect));
    code_ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
    let output = ScrollArea::both()
        .id_salt("made-preview-code")
        .auto_shrink([false, false])
        .show_rows(
            &mut code_ui,
            ROW_HEIGHT,
            document.lines.len(),
            |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for index in range {
                    let line = &document.lines[index];
                    let mut job = LayoutJob::default();
                    job.wrap.max_width = f32::INFINITY;
                    if line.spans.is_empty() {
                        job.append(
                            " ",
                            0.0,
                            TextFormat {
                                font_id: FontId::monospace(13.0),
                                color: theme::TEXT_SECONDARY,
                                line_height: Some(ROW_HEIGHT),
                                ..Default::default()
                            },
                        );
                    } else {
                        for span in &line.spans {
                            let color =
                                Color32::from_rgb(span.color[0], span.color[1], span.color[2]);
                            job.append(
                                &span.text,
                                0.0,
                                TextFormat {
                                    font_id: FontId::monospace(13.0),
                                    color,
                                    italics: span.italic,
                                    underline: if span.underline {
                                        Stroke::new(1.0, color)
                                    } else {
                                        Stroke::NONE
                                    },
                                    line_height: Some(ROW_HEIGHT),
                                    ..Default::default()
                                },
                            );
                        }
                    }
                    ui.add(Label::new(job).selectable(false));
                }
            },
        );

    // The gutter is painted after the code viewport and is clipped to its own
    // fixed column, so horizontal code scrolling never moves line numbers.
    let painter = ui.painter().with_clip_rect(gutter_rect);
    painter.rect_filled(gutter_rect, 0.0, theme::BG_DEEP);
    painter.line_segment(
        [
            egui::pos2(gutter_rect.right() - 0.5, gutter_rect.top()),
            egui::pos2(gutter_rect.right() - 0.5, gutter_rect.bottom()),
        ],
        Stroke::new(1.0, theme::BORDER_HEADER),
    );
    let offset = output.state.clamped_offset().y;
    let first = ((offset / ROW_HEIGHT).floor() as isize - 1).max(0) as usize;
    let visible = (gutter_rect.height() / ROW_HEIGHT).ceil() as usize + 3;
    let last = (first + visible).min(document.lines.len());
    for index in first..last {
        let top = output.inner_rect.top() + index as f32 * ROW_HEIGHT - offset;
        if top + ROW_HEIGHT < gutter_rect.top() || top > gutter_rect.bottom() {
            continue;
        }
        painter.text(
            egui::pos2(gutter_rect.right() - 8.0, top + 2.0),
            Align2::RIGHT_TOP,
            format!("{:>5}", index + 1),
            FontId::monospace(10.5),
            theme::TEXT_TERTIARY,
        );
    }
}

fn preview_shortcut_action_from_input(ctx: &egui::Context) -> Option<PreviewAction> {
    let events = ctx.input(|input| input.events.clone());
    for event in events {
        if let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        {
            if let Some(action) = preview_shortcut_action(key, modifiers) {
                return Some(action);
            }
        }
    }
    None
}

fn preview_kind_heading(kind: PreviewKind) -> &'static str {
    match kind {
        PreviewKind::Binary => "Binary file",
        PreviewKind::TooLarge => "File too large",
        PreviewKind::InvalidUtf8 => "Unsupported text encoding",
        PreviewKind::Missing => "File unavailable",
        PreviewKind::Unreadable => "File unreadable",
        PreviewKind::Forbidden => "File outside the workspace",
        PreviewKind::Text => "Preview",
    }
}

fn root_error_message(path: &Path, error: &PathError) -> String {
    sanitize_display(
        &format!(
            "The explorer cannot open {}: {error}",
            sanitize_display(&path.display().to_string(), 512)
        ),
        700,
    )
}

/// Plain-text rendering helper used by tests: return the display text of a
/// document. It never interprets Markdown, HTML, or ANSI.
pub fn document_text(document: &PreviewDocument) -> String {
    document
        .lines
        .iter()
        .map(PreviewLine::text)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn dummy_document(path: &Path) -> PreviewDocument {
        PreviewDocument {
            path: path.to_path_buf(),
            kind: PreviewKind::Text,
            size: Some(4),
            message: None,
            syntax: None,
            plain_text: true,
            lines: vec![PreviewLine {
                spans: vec![PreviewSpan {
                    text: "old".to_owned(),
                    color: [1, 2, 3],
                    bold: false,
                    italic: false,
                    underline: false,
                }],
            }],
        }
    }

    fn output_text(output: &egui::FullOutput) -> String {
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
    fn tree_queue_saturation_keeps_cached_rows_and_does_not_install_errors() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("keep.txt"), "keep").unwrap();
        let mut pane = ExplorerPane::new(temp.path(), RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !pane.is_root_ready() {
            pane.tick(&mut log);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(pane.is_root_ready());
        let rows_before = pane.visible_row_count();
        assert!(rows_before >= 1);

        pane.tree_worker = TreeWorker::blocked();
        if let Some(root) = pane.root.clone() {
            pane.queue_reload(root);
        }
        assert!(!pane.pending_tree_loads.is_empty());
        assert_eq!(pane.visible_row_count(), rows_before);
        assert!(pane.root_error().is_none());
        pane.retry_pending_tree_loads();
        assert_eq!(pane.visible_row_count(), rows_before);
    }

    #[test]
    fn low_contrast_preview_spans_are_lifted_to_aa_before_rendering() {
        let background = [theme::BG_CARD.r(), theme::BG_CARD.g(), theme::BG_CARD.b()];
        let lifted = ensure_preview_contrast([30, 32, 34], background);
        assert!(
            contrast_ratio(lifted, background) >= 4.5,
            "adjusted syntax color still fails AA: {lifted:?}"
        );
        // A color that already passes is left untouched.
        let readable = ensure_preview_contrast([230, 232, 236], background);
        assert_eq!(readable, [230, 232, 236]);
    }

    #[test]
    fn preview_clears_content_when_the_selected_path_changes() {
        let mut preview = PreviewState::default();
        let first = Path::new("/tmp/first.txt");
        let second = Path::new("/tmp/second.txt");
        preview.begin_request(first, 1);
        preview.document = Some(dummy_document(first));
        preview.begin_request(first, 2);
        assert!(
            preview.document.is_some(),
            "same-path refresh retains content"
        );
        preview.begin_request(second, 3);
        assert!(
            preview.document.is_none(),
            "new path must clear stale content"
        );
    }

    #[test]
    fn pending_tree_loads_coalesce_by_directory_and_stay_bounded() {
        let temp = tempfile::tempdir().unwrap();
        for index in 0..(MAX_PENDING_TREE_LOADS + 5) {
            fs::create_dir_all(temp.path().join(format!("d{index}"))).unwrap();
        }
        let mut tree = TreeModel::new(temp.path(), RootGeneration::INITIAL).unwrap();
        let guard = tree.path_guard().clone();
        let first = tree.begin_load(temp.path()).unwrap();
        let mut pending = PendingTreeLoads::default();
        pending.offer(first.clone(), guard.clone());
        pending.offer(first.clone(), guard.clone());
        assert_eq!(pending.len(), 1, "duplicate directory coalesces");
        for index in 0..(MAX_PENDING_TREE_LOADS + 5) {
            let dir = temp.path().join(format!("d{index}"));
            let request = tree.begin_load(&dir).unwrap();
            pending.offer(request, guard.clone());
        }
        assert_eq!(pending.len(), MAX_PENDING_TREE_LOADS);
        assert_eq!(pending.take().len(), MAX_PENDING_TREE_LOADS);
        assert_eq!(pending.len(), 0);
    }

    #[test]
    fn stale_watcher_install_result_is_discarded() {
        let temp = tempfile::tempdir().unwrap();
        let mut pane = ExplorerPane::new(temp.path(), RootGeneration::new(4));
        pane.watcher_loading = true;
        pane.apply_watcher_result(WatcherResult {
            generation: RootGeneration::new(3),
            watcher: Err("stale".to_owned()),
        });
        assert!(pane.watcher_error().is_none());
        assert!(pane.watcher_loading());
    }

    fn wait_for_root(pane: &mut ExplorerPane, log: &mut DiagnosticLog) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !pane.is_root_ready() {
            pane.tick(log);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(pane.is_root_ready());
    }

    fn wait_for_preview(pane: &mut ExplorerPane, log: &mut DiagnosticLog) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && pane.preview_document().is_none() {
            pane.tick(log);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(pane.preview_document().is_some());
    }

    fn text_positions(output: &egui::FullOutput) -> Vec<(String, egui::Pos2)> {
        fn visit(shape: &egui::Shape, positions: &mut Vec<(String, egui::Pos2)>) {
            match shape {
                egui::Shape::Text(text) => positions.push((text.galley.job.text.clone(), text.pos)),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        visit(shape, positions);
                    }
                }
                _ => {}
            }
        }
        let mut positions = Vec::new();
        for clipped in &output.shapes {
            visit(&clipped.shape, &mut positions);
        }
        positions
    }

    #[test]
    fn opening_a_file_requests_the_preview_surface_and_focus() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("main.rs");
        fs::write(&file, "fn main() {}\n").unwrap();
        let mut pane = ExplorerPane::new(temp.path(), RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);
        pane.open_preview(file.clone());
        assert!(pane.take_preview_opened(), "file open must select preview");
        assert!(!pane.take_preview_opened(), "the event is consumed once");
        assert_eq!(pane.preview_path(), Some(file.as_path()));
        wait_for_preview(&mut pane, &mut log);

        let ctx = egui::Context::default();
        theme::install(&ctx);
        pane.set_ui_scope(egui::Id::unique("test-preview-scope"));
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 620.0),
            )),
            ..Default::default()
        };
        let mut action = None;
        let mut output = ctx.run_ui(raw, |ui| {
            action = Some(pane.preview_ui(ui, &mut log, &theme::Layout::for_window(900.0, 620.0)));
        });
        output.textures_delta.clear();
        assert_eq!(action, Some(PreviewUiAction::None));
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(pane.preview_focus_id())
        );
        let text = output_text(&output);
        assert!(text.contains("Close"));
        assert!(text.contains("Refresh"));
        assert!(text.contains("Copy path"));
        assert!(text.contains("read-only"));
        assert!(!text.contains("Select a file"));
    }

    #[test]
    fn selecting_an_error_row_never_opens_or_replaces_a_preview() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("open.txt");
        fs::write(&file, "open").unwrap();
        let mut pane = ExplorerPane::new(temp.path(), RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);
        pane.open_preview(file.clone());
        wait_for_preview(&mut pane, &mut log);
        pane.take_preview_opened();

        pane.select_row_click(&TreeRow {
            path: temp.path().join("unreadable"),
            name: "unreadable".to_owned(),
            depth: 0,
            kind: EntryKind::Error,
            expanded: false,
            selected: false,
            error: Some("permission denied".to_owned()),
        });
        assert_eq!(pane.preview_path(), Some(file.as_path()));
        assert!(!pane.take_preview_opened());
        assert_eq!(pane.preview.error, None);
    }

    #[test]
    fn preview_line_numbers_stay_fixed_when_code_scrolls_horizontally() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("long.txt");
        fs::write(&file, format!("{}\nsecond\n", "x".repeat(200))).unwrap();
        let mut pane = ExplorerPane::new(temp.path(), RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);
        pane.open_preview(file);
        wait_for_preview(&mut pane, &mut log);
        let ctx = egui::Context::default();
        theme::install(&ctx);
        pane.set_ui_scope(egui::Id::unique("fixed-gutter"));
        let mut render = |events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                let _ = pane.preview_ui(ui, &mut log, &theme::Layout::for_window(1280.0, 800.0));
            });
            output.textures_delta.clear();
            output
        };
        let initial = text_positions(&render(vec![egui::Event::PointerMoved(egui::pos2(
            600.0, 300.0,
        ))]));
        let line_one = initial
            .iter()
            .find(|(text, _)| text.trim() == "1")
            .map(|(_, pos)| *pos)
            .expect("line number 1");
        for _ in 0..3 {
            let _ = render(vec![
                egui::Event::PointerMoved(egui::pos2(600.0, 300.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(-160.0, 0.0),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                },
            ]);
        }
        let scrolled = text_positions(&render(Vec::new()));
        let line_one_after = scrolled
            .iter()
            .find(|(text, _)| text.trim() == "1")
            .map(|(_, pos)| *pos)
            .expect("line number 1 after scroll");
        assert!(
            (line_one_after.x - line_one.x).abs() < 0.01,
            "gutter moved horizontally: {line_one:?} -> {line_one_after:?}"
        );
        let code_before = initial
            .iter()
            .find(|(text, _)| text.contains("xxxx"))
            .map(|(_, pos)| *pos)
            .expect("code line");
        let code_after = scrolled
            .iter()
            .find(|(text, _)| text.contains("xxxx"))
            .map(|(_, pos)| *pos)
            .expect("code line after horizontal scroll");
        assert!(
            code_after.x < code_before.x - 1.0,
            "code did not scroll horizontally: {code_before:?} -> {code_after:?}"
        );
    }

    #[test]
    fn compact_preview_keeps_close_and_read_only_reachable() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("compact.rs");
        fs::write(&file, "fn main() {}\n").unwrap();
        let mut pane = ExplorerPane::new(temp.path(), RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);
        pane.open_preview(file);
        wait_for_preview(&mut pane, &mut log);
        let ctx = egui::Context::default();
        theme::install(&ctx);
        pane.set_ui_scope(egui::Id::unique("compact-preview-scope"));
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(410.0, 392.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            let _ = pane.preview_ui(ui, &mut log, &theme::Layout::for_window(720.0, 480.0));
        });
        output.textures_delta.clear();
        let text = output_text(&output);
        assert!(text.contains("PREVIEW"));
        assert!(text.contains("read-only"));
        assert!(text.contains("Close"));
        assert!(text.contains("…"));
        assert!(ctx
            .read_response(pane.preview_action_id(PreviewAction::Close))
            .is_some());
    }

    fn tree_key_frame(
        ctx: &egui::Context,
        pane: &mut ExplorerPane,
        log: &mut DiagnosticLog,
        width: f32,
        height: f32,
        keys: &[egui::Key],
    ) -> egui::FullOutput {
        let events = keys
            .iter()
            .map(|key| egui::Event::Key {
                key: *key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .collect();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, height),
            )),
            events,
            ..Default::default()
        };
        let layout = theme::Layout::for_window(width, height);
        let mut output = ctx.run_ui(raw, |ui| pane.tree_ui(ui, log, &layout));
        output.textures_delta.clear();
        output
    }

    #[test]
    fn keyboard_moves_selection_and_activates_only_on_enter_at_both_sizes() {
        for (width, height) in [(1280.0, 800.0), (720.0, 480.0)] {
            let temp = tempfile::tempdir().unwrap();
            let directory = temp.path().join("dir");
            fs::create_dir(&directory).unwrap();
            fs::write(directory.join("nested.txt"), "nested").unwrap();
            let file = temp.path().join("b.txt");
            fs::write(&file, "file").unwrap();
            let expected_directory = directory.canonicalize().unwrap();
            let expected_file = file.canonicalize().unwrap();
            let mut pane = ExplorerPane::new(temp.path(), RootGeneration::INITIAL);
            let mut log = DiagnosticLog::new();
            wait_for_root(&mut pane, &mut log);
            let ctx = egui::Context::default();
            theme::install(&ctx);
            pane.set_ui_scope(egui::Id::unique(("keyboard-tree", width as u32)));
            tree_key_frame(&ctx, &mut pane, &mut log, width, height, &[]);
            let tree_id = pane.tree_focus_id();
            let container = ctx.read_response(tree_id).expect("tree container");
            assert!(container.sense.is_focusable());
            let row = ctx
                .read_response(pane.row_widget_id(0))
                .expect("first tree row");
            assert!(
                !row.sense.is_focusable(),
                "rows must not be individual Tab stops"
            );
            ctx.memory_mut(|memory| memory.request_focus(tree_id));
            tree_key_frame(&ctx, &mut pane, &mut log, width, height, &[]);
            tree_key_frame(&ctx, &mut pane, &mut log, width, height, &[]);
            assert_eq!(
                pane.selected_path(),
                Some(expected_directory.as_path()),
                "focusing the tree must select the first row without activating it"
            );
            assert!(pane.preview_path().is_none());

            tree_key_frame(
                &ctx,
                &mut pane,
                &mut log,
                width,
                height,
                &[egui::Key::ArrowDown],
            );
            assert_eq!(pane.selected_path(), Some(expected_file.as_path()));
            assert!(pane.preview_path().is_none());
            tree_key_frame(&ctx, &mut pane, &mut log, width, height, &[egui::Key::Home]);
            assert_eq!(pane.selected_path(), Some(expected_directory.as_path()));
            assert!(pane.preview_path().is_none());
            tree_key_frame(&ctx, &mut pane, &mut log, width, height, &[egui::Key::End]);
            assert_eq!(pane.selected_path(), Some(expected_file.as_path()));
            assert!(pane.preview_path().is_none());
            tree_key_frame(&ctx, &mut pane, &mut log, width, height, &[egui::Key::Home]);
            assert_eq!(pane.selected_path(), Some(expected_directory.as_path()));
            assert!(pane.preview_path().is_none());

            tree_key_frame(
                &ctx,
                &mut pane,
                &mut log,
                width,
                height,
                &[egui::Key::Enter],
            );
            assert!(
                pane.preview_path().is_none(),
                "Enter on a directory must toggle it, not open a preview"
            );
            let deadline = Instant::now() + Duration::from_secs(3);
            while pane.visible_row_count() < 3 && Instant::now() < deadline {
                pane.tick(&mut log);
                std::thread::sleep(Duration::from_millis(5));
            }
            tree_key_frame(&ctx, &mut pane, &mut log, width, height, &[]);
            assert_eq!(pane.visible_row_count(), 3);
            tree_key_frame(
                &ctx,
                &mut pane,
                &mut log,
                width,
                height,
                &[egui::Key::ArrowDown],
            );
            let expected_nested = expected_directory.join("nested.txt");
            assert_eq!(pane.selected_path(), Some(expected_nested.as_path()));
            assert!(pane.preview_path().is_none());
            tree_key_frame(
                &ctx,
                &mut pane,
                &mut log,
                width,
                height,
                &[egui::Key::PageDown],
            );
            assert_eq!(pane.selected_path(), Some(expected_file.as_path()));
            assert!(pane.preview_path().is_none());
            tree_key_frame(&ctx, &mut pane, &mut log, width, height, &[egui::Key::Home]);
            tree_key_frame(&ctx, &mut pane, &mut log, width, height, &[egui::Key::End]);
            assert!(pane.preview_path().is_none());
            tree_key_frame(
                &ctx,
                &mut pane,
                &mut log,
                width,
                height,
                &[egui::Key::Enter],
            );
            assert_eq!(pane.preview_path(), Some(expected_file.as_path()));
        }
    }

    #[test]
    fn ten_thousand_row_selection_updates_flags_without_rebuilding() {
        let temp = tempfile::tempdir().unwrap();
        for index in 0..10_000 {
            fs::write(temp.path().join(format!("file-{index:05}.txt")), "x").unwrap();
        }
        let mut pane = ExplorerPane::new(temp.path(), RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);
        assert_eq!(pane.visible_row_count(), 10_000);
        let rebuilds = pane.row_rebuilds();
        // Warm caches before measuring; the contract is steady-state selection
        // cost, not first-touch page faults from creating 10k files.
        for _ in 0..200 {
            pane.move_selection(1);
        }
        // Median of three p95 batches keeps the 5ms contract while tolerating
        // one scheduler- or I/O-noisy batch on hosted runners.
        let mut p95s = Vec::with_capacity(3);
        for _ in 0..3 {
            let mut samples = Vec::with_capacity(200);
            for _ in 0..200 {
                let started = Instant::now();
                pane.move_selection(1);
                samples.push(started.elapsed());
            }
            samples.sort_unstable();
            p95s.push(samples[189]);
        }
        p95s.sort_unstable();
        let p95 = p95s[1];
        // Test-local guard. The product contract is a 60 Hz frame p95 below
        // 16.667 ms (wiki/performance.md). Windows and macOS x86_64 CI debug
        // runners exceed the tighter 5 ms guard (macOS x86_64 measured 5.465
        // ms); macOS arm64 and Linux keep 5 ms. One frame is the defensible
        // ceiling where the slower runners need it.
        let budget = if cfg!(any(
            windows,
            all(target_os = "macos", target_arch = "x86_64")
        )) {
            Duration::from_millis(16)
        } else {
            Duration::from_millis(5)
        };
        assert_eq!(
            pane.row_rebuilds(),
            rebuilds,
            "selection movement must not rebuild all rows"
        );
        assert!(
            p95 < budget,
            "10k-row selection median p95 was {p95:?} (budget {budget:?})"
        );
        assert!(pane.preview_path().is_none());
        assert!(pane.selected_path().is_some());
        let rebuilds_before_click = pane.row_rebuilds();
        let first = pane.rows[0].clone();
        pane.select_row_click(&first);
        assert_eq!(
            pane.row_rebuilds(),
            rebuilds_before_click,
            "click selection must not rebuild all rows"
        );
    }

    #[test]
    fn a_new_root_clears_the_previous_preview() {
        let temp = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = temp.path().join("open.txt");
        fs::write(&file, "open").unwrap();
        let mut pane = ExplorerPane::new(temp.path(), RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);
        pane.open_preview(file);
        wait_for_preview(&mut pane, &mut log);
        pane.take_preview_opened();

        pane.install_root(other.path());
        assert!(pane.preview_path().is_none());
        assert!(pane.preview_document().is_none());
        assert!(!pane.take_preview_opened());
        assert_eq!(
            pane.root(),
            Some(other.path().canonicalize().unwrap().as_path())
        );
    }
}
