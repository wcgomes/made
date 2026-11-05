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

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
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
use crate::explorer_icons;
use crate::sanitize::sanitize_display;
use crate::theme;
use crate::worktree::{ChangeKind, FolderMark, GitStatusSnapshot};

/// Edge of the square icon slot that file and folder rows reserve, in points.
const ENTRY_ICON_SIZE: f32 = 16.0;

/// Status-dot hover strip width. Full row height, flush to the row's right edge.
const STATUS_DOT_HIT_WIDTH: f32 = 28.0;

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
            } => Self::from_text(path, &content, content_size, highlighter),
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

    /// Build a display document from a completed remote preview read.
    ///
    /// The path is used only for display and syntax selection; no host file is
    /// read. Text size comes from the received byte length because the remote
    /// preview reply carries content, not metadata.
    pub fn from_remote(
        path: &made_remote::protocol::RemotePath,
        kind: made_remote::protocol::PreviewKind,
        data: &[u8],
        highlighter: &Highlighter,
    ) -> Self {
        let display = path
            .to_utf8()
            .map(str::to_owned)
            .unwrap_or_else(|_| String::from_utf8_lossy(path.as_bytes()).into_owned());
        let path = PathBuf::from(display);
        if kind == made_remote::protocol::PreviewKind::Text {
            if let Ok(content) = std::str::from_utf8(data) {
                return Self::from_text(path, content, data.len() as u64, highlighter);
            }
        }
        let (kind, message) = match kind {
            made_remote::protocol::PreviewKind::Text
            | made_remote::protocol::PreviewKind::InvalidUtf8 => (
                PreviewKind::InvalidUtf8,
                "The file is not valid UTF-8; raw bytes are not shown.",
            ),
            made_remote::protocol::PreviewKind::Binary => (
                PreviewKind::Binary,
                "Binary file. Preview shows no raw bytes.",
            ),
            made_remote::protocol::PreviewKind::TooLarge => {
                (PreviewKind::TooLarge, "File is too large to preview.")
            }
            made_remote::protocol::PreviewKind::Missing => {
                (PreviewKind::Missing, "The file no longer exists.")
            }
            made_remote::protocol::PreviewKind::Unreadable => {
                (PreviewKind::Unreadable, "The file could not be read.")
            }
            made_remote::protocol::PreviewKind::NotRegular => {
                (PreviewKind::Unreadable, "The path is not a regular file.")
            }
        };
        Self {
            path,
            kind,
            size: None,
            message: Some(message.to_owned()),
            syntax: None,
            plain_text: true,
            lines: Vec::new(),
        }
    }

    fn from_text(path: PathBuf, content: &str, size: u64, highlighter: &Highlighter) -> Self {
        let highlighted = highlighter.highlight(&path, content);
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
            kind: PreviewKind::Text,
            size: Some(size),
            message: None,
            syntax: highlighted.syntax,
            plain_text: highlighted.plain_text,
            lines,
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

    #[cfg(test)]
    fn blocked() -> Self {
        let (jobs, job_rx) = mpsc::sync_channel::<WatcherJob>(1);
        drop(job_rx);
        let (_result_tx, results) = mpsc::sync_channel::<WatcherResult>(1);
        Self { jobs, results }
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

/// Display state for the shared full-left preview card.
pub(crate) struct PreviewCardState<'a> {
    pub(crate) path_name: Option<&'a str>,
    pub(crate) path_full: Option<&'a str>,
    pub(crate) document: Option<&'a PreviewDocument>,
    pub(crate) loading: bool,
    pub(crate) error: Option<&'a str>,
    pub(crate) refreshed_at: Option<Instant>,
    pub(crate) focus_pending: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PreviewCardResult {
    pub(crate) surface: PreviewUiAction,
    pub(crate) refresh: bool,
    pub(crate) copy_path: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PreviewCardIds {
    pub(crate) focus: egui::Id,
    pub(crate) close: egui::Id,
    pub(crate) terminal: egui::Id,
    pub(crate) refresh: egui::Id,
    pub(crate) copy: egui::Id,
}

/// Which explorer body the shared card is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExplorerViewKind {
    Files,
    Git,
}

/// One interaction chosen from the shared explorer card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExplorerRowAction {
    /// Pointer activation: select and then activate the row.
    Click(usize),
    /// Keyboard movement: select without opening or toggling.
    Select(usize),
    /// Keyboard expand/collapse: select and toggle a directory.
    Toggle(usize),
    /// Enter/Space: activate the already selected row.
    Activate(usize),
}

#[derive(Debug, Default)]
struct ExplorerCardResult {
    switch: Option<ExplorerViewKind>,
    row: Option<ExplorerRowAction>,
    git_row: Option<(usize, usize)>,
    retry: bool,
}

/// A tooltip source for a tree row. Local rows keep their path borrowed; remote
/// rows carry an already-rendered POSIX path.
enum RowTooltip<'a> {
    Path(&'a Path),
    Text(&'a str),
}

impl RowTooltip<'_> {
    fn text(&self) -> String {
        match self {
            Self::Path(path) => path.display().to_string(),
            Self::Text(text) => (*text).to_owned(),
        }
    }
}

/// A change mark shown at the right edge of a tree row.
enum RowMark<'a> {
    Local(&'a FolderMark),
    Remote(&'a [ChangeKind]),
}

impl RowMark<'_> {
    fn color(&self) -> Color32 {
        match self {
            Self::Local(mark) => folder_mark_color(mark),
            Self::Remote(kinds) => change_kinds_color(kinds),
        }
    }

    fn tooltip(&self) -> String {
        match self {
            Self::Local(mark) => mark.tooltip(),
            Self::Remote(kinds) => kinds
                .iter()
                .map(|kind| kind.label())
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

/// One visible tree row, independent of whether it came from the host
/// filesystem or a remote projection.
struct ExplorerRowView<'a> {
    name: Cow<'a, str>,
    depth: usize,
    kind: EntryKind,
    expanded: bool,
    selected: bool,
    error: Option<&'a str>,
    tooltip: RowTooltip<'a>,
    mark: Option<RowMark<'a>>,
    icon: Option<explorer_icons::IconId>,
}

/// Lazily supplies visible rows so the shared renderer can stay virtualized.
trait ExplorerRowSource {
    fn row_count(&self) -> usize;
    fn row(&self, index: usize) -> ExplorerRowView<'_>;
    fn selected_index(&self) -> Option<usize> {
        (0..self.row_count()).find(|&index| self.row(index).selected)
    }
}

struct GitRowView {
    name: String,
    kind: ChangeKind,
    selected: bool,
    icon: Option<explorer_icons::IconId>,
    tooltip: String,
    original: Option<String>,
}

struct GitGroupView {
    title: &'static str,
    rows: Vec<GitRowView>,
}

struct GitBodyView {
    has_status: bool,
    failed: bool,
    stale: bool,
    groups: Vec<GitGroupView>,
}

enum ExplorerTreeBody<'a> {
    Rows(&'a dyn ExplorerRowSource),
    Loading,
    Empty,
    Error(String),
}

/// Everything the shared explorer card needs to paint one frame.
struct ExplorerCardView<'a> {
    git_open: bool,
    show_icon: bool,
    root_name: Option<(String, String)>,
    git_header: Option<&'a GitHeader>,
    source_control_tooltip: String,
    source_control_rings: theme::SourceControlRings,
    root_unavailable: Option<String>,
    tree: ExplorerTreeBody<'a>,
    git: Option<GitBodyView>,
    scroll_to_selection: bool,
    /// Only offer Retry when the caller can actually re-request the root.
    retry_available: bool,
}

/// Branch and worktree facts the explorer header needs. Made never invents a branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitHeader {
    /// Icon is shown only when metadata is known and the root is a worktree.
    pub show_icon: bool,
    /// Attached branch, when metadata reported one.
    pub branch: Option<String>,
    /// Whether HEAD is detached.
    pub detached: bool,
    /// `rev-parse --short=8 HEAD` when detached. May be longer than 8 characters.
    pub commit_abbrev: Option<String>,
    /// Commit subject when detached. Empty means the subject was empty.
    pub commit_subject: Option<String>,
    /// Whether this directory is a linked worktree.
    pub is_linked_worktree: bool,
    /// Worktree directory name. Shown only when it differs from the visible head.
    pub directory_name: Option<String>,
}

/// Status snapshot pushed in from the workbench worker. The explorer never runs git.
#[derive(Debug, Clone, Default)]
pub struct ExplorerGitChrome {
    /// Header facts for the icon and branch line.
    pub header: GitHeader,
    /// Latest successful status, if any.
    pub status: Option<GitStatusSnapshot>,
    /// The last read failed and there is no snapshot to keep.
    pub failed: bool,
    /// A status read is in flight and no snapshot is ready yet.
    pub loading: bool,
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
    /// Icon for each row in `rows`, resolved once when the rows are rebuilt.
    row_icons: Vec<Option<explorer_icons::IconId>>,
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
    native_backdrop_applied: bool,
    source_control_open: bool,
    source_control_opened: bool,
    fs_activity: bool,
    git_header: GitHeader,
    git_status: Option<GitStatusSnapshot>,
    git_status_failed: bool,
    git_status_loading: bool,
    git_selected: Option<PathBuf>,
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
            row_icons: Vec::new(),
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
            native_backdrop_applied: false,
            source_control_open: false,
            source_control_opened: false,
            fs_activity: false,
            git_header: GitHeader::default(),
            git_status: None,
            git_status_failed: false,
            git_status_loading: false,
            git_selected: None,
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
        self.row_icons.clear();
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
        self.fs_activity = true;
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
        self.row_icons = self
            .rows
            .iter()
            .map(|row| explorer_icons::resolve(&row.name, row.kind, row.expanded))
            .collect();
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

    pub fn tree_ui(&mut self, ui: &mut Ui, log: &mut DiagnosticLog, layout: &theme::Layout) {
        self.ensure_rows();
        self.explorer_card(ui, log, layout);
    }

    /// Select the native-backdrop Explorer tint for the next paint.
    pub fn set_native_backdrop_applied(&mut self, applied: bool) {
        self.native_backdrop_applied = applied;
    }

    /// Replace the read-only git chrome. Leaving a repository returns to the file tree.
    pub fn apply_git_chrome(&mut self, chrome: ExplorerGitChrome) {
        if !chrome.header.show_icon {
            self.source_control_open = false;
        }
        self.git_header = chrome.header;
        self.git_status = chrome.status;
        self.git_status_failed = chrome.failed;
        self.git_status_loading = chrome.loading;
    }

    /// Whether the source-control view is showing. Per workspace, in memory.
    pub fn source_control_open(&self) -> bool {
        self.source_control_open && self.git_header.show_icon
    }

    /// Take the "user just opened source control" edge so the workbench can refresh.
    pub fn take_source_control_opened(&mut self) -> bool {
        std::mem::take(&mut self.source_control_opened)
    }

    /// Take filesystem activity observed since the last poll.
    pub fn take_fs_activity(&mut self) -> bool {
        std::mem::take(&mut self.fs_activity)
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

    fn explorer_card(&mut self, ui: &mut Ui, _log: &mut DiagnosticLog, layout: &theme::Layout) {
        let git_open = self.source_control_open();
        let show_icon = self.git_header.show_icon;
        let root_name = if git_open {
            None
        } else {
            self.root.as_ref().map(|root| {
                let full = root.display().to_string();
                let name = root
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| full.clone());
                (name, full)
            })
        };
        let tooltip = source_control_tooltip(self.git_status.as_ref());
        let rings = source_control_rings(self.git_status.as_ref());
        let scroll_to_selection = std::mem::take(&mut self.scroll_to_selection);
        let files = self.git_status.as_ref().map(file_status_marks);
        let result = {
            let tree = if let Some(error) = self.root_error.as_ref() {
                ExplorerTreeBody::Error(error.clone())
            } else if self.rows.is_empty() {
                if self
                    .tree
                    .as_ref()
                    .is_some_and(|tree| tree.loading_dirs().next().is_some())
                {
                    ExplorerTreeBody::Loading
                } else {
                    ExplorerTreeBody::Empty
                }
            } else {
                ExplorerTreeBody::Rows(&LocalRows {
                    rows: &self.rows,
                    icons: &self.row_icons,
                    folders: self.git_status.as_ref().map(|status| &status.folders),
                    files: files.as_ref(),
                })
            };
            let git = git_open.then(|| {
                local_git_body(
                    self.git_status.as_ref(),
                    self.git_selected.as_deref(),
                    self.git_status_failed,
                )
            });
            let view = ExplorerCardView {
                git_open,
                show_icon,
                root_name,
                git_header: git_open.then_some(&self.git_header),
                source_control_tooltip: tooltip,
                source_control_rings: rings,
                root_unavailable: self.root_unavailable.clone(),
                tree,
                git,
                scroll_to_selection,
                retry_available: true,
            };
            explorer_card_ui(
                ui,
                &view,
                self.ui_scope,
                self.native_backdrop_applied,
                layout,
            )
        };
        self.apply_explorer_card_result(result);
    }

    fn apply_explorer_card_result(&mut self, result: ExplorerCardResult) {
        if let Some(switch) = result.switch {
            match switch {
                ExplorerViewKind::Files => self.source_control_open = false,
                ExplorerViewKind::Git => {
                    self.source_control_open = true;
                    self.source_control_opened = true;
                }
            }
        }
        if result.retry {
            self.retry();
        }
        if let Some(action) = result.row {
            match action {
                ExplorerRowAction::Click(index) => {
                    if let Some(row) = self.rows.get(index).cloned() {
                        self.select_row_click(&row);
                        // Pointer selection must not recenter the tree; only
                        // keyboard/programmatic movement reveals the row.
                        self.scroll_to_selection = false;
                    }
                }
                ExplorerRowAction::Select(index) => {
                    if let Some(path) = self.rows.get(index).map(|row| row.path.clone()) {
                        self.apply_selection_path(&path);
                    }
                }
                ExplorerRowAction::Toggle(index) => {
                    if let Some(row) = self.rows.get(index).cloned() {
                        self.apply_selection_path(&row.path);
                        self.toggle_dir(row.path.clone());
                        self.rows_dirty = true;
                    }
                }
                ExplorerRowAction::Activate(index) => {
                    if let Some(row) = self.rows.get(index).cloned() {
                        self.activate_row(&row);
                    }
                }
            }
        }
        if let Some((group_index, row_index)) = result.git_row {
            if let Some(row) = self
                .git_status
                .as_ref()
                .and_then(|status| status.groups.get(group_index))
                .and_then(|group| group.rows.get(row_index))
                .cloned()
            {
                self.git_selected = Some(row.path.clone());
                if row.opens_preview() {
                    self.open_preview(row.path);
                }
            }
        }
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
        let path = self.preview.path.as_ref().expect("preview path checked");
        let full = path.display().to_string();
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| full.clone());
        let focus_pending = self.preview_focus_pending;
        let result = preview_card_ui(
            ui,
            PreviewCardState {
                path_name: Some(&name),
                path_full: Some(&full),
                document: self.preview.document.as_ref(),
                loading: self.preview.loading,
                error: self.preview.error.as_deref(),
                refreshed_at: self.preview.refreshed_at,
                focus_pending,
            },
            PreviewCardIds {
                focus: self.preview_focus_id(),
                close: self.preview_action_id(PreviewAction::Close),
                terminal: self.preview_terminal_id(),
                refresh: self.preview_action_id(PreviewAction::Refresh),
                copy: self.preview_action_id(PreviewAction::CopyPath),
            },
            layout,
        );
        self.preview_focus_pending = false;
        if result.copy_path {
            ui.ctx().copy_text(sanitize_display(&full, 4096));
            log.record(LogLevel::Info, "preview path copied");
        }
        if result.refresh {
            self.refresh_preview();
        }
        if result.surface == PreviewUiAction::Close {
            self.clear_preview();
        }
        result.surface
    }
}

struct LocalRows<'a> {
    rows: &'a [TreeRow],
    icons: &'a [Option<explorer_icons::IconId>],
    folders: Option<&'a BTreeMap<PathBuf, FolderMark>>,
    files: Option<&'a BTreeMap<PathBuf, FolderMark>>,
}

impl ExplorerRowSource for LocalRows<'_> {
    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn row(&self, index: usize) -> ExplorerRowView<'_> {
        let row = &self.rows[index];
        let mark = match row.kind {
            EntryKind::Directory => self
                .folders
                .and_then(|folders| folders.get(&row.path))
                .map(RowMark::Local),
            EntryKind::File => self
                .files
                .and_then(|files| files.get(&row.path))
                .map(RowMark::Local),
            _ => None,
        };
        ExplorerRowView {
            name: Cow::Borrowed(&row.name),
            depth: row.depth,
            kind: row.kind,
            expanded: row.expanded,
            selected: row.selected,
            error: row.error.as_deref(),
            tooltip: RowTooltip::Path(&row.path),
            mark,
            icon: self.icons.get(index).copied().flatten(),
        }
    }

    fn selected_index(&self) -> Option<usize> {
        self.rows.iter().position(|row| row.selected)
    }
}

fn local_git_body(
    status: Option<&GitStatusSnapshot>,
    selected: Option<&Path>,
    failed: bool,
) -> GitBodyView {
    let mut groups = Vec::new();
    if let Some(status) = status {
        for group in &status.groups {
            if group.rows.is_empty() {
                continue;
            }
            let rows = group
                .rows
                .iter()
                .map(|row| {
                    let entry_kind = if row.is_directory {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    };
                    let icon_name = row
                        .relative
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("");
                    GitRowView {
                        name: row.relative.display().to_string(),
                        kind: row.kind,
                        selected: selected == Some(row.path.as_path()),
                        icon: explorer_icons::resolve(icon_name, entry_kind, false),
                        tooltip: row.path.display().to_string(),
                        original: row
                            .original_relative
                            .as_ref()
                            .map(|path| path.display().to_string()),
                    }
                })
                .collect();
            groups.push(GitGroupView {
                title: group.title,
                rows,
            });
        }
    }
    GitBodyView {
        has_status: status.is_some(),
        failed,
        stale: false,
        groups,
    }
}

