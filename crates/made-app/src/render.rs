//! Dedicated terminal grid renderer and frame pacing (Phase 1, tasks 1.9-1.10).
//!
//! The terminal is not an egui text widget. [`GridRenderer`] converts an owned
//! [`TerminalSnapshot`] into cached, grouped glyph runs, and [`paint_grid`]
//! draws those runs with a monospace font. Rows are rebuilt only when the
//! terminal version, the damage set, the selection, the palette, or the grid
//! size changes.
//!
//! The cache and style mapping are pure values with no GPU or display
//! dependency, so the renderer tests run headlessly. `paint_grid` is the only
//! function that needs an egui [`Painter`].
//!
//! # Font fallback
//!
//! egui's bundled default fonts provide the monospace grid, Latin text, and
//! emoji. The `system_fonts` eframe feature additionally asks the platform's
//! font fallback (through a cross-platform provider, not Made's platform crate)
//! for an installed face when a glyph is missing, so CJK and other scripts have
//! a real chance to render. If no face is available, egui draws its replacement
//! glyph inside the cell width reported by the emulator, so grid geometry stays
//! correct and the TUI is not corrupted. The emulator's per-code-point width
//! behavior and this replacement-glyph fallback are the documented v1 limits.
//!
//! Box drawing, block elements, and braille are not taken from that font.
//! The cell-glyph painter draws them as geometry fitted to the cell, then clips
//! the ink, so a mark cannot cover the next column or merge into a solid rectangle.

use std::time::Duration;

use eframe::egui::{self, Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, Vec2};

use crate::cell_glyphs::{self, Span};
use made_terminal::{
    CellAttributes, CellSnapshot, CursorShape, CursorSnapshot, TerminalColor, TerminalDamage,
    TerminalSize, TerminalSnapshot,
};

/// Default terminal font size in points.
pub const DEFAULT_FONT_SIZE: f32 = 13.0;

/// Padding between the pane edge and the first cell.
pub const TERMINAL_PADDING: f32 = 8.0;

/// Width reserved at the right edge for the scrollbar.
pub const SCROLLBAR_WIDTH: f32 = 8.0;

/// Cell metrics derived from the active monospace font.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellMetrics {
    /// Advance width of one narrow cell, in points.
    pub width: f32,
    /// Row height, in points.
    pub height: f32,
}

impl CellMetrics {
    /// Build metrics, clamping degenerate values so grid math cannot divide by
    /// zero or produce an unusable viewport.
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width: if width.is_finite() {
                width.max(1.0)
            } else {
                1.0
            },
            height: if height.is_finite() {
                height.max(1.0)
            } else {
                1.0
            },
        }
    }

    /// Measure the host monospace font through egui's font atlas.
    ///
    /// This only needs a font atlas, not a GPU surface, so it is safe to call
    /// from a headless egui context.
    pub fn from_context(ctx: &egui::Context, font_size: f32) -> Self {
        let font_id = FontId::monospace(font_size);
        let width = ctx.fonts_mut(|fonts| fonts.glyph_width(&font_id, 'M'));
        let height = ctx.fonts_mut(|fonts| fonts.row_height(&font_id));
        let width = if width > 0.0 { width } else { font_size * 0.6 };
        Self::new(width, height)
    }

    /// Grid dimensions for an available pixel area.
    pub fn grid_size(self, available: Vec2) -> TerminalSize {
        let cols = (available.x / self.width).floor().max(2.0) as usize;
        let rows = (available.y / self.height).floor().max(1.0) as usize;
        TerminalSize { rows, cols }
    }

    /// Pixel size of a grid.
    pub fn pixel_size(self, size: TerminalSize) -> Vec2 {
        Vec2::new(
            size.cols as f32 * self.width,
            size.rows as f32 * self.height,
        )
    }
}

impl Default for CellMetrics {
    fn default() -> Self {
        Self {
            width: 8.0,
            height: 16.0,
        }
    }
}

/// Content rect, paint origin, and hit rect for one terminal pane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalContentLayout {
    /// Area available for whole cells after the mode's inset.
    pub content: Rect,
    /// Top-left of cell (0, 0).
    pub origin: Pos2,
    /// Hit-test and clip rect. Primary screen keeps the padded content rect;
    /// alternate screen is the exact centered cell grid.
    pub grid_rect: Rect,
    /// Columns and rows that fit in `content`.
    pub size: TerminalSize,
}

/// Minimum uniform inset that keeps a cell corner inside a 1px inside stroke
/// on a rounded card.
///
/// The corner must sit inside the inner edge of the stroke. A half pixel keeps
/// antialiased stroke coverage off the glyph.
pub fn alternate_screen_inset(card_radius: f32) -> f32 {
    let radius = if card_radius.is_finite() {
        card_radius.max(1.0)
    } else {
        1.0
    };
    let inner = (radius - 1.0).max(0.0);
    let corner = radius - inner / std::f32::consts::SQRT_2;
    (corner + 0.5).max(1.0)
}

