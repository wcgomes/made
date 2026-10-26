//! VT parsing and screen state behind the `TerminalCore` boundary.
//!
//! # Implementation choice (design D6 fallback)
//!
//! The change design names `libghostty-vt` as the preferred terminal core and
//! explicitly permits a fallback behind this boundary. This environment has no
//! Zig toolchain, so `libghostty-vt` cannot be built. The current implementation
//! therefore wraps [`alacritty_terminal`] 0.26, which provides the required VT
//! parser, alternate screen, bounded scrollback, Unicode width handling, title
//! events, and selection extraction.
//!
//! The boundary is deliberately narrow and uses only Made-owned types in its
//! public surface: bytes in; owned snapshots, cursor, title, cwd, damage, and
//! selection text out. Swapping in `libghostty-vt` later means replacing the
//! private `Term` field and the mapping functions below, not changing callers.
//!
//! OSC 7 (shell cwd) is not handled by `alacritty_terminal`'s event API, so a
//! small incremental OSC scanner runs alongside the VT parser. It only observes
//! the byte stream and never alters what the emulator sees.
//!
//! Known fallback limits: width handling is per Unicode code point, so
//! multi-code-point ZWJ emoji sequences are not clustered into one grapheme.
//! Single emoji and CJK width are correct; the renderer must trust cell widths.
//!
//! The crate stays OS-agnostic: no `cfg(target_os)` and no direct OS APIs.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions as VtDimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::{Cell as VtCell, Flags as VtFlags};
use alacritty_terminal::term::{Config as VtConfig, RenderableCursor, Term, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::{
    Color as VtColor, CursorShape as VtCursorShape, NamedColor, Processor,
};

use made_platform::{Clipboard, PlatformError};

/// Maximum number of bytes retained for one OSC sequence by the cwd scanner.
const OSC_BUFFER_LIMIT: usize = 8 * 1024;

/// Terminal viewport size in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    pub rows: usize,
    pub cols: usize,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self { rows: 24, cols: 80 }
    }
}

/// A color independent of the emulator implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TerminalColor {
    /// The terminal's default foreground or background.
    #[default]
    Default,
    /// Index into the 256-color palette.
    Indexed(u8),
    /// 24-bit truecolor.
    Rgb(u8, u8, u8),
}

/// Cell attribute flags used by the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellAttributes {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub hidden: bool,
    pub strikethrough: bool,
}

/// One rendered cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellSnapshot {
    pub ch: char,
    /// Combining characters attached to `ch` (zero-width code points).
    pub combining: Vec<char>,
    pub fg: TerminalColor,
    pub bg: TerminalColor,
    pub attributes: CellAttributes,
    /// This cell is the leading cell of a double-width character.
    pub wide: bool,
    /// This cell is the trailing spacer of a double-width character.
    pub wide_spacer: bool,
    /// This cell is the last cell of a soft-wrapped line.
    pub wrapped: bool,
}

impl Default for CellSnapshot {
    fn default() -> Self {
        Self {
            ch: ' ',
            combining: Vec::new(),
            fg: TerminalColor::Default,
            bg: TerminalColor::Default,
            attributes: CellAttributes::default(),
            wide: false,
            wide_spacer: false,
            wrapped: false,
        }
    }
}

impl CellSnapshot {
    /// Text contributed by this cell, excluding trailing wide spacers.
    pub fn text(&self) -> String {
        if self.wide_spacer {
            return String::new();
        }
        let mut text = String::new();
        text.push(self.ch);
        text.extend(self.combining.iter());
        text
    }
}

/// Cursor shape exposed to the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
    HollowBlock,
    Hidden,
}

/// Cursor state in viewport coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorSnapshot {
    /// Visible row, clamped to the viewport.
    pub row: usize,
    /// Visible column.
    pub col: usize,
    pub shape: CursorShape,
    /// Whether the cursor is inside the viewport and visible.
    pub visible: bool,
    pub blinking: bool,
}

impl Default for CursorSnapshot {
    fn default() -> Self {
        Self {
            row: 0,
            col: 0,
            shape: CursorShape::Block,
            visible: true,
            blinking: false,
        }
    }
}

