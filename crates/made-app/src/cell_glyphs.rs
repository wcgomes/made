//! Cell geometry for terminal chrome.
//!
//! Box drawing, block elements, braille, and dingbat crosses are drawn to the
//! cell instead of the monospace font. Font glyphs for those ranges overflow
//! the cell on the bundled face and on macOS fallback fonts, which turns
//! braille art into a solid rectangle and lets a border cover the next letter.

use eframe::egui::{self, pos2, vec2, Color32, Painter, Pos2, Rect, Stroke};

/// One placed piece of a glyph run, in column order.
#[derive(Clone, Debug)]
pub(crate) enum Span {
    /// Ordinary font text covering `cols` columns.
    Text {
        /// First column.
        col: usize,
        /// Columns occupied.
        cols: usize,
        /// Byte start in the run text.
        start: usize,
        /// Byte end in the run text.
        end: usize,
    },
    /// A character drawn as cell geometry.
    Graphic {
        /// First column.
        col: usize,
        /// Columns occupied.
        cols: usize,
        /// What to draw.
        graphic: Graphic,
    },
}

impl Span {
    /// First column of this span.
    pub(crate) fn col(&self) -> usize {
        match self {
            Span::Text { col, .. } | Span::Graphic { col, .. } => *col,
        }
    }

    /// Number of columns this span occupies.
    pub(crate) fn cols(&self) -> usize {
        match self {
            Span::Text { cols, .. } | Span::Graphic { cols, .. } => *cols,
        }
    }
}

/// A terminal-chrome glyph drawn inside one cell.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Graphic {
    /// Braille pattern bits, dot 1 in the low bit.
    Braille(u8),
    /// Box-drawing arms.
    Box(BoxDraw),
    /// Solid fractions of the cell.
    Fill(Fill),
    /// Translucent full-cell shade. `alpha` is 0–1.
    Shade(f32),
    /// Inset X for dingbat crosses such as U+2715, the session-tab close mark.
    Cross,
}

/// Up to three axis-aligned pieces of a block element.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fill {
    rects: [UnitRect; 3],
    len: u8,
}

impl Fill {
    fn rects(&self) -> &[UnitRect] {
        &self.rects[..usize::from(self.len)]
    }
}

/// A rectangle in cell units. Origin is the top-left. Each edge is 0–1.
#[derive(Clone, Copy, Debug)]
struct UnitRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// Stroke weight of one box-drawing arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineWeight {
    /// No arm.
    None,
    /// Single light stroke.
    Light,
    /// Single heavy stroke.
    Heavy,
    /// Two parallel light strokes.
    Double,
}

/// Which diagonal strokes a box-drawing character includes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Diagonal {
    /// No diagonal.
    None,
    /// Upper right to lower left.
    Forward,
    /// Upper left to lower right.
    Back,
    /// Both diagonals.
    Cross,
}

/// The arms of one box-drawing character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoxDraw {
    /// Arm toward the left edge.
    pub(crate) left: LineWeight,
    /// Arm toward the right edge.
    pub(crate) right: LineWeight,
    /// Arm toward the top edge.
    pub(crate) up: LineWeight,
    /// Arm toward the bottom edge.
    pub(crate) down: LineWeight,
    /// Join the two arms with a quarter ellipse instead of a center corner.
    pub(crate) arc: bool,
    /// Dash count. Zero is a solid stroke.
    pub(crate) dashes: u8,
    /// Diagonal strokes, used by the slash characters.
    pub(crate) diagonal: Diagonal,
}