/// Lay out the cell grid for primary-screen padding or alternate-screen TUI mode.
///
/// Primary screen keeps the 8px padding and the extra scrollbar reservation on
/// the right, and paints from the top-left so leftover pixels stay on the right
/// and bottom. Alternate-screen mode drops that reservation, uses a uniform
/// minimum inset, and splits leftover pixels evenly.
pub fn terminal_content_layout(
    pane: Rect,
    metrics: CellMetrics,
    alternate_screen: bool,
    card_radius: f32,
) -> TerminalContentLayout {
    let content = if alternate_screen {
        let inset = alternate_screen_inset(card_radius);
        Rect::from_min_max(
            pane.min + Vec2::splat(inset),
            (pane.max - Vec2::splat(inset)).max(pane.min + Vec2::splat(inset)),
        )
    } else {
        Rect::from_min_max(
            pane.min + Vec2::splat(TERMINAL_PADDING),
            (pane.max - Vec2::splat(TERMINAL_PADDING) - Vec2::new(SCROLLBAR_WIDTH, 0.0))
                .max(pane.min + Vec2::splat(TERMINAL_PADDING)),
        )
    };
    let size = metrics.grid_size(content.size());
    let used = metrics.pixel_size(size);
    let origin = if alternate_screen {
        let spare_x = (content.width() - used.x).max(0.0);
        let spare_y = (content.height() - used.y).max(0.0);
        content.min + Vec2::new(spare_x * 0.5, spare_y * 0.5)
    } else {
        content.min
    };
    let grid_rect = if alternate_screen {
        Rect::from_min_size(origin, used)
    } else {
        content
    };
    TerminalContentLayout {
        content,
        origin,
        grid_rect,
        size,
    }
}

/// A viewport cell coordinate used for selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PartialOrd, Ord)]
pub struct CellPoint {
    /// Zero-based viewport row.
    pub row: usize,
    /// Zero-based column.
    pub col: usize,
}

impl CellPoint {
    /// Build a cell coordinate.
    pub const fn new(row: usize, col: usize) -> Self {
        Self { row, col }
    }
}

/// Inclusive selection range in viewport coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionRange {
    /// Anchor cell.
    pub start: CellPoint,
    /// Active cell.
    pub end: CellPoint,
}

impl SelectionRange {
    /// Build a range.
    pub const fn new(start: CellPoint, end: CellPoint) -> Self {
        Self { start, end }
    }

    /// The same range with `start` before `end` in row-major order.
    pub fn normalized(self) -> Self {
        if (self.end.row, self.end.col) < (self.start.row, self.start.col) {
            Self {
                start: self.end,
                end: self.start,
            }
        } else {
            self
        }
    }

    /// Whether a viewport cell is inside the selection (inclusive).
    pub fn contains(self, point: CellPoint) -> bool {
        let range = self.normalized();
        if point.row < range.start.row || point.row > range.end.row {
            return false;
        }
        if range.start.row == range.end.row {
            return point.col >= range.start.col && point.col <= range.end.col;
        }
        if point.row == range.start.row {
            return point.col >= range.start.col;
        }
        if point.row == range.end.row {
            return point.col <= range.end.col;
        }
        true
    }

    /// Lowest row touched by the range.
    pub fn first_row(self) -> usize {
        self.normalized().start.row
    }

    /// Highest row touched by the range.
    pub fn last_row(self) -> usize {
        self.normalized().end.row
    }

    /// Whether two selections touch any of the same rows.
    pub fn rows_overlap(self, other: Self) -> bool {
        self.first_row() <= other.last_row() && other.first_row() <= self.last_row()
    }
}

/// The visual style of one grouped run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunStyle {
    /// Foreground color.
    pub foreground: [u8; 3],
    /// Background color.
    pub background: [u8; 3],
    /// Bold text.
    pub bold: bool,
    /// Dim text.
    pub dim: bool,
    /// Italic text.
    pub italic: bool,
    /// Underlined text.
    pub underline: bool,
    /// Struck-through text.
    pub strikethrough: bool,
    /// This run is inside the selection.
    pub selected: bool,
}

impl RunStyle {
    /// Whether the run has no visible glyphs.
    pub fn is_invisible(&self, text: &str) -> bool {
        text.is_empty() || text.chars().all(char::is_whitespace)
    }
}

/// A contiguous group of cells with one style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphRun {
    /// First column of the run.
    pub start_col: usize,
    /// Number of grid columns occupied.
    pub cells: usize,
    /// Glyph text. Trailing wide spacers are never included. Hidden cells
    /// produce a run with empty text so their background is still painted.
    pub text: String,
    /// Visual style.
    pub style: RunStyle,
    /// This run is a single double-width glyph.
    pub wide: bool,
}

impl GlyphRun {
    /// Background rectangle for this run.
    pub fn background_rect(&self, origin: Pos2, metrics: CellMetrics) -> Rect {
        Rect::from_min_size(
            Pos2::new(origin.x + self.start_col as f32 * metrics.width, origin.y),
            Vec2::new(self.cells as f32 * metrics.width, metrics.height),
        )
    }
}

/// Cached runs for one viewport row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RowRuns {
    /// Viewport row index.
    pub row: usize,
    /// Runs in column order.
    pub runs: Vec<GlyphRun>,
}

impl RowRuns {
    /// Total glyph cells represented by the row.
    pub fn covered_cells(&self) -> usize {
        self.runs.iter().map(|run| run.cells).sum()
    }
}

/// Palette used by the terminal renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridPalette {
    /// Default background.
    pub background: [u8; 3],
    /// Default foreground.
    pub foreground: [u8; 3],
    /// Cursor color.
    pub cursor: [u8; 3],
    /// Selection background.
    pub selection_background: [u8; 3],
    /// Selection foreground.
    pub selection_foreground: [u8; 3],
    /// The 16 basic ANSI colors.
    pub ansi: [[u8; 3]; 16],
}

impl Default for GridPalette {
    fn default() -> Self {
        Self {
            background: [16, 18, 22],
            foreground: [216, 222, 233],
            cursor: [236, 239, 244],
            selection_background: [59, 91, 138],
            selection_foreground: [255, 255, 255],
            ansi: [
                [22, 24, 30],
                [191, 97, 106],
                [163, 190, 140],
                [235, 203, 139],
                [129, 161, 193],
                [180, 142, 173],
                [136, 192, 208],
                [200, 205, 212],
                [76, 86, 106],
                [191, 97, 106],
                [163, 190, 140],
                [235, 203, 139],
                [129, 161, 193],
                [180, 142, 173],
                [143, 199, 214],
                [236, 239, 244],
            ],
        }
    }
}