fn explorer_card_ui(
    ui: &mut Ui,
    view: &ExplorerCardView<'_>,
    scope: egui::Id,
    native_backdrop: bool,
    layout: &theme::Layout,
) -> ExplorerCardResult {
    let mut result = ExplorerCardResult::default();
    // Allocate the whole card so the resizable split panel does not
    // shrink-wrap to the tree content on the second frame.
    let (card_rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    theme::paint_glass_card(ui.painter(), card_rect, layout, native_backdrop);
    let header_rect = egui::Rect::from_min_size(
        card_rect.min,
        egui::vec2(card_rect.width(), layout.card_header_height),
    );
    theme::paint_glass_card_header(ui, header_rect, layout, native_backdrop);
    let content = header_rect.shrink2(egui::vec2(layout.header_padding_x, 0.0));
    let show_icon = view.show_icon;
    let spacing = ui.spacing().item_spacing.x;
    let icon_extent = theme::icon_action_extent(24.0);
    // The name stops before the gap and both 24×24 icons. A character
    // estimate cannot do that: wide glyphs would paint into the icons.
    let icon_reserve = header_icon_reserve(icon_extent, spacing, show_icon);
    let left_rect = egui::Rect::from_min_max(
        content.min,
        egui::pos2(
            (content.right() - icon_reserve).max(content.left()),
            content.bottom(),
        ),
    );
    let mut left = ui.new_child(
        UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    let git_open = view.git_open;
    // Owned before the label is painted: the aside width is measured from
    // the section label's right edge, and the git view must not fall back
    // to the root name when the head line is empty.
    let aside = if git_open {
        view.git_header.and_then(|header| {
            let raw_head = source_control_head_label(
                header.branch.as_deref(),
                header.detached,
                header.commit_abbrev.as_deref(),
            );
            let (head, shown_name) = repos_aside_texts(
                &raw_head,
                header.is_linked_worktree,
                header.directory_name.as_deref(),
            );
            if head.is_empty() && shown_name.is_none() {
                None
            } else {
                let hover = source_control_head_hover(
                    &raw_head,
                    header.detached,
                    header.commit_abbrev.as_deref(),
                    header.commit_subject.as_deref(),
                );
                Some((head, hover, shown_name))
            }
        })
    } else {
        view.root_name
            .as_ref()
            .map(|(name, full)| (name.clone(), full.clone(), None))
    };
    theme::section_label(&mut left, if git_open { "REPOS" } else { "EXPLORER" });
    if let Some((text, hover, shown_name)) = aside {
        let name_width = root_name_width(
            left.min_rect().right(),
            content.right(),
            spacing,
            icon_extent,
            show_icon,
        );
        if name_width >= 1.0 {
            let name_left = left.min_rect().right() + spacing;
            let name_rect = egui::Rect::from_min_size(
                egui::pos2(name_left, left_rect.top()),
                egui::vec2(name_width, left_rect.height()),
            );
            if git_open {
                paint_repos_aside_ui(ui, name_rect, &text, &hover, shown_name.as_deref(), scope);
            } else {
                paint_clipped_label(
                    ui,
                    name_rect,
                    name_left,
                    &text,
                    name_width,
                    scope.with("explorer-root-name"),
                    Some(&hover),
                );
            }
        }
    }
    if show_icon {
        if let Some(switch) = explorer_header_icons_ui(
            ui,
            content,
            git_open,
            scope,
            native_backdrop,
            &view.source_control_tooltip,
            view.source_control_rings,
        ) {
            result.switch = Some(switch);
        }
    }
    theme::paint_card_border(ui, card_rect, layout);

    let body_rect = egui::Rect::from_min_max(
        egui::pos2(card_rect.left(), header_rect.bottom()),
        card_rect.max,
    );
    let content_rect = body_rect.shrink2(egui::vec2(8.0, 4.0));
    let mut body = ui.new_child(UiBuilder::new().max_rect(content_rect));
    if let Some(unavailable) = &view.root_unavailable {
        tone_strip(
            &mut body,
            layout,
            26.0,
            theme::WARNING_TINT,
            theme::WARNING,
            "Active root unavailable — showing the last folder.",
            sanitize_display(unavailable, 600),
        );
        body.add_space(4.0);
    }
    if git_open {
        if let Some(git) = &view.git {
            git_body_ui(&mut body, git, scope, &mut result);
        }
    } else {
        explorer_tree_body_ui(
            &mut body,
            &view.tree,
            scope,
            layout,
            view.scroll_to_selection,
            view.retry_available,
            &mut result,
        );
    }
    result
}

fn explorer_tree_body_ui(
    ui: &mut Ui,
    body: &ExplorerTreeBody<'_>,
    scope: egui::Id,
    layout: &theme::Layout,
    scroll_to_selection: bool,
    retry_available: bool,
    result: &mut ExplorerCardResult,
) {
    match body {
        ExplorerTreeBody::Error(message) => {
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
                    ui.label(theme::body(sanitize_display(message, 700)));
                    if retry_available
                        && theme::secondary_button(ui, "Retry", layout.button_height).clicked()
                    {
                        result.retry = true;
                    }
                });
        }
        ExplorerTreeBody::Loading => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0).color(theme::ACCENT));
                ui.label(theme::body("Loading folder…"));
            });
        }
        ExplorerTreeBody::Empty => {
            ui.add_space(6.0);
            ui.label(theme::body("This folder is empty."));
        }
        ExplorerTreeBody::Rows(source) => {
            explorer_tree_rows_ui(ui, *source, scope, scroll_to_selection, result);
        }
    }
}

fn explorer_tree_rows_ui(
    ui: &mut Ui,
    source: &dyn ExplorerRowSource,
    scope: egui::Id,
    scroll_to_selection: bool,
    result: &mut ExplorerCardResult,
) {
    let total = source.row_count();
    if total == 0 {
        ui.add_space(6.0);
        ui.label(theme::body("This folder is empty."));
        return;
    }
    let tree_id = scope.with("tree-focus");
    let mut action = None;
    let mut effective_first = None;
    let selected_index = source.selected_index();
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
    if container.has_focus() && selected_index.is_none() {
        effective_first = Some(0);
        action = Some(ExplorerRowAction::Select(0));
    }

    let mut scroll_to = if scroll_to_selection {
        selected_index
    } else {
        None
    };
    if container.has_focus() {
        let events = ui.ctx().input(|input| input.events.clone());
        let mut cursor = selected_index;
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
                egui::Key::ArrowDown => {
                    let next = cursor.map_or(0, |index| (index + 1).min(total - 1));
                    cursor = Some(next);
                    action = Some(ExplorerRowAction::Select(next));
                    scroll_to = Some(next);
                }
                egui::Key::ArrowUp => {
                    let next = cursor.map_or(total - 1, |index| index.saturating_sub(1));
                    cursor = Some(next);
                    action = Some(ExplorerRowAction::Select(next));
                    scroll_to = Some(next);
                }
                egui::Key::Home => {
                    cursor = Some(0);
                    action = Some(ExplorerRowAction::Select(0));
                    scroll_to = Some(0);
                }
                egui::Key::End => {
                    let last = total - 1;
                    cursor = Some(last);
                    action = Some(ExplorerRowAction::Select(last));
                    scroll_to = Some(last);
                }
                egui::Key::PageUp => {
                    let next = cursor.map_or(total - 1, |index| index.saturating_sub(10));
                    cursor = Some(next);
                    action = Some(ExplorerRowAction::Select(next));
                    scroll_to = Some(next);
                }
                egui::Key::PageDown => {
                    let next = cursor.map_or(0, |index| (index + 10).min(total - 1));
                    cursor = Some(next);
                    action = Some(ExplorerRowAction::Select(next));
                    scroll_to = Some(next);
                }
                egui::Key::ArrowRight => {
                    if let Some(index) = cursor {
                        let row = source.row(index);
                        if row.kind == EntryKind::Directory {
                            action = Some(ExplorerRowAction::Toggle(index));
                        }
                    }
                }
                egui::Key::ArrowLeft => {
                    if let Some(index) = cursor {
                        let row = source.row(index);
                        if row.kind == EntryKind::Directory && row.expanded {
                            action = Some(ExplorerRowAction::Toggle(index));
                        } else if row.depth > 0 {
                            let parent = (0..index)
                                .rev()
                                .find(|&candidate| source.row(candidate).depth + 1 == row.depth);
                            if let Some(parent) = parent {
                                cursor = Some(parent);
                                action = Some(ExplorerRowAction::Select(parent));
                                scroll_to = Some(parent);
                            }
                        }
                    }
                }
                egui::Key::Enter | egui::Key::Space => {
                    if let Some(index) = cursor {
                        action = Some(ExplorerRowAction::Activate(index));
                    }
                }
                _ => {}
            }
        }
    }

    let mut clicked = None;
    ui.spacing_mut().item_spacing.y = 0.0;
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, 24.0, total, |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for index in range {
                let row = source.row(index);
                let selected = row.selected || effective_first == Some(index);
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), Sense::hover());
                let response = ui.interact(rect, scope.with(("tree-row", index)), Sense::CLICK);
                let fill = if selected {
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

                let icon_slot_left = 8.0 + row.depth as f32 * 14.0;
                let expanded = row.kind == EntryKind::Directory && row.expanded;
                let color = if row.kind == EntryKind::Error {
                    theme::ERROR
                } else if selected || expanded {
                    theme::TEXT_PRIMARY
                } else {
                    theme::TEXT_SECONDARY
                };
                let icon_color = if row.kind == EntryKind::Error {
                    theme::ERROR
                } else if selected || expanded {
                    theme::TEXT_PRIMARY
                } else {
                    theme::TEXT_TERTIARY
                };
                let name = sanitize_display(&row.name, 200);
                let text_left = rect.left() + 28.0 + row.depth as f32 * 14.0;
                // A 6px dot sits 8px in from the row's right edge. Text stops
                // before it so the label never covers the mark.
                let dot_center = row
                    .mark
                    .as_ref()
                    .map(|_| egui::pos2(rect.right() - 8.0 - 3.0, rect.center().y));
                let text_right = dot_center
                    .map(|center| center.x - 3.0 - 6.0)
                    .unwrap_or(rect.right() - 6.0);
                let width = (text_right - text_left).max(1.0);
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
                let marker_center = egui::pos2(rect.left() + icon_slot_left + 8.0, rect.center().y);
                match row.kind {
                    EntryKind::Directory | EntryKind::File => {
                        if let Some(icon) = row.icon {
                            paint_entry_icon(ui, marker_center, icon);
                        }
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

                let pointer_on_dot = if let (Some(mark), Some(center)) = (&row.mark, dot_center) {
                    paint_status_dot(
                        ui,
                        scope.with(("tree-dot", index)),
                        rect,
                        center,
                        mark.color(),
                        &mark.tooltip(),
                    )
                } else {
                    false
                };
                let path_tooltip = if let Some(error) = row.error {
                    sanitize_display(error, 600)
                } else {
                    sanitize_display(&row.tooltip.text(), 600)
                };
                // The kind strip owns the tooltip while the pointer is in it.
                let response = if pointer_on_dot {
                    response
                } else {
                    response.on_hover_text(path_tooltip)
                };
                if scroll_to == Some(index) {
                    response.scroll_to_me(Some(Align::Center));
                }
                if response.clicked() {
                    clicked = Some(ExplorerRowAction::Click(index));
                }
            }
        });
    if let Some(clicked) = clicked {
        ui.memory_mut(|memory| memory.request_focus(tree_id));
        result.row = Some(clicked);
    } else if let Some(action) = action {
        result.row = Some(action);
    }
}

fn paint_repos_aside_ui(
    ui: &mut Ui,
    clip: egui::Rect,
    head: &str,
    head_hover: &str,
    shown_name: Option<&str>,
    scope: egui::Id,
) {
    let head = sanitize_display(head, 160);
    let shown = shown_name
        .map(|name| sanitize_display(name, 160))
        .filter(|name| !name.is_empty());
    let Some(name) = shown else {
        if !head.is_empty() {
            paint_clipped_label(
                ui,
                clip,
                clip.left(),
                &head,
                clip.width(),
                scope.with("source-control-head"),
                Some(head_hover),
            );
        }
        return;
    };
    let gap = 4.0;
    let icon = theme::LINKED_WORKTREE_ICON;
    let mut head_w = if head.is_empty() {
        0.0
    } else {
        mono_width(ui, &head)
    };
    let mut name_w = mono_width(ui, &name);
    let leading = if head_w > 0.0 { gap } else { 0.0 };
    let block = leading + icon + gap;
    let overflow = (head_w + block + name_w - clip.width()).max(0.0);
    if overflow > 0.0 {
        let name_cut = overflow.min((name_w - 1.0).max(0.0));
        name_w -= name_cut;
        head_w = (head_w - (overflow - name_cut)).max(0.0);
    }
    let mut x = clip.left();
    if head_w >= 1.0 && !head.is_empty() {
        paint_clipped_label(
            ui,
            clip,
            x,
            &head,
            head_w,
            scope.with("source-control-head"),
            Some(head_hover),
        );
        x += head_w + gap;
    }
    if x + icon > clip.right() {
        return;
    }
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(x + icon * 0.5, clip.center().y),
        egui::vec2(icon, icon),
    );
    theme::paint_icon(
        &ui.painter().with_clip_rect(clip),
        icon_rect,
        theme::IconGlyph::LinkedWorktree,
        theme::TEXT_TERTIARY,
    );
    x += icon + gap;
    name_w = name_w.min((clip.right() - x).max(0.0));
    let hover = linked_worktree_hover(&name);
    if name_w < 1.0 {
        ui.interact(icon_rect, scope.with("linked-worktree"), Sense::hover())
            .on_hover_text(sanitize_display(&hover, 600));
        return;
    }
    let text_rect = paint_clipped_label(ui, clip, x, &name, name_w, scope, None);
    ui.interact(
        icon_rect.union(text_rect),
        scope.with("linked-worktree"),
        Sense::hover(),
    )
    .on_hover_text(sanitize_display(&hover, 600));
}

fn explorer_header_icons_ui(
    ui: &mut Ui,
    content: egui::Rect,
    git_open: bool,
    scope: egui::Id,
    native_backdrop: bool,
    source_control_tooltip: &str,
    rings: theme::SourceControlRings,
) -> Option<ExplorerViewKind> {
    let extent = 24.0;
    let gap = theme::explorer_header_glass(native_backdrop);
    let folders_rect = egui::Rect::from_min_size(
        egui::pos2(
            content.right() - extent * 2.0 - HEADER_ICON_GAP,
            content.top(),
        ),
        egui::vec2(extent, content.height()),
    );
    let git_rect = egui::Rect::from_min_size(
        egui::pos2(content.right() - extent, content.top()),
        egui::vec2(extent, content.height()),
    );
    let mut folders_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(folders_rect)
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    let folders = theme::icon_action(
        &mut folders_ui,
        scope.with("explorer-view"),
        theme::IconGlyph::Folders,
        "Explorer",
        !git_open,
        extent,
        gap,
    );
    let mut git_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(git_rect)
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    let git = theme::source_control_action(
        &mut git_ui,
        scope.with("source-control"),
        source_control_tooltip,
        git_open,
        extent,
        gap,
        rings,
    );
    // The tooltip may include a count. The accessible name does not.
    git.response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Source Control"));
    // The active icon does not switch. The other icon switches only the body.
    if folders.activated && git_open {
        return Some(ExplorerViewKind::Files);
    }
    if git.activated && !git_open {
        return Some(ExplorerViewKind::Git);
    }
    None
}

fn git_body_ui(ui: &mut Ui, body: &GitBodyView, scope: egui::Id, result: &mut ExplorerCardResult) {
    if !body.has_status {
        let message = if body.failed {
            "Git status unavailable"
        } else {
            "Reading changes…"
        };
        ui.label(quiet_line(message));
        return;
    }
    if body.stale {
        ui.label(quiet_line(
            "Git status unavailable — showing the last status",
        ));
    }
    if body.groups.is_empty() {
        if !body.stale {
            ui.label(quiet_line("No changes"));
        }
        return;
    }
    let mut clicked = None;
    ui.spacing_mut().item_spacing.y = 2.0;
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (group_index, group) in body.groups.iter().enumerate() {
                if group.rows.is_empty() {
                    continue;
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{} ({})", group.title, group.rows.len()))
                        .font(FontId::proportional(10.5))
                        .color(theme::TEXT_TERTIARY),
                );
                for (row_index, row) in group.rows.iter().enumerate() {
                    if git_row_ui(ui, scope.with((group.title, row_index)), row) {
                        clicked = Some((group_index, row_index));
                    }
                }
            }
        });
    if let Some(clicked) = clicked {
        result.git_row = Some(clicked);
    }
}

fn git_row_ui(ui: &mut Ui, id: egui::Id, row: &GitRowView) -> bool {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), Sense::hover());
    let response = ui.interact(rect, id, Sense::CLICK);
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
    if let Some(icon) = row.icon {
        paint_entry_icon(ui, egui::pos2(rect.left() + 16.0, rect.center().y), icon);
    }
    let dot_center = egui::pos2(rect.right() - 8.0 - 3.0, rect.center().y);
    let relative = sanitize_display(&row.name, 240);
    let text_left = rect.left() + 28.0;
    let text_right = dot_center.x - 3.0 - 6.0;
    let width = (text_right - text_left).max(1.0);
    let name_color = if row.selected {
        theme::TEXT_PRIMARY
    } else {
        theme::TEXT_SECONDARY
    };
    theme::paint_ellipsized_centered(
        ui.painter(),
        text_left,
        rect.center().y,
        relative,
        FontId::proportional(12.5),
        name_color,
        width,
    );
    let pointer_on_dot = paint_status_dot(
        ui,
        id.with("status-dot"),
        rect,
        dot_center,
        change_kind_color(row.kind),
        row.kind.label(),
    );
    let mut tooltip = sanitize_display(&row.tooltip, 600);
    if let Some(original) = &row.original {
        tooltip = format!(
            "{tooltip}\nrenamed from {}",
            sanitize_display(original, 400)
        );
    }
    let clicked = response.clicked();
    if !pointer_on_dot {
        response.on_hover_text(tooltip);
    }
    clicked
}