/// Split `text` into column-aligned spans.
///
/// A geometric character is its own span, so its ink cannot shift or cover the
/// following letter. Combining marks stay on the base character's column.
/// A wide run is one span covering `cells` columns.
#[must_use]
pub(crate) fn layout_spans(start_col: usize, cells: usize, text: &str, wide: bool) -> Vec<Span> {
    if cells == 0 || text.is_empty() {
        return Vec::new();
    }
    if wide {
        let graphic = text.chars().next().and_then(classify);
        return vec![span_for(start_col, cells, 0, text.len(), graphic)];
    }

    let mut spans = Vec::new();
    let mut chars = text.char_indices().peekable();
    let end_col = start_col.saturating_add(cells);
    let mut col = start_col;
    let mut text_start: Option<usize> = None;
    let mut text_end = 0;
    let mut text_col = start_col;
    let mut text_cols = 0;

    while col < end_col {
        let Some((index, ch)) = chars.next() else {
            break;
        };
        let mut end = index + ch.len_utf8();
        while let Some(&(next_index, next)) = chars.peek() {
            if !is_attached(next) {
                break;
            }
            chars.next();
            end = next_index + next.len_utf8();
        }

        if let Some(graphic) = classify(ch) {
            if text_cols > 0 {
                spans.push(Span::Text {
                    col: text_col,
                    cols: text_cols,
                    start: text_start.unwrap_or(index),
                    end: text_end,
                });
                text_cols = 0;
                text_start = None;
            }
            spans.push(Span::Graphic {
                col,
                cols: 1,
                graphic,
            });
        } else {
            if text_cols == 0 {
                text_col = col;
                text_start = Some(index);
            }
            text_cols += 1;
            text_end = end;
        }
        col += 1;
    }

    if text_cols > 0 {
        spans.push(Span::Text {
            col: text_col,
            cols: text_cols,
            start: text_start.unwrap_or(0),
            end: text_end,
        });
    }
    spans
}

fn span_for(col: usize, cols: usize, start: usize, end: usize, graphic: Option<Graphic>) -> Span {
    match graphic {
        Some(graphic) => Span::Graphic { col, cols, graphic },
        None => Span::Text {
            col,
            cols,
            start,
            end,
        },
    }
}

/// Draw one geometric glyph inside `rect`.
pub(crate) fn paint_graphic(
    painter: &Painter,
    rect: Rect,
    graphic: Graphic,
    fg: Color32,
    bg: Color32,
) {
    match graphic {
        Graphic::Braille(bits) => {
            let painter = painter.with_clip_rect(rect);
            paint_braille(&painter, rect, bits, fg);
        }
        Graphic::Box(draw) => {
            let painter = painter.with_clip_rect(rect.expand(0.75));
            paint_box(&painter, rect, draw, fg, bg);
        }
        Graphic::Fill(fill) => paint_fill(painter, rect, fill, fg),
        Graphic::Shade(alpha) => {
            painter.rect_filled(rect, 0.0, fg.gamma_multiply(alpha.clamp(0.0, 1.0)));
        }
        Graphic::Cross => {
            let painter = painter.with_clip_rect(rect);
            paint_cross(&painter, rect, fg);
        }
    }
}

fn classify(ch: char) -> Option<Graphic> {
    if let Some(bits) = braille_bits(ch) {
        return Some(Graphic::Braille(bits));
    }
    if is_cross(ch) {
        return Some(Graphic::Cross);
    }
    if let Some(draw) = box_draw(ch) {
        return Some(Graphic::Box(draw));
    }
    block_graphic(ch)
}

/// Dingbat crosses. The bundled monospace face has U+00D7 but not these, so a
/// system fallback draws a stray box where a tab close "x" should be.
fn is_cross(ch: char) -> bool {
    matches!(ch, '\u{2715}' | '\u{2716}' | '\u{2717}' | '\u{2718}')
}

fn paint_cross(painter: &Painter, rect: Rect, fg: Color32) {
    let inset_x = rect.width() * 0.28;
    let inset_y = rect.height() * 0.32;
    let top = rect.top() + inset_y;
    let bottom = rect.bottom() - inset_y;
    let left = rect.left() + inset_x;
    let right = rect.right() - inset_x;
    let stroke = Stroke::new(stroke_width(rect).clamp(1.0, 2.0), fg);
    painter.line_segment([pos2(left, top), pos2(right, bottom)], stroke);
    painter.line_segment([pos2(right, top), pos2(left, bottom)], stroke);
}

fn braille_bits(ch: char) -> Option<u8> {
    let code = u32::from(ch);
    if (0x2800..=0x28FF).contains(&code) {
        Some((code - 0x2800) as u8)
    } else {
        None
    }
}

fn is_attached(ch: char) -> bool {
    matches!(
        ch,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FE20}'..='\u{FE2F}'
            | '\u{200D}'
    )
}