/// One row of terminal cells.
///
/// Debug output reports counts rather than cell text; screen and scrollback
/// content can be arbitrarily large and may contain user secrets.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct LineSnapshot {
    pub cells: Vec<CellSnapshot>,
    /// Plain text with wide spacers skipped and trailing blanks trimmed.
    pub text: String,
    /// Whether the row continues on the next row (soft wrap).
    pub wrapped: bool,
}

impl fmt::Debug for LineSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LineSnapshot")
            .field("cell_count", &self.cells.len())
            .field("text_len", &self.text.len())
            .field("wrapped", &self.wrapped)
            .finish()
    }
}

/// Owned render snapshot of the visible viewport.
///
/// Debug output deliberately omits cell text and title contents. Terminal
/// output is untrusted and can be large; callers that need content use
/// [`Self::text`], [`Self::row_text`], or the cell accessors.
#[derive(Clone, PartialEq, Eq)]
pub struct TerminalSnapshot {
    pub size: TerminalSize,
    /// Row-major cells, `size.rows * size.cols` entries.
    pub cells: Vec<CellSnapshot>,
    pub cursor: CursorSnapshot,
    pub alternate_screen: bool,
    pub title: Option<String>,
    pub cwd: Option<PathBuf>,
    pub display_offset: usize,
    pub version: u64,
}

impl fmt::Debug for TerminalSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalSnapshot")
            .field("size", &self.size)
            .field("cell_count", &self.cells.len())
            .field("cursor", &self.cursor)
            .field("alternate_screen", &self.alternate_screen)
            .field("title_len", &self.title.as_ref().map(String::len))
            .field("cwd", &self.cwd)
            .field("display_offset", &self.display_offset)
            .field("version", &self.version)
            .finish()
    }
}

impl TerminalSnapshot {
    /// Cell at a viewport position.
    pub fn cell(&self, row: usize, col: usize) -> Option<&CellSnapshot> {
        if row >= self.size.rows || col >= self.size.cols {
            return None;
        }
        self.cells.get(row * self.size.cols + col)
    }

    /// Text of one viewport row.
    pub fn row_text(&self, row: usize) -> String {
        if row >= self.size.rows {
            return String::new();
        }
        let start = row * self.size.cols;
        line_text(&self.cells[start..start + self.size.cols])
    }

    /// Visible text, rows joined by newlines with trailing empty rows removed.
    pub fn text(&self) -> String {
        let mut rows: Vec<String> = (0..self.size.rows).map(|row| self.row_text(row)).collect();
        while rows.last().is_some_and(|row| row.is_empty()) {
            rows.pop();
        }
        rows.join("\n")
    }
}

/// Damaged viewport lines since the previous [`TerminalCore::take_damage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageLine {
    pub row: usize,
    pub left: usize,
    pub right: usize,
}

/// Accumulated damage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalDamage {
    /// Render every cell.
    pub full: bool,
    pub lines: Vec<DamageLine>,
}

impl TerminalDamage {
    pub fn is_empty(&self) -> bool {
        !self.full && self.lines.is_empty()
    }
}

/// VT terminal state behind the replaceable `TerminalCore` boundary.
pub struct TerminalCore {
    term: Term<CoreListener>,
    parser: Processor,
    listener: Arc<Mutex<ListenerState>>,
    size: TerminalSize,
    scrollback_limit: usize,
    version: u64,
    damage: TerminalDamage,
    osc: OscScanner,
    cwd: Option<PathBuf>,
}

impl TerminalCore {
    /// Create a terminal with a bounded scrollback and the default 24x80 size.
    pub fn new() -> Self {
        Self::with_size(TerminalSize::default(), 10_000)
    }

    /// Create a terminal of `size` with `scrollback_lines` of history.
    pub fn with_size(size: TerminalSize, scrollback_lines: usize) -> Self {
        let size = normalize_size(size);
        let config = VtConfig {
            scrolling_history: scrollback_lines,
            ..VtConfig::default()
        };
        let listener_state = Arc::new(Mutex::new(ListenerState::default()));
        let listener = CoreListener {
            state: listener_state.clone(),
        };
        let dimensions = VtSize { size };
        let term = Term::new(config, &dimensions, listener);
        let mut core = Self {
            term,
            parser: Processor::new(),
            listener: listener_state,
            size,
            scrollback_limit: scrollback_lines,
            version: 0,
            damage: TerminalDamage::default(),
            osc: OscScanner::default(),
            cwd: None,
        };
        core.collect_damage();
        core
    }

