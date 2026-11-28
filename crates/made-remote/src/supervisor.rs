//! Native per-tab owner. The supervisor process, not the SSH frontend, holds the
//! PTY. Frontend death drops the attachment only.
//!
//! No tmux binary, socket, config, probe, or fallback.
//!
//! # Platform contracts observed in this tree
//!
//! - Detach is `made_platform::spawn_detached_command` (module `detach` is
//!   private; that function is the re-export). It starts a new session, ignores
//!   SIGHUP, closes inherited stdio, and uses a private directory. The frontend
//!   calls it and passes `<data>/remote/detach` as that directory. This module
//!   does not reimplement OS detach.
//! - Endpoint publish uses the existing `write_private_metadata_atomic` /
//!   `read_private_metadata`. The draft helpers `write_private` and
//!   `read_private_file` are not the atomic API.
//! - `PtyProcess` is the owner. It is the public adapter over
//!   `Platform::spawn_pty` / `PtySession`. Dropping it is the scoped close;
//!   dropping an attachment does not drop it.
//! - `CwdReportSink` exists on `made_platform::shell_integration` but that
//!   module is still private. The import is the expected public path once
//!   platform registers it (task 3.1). No `cfg(target_os)` is added here.
use crate::{
    claude::{self, ClaudeSpool, ClaudeTracker, ListingWorker},
    clipboard::Uploads,
    filesystem::{Filesystem, WatchOutcome},
    git::GitRunner,
    grok::{self, GrokTracker},
    integration::Integration,
    protocol::*,
};
use made_bridge::remote_rpc::{RpcConnection, RpcListener};
use made_platform::open_url::{RateDecision, UrlRateGate, UrlRequestSink, UrlToken};
use made_platform::shell_integration::{CwdReportSink, ShellLaunchPlan};
use made_platform::PtySize;
use made_terminal::{
    CellAttributes, CellSnapshot, CursorShape, InputModes, LineSnapshot, MouseMode, PtyEvent,
    PtyOptions, PtyProcess, TerminalColor, TerminalCore, TerminalSize, TerminalSnapshot,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

/// Scrollback bound shared with the local terminal. Not a replay log.
pub const SCROLLBACK_LINES: usize = 10_000;

/// Leaves the alternate screen and resets the input and display modes a
/// full-screen program may leave set when it exits: the scroll region and
/// origin mode (with the cursor kept where it is), autowrap, attributes,
/// cursor visibility and shape, cursor and keypad application modes, mouse
/// and focus reporting (including the UTF-8 and urxvt mouse encodings), the
/// enhanced keyboard protocol, bracketed paste and the window title. The terminal
/// pane uses the same bytes when it starts a shell after a local TUI exit.
pub const LEAVE_PROGRAM_MODES: &[u8] = b"\x1b[?1049l\x1b7\x1b[?6l\x1b[r\x1b8\x1b[?7h\x1b[0m\x1b[?25h\x1b[0 q\x1b[?1l\x1b>\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1015l\x1b[?1004l\x1b[?2004l\x1b[<99u\x1b]2;\x07";

/// Starts the OpenCode integration a plain shell launch owns.
pub type IntegrationStart = Box<dyn FnOnce() -> Option<Integration> + Send>;

/// The shell a `StartShell` request runs: what `LaunchTarget::Shell` runs,
/// with the environment a fresh `Terminal SSH` shell gets.
pub struct FollowUpShell {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// Set on top of the launch's environment: what a shell launch receives
    /// beyond this launch (`launch::LaunchEnvironment::follow_up`).
    pub env: Vec<(OsString, OsString)>,
    /// Starts the OpenCode integration a shell launch owns, for a launch that
    /// has none (Grok Build and Claude Code own no bridge until a shell takes
    /// their place). Its variable goes to the shell.
    pub integration: Option<IntegrationStart>,
}

/// Longest wait for output a finished child printed before its exit was
/// reported, when its terminal gives way to the follow-up shell.
const EXIT_OUTPUT_DRAIN: Duration = Duration::from_millis(100);

/// About one forwarded URL request per tab in this window; repeats of the
/// last URL inside it are coalesced.
pub const URL_REQUEST_WINDOW: Duration = Duration::from_secs(1);

/// Whether a routed URL request is acknowledged: a repeat inside the window
/// is coalesced, a different URL inside it is refused, and a fresh one is
/// acknowledged only when `deliver` queued its frame. It is recorded only
/// then, so an undelivered request's retry is fresh again rather than a
/// duplicate acknowledged without delivery.
fn url_request_acked(
    gate: &mut UrlRateGate,
    url: &made_platform::open_url::SafeUrl,
    now: Instant,
    deliver: impl FnOnce() -> bool,
) -> bool {
    match gate.check(url, now) {
        RateDecision::Duplicate => true,
        RateDecision::Limited => false,
        RateDecision::Fresh => {
            let delivered = deliver();
            if delivered {
                gate.record(url, now);
            }
            delivered
        }
    }
}

/// Point the child's `BROWSER` at the companion's URL helper mode: a
/// `made-open` symlink to the companion in this tab's private runtime
/// directory, since `BROWSER` cannot carry a mode argument. A user `BROWSER`
/// is preserved as `MADE_ORIGINAL_BROWSER`. On any failure the child keeps
/// its environment and no URL variables are set.
fn url_open_launch(
    runtime: &Path,
    companion: &Path,
    env: &mut Vec<(OsString, OsString)>,
) -> Option<(UrlRequestSink, UrlToken)> {
    use made_platform::open_url::{
        browser_environment, link_remote_helper, BROWSER_ENV, REMOTE_HELPER_NAME, URL_SOCKET_ENV,
        URL_TOKEN_ENV,
    };
    env.retain(|(key, _)| key != URL_SOCKET_ENV && key != URL_TOKEN_ENV);
    let helper = runtime.join(REMOTE_HELPER_NAME);
    link_remote_helper(companion, &helper).ok()?;
    let sink = UrlRequestSink::new(runtime).ok()?;
    let token = UrlToken::generate().ok()?;
    let inherited = env
        .iter()
        .find(|(key, _)| key == BROWSER_ENV)
        .map(|(_, value)| value.clone());
    let set = browser_environment(&helper, sink.socket_path(), &token, inherited.as_deref());
    env.retain(|(key, _)| set.iter().all(|(set_key, _)| set_key != key));
    env.extend(set);
    Some((sink, token))
}

#[derive(Debug)]
pub struct Lease {
    renewed: Instant,
}
impl Lease {
    pub fn new(now: Instant) -> Self {
        Self { renewed: now }
    }
    pub fn renew(&mut self, now: Instant) {
        self.renewed = now;
    }
    pub fn expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.renewed) >= DETACHED_LEASE
    }
}

#[derive(Serialize, Deserialize)]
pub struct ChildLaunch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
}

pub use made_platform::locale::{ensure_utf8_locale, installed_locales};

#[derive(Serialize, Deserialize)]
pub struct Endpoint {
    pub addr: SocketAddr,
}

pub fn capability_bytes(identity: &Identity) -> Result<[u8; 32], ErrorCode> {
    serde_json::from_value(serde_json::to_value(&identity.tab).map_err(|_| ErrorCode::Io)?)
        .map_err(|_| ErrorCode::Io)
}

pub fn runtime_directory(base: &Path, identity: &Identity) -> Result<PathBuf, ErrorCode> {
    let capability = capability_bytes(identity)?;
    // Public one-way locator; never put the bearer capability in argv. A
    // collision refuses Open (exclusive directory creation), never aliases tabs.
    let name = made_bridge::Token::from_bytes(capability)
        .fingerprint()
        .replace(':', "-");
    Ok(base.join("remote/runtime").join(name))
}

fn set_var(
    env: &mut Vec<(OsString, OsString)>,
    key: impl Into<OsString>,
    value: impl Into<OsString>,
) {
    let key = key.into();
    env.retain(|(existing, _)| *existing != key);
    env.push((key, value.into()));
}

fn io_error(_: impl std::fmt::Debug) -> ErrorCode {
    ErrorCode::Io
}

fn publish_private(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), ErrorCode> {
    made_platform::write_private_metadata_atomic(directory, name, bytes, bytes.len().max(1))
        .map_err(io_error)
}

/// Bounded VT plus the PTY child. The attachment does not own this value.
pub struct NativeSession {
    process: PtyProcess,
    vt: VtState,
}