impl GridPalette {
    /// Resolve an indexed color: 0-15 ANSI, 16-231 6x6x6 cube, 232-255
    /// grayscale ramp.
    pub fn indexed(&self, index: u8) -> [u8; 3] {
        match index {
            0..=15 => self.ansi[index as usize],
            16..=231 => {
                let value = index - 16;
                let red = value / 36;
                let green = (value % 36) / 6;
                let blue = value % 6;
                [
                    cube_component(red),
                    cube_component(green),
                    cube_component(blue),
                ]
            }
            _ => {
                let level = 8 + (index - 232) * 10;
                [level, level, level]
            }
        }
    }

    /// RGB to report for one color query. Indexed slots use [`Self::indexed`].
    pub(crate) fn reply_rgb(&self, query: made_terminal::QueriedColor) -> [u8; 3] {
        match query {
            made_terminal::QueriedColor::Foreground => self.foreground,
            made_terminal::QueriedColor::Background => self.background,
            made_terminal::QueriedColor::Cursor => self.cursor,
            made_terminal::QueriedColor::Indexed(index) => self.indexed(index),
        }
    }

    /// Resolve a terminal color to RGB.
    pub fn resolve(&self, color: TerminalColor, default: [u8; 3]) -> [u8; 3] {
        match color {
            TerminalColor::Default => default,
            TerminalColor::Indexed(index) => self.indexed(index),
            TerminalColor::Rgb(red, green, blue) => [red, green, blue],
        }
    }

    /// Build the visual style for one cell.
    pub fn cell_style(
        &self,
        cell: &CellSnapshot,
        attributes: CellAttributes,
        selected: bool,
    ) -> RunStyle {
        let mut foreground = self.resolve(cell.fg, self.foreground);
        let mut background = self.resolve(cell.bg, self.background);

        // Classic terminal behavior: bold promotes the eight base indexed colors
        // to their bright counterpart.
        if attributes.bold {
            if let TerminalColor::Indexed(index @ 0..=7) = cell.fg {
                foreground = self.indexed(index + 8);
            }
        }
        if attributes.dim {
            foreground = [
                (foreground[0] as u16 * 2 / 3) as u8,
                (foreground[1] as u16 * 2 / 3) as u8,
                (foreground[2] as u16 * 2 / 3) as u8,
            ];
        }
        if attributes.inverse {
            std::mem::swap(&mut foreground, &mut background);
        }
        if selected {
            foreground = self.selection_foreground;
            background = self.selection_background;
        }

        RunStyle {
            foreground,
            background,
            bold: attributes.bold,
            dim: attributes.dim,
            italic: attributes.italic,
            underline: attributes.underline,
            strikethrough: attributes.strikethrough,
            selected,
        }
    }
}

fn cube_component(value: u8) -> u8 {
    if value == 0 {
        0
    } else {
        55 + value * 40
    }
}

/// Damage-driven terminal grid cache.
#[derive(Debug, Clone)]
pub struct GridRenderer {
    metrics: CellMetrics,
    size: TerminalSize,
    rows: Vec<RowRuns>,
    last_version: Option<u64>,
    last_selection: Option<SelectionRange>,
    last_palette: GridPalette,
    rebuilds: u64,
    last_rebuild_rows: usize,
    last_rebuild_full: bool,
}

impl GridRenderer {
    /// Empty renderer.
    pub fn new(metrics: CellMetrics) -> Self {
        Self {
            metrics,
            size: TerminalSize { rows: 0, cols: 0 },
            rows: Vec::new(),
            last_version: None,
            last_selection: None,
            last_palette: GridPalette::default(),
            rebuilds: 0,
            last_rebuild_rows: 0,
            last_rebuild_full: true,
        }
    }

    /// Current cell metrics.
    pub fn metrics(&self) -> CellMetrics {
        self.metrics
    }

    /// Replace metrics. A change invalidates every row.
    pub fn set_metrics(&mut self, metrics: CellMetrics) {
        if metrics != self.metrics {
            self.metrics = metrics;
            self.invalidate_all();
        }
    }

    /// Current grid size.
    pub fn size(&self) -> TerminalSize {
        self.size
    }

    /// Total rows rebuilt across all frames (tests and diagnostics).
    pub fn total_rebuilds(&self) -> u64 {
        self.rebuilds
    }

    /// Rows rebuilt by the most recent frame.
    pub fn last_rebuild_rows(&self) -> usize {
        self.last_rebuild_rows
    }

    /// Whether the most recent frame rebuilt the whole viewport.
    pub fn last_rebuild_was_full(&self) -> bool {
        self.last_rebuild_full
    }

    /// Cached rows.
    pub fn rows(&self) -> &[RowRuns] {
        &self.rows
    }

    /// Cached runs for one row.
    pub fn row(&self, row: usize) -> Option<&RowRuns> {
        self.rows.get(row)
    }

    /// Drop every cached row.
    pub fn invalidate_all(&mut self) {
        self.last_version = None;
        self.last_selection = None;
        self.rows.clear();
        self.size = TerminalSize { rows: 0, cols: 0 };
    }