    /// Feed PTY output bytes.
    pub fn feed(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.parser.advance(&mut self.term, bytes);
        self.osc.advance(bytes);
        if let Some(cwd) = self.osc.take_cwd() {
            self.cwd = Some(cwd);
        }
        self.version = self.version.wrapping_add(1);
        self.collect_damage();
    }

    /// Resize the grid. `rows` and `cols` are clamped to the emulator minimum.
    pub fn resize(&mut self, rows: usize, cols: usize) {
        let size = normalize_size(TerminalSize { rows, cols });
        if size == self.size {
            return;
        }
        self.term.resize(VtSize { size });
        self.size = size;
        self.version = self.version.wrapping_add(1);
        self.damage.full = true;
        self.damage.lines.clear();
        self.term.reset_damage();
    }

    /// Current viewport size.
    pub fn size(&self) -> TerminalSize {
        self.size
    }

    /// Monotonic content version. Bumped by feed, resize, and scroll.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Take accumulated damage. The caller should redraw at least these lines.
    pub fn take_damage(&mut self) -> TerminalDamage {
        std::mem::take(&mut self.damage)
    }

    /// Owned visible viewport snapshot.
    pub fn snapshot(&self) -> TerminalSnapshot {
        let content = self.term.renderable_content();
        let offset = content.display_offset;
        let mut cells = vec![CellSnapshot::default(); self.size.rows * self.size.cols];

        for indexed in content.display_iter {
            let row = indexed.point.line.0 + offset as i32;
            if row < 0 || row as usize >= self.size.rows {
                continue;
            }
            let col = indexed.point.column.0;
            if col >= self.size.cols {
                continue;
            }
            let index = row as usize * self.size.cols + col;
            cells[index] = map_cell(indexed.cell);
        }

        let cursor_row = content.cursor.point.line.0 + offset as i32;
        let visible_row = cursor_row.clamp(0, self.size.rows.saturating_sub(1) as i32) as usize;
        let cursor_col = content.cursor.point.column.0.min(self.size.cols - 1);
        let shape = map_cursor_shape(content.cursor);
        let cursor = CursorSnapshot {
            row: visible_row,
            col: cursor_col,
            shape,
            visible: shape != CursorShape::Hidden
                && cursor_row >= 0
                && (cursor_row as usize) < self.size.rows,
            blinking: self.term.cursor_style().blinking,
        };

        TerminalSnapshot {
            size: self.size,
            cells,
            cursor,
            alternate_screen: content.mode.contains(TermMode::ALT_SCREEN),
            title: self.title(),
            cwd: self.cwd.clone(),
            display_offset: offset,
            version: self.version,
        }
    }

    /// Latest history lines, oldest first, limited to `max_lines` from the end.
    pub fn history(&self, max_lines: usize) -> Vec<LineSnapshot> {
        let history = self.history_len();
        let take = history.min(max_lines);
        let start = history - take;
        (start..history)
            .filter_map(|index| {
                let line = index as i32 - history as i32;
                self.line_at(line)
            })
            .collect()
    }

    /// Number of scrollback lines currently retained (bounded ring).
    pub fn history_len(&self) -> usize {
        self.term.history_size()
    }

    /// Alias for [`TerminalCore::history_len`] used by UI code.
    pub fn scrollback_len(&self) -> usize {
        self.history_len()
    }

    /// The configured scrollback bound.
    pub fn scrollback_limit(&self) -> usize {
        self.scrollback_limit
    }

    /// Snapshot of one grid line. Negative indices are history lines.
    pub fn line_at(&self, line: i32) -> Option<LineSnapshot> {
        let history = self.history_len() as i32;
        let bottom = self.size.rows as i32 - 1;
        if line < -history || line > bottom {
            return None;
        }
        let row = &self.term.grid()[Line(line)];
        let cells: Vec<CellSnapshot> = (0..self.size.cols)
            .map(|col| map_cell(&row[Column(col)]))
            .collect();
        let text = line_text(&cells);
        let wrapped = cells.last().is_some_and(|cell| cell.wrapped);
        Some(LineSnapshot {
            cells,
            text,
            wrapped,
        })
    }

    /// Text of one viewport row.
    pub fn row_text(&self, row: usize) -> String {
        let offset = self.term.grid().display_offset() as i32;
        self.line_at(row as i32 - offset)
            .map(|line| line.text)
            .unwrap_or_default()
    }