impl NativeSession {
    pub fn spawn(
        platform: &dyn made_platform::Platform,
        options: PtyOptions,
    ) -> Result<Self, ErrorCode> {
        let rows = options.size.rows;
        let cols = options.size.cols;
        let process = PtyProcess::spawn(platform, options).map_err(io_error)?;
        Ok(Self {
            process,
            vt: VtState::new(rows, cols),
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.process.pid()
    }

    /// Whether the managed child is still running. Monitor failure reports
    /// not running: ingestion must not attribute rows to an unobservable
    /// process.
    pub fn is_running(&self) -> bool {
        self.process.try_is_running().unwrap_or(false)
    }

    /// Whether the child is known to have ended. A monitor failure leaves
    /// its state unknown, which is not an end: replacing it could kill a
    /// child that still runs.
    pub fn has_ended(&self) -> bool {
        self.process.try_is_running().is_ok_and(|running| !running)
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), ErrorCode> {
        self.process.resize(rows, cols).map_err(io_error)?;
        self.vt.resize(rows, cols);
        Ok(())
    }

    /// Change only the PTY window size, so the child gets SIGWINCH, and leave
    /// the bounded VT state alone. The redraw nudge uses it: the child repaints
    /// at the transient size and again at the real size, and the VT follows
    /// from that output.
    pub fn resize_pty_only(&mut self, rows: u16, cols: u16) -> Result<(), ErrorCode> {
        self.process.resize(rows, cols).map_err(io_error)
    }

    pub fn alternate_screen(&self) -> bool {
        self.vt.alternate_screen()
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), ErrorCode> {
        self.process.write(bytes).map_err(|_| ErrorCode::Limit)
    }

    pub fn terminate(&mut self) -> bool {
        self.process.terminate().is_ok()
    }

    /// Replace the exited child by a new one in the same terminal, keeping
    /// the bounded VT state: the screen and scrollback the old child left
    /// stay, and the modes it may have left set are reset. Returns the old
    /// child's output that was still queued, then the mode reset, in the order
    /// the host has to apply them; the VT already holds all of it. A child
    /// that cannot start leaves the session untouched.
    pub fn restart(
        &mut self,
        platform: &dyn made_platform::Platform,
        options: PtyOptions,
    ) -> Result<Vec<Vec<u8>>, ErrorCode> {
        if !self.has_ended() {
            return Err(ErrorCode::InvalidRequest);
        }
        let mut next = PtyProcess::spawn(platform, options).map_err(io_error)?;
        std::mem::swap(&mut self.process, &mut next);
        let mut old = next;
        // The exit can be reported before the reader forwarded the child's
        // last bytes; a process it left behind may keep writing, so the
        // deadline ends the drain.
        let mut pieces = Vec::new();
        let end = Instant::now() + EXIT_OUTPUT_DRAIN;
        loop {
            let remaining = end.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let Ok(PtyEvent::Data(data)) = old.recv_timeout(remaining) else {
                break;
            };
            self.vt.feed(&data);
            pieces.push(data);
        }
        let _ = old.terminate();
        self.vt.feed(LEAVE_PROGRAM_MODES);
        pieces.push(LEAVE_PROGRAM_MODES.to_vec());
        Ok(pieces)
    }

    /// Feed already-read PTY bytes into the bounded core. Raw bytes are not kept.
    pub fn observe(&mut self, bytes: &[u8]) {
        self.vt.feed(bytes);
    }

    pub fn poll(&mut self) -> Option<PtyEvent> {
        self.process.poll()
    }

    pub fn resnapshot(&self) -> Vec<u8> {
        self.vt.resnapshot()
    }
}

/// Primary grid retained across alternate-screen entry. Alacritty does not
/// expose the inactive grid, so the transition is split before it is lost.
pub struct VtState {
    core: TerminalCore,
    primary: PrimaryRetention,
}

struct PrimaryRetention {
    history: Vec<LineSnapshot>,
    screen: TerminalSnapshot,
    modes: InputModes,
}

impl VtState {
    pub fn new(rows: u16, cols: u16) -> Self {
        let mut core = TerminalCore::with_size(size_of(rows, cols), SCROLLBACK_LINES);
        core.resize(usize::from(rows.max(1)), usize::from(cols.max(1)));
        let primary = PrimaryRetention::capture(&core);
        Self { core, primary }
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.core
            .resize(usize::from(rows.max(1)), usize::from(cols.max(1)));
        if !self.core.alternate_screen() {
            self.primary = PrimaryRetention::capture(&self.core);
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == 0x1b {
                if !self.core.alternate_screen() {
                    self.primary = PrimaryRetention::capture(&self.core);
                }
                let end = index + sequence_end(&bytes[index..]);
                self.core.feed(&bytes[index..end]);
                index = end;
            } else {
                let next = bytes[index..]
                    .iter()
                    .position(|byte| *byte == 0x1b)
                    .map(|offset| index + offset)
                    .unwrap_or(bytes.len());
                self.core.feed(&bytes[index..next]);
                index = next;
            }
            if !self.core.alternate_screen() {
                self.primary = PrimaryRetention::capture(&self.core);
            }
        }
        // OSC 52 copies are applied by the client's own core; do not retain them.
        let _ = self.core.take_clipboard_store();
    }

    pub fn resnapshot(&self) -> Vec<u8> {
        encode_resnapshot(&self.core, &self.primary)
    }

    pub fn alternate_screen(&self) -> bool {
        self.core.alternate_screen()
    }

    pub fn modes(&self) -> InputModes {
        self.core.input_modes()
    }

    pub fn screen_text(&self) -> String {
        self.core.screen_text()
    }
}

impl PrimaryRetention {
    fn capture(core: &TerminalCore) -> Self {
        Self {
            history: core.history(SCROLLBACK_LINES),
            screen: core.snapshot(),
            modes: core.input_modes(),
        }
    }
}

fn size_of(rows: u16, cols: u16) -> TerminalSize {
    TerminalSize {
        rows: usize::from(rows.max(1)),
        cols: usize::from(cols.max(1)),
    }
}

fn sequence_end(bytes: &[u8]) -> usize {
    if bytes.is_empty() {
        return 0;
    }
    if bytes[0] != 0x1b {
        return 1;
    }
    if bytes.len() == 1 {
        return 1;
    }
    match bytes[1] {
        b'[' => {
            let mut index = 2;
            while index < bytes.len() && index < 66 {
                let byte = bytes[index];
                index += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
            index
        }
        b']' => {
            let mut index = 2;
            while index < bytes.len() && index < 258 {
                if bytes[index] == 0x07 {
                    return index + 1;
                }
                if bytes[index] == 0x1b && index + 1 < bytes.len() && bytes[index + 1] == b'\\' {
                    return index + 2;
                }
                index += 1;
            }
            index
        }
        _ => 2.min(bytes.len()),
    }
}

fn encode_resnapshot(core: &TerminalCore, primary: &PrimaryRetention) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"\x1b[0m");
    paint_history(&mut out, &primary.history, primary.screen.size.rows);
    // No `ESC[2J`: `paint_viewport` writes every cell of every row, and the
    // client feeds this into a fresh grid. An erase-display would also push a
    // blank line into the client's scrollback, which surfaces as an empty
    // first row above the prompt once the pane grows past the attach size.
    paint_viewport(&mut out, &primary.screen);
    push_modes(&mut out, &primary.modes);
    push_cursor(&mut out, &primary.screen);
    if core.alternate_screen() {
        out.extend_from_slice(b"\x1b[?1049h");
        let alternate = core.snapshot();
        paint_viewport(&mut out, &alternate);
        push_modes(&mut out, &core.input_modes());
        push_cursor(&mut out, &alternate);
    }
    out
}

fn paint_history(out: &mut Vec<u8>, history: &[LineSnapshot], rows: usize) {
    if history.is_empty() || rows == 0 {
        return;
    }
    // Write the history from the top row, then scroll the viewport blank:
    // every history line ends up in the client's scrollback and no blank
    // line precedes it. The viewport is painted over the empty grid next.
    push_cup(out, 0, 0);
    for line in history {
        push_line_text(out, line);
        if !line.wrapped {
            out.extend_from_slice(b"\r\n");
        }
    }
    for _ in 1..rows {
        out.extend_from_slice(b"\r\n");
    }
}

fn paint_viewport(out: &mut Vec<u8>, snapshot: &TerminalSnapshot) {
    let cols = snapshot.size.cols;
    for row in 0..snapshot.size.rows {
        push_cup(out, row, 0);
        let start = row * cols;
        push_cells(out, &snapshot.cells[start..start + cols]);
    }
}

fn push_line_text(out: &mut Vec<u8>, line: &LineSnapshot) {
    push_cells(out, &line.cells);
}

type Style = (TerminalColor, TerminalColor, CellAttributes);

/// Cell text with its colors and attributes. Without them a reattached TUI
/// shows monochrome text that the application, believing the old screen is
/// intact, only partly repaints. Each call starts and ends in the default style.
fn push_cells(out: &mut Vec<u8>, cells: &[CellSnapshot]) {
    let default = Style::default();
    let mut style = default;
    for cell in cells {
        if cell.wide_spacer {
            continue;
        }
        let wanted = (cell.fg, cell.bg, cell.attributes);
        if wanted != style {
            push_sgr(out, wanted);
            style = wanted;
        }
        push_char(out, cell.ch);
        for extra in &cell.combining {
            push_char(out, *extra);
        }
    }
    if style != default {
        out.extend_from_slice(b"\x1b[0m");
    }
}

fn push_sgr(out: &mut Vec<u8>, (fg, bg, attributes): Style) {
    out.extend_from_slice(b"\x1b[0");
    for (on, code) in [
        (attributes.bold, 1),
        (attributes.dim, 2),
        (attributes.italic, 3),
        (attributes.underline, 4),
        (attributes.inverse, 7),
        (attributes.hidden, 8),
        (attributes.strikethrough, 9),
    ] {
        if on {
            let _ = write!(out, ";{code}");
        }
    }
    for (color, base) in [(fg, 30), (bg, 40)] {
        match color {
            TerminalColor::Default => {}
            TerminalColor::Indexed(index) => {
                let _ = write!(out, ";{};5;{index}", base + 8);
            }
            TerminalColor::Rgb(r, g, b) => {
                let _ = write!(out, ";{};2;{r};{g};{b}", base + 8);
            }
        }
    }
    out.push(b'm');
}

fn push_char(out: &mut Vec<u8>, ch: char) {
    let ch = if ch.is_control() { ' ' } else { ch };
    let mut buf = [0u8; 4];
    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
}

fn push_cup(out: &mut Vec<u8>, row: usize, col: usize) {
    let _ = write!(out, "\x1b[{};{}H", row + 1, col + 1);
}

fn push_modes(out: &mut Vec<u8>, modes: &InputModes) {
    private_mode(out, 1, modes.app_cursor);
    if modes.app_keypad {
        out.extend_from_slice(b"\x1b=");
    } else {
        out.extend_from_slice(b"\x1b>");
    }
    private_mode(out, 1000, false);
    private_mode(out, 1002, false);
    private_mode(out, 1003, false);
    let mouse = match modes.mouse_mode {
        MouseMode::Click => Some(1000),
        MouseMode::Drag => Some(1002),
        MouseMode::Motion => Some(1003),
        MouseMode::Off => None,
    };
    if let Some(mode) = mouse {
        private_mode(out, mode, true);
    }
    private_mode(out, 1006, modes.sgr_mouse);
    private_mode(out, 1004, modes.focus_reporting);
    private_mode(out, 2004, modes.bracketed_paste);
    if modes.csi_u {
        out.extend_from_slice(b"\x1b[=1u");
    } else {
        out.extend_from_slice(b"\x1b[=0u");
    }
}

fn private_mode(out: &mut Vec<u8>, mode: u16, on: bool) {
    let flag = if on { 'h' } else { 'l' };
    let _ = write!(out, "\x1b[?{mode}{flag}");
}

fn push_cursor(out: &mut Vec<u8>, snapshot: &TerminalSnapshot) {
    let cursor = &snapshot.cursor;
    if cursor.shape == CursorShape::Hidden || !cursor.visible {
        out.extend_from_slice(b"\x1b[?25l");
    } else {
        out.extend_from_slice(b"\x1b[?25h");
        let style = match (cursor.shape, cursor.blinking) {
            (CursorShape::Underline, true) => 3,
            (CursorShape::Underline, false) => 4,
            (CursorShape::Bar, true) => 5,
            (CursorShape::Bar, false) => 6,
            (_, true) => 1,
            (_, false) => 2,
        };
        let _ = write!(out, "\x1b[{style} q");
    }
    push_cup(out, cursor.row, cursor.col);
}

struct OutputQueue {
    control: SyncSender<WireEnvelope>,
    data: SyncSender<WireEnvelope>,
}

impl OutputQueue {
    // The unsent frame has to travel in `TrySendError` so the caller can keep it.
    #[allow(clippy::result_large_err)]
    fn try_send(&self, frame: WireEnvelope) -> Result<(), TrySendError<WireEnvelope>> {
        if is_data(&frame.operation) {
            self.data.try_send(frame)
        } else {
            self.control.try_send(frame)
        }
    }
}

/// Next resnapshot frame. `completed` stays false until the single `done: true`
/// frame is produced. Live output must not be emitted before that.
pub fn next_snapshot_frame(rest: &[u8], completed: bool) -> Option<(Operation, usize, bool)> {
    if completed {
        return None;
    }
    if rest.is_empty() {
        let data = Chunk::new(Vec::new()).ok()?;
        return Some((Operation::TerminalOutput { data, done: true }, 0, true));
    }
    let take = MAX_CHUNK_BYTES.min(rest.len());
    let data = Chunk::new(rest[..take].to_vec()).ok()?;
    Some((Operation::TerminalOutput { data, done: false }, take, false))
}

/// Live PTY bytes after the resnapshot. Never a second `done: true`, never empty.
pub fn live_terminal_output(bytes: &[u8]) -> Option<Operation> {
    if bytes.is_empty() {
        return None;
    }
    let data = Chunk::new(bytes.to_vec()).ok()?;
    Some(Operation::TerminalOutput { data, done: false })
}

/// Claude Code state rides the data lane too, so it reaches the client after
/// the resnapshot frames queued before it, never ahead of the completing one.
/// So does the child's exit: it follows every output frame the child printed
/// before it, and the state pushed before it.
fn is_data(operation: &Operation) -> bool {
    matches!(
        operation,
        Operation::TerminalOutput { .. }
            | Operation::TerminalExit { .. }
            | Operation::Listed { .. }
            | Operation::GitResult { .. }
            | Operation::PreviewResult { .. }
            | Operation::ClaudeSnapshot { .. }
            | Operation::ClaudeEvent { .. }
            | Operation::ClaudeListing { .. }
    )
}

struct Attachment {
    stream: TcpStream,
    input: Receiver<WireEnvelope>,
    output: OutputQueue,
    identity: Identity,
    request: u64,
    watch_request: Option<u64>,
    snapshot_rest: Vec<u8>,
    /// Set only after the single completing `done: true` frame is queued.
    snapshot_closed: bool,
    /// Re-emit the shell cwd once the resnapshot has been queued.
    cwd_pending: bool,
    pending_relay: Option<Vec<u8>>,
    pending_control: Vec<WireEnvelope>,
}

/// Heartbeat, terminal I/O, and close are not filesystem-generation scoped.
pub fn generation_exempt(operation: &Operation) -> bool {
    matches!(
        operation,
        Operation::Heartbeat
            | Operation::Capabilities
            | Operation::Close
            | Operation::Cancel { .. }
            | Operation::TerminalInput { .. }
            | Operation::TerminalResize { .. }
            | Operation::StartShell { .. }
            | Operation::SessionState
            | Operation::Focus { .. }
            | Operation::GrokState
            | Operation::ClaudeState
            | Operation::ClaudeTrigger
            | Operation::UploadBegin { .. }
            | Operation::UploadChunk { .. }
            | Operation::UploadCommit { .. }
            | Operation::UploadAbort { .. }
    )
}

pub fn transport_matches(request: &Identity, current: &Identity) -> bool {
    request.tab.authenticates(&current.tab)
        && request.authority == current.authority
        && request.epoch == current.epoch
}

pub struct Supervisor {
    identity: Identity,
    runtime: PathBuf,
    session: NativeSession,
    filesystem: Filesystem,
    uploads: Uploads,
    integration: Option<Integration>,
    grok: GrokTracker,
    grok_detected: bool,
    claude: ClaudeTracker,
    /// The tab's spool, removed with the supervisor.
    _claude_spool: Option<ClaudeSpool>,
    /// A `ClaudeState` request answered once the resnapshot completes.
    claude_state_pending: Option<WireEnvelope>,
    /// The current attachment asked for Claude Code state. Pushes start only
    /// then, so a client that never asks never receives them.
    claude_subscribed: bool,
    lease: Lease,
    attachment: Option<Attachment>,
    attached_once: bool,
    cwd_sink: CwdReportSink,
    /// The tab's URL request endpoint and the token its child presents. Absent
    /// when the helper link or the endpoint could not be created; the child
    /// then keeps its own `BROWSER`.
    url_open: Option<(UrlRequestSink, UrlToken)>,
    url_gate: UrlRateGate,
    shell_plan: Option<ShellLaunchPlan>,
    shell_cwd: Option<RemotePath>,
    /// A validated report has arrived, so a reattach must restore it: the
    /// client forgets the shell root with each new epoch.
    cwd_reported: bool,
    /// Current PTY window size as last requested by an attach or resize.
    pty_size: (u16, u16),
    /// The child is the user's shell rather than a TUI launched by Made.
    shell_target: bool,
    /// The remote interactive shell a `StartShell` request starts after the
    /// launch child ended. Absent for a companion that was not given one.
    follow_up: Option<FollowUpShell>,
    /// The launch environment before any shell bootstrap, which the follow-up
    /// shell starts from, and the companion that reports its directory.
    launch_env: Vec<(OsString, OsString)>,
    companion: PathBuf,
    /// Output the replaced child printed last, then the mode reset, still to
    /// reach the attached client before the new shell's own output.
    relay_tail: VecDeque<Vec<u8>>,
    /// The current child's exit was queued for the attached client. A child
    /// exits once: reporting it every pass would flood the client and reach
    /// the shell that replaces the child.
    exit_reported: bool,
    redraw: Option<RedrawNudge>,
    closed: bool,
    git_reads: GitReads,
}

/// One Git read waiting for the Git thread.
struct GitJob {
    identity: Identity,
    request_id: u64,
    command: crate::protocol::GitRead,
    directory: PathBuf,
}

struct GitDone {
    identity: Identity,
    request_id: u64,
    command: crate::protocol::GitRead,
    outcome: Result<Vec<u8>, ErrorCode>,
}

/// Git reads run on their own thread, one at a time. A `git status` of a big
/// repository takes long enough to stall the loop that relays the terminal and
/// answers every other request, and the client refreshes Git on every change
/// notification and on its periodic safety net.
struct GitReads {
    jobs: SyncSender<GitJob>,
    results: Receiver<GitDone>,
    /// The attachment epoch and root generation reads are still wanted for.
    /// A queued read of an older one is skipped before it runs.
    current: Arc<GitWanted>,
}

#[derive(Default)]
struct GitWanted {
    epoch: AtomicU64,
    generation: AtomicU64,
}

impl GitWanted {
    fn set(&self, identity: Option<&Identity>) {
        let (epoch, generation) = identity.map_or((0, 0), |i| (i.epoch, i.generation));
        self.epoch.store(epoch, Ordering::Relaxed);
        self.generation.store(generation, Ordering::Relaxed);
    }
    fn wants(&self, identity: &Identity) -> bool {
        self.epoch.load(Ordering::Relaxed) == identity.epoch
            && self.generation.load(Ordering::Relaxed) == identity.generation
    }
}

/// Reads queued for the Git thread; the client never has more in flight than
/// `MAX_IN_FLIGHT`, so this only bounds a misbehaving one.
const GIT_QUEUE: usize = 64;

impl GitReads {
    fn start() -> Self {
        let (jobs, job_rx) = mpsc::sync_channel::<GitJob>(GIT_QUEUE);
        let (done_tx, results) = mpsc::channel();
        let current = Arc::new(GitWanted::default());
        let wanted = Arc::clone(&current);
        thread::spawn(move || {
            let runner = GitRunner::new();
            while let Ok(job) = job_rx.recv() {
                // Nobody waits for the read of a detached attachment or a
                // previous root: do not spend a `git status` on it.
                if !wanted.wants(&job.identity) {
                    continue;
                }
                let outcome = runner
                    .read_bytes(job.command.into(), &job.directory)
                    .map_err(|_| ErrorCode::Unavailable);
                let done = GitDone {
                    identity: job.identity,
                    request_id: job.request_id,
                    command: job.command,
                    outcome,
                };
                if done_tx.send(done).is_err() {
                    break;
                }
            }
        });
        Self {
            jobs,
            results,
            current,
        }
    }
}

/// Delay between the transient and the real window size of a redraw nudge.
/// Back-to-back resizes can be coalesced into one signal whose size matches
/// the old one, which applications treat as "nothing changed".
const NUDGE_STEP: Duration = Duration::from_millis(60);

/// After a reattach the client holds only the supervisor's bounded text
/// snapshot, while the application believes its previous screen is intact and
/// repaints only what changes. Resizing the PTY to a transient size and back
/// makes it send SIGWINCH and repaint everything. The client's own follow-up
/// `TerminalResize` (reattach always opens at the default size) usually does
/// this too and then cancels the nudge, but it sends nothing when the real
/// size equals the default or the hidden tab never lays out, so the nudge is
/// the guarantee.
struct RedrawNudge {
    shrunk: bool,
    due: Instant,
}

impl Supervisor {
    pub fn create(
        first: &WireEnvelope,
        runtime: PathBuf,
        data: &Path,
        companion: &Path,
        launch: ChildLaunch,
        integration: Option<Integration>,
    ) -> Result<Self, ErrorCode> {
        let Operation::Open {
            directory,
            rows,
            cols,
            ..
        } = &first.operation
        else {
            return Err(ErrorCode::InvalidRequest);
        };
        first.validate().map_err(|_| ErrorCode::InvalidRequest)?;
        let filesystem = Filesystem::new(directory, first.identity.generation)?;
        let uploads = Uploads::new(data.join("remote/clipboard-images"))?;
        let cwd_sink = CwdReportSink::new(&runtime).map_err(io_error)?;
        let mut launch = launch;
        let url_open = url_open_launch(&runtime, companion, &mut launch.env);
        // Helper start: spools of supervisors that are gone are swept first;
        // every live supervisor's spool stays.
        if let Some(runtime_root) = runtime.parent() {
            claude::sweep_stale_spools(runtime_root);
        }
        // The launch composition bound this tab's spool path; create it before
        // the child starts, or drop the binding so no hook writes elsewhere.
        let spool_binding = claude::spool_path(&runtime);
        let claude_spool = if launch.env.iter().any(|(key, value)| {
            key == claude::CLAUDE_SPOOL_ENV && Path::new(value) == spool_binding
        }) {
            let spool = ClaudeSpool::create(&runtime).ok();
            if spool.is_none() {
                launch
                    .env
                    .retain(|(key, _)| key != claude::CLAUDE_SPOOL_ENV);
            }
            spool
        } else {
            None
        };
        ensure_utf8_locale(&mut launch.env, installed_locales);
        let launch_env = launch.env.clone();
        let shell_plan = if matches!(
            first.operation,
            Operation::Open {
                target: LaunchTarget::Shell,
                ..
            }
        ) {
            let mut request =
                made_platform::PtyRequest::new(launch.program.as_os_str(), &launch.cwd);
            request.args = launch.args;
            request.env = launch.env;
            let plan =
                ShellLaunchPlan::with_cwd_sink(request, companion.into(), cwd_sink.path().into());
            launch.args = plan.request().args.clone();
            launch.env = plan.request().env.clone();
            Some(plan)
        } else {
            None
        };
        let child_bytes = serde_json::to_vec(&launch).map_err(io_error)?;
        publish_private(&runtime, "child.json", &child_bytes)?;
        let child_path = runtime.join("child.json");
        let options = PtyOptions::new(&launch.cwd)
            .with_shell(companion)
            .with_args([OsString::from("--child"), child_path.into_os_string()])
            .with_size(PtySize {
                rows: *rows,
                cols: *cols,
            });
        let session = NativeSession::spawn(&made_platform::host_platform(), options)?;
        // Grok ingestion reads only the two fixed Grok-home files and filters
        // by live ancestry against this managed PTY child. The staged grok-made
        // directory sits beside the companion executable.
        let staged = companion
            .parent()
            .map(|directory| directory.join("plugins/grok-made"));
        let environment = made_platform::Environment::current();
        let grok_tracker = GrokTracker::new(
            session.pid(),
            matches!(
                first.operation,
                Operation::Open {
                    target: LaunchTarget::Grok,
                    ..
                }
            ),
            grok::GrokSources {
                files: grok::GrokFiles::from_environment(&environment),
                staged,
            },
        );
        let grok_detected = grok::detect_grok(&environment).is_some();
        // Claude Code ingestion reads only this tab's spool; the listing runs
        // the detected claude under the server user's app-owned coordination.
        let claude_tracker = ClaudeTracker::new(
            session.pid(),
            matches!(
                first.operation,
                Operation::Open {
                    target: LaunchTarget::Claude,
                    ..
                }
            ),
            claude_spool
                .as_ref()
                .map(|spool| spool.path().to_path_buf()),
            Some(ListingWorker::new(
                grok::detect_on_path(&environment, "claude"),
                data.join("remote/claude"),
            )),
        );
        Ok(Self {
            identity: first.identity.clone(),
            runtime,
            session,
            filesystem,
            uploads,
            integration,
            grok: grok_tracker,
            grok_detected,
            claude: claude_tracker,
            _claude_spool: claude_spool,
            claude_state_pending: None,
            claude_subscribed: false,
            lease: Lease::new(Instant::now()),
            attachment: None,
            attached_once: false,
            cwd_sink,
            url_open,
            url_gate: UrlRateGate::new(URL_REQUEST_WINDOW),
            shell_plan,
            shell_cwd: Some(directory.clone()),
            cwd_reported: false,
            pty_size: (*rows, *cols),
            shell_target: matches!(
                first.operation,
                Operation::Open {
                    target: LaunchTarget::Shell,
                    ..
                }
            ),
            follow_up: None,
            launch_env,
            companion: companion.to_path_buf(),
            relay_tail: VecDeque::new(),
            exit_reported: false,
            redraw: None,
            closed: false,
            git_reads: GitReads::start(),
        })
    }