    /// Rebuild damaged rows and return the cached frame.
    ///
    /// The terminal version is the primary change signal. When the version is
    /// unchanged only selection changes can dirty rows. A full damage marker,
    /// a resize, or a palette change rebuilds the whole viewport.
    pub fn frame(
        &mut self,
        snapshot: &TerminalSnapshot,
        damage: &TerminalDamage,
        selection: Option<SelectionRange>,
        palette: &GridPalette,
    ) -> &[RowRuns] {
        let size_changed = snapshot.size != self.size;
        if size_changed {
            self.size = snapshot.size;
            self.rows = vec![RowRuns::default(); snapshot.size.rows];
            self.last_version = None;
        }
        if self.rows.len() != snapshot.size.rows {
            self.rows.resize_with(snapshot.size.rows, RowRuns::default);
            self.last_version = None;
        }

        let palette_changed = *palette != self.last_palette;
        let version_changed = self.last_version != Some(snapshot.version);
        let selection_changed = selection != self.last_selection;

        let mut dirty_rows: Vec<usize> = Vec::new();
        let mut full = size_changed || palette_changed || self.last_version.is_none();

        if version_changed && !full {
            if damage.full {
                full = true;
            } else {
                for line in &damage.lines {
                    if line.row < snapshot.size.rows {
                        dirty_rows.push(line.row);
                    }
                }
            }
        }

        if selection_changed && !full {
            if let Some(old) = self.last_selection {
                for row in
                    old.first_row()..=old.last_row().min(snapshot.size.rows.saturating_sub(1))
                {
                    dirty_rows.push(row);
                }
            }
            if let Some(new) = selection {
                for row in
                    new.first_row()..=new.last_row().min(snapshot.size.rows.saturating_sub(1))
                {
                    dirty_rows.push(row);
                }
            }
        }

        dirty_rows.sort_unstable();
        dirty_rows.dedup();

        if full {
            self.rebuild_rows(snapshot, selection, palette, 0..snapshot.size.rows);
            self.last_rebuild_rows = snapshot.size.rows;
            self.last_rebuild_full = true;
        } else if !dirty_rows.is_empty() {
            for &row in &dirty_rows {
                self.rebuild_rows(snapshot, selection, palette, row..row + 1);
            }
            self.last_rebuild_rows = dirty_rows.len();
            self.last_rebuild_full = false;
        } else {
            self.last_rebuild_rows = 0;
            self.last_rebuild_full = false;
        }

        self.last_version = Some(snapshot.version);
        self.last_selection = selection;
        self.last_palette = *palette;
        &self.rows
    }

    fn rebuild_rows(
        &mut self,
        snapshot: &TerminalSnapshot,
        selection: Option<SelectionRange>,
        palette: &GridPalette,
        rows: std::ops::Range<usize>,
    ) {
        for row in rows {
            let start = row * snapshot.size.cols;
            let end = start + snapshot.size.cols;
            let cells = snapshot.cells.get(start..end).unwrap_or(&[]);
            let runs = build_row_runs(row, cells, selection, palette);
            self.rows[row] = RowRuns { row, runs };
            self.rebuilds = self.rebuilds.saturating_add(1);
        }
    }
}

/// Build the grouped runs for one row. Pure function used by tests and the
/// renderer.
pub fn build_row_runs(
    row: usize,
    cells: &[CellSnapshot],
    selection: Option<SelectionRange>,
    palette: &GridPalette,
) -> Vec<GlyphRun> {
    let mut runs = Vec::new();
    let mut current: Option<GlyphRun> = None;

    let flush = |runs: &mut Vec<GlyphRun>, current: &mut Option<GlyphRun>| {
        if let Some(run) = current.take() {
            runs.push(run);
        }
    };

    for (col, cell) in cells.iter().enumerate() {
        if cell.wide_spacer {
            // The leading wide cell already covers both columns. A spacer never
            // produces its own glyph.
            continue;
        }

        let point = CellPoint::new(row, col);
        let selected = selection.is_some_and(|range| range.contains(point));
        let style = palette.cell_style(cell, cell.attributes, selected);

        if cell.wide {
            // A double-width glyph owns two columns. Draw it as its own run so
            // the following narrow run starts at the correct cell boundary.
            flush(&mut runs, &mut current);
            runs.push(GlyphRun {
                start_col: col,
                cells: 2,
                text: if cell.attributes.hidden {
                    String::new()
                } else {
                    cell.text()
                },
                style,
                wide: true,
            });
            continue;
        }

        if cell.attributes.hidden {
            // Keep hidden cells as their own run so a following visible cell
            // keeps its exact column instead of being shifted left.
            flush(&mut runs, &mut current);
            runs.push(GlyphRun {
                start_col: col,
                cells: 1,
                text: String::new(),
                style,
                wide: false,
            });
            continue;
        }

        let matches = current
            .as_ref()
            .is_some_and(|run| run.style == style && !run.wide && run.start_col + run.cells == col);
        if !matches {
            flush(&mut runs, &mut current);
            current = Some(GlyphRun {
                start_col: col,
                cells: 1,
                text: String::new(),
                style,
                wide: false,
            });
        }
        if let Some(run) = current.as_mut() {
            // A contiguous same-style narrow cell extends the run. Hidden cells
            // contribute background but no glyph text.
            if run.start_col + run.cells == col {
                run.cells += 1;
            }
            if !cell.attributes.hidden {
                run.text.push_str(&cell.text());
            }
        }
    }

    flush(&mut runs, &mut current);
    runs
}

/// Cursor blink phase length (on or off).
pub const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(530);

/// Whether a blinking cursor is in its visible phase at `elapsed` since the
/// pane's blink epoch. Steady cursors and unfocused panes stay visible.
pub fn cursor_blink_on(blinking: bool, focused: bool, elapsed: Duration) -> bool {
    if !blinking || !focused {
        return true;
    }
    (elapsed.as_millis() / CURSOR_BLINK_INTERVAL.as_millis()).is_multiple_of(2)
}