    /// Visible viewport text.
    pub fn screen_text(&self) -> String {
        self.snapshot().text()
    }

    /// Scroll the viewport through history. Positive delta scrolls up.
    pub fn scroll(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        self.term.scroll_display(Scroll::Delta(delta));
        self.version = self.version.wrapping_add(1);
        self.damage.full = true;
        self.damage.lines.clear();
    }

    /// Return the viewport to the live screen.
    pub fn scroll_to_bottom(&mut self) {
        if self.display_offset() == 0 {
            return;
        }
        self.term.scroll_display(Scroll::Bottom);
        self.version = self.version.wrapping_add(1);
        self.damage.full = true;
        self.damage.lines.clear();
    }

    /// Current scrollback display offset.
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Latest window title set by OSC 0 or OSC 2.
    pub fn title(&self) -> Option<String> {
        self.listener_state().title.clone()
    }

    /// Current shell working directory reported by OSC 7.
    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    /// Whether the alternate screen is active.
    pub fn alternate_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// Take VT replies (device attributes, mode reports, size reports) that
    /// must be written back to the PTY.
    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.listener_state_mut().replies)
    }

    /// Take and clear the bell flag.
    pub fn take_bell(&mut self) -> bool {
        std::mem::replace(&mut self.listener_state_mut().bell, false)
    }

    /// Terminal input modes for the encoder.
    pub fn input_modes(&self) -> crate::input::InputModes {
        let mode = *self.term.mode();
        crate::input::InputModes {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            app_keypad: mode.contains(TermMode::APP_KEYPAD),
            mouse_mode: if mode.contains(TermMode::MOUSE_MOTION) {
                crate::input::MouseMode::Motion
            } else if mode.contains(TermMode::MOUSE_DRAG) {
                crate::input::MouseMode::Drag
            } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
                crate::input::MouseMode::Click
            } else {
                crate::input::MouseMode::Off
            },
            sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            focus_reporting: mode.contains(TermMode::FOCUS_IN_OUT),
            csi_u: mode.contains(TermMode::DISAMBIGUATE_ESC_CODES),
        }
    }

    /// Start a simple selection at a viewport position.
    pub fn select_start(&mut self, row: usize, col: usize) {
        let point = self.viewport_point(row, col);
        self.term.selection = Some(Selection::new(SelectionType::Simple, point, Side::Left));
    }

    /// Extend the active selection to a viewport position.
    pub fn select_update(&mut self, row: usize, col: usize) {
        let point = self.viewport_point(row, col);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, Side::Right);
        }
    }

    /// Clear any selection.
    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    /// Whether a selection exists.
    pub fn has_selection(&self) -> bool {
        self.term.selection.is_some()
    }

    /// Selected text, or `None` when nothing is selected.
    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    /// Copy the selection to a clipboard. Returns `false` when nothing is
    /// selected.
    pub fn copy_selection(&self, clipboard: &mut dyn Clipboard) -> Result<bool, PlatformError> {
        let Some(text) = self.selection_text() else {
            return Ok(false);
        };
        clipboard.set_text(&text)?;
        Ok(true)
    }

    fn viewport_point(&self, row: usize, col: usize) -> Point {
        let offset = self.term.grid().display_offset() as i32;
        Point::new(
            Line(row as i32 - offset),
            Column(col.min(self.size.cols - 1)),
        )
    }

    fn collect_damage(&mut self) {
        let mut full = false;
        let mut lines = Vec::new();
        match self.term.damage() {
            TermDamage::Full => full = true,
            TermDamage::Partial(iterator) => {
                for line in iterator {
                    lines.push(DamageLine {
                        row: line.line,
                        left: line.left,
                        right: line.right,
                    });
                }
            }
        }
        self.term.reset_damage();

        if full {
            self.damage.full = true;
            self.damage.lines.clear();
        } else if !self.damage.full {
            for line in lines {
                if let Some(existing) = self
                    .damage
                    .lines
                    .iter_mut()
                    .find(|existing| existing.row == line.row)
                {
                    existing.left = existing.left.min(line.left);
                    existing.right = existing.right.max(line.right);
                } else {
                    self.damage.lines.push(line);
                }
            }
        }
    }

    fn listener_state(&self) -> MutexGuard<'_, ListenerState> {
        self.listener.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn listener_state_mut(&mut self) -> MutexGuard<'_, ListenerState> {
        self.listener.lock().unwrap_or_else(|err| err.into_inner())
    }
}