    /// Let the host start the remote interactive shell in this attachment's
    /// terminal once the launch child ended (`StartShell`): after a launch-bar
    /// TUI's clean exit, and for the `Open shell` action of an unclean end of
    /// a TUI or of a plain shell.
    pub fn with_follow_up_shell(mut self, shell: FollowUpShell) -> Self {
        self.follow_up = Some(shell);
        self
    }

    fn install_attachment(&mut self, connection: RpcConnection) -> Result<(), ErrorCode> {
        let epoch = connection.epoch();
        let mut stream = connection.into_stream();
        // BSD/macOS accepted sockets inherit the listener's O_NONBLOCK, and
        // set_read_timeout(None) does not clear it. The attachment reader
        // relies on blocking reads: one idle nonblocking read fails
        // read_exact with EAGAIN and silently kills the attachment. Linux
        // never inherits the flag, so only macOS hosts exposed this.
        stream.set_nonblocking(false).map_err(io_error)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(120)))
            .map_err(io_error)?;
        let first = read_envelope(&mut stream).map_err(io_error)?;
        // A rejected attach must be distinguishable from a dead transport.
        // Always answer with the code after the connection authenticated.
        let reject = |stream: &mut TcpStream, code: ErrorCode| {
            let frame = WireEnvelope::new(
                first.identity.clone(),
                first.request_id,
                Operation::Error { code },
            );
            let _ = write_envelope(stream, &frame);
            code
        };
        if epoch < self.identity.epoch || (self.attached_once && epoch <= self.identity.epoch) {
            return Err(reject(&mut stream, ErrorCode::Stale));
        }
        if !transport_matches(
            &first.identity,
            &Identity {
                epoch,
                ..self.identity.clone()
            },
        ) || first.identity.epoch != epoch
        {
            return Err(reject(&mut stream, ErrorCode::Unauthorized));
        }
        let (rows, cols) = match &first.operation {
            Operation::Open { rows, cols, .. }
                if self.attachment.is_none() && epoch == self.identity.epoch =>
            {
                (*rows, *cols)
            }
            Operation::Attach { rows, cols } => (*rows, *cols),
            _ => return Err(reject(&mut stream, ErrorCode::InvalidRequest)),
        };
        if let Err(code) = self.session.resize(rows, cols) {
            return Err(reject(&mut stream, code));
        }
        self.pty_size = (rows, cols);
        let (input_tx, input) = mpsc::sync_channel(MAX_IN_FLIGHT);
        let (data_tx, data_rx) = mpsc::sync_channel(MAX_QUEUED_CHUNKS);
        let (control_tx, control_rx) = mpsc::sync_channel(MAX_IN_FLIGHT);
        let output = OutputQueue {
            control: control_tx,
            data: data_tx,
        };
        let mut identity = first.identity.clone();
        identity.generation = self.filesystem.generation();
        let attached = WireEnvelope::new(
            first.identity.clone(),
            first.request_id,
            Operation::Attached,
        );
        // Epoch is not current until Attached is actually queued.
        output.try_send(attached).map_err(|_| ErrorCode::Limit)?;
        let snapshot = self.session.resnapshot();
        let mut reader = stream.try_clone().map_err(io_error)?;
        let writer = stream.try_clone().map_err(io_error)?;
        reader.set_read_timeout(None).map_err(io_error)?;
        // Attached is queued and the new streams exist. Only then replace the
        // previous frontend and confirm the epoch.
        self.detach_frontend();
        self.identity.epoch = epoch;
        let reattach = self.attached_once;
        self.attached_once = true;
        self.identity.generation = self.filesystem.generation();
        self.lease.renew(Instant::now());
        if let Some(integration) = &mut self.integration {
            integration.reset_attachment();
        }
        self.grok.reset_attachment();
        self.claude.reset_attachment();
        self.claude_state_pending = None;
        self.claude_subscribed = false;
        // The new attachment's resnapshot already holds what the tail carries,
        // and it has not learned that the child ended.
        self.relay_tail.clear();
        self.exit_reported = false;
        self.uploads.pending_abort_all();
        thread::spawn(move || {
            while let Ok(frame) = read_envelope(&mut reader) {
                if input_tx.send(frame).is_err() {
                    break;
                }
            }
        });
        thread::spawn(move || writer_loop(writer, control_rx, data_rx));
        // A TUI cannot be assumed to repaint from a text snapshot. A shell
        // prompt is left alone unless something full-screen runs inside it.
        self.redraw =
            (reattach && (!self.shell_target || self.session.alternate_screen())).then(|| {
                RedrawNudge {
                    shrunk: false,
                    due: Instant::now(),
                }
            });
        // A new attachment starts without subscriptions: its client asks for
        // them again after the resnapshot, under its own request ids.
        self.filesystem.cancel_watch();
        self.git_reads.current.set(Some(&identity));
        self.attachment = Some(Attachment {
            stream,
            input,
            output,
            identity,
            request: first.request_id,
            watch_request: None,
            snapshot_rest: snapshot,
            snapshot_closed: false,
            cwd_pending: reattach,
            pending_relay: None,
            pending_control: Vec::new(),
        });
        Ok(())
    }

    fn detach_frontend(&mut self) {
        // Nobody reads notifications while detached, and the next client
        // subscribes again after its resnapshot.
        self.filesystem.cancel_watch();
        self.git_reads.current.set(None);
        if let Some(attachment) = self.attachment.take() {
            let _ = attachment.stream.shutdown(Shutdown::Both);
            drop(attachment);
        }
    }

    pub fn run(mut self) -> Result<(), ErrorCode> {
        let listener = Arc::new(
            RpcListener::bind(capability_bytes(&self.identity)?, self.identity.authority.0)
                .map_err(io_error)?,
        );
        let endpoint = serde_json::to_vec(&Endpoint {
            addr: listener.local_addr().map_err(io_error)?,
        })
        .map_err(io_error)?;
        publish_private(&self.runtime, "endpoint.json", &endpoint)?;
        let (authenticated, ready) = mpsc::sync_channel(8);
        let pending = Arc::new(AtomicUsize::new(0));
        loop {
            if self.lease.expired(Instant::now()) || self.closed {
                self.detach_frontend();
                let _ = self.session.terminate();
                break;
            }
            if let Some(stream) = listener.accept().map_err(io_error)? {
                if pending.load(Ordering::Relaxed) < 8 {
                    pending.fetch_add(1, Ordering::Relaxed);
                    let listener = Arc::clone(&listener);
                    let pending = Arc::clone(&pending);
                    let authenticated = authenticated.clone();
                    thread::spawn(move || {
                        if let Ok(connection) = listener.authenticate(stream) {
                            let _ = authenticated.try_send(connection);
                        }
                        pending.fetch_sub(1, Ordering::Relaxed);
                    });
                }
            }
            if let Ok(connection) = ready.try_recv() {
                let _ = self.install_attachment(connection);
            }
            let request = match self
                .attachment
                .as_ref()
                .map(|attachment| attachment.input.try_recv())
            {
                Some(Ok(request)) => Some(request),
                Some(Err(TryRecvError::Disconnected)) => {
                    self.detach_frontend();
                    None
                }
                _ => None,
            };
            if let Some(request) = request {
                if !self.accept_request(&request) {
                    self.reply(
                        &request,
                        Operation::Error {
                            code: ErrorCode::Stale,
                        },
                    );
                } else {
                    let close = matches!(request.operation, Operation::Close);
                    match self.dispatch(&request) {
                        Ok(replies) => {
                            for operation in replies {
                                self.reply(&request, operation);
                            }
                        }
                        Err(code) => self.reply(&request, Operation::Error { code }),
                    }
                    if close {
                        thread::sleep(Duration::from_millis(20));
                        self.detach_frontend();
                        break;
                    }
                }
            }
            self.poll_events();
            thread::sleep(Duration::from_millis(1));
        }
        if let Some(integration) = &self.integration {
            let _ = integration.verify_config();
        }
        let _ = fs::remove_dir_all(&self.runtime);
        Ok(())
    }

    fn accept_request(&self, request: &WireEnvelope) -> bool {
        transport_matches(&request.identity, &self.identity)
            && (request.identity.generation == self.identity.generation
                || generation_exempt(&request.operation))
    }

    fn reply(&mut self, request: &WireEnvelope, operation: Operation) {
        let response = WireEnvelope::new(request.identity.clone(), request.request_id, operation);
        self.enqueue(response);
    }

    /// Full is backpressure: keep the frontend. Only a disconnected queue drops it.
    /// Whether the frame is on its way: queued, or parked until the queue has room.
    fn enqueue(&mut self, frame: WireEnvelope) -> bool {
        let Some(attachment) = self.attachment.as_mut() else {
            return false;
        };
        match attachment.output.try_send(frame) {
            Ok(()) => true,
            Err(TrySendError::Full(frame)) => {
                if attachment.pending_control.len() < MAX_IN_FLIGHT {
                    attachment.pending_control.push(frame);
                    true
                } else {
                    false
                }
            }
            Err(TrySendError::Disconnected(_)) => {
                self.detach_frontend();
                false
            }
        }
    }

    fn dispatch(&mut self, request: &WireEnvelope) -> Result<Vec<Operation>, ErrorCode> {
        use Operation::*;
        let reply = match &request.operation {
            Capabilities => CapabilitiesResult {
                capabilities: crate::protocol::Capabilities {
                    target: current_target()?,
                    tmux: false,
                    opencode: self.integration.is_some(),
                    shell_cwd: self
                        .shell_plan
                        .as_ref()
                        .is_some_and(|plan| plan.capability().is_supported()),
                    helper_version: env!("CARGO_PKG_VERSION").into(),
                    // Honest runtime detection. tmux stays false on the wire.
                    grok: self.grok_detected,
                    python3: false,
                    start_shell: true,
                    watch_dirs: true,
                },
            },
            Heartbeat => {
                self.lease.renew(Instant::now());
                Ack
            }
            Close => {
                self.closed = true;
                Closed {
                    confirmed: self.session.terminate(),
                }
            }
            RootValidate { path } => self.filesystem.validate_root(path)?,
            RootSelect { path } => {
                let result = self.filesystem.select(path)?;
                self.uploads.pending_abort_all();
                self.identity.generation = self.filesystem.generation();
                if let Some(attachment) = &mut self.attachment {
                    attachment.identity.generation = self.identity.generation;
                    attachment.watch_request = None;
                    self.git_reads.current.set(Some(&attachment.identity));
                }
                result
            }
            List { path } => return self.filesystem.list(path),
            Preview { path } | Refresh { path } => return self.filesystem.preview(path),
            Watch { path } => {
                self.filesystem.watch(path)?;
                if let Some(attachment) = &mut self.attachment {
                    attachment.watch_request = Some(request.request_id);
                }
                Changed { path: path.clone() }
            }
            WatchDir { path } => match self.filesystem.watch_dir(path, request.request_id)? {
                WatchOutcome::Live => Changed { path: path.clone() },
                // Established on the watcher thread, never on this loop; the
                // acknowledgement follows from `poll_events`.
                WatchOutcome::Pending => return Ok(vec![]),
            },
            UnwatchDir { path } => {
                self.filesystem.unwatch_dir(path);
                Ack
            }
            Rescan { path } => {
                self.filesystem.list(path)?;
                Changed { path: path.clone() }
            }
            GitRead { command } => {
                // Answered from `poll_events` when the Git thread is done.
                self.git_reads
                    .jobs
                    .try_send(GitJob {
                        identity: request.identity.clone(),
                        request_id: request.request_id,
                        command: *command,
                        directory: self.filesystem.root().to_path_buf(),
                    })
                    .map_err(|error| match error {
                        // The queue is full: back-pressure, ask again.
                        TrySendError::Full(_) => ErrorCode::Limit,
                        // The Git thread is gone: say so, do not look busy.
                        TrySendError::Disconnected(_) => ErrorCode::Unavailable,
                    })?;
                return Ok(vec![]);
            }
            SessionState => self
                .integration
                .as_ref()
                .ok_or(ErrorCode::Unavailable)?
                .state()?,
            GrokState => self.grok.state(),
            ClaudeState => {
                self.claude_subscribed = true;
                // Answered only after the completing resnapshot frame.
                if !self
                    .attachment
                    .as_ref()
                    .is_some_and(|attachment| attachment.snapshot_closed)
                {
                    self.claude_state_pending = Some(request.clone());
                    return Ok(vec![]);
                }
                self.claude.state()
            }
            ClaudeTrigger => {
                self.claude.trigger();
                Ack
            }
            Focus { session_id } => {
                self.integration
                    .as_mut()
                    .ok_or(ErrorCode::Unavailable)?
                    .focus(request.request_id, session_id)?;
                return Ok(vec![]);
            }
            TerminalInput { data } => {
                self.session.write(data.as_bytes())?;
                Ack
            }
            TerminalResize { rows, cols } => {
                // A real resize signals the child itself.
                self.session.resize(*rows, *cols)?;
                self.pty_size = (*rows, *cols);
                self.redraw = None;
                Ack
            }
            StartShell {
                directory,
                fallback,
            } => ShellStarted {
                path: self.start_follow_up_shell(directory, fallback)?,
            },
            UploadBegin { upload_id, total } => {
                self.uploads.begin(*upload_id, *total)?;
                Ack
            }
            UploadChunk {
                upload_id,
                offset,
                data,
            } => {
                self.uploads.chunk(*upload_id, *offset, data)?;
                Ack
            }
            UploadCommit { upload_id } => UploadResult {
                upload_id: *upload_id,
                path: self.uploads.commit(*upload_id)?,
            },
            UploadAbort { upload_id } => {
                self.uploads.abort(*upload_id);
                Ack
            }
            Cancel { request_id } => {
                if let Some(attachment) = &mut self.attachment {
                    if attachment.watch_request == Some(*request_id) {
                        attachment.watch_request = None;
                        self.filesystem.cancel_watch();
                    }
                }
                self.filesystem.cancel_watch_request(*request_id);
                if let Some(integration) = &mut self.integration {
                    integration.cancel(*request_id);
                }
                Ack
            }
            _ => return Err(ErrorCode::InvalidRequest),
        };
        Ok(vec![reply])
    }

    /// Start the remote interactive shell in this terminal after the launch
    /// child exited, keeping the screen and scrollback it left. The shell runs
    /// at the terminal's current size in `directory`, or in `fallback` when
    /// that is not a usable directory on this server; the directory it starts
    /// in is returned. The child's last output and the reset of the modes it
    /// may have left set reach the client before the shell's own output.
    fn start_follow_up_shell(
        &mut self,
        directory: &RemotePath,
        fallback: &RemotePath,
    ) -> Result<RemotePath, ErrorCode> {
        // Only a child known to have ended is replaced: a shell already
        // running here, whether this request or `Terminal SSH` started it,
        // stays, and so does a child whose state the monitor lost.
        if !self.session.has_ended() {
            return Err(ErrorCode::InvalidRequest);
        }
        let (program, args, extra) = self
            .follow_up
            .as_ref()
            .map(|shell| (shell.program.clone(), shell.args.clone(), shell.env.clone()))
            .ok_or(ErrorCode::Unavailable)?;
        let (path, cwd) = [directory, fallback]
            .into_iter()
            .find_map(|candidate| {
                let native = crate::filesystem::native(candidate).ok()?;
                fs::metadata(&native)
                    .is_ok_and(|metadata| metadata.is_dir())
                    .then(|| (candidate.clone(), native))
            })
            .ok_or(ErrorCode::Unavailable)?;
        // The environment a fresh `Terminal SSH` shell gets: the launch's own,
        // with what a shell launch has beyond it.
        let mut env = self.launch_env.clone();
        for (key, value) in extra {
            set_var(&mut env, key, value);
        }
        // A shell launch owns the OpenCode integration; start the one a Grok
        // Build or Claude Code launch did not. A failed start leaves the
        // terminal without it, as at a shell launch.
        if self.integration.is_none() {
            if let Some(start) = self
                .follow_up
                .as_mut()
                .and_then(|shell| shell.integration.take())
            {
                self.integration = start();
            }
        }
        if let Some(integration) = &self.integration {
            let (key, value) = integration.environment();
            set_var(&mut env, key, value);
        }
        // The Claude Code spool binding needs the tab's spool; create it when
        // the launch had none, or drop the binding, as at a shell launch.
        let binding = claude::spool_path(&self.runtime);
        if self._claude_spool.is_none()
            && env
                .iter()
                .any(|(key, value)| key == claude::CLAUDE_SPOOL_ENV && Path::new(value) == binding)
        {
            match ClaudeSpool::create(&self.runtime) {
                Ok(spool) => {
                    // Bound now, so a start that fails below and is retried
                    // finds the tracker reading it.
                    self._claude_spool = Some(spool);
                    self.claude.bind_spool(binding);
                }
                Err(_) => env.retain(|(key, _)| key != claude::CLAUDE_SPOOL_ENV),
            }
        }
        let mut request = made_platform::PtyRequest::new(program.as_os_str(), &cwd);
        request.args = args;
        request.env = env;
        let plan = ShellLaunchPlan::with_cwd_sink(
            request,
            self.companion.clone(),
            self.cwd_sink.path().into(),
        );
        let launch = ChildLaunch {
            program,
            args: plan.request().args.clone(),
            cwd: cwd.clone(),
            env: plan.request().env.clone(),
        };
        let child_bytes = serde_json::to_vec(&launch).map_err(io_error)?;
        publish_private(&self.runtime, "child-shell.json", &child_bytes)?;
        let options = PtyOptions::new(&cwd)
            .with_shell(&self.companion)
            .with_args([
                OsString::from("--child"),
                self.runtime.join("child-shell.json").into_os_string(),
            ])
            .with_size(PtySize {
                rows: self.pty_size.0,
                cols: self.pty_size.1,
            });
        let pieces = self
            .session
            .restart(&made_platform::host_platform(), options)?;
        // What the old child printed last may still wait for the client.
        if let Some(bytes) = self
            .attachment
            .as_mut()
            .and_then(|attachment| attachment.pending_relay.take())
        {
            self.relay_tail.push_back(bytes);
        }
        self.relay_tail.extend(pieces);
        self.shell_plan = Some(plan);
        self.exit_reported = false;
        self.shell_target = true;
        self.shell_cwd = Some(path.clone());
        self.redraw = None;
        // Output and state of the exited child belong to it alone.
        self.grok.rebind(self.session.pid(), false);
        self.claude.rebind(self.session.pid(), false);
        Ok(path)
    }

    fn step_redraw_nudge(&mut self) {
        let Some(nudge) = &mut self.redraw else {
            return;
        };
        if self.attachment.is_none() {
            // The frontend left between the two sizes: do not strand the child.
            if nudge.shrunk {
                let (rows, cols) = self.pty_size;
                let _ = self.session.resize_pty_only(rows, cols);
            }
            self.redraw = None;
            return;
        }
        if Instant::now() < nudge.due {
            return;
        }
        let (rows, cols) = self.pty_size;
        if nudge.shrunk {
            let _ = self.session.resize_pty_only(rows, cols);
            self.redraw = None;
            return;
        }
        nudge.shrunk = true;
        nudge.due = Instant::now() + NUDGE_STEP;
        let transient = if rows > 1 { rows - 1 } else { rows + 1 };
        if self.session.resize_pty_only(transient, cols).is_err() {
            self.redraw = None;
        }
    }

    /// Forward this tab's URL requests to the attached client. A request is
    /// acknowledged only once its frame is queued for an attachment whose
    /// resnapshot completed; while detached it is refused and dropped, never
    /// queued for a later attach, so nothing is replayed.
    fn poll_url_requests(&mut self) {
        let Some((sink, token)) = self.url_open.as_mut() else {
            return;
        };
        let requests = sink.poll();
        let token = token.clone();
        for request in requests {
            let attached = self
                .attachment
                .as_ref()
                .is_some_and(|attachment| attachment.snapshot_closed);
            if !token.matches(request.token()) || !attached {
                request.reject();
                continue;
            }
            let mut gate =
                std::mem::replace(&mut self.url_gate, UrlRateGate::new(URL_REQUEST_WINDOW));
            let url = request.url().clone();
            let acked = url_request_acked(&mut gate, &url, Instant::now(), || {
                self.forward_url(url.as_str().to_owned())
            });
            self.url_gate = gate;
            if acked {
                request.accept();
            } else {
                request.reject();
            }
        }
    }

    fn forward_url(&mut self, url: String) -> bool {
        let Some(attachment) = self.attachment.as_mut() else {
            return false;
        };
        let frame = WireEnvelope::new(
            attachment.identity.clone(),
            attachment.request,
            Operation::OpenUrlRequest { url },
        );
        match attachment.output.try_send(frame) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => false,
            Err(TrySendError::Disconnected(_)) => {
                self.detach_frontend();
                false
            }
        }
    }

    fn poll_events(&mut self) {
        self.step_redraw_nudge();
        self.poll_url_requests();
        let previous_cwd = self.shell_cwd.clone();
        if let Ok(Some(bytes)) = self.cwd_sink.poll() {
            if let Ok(path) = RemotePath::new(bytes) {
                if let Ok(native) = crate::filesystem::native(&path) {
                    if let Ok(guard) = made_explorer::paths::PathGuard::new(native) {
                        self.shell_cwd = crate::filesystem::remote(guard.root()).ok();
                        self.cwd_reported = self.cwd_reported || self.shell_cwd.is_some();
                    }
                }
            }
        }
        // Bounded Grok ingestion runs every pass, attached or not; pushes are
        // dropped by `enqueue` while detached and rebuilt after re-attach.
        let child_alive = self.session.is_running();
        let grok_pushes = self.grok.tick(Instant::now(), child_alive);
        if let Some(attachment) = &self.attachment {
            let identity = attachment.identity.clone();
            let request = attachment.request;
            for operation in grok_pushes {
                self.enqueue(WireEnvelope::new(identity.clone(), request, operation));
            }
        }
        self.flush_attachment();
        // Claude Code state is kept while detached and reaches the client only
        // after the completing resnapshot frame, as a fresh snapshot rather
        // than a replay of what happened meanwhile.
        let deliver = self.claude_subscribed
            && self
                .attachment
                .as_ref()
                .is_some_and(|attachment| attachment.snapshot_closed);
        let claude_pushes = self.claude.tick(Instant::now(), deliver);
        if deliver {
            if let Some(attachment) = &self.attachment {
                let identity = attachment.identity.clone();
                let request = attachment.request;
                for operation in claude_pushes {
                    self.enqueue(WireEnvelope::new(identity.clone(), request, operation));
                }
            }
            // The deferred reply follows this pass's pushes on the same lane
            // and carries a newer revision than any of them.
            if let Some(request) = self.claude_state_pending.take() {
                let state = self.claude.state();
                self.reply(&request, state);
            }
        }
        self.observe_pty();
        if self.attachment.is_none() {
            if let Some(integration) = &mut self.integration {
                integration.poll();
            }
            return;
        }
        let restore_cwd = self.cwd_reported
            && self.attachment.as_mut().is_some_and(|attachment| {
                attachment.snapshot_closed && std::mem::take(&mut attachment.cwd_pending)
            });
        if previous_cwd != self.shell_cwd || restore_cwd {
            if let Some(path) = self.shell_cwd.clone() {
                if let Some(attachment) = &self.attachment {
                    let frame = WireEnvelope::new(
                        attachment.identity.clone(),
                        attachment.request,
                        Operation::ShellCwd { path },
                    );
                    self.enqueue(frame);
                }
            }
        }
        let change = self.filesystem.poll_change();
        if let Some(operation) = change {
            if let Some(attachment) = &self.attachment {
                if let Some(id) = attachment.watch_request {
                    let frame = WireEnvelope::new(attachment.identity.clone(), id, operation);
                    self.enqueue(frame);
                }
            }
        }
        // Git reads the Git thread finished. One of a previous attachment
        // (another epoch) is dropped, never sent to the current client.
        while let Ok(done) = self.git_reads.results.try_recv() {
            let Some(attachment) = &self.attachment else {
                continue;
            };
            if !transport_matches(&done.identity, &attachment.identity) {
                continue;
            }
            let operations = match done.outcome {
                Ok(bytes) => chunk_git(done.command, &bytes),
                Err(code) => vec![Operation::Error { code }],
            };
            for operation in operations {
                self.enqueue(WireEnvelope::new(
                    done.identity.clone(),
                    done.request_id,
                    operation,
                ));
            }
        }
        // `WatchDir` registrations that finished: acknowledged, or refused.
        for (id, operation) in self.filesystem.poll_watch_results() {
            if let Some(attachment) = &self.attachment {
                let frame = WireEnvelope::new(attachment.identity.clone(), id, operation);
                self.enqueue(frame);
            }
        }
        // Per-directory and Git notifications of the `WatchDir` subscriptions.
        // A full output queue drops one; the client's periodic refresh is the
        // backstop for any notification that does not arrive.
        for (id, operation) in self.filesystem.poll_changes() {
            if let Some(attachment) = &self.attachment {
                let frame = WireEnvelope::new(attachment.identity.clone(), id, operation);
                self.enqueue(frame);
            }
        }
        if let Some(integration) = &mut self.integration {
            let events = integration.poll();
            if let Some(attachment) = &self.attachment {
                let identity = attachment.identity.clone();
                let request = attachment.request;
                for (id, operation) in events {
                    let frame =
                        WireEnvelope::new(identity.clone(), id.unwrap_or(request), operation);
                    self.enqueue(frame);
                }
            }
        }
    }

    fn flush_attachment(&mut self) {
        let Some(attachment) = self.attachment.as_mut() else {
            return;
        };
        if !attachment.pending_control.is_empty() {
            let pending = std::mem::take(&mut attachment.pending_control);
            let output = OutputQueue {
                control: attachment.output.control.clone(),
                data: attachment.output.data.clone(),
            };
            let mut blocked = false;
            let mut rest = Vec::new();
            for frame in pending {
                if blocked {
                    rest.push(frame);
                    continue;
                }
                match output.try_send(frame) {
                    Ok(()) => {}
                    Err(TrySendError::Full(frame)) => {
                        rest.push(frame);
                        blocked = true;
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        self.detach_frontend();
                        return;
                    }
                }
            }
            if let Some(attachment) = self.attachment.as_mut() {
                attachment.pending_control = rest;
            }
        }
        self.flush_snapshot();
    }

    fn flush_snapshot(&mut self) {
        let Some(attachment) = self.attachment.as_mut() else {
            return;
        };
        while !attachment.snapshot_closed {
            let Some((operation, consumed, closed)) =
                next_snapshot_frame(&attachment.snapshot_rest, false)
            else {
                return;
            };
            let frame =
                WireEnvelope::new(attachment.identity.clone(), attachment.request, operation);
            match attachment.output.try_send(frame) {
                Ok(()) => {
                    if consumed > 0 {
                        attachment.snapshot_rest.drain(..consumed);
                    }
                    if closed {
                        attachment.snapshot_closed = true;
                    }
                }
                Err(TrySendError::Full(_)) => return,
                Err(TrySendError::Disconnected(_)) => {
                    self.detach_frontend();
                    return;
                }
            }
        }
    }

    fn observe_pty(&mut self) {
        const MAX_PTY_EVENTS_PER_TICK: usize = 256;
        let snapshot_pending = self
            .attachment
            .as_ref()
            .is_some_and(|attachment| !attachment.snapshot_closed);
        if snapshot_pending {
            return;
        }
        // A single event per main-loop tick puts every later keystroke echo
        // behind the shell's startup output and command output. Drain the
        // reader queue in one pass; the bounded output queue is the
        // backpressure point.
        for _ in 0..MAX_PTY_EVENTS_PER_TICK {
            if let Some(bytes) = self
                .attachment
                .as_mut()
                .and_then(|attachment| attachment.pending_relay.take())
            {
                if !self.relay_terminal(bytes.clone()) {
                    if let Some(attachment) = self.attachment.as_mut() {
                        attachment.pending_relay = Some(bytes);
                    }
                    return;
                }
            }
            // The replaced child's last output and the mode reset go first.
            if let Some(bytes) = self.relay_tail.pop_front() {
                if self.attachment.is_some() && !self.relay_terminal(bytes.clone()) {
                    self.relay_tail.push_front(bytes);
                    return;
                }
                continue;
            }
            let Some(event) = self.session.poll() else {
                return;
            };
            match event {
                PtyEvent::Data(bytes) => {
                    self.session.observe(&bytes);
                    if self.attachment.is_some() && !self.relay_terminal(bytes.clone()) {
                        if let Some(attachment) = self.attachment.as_mut() {
                            attachment.pending_relay = Some(bytes);
                        }
                        return;
                    }
                }
                PtyEvent::Exited(status) => {
                    // A reaped child reports its exit status on every poll;
                    // the client learns of it once per attachment.
                    if !self.exit_reported {
                        if let Some(attachment) = &self.attachment {
                            let frame = WireEnvelope::new(
                                attachment.identity.clone(),
                                attachment.request,
                                Operation::TerminalExit { code: status.code },
                            );
                            self.exit_reported = self.enqueue(frame);
                        }
                    }
                    return;
                }
                _ => {}
            }
        }
    }

    fn relay_terminal(&mut self, bytes: Vec<u8>) -> bool {
        let Some(attachment) = self.attachment.as_ref() else {
            return true;
        };
        let Some(operation) = live_terminal_output(&bytes) else {
            return true;
        };
        let frame = WireEnvelope::new(attachment.identity.clone(), attachment.request, operation);
        match attachment.output.try_send(frame) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => false,
            Err(TrySendError::Disconnected(_)) => {
                self.detach_frontend();
                true
            }
        }
    }
}

