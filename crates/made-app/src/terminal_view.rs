//! Terminal pane view: grid rendering, input, selection, copy/paste, title,
//! focus reporting, mouse reporting, and IME (Phase 1, tasks 1.16-1.17).
//!
//! This module owns no VT parsing and no OS-specific code. Output arrives from
//! `PtyProcess`, is parsed by `TerminalCore`, and is drawn by the dedicated
//! [`crate::render::GridRenderer`]. Every key, mouse, paste, focus, and IME byte
//! is encoded by `made_terminal::InputEncoder`.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Event, Key, Rect, Sense, Ui, Vec2};
use made_platform::{ExitStatus, ImeComposition, ImeEvent, Platform, PtySize};
use made_terminal::{
    InputEncoder, Key as TerminalKey, Modifiers as TerminalModifiers, MouseButton, MouseEvent,
    MouseEventKind, MouseMode, PtyError, PtyOptions, PtyProcess, TerminalCore, TerminalDamage,
    TerminalSize, TerminalSnapshot,
};

use crate::diagnostics::{DiagnosticLog, LogLevel};
use crate::metrics::Metrics;
use crate::render::{
    cursor_blink_on, cursor_blink_repaint_after, paint_grid, terminal_content_layout, CellMetrics,
    CellPoint, GridPalette, GridRenderer, PaintOptions, SelectionRange, SCROLLBAR_WIDTH,
    TERMINAL_PADDING,
};

/// Maximum PTY events drained per frame.
const MAX_PTY_EVENTS_PER_FRAME: usize = 128;

/// Maximum PTY bytes parsed per frame. Input and rendering always take priority
/// over a burst of background output.
const MAX_PTY_BYTES_PER_FRAME: usize = 128 * 1024;

/// Maximum scrollback lines retained by the terminal core.
const SCROLLBACK_LINES: usize = 10_000;

/// One paste may not exceed 1 MiB.
const MAX_PASTE_BYTES: usize = 1024 * 1024;

/// Maximum application-side pending input before new input is refused.
const MAX_PENDING_INPUT_BYTES: usize = 4 * 1024 * 1024;

/// One retry batch. The PTY adapter atomically enqueues whole chunks.
const INPUT_RETRY_BATCH: usize = 16 * 1024;

/// Longest visible-input suffix searched for in a painted terminal row.
///
/// A short suffix fits on one row even when the user typed a long command or
/// pasted a block, so damage-row matching stays cheap and reliable.
const INPUT_ECHO_MAX_CHARS: usize = 16;

/// Input with no painted echo is not measurable forever. This matches the
/// previous stuck-input timeout and keeps the marker bounded.
const INPUT_ECHO_TIMEOUT: Duration = Duration::from_millis(500);

/// User input waiting for its echo to be painted.
#[derive(Debug, Clone)]
struct InputEcho {
    started_at: Instant,
    text: String,
}

impl InputEcho {
    fn new(started_at: Instant, text: String) -> Self {
        Self { started_at, text }
    }

    fn is_measurable(&self) -> bool {
        // A single character can already be present in unrelated output, so it
        // cannot prove that the input echo was painted. Longer text is stable
        // under normal interactive echo and paste. Control-only input carries
        // no searchable echo and is therefore not measured.
        self.text.chars().count() >= 2
    }
}

#[derive(Debug, Default)]
struct PendingInput {
    bytes: VecDeque<u8>,
}

impl PendingInput {
    fn enqueue(&mut self, bytes: &[u8]) -> bool {
        if self.bytes.len().saturating_add(bytes.len()) > MAX_PENDING_INPUT_BYTES {
            return false;
        }
        self.bytes.extend(bytes);
        true
    }

    fn front_batch(&mut self) -> &[u8] {
        let contiguous = self.bytes.make_contiguous();
        &contiguous[..contiguous.len().min(INPUT_RETRY_BATCH)]
    }

    fn consume(&mut self, count: usize) {
        self.bytes.drain(..count.min(self.bytes.len()));
    }

    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn clear(&mut self) {
        self.bytes.clear();
    }
}

/// Initial grid used before the first layout pass.
pub const DEFAULT_ROWS: usize = 36;
/// Initial grid used before the first layout pass.
pub const DEFAULT_COLS: usize = 120;

/// Events produced by the terminal view for the workbench to act on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalUiOutput {
    /// The user asked to relaunch the shell.
    pub restart_requested: bool,
    /// The pane wrote input to the child this frame.
    pub input_sent: bool,
    /// A short, safe notice for the status area.
    pub notice: Option<String>,
    /// F6 requested moving keyboard focus out of the terminal to workspace chrome.
    pub focus_exit_requested: bool,
}

/// One running terminal pane.
pub struct TerminalPane {
    core: TerminalCore,
    pty: Option<PtyProcess>,
    options: PtyOptions,
    encoder: InputEncoder,
    renderer: GridRenderer,
    font_size: f32,
    title: Option<String>,
    cwd: Option<PathBuf>,
    exit: Option<ExitStatus>,
    monitor_error: Option<String>,
    read_error: Option<String>,
    eof: bool,
    focused: bool,
    blink_epoch: Instant,
    selecting: bool,
    selection: Option<SelectionRange>,
    /// Cell where the current local-selection press began, before it is committed.
    selection_anchor: Option<CellPoint>,
    mouse_button_down: Option<MouseButton>,
    ime: ImeComposition,
    ime_preedit: String,
    input_echo: Option<InputEcho>,
    recent_typed_text: String,
    cwd_changed_at: Option<Instant>,
    pending_input: PendingInput,
    input_notice: Option<String>,
    focus_on_next_frame: bool,
    wrote_input: bool,
    palette: GridPalette,
    visible: bool,
    focus_id: Option<egui::Id>,
    reset_focus: bool,
    grid_paint_count: u64,
    last_grid_paint_visible: bool,
    output_bytes: u64,
}

impl std::fmt::Debug for TerminalPane {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TerminalPane")
            .field("pid", &self.pid())
            .field("size", &self.core.size())
            .field("running", &self.is_running())
            .field("exit", &self.exit)
            .field("monitor_error", &self.monitor_error)
            .finish_non_exhaustive()
    }
}

impl TerminalPane {
    /// Spawn a shell for `options` and size the terminal core to match.
    pub fn spawn(platform: &dyn Platform, options: PtyOptions) -> Result<Self, PtyError> {
        let size = TerminalSize {
            rows: usize::from(options.size.rows.max(1)),
            cols: usize::from(options.size.cols.max(2)),
        };
        let pty = PtyProcess::spawn(platform, options.clone())?;
        let core = TerminalCore::with_size(size, SCROLLBACK_LINES);
        let encoder = InputEncoder::new(core.input_modes());
        let mut pane = Self {
            core,
            pty: Some(pty),
            options,
            encoder,
            renderer: GridRenderer::new(CellMetrics::default()),
            font_size: crate::render::DEFAULT_FONT_SIZE,
            title: None,
            cwd: None,
            exit: None,
            monitor_error: None,
            read_error: None,
            eof: false,
            focused: false,
            blink_epoch: Instant::now(),
            selecting: false,
            selection: None,
            selection_anchor: None,
            mouse_button_down: None,
            ime: ImeComposition::new(),
            ime_preedit: String::new(),
            input_echo: None,
            recent_typed_text: String::new(),
            cwd_changed_at: None,
            pending_input: PendingInput::default(),
            input_notice: None,
            focus_on_next_frame: false,
            wrote_input: false,
            palette: crate::theme::terminal_palette(),
            visible: true,
            focus_id: None,
            reset_focus: false,
            grid_paint_count: 0,
            last_grid_paint_visible: false,
            output_bytes: 0,
        };
        pane.encoder.set_modes(pane.core.input_modes());
        pane.core.clear_selection();
        Ok(pane)
    }

    /// Current child process id.
    pub fn pid(&self) -> Option<u32> {
        self.pty.as_ref().and_then(PtyProcess::pid)
    }

    /// Whether the child is known to be running.
    pub fn is_running(&self) -> bool {
        self.pty
            .as_ref()
            .map(|pty| pty.is_running())
            .unwrap_or(false)
    }

    /// Whether a child exists at all.
    pub fn has_child(&self) -> bool {
        self.pty.is_some()
    }

    /// Current terminal grid size.
    pub fn size(&self) -> TerminalSize {
        self.core.size()
    }

    /// PTY options used for relaunch. Environment values are never printed.
    pub fn options(&self) -> &PtyOptions {
        &self.options
    }

    /// Current terminal title (untrusted display text).
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Whether the child is currently using the VT alternate screen.
    pub fn alternate_screen(&self) -> bool {
        self.core.alternate_screen()
    }

    /// Shell working directory reported by OSC 7, when available.
    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    /// Take a changed cwd together with the instant the report was parsed.
    ///
    /// The timestamp is captured when the PTY pump consumes the OSC 7 report,
    /// not when the workbench later happens to service its tick. Carrying it
    /// with the value keeps cwd-to-explorer latency honest under output load.
    pub fn take_cwd_change(&mut self) -> Option<(PathBuf, Instant)> {
        let reported_at = self.cwd_changed_at.take()?;
        self.cwd.clone().map(|cwd| (cwd, reported_at))
    }

    /// Shell cwd support and fallback reason for Diagnostics.
    pub fn shell_cwd_capability(&self) -> Option<&made_platform::ShellCwdCapability> {
        self.pty.as_ref().map(PtyProcess::shell_cwd_capability)
    }

    /// App-owned bootstrap location, if this shell uses one.
    pub fn bootstrap_directory(&self) -> Option<&Path> {
        self.pty.as_ref().and_then(PtyProcess::bootstrap_directory)
    }

    /// Actual grid paints, excluding hidden runtime polling.
    pub fn grid_paint_count(&self) -> u64 {
        self.grid_paint_count
    }