fn paint_braille(painter: &Painter, rect: Rect, bits: u8, fg: Color32) {
    if bits == 0 {
        return;
    }
    let slot_w = rect.width() / 2.0;
    let slot_h = rect.height() / 4.0;
    let radius = slot_w.min(slot_h) * 0.36;
    if radius < 0.2 {
        return;
    }
    // Dot order is the Unicode braille cell: 1 4 / 2 5 / 3 6 / 7 8.
    const DOTS: [(f32, f32); 8] = [
        (0.0, 0.0),
        (0.0, 1.0),
        (0.0, 2.0),
        (1.0, 0.0),
        (1.0, 1.0),
        (1.0, 2.0),
        (0.0, 3.0),
        (1.0, 3.0),
    ];
    for (bit, (col, row)) in DOTS.iter().enumerate() {
        if bits & (1 << bit) == 0 {
            continue;
        }
        let center = pos2(
            rect.left() + (*col + 0.5) * slot_w,
            rect.top() + (*row + 0.5) * slot_h,
        );
        painter.circle_filled(center, radius, fg);
    }
}

fn paint_fill(painter: &Painter, rect: Rect, fill: Fill, fg: Color32) {
    let grow = 0.3;
    let painter = painter.with_clip_rect(rect.expand(grow));
    for unit in fill.rects() {
        let mut piece = Rect::from_min_size(
            pos2(
                rect.left() + rect.width() * unit.x,
                rect.top() + rect.height() * unit.y,
            ),
            vec2(rect.width() * unit.w, rect.height() * unit.h),
        );
        if unit.x <= 0.001 {
            piece.min.x -= grow;
        }
        if unit.y <= 0.001 {
            piece.min.y -= grow;
        }
        if unit.x + unit.w >= 0.999 {
            piece.max.x += grow;
        }
        if unit.y + unit.h >= 0.999 {
            piece.max.y += grow;
        }
        painter.rect_filled(piece, 0.0, fg);
    }
}

fn paint_box(painter: &Painter, rect: Rect, draw: BoxDraw, fg: Color32, bg: Color32) {
    if draw.arc {
        paint_arc(painter, rect, draw, fg);
        return;
    }
    if draw.diagonal != Diagonal::None {
        paint_diagonal(painter, rect, draw.diagonal, fg);
        return;
    }
    paint_arms(painter, rect, draw, fg, bg, false);
    if is_double(draw.left) || is_double(draw.right) || is_double(draw.up) || is_double(draw.down) {
        paint_arms(painter, rect, draw, fg, bg, true);
    }
}

fn is_double(weight: LineWeight) -> bool {
    weight == LineWeight::Double
}