pub(crate) fn preview_card_ui(
    ui: &mut Ui,
    state: PreviewCardState<'_>,
    ids: PreviewCardIds,
    layout: &theme::Layout,
) -> PreviewCardResult {
    let mut result = PreviewCardResult::default();
    let card_rect = ui.available_rect_before_wrap();
    theme::paint_card(ui.painter(), card_rect, layout, theme::BG_CARD);

    let focus_response = ui.interact(card_rect, ids.focus, Sense::click());
    let pressed_here = ui.ctx().input(|input| {
        input.pointer.any_pressed()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| card_rect.contains(pos))
    });
    if state.focus_pending || pressed_here {
        focus_response.request_focus();
    }
    if ui.memory(|memory| memory.has_focus(ids.focus)) {
        if let Some(action) = preview_shortcut_action_from_input(ui.ctx()) {
            match action {
                PreviewAction::Refresh => result.refresh = true,
                PreviewAction::CopyPath => result.copy_path = true,
                PreviewAction::Close => {
                    result.surface = PreviewUiAction::Close;
                    return result;
                }
            }
        }
    }

    let header_rect = egui::Rect::from_min_size(
        card_rect.min,
        egui::vec2(card_rect.width(), layout.card_header_height),
    );
    paint_card_header(ui, header_rect, layout, theme::BG_HEADER);
    let has_path = state.path_name.is_some();
    let filename_chars = if card_rect.width() < 480.0 { 10 } else { 20 };
    let content = header_rect.shrink2(egui::vec2(layout.header_padding_x, 0.0));
    let mut right = ui.new_child(
        UiBuilder::new()
            .max_rect(content)
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    let close_action = theme::ghost_action_with(
        &mut right,
        ids.close,
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
        result.surface = PreviewUiAction::Close;
    }
    let terminal_action = theme::ghost_action(
        &mut right,
        ids.terminal,
        "Terminal",
        false,
        layout.button_height,
        theme::BG_HEADER,
    );
    terminal_action
        .response
        .on_hover_text("Return to the live terminal; the preview stays loaded");
    if terminal_action.activated {
        result.surface = PreviewUiAction::ShowTerminal;
    }
    if layout.compact {
        let more = theme::ghost_action(
            &mut right,
            ids.refresh.with("menu"),
            "…",
            false,
            layout.button_height,
            theme::BG_HEADER,
        );
        let more_response = more.response;
        egui::Popup::menu(&more_response).show(|ui| {
            if ui.button(PreviewAction::Refresh.label()).clicked() {
                result.refresh = true;
                ui.close();
            }
            if ui.button(PreviewAction::CopyPath.label()).clicked() {
                result.copy_path = true;
                ui.close();
            }
        });
        more_response.on_hover_text("Preview actions");
    } else {
        let refresh_action = theme::ghost_action_with(
            &mut right,
            ids.refresh,
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
            result.refresh = true;
        }
        let copy_action = theme::ghost_action_with(
            &mut right,
            ids.copy,
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
            result.copy_path = true;
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
    if let Some(name) = state.path_name {
        let remaining = (left_rect.right() - left.min_rect().right() - 6.0).max(0.0);
        let chars = ((remaining / 6.2).floor() as usize).min(filename_chars);
        if chars >= 4 {
            left.label(
                RichText::new(theme::truncate_middle(&sanitize_display(name, 160), chars))
                    .font(FontId::proportional(12.5))
                    .color(theme::TEXT_PRIMARY),
            )
            .on_hover_text(sanitize_display(state.path_full.unwrap_or(name), 900));
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
    if result.surface != PreviewUiAction::None {
        return result;
    }
    theme::paint_card_border(ui, card_rect, layout);

    let meta_height = if layout.hide_preview_meta { 0.0 } else { 18.0 };
    let meta_rect = egui::Rect::from_min_size(
        egui::pos2(card_rect.left(), header_rect.bottom()),
        egui::vec2(card_rect.width(), meta_height),
    );
    let body_top = meta_rect.bottom();
    let body_rect = egui::Rect::from_min_max(egui::pos2(card_rect.left(), body_top), card_rect.max);
    let mut body =
        ui.new_child(UiBuilder::new().max_rect(body_rect.shrink2(egui::vec2(10.0, 6.0))));
    if !layout.hide_preview_meta {
        if let Some(document) = state.document {
            let meta = preview_meta(document, state.refreshed_at, layout.compact);
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

    if state.loading {
        body.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(12.0).color(theme::ACCENT));
            ui.label(theme::body("Loading preview…"));
        });
    }
    if let Some(error) = state.error {
        tone_strip(
            &mut body,
            layout,
            26.0,
            theme::ERROR_TINT,
            theme::ERROR,
            &sanitize_display(error, 400),
            sanitize_display(error, 700),
        );
        body.add_space(4.0);
    }
    let Some(document) = state.document else {
        return result;
    };
    if !document.is_text() {
        preview_state_card(&mut body, document, layout);
        return result;
    }
    if document.lines.is_empty() {
        preview_empty_card(&mut body, document, layout);
        return result;
    }
    preview_code_ui(&mut body, document);
    result
}

/// Empty space between the folders target and the source-control target.
const HEADER_ICON_GAP: f32 = 4.0;

/// Width reserved at the header's right edge for the view icons, the gap
/// between them, and the gap before them. Otherwise nothing.
fn header_icon_reserve(icon_extent: f32, spacing: f32, show_icon: bool) -> f32 {
    if show_icon {
        icon_extent * 2.0 + HEADER_ICON_GAP + spacing
    } else {
        0.0
    }
}

/// Pixel width left for the root name after the section label, the item gap,
/// and — when shown — both header icons plus the gap before them.
fn root_name_width(
    label_right: f32,
    content_right: f32,
    spacing: f32,
    icon_extent: f32,
    show_icon: bool,
) -> f32 {
    let reserve = header_icon_reserve(icon_extent, spacing, show_icon);
    let name_left = label_right + spacing;
    let name_right = content_right - reserve;
    (name_right - name_left).max(0.0)
}

fn mono_width(ui: &Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            text.to_owned(),
            FontId::monospace(10.5),
            theme::TEXT_TERTIARY,
        )
        .size()
        .x
}

/// Paint one ellipsized header string inside `clip`. Returns the text hit rect.
///
/// `hover` attaches the tooltip. Pass `None` when a wider hit target owns it.
fn paint_clipped_label(
    ui: &mut Ui,
    clip: egui::Rect,
    left: f32,
    text: &str,
    max_width: f32,
    id: egui::Id,
    hover: Option<&str>,
) -> egui::Rect {
    if max_width < 1.0 || text.is_empty() {
        return egui::Rect::from_min_size(egui::pos2(left, clip.center().y), egui::Vec2::ZERO);
    }
    let painter = ui.painter().with_clip_rect(clip);
    let galley = theme::paint_ellipsized_centered(
        &painter,
        left,
        clip.center().y,
        text,
        FontId::monospace(10.5),
        theme::TEXT_TERTIARY,
        max_width,
    );
    let text_rect = egui::Rect::from_min_size(
        egui::pos2(left, clip.center().y - galley.size().y / 2.0),
        galley.size(),
    );
    if let Some(hover) = hover {
        ui.interact(text_rect, id, Sense::hover())
            .on_hover_text(sanitize_display(hover, 600));
    }
    text_rect
}

/// First 8 characters of `rev-parse --short=8 HEAD`, or the whole string if shorter.
fn visible_commit_id(abbrev: Option<&str>) -> String {
    abbrev
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(8).collect())
        .unwrap_or_default()
}

fn source_control_head_label(
    branch: Option<&str>,
    detached: bool,
    commit_abbrev: Option<&str>,
) -> String {
    if detached {
        visible_commit_id(commit_abbrev)
    } else {
        branch
            .map(str::trim)
            .filter(|branch| !branch.is_empty())
            .unwrap_or("")
            .to_owned()
    }
}

/// Folder name for the linked-worktree icon, even when it repeats the head.
fn linked_worktree_folder(linked: bool, directory_name: Option<&str>) -> Option<String> {
    if !linked {
        return None;
    }
    directory_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// Extra folder name beside a different head. Omitted when it repeats the
/// visible branch or 8-character id.
fn source_control_worktree_name(
    head: &str,
    linked: bool,
    directory_name: Option<&str>,
) -> Option<String> {
    let name = linked_worktree_folder(linked, directory_name)?;
    if !head.is_empty() && head == name {
        return None;
    }
    Some(name)
}

/// Head text and the folder name painted after the worktree icon.
///
/// A distinct folder name keeps the head. A folder name that repeats the head
/// replaces it, so the painter shows the icon and that name only.
fn repos_aside_texts(
    head: &str,
    linked: bool,
    directory_name: Option<&str>,
) -> (String, Option<String>) {
    let folder = linked_worktree_folder(linked, directory_name);
    let shown = source_control_worktree_name(head, linked, directory_name);
    if folder.is_some() && shown.is_none() {
        (String::new(), folder)
    } else {
        (head.to_owned(), shown)
    }
}

fn linked_worktree_hover(name: &str) -> String {
    format!("{name} is a linked worktree")
}

/// Hover text for the head line.
///
/// An attached line hovers as itself. A detached line shows the commit subject,
/// or the visible id when the subject is empty. A longer abbreviation and the
/// full visible line stay in the hover when they add text the subject does not.
fn source_control_head_hover(
    line: &str,
    detached: bool,
    commit_abbrev: Option<&str>,
    commit_subject: Option<&str>,
) -> String {
    if !detached {
        return line.to_owned();
    }
    let abbrev = commit_abbrev
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let id = visible_commit_id(abbrev);
    let subject = commit_subject
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut parts: Vec<String> = Vec::new();
    if let Some(subject) = subject {
        parts.push(subject.to_owned());
    } else if !id.is_empty() {
        parts.push(id);
    }
    if let Some(abbrev) = abbrev {
        if abbrev.chars().count() > 8 && !parts.iter().any(|part| part.contains(abbrev)) {
            parts.push(abbrev.to_owned());
        }
    }
    if !line.is_empty() && !parts.iter().any(|part| part.contains(line)) {
        parts.push(line.to_owned());
    }
    parts.join("\n")
}

fn quiet_line(text: &str) -> RichText {
    RichText::new(text)
        .font(FontId::proportional(12.5))
        .color(theme::TEXT_TERTIARY)
}

fn source_control_tooltip(status: Option<&GitStatusSnapshot>) -> String {
    let Some(status) = status else {
        return "Source Control".to_owned();
    };
    const ORDER: [&str; 4] = [
        crate::worktree::GROUP_MERGE,
        crate::worktree::GROUP_STAGED,
        crate::worktree::GROUP_CHANGES,
        crate::worktree::GROUP_UNTRACKED,
    ];
    let mut parts = Vec::new();
    for title in ORDER {
        let count: usize = status
            .groups
            .iter()
            .filter(|group| group.title == title)
            .map(|group| group.rows.len())
            .sum();
        if count > 0 {
            parts.push(format!("{title} ({count})"));
        }
    }
    if parts.is_empty() {
        "Source Control".to_owned()
    } else {
        format!("Source Control - {}", parts.join(" · "))
    }
}

fn source_control_rings(status: Option<&GitStatusSnapshot>) -> theme::SourceControlRings {
    let Some(status) = status else {
        return theme::SourceControlRings::default();
    };
    let mut rings = theme::SourceControlRings::default();
    for group in &status.groups {
        for row in &group.rows {
            match row.kind {
                ChangeKind::New => rings.highest = Some(theme::SUCCESS),
                ChangeKind::Modified => rings.middle = Some(theme::WARNING),
                ChangeKind::Deleted | ChangeKind::Conflict => rings.lowest = Some(theme::ERROR),
                ChangeKind::Renamed => {}
            }
        }
    }
    rings
}

fn change_kind_color(kind: ChangeKind) -> Color32 {
    match kind {
        ChangeKind::New => theme::SUCCESS,
        ChangeKind::Modified => theme::WARNING,
        ChangeKind::Deleted | ChangeKind::Conflict => theme::ERROR,
        ChangeKind::Renamed => theme::INFO,
    }
}

/// Kinds for file paths that already appear in the status list.
///
/// Directory entries stay on the folder map. A deleted path that is not a
/// tree row is recorded here only as a mark; the tree never inserts a row.
fn file_status_marks(status: &GitStatusSnapshot) -> BTreeMap<PathBuf, FolderMark> {
    let mut kinds: BTreeMap<PathBuf, BTreeSet<ChangeKind>> = BTreeMap::new();
    for group in &status.groups {
        for row in &group.rows {
            if row.is_directory {
                continue;
            }
            kinds.entry(row.path.clone()).or_default().insert(row.kind);
        }
    }
    kinds
        .into_iter()
        .map(|(path, kinds)| {
            (
                path,
                FolderMark {
                    kinds: kinds.into_iter().collect(),
                },
            )
        })
        .collect()
}

fn folder_mark_color(mark: &FolderMark) -> Color32 {
    if mark.is_mixed() {
        theme::ACCENT
    } else {
        mark.single()
            .map(change_kind_color)
            .unwrap_or(theme::ACCENT)
    }
}

fn change_kinds_color(kinds: &[ChangeKind]) -> Color32 {
    if kinds.len() > 1 {
        theme::ACCENT
    } else {
        kinds
            .first()
            .copied()
            .map(change_kind_color)
            .unwrap_or(theme::ACCENT)
    }
}

/// 6px kind dot. The hover strip is the full row height and at least 28px wide,
/// ending at the row's right edge. The tooltip is anchored at the dot, not the strip.
///
/// Returns whether the pointer is in that strip, so the row can skip its path tooltip.
fn paint_status_dot(
    ui: &mut Ui,
    id: egui::Id,
    row: egui::Rect,
    center: egui::Pos2,
    color: Color32,
    tooltip: &str,
) -> bool {
    ui.painter().circle_filled(center, 3.0, color);
    let width = STATUS_DOT_HIT_WIDTH.min(row.width());
    let hit = egui::Rect::from_min_max(
        egui::pos2(row.right() - width, row.top()),
        row.right_bottom(),
    );
    let response = ui.interact(hit, id, Sense::hover());
    let pointer_in_strip = response.contains_pointer();
    if pointer_in_strip {
        // `on_hover_text` would anchor to this strip. Pin the tooltip to the dot.
        egui::Popup::from_response(&response)
            .kind(egui::PopupKind::Tooltip)
            .at_position(center)
            .gap(4.0)
            .sense(Sense::hover())
            .interactable(false)
            .show(|ui| {
                ui.set_max_width(ui.spacing().tooltip_width);
                ui.label(tooltip);
            });
    }
    pointer_in_strip
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

/// Paint a file or folder theme icon in the 16px slot centered on `center`.
///
/// The image keeps the default white tint, so the upstream artwork is drawn
/// exactly as shipped in every row state.
fn paint_entry_icon(ui: &Ui, center: egui::Pos2, icon: explorer_icons::IconId) {
    let rect = egui::Rect::from_center_size(center, egui::Vec2::splat(ENTRY_ICON_SIZE));
    egui::Image::from_bytes(icon.uri(), icon.bytes()).paint_at(ui, rect);
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

/// What the remote explorer asked the owning tab to do. Paths stay remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteExplorerAction {
    SetMode(crate::remote_workspace::ExplorerMode),
    Expand(made_remote::protocol::RemotePath),
    Collapse(made_remote::protocol::RemotePath),
    Select(made_remote::protocol::RemotePath),
    SelectGit(made_remote::protocol::RemotePath),
    OpenPreview(made_remote::protocol::RemotePath),
    RetryRoot,
}

struct RemoteRow {
    path: made_remote::protocol::RemotePath,
    name: String,
    depth: usize,
    kind: EntryKind,
    expanded: bool,
    selected: bool,
    tooltip: String,
    mark: Option<Vec<ChangeKind>>,
    icon: Option<explorer_icons::IconId>,
}

#[derive(Default)]
struct RemoteRows {
    rows: Vec<RemoteRow>,
}

#[derive(Default)]
pub struct RemoteExplorerCache {
    key: Option<(made_remote::protocol::RemotePath, u64)>,
    rows: RemoteRows,
    file_marks: std::collections::HashMap<made_remote::protocol::RemotePath, Vec<ChangeKind>>,
}

impl ExplorerRowSource for RemoteRows {
    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn row(&self, index: usize) -> ExplorerRowView<'_> {
        let row = &self.rows[index];
        ExplorerRowView {
            name: Cow::Borrowed(&row.name),
            depth: row.depth,
            kind: row.kind,
            expanded: row.expanded,
            selected: row.selected,
            error: None,
            tooltip: RowTooltip::Text(&row.tooltip),
            mark: row.mark.as_deref().map(RowMark::Remote),
            icon: row.icon,
        }
    }

    fn selected_index(&self) -> Option<usize> {
        self.rows.iter().position(|row| row.selected)
    }
}

/// Presentation and interaction context for one remote Explorer frame.
#[derive(Debug, Clone, Copy)]
pub struct RemoteExplorerOptions {
    pub scope: egui::Id,
    pub native_backdrop: bool,
    /// Only a tab with a live remote connection can re-request its root.
    pub allow_root_retry: bool,
}

/// Draw a remote explorer from an already-applied projection through the same
/// card, header, row, and Git renderer used by the local Explorer. This
/// function does not read the host filesystem and does not start a local
/// watcher.
pub fn remote_explorer_ui(
    ui: &mut Ui,
    root: &made_remote::protocol::RemotePath,
    projection: &crate::remote_workspace::WorkspaceProjection,
    layout: &theme::Layout,
    options: RemoteExplorerOptions,
    cache: &mut RemoteExplorerCache,
) -> Vec<RemoteExplorerAction> {
    use crate::remote_workspace::{ExplorerMode, Freshness};

    let scope = options.scope;
    let native_backdrop = options.native_backdrop;
    let allow_root_retry = options.allow_root_retry;

    let git_view = projection.git_view.as_ref();
    let show_icon =
        projection.git_root_known || git_view.is_some_and(|git| git.worktree_name.is_some());
    let git_open = projection.mode == ExplorerMode::Git && show_icon;
    let header_root = if matches!(
        projection.root_state,
        Freshness::Loading | Freshness::Stale | Freshness::Unavailable(_)
    ) {
        projection.requested_root.as_ref().unwrap_or(root)
    } else {
        root
    };
    let git_header = git_view.map(remote_git_header);
    let root_full = sanitize_display(&remote_path_text(header_root), 900);
    let root_name = sanitize_display(&remote_root_name(header_root), 160);
    let mut root_unavailable = None;
    let root_failed = matches!(projection.root_state, Freshness::Unavailable(_));
    let tree = if git_open {
        ExplorerTreeBody::Empty
    } else if root_failed {
        // A failed pre-flight SSH stage or attach never painted data. Show the
        // bounded failure instead of a spinner or a partial/stale tree.
        root_unavailable = Some("Remote files unavailable".to_string());
        ExplorerTreeBody::Error(
            if allow_root_retry {
                "The remote folder did not open because the SSH attempt failed. Retry after the companion is available."
            } else {
                "The remote folder did not open because the SSH attempt failed. Reconnect after the companion is available."
            }
            .to_owned(),
        )
    } else {
        let directory = projection.directories.get(header_root).or_else(|| {
            projection
                .requested_root
                .as_ref()
                .and_then(|requested| projection.directories.get(requested))
        });
        let cache_key = (header_root.clone(), projection.revision);
        if cache.key.as_ref() != Some(&cache_key) {
            cache.rows.rows.clear();
            cache.file_marks = remote_file_marks(git_view);
            if let Some(directory) = directory {
                collect_remote_rows(
                    &directory.path,
                    projection,
                    git_view,
                    &cache.file_marks,
                    &mut cache.rows.rows,
                    0,
                );
            }
            cache.key = Some(cache_key);
        }
        match directory {
            None if matches!(projection.root_state, Freshness::Unavailable(_)) => {
                root_unavailable = Some("Remote directory unavailable".to_owned());
                ExplorerTreeBody::Empty
            }
            None => ExplorerTreeBody::Loading,
            Some(directory) => {
                if matches!(directory.freshness, Freshness::Unavailable(_)) {
                    root_unavailable = Some("Remote directory unavailable".to_owned());
                }
                if cache.rows.rows.is_empty() {
                    if matches!(directory.freshness, Freshness::Loading | Freshness::Stale) {
                        ExplorerTreeBody::Loading
                    } else {
                        ExplorerTreeBody::Empty
                    }
                } else {
                    ExplorerTreeBody::Rows(&cache.rows)
                }
            }
        }
    };
    let git = git_open.then(|| remote_git_body(root, git_view, projection.git_selected.as_ref()));
    let view = ExplorerCardView {
        git_open,
        show_icon,
        root_name: Some((root_name, root_full)),
        git_header: git_header.as_ref(),
        source_control_tooltip: remote_source_control_tooltip(git_view),
        source_control_rings: remote_source_control_rings(git_view),
        root_unavailable,
        tree,
        git,
        scroll_to_selection: false,
        retry_available: allow_root_retry,
    };
    let result = explorer_card_ui(ui, &view, scope, native_backdrop, layout);
    let mut actions = Vec::new();
    if projection.mode == ExplorerMode::Git && !show_icon {
        actions.push(RemoteExplorerAction::SetMode(ExplorerMode::Files));
    }
    if let Some(switch) = result.switch {
        actions.push(RemoteExplorerAction::SetMode(match switch {
            ExplorerViewKind::Files => ExplorerMode::Files,
            ExplorerViewKind::Git => ExplorerMode::Git,
        }));
    }
    if result.retry {
        actions.push(RemoteExplorerAction::RetryRoot);
    }
    if let Some(action) = result.row {
        let index = match action {
            ExplorerRowAction::Click(index)
            | ExplorerRowAction::Select(index)
            | ExplorerRowAction::Toggle(index)
            | ExplorerRowAction::Activate(index) => index,
        };
        if let Some(row) = cache.rows.rows.get(index) {
            let toggles = matches!(
                action,
                ExplorerRowAction::Click(_)
                    | ExplorerRowAction::Toggle(_)
                    | ExplorerRowAction::Activate(_)
            );
            actions.push(RemoteExplorerAction::Select(row.path.clone()));
            if toggles && row.kind == EntryKind::Directory {
                actions.push(if row.expanded {
                    RemoteExplorerAction::Collapse(row.path.clone())
                } else {
                    RemoteExplorerAction::Expand(row.path.clone())
                });
            } else if matches!(
                action,
                ExplorerRowAction::Click(_) | ExplorerRowAction::Activate(_)
            ) && row.kind == EntryKind::File
            {
                actions.push(RemoteExplorerAction::OpenPreview(row.path.clone()));
            }
        }
    }
    if let Some((group_index, row_index)) = result.git_row {
        if let Some(row) = git_view
            .and_then(|git| git.groups.get(group_index))
            .and_then(|group| group.rows.get(row_index))
        {
            actions.push(RemoteExplorerAction::SelectGit(row.path.clone()));
            if !row.directory {
                actions.push(RemoteExplorerAction::OpenPreview(row.path.clone()));
            }
        }
    }
    actions
}

fn collect_remote_rows(
    path: &made_remote::protocol::RemotePath,
    projection: &crate::remote_workspace::WorkspaceProjection,
    git_view: Option<&crate::remote_workspace::RemoteGitView>,
    file_marks: &std::collections::HashMap<made_remote::protocol::RemotePath, Vec<ChangeKind>>,
    rows: &mut Vec<RemoteRow>,
    depth: usize,
) {
    use crate::remote_workspace::Freshness;
    use made_remote::protocol::{MAX_TREE_DEPTH, MAX_TREE_ROWS};
    if rows.len() >= MAX_TREE_ROWS {
        return;
    }
    let Some(directory) = projection.directories.get(path) else {
        return;
    };
    for entry in &directory.entries {
        if rows.len() >= MAX_TREE_ROWS {
            return;
        }
        if rows.len() + 1 >= MAX_TREE_ROWS {
            push_remote_limit_row(
                rows,
                path,
                depth,
                format!("Tree is limited to {MAX_TREE_ROWS} visible rows."),
            );
            return;
        }
        let name = remote_file_name(&entry.path);
        let expanded = projection.expanded.contains(&entry.path);
        let kind = remote_entry_kind(entry.kind);
        let mark = match kind {
            EntryKind::Directory => git_view
                .and_then(|git| git.folder_marks.get(&entry.path))
                .map(|kinds| kinds.iter().copied().map(remote_change_kind).collect()),
            EntryKind::File => file_marks.get(&entry.path).cloned(),
            _ => None,
        };
        let icon = explorer_icons::resolve(&name, kind, expanded);
        rows.push(RemoteRow {
            path: entry.path.clone(),
            name,
            depth,
            kind,
            expanded,
            selected: projection.selected.as_ref() == Some(&entry.path),
            tooltip: remote_path_text(&entry.path),
            mark,
            icon,
        });
        if expanded && kind == EntryKind::Directory {
            if depth + 1 > MAX_TREE_DEPTH {
                push_remote_limit_row(
                    rows,
                    &entry.path,
                    depth,
                    format!("Tree expansion is limited to {MAX_TREE_DEPTH} levels."),
                );
            } else {
                match projection.directories.get(&entry.path) {
                    Some(child) if matches!(child.freshness, Freshness::Unavailable(_)) => {
                        if rows.len() + 1 >= MAX_TREE_ROWS {
                            push_remote_limit_row(
                                rows,
                                &entry.path,
                                depth,
                                format!("Tree is limited to {MAX_TREE_ROWS} visible rows."),
                            );
                            return;
                        }
                        rows.push(RemoteRow {
                            path: entry.path.clone(),
                            name: remote_file_name(&entry.path),
                            depth: depth + 1,
                            kind: EntryKind::Error,
                            expanded: false,
                            selected: false,
                            tooltip: "Remote directory unavailable".to_owned(),
                            mark: None,
                            icon: None,
                        });
                    }
                    Some(_) => collect_remote_rows(
                        &entry.path,
                        projection,
                        git_view,
                        file_marks,
                        rows,
                        depth + 1,
                    ),
                    None => {}
                }
            }
        }
    }
    if directory.truncated && rows.len() < MAX_TREE_ROWS {
        if rows.len() + 1 >= MAX_TREE_ROWS {
            push_remote_limit_row(
                rows,
                path,
                depth,
                format!("Tree is limited to {MAX_TREE_ROWS} visible rows."),
            );
            return;
        }
        rows.push(RemoteRow {
            path: directory.path.clone(),
            name: "(listing truncated)".to_owned(),
            depth,
            kind: EntryKind::Error,
            expanded: false,
            selected: false,
            tooltip: format!(
                "Directory has more than {} entries; only the first {} are shown.",
                made_explorer::tree::MAX_DIR_ENTRIES,
                made_explorer::tree::MAX_DIR_ENTRIES
            ),
            mark: None,
            icon: None,
        });
    }
}

fn push_remote_limit_row(
    rows: &mut Vec<RemoteRow>,
    path: &made_remote::protocol::RemotePath,
    depth: usize,
    message: String,
) {
    if rows.len() >= made_remote::protocol::MAX_TREE_ROWS {
        return;
    }
    rows.push(RemoteRow {
        path: path.clone(),
        name: "(tree truncated)".to_owned(),
        depth,
        kind: EntryKind::Error,
        expanded: false,
        selected: false,
        tooltip: message,
        mark: None,
        icon: None,
    });
}

fn remote_entry_kind(kind: made_remote::protocol::EntryKind) -> EntryKind {
    match kind {
        made_remote::protocol::EntryKind::Directory => EntryKind::Directory,
        made_remote::protocol::EntryKind::File => EntryKind::File,
        made_remote::protocol::EntryKind::Symlink => EntryKind::Symlink,
        made_remote::protocol::EntryKind::Other => EntryKind::Other,
    }
}

fn remote_change_kind(kind: made_remote::git::ChangeKind) -> ChangeKind {
    match kind {
        made_remote::git::ChangeKind::New => ChangeKind::New,
        made_remote::git::ChangeKind::Modified => ChangeKind::Modified,
        made_remote::git::ChangeKind::Deleted => ChangeKind::Deleted,
        made_remote::git::ChangeKind::Renamed => ChangeKind::Renamed,
        made_remote::git::ChangeKind::Conflict => ChangeKind::Conflict,
    }
}

fn remote_file_marks(
    git: Option<&crate::remote_workspace::RemoteGitView>,
) -> std::collections::HashMap<made_remote::protocol::RemotePath, Vec<ChangeKind>> {
    let mut marks = std::collections::HashMap::new();
    let Some(git) = git else {
        return marks;
    };
    for group in &git.groups {
        for row in &group.rows {
            if row.directory {
                continue;
            }
            let kind = remote_change_kind(row.kind);
            let kinds = marks.entry(row.path.clone()).or_insert_with(Vec::new);
            if !kinds.contains(&kind) {
                kinds.push(kind);
                kinds.sort();
            }
        }
    }
    marks
}

fn remote_git_header(git: &crate::remote_workspace::RemoteGitView) -> GitHeader {
    GitHeader {
        show_icon: true,
        branch: git.branch.clone(),
        detached: git.detached,
        commit_abbrev: git.commit.clone(),
        commit_subject: git.subject.clone(),
        is_linked_worktree: git.linked_worktree,
        directory_name: git.worktree_name.clone(),
    }
}

fn remote_git_body(
    root: &made_remote::protocol::RemotePath,
    git_view: Option<&crate::remote_workspace::RemoteGitView>,
    selected: Option<&made_remote::protocol::RemotePath>,
) -> GitBodyView {
    use crate::remote_workspace::Freshness;
    let Some(git) = git_view else {
        return GitBodyView {
            has_status: false,
            failed: false,
            stale: false,
            groups: Vec::new(),
        };
    };
    let mut groups = Vec::new();
    for group in &git.groups {
        if group.rows.is_empty() {
            continue;
        }
        let rows = group
            .rows
            .iter()
            .map(|row| {
                let name = remote_relative_text(root, &row.path);
                let name = if name.is_empty() {
                    remote_file_name(&row.path)
                } else {
                    name
                };
                let entry_kind = if row.directory {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                };
                GitRowView {
                    name,
                    kind: remote_change_kind(row.kind),
                    selected: selected == Some(&row.path),
                    icon: explorer_icons::resolve(&remote_file_name(&row.path), entry_kind, false),
                    tooltip: remote_path_text(&row.path),
                    original: row
                        .original
                        .as_ref()
                        .map(|path| remote_relative_text(root, path)),
                }
            })
            .collect();
        groups.push(GitGroupView {
            title: group.title,
            rows,
        });
    }
    let unavailable = matches!(git.freshness, Freshness::Unavailable(_));
    let failed = !git.has_status && unavailable;
    GitBodyView {
        has_status: git.has_status,
        failed,
        stale: git.has_status && unavailable,
        groups,
    }
}

fn remote_source_control_tooltip(git: Option<&crate::remote_workspace::RemoteGitView>) -> String {
    let Some(git) = git else {
        return "Source Control".to_owned();
    };
    let mut parts = Vec::new();
    for title in [
        crate::worktree::GROUP_MERGE,
        crate::worktree::GROUP_STAGED,
        crate::worktree::GROUP_CHANGES,
        crate::worktree::GROUP_UNTRACKED,
    ] {
        let count: usize = git
            .groups
            .iter()
            .filter(|group| group.title == title)
            .map(|group| group.rows.len())
            .sum();
        if count > 0 {
            parts.push(format!("{title} ({count})"));
        }
    }
    if parts.is_empty() {
        "Source Control".to_owned()
    } else {
        format!("Source Control - {}", parts.join(" · "))
    }
}

fn remote_source_control_rings(
    git: Option<&crate::remote_workspace::RemoteGitView>,
) -> theme::SourceControlRings {
    let Some(git) = git else {
        return theme::SourceControlRings::default();
    };
    let mut rings = theme::SourceControlRings::default();
    for row in git.groups.iter().flat_map(|group| &group.rows) {
        match remote_change_kind(row.kind) {
            ChangeKind::New => rings.highest = Some(theme::SUCCESS),
            ChangeKind::Modified => rings.middle = Some(theme::WARNING),
            ChangeKind::Deleted | ChangeKind::Conflict => rings.lowest = Some(theme::ERROR),
            ChangeKind::Renamed => {}
        }
    }
    rings
}

fn remote_relative_text(
    root: &made_remote::protocol::RemotePath,
    path: &made_remote::protocol::RemotePath,
) -> String {
    let root_bytes = root.as_bytes();
    let path_bytes = path.as_bytes();
    let relative = if path_bytes == root_bytes {
        &[][..]
    } else if path_bytes.starts_with(root_bytes) {
        let start = if root_bytes == b"/" {
            1
        } else {
            root_bytes.len() + 1
        };
        path_bytes.get(start..).unwrap_or(&[])
    } else {
        path_bytes
    };
    String::from_utf8_lossy(relative).into_owned()
}

fn remote_root_name(path: &made_remote::protocol::RemotePath) -> String {
    let text = remote_path_text(path);
    text.rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(&text)
        .to_owned()
}

pub(crate) fn remote_path_text(path: &made_remote::protocol::RemotePath) -> String {
    path.to_utf8()
        .map(str::to_owned)
        .unwrap_or_else(|_| String::from_utf8_lossy(path.as_bytes()).into_owned())
}

pub(crate) fn remote_file_name(path: &made_remote::protocol::RemotePath) -> String {
    let text = remote_path_text(path);
    text.rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or("remote")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::GitStatusRow;
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

    fn header_circles(output: &egui::FullOutput) -> Vec<egui::epaint::CircleShape> {
        fn visit(shape: &egui::Shape, circles: &mut Vec<egui::epaint::CircleShape>) {
            match shape {
                egui::Shape::Circle(circle) => circles.push(*circle),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        visit(shape, circles);
                    }
                }
                _ => {}
            }
        }
        let mut circles = Vec::new();
        for clipped in &output.shapes {
            visit(&clipped.shape, &mut circles);
        }
        circles
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
    fn root_name_width_stops_before_the_icon_and_its_gap() {
        let width = root_name_width(80.0, 200.0, 8.0, 24.0, true);
        assert_eq!(width, 200.0 - 52.0 - 8.0 - 80.0 - 8.0);
        assert!(width + 80.0 + 8.0 <= 200.0 - 52.0 - 8.0);
        assert_eq!(root_name_width(80.0, 200.0, 8.0, 24.0, false), 112.0);
        assert_eq!(root_name_width(190.0, 200.0, 8.0, 24.0, true), 0.0);
    }

    #[test]
    fn source_control_head_uses_branch_or_commit_id_and_linked_label_only() {
        assert_eq!(source_control_head_label(Some("main"), false, None), "main");
        assert_eq!(
            source_control_worktree_name("feature", true, Some("linked")).as_deref(),
            Some("linked")
        );
        assert_eq!(
            source_control_worktree_name("feature", true, Some("feature")),
            None
        );
        assert_eq!(
            repos_aside_texts("feature", true, Some("feature")),
            (String::new(), Some("feature".to_owned()))
        );
        assert_eq!(
            source_control_worktree_name("main", false, Some("main")),
            None
        );
        assert_eq!(
            source_control_head_label(None, true, Some("fb16d0ee")),
            "fb16d0ee"
        );
        assert!(!source_control_head_label(None, true, Some("fb16d0ee")).contains("Detached HEAD"));
        assert_eq!(
            source_control_worktree_name("fb16d0ee", true, Some("linked")).as_deref(),
            Some("linked")
        );
        assert_eq!(
            source_control_worktree_name("fb16d0ee", true, Some("fb16d0ee")),
            None
        );
        assert_eq!(
            repos_aside_texts("fb16d0ee", true, Some("fb16d0ee")),
            (String::new(), Some("fb16d0ee".to_owned()))
        );
        assert_eq!(
            repos_aside_texts("feature", true, Some("linked")),
            ("feature".to_owned(), Some("linked".to_owned()))
        );
        assert_eq!(
            repos_aside_texts("main", false, Some("main")),
            ("main".to_owned(), None)
        );
        assert_eq!(source_control_worktree_name("", false, Some("nope")), None);
        let hover = linked_worktree_hover("linked");
        assert!(hover.contains("linked"));
        assert!(hover.contains("linked worktree"));
        assert!(!hover.contains(" · "));
    }

    #[test]
    fn detached_head_hover_shows_subject_or_id_and_keeps_the_visible_line() {
        let line = source_control_head_label(None, true, Some("fb16d0ee"));
        let hover =
            source_control_head_hover(&line, true, Some("fb16d0ee"), Some("Fix the header"));
        assert_eq!(line, "fb16d0ee");
        assert!(hover.contains("Fix the header"));
        assert!(hover.contains("fb16d0ee"));
        assert!(!hover.contains("Detached HEAD"));
        assert!(!hover.contains("is a linked worktree"));

        let empty = source_control_head_hover(&line, true, Some("fb16d0ee"), Some("  "));
        assert!(empty.contains("fb16d0ee"));
        assert!(!empty.contains("Fix the header"));

        let linked = source_control_head_label(None, true, Some("fb16d0eeabcd"));
        let linked_hover =
            source_control_head_hover(&linked, true, Some("fb16d0eeabcd"), Some("Fix the header"));
        assert_eq!(linked, "fb16d0ee");
        assert_eq!(
            source_control_worktree_name(&linked, true, Some("linked")).as_deref(),
            Some("linked")
        );
        assert!(linked_hover.contains("Fix the header"));
        assert!(linked_hover.contains("fb16d0eeabcd"));
        assert!(!linked_hover.contains(" · "));
        assert!(!linked_hover.contains("Detached HEAD"));
        assert!(!linked_hover.contains("is a linked worktree"));
    }

    #[test]
    fn git_view_header_shows_repos_and_does_not_repeat_the_head_line() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace-root");
        fs::create_dir(&root).unwrap();
        let mut pane = ExplorerPane::new(&root, RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);
        let head = "feature";
        pane.apply_git_chrome(ExplorerGitChrome {
            header: GitHeader {
                show_icon: true,
                branch: Some("feature".to_owned()),
                detached: false,
                commit_abbrev: None,
                commit_subject: None,
                is_linked_worktree: true,
                directory_name: Some("linked".to_owned()),
            },
            status: Some(GitStatusSnapshot {
                groups: vec![crate::worktree::GitStatusGroup {
                    title: crate::worktree::GROUP_CHANGES,
                    rows: vec![GitStatusRow {
                        letter: 'M',
                        kind: ChangeKind::Modified,
                        path: root.join("notes.txt"),
                        relative: PathBuf::from("notes.txt"),
                        is_directory: false,
                        original_relative: None,
                    }],
                }],
                ..Default::default()
            }),
            ..ExplorerGitChrome::default()
        });

        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        pane.set_ui_scope(egui::Id::unique("repos-header"));
        let mut render =
            |pane: &mut ExplorerPane, width: f32, height: f32, events: Vec<egui::Event>| {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    events,
                    ..Default::default()
                };
                let mut output = ctx.run_ui(raw, |ui| {
                    pane.tree_ui(ui, &mut log, &theme::Layout::for_window(width, height));
                });
                output.textures_delta.clear();
                output
            };

        let tree = render(&mut pane, 900.0, 620.0, Vec::new());
        let tree_text = output_text(&tree);
        assert!(tree_text.contains("EXPLORER"), "{tree_text}");
        assert!(tree_text.contains("workspace-root"), "{tree_text}");
        assert!(!tree_text.contains("REPOS"), "{tree_text}");
        assert!(!tree_text.contains(head), "{tree_text}");

        pane.source_control_open = true;
        let git = render(&mut pane, 900.0, 620.0, Vec::new());
        let git_text = output_text(&git);
        assert!(git_text.contains("REPOS"), "{git_text}");
        assert!(!git_text.contains("EXPLORER"), "{git_text}");
        assert!(
            !git_text.contains("workspace-root"),
            "git view must not show the root name: {git_text}"
        );
        assert_eq!(
            git_text.matches(head).count(),
            1,
            "head line must be painted once, not repeated in the body: {git_text}"
        );
        assert!(git_text.contains("linked"), "{git_text}");
        assert!(!git_text.contains(" · "), "{git_text}");
        assert!(git_text.contains("Changes"), "{git_text}");
        assert!(git_text.contains("notes.txt"), "{git_text}");

        pane.git_header.detached = true;
        pane.git_header.branch = None;
        pane.git_header.commit_abbrev = Some("fb16d0ee".to_owned());
        pane.git_header.commit_subject = Some("Fix the header".to_owned());
        let detached = render(&mut pane, 900.0, 620.0, Vec::new());
        let detached_text = output_text(&detached);
        assert_eq!(detached_text.matches("fb16d0ee").count(), 1);
        assert!(detached_text.contains("linked"), "{detached_text}");
        assert!(!detached_text.contains(" · "), "{detached_text}");
        assert!(!detached_text.contains("Detached HEAD"), "{detached_text}");
        assert!(!detached_text.contains("feature"), "{detached_text}");
        let id_shape = detached
            .shapes
            .iter()
            .find(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => text.galley.job.text.contains("fb16d0ee"),
                _ => false,
            })
            .expect("commit id");
        let egui::Shape::Text(id_text) = &id_shape.shape else {
            unreachable!("commit id");
        };
        let id_at = id_text.pos + id_text.galley.size() * 0.5;
        let moved = vec![egui::Event::PointerMoved(id_at)];
        let _ = render(&mut pane, 900.0, 620.0, moved.clone());
        let _ = render(&mut pane, 900.0, 620.0, moved);
        let hovered = render(&mut pane, 900.0, 620.0, Vec::new());
        let hovered_text = output_text(&hovered);
        assert!(
            hovered_text.contains("Fix the header"),
            "hover must show the commit subject: {hovered_text}"
        );
        assert!(
            hovered_text.contains("fb16d0ee"),
            "hover must keep the commit id: {hovered_text}"
        );
        assert!(
            !hovered_text.contains("is a linked worktree"),
            "commit-id hover must not become the worktree hover: {hovered_text}"
        );
        pane.git_header.commit_subject = Some(String::new());
        let _ = render(&mut pane, 900.0, 620.0, Vec::new());
        let empty_moved = vec![egui::Event::PointerMoved(id_at)];
        let _ = render(&mut pane, 900.0, 620.0, empty_moved.clone());
        let _ = render(&mut pane, 900.0, 620.0, empty_moved);
        let empty_hover = output_text(&render(&mut pane, 900.0, 620.0, Vec::new()));
        assert!(
            empty_hover.contains("fb16d0ee"),
            "empty subject hover must show the id: {empty_hover}"
        );
        assert!(
            !empty_hover.contains("Fix the header"),
            "empty subject must not keep the previous subject: {empty_hover}"
        );

        let marker = "hover-marker";
        let branch = format!("branch-{}-{marker}", "x".repeat(170));
        pane.git_header.detached = false;
        pane.git_header.is_linked_worktree = false;
        pane.git_header.branch = Some(branch);
        pane.git_header.directory_name = None;
        let long_head = source_control_head_label(pane.git_header.branch.as_deref(), false, None);
        assert!(long_head.chars().count() > 160);
        assert!(long_head.contains(marker));
        // Leave the previous commit-id hover so its tooltip cannot leak the marker.
        let narrow = render(
            &mut pane,
            280.0,
            400.0,
            vec![egui::Event::PointerMoved(egui::pos2(1.0, 390.0))],
        );
        let head_shape = narrow
            .shapes
            .iter()
            .find(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => text.galley.job.text.starts_with("branch-"),
                _ => false,
            })
            .expect("head line");
        let egui::Shape::Text(text) = &head_shape.shape else {
            unreachable!("head line");
        };
        assert!(text.galley.elided, "long head line must truncate");
        let icons_left = 280.0 - theme::Layout::for_window(280.0, 400.0).header_padding_x - 52.0;
        assert!(
            head_shape.clip_rect.right() <= icons_left - 8.0 + 0.5,
            "head clip {:?} must stop before both icons at {icons_left}",
            head_shape.clip_rect
        );
        assert!(
            !output_text(&narrow).contains(marker),
            "the painted head line must not include the tail"
        );
        let hover_at = text.pos + text.galley.size() * 0.5;
        // The widget rect comes from the previous pass. The tooltip Area is
        // painted on the pass after hover begins.
        let moved = vec![egui::Event::PointerMoved(hover_at)];
        let _ = render(&mut pane, 280.0, 400.0, moved.clone());
        let _ = render(&mut pane, 280.0, 400.0, moved);
        let hovered = render(&mut pane, 280.0, 400.0, Vec::new());
        assert!(
            output_text(&hovered).contains(marker),
            "hover must keep the full head line at {hover_at:?}: {}",
            output_text(&hovered)
        );
    }

    #[test]
    fn attached_linked_worktree_header_uses_the_folder_name_not_the_branch() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main");
        let linked = temp.path().join("checkout-dir");
        fs::create_dir_all(&main).unwrap();
        let run_git = |dir: &Path, args: &[&str]| {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_AUTHOR_NAME", "Made Test")
                .env("GIT_AUTHOR_EMAIL", "made@example.invalid")
                .env("GIT_COMMITTER_NAME", "Made Test")
                .env("GIT_COMMITTER_EMAIL", "made@example.invalid")
                .output()
                .expect("git");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run_git(&main, &["init", "-q", "-b", "main"]);
        fs::write(main.join("README.md"), "# fixture\n").unwrap();
        run_git(&main, &["add", "README.md"]);
        run_git(&main, &["commit", "-q", "-m", "initial"]);
        run_git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                linked.to_str().unwrap(),
            ],
        );

        let metadata = crate::worktree::GitRunner::with_program("git").read_metadata(&linked);
        assert_eq!(metadata.branch.as_deref(), Some("feature"));
        assert_eq!(
            metadata.worktree_label.as_deref(),
            Some("feature"),
            "diagnostics label stays the branch"
        );
        assert_eq!(metadata.directory_name.as_deref(), Some("checkout-dir"));
        assert!(metadata.is_linked_worktree);

        let mut pane = ExplorerPane::new(&linked, RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);
        pane.apply_git_chrome(ExplorerGitChrome {
            header: GitHeader {
                show_icon: metadata.is_known() && metadata.is_worktree,
                branch: metadata.branch.clone(),
                detached: metadata.detached,
                commit_abbrev: metadata.commit_abbrev.clone(),
                commit_subject: metadata.commit_subject.clone(),
                is_linked_worktree: metadata.is_linked_worktree,
                directory_name: metadata.directory_name.clone(),
            },
            ..ExplorerGitChrome::default()
        });
        pane.source_control_open = true;
        let ctx = egui::Context::default();
        theme::install(&ctx);
        pane.set_ui_scope(egui::Id::unique("folder-name-header"));
        let mut render = |pane: &mut ExplorerPane| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 620.0),
                )),
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                pane.tree_ui(ui, &mut log, &theme::Layout::for_window(900.0, 620.0));
            });
            output.textures_delta.clear();
            output
        };
        let attached = render(&mut pane);
        let attached_text = output_text(&attached);
        assert!(attached_text.contains("feature"), "{attached_text}");
        assert!(
            attached_text.contains("checkout-dir"),
            "header name must be the folder, not the branch: {attached_text}"
        );
        assert!(!attached_text.contains(" · "), "{attached_text}");
        let name = attached
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.job.text.contains("checkout-dir") => {
                    Some(text)
                }
                _ => None,
            })
            .expect("folder name");
        assert!(
            header_circles(&attached).iter().any(|circle| {
                circle.center.x < name.pos.x && name.pos.x - circle.center.x < 18.0
            }),
            "folder name needs the worktree icon"
        );

        run_git(&linked, &["checkout", "-q", "--detach"]);
        let detached_meta = crate::worktree::GitRunner::with_program("git").read_metadata(&linked);
        assert!(detached_meta.detached);
        assert_eq!(
            detached_meta.directory_name.as_deref(),
            Some("checkout-dir")
        );
        pane.apply_git_chrome(ExplorerGitChrome {
            header: GitHeader {
                show_icon: true,
                branch: detached_meta.branch.clone(),
                detached: true,
                commit_abbrev: detached_meta.commit_abbrev.clone(),
                commit_subject: detached_meta.commit_subject.clone(),
                is_linked_worktree: true,
                directory_name: detached_meta.directory_name.clone(),
            },
            ..ExplorerGitChrome::default()
        });
        pane.source_control_open = true;
        let detached = render(&mut pane);
        let detached_text = output_text(&detached);
        let id = visible_commit_id(detached_meta.commit_abbrev.as_deref());
        assert_eq!(id.chars().count(), 8);
        assert!(detached_text.contains(&id), "{detached_text}");
        assert!(detached_text.contains("checkout-dir"), "{detached_text}");
        assert!(!detached_text.contains("Detached HEAD"), "{detached_text}");
        assert!(!detached_text.contains(" · "), "{detached_text}");
    }

    #[test]
    fn matching_worktree_name_still_shows_the_icon() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace-root");
        fs::create_dir(&root).unwrap();
        let mut pane = ExplorerPane::new(&root, RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);
        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        pane.set_ui_scope(egui::Id::unique("matching-worktree-icon"));
        let mut render = |pane: &mut ExplorerPane, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 620.0),
                )),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                pane.tree_ui(ui, &mut log, &theme::Layout::for_window(900.0, 620.0));
            });
            output.textures_delta.clear();
            output
        };
        fn hover_icon(
            render: &mut impl FnMut(&mut ExplorerPane, Vec<egui::Event>) -> egui::FullOutput,
            pane: &mut ExplorerPane,
            output: &egui::FullOutput,
            label: &str,
        ) -> String {
            let text = output
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) if text.galley.job.text.contains(label) => Some(text),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing {label}"));
            let mark = header_circles(output)
                .into_iter()
                .find(|circle| {
                    circle.center.x < text.pos.x
                        && text.pos.x - circle.center.x < 20.0
                        && (circle.center.y - (text.pos.y + text.galley.size().y * 0.5)).abs()
                            < 10.0
                })
                .unwrap_or_else(|| panic!("{label} has no worktree icon before the text"));
            let moved = vec![egui::Event::PointerMoved(mark.center)];
            let _ = render(pane, moved.clone());
            let _ = render(pane, moved);
            output_text(&render(pane, Vec::new()))
        }

        pane.apply_git_chrome(ExplorerGitChrome {
            header: GitHeader {
                show_icon: true,
                branch: Some("feature".to_owned()),
                detached: false,
                commit_abbrev: None,
                commit_subject: None,
                is_linked_worktree: true,
                directory_name: Some("feature".to_owned()),
            },
            ..ExplorerGitChrome::default()
        });
        pane.source_control_open = true;
        let attached = render(&mut pane, Vec::new());
        let attached_text = output_text(&attached);
        assert_eq!(
            attached_text.matches("feature").count(),
            1,
            "branch must not be printed again: {attached_text}"
        );
        let attached_hover = hover_icon(&mut render, &mut pane, &attached, "feature");
        assert!(
            attached_hover.contains("is a linked worktree"),
            "{attached_hover}"
        );
        assert!(attached_hover.contains("feature"), "{attached_hover}");
        let name_at = attached
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.job.text.contains("feature") => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("folder name");
        let name_moved = vec![egui::Event::PointerMoved(name_at)];
        let _ = render(&mut pane, Vec::new());
        let _ = render(&mut pane, name_moved.clone());
        let _ = render(&mut pane, name_moved);
        let name_hover = output_text(&render(&mut pane, Vec::new()));
        assert!(
            name_hover.contains("is a linked worktree"),
            "name hover: {name_hover}"
        );
        assert!(name_hover.contains("feature"), "{name_hover}");

        pane.apply_git_chrome(ExplorerGitChrome {
            header: GitHeader {
                show_icon: true,
                branch: None,
                detached: true,
                commit_abbrev: Some("fb16d0ee".to_owned()),
                commit_subject: Some("Fix the header".to_owned()),
                is_linked_worktree: true,
                directory_name: Some("fb16d0ee".to_owned()),
            },
            ..ExplorerGitChrome::default()
        });
        pane.source_control_open = true;
        let detached = render(
            &mut pane,
            vec![egui::Event::PointerMoved(egui::pos2(1.0, 600.0))],
        );
        let detached_text = output_text(&detached);
        assert_eq!(
            detached_text.matches("fb16d0ee").count(),
            1,
            "id must not be printed again: {detached_text}"
        );
        assert!(!detached_text.contains("Detached HEAD"), "{detached_text}");
        let detached_hover = hover_icon(&mut render, &mut pane, &detached, "fb16d0ee");
        assert!(
            detached_hover.contains("is a linked worktree"),
            "{detached_hover}"
        );
        assert!(detached_hover.contains("fb16d0ee"), "{detached_hover}");

        pane.apply_git_chrome(ExplorerGitChrome {
            header: GitHeader {
                show_icon: true,
                branch: Some("main".to_owned()),
                detached: false,
                commit_abbrev: None,
                commit_subject: None,
                is_linked_worktree: false,
                directory_name: Some("main".to_owned()),
            },
            ..ExplorerGitChrome::default()
        });
        pane.source_control_open = true;
        let plain = render(
            &mut pane,
            vec![egui::Event::PointerMoved(egui::pos2(1.0, 600.0))],
        );
        let plain_text = output_text(&plain);
        assert_eq!(plain_text.matches("main").count(), 1, "{plain_text}");
        assert!(
            header_circles(&plain)
                .iter()
                .all(|circle| circle.center.x > 800.0),
            "a non-linked checkout has no worktree icon"
        );
    }

    #[test]
    fn linked_worktree_name_has_an_icon_and_a_non_linked_checkout_does_not() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace-root");
        fs::create_dir(&root).unwrap();
        let mut pane = ExplorerPane::new(&root, RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);
        pane.apply_git_chrome(ExplorerGitChrome {
            header: GitHeader {
                show_icon: true,
                branch: None,
                detached: true,
                commit_abbrev: Some("fb16d0ee".to_owned()),
                commit_subject: Some("Fix the header".to_owned()),
                is_linked_worktree: true,
                directory_name: Some("feat-git-repos".to_owned()),
            },
            ..ExplorerGitChrome::default()
        });
        pane.source_control_open = true;

        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        pane.set_ui_scope(egui::Id::unique("linked-worktree-icon"));
        let mut render = |pane: &mut ExplorerPane, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 620.0),
                )),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                pane.tree_ui(ui, &mut log, &theme::Layout::for_window(900.0, 620.0));
            });
            output.textures_delta.clear();
            output
        };

        let linked = render(&mut pane, Vec::new());
        let linked_text = output_text(&linked);
        assert!(linked_text.contains("fb16d0ee"), "{linked_text}");
        assert!(linked_text.contains("feat-git-repos"), "{linked_text}");
        assert!(!linked_text.contains(" · "), "{linked_text}");
        assert!(!linked_text.contains("Detached HEAD"), "{linked_text}");
        let name = linked
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.job.text.contains("feat-git-repos") => {
                    Some(text)
                }
                _ => None,
            })
            .expect("worktree name");
        let circles = header_circles(&linked);
        let mark = circles
            .iter()
            .find(|circle| {
                circle.center.x < name.pos.x
                    && name.pos.x - circle.center.x < 18.0
                    && (circle.center.y - (name.pos.y + name.galley.size().y * 0.5)).abs() < 10.0
            })
            .expect("icon immediately before the worktree name");
        assert!(
            mark.radius < 1.2,
            "worktree mark is the small glyph, not a status dot"
        );

        let name_at = name.pos + name.galley.size() * 0.5;
        let moved = vec![egui::Event::PointerMoved(name_at)];
        let _ = render(&mut pane, moved.clone());
        let _ = render(&mut pane, moved);
        let name_hover = output_text(&render(&mut pane, Vec::new()));
        assert!(
            name_hover.contains("feat-git-repos"),
            "name hover includes the worktree name: {name_hover}"
        );
        assert!(
            name_hover.contains("is a linked worktree"),
            "name hover says it is a linked worktree: {name_hover}"
        );

        let id = linked
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.job.text.contains("fb16d0ee") => Some(text),
                _ => None,
            })
            .expect("commit id");
        let id_at = id.pos + id.galley.size() * 0.5;
        let id_moved = vec![egui::Event::PointerMoved(id_at)];
        let _ = render(&mut pane, Vec::new());
        let _ = render(&mut pane, id_moved.clone());
        let _ = render(&mut pane, id_moved);
        let id_hover = output_text(&render(&mut pane, Vec::new()));
        assert!(
            id_hover.contains("Fix the header"),
            "commit-id hover keeps the subject: {id_hover}"
        );
        assert!(
            !id_hover.contains("is a linked worktree"),
            "commit-id hover must not become the worktree hover: {id_hover}"
        );

        pane.apply_git_chrome(ExplorerGitChrome {
            header: GitHeader {
                show_icon: true,
                branch: Some("main".to_owned()),
                detached: false,
                commit_abbrev: None,
                commit_subject: None,
                is_linked_worktree: false,
                directory_name: Some("feat-git-repos".to_owned()),
            },
            ..ExplorerGitChrome::default()
        });
        let plain = render(
            &mut pane,
            vec![egui::Event::PointerMoved(egui::pos2(1.0, 600.0))],
        );
        let plain_text = output_text(&plain);
        assert!(plain_text.contains("main"), "{plain_text}");
        assert!(!plain_text.contains("feat-git-repos"), "{plain_text}");
        assert!(
            header_circles(&plain)
                .iter()
                .all(|circle| circle.center.x > 800.0),
            "a non-linked checkout has no worktree icon in the header text"
        );
    }

    #[test]
    fn source_control_rings_follow_open_changes_and_leave_the_others_original() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        fs::create_dir(&root).unwrap();
        let mut pane = ExplorerPane::new(&root, RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);

        let ctx = egui::Context::default();
        theme::install(&ctx);
        pane.set_ui_scope(egui::Id::unique("source-control-rings"));
        let mut render = |pane: &mut ExplorerPane, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 620.0),
                )),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                pane.tree_ui(ui, &mut log, &theme::Layout::for_window(900.0, 620.0));
            });
            output.textures_delta.clear();
            output
        };
        let away = vec![egui::Event::PointerMoved(egui::pos2(1.0, 600.0))];
        let paint =
            |render: &mut dyn FnMut(&mut ExplorerPane, Vec<egui::Event>) -> egui::FullOutput,
             pane: &mut ExplorerPane| { render(pane, away.clone()) };
        let row = |kind: ChangeKind, letter: char, directory: bool| GitStatusRow {
            letter,
            kind,
            path: root.join(if directory { "vendor" } else { "notes.txt" }),
            relative: PathBuf::from(if directory { "vendor" } else { "notes.txt" }),
            is_directory: directory,
            original_relative: None,
        };
        let show = |pane: &mut ExplorerPane,
                    status: Option<GitStatusSnapshot>,
                    linked: bool,
                    open: bool| {
            pane.apply_git_chrome(ExplorerGitChrome {
                header: GitHeader {
                    show_icon: true,
                    branch: Some("main".to_owned()),
                    detached: false,
                    commit_abbrev: None,
                    commit_subject: None,
                    is_linked_worktree: linked,
                    directory_name: linked.then(|| "linked".to_owned()),
                },
                status,
                ..ExplorerGitChrome::default()
            });
            pane.source_control_open = open;
        };
        let rings = |output: &egui::FullOutput| {
            let mut found: Vec<_> = header_circles(output)
                .into_iter()
                .filter(|circle| {
                    (circle.radius - 1.25).abs() < 0.05
                        && circle.stroke.width > 1.0
                        && circle.center.x > 800.0
                })
                .collect();
            found.sort_by(|a, b| a.center.y.total_cmp(&b.center.y));
            assert_eq!(found.len(), 3, "the three source-control rings");
            assert!(
                (found[0].center.x - found[2].center.x).abs() < 0.5,
                "highest and lowest rings stay on the stem"
            );
            assert!(
                found[1].center.x > found[0].center.x + 5.0,
                "middle ring is the branch tip"
            );
            [found[0], found[1], found[2]]
        };
        let ring_fill = |output: &egui::FullOutput, at: egui::Pos2| {
            header_circles(output).into_iter().find(|circle| {
                (circle.radius - 1.25).abs() < 0.05
                    && circle.fill != Color32::TRANSPARENT
                    && circle.center.distance(at) < 0.5
            })
        };
        let assert_stem_and_curve = |output: &egui::FullOutput, expected: Color32| {
            let [highest, middle, lowest] = rings(output);
            let anchor = egui::pos2(
                (highest.center.x + lowest.center.x) * 0.5,
                (highest.center.y + lowest.center.y) * 0.5,
            );
            let near =
                |pos: egui::Pos2| pos.distance(anchor) < 10.0 || pos.distance(middle.center) < 10.0;
            let solid = |stroke: &egui::epaint::PathStroke| match stroke.color {
                egui::epaint::ColorMode::Solid(color) => color,
                _ => panic!("glyph stroke should be a solid color"),
            };
            let mut stem = false;
            let mut curve = false;
            fn walk(
                shape: &egui::Shape,
                near: &dyn Fn(egui::Pos2) -> bool,
                solid: &dyn Fn(&egui::epaint::PathStroke) -> Color32,
                expected: Color32,
                stem: &mut bool,
                curve: &mut bool,
            ) {
                match shape {
                    egui::Shape::Path(path)
                        if path.stroke.width > 1.2 && path.points.iter().any(|pos| near(*pos)) =>
                    {
                        assert_eq!(solid(&path.stroke), expected, "stem keeps the icon color");
                        *stem = true;
                    }
                    egui::Shape::CubicBezier(curve_shape)
                        if curve_shape.stroke.width > 1.2
                            && curve_shape.points.iter().any(|pos| near(*pos)) =>
                    {
                        assert_eq!(
                            solid(&curve_shape.stroke),
                            expected,
                            "curve keeps the icon color"
                        );
                        *curve = true;
                    }
                    egui::Shape::Vec(shapes) => {
                        for shape in shapes {
                            walk(shape, near, solid, expected, stem, curve);
                        }
                    }
                    _ => {}
                }
            }
            for clipped in &output.shapes {
                walk(
                    &clipped.shape,
                    &near,
                    &solid,
                    expected,
                    &mut stem,
                    &mut curve,
                );
            }
            assert!(stem, "stem must still be painted");
            assert!(curve, "curve must still be painted");
        };
        let rest = theme::inactive_icon_color();

        show(
            &mut pane,
            Some(GitStatusSnapshot {
                groups: vec![crate::worktree::GitStatusGroup {
                    title: crate::worktree::GROUP_CHANGES,
                    rows: vec![row(ChangeKind::Modified, 'M', false)],
                }],
                ..Default::default()
            }),
            false,
            false,
        );
        let modified = paint(&mut render, &mut pane);
        let [highest, middle, lowest] = rings(&modified);
        assert_eq!(middle.stroke.color, theme::WARNING);
        assert_eq!(highest.stroke.color, rest);
        assert_eq!(lowest.stroke.color, rest);
        assert_stem_and_curve(&modified, rest);
        assert!(
            ring_fill(&modified, middle.center).is_none(),
            "off hover a colored ring stays hollow"
        );

        show(
            &mut pane,
            Some(GitStatusSnapshot {
                groups: vec![crate::worktree::GitStatusGroup {
                    title: crate::worktree::GROUP_UNTRACKED,
                    rows: vec![row(ChangeKind::New, 'U', true)],
                }],
                ..Default::default()
            }),
            false,
            false,
        );
        let [highest, middle, lowest] = rings(&paint(&mut render, &mut pane));
        assert_eq!(
            highest.stroke.color,
            theme::SUCCESS,
            "a directory row counts as new"
        );
        assert_eq!(middle.stroke.color, rest);
        assert_eq!(lowest.stroke.color, rest);

        show(
            &mut pane,
            Some(GitStatusSnapshot {
                groups: vec![crate::worktree::GitStatusGroup {
                    title: crate::worktree::GROUP_STAGED,
                    rows: vec![row(ChangeKind::New, 'A', false)],
                }],
                ..Default::default()
            }),
            false,
            false,
        );
        let [highest, middle, lowest] = rings(&paint(&mut render, &mut pane));
        assert_eq!(highest.stroke.color, theme::SUCCESS, "a staged add is new");
        assert_eq!(middle.stroke.color, rest);
        assert_eq!(lowest.stroke.color, rest);

        show(
            &mut pane,
            Some(GitStatusSnapshot {
                groups: vec![crate::worktree::GitStatusGroup {
                    title: crate::worktree::GROUP_CHANGES,
                    rows: vec![row(ChangeKind::Deleted, 'D', false)],
                }],
                ..Default::default()
            }),
            false,
            false,
        );
        let [highest, middle, lowest] = rings(&paint(&mut render, &mut pane));
        assert_eq!(lowest.stroke.color, theme::ERROR);
        assert_eq!(highest.stroke.color, rest);
        assert_eq!(middle.stroke.color, rest);

        show(
            &mut pane,
            Some(GitStatusSnapshot {
                groups: vec![crate::worktree::GitStatusGroup {
                    title: crate::worktree::GROUP_MERGE,
                    rows: vec![row(ChangeKind::Conflict, 'C', false)],
                }],
                ..Default::default()
            }),
            false,
            false,
        );
        let [highest, middle, lowest] = rings(&paint(&mut render, &mut pane));
        assert_eq!(
            lowest.stroke.color,
            theme::ERROR,
            "a conflict lights only the lowest ring"
        );
        assert_eq!(highest.stroke.color, rest);
        assert_eq!(middle.stroke.color, rest);

        show(
            &mut pane,
            Some(GitStatusSnapshot {
                groups: vec![crate::worktree::GitStatusGroup {
                    title: crate::worktree::GROUP_CHANGES,
                    rows: vec![row(ChangeKind::Renamed, 'R', false)],
                }],
                ..Default::default()
            }),
            false,
            false,
        );
        let [highest, middle, lowest] = rings(&paint(&mut render, &mut pane));
        assert_eq!(highest.stroke.color, rest, "a rename does not light a ring");
        assert_eq!(middle.stroke.color, rest);
        assert_eq!(lowest.stroke.color, rest);

        show(&mut pane, Some(GitStatusSnapshot::default()), false, false);
        let [highest, middle, lowest] = rings(&paint(&mut render, &mut pane));
        assert_eq!(
            highest.stroke.color, rest,
            "a clean snapshot leaves the rings original"
        );
        assert_eq!(middle.stroke.color, rest);
        assert_eq!(lowest.stroke.color, rest);

        show(&mut pane, None, false, false);
        let [highest, middle, lowest] = rings(&paint(&mut render, &mut pane));
        assert_eq!(
            highest.stroke.color, rest,
            "a missing snapshot leaves the rings original"
        );
        assert_eq!(middle.stroke.color, rest);
        assert_eq!(lowest.stroke.color, rest);

        show(
            &mut pane,
            Some(GitStatusSnapshot {
                groups: vec![crate::worktree::GitStatusGroup {
                    title: crate::worktree::GROUP_CHANGES,
                    rows: vec![row(ChangeKind::Modified, 'M', false)],
                }],
                ..Default::default()
            }),
            true,
            true,
        );
        let selected = paint(&mut render, &mut pane);
        let [highest, middle, lowest] = rings(&selected);
        assert_eq!(
            middle.stroke.color,
            theme::WARNING,
            "selected treatment must not replace a lit ring"
        );
        assert_eq!(highest.stroke.color, theme::TEXT_PRIMARY);
        assert_eq!(lowest.stroke.color, theme::TEXT_PRIMARY);
        assert_stem_and_curve(&selected, theme::TEXT_PRIMARY);
        let selected_fill =
            ring_fill(&selected, middle.center).expect("selected colored ring stays filled");
        assert_eq!(selected_fill.fill, theme::WARNING);
        assert!((selected_fill.radius - 1.25).abs() < 0.05);
        assert!(
            ring_fill(&selected, highest.center).is_none(),
            "an original-color ring stays hollow when selected"
        );
        assert!(ring_fill(&selected, lowest.center).is_none());
        let selected_fills = header_circles(&selected)
            .into_iter()
            .filter(|circle| {
                (circle.radius - 1.25).abs() < 0.05 && circle.fill != Color32::TRANSPARENT
            })
            .count();
        assert_eq!(
            selected_fills, 1,
            "selected fill covers only the colored ring, not the stem or curve"
        );
        let worktree = header_circles(&selected)
            .into_iter()
            .filter(|circle| circle.radius < 1.2 && circle.fill != Color32::TRANSPARENT)
            .collect::<Vec<_>>();
        assert!(
            !worktree.is_empty(),
            "linked-worktree icon is still painted"
        );
        assert!(
            worktree
                .iter()
                .all(|circle| circle.fill == theme::TEXT_TERTIARY),
            "linked-worktree icon stays uncolored: {worktree:?}"
        );

        show(
            &mut pane,
            Some(GitStatusSnapshot {
                groups: vec![crate::worktree::GitStatusGroup {
                    title: crate::worktree::GROUP_UNTRACKED,
                    rows: vec![row(ChangeKind::New, 'U', true)],
                }],
                ..Default::default()
            }),
            false,
            false,
        );
        let placed = paint(&mut render, &mut pane);
        let hover_at = rings(&placed)[0].center;
        let moved = vec![egui::Event::PointerMoved(hover_at)];
        let _ = render(&mut pane, moved.clone());
        let hovered = render(&mut pane, moved);
        let [highest, middle, lowest] = rings(&hovered);
        assert_eq!(
            highest.stroke.color,
            theme::SUCCESS,
            "hover must not replace a lit ring"
        );
        assert_eq!(middle.stroke.color, theme::TEXT_PRIMARY);
        assert_eq!(lowest.stroke.color, theme::TEXT_PRIMARY);
        assert_stem_and_curve(&hovered, theme::TEXT_PRIMARY);
        let filled = ring_fill(&hovered, highest.center).expect("hovered colored ring is filled");
        assert_eq!(filled.fill, theme::SUCCESS);
        assert!((filled.radius - 1.25).abs() < 0.05);
        assert!(
            ring_fill(&hovered, middle.center).is_none(),
            "an original-color ring stays hollow on hover"
        );
        assert!(ring_fill(&hovered, lowest.center).is_none());
        let fills = header_circles(&hovered)
            .into_iter()
            .filter(|circle| {
                (circle.radius - 1.25).abs() < 0.05 && circle.fill != Color32::TRANSPARENT
            })
            .count();
        assert_eq!(
            fills, 1,
            "hover fills only the colored ring, not the stem or curve"
        );
    }

    #[test]
    fn known_worktree_shows_both_header_icons_and_a_non_repo_shows_neither() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("notes.txt"), "one\n").unwrap();
        let mut pane = ExplorerPane::new(&root, RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);

        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        pane.set_ui_scope(egui::Id::unique("two-header-icons"));
        let mut frame = 0.0;
        let mut render = |pane: &mut ExplorerPane, events: Vec<egui::Event>| {
            frame += 0.05;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 620.0),
                )),
                time: Some(frame),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                pane.tree_ui(ui, &mut log, &theme::Layout::for_window(900.0, 620.0));
            });
            output.textures_delta.clear();
            output
        };
        let hover_at =
            |render: &mut dyn FnMut(&mut ExplorerPane, Vec<egui::Event>) -> egui::FullOutput,
             pane: &mut ExplorerPane,
             at: egui::Pos2| {
                let moved = vec![egui::Event::PointerMoved(at)];
                let _ = render(pane, moved.clone());
                let _ = render(pane, moved);
                render(pane, Vec::new())
            };
        let click_at =
            |render: &mut dyn FnMut(&mut ExplorerPane, Vec<egui::Event>) -> egui::FullOutput,
             pane: &mut ExplorerPane,
             at: egui::Pos2| {
                let _ = render(pane, vec![egui::Event::PointerMoved(at)]);
                let _ = render(
                    pane,
                    vec![egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                let _ = render(
                    pane,
                    vec![egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            };
        let has_label = |output: &egui::FullOutput, label: &str| {
            text_positions(output).iter().any(|(text, _)| text == label)
        };
        let header_selected_center = |output: &egui::FullOutput| {
            fn visit(shape: &egui::Shape, found: &mut Option<egui::Pos2>) {
                match shape {
                    egui::Shape::Rect(rect)
                        if rect.fill == theme::BG_SELECTED
                            && (rect.rect.width() - 24.0).abs() < 1.0
                            && rect.rect.center().y < 40.0 =>
                    {
                        *found = Some(rect.rect.center());
                    }
                    egui::Shape::Vec(shapes) => {
                        for shape in shapes {
                            visit(shape, found);
                        }
                    }
                    _ => {}
                }
            }
            let mut found = None;
            for clipped in &output.shapes {
                visit(&clipped.shape, &mut found);
            }
            found.expect("folders selected fill")
        };
        let selected_at = |output: &egui::FullOutput, at: egui::Pos2| {
            fn visit(shape: &egui::Shape, at: egui::Pos2, found: &mut bool) {
                match shape {
                    egui::Shape::Rect(rect)
                        if rect.fill == theme::BG_SELECTED
                            && rect.rect.width() < 28.0
                            && rect.rect.center().distance(at) < 8.0 =>
                    {
                        *found = true;
                    }
                    egui::Shape::Vec(shapes) => {
                        for shape in shapes {
                            visit(shape, at, found);
                        }
                    }
                    _ => {}
                }
            }
            let mut found = false;
            for clipped in &output.shapes {
                visit(&clipped.shape, at, &mut found);
            }
            found
        };
        let status_near = |output: &egui::FullOutput, at: egui::Pos2| {
            header_circles(output).into_iter().any(|circle| {
                circle.center.distance(at) < 16.0
                    && (circle.fill == theme::SUCCESS
                        || circle.fill == theme::WARNING
                        || circle.fill == theme::ERROR
                        || circle.stroke.color == theme::SUCCESS
                        || circle.stroke.color == theme::WARNING
                        || circle.stroke.color == theme::ERROR)
            })
        };

        let plain = render(
            &mut pane,
            vec![egui::Event::PointerMoved(egui::pos2(1.0, 600.0))],
        );
        let plain_text = output_text(&plain);
        assert!(plain_text.contains("EXPLORER"), "{plain_text}");
        assert!(plain_text.contains("notes.txt"), "{plain_text}");
        assert!(!plain_text.contains("Source Control"), "{plain_text}");
        assert!(
            !has_label(&plain, "Explorer"),
            "a non-repo must not show the folders tooltip"
        );
        assert!(
            header_circles(&plain)
                .iter()
                .all(|circle| (circle.radius - 1.25).abs() > 0.05),
            "a non-repo has no source-control rings"
        );

        pane.apply_git_chrome(ExplorerGitChrome {
            header: GitHeader {
                show_icon: true,
                branch: Some("main".to_owned()),
                ..GitHeader::default()
            },
            status: Some(GitStatusSnapshot {
                groups: vec![crate::worktree::GitStatusGroup {
                    title: crate::worktree::GROUP_CHANGES,
                    rows: vec![GitStatusRow {
                        letter: 'M',
                        kind: ChangeKind::Modified,
                        path: root.join("notes.txt"),
                        relative: PathBuf::from("notes.txt"),
                        is_directory: false,
                        original_relative: None,
                    }],
                }],
                ..Default::default()
            }),
            failed: true,
            ..ExplorerGitChrome::default()
        });
        let explorer = render(
            &mut pane,
            vec![egui::Event::PointerMoved(egui::pos2(1.0, 600.0))],
        );
        assert!(output_text(&explorer).contains("notes.txt"));
        assert!(!pane.source_control_open());
        let rings = header_circles(&explorer)
            .into_iter()
            .filter(|circle| (circle.radius - 1.25).abs() < 0.05 && circle.center.x > 800.0)
            .collect::<Vec<_>>();
        assert_eq!(rings.len(), 3, "source-control icon keeps its three rings");
        let stem_x = rings
            .iter()
            .map(|circle| circle.center.x)
            .fold(f32::MAX, f32::min);
        let mut stem_y = rings
            .iter()
            .filter(|circle| (circle.center.x - stem_x).abs() < 0.5)
            .map(|circle| circle.center.y);
        let git_center = egui::pos2(
            stem_x + 3.25,
            (stem_y.next().unwrap() + stem_y.next().unwrap()) * 0.5,
        );
        let folders_center = header_selected_center(&explorer);
        assert!(
            (git_center.x - folders_center.x - 28.0).abs() < 0.5,
            "icon centers must be 28px apart, not 24: {:?} vs {:?}",
            folders_center,
            git_center
        );
        assert!(selected_at(&explorer, folders_center));
        assert!(!selected_at(&explorer, git_center));
        assert!(
            !status_near(&explorer, folders_center),
            "the folders icon is not recolored"
        );
        assert!(
            rings
                .iter()
                .any(|circle| circle.stroke.color == theme::WARNING),
            "ring color stays on the source-control icon"
        );

        let explorer_tip = hover_at(&mut render, &mut pane, folders_center);
        let git_tip = hover_at(&mut render, &mut pane, git_center);
        assert!(
            has_label(&explorer_tip, "Explorer"),
            "folders tooltip: {}",
            output_text(&explorer_tip)
        );
        assert!(
            has_label(&git_tip, "Source Control - Changes (1)"),
            "source-control tooltip: {}",
            output_text(&git_tip)
        );

        click_at(&mut render, &mut pane, folders_center);
        let still = render(&mut pane, Vec::new());
        assert!(output_text(&still).contains("EXPLORER"));
        assert!(output_text(&still).contains("notes.txt"));
        assert!(!pane.source_control_open());

        click_at(&mut render, &mut pane, git_center);
        let git = render(
            &mut pane,
            vec![egui::Event::PointerMoved(egui::pos2(1.0, 600.0))],
        );
        let git_text = output_text(&git);
        assert!(git_text.contains("REPOS"), "{git_text}");
        assert!(!git_text.contains("EXPLORER"), "{git_text}");
        assert!(pane.source_control_open());
        assert!(selected_at(&git, git_center));
        assert!(!selected_at(&git, folders_center));
        let git_tip = hover_at(&mut render, &mut pane, git_center);
        let explorer_tip = hover_at(&mut render, &mut pane, folders_center);
        assert!(has_label(&git_tip, "Source Control - Changes (1)"));
        assert!(
            has_label(&explorer_tip, "Explorer"),
            "names do not swap when the git view is open"
        );
        assert!(
            header_circles(&git)
                .iter()
                .any(|circle| circle.stroke.color == theme::WARNING),
            "rings stay colored in the git view"
        );
        assert!(!status_near(&git, folders_center));

        click_at(&mut render, &mut pane, git_center);
        assert!(
            pane.source_control_open(),
            "the active icon does not switch"
        );
        click_at(&mut render, &mut pane, folders_center);
        let back = render(&mut pane, Vec::new());
        assert!(!pane.source_control_open());
        assert!(output_text(&back).contains("EXPLORER"));
        assert!(output_text(&back).contains("notes.txt"));
        assert!(!output_text(&back).contains("REPOS"));

        pane.apply_git_chrome(ExplorerGitChrome {
            failed: true,
            ..ExplorerGitChrome::default()
        });
        let left = render(&mut pane, Vec::new());
        let left_text = output_text(&left);
        assert!(left_text.contains("EXPLORER"), "{left_text}");
        assert!(left_text.contains("notes.txt"), "{left_text}");
        assert!(!left_text.contains("REPOS"));
        assert!(!pane.source_control_open());
        assert!(
            header_circles(&left)
                .iter()
                .all(|circle| (circle.radius - 1.25).abs() > 0.05),
            "leaving a worktree removes both icons"
        );
    }

    #[test]
    fn repos_sections_and_source_control_tooltip_show_row_counts() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("notes.txt"), "one\n").unwrap();
        let mut pane = ExplorerPane::new(&root, RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);

        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        pane.set_ui_scope(egui::Id::unique("repos-counts"));
        let mut frame = 0.0;
        let mut render = |pane: &mut ExplorerPane, events: Vec<egui::Event>| {
            frame += 0.05;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 620.0),
                )),
                time: Some(frame),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                pane.tree_ui(ui, &mut log, &theme::Layout::for_window(900.0, 620.0));
            });
            output.textures_delta.clear();
            output
        };
        let row = |name: &str, directory: bool| GitStatusRow {
            letter: 'M',
            kind: ChangeKind::Modified,
            path: root.join(name),
            relative: PathBuf::from(name),
            is_directory: directory,
            original_relative: None,
        };
        pane.apply_git_chrome(ExplorerGitChrome {
            header: GitHeader {
                show_icon: true,
                branch: Some("main".to_owned()),
                ..GitHeader::default()
            },
            status: Some(GitStatusSnapshot {
                groups: vec![
                    crate::worktree::GitStatusGroup {
                        title: crate::worktree::GROUP_CHANGES,
                        rows: vec![row("notes.txt", false), row("vendor", true)],
                    },
                    crate::worktree::GitStatusGroup {
                        title: crate::worktree::GROUP_STAGED,
                        rows: Vec::new(),
                    },
                ],
                ..Default::default()
            }),
            ..ExplorerGitChrome::default()
        });
        pane.source_control_open = true;
        let git = render(
            &mut pane,
            vec![egui::Event::PointerMoved(egui::pos2(1.0, 600.0))],
        );
        let git_text = output_text(&git);
        assert!(
            git_text.contains("Changes (2)"),
            "section count includes a directory row: {git_text}"
        );
        assert!(
            !git_text.contains("Staged Changes") && !git_text.contains("(0)"),
            "an empty section is omitted: {git_text}"
        );
        let rings = header_circles(&git)
            .into_iter()
            .filter(|circle| {
                (circle.radius - 1.25).abs() < 0.05
                    && circle.stroke.width > 1.0
                    && circle.center.x > 800.0
            })
            .collect::<Vec<_>>();
        assert_eq!(rings.len(), 3);
        let stem_x = rings
            .iter()
            .map(|circle| circle.center.x)
            .fold(f32::MAX, f32::min);
        let mut stem_y = rings
            .iter()
            .filter(|circle| (circle.center.x - stem_x).abs() < 0.5)
            .map(|circle| circle.center.y);
        let git_center = egui::pos2(
            stem_x + 3.25,
            (stem_y.next().unwrap() + stem_y.next().unwrap()) * 0.5,
        );
        let moved = vec![egui::Event::PointerMoved(git_center)];
        let _ = render(&mut pane, moved.clone());
        let _ = render(&mut pane, moved);
        let tip = render(&mut pane, Vec::new());
        assert!(
            text_positions(&tip)
                .iter()
                .any(|(text, _)| text == "Source Control - Changes (2)"),
            "one-section tooltip: {}",
            output_text(&tip)
        );
        pane.apply_git_chrome(ExplorerGitChrome {
            header: GitHeader {
                show_icon: true,
                branch: Some("main".to_owned()),
                ..GitHeader::default()
            },
            status: Some(GitStatusSnapshot {
                groups: vec![
                    crate::worktree::GitStatusGroup {
                        title: crate::worktree::GROUP_CHANGES,
                        rows: vec![row("notes.txt", false), row("vendor", true)],
                    },
                    crate::worktree::GitStatusGroup {
                        title: crate::worktree::GROUP_UNTRACKED,
                        rows: vec![row("extra.txt", false)],
                    },
                ],
                ..Default::default()
            }),
            ..ExplorerGitChrome::default()
        });
        pane.source_control_open = true;
        let both = render(
            &mut pane,
            vec![egui::Event::PointerMoved(egui::pos2(1.0, 600.0))],
        );
        let both_text = output_text(&both);
        assert!(both_text.contains("Changes (2)"), "{both_text}");
        assert!(both_text.contains("Untracked (1)"), "{both_text}");
        let moved = vec![egui::Event::PointerMoved(git_center)];
        let _ = render(&mut pane, moved.clone());
        let _ = render(&mut pane, moved);
        let both_tip = render(&mut pane, Vec::new());
        assert!(
            text_positions(&both_tip)
                .iter()
                .any(|(text, _)| { text == "Source Control - Changes (2) · Untracked (1)" }),
            "two-section tooltip: {}",
            output_text(&both_tip)
        );
        let folders_center = git_center - egui::vec2(28.0, 0.0);
        let folders_moved = vec![egui::Event::PointerMoved(folders_center)];
        let _ = render(&mut pane, folders_moved.clone());
        let _ = render(&mut pane, folders_moved);
        let folders_tip = render(&mut pane, Vec::new());
        assert!(
            text_positions(&folders_tip)
                .iter()
                .any(|(text, _)| text == "Explorer"),
            "folders name stays Explorer: {}",
            output_text(&folders_tip)
        );

        pane.apply_git_chrome(ExplorerGitChrome {
            header: GitHeader {
                show_icon: true,
                branch: Some("main".to_owned()),
                ..GitHeader::default()
            },
            status: Some(GitStatusSnapshot::default()),
            ..ExplorerGitChrome::default()
        });
        pane.source_control_open = true;
        let clean = render(
            &mut pane,
            vec![egui::Event::PointerMoved(egui::pos2(1.0, 600.0))],
        );
        let clean_text = output_text(&clean);
        assert!(clean_text.contains("No changes"), "{clean_text}");
        assert!(!clean_text.contains("(0)"), "{clean_text}");
        let moved = vec![egui::Event::PointerMoved(git_center)];
        let _ = render(&mut pane, moved.clone());
        let _ = render(&mut pane, moved);
        let clean_tip = render(&mut pane, Vec::new());
        assert!(
            text_positions(&clean_tip)
                .iter()
                .any(|(text, _)| text == "Source Control"),
            "a clean tree tooltip is only the name: {}",
            output_text(&clean_tip)
        );
        assert!(
            !text_positions(&clean_tip)
                .iter()
                .any(|(text, _)| text.starts_with("Source Control -")),
            "a clean tree tooltip has no count: {}",
            output_text(&clean_tip)
        );
    }

    #[test]
    fn new_file_rows_show_a_success_dot_including_inside_an_untracked_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let run_git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .env("GIT_AUTHOR_NAME", "Made Test")
                .env("GIT_AUTHOR_EMAIL", "made@example.invalid")
                .env("GIT_COMMITTER_NAME", "Made Test")
                .env("GIT_COMMITTER_EMAIL", "made@example.invalid")
                .output()
                .expect("git");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run_git(&["init", "-q", "-b", "main"]);
        fs::create_dir(root.join("tracked")).unwrap();
        fs::create_dir(root.join("edited")).unwrap();
        fs::write(root.join("tracked").join("keep.txt"), "keep\n").unwrap();
        fs::write(root.join("edited").join("keep.txt"), "keep\n").unwrap();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        run_git(&["add", "tracked/keep.txt", "edited/keep.txt", ".gitignore"]);
        run_git(&["commit", "-q", "-m", "base"]);
        fs::write(root.join("tracked").join("fresh.txt"), "new\n").unwrap();
        fs::create_dir(root.join("newdir")).unwrap();
        fs::write(root.join("newdir").join("inside.txt"), "nested\n").unwrap();
        fs::create_dir(root.join("ignored")).unwrap();
        fs::write(root.join("ignored").join("skip.log"), "secret\n").unwrap();
        fs::write(root.join("edited").join("keep.txt"), "changed\n").unwrap();

        let mut pane = ExplorerPane::new(root, RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);
        let canonical = pane.root().expect("root").to_path_buf();
        for name in ["tracked", "edited", "newdir"] {
            pane.toggle_dir(canonical.join(name));
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && {
            let _ = pane.visible_row_count();
            !pane.rows.iter().any(|row| row.name == "inside.txt")
        } {
            pane.tick(&mut log);
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = pane.visible_row_count();
        assert!(
            pane.rows.iter().any(|row| row.name == "inside.txt"),
            "untracked file must already be a tree row: {:?}",
            pane.rows
                .iter()
                .map(|row| row.name.as_str())
                .collect::<Vec<_>>()
        );
        let snapshot = match crate::worktree::GitRunner::with_program("git")
            .read_status(&canonical, Some(canonical.as_path()))
        {
            crate::worktree::GitStatusRead::Ready(snapshot) => snapshot,
            other => panic!("status should be ready: {other:?}"),
        };
        let marks = file_status_marks(&snapshot);
        let fresh = marks
            .get(&canonical.join("tracked").join("fresh.txt"))
            .expect("new file in a tracked directory");
        assert_eq!(fresh.tooltip(), "New");
        assert_eq!(folder_mark_color(fresh), theme::SUCCESS);
        let inside = marks
            .get(&canonical.join("newdir").join("inside.txt"))
            .expect("file inside an untracked directory");
        assert_eq!(inside.tooltip(), "New");
        assert_eq!(folder_mark_color(inside), theme::SUCCESS);
        let modified = marks
            .get(&canonical.join("edited").join("keep.txt"))
            .expect("modified file");
        assert_eq!(modified.tooltip(), "Modified");
        assert_eq!(folder_mark_color(modified), theme::WARNING);
        assert!(!marks.contains_key(&canonical.join("ignored").join("skip.log")));
        assert_eq!(
            snapshot
                .folders
                .get(&canonical.join("tracked"))
                .expect("tracked parent")
                .tooltip(),
            "New"
        );
        assert_eq!(
            snapshot
                .folders
                .get(&canonical.join("newdir"))
                .expect("untracked parent")
                .tooltip(),
            "New"
        );
        assert_eq!(
            snapshot
                .folders
                .get(&canonical.join("edited"))
                .expect("modified parent")
                .tooltip(),
            "Modified"
        );
        assert!(!snapshot.folders.contains_key(&canonical.join("ignored")));
        pane.apply_git_chrome(ExplorerGitChrome {
            status: Some(snapshot),
            ..ExplorerGitChrome::default()
        });
        assert!(pane.rows.iter().all(|row| row.name != "gone.txt"));

        let ctx = egui::Context::default();
        theme::install(&ctx);
        pane.set_ui_scope(egui::Id::unique("new-file-dot"));
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 620.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            pane.tree_ui(ui, &mut log, &theme::Layout::for_window(900.0, 620.0));
        });
        output.textures_delta.clear();
        let text = output_text(&output);
        assert!(text.contains("fresh.txt"));
        assert!(text.contains("inside.txt"));
        assert!(text.contains("keep.txt"));
        assert!(!text.contains("skip.log"));
        assert!(!text.contains("gone.txt"));
        for letter in ["M", "A", "U", "D", "R", "C"] {
            assert!(
                !text_positions(&output)
                    .iter()
                    .any(|(painted, _)| painted == letter),
                "file tree painted status letter {letter}"
            );
        }
        let dots = status_dots(&output);
        let positions = text_positions(&output);
        assert_eq!(
            dot_for_label(&positions, &dots, "fresh.txt").fill,
            theme::SUCCESS
        );
        assert_eq!(
            dot_for_label(&positions, &dots, "inside.txt").fill,
            theme::SUCCESS
        );
        assert_eq!(
            dot_for_label(&positions, &dots, "newdir").fill,
            theme::SUCCESS
        );
        assert_eq!(
            dot_for_label(&positions, &dots, "keep.txt").fill,
            theme::WARNING
        );
        let ignored = positions
            .iter()
            .find(|(label, _)| label.contains("ignored"))
            .expect("ignored directory is still a tree row")
            .1;
        assert!(
            dots.iter()
                .all(|dot| (dot.center.y - ignored.y).abs() > 12.0),
            "an ignored-only directory must not show a status dot"
        );
    }

    #[test]
    fn git_view_row_shows_an_icon_and_a_dot_whose_tooltip_sits_on_the_dot() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("notes.txt"), "one\n").unwrap();
        fs::create_dir(root.join("vendor")).unwrap();
        let mut pane = ExplorerPane::new(&root, RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);
        let canonical = pane.root().expect("root").to_path_buf();
        let notes = canonical.join("notes.txt");
        let vendor = canonical.join("vendor");
        pane.apply_git_chrome(ExplorerGitChrome {
            header: GitHeader {
                show_icon: true,
                ..GitHeader::default()
            },
            status: Some(GitStatusSnapshot {
                groups: vec![
                    crate::worktree::GitStatusGroup {
                        title: crate::worktree::GROUP_CHANGES,
                        rows: vec![GitStatusRow {
                            letter: 'M',
                            kind: ChangeKind::Modified,
                            path: notes.clone(),
                            relative: PathBuf::from("notes.txt"),
                            is_directory: false,
                            original_relative: None,
                        }],
                    },
                    crate::worktree::GitStatusGroup {
                        title: crate::worktree::GROUP_UNTRACKED,
                        rows: vec![GitStatusRow {
                            letter: 'U',
                            kind: ChangeKind::New,
                            path: vendor.clone(),
                            relative: PathBuf::from("vendor"),
                            is_directory: true,
                            original_relative: None,
                        }],
                    },
                ],
                ..Default::default()
            }),
            ..ExplorerGitChrome::default()
        });
        pane.source_control_open = true;

        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        pane.set_ui_scope(egui::Id::unique("git-row-icon-dot"));
        let mut render = |pane: &mut ExplorerPane, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 620.0),
                )),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                pane.tree_ui(ui, &mut log, &theme::Layout::for_window(900.0, 620.0));
            });
            output.textures_delta.clear();
            output
        };
        let click =
            |render: &mut dyn FnMut(&mut ExplorerPane, Vec<egui::Event>) -> egui::FullOutput,
             pane: &mut ExplorerPane,
             at: egui::Pos2| {
                let _ = render(pane, vec![egui::Event::PointerMoved(at)]);
                let _ = render(
                    pane,
                    vec![egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                let _ = render(
                    pane,
                    vec![egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            };

        let git = render(&mut pane, Vec::new());
        let positions = text_positions(&git);
        for letter in ["M", "A", "U", "D", "R", "C"] {
            assert!(
                !positions.iter().any(|(painted, _)| painted == letter),
                "git view painted status letter {letter}"
            );
        }
        let notes_text = positions
            .iter()
            .find(|(label, _)| label.contains("notes.txt"))
            .expect("notes.txt")
            .1;
        let vendor_text = positions
            .iter()
            .find(|(label, _)| label.contains("vendor"))
            .expect("vendor")
            .1;
        assert!(
            (notes_text.y - vendor_text.y).abs() > 20.0,
            "rows stay on separate 24px lines"
        );
        let icons = textured_rects(&git);
        assert!(
            icons.iter().any(|rect| {
                rect.center().x < notes_text.x
                    && notes_text.x - rect.center().x < 24.0
                    && (rect.center().y - notes_text.y).abs() < 12.0
            }),
            "file icon must sit immediately before notes.txt"
        );
        assert!(
            icons.iter().any(|rect| {
                rect.center().x < vendor_text.x
                    && vendor_text.x - rect.center().x < 24.0
                    && (rect.center().y - vendor_text.y).abs() < 12.0
            }),
            "folder icon must sit immediately before vendor"
        );
        let dots = status_dots(&git);
        let notes_dot = dot_for_label(&positions, &dots, "notes.txt");
        let vendor_dot = dot_for_label(&positions, &dots, "vendor");
        assert_eq!(notes_dot.fill, theme::WARNING);
        assert_eq!(vendor_dot.fill, theme::SUCCESS);
        assert!(notes_dot.center.x > notes_text.x + 40.0);
        assert!(notes_dot.center.x > 800.0, "dot stays on the right");

        let hover_at =
            |render: &mut dyn FnMut(&mut ExplorerPane, Vec<egui::Event>) -> egui::FullOutput,
             pane: &mut ExplorerPane,
             at: egui::Pos2| {
                let moved = vec![egui::Event::PointerMoved(at)];
                let _ = render(pane, moved.clone());
                let _ = render(pane, moved);
                render(pane, Vec::new())
            };
        // Outside the old 12px square (center ± 6) but inside the 28×24 right-edge strip.
        let near_dot = notes_dot.center + egui::vec2(-10.0, -10.0);
        let dot_hover = hover_at(&mut render, &mut pane, near_dot);
        let modified = text_positions(&dot_hover)
            .into_iter()
            .find(|(label, _)| label == "Modified")
            .expect("kind tooltip");
        assert!(
            (modified.1.x - notes_dot.center.x).abs() < 80.0,
            "Modified tooltip must sit on the dot, not the far side of the row: {:?} vs {:?}",
            modified.1,
            notes_dot.center
        );
        assert!(
            !output_text(&dot_hover).contains(&notes.display().to_string()),
            "kind-strip hover must not also show the path"
        );
        let path_hover = hover_at(&mut render, &mut pane, notes_text);
        let path_text = output_text(&path_hover);
        assert!(
            path_text.contains("notes.txt"),
            "row hover keeps the path: {path_text}"
        );
        assert!(
            !text_positions(&path_hover)
                .iter()
                .any(|(label, _)| label == "Modified"),
            "path hover must not show the kind tooltip"
        );

        click(&mut render, &mut pane, notes_text);
        assert!(pane.take_preview_opened(), "clicking a file opens preview");
        click(&mut render, &mut pane, vendor_text);
        assert!(
            !pane.take_preview_opened(),
            "clicking a directory does not open preview"
        );

        pane.source_control_open = false;
        let tree_ctx = egui::Context::default();
        theme::install(&tree_ctx);
        tree_ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        pane.set_ui_scope(egui::Id::unique("file-tree-dot-tooltip"));
        let mut render_tree = |pane: &mut ExplorerPane, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 620.0),
                )),
                events,
                ..Default::default()
            };
            let mut output = tree_ctx.run_ui(raw, |ui| {
                pane.tree_ui(ui, &mut log, &theme::Layout::for_window(900.0, 620.0));
            });
            output.textures_delta.clear();
            output
        };
        let tree = render_tree(&mut pane, Vec::new());
        let tree_positions = text_positions(&tree);
        let tree_dot = dot_for_label(&tree_positions, &status_dots(&tree), "notes.txt");
        let tree_near_dot = tree_dot.center + egui::vec2(-10.0, -10.0);
        let tree_hover = hover_at(&mut render_tree, &mut pane, tree_near_dot);
        let tree_modified = text_positions(&tree_hover)
            .into_iter()
            .find(|(label, _)| label == "Modified")
            .unwrap_or_else(|| {
                panic!(
                    "file-tree kind tooltip at {:?}: {}",
                    tree_dot.center,
                    output_text(&tree_hover)
                )
            });
        assert!(
            (tree_modified.1.x - tree_dot.center.x).abs() < 80.0,
            "file-tree tooltip must sit on the dot: {:?} vs {:?}",
            tree_modified.1,
            tree_dot.center
        );
        assert!(
            !output_text(&tree_hover).contains(&notes.display().to_string()),
            "file-tree kind-strip hover must not also show the path"
        );
        let tree_name = tree_positions
            .iter()
            .find(|(label, _)| label.contains("notes.txt"))
            .expect("tree notes")
            .1;
        let tree_path = hover_at(&mut render_tree, &mut pane, tree_name);
        assert!(output_text(&tree_path).contains(&notes.display().to_string()));
        assert!(
            !text_positions(&tree_path)
                .iter()
                .any(|(label, _)| label == "Modified"),
            "hovering the file name shows the path, not the kind"
        );
    }

    fn textured_rects(output: &egui::FullOutput) -> Vec<egui::Rect> {
        fn visit(shape: &egui::Shape, rects: &mut Vec<egui::Rect>) {
            match shape {
                egui::Shape::Rect(rect) if rect.fill_texture_id() != egui::TextureId::default() => {
                    rects.push(rect.rect);
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        visit(shape, rects);
                    }
                }
                _ => {}
            }
        }
        let mut rects = Vec::new();
        for clipped in &output.shapes {
            visit(&clipped.shape, &mut rects);
        }
        rects
    }

    #[test]
    fn changed_file_row_shows_one_dot_and_an_unchanged_file_does_not() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("changed.txt"), "edited\n").unwrap();
        fs::write(temp.path().join("clean.txt"), "same\n").unwrap();
        fs::write(temp.path().join("both.txt"), "partial\n").unwrap();
        let mut pane = ExplorerPane::new(temp.path(), RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);
        let root = pane.root().expect("root").to_path_buf();
        let snapshot = crate::worktree::group_status(
            &crate::worktree::parse_porcelain_v1_z(" M changed.txt\0AM both.txt\0D  gone.txt\0"),
            &root,
            Some(root.as_path()),
        );
        let marks = file_status_marks(&snapshot);
        let changed = marks
            .get(&root.join("changed.txt"))
            .expect("modified file mark");
        assert_eq!(changed.tooltip(), "Modified");
        assert_eq!(folder_mark_color(changed), theme::WARNING);
        let mixed = marks.get(&root.join("both.txt")).expect("mixed file mark");
        assert_eq!(mixed.tooltip(), "New, Modified");
        assert_eq!(folder_mark_color(mixed), theme::ACCENT);
        assert!(!marks.contains_key(&root.join("clean.txt")));
        assert!(
            marks.contains_key(&root.join("gone.txt")),
            "a deleted path can be marked, but it must not become a row"
        );
        pane.apply_git_chrome(ExplorerGitChrome {
            status: Some(snapshot),
            ..ExplorerGitChrome::default()
        });
        assert_eq!(pane.visible_row_count(), 3);
        assert!(pane.rows.iter().all(|row| row.name != "gone.txt"));

        let ctx = egui::Context::default();
        theme::install(&ctx);
        pane.set_ui_scope(egui::Id::unique("file-dot"));
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 620.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            pane.tree_ui(ui, &mut log, &theme::Layout::for_window(900.0, 620.0));
        });
        output.textures_delta.clear();
        let text = output_text(&output);
        assert!(text.contains("changed.txt"));
        assert!(text.contains("clean.txt"));
        assert!(text.contains("both.txt"));
        assert!(!text.contains("gone.txt"));
        for letter in ["M", "A", "U", "D", "R", "C"] {
            assert!(
                !text_positions(&output)
                    .iter()
                    .any(|(painted, _)| painted == letter),
                "file tree painted status letter {letter}"
            );
        }
        let dots = status_dots(&output);
        let positions = text_positions(&output);
        let changed_dot = dot_for_label(&positions, &dots, "changed.txt");
        let mixed_dot = dot_for_label(&positions, &dots, "both.txt");
        assert_eq!(changed_dot.fill, theme::WARNING);
        assert_eq!(mixed_dot.fill, theme::ACCENT);
        assert!((changed_dot.radius - 3.0).abs() < 0.01);
        assert!((changed_dot.center.x - mixed_dot.center.x).abs() < 0.01);
        let clean = positions
            .iter()
            .find(|(label, _)| label.contains("clean.txt"))
            .expect("clean file")
            .1;
        assert!(
            dots.iter().all(|dot| (dot.center.y - clean.y).abs() > 12.0),
            "unchanged file must not show a status dot"
        );
    }

    #[test]
    fn folder_dot_and_letter_colors_use_existing_tokens() {
        assert_eq!(change_kind_color(ChangeKind::New), theme::SUCCESS);
        assert_eq!(change_kind_color(ChangeKind::Modified), theme::WARNING);
        assert_eq!(change_kind_color(ChangeKind::Deleted), theme::ERROR);
        assert_eq!(change_kind_color(ChangeKind::Conflict), theme::ERROR);
        assert_eq!(change_kind_color(ChangeKind::Renamed), theme::INFO);
        let mixed = FolderMark {
            kinds: vec![ChangeKind::New, ChangeKind::Modified],
        };
        assert_eq!(folder_mark_color(&mixed), theme::ACCENT);
        let single = FolderMark {
            kinds: vec![ChangeKind::Deleted],
        };
        assert_eq!(folder_mark_color(&single), theme::ERROR);
    }

    #[test]
    fn opening_a_missing_file_reaches_the_missing_preview() {
        let temp = tempfile::tempdir().unwrap();
        let mut pane = ExplorerPane::new(temp.path(), RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && pane.root().is_none() {
            pane.tick(&mut log);
            std::thread::sleep(Duration::from_millis(5));
        }
        let root = pane.root().expect("root").to_path_buf();
        let missing = root.join("gone.txt");
        pane.open_preview(missing);
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && pane.preview_document().is_none() {
            pane.tick(&mut log);
            std::thread::sleep(Duration::from_millis(5));
        }
        let document = pane.preview_document().expect("missing preview");
        assert_eq!(document.kind, PreviewKind::Missing);
        assert!(document.lines.is_empty());
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

    fn status_dots(output: &egui::FullOutput) -> Vec<egui::epaint::CircleShape> {
        fn visit(shape: &egui::Shape, dots: &mut Vec<egui::epaint::CircleShape>) {
            match shape {
                egui::Shape::Circle(circle) if (circle.radius - 3.0).abs() < 0.1 => {
                    dots.push(*circle);
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        visit(shape, dots);
                    }
                }
                _ => {}
            }
        }
        let mut dots = Vec::new();
        for clipped in &output.shapes {
            visit(&clipped.shape, &mut dots);
        }
        dots
    }

    fn dot_for_label(
        positions: &[(String, egui::Pos2)],
        dots: &[egui::epaint::CircleShape],
        label: &str,
    ) -> egui::epaint::CircleShape {
        let text = positions
            .iter()
            .find(|(painted, _)| painted.contains(label))
            .unwrap_or_else(|| panic!("missing row {label}"))
            .1;
        *dots
            .iter()
            .find(|dot| (dot.center.y - text.y).abs() < 16.0)
            .unwrap_or_else(|| panic!("{label} has no status dot; dots={dots:?} text={text:?}"))
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
            // Keyboard behavior is the subject here; a live watcher must not
            // reorder or reload the fixture rows between key frames.
            pane.watcher_installer = WatcherInstaller::blocked();
            pane.watcher_loading = false;
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

    #[test]
    fn virtualized_tree_rows_fill_the_explorer_body() {
        let temp = tempfile::tempdir().unwrap();
        for index in 0..60 {
            fs::write(temp.path().join(format!("file-{index:03}.txt")), "x").unwrap();
        }
        let mut pane = ExplorerPane::new(temp.path(), RootGeneration::INITIAL);
        let mut log = DiagnosticLog::new();
        wait_for_root(&mut pane, &mut log);
        let total = pane.visible_row_count();
        assert!(
            total > 40,
            "need more rows than the body can show, got {total}"
        );

        let ctx = egui::Context::default();
        theme::install(&ctx);
        pane.set_ui_scope(egui::Id::unique("explorer-row-fill"));
        tree_key_frame(&ctx, &mut pane, &mut log, 1280.0, 800.0, &[]);

        let body = ctx
            .read_response(pane.tree_focus_id())
            .expect("tree container");
        let mut painted = 0usize;
        let mut lowest = f32::NEG_INFINITY;
        let mut row_height = None;
        for index in 0..total {
            if let Some(response) = ctx.read_response(pane.row_widget_id(index)) {
                painted += 1;
                lowest = lowest.max(response.rect.bottom());
                row_height = Some(response.rect.height());
            }
        }
        assert!(painted > 0, "no tree rows were painted");
        assert!(
            painted < total,
            "expected a virtual window, painted {painted} of {total}"
        );
        assert_eq!(row_height, Some(24.0), "row height must stay 24px");
        let gap = body.rect.bottom() - lowest;
        assert!(
            gap < 1.0,
            "virtual row window stops short of the explorer body by {gap}px \
             (lowest={lowest}, body_bottom={}, painted={painted}/{total})",
            body.rect.bottom()
        );
    }

    fn remote_projection() -> crate::remote_workspace::WorkspaceProjection {
        use crate::remote_workspace::{
            DirectoryProjection, ExplorerMode, Freshness, RootSource, WorkspaceProjection,
        };
        use made_remote::protocol::{DirectoryEntry, EntryKind as RemoteEntryKind, RemotePath};

        let root = RemotePath::new(b"/home/vscode".to_vec()).unwrap();
        let readme = RemotePath::new(b"/home/vscode/README.md".to_vec()).unwrap();
        let src = RemotePath::new(b"/home/vscode/src".to_vec()).unwrap();
        let main = RemotePath::new(b"/home/vscode/src/main.rs".to_vec()).unwrap();
        let mut directories = std::collections::HashMap::new();
        directories.insert(
            root.clone(),
            DirectoryProjection {
                path: root.clone(),
                entries: vec![
                    DirectoryEntry {
                        path: readme,
                        kind: RemoteEntryKind::File,
                    },
                    DirectoryEntry {
                        path: src.clone(),
                        kind: RemoteEntryKind::Directory,
                    },
                ],
                truncated: false,
                freshness: Freshness::Current,
            },
        );
        directories.insert(
            src.clone(),
            DirectoryProjection {
                path: src.clone(),
                entries: vec![DirectoryEntry {
                    path: main,
                    kind: RemoteEntryKind::File,
                }],
                truncated: false,
                freshness: Freshness::Current,
            },
        );
        WorkspaceProjection {
            source: RootSource::Launch,
            requested_root: None,
            root_state: Freshness::Current,
            directories,
            expanded: std::collections::HashSet::from([src]),
            selected: Some(RemotePath::new(b"/home/vscode/README.md".to_vec()).unwrap()),
            git_selected: None,
            git_root_known: false,
            preview: None,
            preview_revision: 0,
            revision: 0,
            git: Vec::new(),
            git_view: None,
            sessions: None,
            session_state: Freshness::Stale,
            integration_error: None,
            focus: None,
            grok: None,
            mode: ExplorerMode::Files,
            cleanup_unconfirmed: false,
        }
    }

    fn remote_render_ctx(
        projection: &crate::remote_workspace::WorkspaceProjection,
        root: &made_remote::protocol::RemotePath,
    ) -> (egui::Context, egui::FullOutput, Vec<RemoteExplorerAction>) {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let layout = theme::Layout::for_window(1280.0, 800.0);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut cache = RemoteExplorerCache::default();
        let mut output = ctx.run_ui(raw, |ui| {
            actions = remote_explorer_ui(
                ui,
                root,
                projection,
                &layout,
                RemoteExplorerOptions {
                    scope: egui::Id::unique("remote-explorer-test"),
                    native_backdrop: false,
                    allow_root_retry: true,
                },
                &mut cache,
            );
        });
        output.textures_delta.clear();
        (ctx, output, actions)
    }

    fn remote_render(
        projection: &crate::remote_workspace::WorkspaceProjection,
        root: &made_remote::protocol::RemotePath,
    ) -> (egui::FullOutput, Vec<RemoteExplorerAction>) {
        let (_ctx, output, actions) = remote_render_ctx(projection, root);
        (output, actions)
    }

    fn remote_render_with_retry(
        projection: &crate::remote_workspace::WorkspaceProjection,
        root: &made_remote::protocol::RemotePath,
        allow_retry: bool,
    ) -> egui::FullOutput {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let layout = theme::Layout::for_window(1280.0, 800.0);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let mut cache = RemoteExplorerCache::default();
        let mut output = ctx.run_ui(raw, |ui| {
            let _ = remote_explorer_ui(
                ui,
                root,
                projection,
                &layout,
                RemoteExplorerOptions {
                    scope: egui::Id::unique("remote-explorer-retry-test"),
                    native_backdrop: false,
                    allow_root_retry: allow_retry,
                },
                &mut cache,
            );
        });
        output.textures_delta.clear();
        output
    }

    #[test]
    fn disconnected_remote_error_hides_the_inert_retry_action() {
        use crate::remote_workspace::{ExplorerMode, Freshness, RootSource};
        use made_remote::protocol::{ErrorCode, RemotePath};

        let root = RemotePath::new(b"/home/vscode".to_vec()).unwrap();
        let mut projection = remote_projection();
        projection.source = RootSource::Launch;
        projection.requested_root = None;
        projection.root_state = Freshness::Unavailable(ErrorCode::Unavailable);
        projection.directories.clear();
        projection.expanded.clear();
        projection.selected = None;
        projection.git_selected = None;
        projection.git_root_known = false;
        projection.git.clear();
        projection.git_view = None;
        projection.mode = ExplorerMode::Files;

        let disconnected = output_text(&remote_render_with_retry(&projection, &root, false));
        assert!(
            disconnected.contains("Explorer unavailable"),
            "{disconnected:?}"
        );
        assert!(
            !disconnected.contains("Retry"),
            "a pre-attach failure must not offer an inert Retry: {disconnected:?}"
        );

        let connected = output_text(&remote_render_with_retry(&projection, &root, true));
        assert!(
            connected.contains("Retry"),
            "a connected root failure keeps its retry action: {connected:?}"
        );
    }

    #[test]
    fn remote_explorer_uses_local_explorer_chrome_without_ssh_status() {
        use crate::remote_workspace::{ExplorerMode, Freshness};
        use made_remote::protocol::RemotePath;

        let root = RemotePath::new(b"/home/vscode".to_vec()).unwrap();
        let mut projection = remote_projection();
        let (output, _) = remote_render(&projection, &root);
        let text = output_text(&output);
        for expected in ["EXPLORER", "vscode", "README.md", "src", "main.rs"] {
            assert!(text.contains(expected), "missing {expected:?} in {text:?}");
        }
        for forbidden in [
            "SSH connected",
            "SSH disconnected",
            "integration unknown",
            "integration connected",
            "Files",
            "Source Control",
            "Preview",
            "Remote listing",
        ] {
            assert!(
                !text.contains(forbidden),
                "remote explorer must not paint {forbidden:?}: {text:?}"
            );
        }

        projection.mode = ExplorerMode::Git;
        projection.git_view = Some(crate::remote_workspace::RemoteGitView {
            branch: Some("main".to_owned()),
            detached: false,
            commit: None,
            subject: None,
            linked_worktree: false,
            worktree_name: Some("PlantSuite".to_owned()),
            groups: vec![crate::remote_workspace::RemoteGitGroup {
                title: crate::worktree::GROUP_CHANGES,
                rows: vec![crate::remote_workspace::RemoteGitRow {
                    path: RemotePath::new(b"/home/vscode/README.md".to_vec()).unwrap(),
                    original: None,
                    letter: 'M',
                    kind: made_remote::git::ChangeKind::Modified,
                    directory: false,
                }],
            }],
            freshness: Freshness::Current,
            folder_marks: std::collections::HashMap::new(),
            has_status: true,
        });
        let (output, _) = remote_render(&projection, &root);
        let text = output_text(&output);
        for expected in ["REPOS", "main", "Changes (1)", "README.md"] {
            assert!(text.contains(expected), "missing {expected:?} in {text:?}");
        }
        for forbidden in [
            "EXPLORER",
            "SSH connected",
            "SSH disconnected",
            "integration unknown",
            "Files",
            "Source Control",
            "Preview",
            "Remote Git metadata",
        ] {
            assert!(
                !text.contains(forbidden),
                "remote source control must not paint {forbidden:?}: {text:?}"
            );
        }
    }

    #[test]
    fn remote_preview_uses_the_full_left_surface() {
        use made_remote::protocol::{PreviewKind as RemotePreviewKind, RemotePath};

        let path = RemotePath::new(b"/home/vscode/README.md".to_vec()).unwrap();
        let content = "# PlantSuite\n\nRemote preview.\n";
        let document = PreviewDocument::from_remote(
            &path,
            RemotePreviewKind::Text,
            content.as_bytes(),
            &Highlighter::new(),
        );
        assert_eq!(document.size, Some(content.len() as u64));
        assert_eq!(document.line_count(), 3);

        let ctx = egui::Context::default();
        theme::install(&ctx);
        let layout = theme::Layout::for_window(1280.0, 800.0);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            let _ = preview_card_ui(
                ui,
                PreviewCardState {
                    path_name: Some("README.md"),
                    path_full: Some("/home/vscode/README.md"),
                    document: Some(&document),
                    loading: false,
                    error: None,
                    refreshed_at: None,
                    focus_pending: false,
                },
                PreviewCardIds {
                    focus: egui::Id::unique("remote-preview-focus"),
                    close: egui::Id::unique("remote-preview-close"),
                    terminal: egui::Id::unique("remote-preview-terminal"),
                    refresh: egui::Id::unique("remote-preview-refresh"),
                    copy: egui::Id::unique("remote-preview-copy"),
                },
                &layout,
            );
        });
        output.textures_delta.clear();
        let text = output_text(&output);
        for expected in [
            "PREVIEW",
            "README.md",
            "read-only",
            "Copy path",
            "Refresh",
            "Close",
            "Terminal",
            "1",
            "PlantSuite",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in {text:?}");
        }
        assert!(
            !text.contains("SSH"),
            "preview must not paint SSH chrome: {text:?}"
        );
    }
    #[test]
    fn remote_git_failure_uses_the_local_unavailable_state_and_keeps_last_rows() {
        use crate::remote_workspace::{
            ExplorerMode, Freshness, RemoteGitGroup, RemoteGitRow, RemoteGitView,
        };
        use made_remote::protocol::RemotePath;

        let root = RemotePath::new(b"/home/vscode".to_vec()).unwrap();
        let mut projection = remote_projection();
        projection.mode = ExplorerMode::Git;
        projection.git_root_known = true;
        projection.git_view = Some(RemoteGitView {
            branch: Some("main".to_owned()),
            detached: false,
            commit: None,
            subject: None,
            linked_worktree: false,
            worktree_name: Some("PlantSuite".to_owned()),
            groups: Vec::new(),
            freshness: Freshness::Unavailable(made_remote::protocol::ErrorCode::Unavailable),
            folder_marks: std::collections::HashMap::new(),
            has_status: false,
        });
        let (output, _) = remote_render(&projection, &root);
        let text = output_text(&output);
        assert!(text.contains("REPOS"), "{text}");
        assert!(text.contains("main"), "{text}");
        assert!(text.contains("Git status unavailable"), "{text}");
        assert!(!text.contains("No changes"), "{text}");

        projection.git_view = Some(RemoteGitView {
            branch: Some("main".to_owned()),
            detached: false,
            commit: None,
            subject: None,
            linked_worktree: false,
            worktree_name: Some("PlantSuite".to_owned()),
            groups: vec![RemoteGitGroup {
                title: crate::worktree::GROUP_CHANGES,
                rows: vec![RemoteGitRow {
                    path: RemotePath::new(b"/home/vscode/README.md".to_vec()).unwrap(),
                    original: None,
                    letter: 'M',
                    kind: made_remote::git::ChangeKind::Modified,
                    directory: false,
                }],
            }],
            freshness: Freshness::Unavailable(made_remote::protocol::ErrorCode::Unavailable),
            folder_marks: std::collections::HashMap::new(),
            has_status: true,
        });
        let (output, _) = remote_render(&projection, &root);
        let text = output_text(&output);
        assert!(text.contains("Changes (1)"), "{text}");
        assert!(text.contains("README.md"), "{text}");
        assert!(text.contains("Git status unavailable"), "{text}");
        assert!(!text.contains("No changes"), "{text}");

        // A previously clean status must not silently become "No changes"
        // after a failed refresh.
        projection.git_view = Some(RemoteGitView {
            branch: Some("main".to_owned()),
            detached: false,
            commit: None,
            subject: None,
            linked_worktree: false,
            worktree_name: Some("PlantSuite".to_owned()),
            groups: Vec::new(),
            freshness: Freshness::Unavailable(made_remote::protocol::ErrorCode::Unavailable),
            folder_marks: std::collections::HashMap::new(),
            has_status: true,
        });
        let (output, _) = remote_render(&projection, &root);
        let text = output_text(&output);
        assert!(text.contains("Git status unavailable"), "{text}");
        assert!(!text.contains("No changes"), "{text}");
    }

    #[test]
    fn remote_root_listing_failure_keeps_the_last_tree_and_shows_the_shared_strip() {
        use crate::remote_workspace::Freshness;
        use made_remote::protocol::RemotePath;

        let root = RemotePath::new(b"/home/vscode".to_vec()).unwrap();
        let mut projection = remote_projection();
        projection.directories.get_mut(&root).unwrap().freshness =
            Freshness::Unavailable(made_remote::protocol::ErrorCode::Unavailable);
        let (output, _) = remote_render(&projection, &root);
        let text = output_text(&output);
        assert!(
            text.contains("Active root unavailable — showing the last folder."),
            "{text}"
        );
        assert!(text.contains("README.md"), "{text}");
        assert!(text.contains("src"), "{text}");
        assert!(!text.contains("Explorer unavailable"), "{text}");
    }

    #[test]
    fn remote_expanded_directory_failure_adds_the_local_error_row() {
        use crate::remote_workspace::Freshness;
        use made_remote::protocol::RemotePath;

        let root = RemotePath::new(b"/home/vscode".to_vec()).unwrap();
        let src = RemotePath::new(b"/home/vscode/src".to_vec()).unwrap();
        let mut projection = remote_projection();
        projection.directories.get_mut(&src).unwrap().freshness =
            Freshness::Unavailable(made_remote::protocol::ErrorCode::Unavailable);
        let (output, _) = remote_render(&projection, &root);
        let text = output_text(&output);
        assert!(
            text.matches("src").count() >= 2,
            "expected the directory row and its error row: {text}"
        );
    }

    #[test]
    fn remote_depth_cap_emits_the_local_tree_limit_row() {
        use crate::remote_workspace::{
            DirectoryProjection, ExplorerMode, Freshness, RootSource, WorkspaceProjection,
        };
        use made_remote::protocol::{DirectoryEntry, EntryKind as RemoteEntryKind, RemotePath};

        let root = RemotePath::new(b"/home/vscode".to_vec()).unwrap();
        let mut directories = std::collections::HashMap::new();
        let mut expanded = std::collections::HashSet::new();
        let mut current = root.clone();
        for index in 0..66 {
            let child = RemotePath::new(format!("/home/vscode/d{index}").into_bytes()).unwrap();
            directories.insert(
                current.clone(),
                DirectoryProjection {
                    path: current.clone(),
                    entries: vec![DirectoryEntry {
                        path: child.clone(),
                        kind: RemoteEntryKind::Directory,
                    }],
                    truncated: false,
                    freshness: Freshness::Current,
                },
            );
            expanded.insert(current);
            current = child;
        }
        directories.insert(
            current.clone(),
            DirectoryProjection {
                path: current.clone(),
                entries: Vec::new(),
                truncated: false,
                freshness: Freshness::Current,
            },
        );
        expanded.insert(current);
        let projection = WorkspaceProjection {
            source: RootSource::Launch,
            requested_root: None,
            root_state: Freshness::Current,
            directories,
            expanded,
            selected: None,
            git_selected: None,
            git_root_known: false,
            preview: None,
            preview_revision: 0,
            revision: 0,
            git: Vec::new(),
            git_view: None,
            sessions: None,
            session_state: Freshness::Stale,
            integration_error: None,
            focus: None,
            grok: None,
            mode: ExplorerMode::Files,
            cleanup_unconfirmed: false,
        };
        let mut rows = Vec::new();
        collect_remote_rows(
            &root,
            &projection,
            None,
            &std::collections::HashMap::new(),
            &mut rows,
            0,
        );
        assert!(
            rows.iter().any(|row| row.name == "(tree truncated)"),
            "expected the local tree limit row"
        );
    }

    #[test]
    fn remote_source_control_icon_appears_when_toplevel_is_known() {
        use made_remote::protocol::RemotePath;

        let root = RemotePath::new(b"/home/vscode".to_vec()).unwrap();
        let mut projection = remote_projection();
        projection.git_root_known = true;
        projection.git_view = None;
        let (ctx, output, _) = remote_render_ctx(&projection, &root);
        assert!(
            ctx.read_response(egui::Id::unique("remote-explorer-test").with("source-control"))
                .is_some(),
            "source-control icon must not wait for all eight Git reads"
        );
        assert!(output_text(&output).contains("EXPLORER"));
    }

    #[test]
    fn remote_header_uses_the_requested_root_while_loading() {
        use crate::remote_workspace::Freshness;
        use made_remote::protocol::RemotePath;

        let root = RemotePath::new(b"/home/vscode".to_vec()).unwrap();
        let requested = RemotePath::new(b"/workspaces/PlantSuite".to_vec()).unwrap();
        let mut projection = remote_projection();
        projection.root_state = Freshness::Loading;
        projection.requested_root = Some(requested);
        let (output, _) = remote_render(&projection, &root);
        let text = output_text(&output);
        assert!(text.contains("PlantSuite"), "{text}");
        assert!(!text.contains("vscode"), "{text}");
    }
}