/// Time until a focused, visible blinking cursor changes phase.
pub fn cursor_blink_repaint_after(
    cursor: CursorSnapshot,
    focused: bool,
    elapsed: Duration,
) -> Option<Duration> {
    if !focused || !cursor.visible || !cursor.blinking {
        return None;
    }
    let remainder = elapsed.as_millis() % CURSOR_BLINK_INTERVAL.as_millis();
    Some(CURSOR_BLINK_INTERVAL - Duration::from_millis(remainder as u64))
}

/// Paint options for one terminal frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintOptions {
    /// Font size in points.
    pub font_size: f32,
    /// Cell metrics used to build the runs.
    pub metrics: CellMetrics,
    /// Whether the terminal pane has keyboard focus.
    pub focused: bool,
    /// Whether the cursor blink phase is visible this frame.
    pub cursor_blink_on: bool,
}

impl Default for PaintOptions {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            metrics: CellMetrics::default(),
            focused: true,
            cursor_blink_on: true,
        }
    }
}

/// Whether the cursor should be painted for these options.
pub fn cursor_painted(cursor: CursorSnapshot, options: PaintOptions) -> bool {
    cursor.visible && (!cursor.blinking || options.cursor_blink_on)
}

/// Draw cached runs and the cursor.
pub fn paint_grid(
    painter: &Painter,
    origin: Pos2,
    grid_rect: Rect,
    rows: &[RowRuns],
    cursor: CursorSnapshot,
    options: PaintOptions,
    palette: &GridPalette,
) {
    let painter = painter.with_clip_rect(grid_rect);
    let font = FontId::monospace(options.font_size);
    let metrics = options.metrics;
    let ppp = painter.pixels_per_point();

    for row in rows {
        let y = row_top(origin.y, row.row, metrics.height, ppp);
        let next_y = row_top(origin.y, row.row + 1, metrics.height, ppp);
        if next_y < grid_rect.top() || y > grid_rect.bottom() {
            continue;
        }
        let row_origin = Pos2::new(origin.x, y);
        for run in &row.runs {
            if run.style.background != palette.background {
                let mut rect = run.background_rect(row_origin, metrics);
                rect.max.y = next_y;
                painter.rect_filled(rect, 0.0, rgb(run.style.background));
            }
        }
        for run in &row.runs {
            if run.style.is_invisible(&run.text) {
                continue;
            }
            let color = rgb(run.style.foreground);
            let spans = cell_glyphs::layout_spans(run.start_col, run.cells, &run.text, run.wide);
            for span in &spans {
                match span {
                    Span::Graphic { graphic, .. } => {
                        let span_rect = Rect::from_min_size(
                            Pos2::new(origin.x + span.col() as f32 * metrics.width, y),
                            Vec2::new(span.cols() as f32 * metrics.width, metrics.height),
                        );
                        cell_glyphs::paint_graphic(
                            &painter,
                            span_rect,
                            cell_glyphs::GlyphGrid {
                                origin,
                                col: span.col(),
                                row: row.row,
                                cols: span.cols(),
                                width: metrics.width,
                                height: metrics.height,
                            },
                            *graphic,
                            color,
                            rgb(run.style.background),
                        );
                    }
                    Span::Text {
                        start, end, col, ..
                    } => {
                        // No per-run clip. Backgrounds for this row are already
                        // filled, so a later highlight cannot cover this edge.
                        let text = &run.text[*start..*end];
                        let bold = synthetic_bold_offset(span, run.style.bold);
                        for (range, x) in
                            text_glyph_places(origin.x, *col, text, run.wide, metrics.width, ppp)
                        {
                            let pos = Pos2::new(x, y);
                            let glyph = &text[range];
                            painter.text(pos, Align2::LEFT_TOP, glyph, font.clone(), color);
                            if let Some(offset) = bold {
                                painter.text(
                                    pos + offset,
                                    Align2::LEFT_TOP,
                                    glyph,
                                    font.clone(),
                                    color,
                                );
                            }
                        }
                    }
                }
            }
        }
        for run in &row.runs {
            if run.style.is_invisible(&run.text) {
                continue;
            }
            if !run.style.underline && !run.style.strikethrough {
                continue;
            }
            let rect = run.background_rect(row_origin, metrics);
            let color = rgb(run.style.foreground);
            if run.style.underline {
                let line_y = y + metrics.height - 1.5;
                painter.line_segment(
                    [
                        Pos2::new(rect.left(), line_y),
                        Pos2::new(rect.right(), line_y),
                    ],
                    Stroke::new(1.0, color),
                );
            }
            if run.style.strikethrough {
                let line_y = y + metrics.height * 0.55;
                painter.line_segment(
                    [
                        Pos2::new(rect.left(), line_y),
                        Pos2::new(rect.right(), line_y),
                    ],
                    Stroke::new(1.0, color),
                );
            }
        }
    }

    if cursor_painted(cursor, options) {
        let cursor_pos = Pos2::new(
            origin.x + cursor.col as f32 * metrics.width,
            row_top(origin.y, cursor.row, metrics.height, ppp),
        );
        let cursor_rect = Rect::from_min_size(cursor_pos, Vec2::new(metrics.width, metrics.height));
        let color = rgb(palette.cursor);
        if options.focused {
            match cursor.shape {
                CursorShape::Block | CursorShape::HollowBlock => {
                    if cursor.shape == CursorShape::Block {
                        painter.rect_filled(cursor_rect, 0.0, color.gamma_multiply(0.55));
                    }
                    painter.rect_stroke(
                        cursor_rect.shrink(0.5),
                        0.0,
                        Stroke::new(1.0, color),
                        egui::StrokeKind::Inside,
                    );
                }
                CursorShape::Underline => {
                    painter.rect_filled(
                        Rect::from_min_max(
                            Pos2::new(cursor_rect.left(), cursor_rect.bottom() - 2.0),
                            cursor_rect.right_bottom(),
                        ),
                        0.0,
                        color,
                    );
                }
                CursorShape::Bar => {
                    painter.rect_filled(
                        Rect::from_min_size(cursor_rect.left_top(), Vec2::new(2.0, metrics.height)),
                        0.0,
                        color,
                    );
                }
                CursorShape::Hidden => {}
            }
        } else if cursor.shape != CursorShape::Hidden {
            painter.rect_stroke(
                cursor_rect.shrink(0.5),
                0.0,
                Stroke::new(1.0, color.gamma_multiply(0.6)),
                egui::StrokeKind::Inside,
            );
        }
    }
}