fn writer_loop(
    mut writer: TcpStream,
    control_rx: Receiver<WireEnvelope>,
    data_rx: Receiver<WireEnvelope>,
) {
    loop {
        let frame = match control_rx.try_recv() {
            Ok(frame) => frame,
            Err(TryRecvError::Disconnected) => match data_rx.try_recv() {
                Ok(frame) => frame,
                Err(TryRecvError::Empty) => {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(TryRecvError::Disconnected) => break,
            },
            Err(TryRecvError::Empty) => match data_rx.try_recv() {
                Ok(frame) => frame,
                Err(TryRecvError::Empty) => {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(TryRecvError::Disconnected) => {
                    match control_rx.recv_timeout(Duration::from_millis(5)) {
                        Ok(frame) => frame,
                        Err(_) => break,
                    }
                }
            },
        };
        if write_frame_retry(&mut writer, &frame).is_err() {
            break;
        }
    }
}

fn write_frame_retry(writer: &mut TcpStream, frame: &WireEnvelope) -> io::Result<()> {
    let bytes =
        encode(frame).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid frame"))?;
    let mut offset = 0;
    while offset < bytes.len() {
        match writer.write(&bytes[offset..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "attachment closed",
                ))
            }
            Ok(n) => offset += n,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                ) =>
            {
                // Backpressure is not attachment death. A broken pipe still returns.
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error),
        }
    }
    writer.flush()
}