impl Default for TerminalCore {
    fn default() -> Self {
        Self::new()
    }
}

fn normalize_size(size: TerminalSize) -> TerminalSize {
    TerminalSize {
        rows: size.rows.max(1),
        cols: size.cols.max(2),
    }
}

fn line_text(cells: &[CellSnapshot]) -> String {
    let mut text = String::new();
    for cell in cells {
        if cell.wide_spacer {
            continue;
        }
        text.push(cell.ch);
        text.extend(cell.combining.iter());
    }
    let trimmed = text.trim_end_matches(' ').len();
    text.truncate(trimmed);
    text
}

fn map_cell(cell: &VtCell) -> CellSnapshot {
    let flags = cell.flags;
    CellSnapshot {
        ch: cell.c,
        combining: cell
            .zerowidth()
            .map(|chars| chars.to_vec())
            .unwrap_or_default(),
        fg: map_color(cell.fg),
        bg: map_color(cell.bg),
        attributes: CellAttributes {
            bold: flags.contains(VtFlags::BOLD),
            dim: flags.contains(VtFlags::DIM),
            italic: flags.contains(VtFlags::ITALIC),
            underline: flags.intersects(VtFlags::ALL_UNDERLINES),
            inverse: flags.contains(VtFlags::INVERSE),
            hidden: flags.contains(VtFlags::HIDDEN),
            strikethrough: flags.contains(VtFlags::STRIKEOUT),
        },
        wide: flags.contains(VtFlags::WIDE_CHAR),
        wide_spacer: flags
            .intersects(VtFlags::WIDE_CHAR_SPACER | VtFlags::LEADING_WIDE_CHAR_SPACER),
        wrapped: flags.contains(VtFlags::WRAPLINE),
    }
}

fn map_color(color: VtColor) -> TerminalColor {
    match color {
        VtColor::Spec(rgb) => TerminalColor::Rgb(rgb.r, rgb.g, rgb.b),
        VtColor::Indexed(index) => TerminalColor::Indexed(index),
        VtColor::Named(named) => match named {
            NamedColor::Foreground | NamedColor::Background | NamedColor::Cursor => {
                TerminalColor::Default
            }
            NamedColor::Black => TerminalColor::Indexed(0),
            NamedColor::Red => TerminalColor::Indexed(1),
            NamedColor::Green => TerminalColor::Indexed(2),
            NamedColor::Yellow => TerminalColor::Indexed(3),
            NamedColor::Blue => TerminalColor::Indexed(4),
            NamedColor::Magenta => TerminalColor::Indexed(5),
            NamedColor::Cyan => TerminalColor::Indexed(6),
            NamedColor::White => TerminalColor::Indexed(7),
            NamedColor::BrightBlack => TerminalColor::Indexed(8),
            NamedColor::BrightRed => TerminalColor::Indexed(9),
            NamedColor::BrightGreen => TerminalColor::Indexed(10),
            NamedColor::BrightYellow => TerminalColor::Indexed(11),
            NamedColor::BrightBlue => TerminalColor::Indexed(12),
            NamedColor::BrightMagenta => TerminalColor::Indexed(13),
            NamedColor::BrightCyan => TerminalColor::Indexed(14),
            NamedColor::BrightWhite => TerminalColor::Indexed(15),
            NamedColor::DimBlack => TerminalColor::Indexed(0),
            NamedColor::DimRed => TerminalColor::Indexed(1),
            NamedColor::DimGreen => TerminalColor::Indexed(2),
            NamedColor::DimYellow => TerminalColor::Indexed(3),
            NamedColor::DimBlue => TerminalColor::Indexed(4),
            NamedColor::DimMagenta => TerminalColor::Indexed(5),
            NamedColor::DimCyan => TerminalColor::Indexed(6),
            NamedColor::DimWhite => TerminalColor::Indexed(7),
            NamedColor::BrightForeground => TerminalColor::Default,
            NamedColor::DimForeground => TerminalColor::Default,
        },
    }
}

