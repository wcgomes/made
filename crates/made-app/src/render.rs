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

use eframe::egui::{self, Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, Vec2};
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

/// Paint options for one terminal frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintOptions {
    /// Font size in points.
    pub font_size: f32,
    /// Cell metrics used to build the runs.
    pub metrics: CellMetrics,
    /// Whether the terminal pane has keyboard focus.
    pub focused: bool,
}

impl Default for PaintOptions {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            metrics: CellMetrics::default(),
            focused: true,
        }
    }
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

    for row in rows {
        let y = origin.y + row.row as f32 * metrics.height;
        if y + metrics.height < grid_rect.top() || y > grid_rect.bottom() {
            continue;
        }
        for run in &row.runs {
            let rect = run.background_rect(Pos2::new(origin.x, y), metrics);
            if run.style.background != palette.background {
                painter.rect_filled(rect, 0.0, rgb(run.style.background));
            }
            if run.style.is_invisible(&run.text) {
                continue;
            }

            let text_pos = Pos2::new(rect.left(), y);
            let color = rgb(run.style.foreground);
            painter.text(text_pos, Align2::LEFT_TOP, &run.text, font.clone(), color);
            if run.style.bold {
                // egui's bundled monospace face has one weight; a sub-pixel
                // overdraw is the documented synthetic-bold fallback.
                painter.text(
                    text_pos + Vec2::new(0.6, 0.0),
                    Align2::LEFT_TOP,
                    &run.text,
                    font.clone(),
                    color,
                );
            }
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

    if cursor.visible {
        let cursor_pos = Pos2::new(
            origin.x + cursor.col as f32 * metrics.width,
            origin.y + cursor.row as f32 * metrics.height,
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
    fn selection_range_is_inclusive_and_normalizes_reversed_drags() {
        let range = SelectionRange::new(CellPoint::new(2, 3), CellPoint::new(0, 1));
        assert!(range.contains(CellPoint::new(0, 1)));
        assert!(range.contains(CellPoint::new(1, 99)));
        assert!(range.contains(CellPoint::new(2, 3)));
        assert!(!range.contains(CellPoint::new(2, 4)));
        assert!(!range.contains(CellPoint::new(0, 0)));
    }
}