fn current_target() -> Result<RemoteTarget, ErrorCode> {
    match made_platform::remote_target().map_err(io_error)?.as_str() {
        "x86_64-unknown-linux-gnu" => Ok(RemoteTarget::LinuxX64),
        "aarch64-unknown-linux-gnu" => Ok(RemoteTarget::LinuxArm64),
        "x86_64-apple-darwin" => Ok(RemoteTarget::MacosX64),
        "aarch64-apple-darwin" => Ok(RemoteTarget::MacosArm64),
        _ => Err(ErrorCode::Unsupported),
    }
}

fn chunk_git(command: crate::protocol::GitRead, bytes: &[u8]) -> Vec<Operation> {
    let mut results = Vec::new();
    for (index, chunk) in bytes.chunks(MAX_CHUNK_BYTES).enumerate() {
        let Ok(data) = Chunk::new(chunk.to_vec()) else {
            break;
        };
        results.push(Operation::GitResult {
            command,
            offset: (index * MAX_CHUNK_BYTES) as u64,
            data,
            done: (index + 1) * MAX_CHUNK_BYTES >= bytes.len(),
        });
    }
    if results.is_empty() {
        if let Ok(data) = Chunk::new(Vec::new()) {
            results.push(Operation::GitResult {
                command,
                offset: 0,
                data,
                done: true,
            });
        }
    }
    results
}