/// Device-pixel top of a row. Same snap as a column edge.
fn row_top(origin_y: f32, row: usize, cell_height: f32, pixels_per_point: f32) -> f32 {
    cell_glyphs::grid_edge(origin_y, row as f32, cell_height, pixels_per_point)
}

/// Where each font glyph is drawn. A narrow character's x is the device-pixel
/// edge of its own column, so a background split cannot move it.
fn text_glyph_places(
    origin_x: f32,
    start_col: usize,
    text: &str,
    wide: bool,
    cell_width: f32,
    pixels_per_point: f32,
) -> Vec<(std::ops::Range<usize>, f32)> {
    let x_at =
        |col: usize| cell_glyphs::grid_edge(origin_x, col as f32, cell_width, pixels_per_point);
    if wide {
        return vec![(0..text.len(), x_at(start_col))];
    }
    let mut places = Vec::new();
    let mut chars = text.char_indices().peekable();
    let mut col = start_col;
    while let Some((index, ch)) = chars.next() {
        let mut end = index + ch.len_utf8();
        while let Some(&(next_index, next)) = chars.peek() {
            if !cell_glyphs::is_attached(next) {
                break;
            }
            chars.next();
            end = next_index + next.len_utf8();
        }
        places.push((index..end, x_at(col)));
        col += 1;
    }
    places
}

/// Second draw for synthetic bold. Geometric marks return none so a 1px
/// square is not drawn twice 0.6px apart.
fn synthetic_bold_offset(span: &Span, bold: bool) -> Option<Vec2> {
    if bold && matches!(span, Span::Text { .. }) {
        Some(Vec2::new(0.6, 0.0))
    } else {
        None
    }
}