fn paint_arms(painter: &Painter, rect: Rect, draw: BoxDraw, fg: Color32, bg: Color32, punch: bool) {
    let light = stroke_width(rect);
    let center = rect.center();
    paint_axis(
        painter,
        rect,
        center,
        draw.left,
        draw.right,
        true,
        draw.dashes,
        light,
        fg,
        bg,
        punch,
    );
    paint_axis(
        painter,
        rect,
        center,
        draw.up,
        draw.down,
        false,
        draw.dashes,
        light,
        fg,
        bg,
        punch,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_axis(
    painter: &Painter,
    rect: Rect,
    center: Pos2,
    toward_min: LineWeight,
    toward_max: LineWeight,
    horizontal: bool,
    dashes: u8,
    light: f32,
    fg: Color32,
    bg: Color32,
    punch: bool,
) {
    if toward_min == LineWeight::None && toward_max == LineWeight::None {
        return;
    }
    if toward_min == toward_max {
        let (start, end) = if horizontal {
            (pos2(rect.left(), center.y), pos2(rect.right(), center.y))
        } else {
            (pos2(center.x, rect.top()), pos2(center.x, rect.bottom()))
        };
        paint_segment(
            painter,
            nudge_edge(rect, start),
            nudge_edge(rect, end),
            toward_min,
            dashes,
            light,
            fg,
            bg,
            punch,
        );
        return;
    }
    if toward_min != LineWeight::None {
        let end = if horizontal {
            pos2(rect.left(), center.y)
        } else {
            pos2(center.x, rect.top())
        };
        paint_segment(
            painter,
            center,
            nudge_edge(rect, end),
            toward_min,
            dashes,
            light,
            fg,
            bg,
            punch,
        );
    }
    if toward_max != LineWeight::None {
        let end = if horizontal {
            pos2(rect.right(), center.y)
        } else {
            pos2(center.x, rect.bottom())
        };
        paint_segment(
            painter,
            center,
            nudge_edge(rect, end),
            toward_max,
            dashes,
            light,
            fg,
            bg,
            punch,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_segment(
    painter: &Painter,
    start: Pos2,
    end: Pos2,
    weight: LineWeight,
    dashes: u8,
    light: f32,
    fg: Color32,
    bg: Color32,
    punch: bool,
) {
    if punch {
        if weight == LineWeight::Double {
            painter.line_segment([start, end], Stroke::new(light, bg));
        }
        return;
    }
    if dashes >= 2 {
        let parts = f32::from(dashes) * 2.0 - 1.0;
        let delta = end - start;
        for index in 0..dashes {
            let from = f32::from(index) * 2.0 / parts;
            let to = (f32::from(index) * 2.0 + 1.0) / parts;
            paint_solid(
                painter,
                start + delta * from,
                start + delta * to,
                weight,
                light,
                fg,
            );
        }
        return;
    }
    paint_solid(painter, start, end, weight, light, fg);
}

fn paint_solid(
    painter: &Painter,
    start: Pos2,
    end: Pos2,
    weight: LineWeight,
    light: f32,
    fg: Color32,
) {
    let width = match weight {
        LineWeight::None => return,
        LineWeight::Light => light,
        LineWeight::Heavy => (light * 2.0).max(light + 1.0),
        LineWeight::Double => light * 3.0,
    };
    painter.line_segment([start, end], Stroke::new(width, fg));
}

fn paint_arc(painter: &Painter, rect: Rect, draw: BoxDraw, fg: Color32) {
    let rx = rect.width() * 0.5;
    let ry = rect.height() * 0.5;
    // Screen angles: 0 is +x, and positive sine points down.
    let (center, start, end) = if draw.right != LineWeight::None && draw.down != LineWeight::None {
        (
            rect.right_bottom(),
            std::f32::consts::PI,
            std::f32::consts::FRAC_PI_2 * 3.0,
        )
    } else if draw.left != LineWeight::None && draw.down != LineWeight::None {
        (rect.left_bottom(), 0.0, -std::f32::consts::FRAC_PI_2)
    } else if draw.left != LineWeight::None && draw.up != LineWeight::None {
        (rect.left_top(), 0.0, std::f32::consts::FRAC_PI_2)
    } else {
        (
            rect.right_top(),
            std::f32::consts::FRAC_PI_2,
            std::f32::consts::PI,
        )
    };
    let steps = 6;
    let mut points = Vec::with_capacity(steps + 1);
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let theta = start + (end - start) * t;
        points.push(pos2(
            center.x + rx * theta.cos(),
            center.y + ry * theta.sin(),
        ));
    }
    if let Some(first) = points.first_mut() {
        *first = nudge_edge(rect, *first);
    }
    if let Some(last) = points.last_mut() {
        *last = nudge_edge(rect, *last);
    }
    painter.add(egui::epaint::PathShape::line(
        points,
        Stroke::new(stroke_width(rect), fg),
    ));
}

fn paint_diagonal(painter: &Painter, rect: Rect, diagonal: Diagonal, fg: Color32) {
    let stroke = Stroke::new(stroke_width(rect), fg);
    let top_left = nudge_edge(rect, rect.left_top());
    let top_right = nudge_edge(rect, rect.right_top());
    let bottom_left = nudge_edge(rect, rect.left_bottom());
    let bottom_right = nudge_edge(rect, rect.right_bottom());
    if matches!(diagonal, Diagonal::Forward | Diagonal::Cross) {
        painter.line_segment([top_right, bottom_left], stroke);
    }
    if matches!(diagonal, Diagonal::Back | Diagonal::Cross) {
        painter.line_segment([top_left, bottom_right], stroke);
    }
}

fn nudge_edge(rect: Rect, point: Pos2) -> Pos2 {
    let nudge = 0.75;
    let mut point = point;
    if (point.x - rect.left()).abs() < 0.2 {
        point.x -= nudge;
    } else if (point.x - rect.right()).abs() < 0.2 {
        point.x += nudge;
    }
    if (point.y - rect.top()).abs() < 0.2 {
        point.y -= nudge;
    } else if (point.y - rect.bottom()).abs() < 0.2 {
        point.y += nudge;
    }
    point
}

fn stroke_width(rect: Rect) -> f32 {
    (rect.width().min(rect.height()) / 8.0).clamp(1.0, 2.5)
}

fn arms(left: LineWeight, right: LineWeight, up: LineWeight, down: LineWeight) -> BoxDraw {
    BoxDraw {
        left,
        right,
        up,
        down,
        arc: false,
        dashes: 0,
        diagonal: Diagonal::None,
    }
}

fn horizontal(weight: LineWeight) -> BoxDraw {
    arms(weight, weight, LineWeight::None, LineWeight::None)
}

fn vertical(weight: LineWeight) -> BoxDraw {
    arms(LineWeight::None, LineWeight::None, weight, weight)
}

fn dashed(draw: BoxDraw, dashes: u8) -> BoxDraw {
    BoxDraw { dashes, ..draw }
}

fn arc(draw: BoxDraw) -> BoxDraw {
    BoxDraw { arc: true, ..draw }
}

fn slash(diagonal: Diagonal) -> BoxDraw {
    BoxDraw {
        diagonal,
        ..arms(
            LineWeight::None,
            LineWeight::None,
            LineWeight::None,
            LineWeight::None,
        )
    }
}

fn box_draw(ch: char) -> Option<BoxDraw> {
    use LineWeight::{Double as D, Heavy as H, Light as L, None as N};
    Some(match ch {
        '\u{2500}' => horizontal(L),
        '\u{2501}' => horizontal(H),
        '\u{2502}' => vertical(L),
        '\u{2503}' => vertical(H),
        '\u{2504}' => dashed(horizontal(L), 3),
        '\u{2505}' => dashed(horizontal(H), 3),
        '\u{2506}' => dashed(vertical(L), 3),
        '\u{2507}' => dashed(vertical(H), 3),
        '\u{2508}' => dashed(horizontal(L), 4),
        '\u{2509}' => dashed(horizontal(H), 4),
        '\u{250A}' => dashed(vertical(L), 4),
        '\u{250B}' => dashed(vertical(H), 4),
        '\u{250C}' => arms(N, L, N, L),
        '\u{250D}' => arms(N, L, N, H),
        '\u{250E}' => arms(N, H, N, L),
        '\u{250F}' => arms(N, H, N, H),
        '\u{2510}' => arms(L, N, N, L),
        '\u{2511}' => arms(L, N, N, H),
        '\u{2512}' => arms(H, N, N, L),
        '\u{2513}' => arms(H, N, N, H),
        '\u{2514}' => arms(N, L, L, N),
        '\u{2515}' => arms(N, L, H, N),
        '\u{2516}' => arms(N, H, L, N),
        '\u{2517}' => arms(N, H, H, N),
        '\u{2518}' => arms(L, N, L, N),
        '\u{2519}' => arms(L, N, H, N),
        '\u{251A}' => arms(H, N, L, N),
        '\u{251B}' => arms(H, N, H, N),
        '\u{251C}' => arms(N, L, L, L),
        '\u{251D}' => arms(N, H, L, L),
        '\u{251E}' => arms(N, L, H, L),
        '\u{251F}' => arms(N, L, L, H),
        '\u{2520}' => arms(N, L, H, H),
        '\u{2521}' => arms(N, H, H, L),
        '\u{2522}' => arms(N, H, L, H),
        '\u{2523}' => arms(N, H, H, H),
        '\u{2524}' => arms(L, N, L, L),
        '\u{2525}' => arms(H, N, L, L),
        '\u{2526}' => arms(L, N, H, L),
        '\u{2527}' => arms(L, N, L, H),
        '\u{2528}' => arms(L, N, H, H),
        '\u{2529}' => arms(H, N, H, L),
        '\u{252A}' => arms(H, N, L, H),
        '\u{252B}' => arms(H, N, H, H),
        '\u{252C}' => arms(L, L, N, L),
        '\u{252D}' => arms(H, L, N, L),
        '\u{252E}' => arms(L, H, N, L),
        '\u{252F}' => arms(H, H, N, L),
        '\u{2530}' => arms(L, L, N, H),
        '\u{2531}' => arms(H, L, N, H),
        '\u{2532}' => arms(L, H, N, H),
        '\u{2533}' => arms(H, H, N, H),
        '\u{2534}' => arms(L, L, L, N),
        '\u{2535}' => arms(H, L, L, N),
        '\u{2536}' => arms(L, H, L, N),
        '\u{2537}' => arms(H, H, L, N),
        '\u{2538}' => arms(L, L, H, N),
        '\u{2539}' => arms(H, L, H, N),
        '\u{253A}' => arms(L, H, H, N),
        '\u{253B}' => arms(H, H, H, N),
        '\u{253C}' => arms(L, L, L, L),
        '\u{253D}' => arms(H, L, L, L),
        '\u{253E}' => arms(L, H, L, L),
        '\u{253F}' => arms(H, H, L, L),
        '\u{2540}' => arms(L, L, H, L),
        '\u{2541}' => arms(L, L, L, H),
        '\u{2542}' => arms(L, L, H, H),
        '\u{2543}' => arms(H, L, H, L),
        '\u{2544}' => arms(L, H, H, L),
        '\u{2545}' => arms(H, L, L, H),
        '\u{2546}' => arms(L, H, L, H),
        '\u{2547}' => arms(H, H, H, L),
        '\u{2548}' => arms(H, H, L, H),
        '\u{2549}' => arms(H, L, H, H),
        '\u{254A}' => arms(L, H, H, H),
        '\u{254B}' => arms(H, H, H, H),
        '\u{254C}' => dashed(horizontal(L), 2),
        '\u{254D}' => dashed(horizontal(H), 2),
        '\u{254E}' => dashed(vertical(L), 2),
        '\u{254F}' => dashed(vertical(H), 2),
        '\u{2550}' => horizontal(D),
        '\u{2551}' => vertical(D),
        '\u{2552}' => arms(N, D, N, L),
        '\u{2553}' => arms(N, L, N, D),
        '\u{2554}' => arms(N, D, N, D),
        '\u{2555}' => arms(D, N, N, L),
        '\u{2556}' => arms(L, N, N, D),
        '\u{2557}' => arms(D, N, N, D),
        '\u{2558}' => arms(N, D, L, N),
        '\u{2559}' => arms(N, L, D, N),
        '\u{255A}' => arms(N, D, D, N),
        '\u{255B}' => arms(D, N, L, N),
        '\u{255C}' => arms(L, N, D, N),
        '\u{255D}' => arms(D, N, D, N),
        '\u{255E}' => arms(N, D, L, L),
        '\u{255F}' => arms(N, L, D, D),
        '\u{2560}' => arms(N, D, D, D),
        '\u{2561}' => arms(D, N, L, L),
        '\u{2562}' => arms(L, N, D, D),
        '\u{2563}' => arms(D, N, D, D),
        '\u{2564}' => arms(D, D, N, L),
        '\u{2565}' => arms(L, L, N, D),
        '\u{2566}' => arms(D, D, N, D),
        '\u{2567}' => arms(D, D, L, N),
        '\u{2568}' => arms(L, L, D, N),
        '\u{2569}' => arms(D, D, D, N),
        '\u{256A}' => arms(D, D, L, L),
        '\u{256B}' => arms(L, L, D, D),
        '\u{256C}' => arms(D, D, D, D),
        '\u{256D}' => arc(arms(N, L, N, L)),
        '\u{256E}' => arc(arms(L, N, N, L)),
        '\u{256F}' => arc(arms(L, N, L, N)),
        '\u{2570}' => arc(arms(N, L, L, N)),
        '\u{2571}' => slash(Diagonal::Forward),
        '\u{2572}' => slash(Diagonal::Back),
        '\u{2573}' => slash(Diagonal::Cross),
        '\u{2574}' => arms(L, N, N, N),
        '\u{2575}' => arms(N, N, L, N),
        '\u{2576}' => arms(N, L, N, N),
        '\u{2577}' => arms(N, N, N, L),
        '\u{2578}' => arms(H, N, N, N),
        '\u{2579}' => arms(N, N, H, N),
        '\u{257A}' => arms(N, H, N, N),
        '\u{257B}' => arms(N, N, N, H),
        '\u{257C}' => arms(L, H, N, N),
        '\u{257D}' => arms(N, N, L, H),
        '\u{257E}' => arms(H, L, N, N),
        '\u{257F}' => arms(N, N, H, L),
        _ => return None,
    })
}

fn one(rect: UnitRect) -> Fill {
    Fill {
        rects: [rect, rect, rect],
        len: 1,
    }
}

fn two(first: UnitRect, second: UnitRect) -> Fill {
    Fill {
        rects: [first, second, second],
        len: 2,
    }
}

fn three(first: UnitRect, second: UnitRect, third: UnitRect) -> Fill {
    Fill {
        rects: [first, second, third],
        len: 3,
    }
}

fn block_graphic(ch: char) -> Option<Graphic> {
    const UL: UnitRect = UnitRect {
        x: 0.0,
        y: 0.0,
        w: 0.5,
        h: 0.5,
    };
    const UR: UnitRect = UnitRect {
        x: 0.5,
        y: 0.0,
        w: 0.5,
        h: 0.5,
    };
    const LL: UnitRect = UnitRect {
        x: 0.0,
        y: 0.5,
        w: 0.5,
        h: 0.5,
    };
    const LR: UnitRect = UnitRect {
        x: 0.5,
        y: 0.5,
        w: 0.5,
        h: 0.5,
    };
    let fill = match ch {
        '\u{2580}' => one(UnitRect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 0.5,
        }),
        '\u{2581}' => one(UnitRect {
            x: 0.0,
            y: 0.875,
            w: 1.0,
            h: 0.125,
        }),
        '\u{2582}' => one(UnitRect {
            x: 0.0,
            y: 0.75,
            w: 1.0,
            h: 0.25,
        }),
        '\u{2583}' => one(UnitRect {
            x: 0.0,
            y: 0.625,
            w: 1.0,
            h: 0.375,
        }),
        '\u{2584}' => one(UnitRect {
            x: 0.0,
            y: 0.5,
            w: 1.0,
            h: 0.5,
        }),
        '\u{2585}' => one(UnitRect {
            x: 0.0,
            y: 0.375,
            w: 1.0,
            h: 0.625,
        }),
        '\u{2586}' => one(UnitRect {
            x: 0.0,
            y: 0.25,
            w: 1.0,
            h: 0.75,
        }),
        '\u{2587}' => one(UnitRect {
            x: 0.0,
            y: 0.125,
            w: 1.0,
            h: 0.875,
        }),
        '\u{2588}' => one(UnitRect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        }),
        '\u{2589}' => one(UnitRect {
            x: 0.0,
            y: 0.0,
            w: 0.875,
            h: 1.0,
        }),
        '\u{258A}' => one(UnitRect {
            x: 0.0,
            y: 0.0,
            w: 0.75,
            h: 1.0,
        }),
        '\u{258B}' => one(UnitRect {
            x: 0.0,
            y: 0.0,
            w: 0.625,
            h: 1.0,
        }),
        '\u{258C}' => one(UnitRect {
            x: 0.0,
            y: 0.0,
            w: 0.5,
            h: 1.0,
        }),
        '\u{258D}' => one(UnitRect {
            x: 0.0,
            y: 0.0,
            w: 0.375,
            h: 1.0,
        }),
        '\u{258E}' => one(UnitRect {
            x: 0.0,
            y: 0.0,
            w: 0.25,
            h: 1.0,
        }),
        '\u{258F}' => one(UnitRect {
            x: 0.0,
            y: 0.0,
            w: 0.125,
            h: 1.0,
        }),
        '\u{2590}' => one(UnitRect {
            x: 0.5,
            y: 0.0,
            w: 0.5,
            h: 1.0,
        }),
        '\u{2591}' => return Some(Graphic::Shade(0.25)),
        '\u{2592}' => return Some(Graphic::Shade(0.5)),
        '\u{2593}' => return Some(Graphic::Shade(0.75)),
        '\u{2594}' => one(UnitRect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 0.125,
        }),
        '\u{2595}' => one(UnitRect {
            x: 0.875,
            y: 0.0,
            w: 0.125,
            h: 1.0,
        }),
        '\u{2596}' => one(LL),
        '\u{2597}' => one(LR),
        '\u{2598}' => one(UL),
        '\u{2599}' => three(UL, LL, LR),
        '\u{259A}' => two(UL, LR),
        '\u{259B}' => three(UL, UR, LL),
        '\u{259C}' => three(UL, UR, LR),
        '\u{259D}' => one(UR),
        '\u{259E}' => two(UR, LL),
        '\u{259F}' => three(UR, LL, LR),
        _ => return None,
    };
    Some(Graphic::Fill(fill))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(left: f32, right: f32) -> bool {
        (left - right).abs() < 1e-4
    }

    fn fill_of(ch: char) -> Fill {
        match classify(ch) {
            Some(Graphic::Fill(fill)) => fill,
            other => panic!("expected a fill for {ch:?}, got {other:?}"),
        }
    }

    #[test]
    fn braille_blank_full_and_open_center() {
        assert_eq!(braille_bits('\u{2800}'), Some(0));
        assert_eq!(braille_bits('\u{28FF}'), Some(0xFF));
        let hole = braille_bits('\u{28ED}').expect("pattern");
        assert_eq!(hole & (1 << 1), 0, "dot 2 stays empty");
        assert_eq!(hole & (1 << 4), 0, "dot 5 stays empty");
        assert_ne!(hole, 0);
        assert_ne!(hole, 0xFF);
    }

    #[test]
    fn light_corner_joins_the_adjacent_edges() {
        let corner = box_draw('┌').expect("corner");
        let horizontal = box_draw('─').expect("horizontal");
        let vertical = box_draw('│').expect("vertical");

        assert_eq!(corner.right, LineWeight::Light);
        assert_eq!(corner.down, LineWeight::Light);
        assert_eq!(corner.left, LineWeight::None);
        assert_eq!(corner.up, LineWeight::None);
        assert!(!corner.arc);
        assert_eq!(horizontal.left, corner.right);
        assert_eq!(horizontal.right, LineWeight::Light);
        assert_eq!(vertical.up, corner.down);
        assert_eq!(vertical.down, LineWeight::Light);
    }

    #[test]
    fn rounded_corners_join_the_same_edges() {
        let down_right = box_draw('╭').expect("arc");
        assert!(down_right.arc);
        assert_eq!(down_right.right, box_draw('─').expect("horizontal").left);
        assert_eq!(down_right.down, box_draw('│').expect("vertical").up);

        assert!(box_draw('╮').expect("arc").arc);
        assert!(box_draw('╯').expect("arc").arc);
        assert!(box_draw('╰').expect("arc").arc);
    }

    #[test]
    fn half_blocks_meet_inside_the_cell() {
        let left = fill_of('▌').rects()[0];
        let right = fill_of('▐').rects()[0];
        let full = fill_of('█').rects()[0];

        assert!(near(left.x + left.w, right.x));
        assert!(left.x >= 0.0 && left.x + left.w <= 1.0);
        assert!(right.x >= 0.0 && right.x + right.w <= 1.0);
        assert!(right.y >= 0.0 && right.y + right.h <= 1.0);
        assert!(near(full.x, 0.0) && near(full.y, 0.0));
        assert!(near(full.w, 1.0) && near(full.h, 1.0));
    }

    #[test]
    fn session_tab_close_mark_stays_in_its_column() {
        let spans = layout_spans(4, 2, "\u{2715}+", false);

        assert!(matches!(
            spans[0],
            Span::Graphic {
                graphic: Graphic::Cross,
                ..
            }
        ));
        assert_eq!(spans[0].col(), 4);
        assert_eq!(spans[0].cols(), 1);
        assert_eq!(spans[1].col(), 5);
        assert!(matches!(classify('\u{2716}'), Some(Graphic::Cross)));
        assert!(matches!(classify('\u{2717}'), Some(Graphic::Cross)));
        assert!(matches!(classify('\u{2718}'), Some(Graphic::Cross)));
    }

    #[test]
    fn graphic_column_does_not_cover_the_next_letter() {
        let spans = layout_spans(0, 2, "╭A", false);

        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].col(), 0);
        assert_eq!(spans[0].cols(), 1);
        assert!(matches!(spans[0], Span::Graphic { .. }));
        assert_eq!(spans[1].col(), 1);
        assert_eq!(spans[1].cols(), 1);
        assert!(spans[0].col() + spans[0].cols() <= spans[1].col());
        assert!(matches!(
            spans[1],
            Span::Text {
                start: 3,
                end: 4,
                ..
            }
        ));
    }

    #[test]
    fn combining_mark_stays_on_its_base_column() {
        let spans = layout_spans(0, 2, "a\u{0301}b", false);

        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].cols(), 2);
    }

    #[test]
    fn chrome_ranges_are_fully_classified() {
        for code in 0x2500u32..=0x257F {
            let ch = char::from_u32(code).expect("box drawing");
            assert!(classify(ch).is_some(), "U+{code:04X} missing");
        }
        for code in 0x2580u32..=0x259F {
            let ch = char::from_u32(code).expect("block");
            assert!(classify(ch).is_some(), "U+{code:04X} missing");
        }
        for code in 0x2800u32..=0x28FF {
            let ch = char::from_u32(code).expect("braille");
            assert!(
                matches!(classify(ch), Some(Graphic::Braille(_))),
                "U+{code:04X} missing"
            );
        }
    }

    #[test]
    fn bundled_monospace_has_a_multiplication_sign() {
        let ctx = egui::Context::default();
        let mut has = false;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            has = ui
                .ctx()
                .fonts_mut(|fonts| fonts.has_glyph(&egui::FontId::monospace(13.0), '×'));
        });
        output.textures_delta.clear();
        assert!(has, "the OpenCode tab close mark needs its own geometry");
    }
}