pub fn read_envelope(reader: &mut impl Read) -> io::Result<WireEnvelope> {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix)?;
    let n = u32::from_be_bytes(prefix) as usize;
    if n == 0 || n > MAX_FRAME_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame limit"));
    }
    let mut payload = vec![0; n];
    reader.read_exact(&mut payload)?;
    let frame: WireEnvelope = serde_json::from_slice(&payload)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid frame"))?;
    frame
        .validate()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid frame"))?;
    Ok(frame)
}

pub fn write_envelope(writer: &mut impl Write, frame: &WireEnvelope) -> io::Result<()> {
    writer.write_all(
        &encode(frame).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid frame"))?,
    )?;
    writer.flush()
}

#[cfg(test)]
mod url_request_tests {
    use super::*;
    use made_platform::open_url::validate_url;

    #[test]
    fn a_failed_delivery_is_not_acknowledged_on_retry() {
        let mut gate = UrlRateGate::new(URL_REQUEST_WINDOW);
        let url = validate_url("https://example.com/login").unwrap();
        let start = Instant::now();
        // The output queue was full: refused, and nothing recorded.
        assert!(!url_request_acked(&mut gate, &url, start, || false));
        // The retry inside the window must be delivered, not coalesced.
        let mut delivered = false;
        let retry = start + Duration::from_millis(200);
        assert!(url_request_acked(&mut gate, &url, retry, || {
            delivered = true;
            true
        }));
        assert!(
            delivered,
            "the retry was delivered, not acked as a duplicate"
        );
        // A real repeat of a delivered request is coalesced without delivery.
        assert!(url_request_acked(
            &mut gate,
            &url,
            retry + Duration::from_millis(100),
            || panic!("a duplicate is not delivered again")
        ));
        let other = validate_url("https://other.example/").unwrap();
        assert!(!url_request_acked(
            &mut gate,
            &other,
            retry + Duration::from_millis(100),
            || panic!("a limited request is not delivered")
        ));
    }
}