    /// Whether the most recent actual grid paint happened while the pane was
    /// visible. Surface transitions inside one pass (for example File ->
    /// preview after the terminal was painted) must not be counted as hidden
    /// work.
    pub fn last_grid_paint_visible(&self) -> bool {
        self.last_grid_paint_visible
    }
    /// Output consumed through the bounded PTY pump.
    pub fn output_bytes(&self) -> u64 {
        self.output_bytes
    }
    /// Last scoped focus identity allocated by the visible UI.
    pub fn focus_id(&self) -> Option<egui::Id> {
        self.focus_id
    }
    /// Whether the visible terminal currently owns keyboard/IME input.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Reset UI-only focus, drag and composition on visibility transitions.
    pub fn set_visible(&mut self, visible: bool) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.reset_focus = true;
        self.focus_on_next_frame = visible;
        self.selecting = false;
        self.selection_anchor = None;
        self.mouse_button_down = None;
        self.ime.reset();
        self.ime_preedit.clear();
    }

    /// Apply focus-out even when the terminal is not being painted.
    pub fn sync_visibility(&mut self, ctx: &egui::Context, log: &mut DiagnosticLog) {
        if !std::mem::take(&mut self.reset_focus) {
            return;
        }
        if let Some(id) = self.focus_id {
            ctx.memory_mut(|memory| memory.surrender_focus(id));
        }
        if self.focused {
            self.focused = false;
            let bytes = self.encoder.encode_focus(false);
            self.write(&bytes, log);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::IMEAllowed(false));
    }

    /// Last child exit status.
    pub fn exit_status(&self) -> Option<&ExitStatus> {
        self.exit.as_ref()
    }

    /// Monitor-thread failure, distinct from a normal exit.
    pub fn monitor_error(&self) -> Option<&str> {
        self.monitor_error.as_deref()
    }

    /// Report a one-line shell state for the status area.
    pub fn shell_status_label(&self) -> String {
        if let Some(error) = self.pty.as_ref().and_then(PtyProcess::writer_error) {
            return format!("terminal input failed: {error}");
        }
        if let Some(error) = &self.monitor_error {
            return format!("shell monitor failed: {error}");
        }
        if let Some(error) = &self.read_error {
            return format!("shell output failed: {error}");
        }
        if let Some(exit) = &self.exit {
            return format!("shell exited ({exit})");
        }
        if self.eof {
            return "shell output closed".to_owned();
        }
        if self.has_child() {
            return "shell running".to_owned();
        }
        "shell not running".to_owned()
    }

    /// Capture a cwd change immediately after the PTY chunk that carried the
    /// OSC 7 report is parsed. The workbench carries this timestamp through to
    /// the first painted explorer frame.
    fn note_cwd_after_feed(&mut self) {
        let cwd = self.core.cwd().map(Path::to_path_buf);
        if cwd != self.cwd {
            self.cwd_changed_at = Some(Instant::now());
            self.cwd = cwd;
        }
    }

    /// Drain PTY output into the terminal core. Returns whether anything that
    /// affects rendering changed.
    pub fn pump(&mut self, log: &mut DiagnosticLog) -> bool {
        if self.exit.is_some() || self.monitor_error.is_some() || self.read_error.is_some() {
            if !self.pending_input.is_empty() {
                self.pending_input.clear();
                self.input_notice =
                    Some("Shell is no longer running; input was not sent.".to_owned());
            }
            return false;
        }
        self.flush_pending_input(log);
        let mut rendered = false;
        let mut bytes = 0_usize;
        let mut events = 0_usize;

        while events < MAX_PTY_EVENTS_PER_FRAME && bytes < MAX_PTY_BYTES_PER_FRAME {
            let Some(event) = self.pty.as_ref().and_then(PtyProcess::poll) else {
                break;
            };
            events += 1;
            match event {
                made_terminal::PtyEvent::Data(data) => {
                    bytes = bytes.saturating_add(data.len());
                    self.output_bytes = self.output_bytes.saturating_add(data.len() as u64);
                    self.core.feed(&data);
                    self.note_cwd_after_feed();
                    rendered = true;
                }
                made_terminal::PtyEvent::Eof => {
                    self.eof = true;
                    break;
                }
                made_terminal::PtyEvent::ReadError(message) => {
                    log.record(LogLevel::Warn, format!("terminal read failed: {message}"));
                    self.read_error = Some(message);
                    break;
                }
                made_terminal::PtyEvent::Exited(status) => {
                    log.record(LogLevel::Info, format!("terminal child exited ({status})"));
                    self.exit = Some(status);
                    break;
                }
                made_terminal::PtyEvent::MonitorError(message) => {
                    log.record(
                        LogLevel::Error,
                        format!("terminal child monitor failed: {message}"),
                    );
                    self.monitor_error = Some(message);
                    break;
                }
            }
        }

        let replies = self.core.take_replies();
        if !replies.is_empty() {
            self.write(&replies, log);
        }
        if self.core.take_bell() {
            log.record(LogLevel::Info, "terminal bell");
        }

        self.encoder.set_modes(self.core.input_modes());
        self.title = self.core.title();
        self.note_cwd_after_feed();

        if self.exit.is_some()
            || self.eof
            || self.monitor_error.is_some()
            || self.read_error.is_some()
        {
            self.pending_input.clear();
        }

        rendered
    }

    /// Resize both the emulator and the PTY.
    pub fn resize(&mut self, rows: usize, cols: usize, log: &mut DiagnosticLog) {
        let size = self.core.size();
        if size.rows == rows && size.cols == cols {
            return;
        }
        self.core.resize(rows, cols);
        let clamped = self.core.size();
        if let Some(pty) = self.pty.as_mut() {
            let requested = PtySize {
                rows: clamped.rows.min(u16::MAX as usize) as u16,
                cols: clamped.cols.min(u16::MAX as usize) as u16,
            };
            if let Err(error) = pty.resize(requested.rows, requested.cols) {
                log.record(LogLevel::Warn, format!("terminal resize failed: {error}"));
            }
            self.options.size = requested;
        }
    }

    /// Remember visible user text so input latency can wait for its echo.
    ///
    /// The marker is a short suffix of the most recent printable input. It is
    /// matched only against rows painted in the current pass, so unrelated
    /// background PTY output cannot complete an input-latency measurement.
    fn note_user_input(&mut self, text: &str) {
        let visible: String = text
            .chars()
            .filter(|character| !character.is_control())
            .collect();
        if visible.is_empty() {
            return;
        }
        self.recent_typed_text.push_str(&visible);
        let chars = self.recent_typed_text.chars().count();
        if chars > INPUT_ECHO_MAX_CHARS * 2 {
            self.recent_typed_text = self
                .recent_typed_text
                .chars()
                .skip(chars - INPUT_ECHO_MAX_CHARS)
                .collect();
        }
        let marker: String = {
            let count = self.recent_typed_text.chars().count();
            self.recent_typed_text
                .chars()
                .skip(count.saturating_sub(INPUT_ECHO_MAX_CHARS))
                .collect()
        };
        self.input_echo = Some(InputEcho::new(Instant::now(), marker));
    }

    /// Return the latency for an input echo that is present in the rows painted
    /// in this pass, if any. This is the paint-side completion point for the
    /// input metric; PTY data alone never completes it.
    fn take_painted_input_latency(
        &mut self,
        snapshot: &TerminalSnapshot,
        damage: &TerminalDamage,
    ) -> Option<Duration> {
        let echo = self.input_echo.as_ref()?;
        if !echo.is_measurable() {
            if echo.started_at.elapsed() >= INPUT_ECHO_TIMEOUT {
                self.input_echo = None;
            }
            return None;
        }
        if echo.started_at.elapsed() >= INPUT_ECHO_TIMEOUT {
            self.input_echo = None;
            return None;
        }
        let painted = if damage.full {
            snapshot.text().contains(&echo.text)
        } else {
            damage
                .lines
                .iter()
                .any(|line| snapshot.row_text(line.row).contains(echo.text.as_str()))
        };
        if !painted {
            return None;
        }
        let started = echo.started_at;
        self.input_echo = None;
        Some(started.elapsed())
    }

    /// Queue already encoded bytes and retry the PTY without blocking.
    fn write(&mut self, bytes: &[u8], log: &mut DiagnosticLog) -> bool {
        if bytes.is_empty() {
            return true;
        }
        if !self.pending_input.enqueue(bytes) {
            self.input_notice =
                Some("Input buffer full; some input was not sent to the shell.".to_owned());
            return false;
        }
        self.blink_epoch = Instant::now();
        self.flush_pending_input(log);
        true
    }

    fn flush_pending_input(&mut self, log: &mut DiagnosticLog) {
        loop {
            if self.pending_input.is_empty() {
                return;
            }
            let batch = self.pending_input.front_batch().to_vec();
            let Some(pty) = self.pty.as_mut() else {
                self.pending_input.clear();
                self.input_notice =
                    Some("Shell is no longer running; input was not sent.".to_owned());
                return;
            };
            match pty.write(&batch) {
                Ok(()) => {
                    self.pending_input.consume(batch.len());
                    self.wrote_input = true;
                }
                Err(PtyError::WouldBlock) => return,
                Err(error) => {
                    self.pending_input.clear();
                    self.input_notice = Some("Terminal input failed; see diagnostics.".to_owned());
                    log.record(LogLevel::Warn, format!("terminal input failed: {error}"));
                    return;
                }
            }
        }
    }

    /// Whether application-side input is waiting for PTY queue capacity.
    pub fn has_pending_input(&self) -> bool {
        !self.pending_input.is_empty()
    }

    fn write_paste(&mut self, text: &str, log: &mut DiagnosticLog) -> bool {
        if !paste_allowed(text.len()) {
            self.input_notice = Some(format!(
                "Paste is {} bytes; the limit is 1 MiB and nothing was pasted.",
                text.len()
            ));
            return false;
        }
        self.clear_selection_for_input();
        let bytes = self.encoder.encode_paste(text);
        self.note_user_input(text);
        self.write(&bytes, log)
    }

    /// Encode and write one key press.
    pub fn send_key(
        &mut self,
        key: TerminalKey,
        modifiers: TerminalModifiers,
        log: &mut DiagnosticLog,
    ) {
        if let TerminalKey::Char(character) = key {
            self.note_user_input(&character.to_string());
        }
        self.clear_selection_for_input();
        let bytes = self.encoder.encode_key(key, modifiers);
        self.write(&bytes, log);
    }

    /// Copy the active selection through the platform clipboard.
    ///
    /// Falls back to egui's clipboard when the native clipboard is unavailable
    /// (for example in a headless smoke environment).
    pub fn copy_selection(
        &mut self,
        platform: &dyn Platform,
        ctx: &egui::Context,
        log: &mut DiagnosticLog,
    ) -> Option<String> {
        let text = self.core.selection_text()?;
        if text.is_empty() {
            return None;
        }
        match platform.clipboard() {
            Ok(mut clipboard) => match clipboard.set_text(&text) {
                Ok(()) => {}
                Err(error) => {
                    log.record(LogLevel::Warn, format!("clipboard write failed: {error}"));
                    ctx.copy_text(text.clone());
                }
            },
            Err(error) => {
                log.record(LogLevel::Warn, format!("clipboard unavailable: {error}"));
                ctx.copy_text(text.clone());
            }
        }
        Some(text)
    }

    /// Paste clipboard text as bracketed paste when the application enabled it.
    pub fn paste_clipboard(&mut self, platform: &dyn Platform, log: &mut DiagnosticLog) -> bool {
        let text = match platform.clipboard() {
            Ok(mut clipboard) => match clipboard.get_text() {
                Ok(text) => text,
                Err(error) => {
                    self.input_notice =
                        Some("Clipboard read failed; nothing was pasted.".to_owned());
                    log.record(LogLevel::Warn, format!("clipboard read failed: {error}"));
                    return false;
                }
            },
            Err(error) => {
                self.input_notice = Some("Clipboard unavailable; nothing was pasted.".to_owned());
                log.record(LogLevel::Warn, format!("clipboard unavailable: {error}"));
                return false;
            }
        };
        if text.is_empty() {
            return false;
        }
        self.write_paste(&text, log)
    }

    /// Terminate and reap the child. Idempotent.
    pub fn terminate(&mut self, log: &mut DiagnosticLog) {
        self.pending_input.clear();
        self.input_notice = None;
        let already_exited = self.exit.is_some();
        if let Some(mut pty) = self.pty.take() {
            match pty.terminate() {
                Ok(status) => {
                    let message = if already_exited {
                        format!("terminal child had exited ({status})")
                    } else {
                        format!("terminal child terminated ({status})")
                    };
                    log.record(LogLevel::Info, message);
                    self.exit = Some(status);
                }
                Err(error) => {
                    log.record(
                        LogLevel::Warn,
                        format!("terminal child termination failed: {error}"),
                    );
                }
            }
        }
    }

    /// Relaunch the shell with the same options after it exited.
    pub fn relaunch(
        &mut self,
        platform: &dyn Platform,
        log: &mut DiagnosticLog,
    ) -> Result<(), PtyError> {
        self.terminate(log);
        let pty = PtyProcess::spawn(platform, self.options.clone())?;
        self.pty = Some(pty);
        self.core = TerminalCore::with_size(self.core.size(), SCROLLBACK_LINES);
        self.encoder = InputEncoder::new(self.core.input_modes());
        self.exit = None;
        self.monitor_error = None;
        self.read_error = None;
        self.eof = false;
        self.selection = None;
        self.selecting = false;
        self.selection_anchor = None;
        self.mouse_button_down = None;
        self.focused = false;
        self.ime.reset();
        self.ime_preedit.clear();
        self.pending_input.clear();
        self.input_notice = None;
        self.wrote_input = false;
        self.input_echo = None;
        self.recent_typed_text.clear();
        self.cwd_changed_at = None;
        self.focus_on_next_frame = true;
        self.title = None;
        self.cwd = None;
        self.renderer.invalidate_all();
        log.record(LogLevel::Info, "terminal shell relaunched");
        Ok(())
    }

    /// Draw the terminal and process input.
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        platform: &dyn Platform,
        metrics: &mut Metrics,
        log: &mut DiagnosticLog,
    ) -> TerminalUiOutput {
        let mut output = TerminalUiOutput::default();
        self.sync_visibility(ui.ctx(), log);
        if !self.visible {
            return output;
        }
        self.wrote_input = false;
        let ctx = ui.ctx().clone();

        let measured = CellMetrics::from_context(&ctx, self.font_size);
        self.renderer.set_metrics(measured);
        let cell_metrics = self.renderer.metrics();

        let desired = ui.available_size();
        let (pane_rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
        self.focus_id = Some(response.id);
        if pane_rect.width() <= TERMINAL_PADDING || pane_rect.height() <= TERMINAL_PADDING {
            return output;
        }

        let screen = ui.ctx().content_rect();
        let card_radius =
            crate::theme::Layout::for_window(screen.width(), screen.height()).card_radius as f32;
        let content = terminal_content_layout(
            pane_rect,
            cell_metrics,
            self.core.alternate_screen(),
            card_radius,
        );
        self.resize(content.size.rows, content.size.cols, log);
        let grid_size = self.core.size();
        let grid_rect = content.grid_rect;

        // Focus reporting and keyboard input.
        if std::mem::take(&mut self.focus_on_next_frame) {
            // A relaunched shell must receive focus-in and keyboard input even
            // when the Restart button held focus during the restart.
            response.request_focus();
        }
        let pointer_pressed = ctx.input(|input| {
            input.pointer.any_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|pos| pane_rect.contains(pos))
        });
        if response.clicked() || pointer_pressed {
            response.request_focus();
        } else if ctx.memory(|memory| memory.focused().is_none()) {
            // The terminal is the primary input surface. Give it focus on the
            // first frame and whenever no other widget has claimed focus.
            response.request_focus();
        }
        // Keep navigation keys inside the terminal while it owns keyboard
        // focus. The encoder still receives Tab/arrows/Escape as child input;
        // egui must not move focus to another widget at the same time.
        ctx.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
        let window_focused = ctx.input(|input| input.viewport().focused.unwrap_or(true));
        let focused = response.has_focus() && window_focused;
        if focused != self.focused {
            self.focused = focused;
            if focused {
                self.blink_epoch = Instant::now();
            }
            let bytes = self.encoder.encode_focus(focused);
            self.write(&bytes, log);
            // The terminal is not an egui text widget, so it must opt in to
            // windowing-layer IME events explicitly. Only one surface in Made
            // accepts composed text, so enabling it with terminal focus is safe.
            ctx.send_viewport_cmd(egui::ViewportCommand::IMEAllowed(focused));
            if focused {
                ctx.send_viewport_cmd(egui::ViewportCommand::IMEPurpose(
                    egui::IMEPurpose::Terminal,
                ));
            }
            if !focused {
                self.ime_preedit.clear();
                platform.normalize_ime(&mut self.ime, ImeEvent::Cancel);
            }
        }

        if focused {
            self.process_keyboard(&ctx, platform, log, &mut output);
        }
        if output.input_sent {
            ctx.request_repaint();
        }
        self.process_pointer(
            &ctx,
            &response,
            content.grid_rect,
            cell_metrics,
            platform,
            log,
            &mut output,
        );
        let pane_hovered = response.hovered();
        let snapshot = self.core.snapshot();
        let damage = self.core.take_damage();
        let selection = self.selection;
        let rows = self
            .renderer
            .frame(&snapshot, &damage, selection, &self.palette);
        // The card already paints opaque BG_TERMINAL with the radius in
        // alternate-screen mode. A square body fill would cover the corner
        // cutouts that should show the backdrop.
        let painter = ui.painter_at(pane_rect);
        self.grid_paint_count = self.grid_paint_count.saturating_add(1);
        self.last_grid_paint_visible = self.visible;
        let blink_elapsed = self.blink_epoch.elapsed();
        let cursor_blink_on = cursor_blink_on(snapshot.cursor.blinking, focused, blink_elapsed);
        paint_grid(
            &painter,
            content.origin,
            content.grid_rect,
            rows,
            snapshot.cursor,
            PaintOptions {
                font_size: self.font_size,
                metrics: cell_metrics,
                focused,
                cursor_blink_on,
            },
            &self.palette,
        );
        if let Some(delay) = cursor_blink_repaint_after(snapshot.cursor, focused, blink_elapsed) {
            ctx.request_repaint_after(delay);
        }
        self.process_scrollbar(ui, pane_rect, grid_size, pane_hovered);
        if let Some(elapsed) = self.take_painted_input_latency(&snapshot, &damage) {
            metrics.record_input_latency(elapsed);
        }

        if !self.ime_preedit.is_empty() {
            let cursor = snapshot.cursor;
            let position = content.origin
                + Vec2::new(
                    cursor.col as f32 * cell_metrics.width,
                    cursor.row as f32 * cell_metrics.height,
                );
            painter.rect_filled(
                Rect::from_min_size(
                    position,
                    Vec2::new(
                        (self.ime_preedit.chars().count() as f32) * cell_metrics.width,
                        cell_metrics.height,
                    ),
                ),
                2.0,
                crate::theme::BG_SELECTED,
            );
            painter.text(
                position,
                egui::Align2::LEFT_TOP,
                &self.ime_preedit,
                egui::FontId::monospace(self.font_size),
                crate::theme::TEXT_PRIMARY,
            );
        }

        self.paint_shell_state(ui, grid_rect, &mut output);

        if let Some(notice) = self.input_notice.take() {
            output.notice = Some(match output.notice {
                Some(existing) => format!("{existing}; {notice}"),
                None => notice,
            });
        }
        output.input_sent = self.wrote_input;
        if self.has_pending_input() {
            ctx.request_repaint();
        }

        output
    }

    fn process_keyboard(
        &mut self,
        ctx: &egui::Context,
        platform: &dyn Platform,
        log: &mut DiagnosticLog,
        output: &mut TerminalUiOutput,
    ) {
        let events = ctx.input(|input| input.events.clone());
        let active_modifiers = ctx.input(|input| input.modifiers);
        let mut copied = false;
        let mut pasted = false;
        for event in events {
            match event {
                Event::Text(text) => {
                    // Modified keys are encoded from the logical key path so
                    // Ctrl/Alt/Super are not lost or duplicated.
                    if active_modifiers.ctrl
                        || active_modifiers.alt
                        || active_modifiers.command
                        || active_modifiers.mac_cmd
                    {
                        continue;
                    }
                    self.clear_selection_for_input();
                    let bytes = self.encoder.encode_text(&text);
                    self.note_user_input(&text);
                    self.write(&bytes, log);
                }
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if key == Key::F6 {
                        // Reserved app shortcut: leave terminal focus without
                        // encoding a byte for the child process.
                        output.focus_exit_requested = true;
                        continue;
                    }
                    self.handle_key(
                        ctx,
                        platform,
                        key,
                        modifiers,
                        log,
                        output,
                        &mut copied,
                        &mut pasted,
                    );
                }
                Event::Paste(text) => {
                    if !pasted && !text.is_empty() {
                        pasted = self.write_paste(&text, log);
                    }
                }
                Event::Copy => {
                    if !copied {
                        if let Some(text) = self.copy_selection(platform, ctx, log) {
                            output.notice =
                                Some(format!("copied {} characters", text.chars().count()));
                            copied = true;
                        }
                    }
                }
                Event::Ime(ime) => match ime {
                    egui::ImeEvent::Preedit { text, .. } => {
                        self.ime_preedit = text.clone();
                        platform.normalize_ime(&mut self.ime, ImeEvent::Preedit(text));
                    }
                    egui::ImeEvent::Commit(text) => {
                        self.ime_preedit.clear();
                        if let Some(committed) =
                            platform.normalize_ime(&mut self.ime, ImeEvent::Commit(text.clone()))
                        {
                            self.clear_selection_for_input();
                            let bytes = self.encoder.encode_text(&committed);
                            self.note_user_input(&committed);
                            self.write(&bytes, log);
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_key(
        &mut self,
        ctx: &egui::Context,
        platform: &dyn Platform,
        key: Key,
        modifiers: egui::Modifiers,
        log: &mut DiagnosticLog,
        output: &mut TerminalUiOutput,
        copied: &mut bool,
        pasted: &mut bool,
    ) {
        match route_key(key, modifiers, &self.encoder, self.core.has_selection()) {
            KeyRoute::CopySelection => {
                if !*copied {
                    if let Some(text) = self.copy_selection(platform, ctx, log) {
                        output.notice = Some(format!("copied {} characters", text.chars().count()));
                        *copied = true;
                    }
                }
            }
            KeyRoute::PasteClipboard => {
                if !*pasted {
                    *pasted = self.paste_clipboard(platform, log);
                }
            }
            KeyRoute::Ignore => {}
            KeyRoute::Write(bytes) => {
                self.clear_selection_for_input();
                self.write(&bytes, log);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn process_pointer(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        grid_rect: Rect,
        cell_metrics: CellMetrics,
        platform: &dyn Platform,
        log: &mut DiagnosticLog,
        output: &mut TerminalUiOutput,
    ) {
        if !ctx.input(|input| input.pointer.any_down()) {
            self.mouse_button_down = None;
        }
        let events = ctx.input(|input| input.events.clone());
        let mouse_mode = self.core.input_modes().mouse_mode;
        let core_size = self.core.size();
        let cell_at = |pos: egui::Pos2| -> Option<CellPoint> {
            if !grid_rect.contains(pos) {
                return None;
            }
            let col = ((pos.x - grid_rect.left()) / cell_metrics.width).floor();
            let row = ((pos.y - grid_rect.top()) / cell_metrics.height).floor();
            if col < 0.0 || row < 0.0 {
                return None;
            }
            Some(CellPoint::new(
                (row as usize).min(core_size.rows.saturating_sub(1)),
                (col as usize).min(core_size.cols.saturating_sub(1)),
            ))
        };

        for event in events {
            match event {
                Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } => {
                    // Always clear the held-button state on release, including
                    // releases outside the grid or over the scrollbar.
                    if !pressed {
                        self.mouse_button_down = None;
                    }
                    let Some(point) = cell_at(pos) else {
                        if !pressed && self.selecting {
                            self.finish_local_selection_outside(platform, ctx, log, output);
                        }
                        continue;
                    };
                    response.request_focus();
                    let Some(term_button) = map_pointer_button(button) else {
                        continue;
                    };
                    let shift = modifiers.shift;
                    let local_selection = term_button == MouseButton::Left
                        && local_selection_requested(mouse_mode, shift);
                    if local_selection {
                        if pressed {
                            // A click clears any existing mark. The highlight and
                            // core selection wait until the pointer reaches another cell.
                            self.selecting = true;
                            self.selection_anchor = Some(point);
                            self.selection = None;
                            self.core.clear_selection();
                        } else if self.selecting {
                            self.finish_local_selection(point, platform, ctx, log, output);
                        }
                    } else if forward_mouse_report(mouse_mode, shift) {
                        let kind = if pressed {
                            MouseEventKind::Press
                        } else {
                            MouseEventKind::Release
                        };
                        if pressed {
                            self.mouse_button_down = Some(term_button);
                        }
                        let encoded = self.encoder.encode_mouse(MouseEvent {
                            kind,
                            button: term_button,
                            row: point.row as u16,
                            col: point.col as u16,
                            modifiers: pointer_modifiers(modifiers),
                        });
                        self.write(&encoded, log);
                    }
                }
                Event::PointerMoved(pos) => {
                    let shift = ctx.input(|input| input.modifiers.shift);
                    if self.selecting {
                        if let Some(point) = cell_at(pos) {
                            let anchor = self.selection_anchor.unwrap_or(point);
                            if local_selection_left_anchor(self.selection.is_some(), anchor, point)
                            {
                                self.show_local_selection(anchor, point);
                            }
                        }
                    } else if forward_mouse_report(mouse_mode, shift) {
                        if let Some(point) = cell_at(pos) {
                            let motion = self.core.input_modes().mouse_mode;
                            if motion == MouseMode::Motion
                                || (motion == MouseMode::Drag && self.mouse_button_down.is_some())
                            {
                                let encoded = self.encoder.encode_mouse(MouseEvent {
                                    kind: MouseEventKind::Move,
                                    button: self.mouse_button_down.unwrap_or(MouseButton::None),
                                    row: point.row as u16,
                                    col: point.col as u16,
                                    modifiers: pointer_modifiers(ctx.input(|i| i.modifiers)),
                                });
                                self.write(&encoded, log);
                            }
                        }
                    }
                }
                Event::MouseWheel {
                    delta, modifiers, ..
                } => {
                    let hover = ctx.input(|input| input.pointer.hover_pos());
                    let point = hover.and_then(cell_at);
                    if point.is_none() {
                        continue;
                    }
                    if local_scroll_requested(mouse_mode, modifiers.shift) {
                        if delta.y != 0.0 {
                            self.selection = None;
                            self.core.clear_selection();
                            let mut lines = (delta.y * 3.0).round() as i32;
                            if lines == 0 {
                                lines = delta.y.signum() as i32;
                            }
                            self.core.scroll(lines);
                        }
                    } else if forward_mouse_report(mouse_mode, modifiers.shift) {
                        let button = if delta.y > 0.0 {
                            MouseButton::WheelUp
                        } else {
                            MouseButton::WheelDown
                        };
                        let point = point.unwrap_or(CellPoint::new(0, 0));
                        let count = delta.y.abs().ceil().clamp(1.0, 8.0) as usize;
                        for _ in 0..count {
                            let encoded = self.encoder.encode_mouse(MouseEvent {
                                kind: MouseEventKind::Press,
                                button,
                                row: point.row as u16,
                                col: point.col as u16,
                                modifiers: pointer_modifiers(modifiers),
                            });
                            self.write(&encoded, log);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn process_scrollbar(
        &mut self,
        ui: &mut Ui,
        pane_rect: Rect,
        grid_size: TerminalSize,
        pane_hovered: bool,
    ) {
        let history = self.core.history_len();
        let total = history + grid_size.rows;
        if history == 0 {
            return;
        }
        let track = scrollbar_track_rect(pane_rect);
        if track.height() <= 1.0 {
            return;
        }
        let thumb_height =
            (track.height() * (grid_size.rows as f32 / total as f32)).clamp(24.0, track.height());
        let offset = self.core.display_offset();
        let progress = if history == 0 {
            0.0
        } else {
            offset as f32 / history as f32
        };
        let thumb_top = track.top() + (track.height() - thumb_height) * (1.0 - progress);
        let response = ui.interact(
            track,
            ui.make_persistent_id("made-terminal-scrollbar"),
            Sense::click_and_drag(),
        );
        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                let available = (track.height() - thumb_height).max(1.0);
                let relative = ((pos.y - track.top()) / available).clamp(0.0, 1.0);
                let desired_offset = ((1.0 - relative) * history as f32).round() as i32;
                let delta = desired_offset - offset as i32;
                if delta != 0 {
                    self.selection = None;
                    self.core.clear_selection();
                    self.core.scroll(delta);
                }
            }
        }
        let bar_hot = response.hovered() || response.dragged();
        let fade = ui
            .ctx()
            .animate_bool_with_time(
                ui.make_persistent_id("made-terminal-scrollbar-fade"),
                pane_hovered || bar_hot,
                0.14,
            )
            .clamp(0.0, 1.0);
        if fade <= 0.005 {
            return;
        }
        let (track_color, thumb_color) = scrollbar_paint(fade, bar_hot);
        if track_color.a() > 0 {
            ui.painter().rect_filled(track, 3.0, track_color);
        }
        let thumb_width = scrollbar_thumb_width(track.width(), bar_hot);
        let thumb = Rect::from_min_size(
            egui::Pos2::new(track.right() - thumb_width, thumb_top),
            Vec2::new(thumb_width, thumb_height),
        );
        ui.painter().rect_filled(thumb, 3.0, thumb_color);
    }

    fn paint_shell_state(&mut self, ui: &mut Ui, grid_rect: Rect, output: &mut TerminalUiOutput) {
        let message = if let Some(error) = &self.monitor_error {
            Some(format!("Shell monitor failed: {error}"))
        } else if let Some(error) = &self.read_error {
            Some(format!("Shell output failed: {error}"))
        } else if let Some(exit) = &self.exit {
            Some(format!("Shell exited ({exit})."))
        } else if self.eof {
            Some("Shell output closed.".to_owned())
        } else {
            None
        };
        let Some(message) = message else {
            return;
        };
        let message = crate::sanitize::sanitize_display(&message, 400);

        let message_font = egui::FontId::proportional(11.5);
        let max_overlay_width = if grid_rect.width() < 600.0 {
            340.0
        } else {
            520.0
        };
        let overlay_width = (grid_rect.width() - 20.0).min(max_overlay_width).max(180.0);
        let text_width = (overlay_width - 20.0 - 100.0).max(60.0);
        let galley = ui.painter().layout(
            message.clone(),
            message_font,
            crate::theme::WARNING,
            text_width,
        );
        let overlay_height = (galley.size().y + 20.0).max(48.0);
        let overlay = Rect::from_min_size(
            egui::Pos2::new(
                grid_rect.left() + 10.0,
                grid_rect.bottom() - overlay_height - 10.0,
            ),
            Vec2::new(overlay_width, overlay_height),
        );
        ui.painter().rect(
            overlay,
            egui::CornerRadius::same(8),
            crate::theme::BG_ELEVATED,
            egui::Stroke::new(1.0, crate::theme::WARNING),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(
            overlay.left_top() + Vec2::new(10.0, 10.0),
            galley,
            crate::theme::WARNING,
        );
        let button_rect = Rect::from_min_size(
            egui::Pos2::new(overlay.right() - 96.0, overlay.top() + 9.0),
            Vec2::new(86.0, 24.0),
        );
        let restart_clicked = ui
            .put(
                button_rect,
                egui::Button::new(
                    egui::RichText::new("Restart shell")
                        .font(egui::FontId::proportional(11.5))
                        .color(crate::theme::WARNING),
                )
                .fill(crate::theme::WARNING_TINT)
                .stroke(egui::Stroke::new(1.0, crate::theme::WARNING))
                .corner_radius(egui::CornerRadius::same(6)),
            )
            .clicked();
        if restart_clicked {
            output.restart_requested = true;
        }
    }

    fn clear_selection_for_input(&mut self) {
        self.selection = None;
        self.selection_anchor = None;
        self.selecting = false;
        self.core.clear_selection();
    }

    fn show_local_selection(&mut self, anchor: CellPoint, point: CellPoint) {
        if self.selection.is_none() {
            self.core.select_start(anchor.row, anchor.col);
        }
        self.selection = Some(SelectionRange::new(anchor, point).normalized());
        self.core.select_update(point.row, point.col);
    }

    fn finish_local_selection(
        &mut self,
        point: CellPoint,
        platform: &dyn Platform,
        ctx: &egui::Context,
        log: &mut DiagnosticLog,
        output: &mut TerminalUiOutput,
    ) {
        let anchor = self.selection_anchor.take().unwrap_or(point);
        let committed = self.selection.is_some();
        self.selecting = false;
        if local_selection_left_anchor(committed, anchor, point) {
            self.show_local_selection(anchor, point);
            if let Some(text) = self.copy_selection(platform, ctx, log) {
                output.notice = Some(format!("copied {} characters", text.chars().count()));
            }
        } else {
            self.selection = None;
            self.core.clear_selection();
        }
    }

    fn finish_local_selection_outside(
        &mut self,
        platform: &dyn Platform,
        ctx: &egui::Context,
        log: &mut DiagnosticLog,
        output: &mut TerminalUiOutput,
    ) {
        self.selecting = false;
        self.selection_anchor = None;
        if self.selection.is_some() {
            if let Some(text) = self.copy_selection(platform, ctx, log) {
                output.notice = Some(format!("copied {} characters", text.chars().count()));
            }
        } else {
            self.selection = None;
            self.core.clear_selection();
        }
    }
}

fn paste_allowed(byte_len: usize) -> bool {
    byte_len <= MAX_PASTE_BYTES
}

fn local_selection_requested(mouse_mode: MouseMode, shift: bool) -> bool {
    mouse_mode == MouseMode::Off || shift
}

/// Whether the pointer has left the press cell, or a selection is already committed.
fn local_selection_left_anchor(committed: bool, anchor: CellPoint, point: CellPoint) -> bool {
    committed || point != anchor
}

fn local_scroll_requested(mouse_mode: MouseMode, shift: bool) -> bool {
    mouse_mode == MouseMode::Off || shift
}

/// Gap between the scrollbar's right edge and the card's 1px inside border.
///
/// Four points is the spacing-scale step. The 8px hit area stays; only its
/// right edge moves off the border.
const SCROLLBAR_EDGE_GAP: f32 = 4.0;

/// Full 8px scrollbar hit area inside the terminal pane.
///
/// The rightmost pane pixel is the card border. The track ends one border
/// pixel plus [`SCROLLBAR_EDGE_GAP`] inside that edge so the thumb is not
/// flush with the card.
fn scrollbar_track_rect(pane_rect: Rect) -> Rect {
    let right = pane_rect.right() - 1.0 - SCROLLBAR_EDGE_GAP;
    Rect::from_min_max(
        egui::Pos2::new(right - SCROLLBAR_WIDTH, pane_rect.top() + TERMINAL_PADDING),
        egui::Pos2::new(right, pane_rect.bottom() - TERMINAL_PADDING),
    )
}

/// Track and thumb colors for the fading terminal scrollback bar.
///
/// Only the thumb appears when the terminal pane is hovered. The faint track
/// appears only while the bar itself is hovered or dragged, matching egui's
/// floating scrollbars.
fn scrollbar_paint(fade: f32, bar_hot: bool) -> (egui::Color32, egui::Color32) {
    let fade = fade.clamp(0.0, 1.0);
    let track = if bar_hot {
        crate::theme::SCROLL_TRACK.gamma_multiply(fade * 0.45)
    } else {
        egui::Color32::TRANSPARENT
    };
    let thumb = if bar_hot {
        crate::theme::SCROLL_THUMB_HOVER
    } else {
        crate::theme::SCROLL_THUMB
    }
    .gamma_multiply(fade);
    (track, thumb)
}

/// The visible thumb widens to the full 8px hit area on bar hover/drag.
fn scrollbar_thumb_width(track_width: f32, bar_hot: bool) -> f32 {
    if bar_hot {
        track_width
    } else {
        (track_width * 0.65).max(4.0)
    }
}

fn forward_mouse_report(mouse_mode: MouseMode, shift: bool) -> bool {
    mouse_mode != MouseMode::Off && !shift
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KeyRoute {
    CopySelection,
    PasteClipboard,
    Ignore,
    Write(Vec<u8>),
}

fn route_key(
    key: Key,
    modifiers: egui::Modifiers,
    encoder: &InputEncoder,
    has_selection: bool,
) -> KeyRoute {
    let command = modifiers.command || modifiers.ctrl;

    if command && key == Key::C {
        if has_selection {
            return KeyRoute::CopySelection;
        }
        if modifiers.mac_cmd {
            return KeyRoute::Ignore;
        }
        // On Linux/Windows Ctrl+C without a selection is the interrupt
        // character and must reach the child.
    }
    if command && key == Key::V {
        return KeyRoute::PasteClipboard;
    }

    // Super/Command combinations that are not explicitly handled must not leak
    // into the child as plain text. Ctrl/Super+S and Ctrl+X are NOT special:
    // the TUI receives XOFF and CAN, as it expects.
    if modifiers.mac_cmd && !modifiers.ctrl && !matches!(key, Key::C | Key::V) {
        return KeyRoute::Ignore;
    }

    let terminal_modifiers = TerminalModifiers {
        shift: modifiers.shift,
        alt: modifiers.alt,
        ctrl: modifiers.ctrl || modifiers.command,
        super_key: modifiers.mac_cmd,
    };

    if let Some(terminal_key) = map_key(key) {
        return KeyRoute::Write(encoder.encode_key(terminal_key, terminal_modifiers));
    }

    if terminal_modifiers.ctrl || terminal_modifiers.alt || terminal_modifiers.super_key {
        if let Some(character) = key_character(key) {
            let character = if modifiers.shift {
                character.to_ascii_uppercase()
            } else {
                character
            };
            return KeyRoute::Write(
                encoder.encode_key(TerminalKey::Char(character), terminal_modifiers),
            );
        }
    }
    KeyRoute::Ignore
}

fn map_key(key: Key) -> Option<TerminalKey> {
    match key {
        Key::Enter => Some(TerminalKey::Enter),
        Key::Tab => Some(TerminalKey::Tab),
        Key::Backspace => Some(TerminalKey::Backspace),
        Key::Escape => Some(TerminalKey::Escape),
        Key::ArrowUp => Some(TerminalKey::ArrowUp),
        Key::ArrowDown => Some(TerminalKey::ArrowDown),
        Key::ArrowLeft => Some(TerminalKey::ArrowLeft),
        Key::ArrowRight => Some(TerminalKey::ArrowRight),
        Key::Home => Some(TerminalKey::Home),
        Key::End => Some(TerminalKey::End),
        Key::Insert => Some(TerminalKey::Insert),
        Key::Delete => Some(TerminalKey::Delete),
        Key::PageUp => Some(TerminalKey::PageUp),
        Key::PageDown => Some(TerminalKey::PageDown),
        Key::F1 => Some(TerminalKey::Function(1)),
        Key::F2 => Some(TerminalKey::Function(2)),
        Key::F3 => Some(TerminalKey::Function(3)),
        Key::F4 => Some(TerminalKey::Function(4)),
        Key::F5 => Some(TerminalKey::Function(5)),
        Key::F6 => Some(TerminalKey::Function(6)),
        Key::F7 => Some(TerminalKey::Function(7)),
        Key::F8 => Some(TerminalKey::Function(8)),
        Key::F9 => Some(TerminalKey::Function(9)),
        Key::F10 => Some(TerminalKey::Function(10)),
        Key::F11 => Some(TerminalKey::Function(11)),
        Key::F12 => Some(TerminalKey::Function(12)),
        _ => None,
    }
}

/// Base character for a logical key. Shift is applied by the caller.
fn key_character(key: Key) -> Option<char> {
    Some(match key {
        Key::A => 'a',
        Key::B => 'b',
        Key::C => 'c',
        Key::D => 'd',
        Key::E => 'e',
        Key::F => 'f',
        Key::G => 'g',
        Key::H => 'h',
        Key::I => 'i',
        Key::J => 'j',
        Key::K => 'k',
        Key::L => 'l',
        Key::M => 'm',
        Key::N => 'n',
        Key::O => 'o',
        Key::P => 'p',
        Key::Q => 'q',
        Key::R => 'r',
        Key::S => 's',
        Key::T => 't',
        Key::U => 'u',
        Key::V => 'v',
        Key::W => 'w',
        Key::X => 'x',
        Key::Y => 'y',
        Key::Z => 'z',
        Key::Num0 => '0',
        Key::Num1 => '1',
        Key::Num2 => '2',
        Key::Num3 => '3',
        Key::Num4 => '4',
        Key::Num5 => '5',
        Key::Num6 => '6',
        Key::Num7 => '7',
        Key::Num8 => '8',
        Key::Num9 => '9',
        Key::Space => ' ',
        Key::Colon => ':',
        Key::Comma => ',',
        Key::Backslash => '\\',
        Key::Slash => '/',
        Key::Pipe => '|',
        Key::Questionmark => '?',
        Key::Exclamationmark => '!',
        Key::OpenBracket => '[',
        Key::CloseBracket => ']',
        Key::OpenCurlyBracket => '{',
        Key::CloseCurlyBracket => '}',
        Key::Backtick => '`',
        Key::Minus => '-',
        Key::Period => '.',
        Key::Plus => '+',
        Key::Equals => '=',
        Key::Semicolon => ';',
        Key::Quote => '\'',
        _ => return None,
    })
}

fn map_pointer_button(button: egui::PointerButton) -> Option<MouseButton> {
    match button {
        egui::PointerButton::Primary => Some(MouseButton::Left),
        egui::PointerButton::Secondary => Some(MouseButton::Right),
        egui::PointerButton::Middle => Some(MouseButton::Middle),
        _ => None,
    }
}

fn pointer_modifiers(modifiers: egui::Modifiers) -> TerminalModifiers {
    TerminalModifiers {
        shift: modifiers.shift,
        alt: modifiers.alt,
        ctrl: modifiers.ctrl,
        super_key: modifiers.mac_cmd,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use made_terminal::InputModes;

    fn ctrl() -> egui::Modifiers {
        egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        }
    }

    fn cmd() -> egui::Modifiers {
        egui::Modifiers {
            mac_cmd: true,
            command: true,
            ..egui::Modifiers::NONE
        }
    }

    #[cfg(unix)]
    #[test]
    fn input_resets_the_cursor_blink_epoch() {
        let temp = tempfile::tempdir().unwrap();
        let platform = made_platform::host_platform();
        let mut pane = TerminalPane::spawn(
            &platform,
            PtyOptions::new(temp.path()).with_shell("/bin/sh"),
        )
        .expect("spawn shell");
        pane.blink_epoch = Instant::now() - Duration::from_secs(2);
        let stale = pane.blink_epoch;
        let before_input = Instant::now();
        let mut log = DiagnosticLog::new();

        assert!(pane.write(b"x", &mut log));
        assert!(pane.blink_epoch > stale);
        assert!(
            pane.blink_epoch >= before_input,
            "input must reset the blink epoch to the on phase"
        );
    }

    #[test]
    fn focus_gain_resets_the_cursor_blink_epoch() {
        let temp = tempfile::tempdir().unwrap();
        let platform = made_platform::host_platform();
        let mut pane = TerminalPane::spawn(&platform, PtyOptions::new(temp.path())).unwrap();
        let mut log = DiagnosticLog::new();
        let mut metrics = Metrics::new();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let _ = run_terminal_frame(
                &ctx,
                &mut pane,
                &platform,
                &mut metrics,
                &mut log,
                Vec::new(),
            );
        }
        let terminal_focus = pane.focus_id().expect("terminal focus id");
        assert!(pane.is_focused());

        let other_focus = egui::Id::unique("blink-focus-other");
        ctx.memory_mut(|memory| memory.request_focus(other_focus));
        let _ = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            Vec::new(),
        );
        assert!(!pane.is_focused());

        pane.blink_epoch = Instant::now() - Duration::from_secs(2);
        let before_focus_gain = Instant::now();
        ctx.memory_mut(|memory| memory.request_focus(terminal_focus));
        let _ = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            Vec::new(),
        );

        assert!(pane.is_focused());
        assert!(
            pane.blink_epoch >= before_focus_gain,
            "focus gain must reset the blink epoch to the on phase"
        );
        assert!(cursor_blink_on(true, true, pane.blink_epoch.elapsed()));
        pane.terminate(&mut log);
    }

    #[test]
    fn control_keys_reach_the_encoder_for_the_child_tui() {
        let encoder = InputEncoder::new(InputModes::default());
        assert_eq!(
            route_key(Key::C, ctrl(), &encoder, false),
            KeyRoute::Write(vec![0x03]),
            "Ctrl+C without a selection must be SIGINT"
        );
        assert_eq!(
            route_key(Key::X, ctrl(), &encoder, false),
            KeyRoute::Write(vec![0x18]),
            "Ctrl+X must stay CAN"
        );
        assert_eq!(
            route_key(Key::S, ctrl(), &encoder, false),
            KeyRoute::Write(vec![0x13]),
            "Ctrl+S must stay XOFF for the TUI"
        );
        assert_eq!(
            route_key(Key::V, ctrl(), &encoder, false),
            KeyRoute::PasteClipboard
        );
        assert_eq!(
            route_key(Key::C, ctrl(), &encoder, true),
            KeyRoute::CopySelection
        );
        assert_eq!(route_key(Key::C, cmd(), &encoder, false), KeyRoute::Ignore);
    }

    #[test]
    fn pending_input_is_bounded_and_batched() {
        let mut pending = PendingInput::default();
        assert!(pending.enqueue(b"abc"));
        assert_eq!(pending.front_batch(), b"abc");
        pending.consume(2);
        assert_eq!(pending.len(), 1);
        assert!(!pending.enqueue(&vec![0u8; MAX_PENDING_INPUT_BYTES]));
        assert!(!pending.is_empty());
        pending.clear();
        assert!(pending.is_empty());
        assert!(paste_allowed(MAX_PASTE_BYTES));
        assert!(!paste_allowed(MAX_PASTE_BYTES + 1));
    }

    #[cfg(unix)]
    #[test]
    fn input_latency_waits_for_a_painted_echo_not_background_output() {
        let temp = tempfile::tempdir().unwrap();
        let platform = made_platform::host_platform();
        let mut pane = TerminalPane::spawn(
            &platform,
            PtyOptions::new(temp.path())
                .with_shell("/bin/sh")
                .with_args([
                    "-c",
                    "stty -echo; i=0; while [ $i -lt 20000 ]; do printf 'flood output\\n'; i=$((i+1)); done; sleep 1",
                ])
                .with_size(made_platform::PtySize { rows: 24, cols: 80 }),
        )
        .expect("spawn flood child");
        let mut log = DiagnosticLog::new();
        let mut metrics = Metrics::new();
        let ctx = egui::Context::default();

        // Wait until the child has executed `stty -echo` and its flood has
        // actually reached the PTY, so startup input cannot be echoed by the
        // line discipline.
        let ready_deadline = Instant::now() + Duration::from_secs(2);
        while pane.output_bytes() == 0 && Instant::now() < ready_deadline {
            let _ = pane.pump(&mut log);
            let _ = run_terminal_frame(
                &ctx,
                &mut pane,
                &platform,
                &mut metrics,
                &mut log,
                Vec::new(),
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            pane.output_bytes() > 0,
            "the flood child must produce output before the echo test"
        );
        for character in "QZQZ".chars() {
            pane.send_key(
                made_terminal::Key::Char(character),
                made_terminal::Modifiers::default(),
                &mut log,
            );
        }
        for _ in 0..120 {
            let _ = pane.pump(&mut log);
            let _ = run_terminal_frame(
                &ctx,
                &mut pane,
                &platform,
                &mut metrics,
                &mut log,
                Vec::new(),
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            metrics.input_latency().is_none(),
            "background PTY output alone must not complete input latency: {:?}",
            metrics.input_latency()
        );
        pane.terminate(&mut log);

        // With echo enabled, the same marker becomes a valid painted echo.
        let mut pane = TerminalPane::spawn(
            &platform,
            PtyOptions::new(temp.path())
                .with_shell("/bin/sh")
                .with_args(["-c", "cat"])
                .with_size(made_platform::PtySize { rows: 24, cols: 80 }),
        )
        .expect("spawn echoing child");
        let mut metrics = Metrics::new();
        for character in "ECHO".chars() {
            pane.send_key(
                made_terminal::Key::Char(character),
                made_terminal::Modifiers::default(),
                &mut log,
            );
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && metrics.input_latency().is_none() {
            let _ = pane.pump(&mut log);
            let _ = run_terminal_frame(
                &ctx,
                &mut pane,
                &platform,
                &mut metrics,
                &mut log,
                Vec::new(),
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            metrics.input_latency().is_some(),
            "a painted echo must complete input latency"
        );
        pane.terminate(&mut log);
    }

    #[cfg(unix)]
    #[test]
    fn wouldblock_input_is_buffered_for_retry() {
        let temp = tempfile::tempdir().unwrap();
        let platform = made_platform::host_platform();
        let mut pane = TerminalPane::spawn(
            &platform,
            PtyOptions::new(temp.path())
                .with_shell("/bin/sh")
                .with_args(["-c", "sleep 2"])
                .with_size(made_platform::PtySize { rows: 24, cols: 80 })
                .with_write_queue_capacity(1),
        )
        .expect("spawn non-reading child");
        let mut log = DiagnosticLog::new();
        // Larger than the PTY pipe + one queued chunk, but not enough to
        // stress unrelated timing-sensitive PTY tests running in parallel.
        let payload = vec![b'x'; 256 * 1024];
        assert!(pane.write(&payload, &mut log));
        assert!(
            pane.has_pending_input(),
            "a full PTY queue must leave application-side input pending"
        );
        // The pending bytes are flushed on subsequent pump calls; terminating
        // clears them without blocking the UI.
        pane.terminate(&mut log);
        assert!(!pane.has_pending_input());
    }

    #[test]
    fn relaunch_clears_focus_ime_and_pending_input() {
        let temp = tempfile::tempdir().unwrap();
        let platform = made_platform::host_platform();
        let mut pane = TerminalPane::spawn(
            &platform,
            PtyOptions::new(temp.path()).with_size(made_platform::PtySize { rows: 24, cols: 80 }),
        )
        .expect("spawn test shell");
        let mut log = DiagnosticLog::new();
        pane.focused = true;
        pane.ime_preedit = "composing".to_owned();
        pane.ime.apply(ImeEvent::Preedit("composing".to_owned()));
        assert!(pane.pending_input.enqueue(b"queued"));
        pane.relaunch(&platform, &mut log).expect("relaunch");
        assert!(!pane.focused);
        assert!(pane.focus_on_next_frame);
        assert!(pane.ime_preedit.is_empty());
        assert!(!pane.ime.is_composing());
        assert!(!pane.has_pending_input());
        assert!(pane.is_running());
        pane.terminate(&mut log);
    }

    #[test]
    fn shift_mouse_routing_overrides_reporting() {
        assert!(local_selection_requested(MouseMode::Click, true));
        assert!(!local_selection_requested(MouseMode::Click, false));
        assert!(local_selection_requested(MouseMode::Off, false));
        assert!(local_scroll_requested(MouseMode::Motion, true));
        assert!(!local_scroll_requested(MouseMode::Motion, false));
        assert!(local_scroll_requested(MouseMode::Off, false));
        assert!(forward_mouse_report(MouseMode::Motion, false));
        assert!(!forward_mouse_report(MouseMode::Motion, true));
        assert!(!forward_mouse_report(MouseMode::Off, false));
    }

    #[test]
    fn local_selection_commits_only_after_the_pointer_reaches_another_cell() {
        let anchor = CellPoint::new(1, 2);
        let other = CellPoint::new(1, 5);
        assert!(
            !local_selection_left_anchor(false, anchor, anchor),
            "a click without leaving the cell must not highlight or copy"
        );
        assert!(local_selection_left_anchor(false, anchor, other));
        assert!(
            local_selection_left_anchor(true, anchor, anchor),
            "returning to the anchor after a drag still counts as a selection"
        );
    }

    #[test]
    fn click_does_not_select_or_copy_and_a_drag_to_another_cell_does() {
        let temp = tempfile::tempdir().unwrap();
        let platform = made_platform::host_platform();
        let mut pane = TerminalPane::spawn(
            &platform,
            PtyOptions::new(temp.path()).with_size(made_platform::PtySize { rows: 24, cols: 80 }),
        )
        .unwrap();
        let mut log = DiagnosticLog::new();
        let mut metrics = Metrics::new();
        let ctx = egui::Context::default();
        let _ = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            Vec::new(),
        );
        pane.core.feed(b"hello world");
        let metrics_cells = pane.renderer.metrics();
        let origin = egui::pos2(TERMINAL_PADDING + 1.0, TERMINAL_PADDING + 1.0);
        let other = origin + egui::vec2(metrics_cells.width * 3.0, 0.0);
        let press = |pos: egui::Pos2, pressed: bool, shift: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers {
                shift,
                ..egui::Modifiers::NONE
            },
        };

        let clicked = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![press(origin, true, false)],
        );
        assert!(
            pane.selection.is_none(),
            "a press must not highlight a cell"
        );
        assert!(!pane.core.has_selection());
        assert!(clicked.notice.is_none());
        let released = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![press(origin, false, false)],
        );
        assert!(pane.selection.is_none());
        assert!(!pane.core.has_selection());
        assert!(released.notice.is_none(), "a click must not copy");

        let _ = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![press(origin, true, false)],
        );
        let dragged = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![egui::Event::PointerMoved(other)],
        );
        assert!(
            pane.selection.is_some(),
            "a drag to another cell must select"
        );
        assert!(pane.core.has_selection());
        assert!(dragged.notice.is_none(), "copy waits for release");
        let copied = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![press(other, false, false)],
        );
        assert!(
            copied.notice.as_deref().unwrap_or("").contains("copied"),
            "releasing a drag must still copy: {:?}",
            copied.notice
        );

        // A later click clears the selection and does not copy it again.
        let _ = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![press(other, true, false)],
        );
        assert!(pane.selection.is_none());
        assert!(!pane.core.has_selection());
        let cleared = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![press(other, false, false)],
        );
        assert!(cleared.notice.is_none());
        pane.terminate(&mut log);
    }

    #[test]
    fn typing_clears_an_existing_terminal_selection() {
        let temp = tempfile::tempdir().unwrap();
        let platform = made_platform::host_platform();
        let mut pane = TerminalPane::spawn(
            &platform,
            PtyOptions::new(temp.path()).with_size(made_platform::PtySize { rows: 24, cols: 80 }),
        )
        .unwrap();
        let mut log = DiagnosticLog::new();
        let mut metrics = Metrics::new();
        let ctx = egui::Context::default();
        let _ = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            Vec::new(),
        );
        pane.core.feed(b"hello world");
        let mark = |pane: &mut TerminalPane| {
            pane.core.select_start(0, 0);
            pane.core.select_update(0, 4);
            pane.selection = Some(SelectionRange::new(
                CellPoint::new(0, 0),
                CellPoint::new(0, 4),
            ));
            assert!(
                pane.selection.is_some() && pane.core.has_selection(),
                "the mark must exist before input"
            );
        };
        let assert_cleared = |pane: &TerminalPane, kind: &str| {
            assert!(
                pane.selection.is_none() && !pane.core.has_selection(),
                "{kind} must clear the selection mark"
            );
        };

        mark(&mut pane);
        let _ = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![egui::Event::Text("x".to_owned())],
        );
        assert_cleared(&pane, "typed text");

        mark(&mut pane);
        let _ = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_cleared(&pane, "a writing key");

        mark(&mut pane);
        let _ = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![egui::Event::Ime(egui::ImeEvent::Commit("ime".to_owned()))],
        );
        assert_cleared(&pane, "an IME commit");

        mark(&mut pane);
        let _ = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![egui::Event::Paste("pasted".to_owned())],
        );
        assert_cleared(&pane, "a paste");

        mark(&mut pane);
        let copied = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![egui::Event::Key {
                key: egui::Key::C,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    ctrl: true,
                    ..egui::Modifiers::NONE
                },
            }],
        );
        assert!(
            pane.selection.is_some() && pane.core.has_selection(),
            "copy must keep the selection mark"
        );
        assert!(
            copied.notice.as_deref().unwrap_or("").contains("copied"),
            "copy must still copy: {:?}",
            copied.notice
        );
        pane.terminate(&mut log);
    }

    #[test]
    fn shift_drag_in_a_mouse_reporting_tui_selects_locally() {
        let temp = tempfile::tempdir().unwrap();
        let platform = made_platform::host_platform();
        let mut pane = TerminalPane::spawn(
            &platform,
            PtyOptions::new(temp.path()).with_size(made_platform::PtySize { rows: 24, cols: 80 }),
        )
        .unwrap();
        let mut log = DiagnosticLog::new();
        let mut metrics = Metrics::new();
        let ctx = egui::Context::default();
        let _ = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            Vec::new(),
        );
        pane.core.feed(b"\x1b[?1000hhello");
        assert_eq!(pane.core.input_modes().mouse_mode, MouseMode::Click);
        let cell = pane.renderer.metrics();
        let origin = egui::pos2(TERMINAL_PADDING + 1.0, TERMINAL_PADDING + 1.0);
        let other = origin + egui::vec2(cell.width * 3.0, 0.0);
        let press = |pos: egui::Pos2, pressed: bool, shift: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers {
                shift,
                ..egui::Modifiers::NONE
            },
        };

        let _ = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![press(origin, true, false), press(origin, false, false)],
        );
        assert!(
            pane.selection.is_none() && !pane.core.has_selection(),
            "a click without Shift must stay a mouse report, not a local selection"
        );

        let _ = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![press(origin, true, true)],
        );
        let _ = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![egui::Event::PointerMoved(other)],
        );
        assert!(pane.core.has_selection(), "Shift+drag must select locally");
        let copied = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![press(other, false, true)],
        );
        assert!(copied.notice.as_deref().unwrap_or("").contains("copied"));
        pane.terminate(&mut log);
    }

    fn run_terminal_frame(
        ctx: &egui::Context,
        pane: &mut TerminalPane,
        platform: &dyn made_platform::Platform,
        metrics: &mut Metrics,
        log: &mut DiagnosticLog,
        events: Vec<egui::Event>,
    ) -> TerminalUiOutput {
        run_terminal_frame_capture(ctx, pane, platform, metrics, log, events).0
    }

    fn run_terminal_frame_capture(
        ctx: &egui::Context,
        pane: &mut TerminalPane,
        platform: &dyn made_platform::Platform,
        metrics: &mut Metrics,
        log: &mut DiagnosticLog,
        events: Vec<egui::Event>,
    ) -> (TerminalUiOutput, egui::FullOutput) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 500.0),
            )),
            events,
            ..Default::default()
        };
        let mut output = None;
        let mut frame = ctx.run_ui(raw, |ui| {
            output = Some(pane.ui(ui, platform, metrics, log));
        });
        frame.textures_delta.clear();
        (output.expect("terminal rendered"), frame)
    }

    fn has_large_rect_fill(output: &egui::FullOutput, fill: egui::Color32) -> bool {
        fn visit(shape: &egui::Shape, fill: egui::Color32, found: &mut bool) {
            match shape {
                egui::Shape::Rect(rect) if rect.fill == fill => {
                    if rect.rect.width() * rect.rect.height() > 100_000.0 {
                        *found = true;
                    }
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        visit(shape, fill, found);
                    }
                }
                _ => {}
            }
        }
        let mut found = false;
        for clipped in &output.shapes {
            visit(&clipped.shape, fill, &mut found);
        }
        found
    }

    #[test]
    fn terminal_body_surface_switches_with_alternate_screen() {
        let temp = tempfile::tempdir().unwrap();
        let platform = made_platform::host_platform();
        let mut pane = TerminalPane::spawn(
            &platform,
            PtyOptions::new(temp.path()).with_size(made_platform::PtySize { rows: 24, cols: 80 }),
        )
        .unwrap();
        let mut log = DiagnosticLog::new();
        let mut metrics = Metrics::new();
        let ctx = egui::Context::default();

        let (_, primary) = run_terminal_frame_capture(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            Vec::new(),
        );
        assert!(
            !pane.alternate_screen(),
            "shell starts on the primary screen"
        );
        assert!(
            !has_large_rect_fill(&primary, crate::theme::BG_TERMINAL_GLASS),
            "primary-screen body must not add a second glass layer"
        );
        assert!(
            !has_large_rect_fill(&primary, crate::theme::BG_TERMINAL),
            "primary-screen body must not paint the opaque terminal fill"
        );

        pane.core.feed(b"\x1b[?1049h");
        assert!(pane.alternate_screen(), "ALT_SCREEN must be active");
        let (_, alternate) = run_terminal_frame_capture(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            Vec::new(),
        );
        assert!(
            !has_large_rect_fill(&alternate, crate::theme::BG_TERMINAL_GLASS),
            "alternate-screen body must not paint the glass fill"
        );
        assert!(
            !has_large_rect_fill(&alternate, crate::theme::BG_TERMINAL),
            "alternate-screen body must not paint a square over the rounded card fill"
        );

        pane.core.feed(b"\x1b[?1049l");
        assert!(
            !pane.alternate_screen(),
            "leaving ALT_SCREEN restores the primary screen"
        );
        let (_, restored) = run_terminal_frame_capture(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            Vec::new(),
        );
        assert!(
            !has_large_rect_fill(&restored, crate::theme::BG_TERMINAL_GLASS),
            "the restored primary body must not add a glass layer"
        );
        assert!(
            !has_large_rect_fill(&restored, crate::theme::BG_TERMINAL),
            "the restored primary body must not paint the opaque terminal fill"
        );

        pane.terminate(&mut log);
    }

    #[test]
    fn terminal_focus_locks_navigation_keys_and_f6_requests_focus_exit() {
        let temp = tempfile::tempdir().unwrap();
        let platform = made_platform::host_platform();
        let mut pane = TerminalPane::spawn(
            &platform,
            PtyOptions::new(temp.path()).with_size(made_platform::PtySize { rows: 24, cols: 80 }),
        )
        .unwrap();
        let mut log = DiagnosticLog::new();
        let mut metrics = Metrics::new();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let _ = run_terminal_frame(
                &ctx,
                &mut pane,
                &platform,
                &mut metrics,
                &mut log,
                Vec::new(),
            );
        }
        let term_id = pane.focus_id().expect("terminal focus id");
        assert!(pane.is_focused());
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(term_id));

        for key in [egui::Key::Tab, egui::Key::ArrowDown, egui::Key::Escape] {
            let output = run_terminal_frame(
                &ctx,
                &mut pane,
                &platform,
                &mut metrics,
                &mut log,
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert!(
                output.input_sent,
                "{key:?} should still be encoded for the child"
            );
            assert_eq!(
                ctx.memory(|memory| memory.focused()),
                Some(term_id),
                "{key:?} must not move egui focus off the terminal"
            );
            assert!(pane.is_focused());
        }

        let output = run_terminal_frame(
            &ctx,
            &mut pane,
            &platform,
            &mut metrics,
            &mut log,
            vec![egui::Event::Key {
                key: egui::Key::F6,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(output.focus_exit_requested);
        assert!(!output.input_sent, "F6 must not be encoded for the child");
        pane.terminate(&mut log);
    }

    #[test]
    fn hiding_a_terminal_resets_drag_focus_and_ime_without_touching_its_process() {
        let temp = tempfile::tempdir().unwrap();
        let platform = made_platform::host_platform();
        let mut pane = TerminalPane::spawn(&platform, PtyOptions::new(temp.path())).unwrap();
        let pid = pane.pid();
        let ctx = egui::Context::default();
        let id = egui::Id::unique("owned-terminal-focus");
        ctx.memory_mut(|memory| memory.request_focus(id));
        pane.focus_id = Some(id);
        pane.focused = true;
        pane.selecting = true;
        pane.mouse_button_down = Some(MouseButton::Left);
        pane.ime.apply(ImeEvent::Preedit("unfinished".to_owned()));
        pane.ime_preedit = "unfinished".to_owned();
        pane.set_visible(false);
        let mut log = DiagnosticLog::new();
        pane.sync_visibility(&ctx, &mut log);
        assert!(!pane.focused);
        assert!(!pane.selecting);
        assert!(pane.mouse_button_down.is_none());
        assert!(!pane.ime.is_composing());
        assert!(pane.ime_preedit.is_empty());
        assert_ne!(ctx.memory(|memory| memory.focused()), Some(id));
        assert_eq!(pane.pid(), pid);
        assert!(pane.is_running());
        pane.set_visible(true);
        assert!(pane.focus_on_next_frame);
        assert_eq!(pane.grid_paint_count(), 0);
        pane.terminate(&mut log);
    }

    #[test]
    fn terminal_scrollbar_fades_thumb_only_and_reveals_track_on_bar_hover() {
        let pane = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 500.0));
        let track = scrollbar_track_rect(pane);
        assert_eq!(track.width(), SCROLLBAR_WIDTH);
        assert_eq!(track.width(), 8.0);
        assert_eq!(track.right(), pane.right() - 1.0 - SCROLLBAR_EDGE_GAP);
        assert!(
            pane.right() - track.right() > 1.0,
            "scrollbar must not sit flush with the card border"
        );
        assert_eq!(track.height(), 500.0 - 2.0 * TERMINAL_PADDING);

        let (track, thumb) = scrollbar_paint(0.0, false);
        assert_eq!(track.a(), 0);
        assert_eq!(thumb.a(), 0);

        let (track, thumb) = scrollbar_paint(0.5, false);
        let rest = crate::theme::SCROLL_THUMB.to_srgba_unmultiplied();
        assert_eq!(
            track.a(),
            0,
            "pane hover must paint only the thumb, not a full-height track"
        );
        assert!(thumb.a() > 0);
        let thumb_rgb = thumb.to_srgba_unmultiplied();
        assert!(
            thumb_rgb[0].abs_diff(rest[0]) <= 1
                && thumb_rgb[1].abs_diff(rest[1]) <= 1
                && thumb_rgb[2].abs_diff(rest[2]) <= 1,
            "rest thumb must use SCROLL_THUMB: {thumb_rgb:?} vs {rest:?}"
        );

        let (hot_track, hot_thumb) = scrollbar_paint(1.0, true);
        assert!(
            hot_track.a() > 0,
            "bar hover/drag may reveal the faint track"
        );
        let hover = crate::theme::SCROLL_THUMB_HOVER.to_srgba_unmultiplied();
        let hot_rgb = hot_thumb.to_srgba_unmultiplied();
        assert!(
            hot_rgb[0].abs_diff(hover[0]) <= 1
                && hot_rgb[1].abs_diff(hover[1]) <= 1
                && hot_rgb[2].abs_diff(hover[2]) <= 1,
            "bar hover must use SCROLL_THUMB_HOVER: {hot_rgb:?} vs {hover:?}"
        );
        let (_, same_fade_hot) = scrollbar_paint(0.5, true);
        assert_eq!(same_fade_hot.a(), thumb.a());
        assert_ne!(
            same_fade_hot.to_srgba_unmultiplied(),
            thumb.to_srgba_unmultiplied(),
            "bar hover must strengthen the color at the same fade"
        );
        assert!(
            scrollbar_thumb_width(SCROLLBAR_WIDTH, true)
                > scrollbar_thumb_width(SCROLLBAR_WIDTH, false)
        );
    }

    #[test]
    fn alternate_screen_inset_is_uniform_and_drops_the_scrollbar_reservation() {
        let pane = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 500.0));
        let metrics = CellMetrics::new(7.8, 16.0);
        for radius in [8.0_f32, 10.0] {
            let inset = crate::render::alternate_screen_inset(radius);
            assert!(
                inset < TERMINAL_PADDING,
                "alternate inset {inset} must be smaller than primary padding"
            );
            let inner = radius - 1.0;
            let corner_distance = (radius - inset) * std::f32::consts::SQRT_2;
            assert!(
                corner_distance < inner,
                "cell corner must clear the inside stroke at radius {radius}"
            );
            let alternate = crate::render::terminal_content_layout(pane, metrics, true, radius);
            let primary = crate::render::terminal_content_layout(pane, metrics, false, radius);
            assert!(
                (alternate.content.left() - inset).abs() < 0.01,
                "left inset must be the uniform minimum"
            );
            assert!(
                (pane.right() - alternate.content.right() - inset).abs() < 0.01,
                "right inset must match the left inset, with no scrollbar reservation"
            );
            assert!(
                (alternate.content.top() - inset).abs() < 0.01
                    && (pane.bottom() - alternate.content.bottom() - inset).abs() < 0.01
            );
            let used = metrics.pixel_size(alternate.size);
            let left_spare = alternate.origin.x - alternate.content.left();
            let right_spare = alternate.content.right() - (alternate.origin.x + used.x);
            let top_spare = alternate.origin.y - alternate.content.top();
            let bottom_spare = alternate.content.bottom() - (alternate.origin.y + used.y);
            assert!((left_spare - right_spare).abs() < 0.01);
            assert!((top_spare - bottom_spare).abs() < 0.01);
            assert!(left_spare >= 0.0 && top_spare >= 0.0);
            assert!(
                (primary.content.left() - TERMINAL_PADDING).abs() < 0.01
                    && (pane.right()
                        - primary.content.right()
                        - TERMINAL_PADDING
                        - SCROLLBAR_WIDTH)
                        .abs()
                        < 0.01,
                "primary screen must keep the 8px padding and scrollbar reservation"
            );
            assert!(alternate.size.cols >= primary.size.cols);
            assert_eq!(primary.origin, primary.content.min);
        }
    }
}