/// Convert an RGB triple to an egui color.
pub const fn rgb(color: [u8; 3]) -> Color32 {
    Color32::from_rgb(color[0], color[1], color[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(ch: char, fg: TerminalColor, bg: TerminalColor) -> CellSnapshot {
        CellSnapshot {
            ch,
            fg,
            bg,
            ..CellSnapshot::default()
        }
    }

    fn attributes() -> CellAttributes {
        CellAttributes::default()
    }

    #[test]
    fn cursor_blink_uses_530_ms_on_off_boundaries() {
        let on = |elapsed_ms| cursor_blink_on(true, true, Duration::from_millis(elapsed_ms));

        assert_eq!(CURSOR_BLINK_INTERVAL, Duration::from_millis(530));
        assert!(on(0));
        assert!(on(529));
        assert!(!on(530));
        assert!(!on(1059));
        assert!(on(1060));
    }

    #[test]
    fn cursor_blink_stays_on_for_steady_and_unfocused_cursors() {
        assert!(cursor_blink_on(false, true, Duration::from_millis(530)));
        assert!(cursor_blink_on(true, false, Duration::from_millis(530)));
        assert!(cursor_blink_on(true, true, Duration::ZERO));
        assert!(!cursor_blink_on(true, true, Duration::from_millis(530)));
    }

    #[test]
    fn cursor_paint_gate_keeps_steady_and_hides_off_phase_blinks() {
        let steady = CursorSnapshot {
            blinking: false,
            visible: true,
            ..CursorSnapshot::default()
        };
        let blinking = CursorSnapshot {
            blinking: true,
            ..steady
        };
        let hidden = CursorSnapshot {
            visible: false,
            ..blinking
        };
        let off_phase = PaintOptions {
            cursor_blink_on: false,
            ..PaintOptions::default()
        };
        let on_phase = PaintOptions {
            cursor_blink_on: true,
            ..PaintOptions::default()
        };

        assert!(cursor_painted(steady, off_phase));
        assert!(!cursor_painted(blinking, off_phase));
        assert!(cursor_painted(blinking, on_phase));
        assert!(!cursor_painted(hidden, on_phase));
    }

    #[test]
    fn cursor_blink_repaint_delay_targets_the_next_boundary() {
        let cursor = CursorSnapshot {
            blinking: true,
            visible: true,
            ..CursorSnapshot::default()
        };
        assert_eq!(
            cursor_blink_repaint_after(cursor, true, Duration::ZERO),
            Some(Duration::from_millis(530))
        );
        assert_eq!(
            cursor_blink_repaint_after(cursor, true, Duration::from_millis(529)),
            Some(Duration::from_millis(1))
        );
        assert_eq!(
            cursor_blink_repaint_after(cursor, true, Duration::from_millis(530)),
            Some(Duration::from_millis(530))
        );
        assert_eq!(
            cursor_blink_repaint_after(cursor, true, Duration::from_millis(1059)),
            Some(Duration::from_millis(1))
        );
        assert_eq!(
            cursor_blink_repaint_after(cursor, true, Duration::from_millis(1060)),
            Some(Duration::from_millis(530))
        );

        let steady = CursorSnapshot {
            blinking: false,
            ..cursor
        };
        assert_eq!(
            cursor_blink_repaint_after(steady, true, Duration::from_millis(530)),
            None
        );
        assert_eq!(
            cursor_blink_repaint_after(cursor, false, Duration::from_millis(530)),
            None
        );
        let hidden = CursorSnapshot {
            visible: false,
            ..cursor
        };
        assert_eq!(
            cursor_blink_repaint_after(hidden, true, Duration::from_millis(530)),
            None
        );
    }

    #[test]
    fn indexed_palette_covers_ansi_cube_and_grayscale() {
        let palette = GridPalette::default();
        assert_eq!(palette.indexed(1), palette.ansi[1]);
        assert_eq!(palette.indexed(16), [0, 0, 0]);
        assert_eq!(palette.indexed(231), [255, 255, 255]);
        assert_eq!(palette.indexed(232), [8, 8, 8]);
        assert_eq!(palette.indexed(255), [238, 238, 238]);
    }

    #[test]
    fn truecolor_and_inverse_map_to_runs() {
        let palette = GridPalette::default();
        let cell = cell(
            'x',
            TerminalColor::Rgb(1, 2, 3),
            TerminalColor::Rgb(4, 5, 6),
        );
        let style = palette.cell_style(&cell, attributes(), false);
        assert_eq!(style.foreground, [1, 2, 3]);
        assert_eq!(style.background, [4, 5, 6]);

        let mut inverse = attributes();
        inverse.inverse = true;
        let style = palette.cell_style(&cell, inverse, false);
        assert_eq!(style.foreground, [4, 5, 6]);
        assert_eq!(style.background, [1, 2, 3]);
    }

    #[test]
    fn bold_promotes_base_indexed_colors() {
        let palette = GridPalette::default();
        let cell = cell('x', TerminalColor::Indexed(2), TerminalColor::Default);
        let mut attrs = attributes();
        attrs.bold = true;
        let style = palette.cell_style(&cell, attrs, false);
        assert_eq!(style.foreground, palette.ansi[10]);
    }

    #[test]
    fn contiguous_cells_group_into_one_run_and_style_changes_split() {
        let palette = GridPalette::default();
        let cells = vec![
            cell('a', TerminalColor::Default, TerminalColor::Default),
            cell('b', TerminalColor::Default, TerminalColor::Default),
            cell('c', TerminalColor::Rgb(9, 9, 9), TerminalColor::Default),
        ];
        let runs = build_row_runs(0, &cells, None, &palette);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "ab");
        assert_eq!(runs[0].cells, 2);
        assert_eq!(runs[1].text, "c");
    }

    #[test]
    fn wide_glyphs_get_their_own_two_cell_run() {
        let palette = GridPalette::default();
        let mut wide = cell('界', TerminalColor::Default, TerminalColor::Default);
        wide.wide = true;
        let mut spacer = cell(' ', TerminalColor::Default, TerminalColor::Default);
        spacer.wide_spacer = true;
        let cells = vec![
            cell('a', TerminalColor::Default, TerminalColor::Default),
            wide,
            spacer,
            cell('b', TerminalColor::Default, TerminalColor::Default),
        ];
        let runs = build_row_runs(0, &cells, None, &palette);
        let texts: Vec<&str> = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(texts, vec!["a", "界", "b"]);
        assert_eq!(runs[1].cells, 2);
        assert!(runs[1].wide);
    }

    #[test]
    fn selection_splits_runs_and_uses_selection_colors() {
        let palette = GridPalette::default();
        let cells = vec![
            cell('a', TerminalColor::Default, TerminalColor::Default),
            cell('b', TerminalColor::Default, TerminalColor::Default),
            cell('c', TerminalColor::Default, TerminalColor::Default),
        ];
        let selection = SelectionRange::new(CellPoint::new(0, 1), CellPoint::new(0, 1));
        let runs = build_row_runs(0, &cells, Some(selection), &palette);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[1].text, "b");
        assert!(runs[1].style.selected);
        assert_eq!(runs[1].style.background, palette.selection_background);
    }

    #[test]
    fn opencode_spinner_row_keeps_sent_rgb_and_square_marks() {
        use crate::cell_glyphs::{layout_spans, Graphic, Span};
        use made_terminal::{TerminalCore, TerminalSize};

        let palette = GridPalette::default();
        let mut core = TerminalCore::with_size(TerminalSize { rows: 2, cols: 8 }, 100);
        // Vivid secondary blue, then the muted grays a terminal that answered
        // OSC 10/11 would already have blended. Neither is remapped.
        let bytes = "\u{1b}[38;2;92;156;245m■\u{1b}[38;2;100;111;127m■\u{1b}[38;2;113;121;134m⬝\u{1b}[1;38;2;99;125;161m⬝";
        core.feed(bytes.as_bytes());
        let snapshot = core.snapshot();
        let expect = [
            ('■', [92, 156, 245], false),
            ('■', [100, 111, 127], false),
            ('⬝', [113, 121, 134], false),
            ('⬝', [99, 125, 161], true),
        ];
        for (col, (ch, rgb, bold)) in expect.into_iter().enumerate() {
            let cell = snapshot.cell(0, col).expect("spinner cell");
            assert_eq!(cell.ch, ch);
            assert!(!cell.wide, "spinner marks occupy one cell");
            assert_eq!(cell.fg, TerminalColor::Rgb(rgb[0], rgb[1], rgb[2]));
            assert_eq!(cell.attributes.bold, bold);
            let style = palette.cell_style(cell, cell.attributes, false);
            assert_eq!(
                style.foreground, rgb,
                "truecolor must reach the painter unchanged"
            );
            assert_ne!(style.foreground, palette.ansi[4]);
            let text = cell.text();
            let spans = layout_spans(0, 1, &text, false);
            assert_eq!(spans.len(), 1, "one mark per cell");
            assert!(
                matches!(
                    spans[0],
                    Span::Graphic {
                        graphic: Graphic::Square(_),
                        ..
                    }
                ),
                "font text is what fragments ⬝ and overflows ■"
            );
            assert_eq!(
                synthetic_bold_offset(&spans[0], style.bold),
                None,
                "a geometric square must not be drawn twice"
            );
        }
    }

    #[test]
    fn color_queries_use_the_palette_the_grid_paints() {
        use made_terminal::{TerminalCore, TerminalSize};

        let palette = crate::theme::terminal_palette();
        let mut core = TerminalCore::with_size(TerminalSize { rows: 2, cols: 8 }, 100);
        core.feed(
            b"\x1b]10;?\x07\x1b]11;?\x07\x1b]12;?\x1b\\\x1b]4;8;?\x07\x1b]4;21;?\x07\x1b]4;244;?\x07",
        );
        let replies = String::from_utf8(core.take_replies(|query| palette.reply_rgb(query)))
            .expect("reply bytes");

        let osc = |prefix: &str, rgb: [u8; 3], terminator: &str| {
            format!(
                "\u{1b}]{prefix};rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}{terminator}",
                rgb[0], rgb[0], rgb[1], rgb[1], rgb[2], rgb[2]
            )
        };
        let expect = format!(
            "{}{}{}{}{}{}",
            osc("10", palette.foreground, "\u{7}"),
            osc("11", palette.background, "\u{7}"),
            osc("12", palette.cursor, "\u{1b}\\"),
            osc("4;8", palette.indexed(8), "\u{7}"),
            osc("4;21", palette.indexed(21), "\u{7}"),
            osc("4;244", palette.indexed(244), "\u{7}"),
        );
        assert_eq!(replies, expect);
    }

    #[test]
    fn row_origin_snaps_to_the_device_pixel() {
        let origin_y = 3.7;
        let height = 17.3;
        let ppp = 2.0;
        assert_eq!(row_top(origin_y, 0, height, ppp), 3.5);
        assert_ne!(row_top(origin_y, 0, height, ppp), origin_y);
        assert_eq!(row_top(origin_y, 2, height, ppp), 38.5);
        assert_ne!(row_top(origin_y, 2, height, ppp), origin_y + 2.0 * height);
        assert_eq!(
            row_top(origin_y, 1, height, ppp),
            cell_glyphs::grid_edge(origin_y, 1.0, height, ppp)
        );
    }

    #[test]
    fn narrow_glyph_x_depends_only_on_its_column() {
        let origin_x = 10.2;
        let width = 8.4;
        let ppp = 2.0;
        let alone = text_glyph_places(origin_x, 3, "c", false, width, ppp);
        let run = text_glyph_places(origin_x, 1, "abc", false, width, ppp);
        let expected = ((origin_x + 3.0 * width) * ppp).round() / ppp;
        assert_eq!(alone.len(), 1);
        assert_eq!(run.len(), 3);
        assert_eq!(alone[0].1, expected);
        assert_eq!(run[2].1, expected);
        let from_run_start = ((origin_x + width) * ppp).round() / ppp + 2.0 * width;
        assert_ne!(run[2].1, from_run_start);

        let wide = text_glyph_places(origin_x, 2, "界", true, width, ppp);
        assert_eq!(wide.len(), 1);
        assert_eq!(wide[0].0, 0.."界".len());
        assert_eq!(wide[0].1, ((origin_x + 2.0 * width) * ppp).round() / ppp);

        let marked = text_glyph_places(origin_x, 0, "a\u{0301}b", false, width, ppp);
        assert_eq!(marked.len(), 2);
        assert_eq!(marked[0].0, 0.."a\u{0301}".len());
        assert_eq!(marked[1].1, ((origin_x + width) * ppp).round() / ppp);

        let voiced = text_glyph_places(origin_x, 0, "a\u{3099}b", false, width, ppp);
        assert_eq!(voiced.len(), 2);
        assert_eq!(voiced[0].0, 0.."a\u{3099}".len());
        assert_eq!(voiced[0].1, marked[0].1);
        assert_eq!(voiced[1].1, ((origin_x + width) * ppp).round() / ppp);

        assert!(cell_glyphs::is_attached('\u{3099}'));
        assert!(cell_glyphs::is_attached('\u{309A}'));
        let one_cell = cell_glyphs::layout_spans(0, 1, "か\u{3099}", false);
        assert_eq!(one_cell.len(), 1);
        assert_eq!(one_cell[0].cols(), 1);
        let Span::Text { start, end, .. } = &one_cell[0] else {
            panic!("dakuten stays on the base character");
        };
        assert!("か\u{3099}"[*start..*end].contains('\u{3099}'));
        let with_next = cell_glyphs::layout_spans(0, 2, "か\u{3099}b", false);
        assert_eq!(with_next.len(), 1);
        assert_eq!(with_next[0].cols(), 2);
    }

    #[test]
    fn selection_range_is_inclusive_and_normalizes_reversed_drags() {
        let range = SelectionRange::new(CellPoint::new(2, 3), CellPoint::new(0, 1));
        assert!(range.contains(CellPoint::new(0, 1)));
        assert!(range.contains(CellPoint::new(1, 99)));
        assert!(range.contains(CellPoint::new(2, 3)));
        assert!(!range.contains(CellPoint::new(2, 4)));
        assert!(!range.contains(CellPoint::new(0, 0)));
    }
}