fn map_cursor_shape(cursor: RenderableCursor) -> CursorShape {
    match cursor.shape {
        VtCursorShape::Block => CursorShape::Block,
        VtCursorShape::Underline => CursorShape::Underline,
        VtCursorShape::Beam => CursorShape::Bar,
        VtCursorShape::HollowBlock => CursorShape::HollowBlock,
        VtCursorShape::Hidden => CursorShape::Hidden,
    }
}

#[derive(Debug, Clone, Copy)]
struct VtSize {
    size: TerminalSize,
}

impl VtDimensions for VtSize {
    fn total_lines(&self) -> usize {
        self.size.rows
    }

    fn screen_lines(&self) -> usize {
        self.size.rows
    }

    fn columns(&self) -> usize {
        self.size.cols
    }
}

#[derive(Default)]
struct ListenerState {
    title: Option<String>,
    replies: Vec<u8>,
    bell: bool,
}

struct CoreListener {
    state: Arc<Mutex<ListenerState>>,
}

impl EventListener for CoreListener {
    fn send_event(&self, event: Event) {
        let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
        match event {
            Event::Title(title) => state.title = Some(title),
            Event::ResetTitle => state.title = None,
            Event::Bell => state.bell = true,
            Event::PtyWrite(text) => state.replies.extend_from_slice(text.as_bytes()),
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum OscState {
    #[default]
    Ground,
    Escape,
    Osc,
    OscEscape,
}

/// Incremental OSC 7 scanner. Runs beside the emulator; never modifies bytes.
///
/// It deliberately does not implement `Debug`: its buffer can hold terminal
/// content, and there is no safe reason to dump it.
#[derive(Default)]
struct OscScanner {
    state: OscState,
    buffer: Vec<u8>,
    overflow: bool,
    cwd: Option<PathBuf>,
}

impl OscScanner {
    fn advance(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            match self.state {
                OscState::Ground => {
                    if byte == 0x1b {
                        self.state = OscState::Escape;
                    }
                }
                OscState::Escape => {
                    if byte == b']' {
                        self.buffer.clear();
                        self.overflow = false;
                        self.state = OscState::Osc;
                    } else {
                        self.state = OscState::Ground;
                    }
                }
                OscState::Osc => match byte {
                    0x07 => {
                        self.finish();
                        self.state = OscState::Ground;
                    }
                    0x1b => self.state = OscState::OscEscape,
                    byte => self.push(byte),
                },
                OscState::OscEscape => {
                    if byte == b'\\' {
                        self.finish();
                        self.state = OscState::Ground;
                    } else {
                        // Not a string terminator; discard the OSC attempt.
                        self.state = OscState::Ground;
                    }
                }
            }
        }
    }

    fn push(&mut self, byte: u8) {
        if self.buffer.len() < OSC_BUFFER_LIMIT {
            self.buffer.push(byte);
        } else {
            self.overflow = true;
        }
    }

    fn finish(&mut self) {
        if !self.overflow {
            if let Some(path) = parse_osc7(&self.buffer) {
                self.cwd = Some(path);
            }
        }
        self.buffer.clear();
        self.overflow = false;
    }

    fn take_cwd(&mut self) -> Option<PathBuf> {
        self.cwd.take()
    }
}

fn parse_osc7(buffer: &[u8]) -> Option<PathBuf> {
    let text = std::str::from_utf8(buffer).ok()?;
    let (code, uri) = text.split_once(';')?;
    if code != "7" {
        return None;
    }
    parse_file_uri(uri)
}

/// Parse bounded local OSC 7 URIs. Authority is never a remote filesystem grant.
fn parse_file_uri(uri: &str) -> Option<PathBuf> {
    if uri.len() > 16_384
        || uri
            .bytes()
            .any(|b| !(b.is_ascii_alphanumeric() || b"-._~:/@!$&'()*+,;=%".contains(&b)))
    {
        return None;
    }
    let rest = uri.strip_prefix("file://")?;
    let slash = rest.find('/')?;
    let authority = &rest[..slash];
    if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
        return None;
    }
    let path = &rest[slash..];
    let decoded = percent_decode(path.as_bytes())?;
    let decoded = String::from_utf8(decoded).ok()?;
    made_platform::local_cwd_path(&decoded)
}

fn percent_decode(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let high = hex_value(bytes[index + 1])?;
            let low = hex_value(bytes[index + 2])?;
            out.push((high << 4) | low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    Some(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
