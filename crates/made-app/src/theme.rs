//! “Calm Editor” design tokens and egui theme installation.
//!
//! The contract is `specs/changes/made-ui-ux-v1/design.md`. Terminal, preview, and picker surfaces
//! stay opaque; the top chrome band and the Explorer card use semi-transparent
//! dark fills as an in-app frosted-glass approximation. A native backdrop
//! (macOS vibrancy or Windows Mica) can be layered behind the window when the
//! platform supports it; the in-app glass remains legible on its own and stays
//! the default on every platform without that effect. Keep the palette and
//! geometry here rather than scattering literals through the views.

use eframe::egui::style::{WidgetVisuals, Widgets};
use eframe::egui::text::LayoutJob;
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Frame, Margin, RichText, Shadow, Stroke, StrokeKind,
    TextFormat, TextStyle, Ui, UiBuilder, Visuals,
};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Global kill switch for the translucent native window.
///
/// When `false`, no transparency is requested and [`clear_color`] is opaque.
/// The rest of the UI is identical; it simply renders on the solid backdrop.
pub const TRANSLUCENT_WINDOW: bool = true;

// ── Colors ──────────────────────────────────────────────────────────────────

// The default clear color is deliberately darker than the glass surfaces:
// alpha 190 keeps the window translucent without making text chase a bright
// desktop. When a native compositor backdrop is active, alpha 150 lets the OS
// material read through; the glass fills below carry the text contrast.
pub const BG_CLEAR: Color32 = Color32::from_rgba_unmultiplied_const(10, 11, 14, 190);
pub const BG_CLEAR_NATIVE: Color32 = Color32::from_rgba_unmultiplied_const(10, 11, 14, 150);
pub const BG_CLEAR_SOLID: Color32 = Color32::from_rgb(10, 11, 14);
pub const BG_CHROME: Color32 = Color32::from_rgb(20, 22, 27);
pub const BG_CARD: Color32 = Color32::from_rgb(23, 26, 32);
pub const BG_HEADER: Color32 = Color32::from_rgb(20, 23, 28);
pub const BG_DEEP: Color32 = Color32::from_rgb(18, 21, 26);
pub const BG_TERMINAL: Color32 = Color32::from_rgb(11, 13, 16);
pub const BG_ELEVATED: Color32 = Color32::from_rgb(27, 31, 38);
pub const BG_HOVER: Color32 = Color32::from_rgb(29, 33, 41);
pub const BG_SELECTED: Color32 = Color32::from_rgb(34, 42, 53);

// Frosted-glass approximation for the top chrome band and Explorer card.
// The RGB values preserve the opaque tonal colors. Alpha 202–208 leaves the
// dark backdrop visible while keeping primary/secondary text contrast intact;
// these fills are used with or without a native backdrop.
pub const BG_CHROME_GLASS: Color32 = Color32::from_rgba_unmultiplied_const(20, 22, 27, 205);
pub const BG_CARD_GLASS: Color32 = Color32::from_rgba_unmultiplied_const(23, 26, 32, 202);
pub const BG_HEADER_GLASS: Color32 = Color32::from_rgba_unmultiplied_const(20, 23, 28, 208);
pub const GLASS_HIGHLIGHT: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 12);
pub const GLASS_SHADOW: Color32 = Color32::from_rgba_unmultiplied_const(0, 0, 0, 58);

pub const BORDER: Color32 = Color32::from_rgb(38, 43, 52);
pub const BORDER_HEADER: Color32 = Color32::from_rgb(32, 37, 45);
pub const BORDER_CONTROL: Color32 = Color32::from_rgb(94, 105, 123);
pub const BORDER_CONTROL_HOVER: Color32 = Color32::from_rgb(122, 132, 148);

pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(230, 232, 236);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(188, 194, 204);
pub const TEXT_TERTIARY: Color32 = Color32::from_rgb(154, 162, 176);
pub const TEXT_DISABLED: Color32 = Color32::from_rgb(122, 132, 148);

pub const ACCENT: Color32 = Color32::from_rgb(233, 167, 106);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(242, 184, 127);
pub const ACCENT_PRESSED: Color32 = Color32::from_rgb(216, 147, 85);
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(20, 16, 11);
pub const ACCENT_TINT: Color32 = Color32::from_rgb(48, 43, 41);

pub const SUCCESS: Color32 = Color32::from_rgb(124, 199, 154);
pub const SUCCESS_TINT: Color32 = Color32::from_rgb(35, 47, 47);
pub const WARNING: Color32 = Color32::from_rgb(233, 184, 114);
pub const WARNING_TINT: Color32 = Color32::from_rgb(48, 45, 42);
pub const ERROR: Color32 = Color32::from_rgb(229, 138, 138);
pub const ERROR_TINT: Color32 = Color32::from_rgb(48, 39, 45);
pub const INFO: Color32 = Color32::from_rgb(143, 184, 232);
pub const INFO_TINT: Color32 = Color32::from_rgb(37, 45, 56);
pub const FOCUS: Color32 = Color32::from_rgb(244, 196, 142);

pub const SCROLL_TRACK: Color32 = Color32::from_rgb(20, 23, 29);
pub const SCROLL_THUMB: Color32 = Color32::from_rgb(98, 108, 125);
pub const SCROLL_THUMB_HOVER: Color32 = Color32::from_rgb(122, 132, 148);

// Terminal palette (also exported to the grid renderer).
pub const TERMINAL_BACKGROUND: [u8; 3] = [11, 13, 16];
pub const TERMINAL_FOREGROUND: [u8; 3] = [215, 221, 231];
pub const TERMINAL_CURSOR: [u8; 3] = [242, 184, 127];
pub const TERMINAL_SELECTION_BACKGROUND: [u8; 3] = [46, 58, 78];
pub const TERMINAL_SELECTION_FOREGROUND: [u8; 3] = [255, 255, 255];
pub const TERMINAL_BRIGHT_BLACK: [u8; 3] = [112, 123, 142];

// ── Floating shadows ────────────────────────────────────────────────────────

pub const SHADOW_FLOAT: Shadow = Shadow {
    offset: [0, 12],
    blur: 32,
    spread: 0,
    color: Color32::from_black_alpha(120),
};
pub const SHADOW_POPUP: Shadow = Shadow {
    offset: [0, 6],
    blur: 18,
    spread: 0,
    color: Color32::from_black_alpha(100),
};
pub const SHADOW_TOAST: Shadow = Shadow {
    offset: [0, 4],
    blur: 14,
    spread: 0,
    color: Color32::from_black_alpha(90),
};

// ── Typography ──────────────────────────────────────────────────────────────

pub const HERO_SIZE: f32 = 22.0;
pub const BODY_SIZE: f32 = 12.5;
pub const BUTTON_SIZE: f32 = 12.0;
pub const SECTION_SIZE: f32 = 10.5;
pub const SMALL_SIZE: f32 = 10.0;
pub const META_SIZE: f32 = 10.5;
pub const MONO_SIZE: f32 = 13.0;
pub const PREVIEW_LINE_HEIGHT: f32 = 18.0;

/// Monospace wordmark shown in workspace and picker chrome.
pub const WORDMARK: &str = "MADE";
/// Wordmark hover text.
pub const WORDMARK_TOOLTIP: &str = "Minimal Agentic Development Environment";
/// Wordmark font size.
pub const WORDMARK_SIZE: f32 = 14.0;
/// Wordmark tracking in points.
pub const WORDMARK_SPACING: f32 = 1.8;

/// The two-tone `MADE` wordmark: accent initial, primary remainder.
pub fn wordmark_job() -> LayoutJob {
    let mut primary = TextFormat {
        font_id: FontId::monospace(WORDMARK_SIZE),
        color: TEXT_PRIMARY,
        extra_letter_spacing: WORDMARK_SPACING,
        ..Default::default()
    };
    let mut accent = primary.clone();
    accent.color = ACCENT;
    let mut job = LayoutJob::default();
    job.append("M", 0.0, accent);
    job.append("ADE", 0.0, std::mem::take(&mut primary));
    job
}

fn wordmark_cache_id(ctx: &egui::Context) -> egui::Id {
    egui::Id::unique(("made-wordmark-galley", ctx.pixels_per_point().to_bits()))
}

/// Cached wordmark galley. The cache is keyed by pixels-per-point so a DPI
/// change cannot leave a stale layout behind.
pub fn wordmark_galley(ui: &Ui) -> Arc<egui::Galley> {
    let ctx = ui.ctx();
    let id = wordmark_cache_id(ctx);
    if let Some(galley) = ctx.data(|data| data.get_temp::<Arc<egui::Galley>>(id)) {
        return galley;
    }
    let galley = ui.painter().layout_job(wordmark_job());
    ctx.data_mut(|data| data.insert_temp(id, galley.clone()));
    galley
}

/// Width of the cached wordmark without painting it.
pub fn wordmark_width(ui: &Ui) -> f32 {
    wordmark_galley(ui).size().x
}

/// Stable id for the wordmark hover target.
pub fn wordmark_id() -> egui::Id {
    egui::Id::unique("made-wordmark")
}

/// Paint the two-tone wordmark and expose its expansion tooltip.
pub fn wordmark(ui: &mut Ui) -> egui::Response {
    let galley = wordmark_galley(ui);
    let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    ui.painter().galley(rect.min, galley, TEXT_PRIMARY);
    ui.interact(rect, wordmark_id(), egui::Sense::hover())
        .on_hover_text(WORDMARK_TOOLTIP)
}

/// Normal body text.
pub fn body(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .font(FontId::proportional(BODY_SIZE))
        .color(TEXT_SECONDARY)
}

/// Smaller detail text at the minimum permitted size.
pub fn micro(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .font(FontId::proportional(META_SIZE))
        .color(TEXT_TERTIARY)
}

/// Monospaced path/value text.
pub fn mono(text: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(text)
        .font(FontId::monospace(size))
        .color(color)
}

/// A section label (`EXPLORER`, `PREVIEW`, …) with the contract's tracking.
pub fn section_label(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(section_job(text)));
}

/// Layout job for a section label.
pub fn section_job(text: &str) -> LayoutJob {
    let mut job = LayoutJob::simple_singleline(
        text.to_owned(),
        FontId::proportional(SECTION_SIZE),
        TEXT_TERTIARY,
    );
    if let Some(section) = job.sections.first_mut() {
        section.format.extra_letter_spacing = 0.6;
    }
    job
}

/// Build a single-line job that ellipsizes at `max_width`.
pub fn ellipsized_job(
    text: impl Into<String>,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> LayoutJob {
    let mut job = LayoutJob::simple_singleline(text.into(), font, color);
    job.wrap.max_width = max_width.max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job
}

/// Paint ellipsized text, returning the galley so callers can use it for hit
/// areas or accessibility labels.
pub fn paint_ellipsized(
    painter: &egui::Painter,
    pos: egui::Pos2,
    text: impl Into<String>,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let job = ellipsized_job(text, font, color, max_width);
    let galley = painter.layout_job(job);
    painter.galley(pos, galley.clone(), color);
    galley
}

// ── Geometry ────────────────────────────────────────────────────────────────

/// Narrowest usable full-height explorer card.
pub const EXPLORER_MIN_WIDTH: f32 = 200.0;

/// Responsive layout metrics from the design contract.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub compact: bool,
    pub window_width: f32,
    pub window_height: f32,
    pub gutter: f32,
    pub gap: f32,
    pub top_height: f32,
    pub card_radius: u8,
    pub card_header_height: f32,
    pub header_padding_x: f32,
    pub content_padding: f32,
    pub tab_strip_height: f32,
    pub tab_height: f32,
    pub button_height: f32,
    pub primary_height: f32,
    pub status_pill_height: f32,
    pub right_card_width: f32,
    pub hide_preview_meta: bool,
    pub max_tabs: usize,
    pub title_chars: usize,
    pub compact_title_chars: usize,
    pub path_line_chars: usize,
    pub root_name_chars: usize,
}

impl Layout {
    /// Derive all metrics from the native window size.
    pub fn for_window(width: f32, height: f32) -> Self {
        let compact = width < 900.0;
        let gutter = if compact { 8.0 } else { 10.0 };
        let content_width = (width - 2.0 * gutter).max(1.0);
        let right_card_width = if width >= 1100.0 {
            440.0
        } else {
            (content_width * 0.35).clamp(288.0, 440.0)
        };
        let narrow_right = right_card_width < 360.0;
        Self {
            compact,
            window_width: width,
            window_height: height,
            gutter,
            gap: if compact { 6.0 } else { 8.0 },
            top_height: if compact { 36.0 } else { 40.0 },
            card_radius: if compact { 8 } else { 10 },
            card_header_height: if compact { 30.0 } else { 34.0 },
            header_padding_x: if compact { 10.0 } else { 12.0 },
            content_padding: if compact { 10.0 } else { 12.0 },
            tab_strip_height: if compact { 34.0 } else { 36.0 },
            tab_height: if compact { 26.0 } else { 28.0 },
            button_height: if compact { 24.0 } else { 26.0 },
            primary_height: if compact { 26.0 } else { 28.0 },
            status_pill_height: if compact { 18.0 } else { 20.0 },
            right_card_width,
            hide_preview_meta: height < 600.0,
            max_tabs: if compact { 3 } else { 5 },
            title_chars: 28,
            compact_title_chars: 18,
            path_line_chars: if compact { 48 } else { 96 },
            root_name_chars: if narrow_right { 14 } else { 28 },
        }
    }

    /// Content width inside the root gutter.
    pub fn content_width(&self) -> f32 {
        (self.window_width - 2.0 * self.gutter).max(1.0)
    }

    /// Toast inset from the bottom-right corner. There is no status bar.
    pub fn toast_bottom_offset(&self) -> f32 {
        -self.gutter
    }

    /// Default full-height explorer card width for this window.
    pub fn explorer_default_width(&self) -> f32 {
        self.right_card_width
    }

    /// Smallest left surface that still fits a terminal grid and preview header.
    pub fn min_left_width(&self) -> f32 {
        if self.compact {
            300.0
        } else {
            360.0
        }
    }

    /// Largest explorer width that leaves a usable left surface.
    pub fn explorer_max_width(&self, available_width: f32) -> f32 {
        (available_width - self.gap - self.min_left_width()).max(EXPLORER_MIN_WIDTH)
    }

    /// Apply compact defaults and viewport bounds to a stored explorer width.
    pub fn explorer_width(&self, stored: f32, available_width: f32) -> f32 {
        let stored = if (stored - 440.0).abs() < 0.5 {
            self.explorer_default_width()
        } else {
            stored
        };
        stored.clamp(EXPLORER_MIN_WIDTH, self.explorer_max_width(available_width))
    }
}

/// Root gutter for a window width.
pub fn gutter(width: f32) -> f32 {
    Layout::for_window(width, 0.0).gutter
}

// ── Frames and painting ─────────────────────────────────────────────────────

/// Opaque card frame with a one-pixel decorative border and no shadow.
pub fn card_frame(layout: &Layout) -> Frame {
    Frame::new()
        .fill(BG_CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(layout.card_radius))
}

/// Terminal card frame. The terminal body is inset so its square grid fill
/// never reaches the rounded outer corners.
pub fn terminal_frame(layout: &Layout) -> Frame {
    Frame::new()
        .fill(BG_TERMINAL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(layout.card_radius))
}

/// Opaque chrome band (workspace strip).
pub fn band_frame(layout: &Layout) -> Frame {
    Frame::new()
        .fill(BG_CHROME)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(if layout.compact { 7 } else { 8 }))
}

/// Floating diagnostics frame.
pub fn floating_frame() -> Frame {
    Frame::new()
        .fill(BG_CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(14))
        .shadow(SHADOW_FLOAT)
}

/// Paint the standard two-stage focus ring.
pub fn paint_focus_ring(ui: &Ui, rect: egui::Rect, radius: impl Into<CornerRadius>, gap: Color32) {
    let radius = radius.into();
    let painter = ui.painter();
    painter.rect_stroke(
        rect.expand(1.0),
        radius,
        Stroke::new(2.0, gap),
        StrokeKind::Outside,
    );
    painter.rect_stroke(
        rect.expand(3.0),
        radius,
        Stroke::new(2.0, FOCUS),
        StrokeKind::Outside,
    );
}

/// Paint a 1.5px focus ring inside a clipped custom row.
pub fn paint_inside_focus_ring(
    painter: &egui::Painter,
    rect: egui::Rect,
    radius: impl Into<CornerRadius>,
) {
    painter.rect_stroke(
        rect.shrink(1.0),
        radius,
        Stroke::new(1.5, FOCUS),
        StrokeKind::Inside,
    );
}

/// Draw an opaque card fill with the standard border.
pub fn paint_card(painter: &egui::Painter, rect: egui::Rect, layout: &Layout, fill: Color32) {
    painter.rect(
        rect,
        CornerRadius::same(layout.card_radius),
        fill,
        Stroke::new(1.0, BORDER),
        StrokeKind::Inside,
    );
}

/// Repaint a card border above the header fill. Card headers are painted after
/// the card fill and would otherwise cover the top one-pixel border.
pub fn paint_card_border(ui: &Ui, rect: egui::Rect, layout: &Layout) {
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(layout.card_radius),
        Stroke::new(1.0, BORDER),
        StrokeKind::Inside,
    );
}

fn paint_glass_highlight(painter: &egui::Painter, rect: egui::Rect, radius: u8) {
    let inset = radius as f32 + 1.0;
    let y = rect.top() + 1.0;
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(rect.left() + inset, y),
            egui::pos2(rect.right() - inset, y + 1.0),
        ),
        0.0,
        GLASS_HIGHLIGHT,
    );
}

/// In-app frosted band for the top chrome strip.
///
/// True backdrop blur needs compositor support; the translucent tonal fill,
/// soft shadow, and inner top highlight are the cheap approximation.
pub fn paint_glass_band(painter: &egui::Painter, rect: egui::Rect, radius: u8) {
    painter.rect_filled(
        rect.translate(egui::vec2(0.0, 2.0)).expand(1.0),
        CornerRadius::same(radius.saturating_add(1)),
        GLASS_SHADOW,
    );
    painter.rect(
        rect,
        CornerRadius::same(radius),
        BG_CHROME_GLASS,
        Stroke::new(1.0, BORDER),
        StrokeKind::Inside,
    );
    paint_glass_highlight(painter, rect, radius);
}

/// In-app frosted fill for the full-height Explorer card.
///
/// Terminal and preview cards stay opaque; only the Explorer surface uses this.
pub fn paint_glass_card(painter: &egui::Painter, rect: egui::Rect, layout: &Layout) {
    painter.rect_filled(
        rect.translate(egui::vec2(0.0, 2.0)).expand(1.0),
        CornerRadius::same(layout.card_radius.saturating_add(1)),
        GLASS_SHADOW,
    );
    painter.rect(
        rect,
        CornerRadius::same(layout.card_radius),
        BG_CARD_GLASS,
        Stroke::new(1.0, BORDER),
        StrokeKind::Inside,
    );
}

/// Paint the Explorer header with the glass tone and top highlight.
pub fn paint_glass_card_header(ui: &Ui, rect: egui::Rect, layout: &Layout) {
    let radius = CornerRadius {
        nw: layout.card_radius,
        ne: layout.card_radius,
        sw: 0,
        se: 0,
    };
    ui.painter().rect_filled(rect, radius, BG_HEADER_GLASS);
    paint_glass_highlight(ui.painter(), rect, layout.card_radius);
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.bottom() - 0.5),
            egui::pos2(rect.right(), rect.bottom() - 0.5),
        ],
        Stroke::new(1.0, BORDER_HEADER),
    );
}

/// Standard ghost button.
pub fn ghost_button(ui: &mut Ui, label: &str, selected: bool, height: f32) -> egui::Response {
    ghost_button_with(ui, label, selected, height, true)
}

/// Ghost button with an explicit enabled state.
pub fn ghost_button_with(
    ui: &mut Ui,
    label: &str,
    selected: bool,
    height: f32,
    enabled: bool,
) -> egui::Response {
    let font = FontId::proportional(11.5);
    let text_width = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), TEXT_TERTIARY)
        .size()
        .x;
    let width = (text_width + 16.0).max(28.0);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(width, height.max(24.0)), egui::Sense::hover());
    let hovered = ui.rect_contains_pointer(rect);
    let fill = if !enabled {
        Color32::TRANSPARENT
    } else if selected {
        BG_SELECTED
    } else if hovered {
        BG_HOVER
    } else {
        Color32::TRANSPARENT
    };
    let stroke = if selected {
        Stroke::new(1.0, BORDER_CONTROL)
    } else {
        Stroke::NONE
    };
    let color = if !enabled {
        TEXT_DISABLED
    } else if selected || hovered {
        TEXT_PRIMARY
    } else {
        TEXT_TERTIARY
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(6),
        fill,
        stroke,
        StrokeKind::Inside,
    );
    let mut builder = UiBuilder::new()
        .max_rect(rect)
        .layout(egui::Layout::centered_and_justified(
            egui::Direction::TopDown,
        ));
    if !enabled {
        builder = builder.disabled();
    }
    let mut child = ui.new_child(builder);
    let response = child.add(
        egui::Button::new(RichText::new(label).font(font).color(color))
            .frame(false)
            .min_size(rect.size()),
    );
    if enabled && response.has_focus() {
        paint_focus_ring(ui, rect, CornerRadius::same(6), BG_CHROME);
    }
    response
}

/// Result of a custom-painted action control with a stable egui id.
#[derive(Debug, Clone)]
pub struct ActionResponse {
    /// Pointer/focus interaction response.
    pub response: egui::Response,
    /// Whether the control was activated by pointer or keyboard this frame.
    pub activated: bool,
}

// ── Icon-only actions ───────────────────────────────────────────────────────

/// Monochrome app-drawn glyphs used by icon-only header actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconGlyph {
    /// Folder with a plus: open another workspace.
    FolderPlus,
    /// Outlined panel with a right column: show/hide the explorer.
    PanelRight,
    /// Heartbeat line: workspace and integration diagnostics.
    Diagnostics,
    /// List rows with markers: optional OpenCode sessions.
    SessionList,
    /// Small X: dismiss the picker or a chrome surface.
    Close,
}

/// Square hit target for an icon-only action at `height`.
pub fn icon_action_extent(height: f32) -> f32 {
    height.max(24.0)
}

/// Icon-only ghost action with a caller-supplied stable id.
///
/// The square target is at least 24 points, uses the same selected/hover/focus
/// treatment as [`ghost_action`], and exposes `label` as both the tooltip and
/// the accessibility name.
pub fn icon_action(
    ui: &mut Ui,
    id: egui::Id,
    glyph: IconGlyph,
    label: &str,
    selected: bool,
    height: f32,
    gap: Color32,
) -> ActionResponse {
    let extent = icon_action_extent(height);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(extent, extent), egui::Sense::hover());
    let mut response = ui.interact(rect, id, egui::Sense::click());
    let hovered = response.hovered();
    let fill = if selected {
        BG_SELECTED
    } else if hovered {
        BG_HOVER
    } else {
        Color32::TRANSPARENT
    };
    let stroke = if selected {
        Stroke::new(1.0, BORDER_CONTROL)
    } else {
        Stroke::NONE
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(6),
        fill,
        stroke,
        StrokeKind::Inside,
    );
    let color = if selected || hovered {
        TEXT_PRIMARY
    } else {
        TEXT_TERTIARY
    };
    paint_icon(ui.painter(), rect, glyph, color);
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.has_focus() {
        paint_focus_ring(ui, rect, CornerRadius::same(6), gap);
    }
    let keyboard = response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        });
    let activated = response.clicked() || keyboard;
    response = response.on_hover_text(label.to_owned());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    ActionResponse {
        response,
        activated,
    }
}

fn paint_icon(painter: &egui::Painter, rect: egui::Rect, glyph: IconGlyph, color: Color32) {
    let center = rect.center();
    let point = |x: f32, y: f32| center + egui::vec2(x, y);
    let stroke = Stroke::new(1.5, color);
    let line = |points: Vec<egui::Pos2>| {
        painter.add(egui::Shape::line(points, stroke));
    };
    match glyph {
        IconGlyph::FolderPlus => {
            let folder = vec![
                point(-7.0, 4.5),
                point(-7.0, -1.5),
                point(-3.5, -1.5),
                point(-2.5, -3.5),
                point(4.5, -3.5),
                point(6.0, -1.5),
                point(7.0, -1.5),
                point(7.0, 4.5),
            ];
            painter.add(egui::epaint::PathShape::closed_line(folder, stroke));
            line(vec![point(-2.5, 1.5), point(2.5, 1.5)]);
            line(vec![point(0.0, -1.0), point(0.0, 4.0)]);
        }
        IconGlyph::PanelRight => {
            let panel = egui::Rect::from_center_size(center, egui::vec2(14.0, 11.0));
            painter.rect_stroke(panel, CornerRadius::same(2), stroke, StrokeKind::Middle);
            line(vec![point(2.0, -5.5), point(2.0, 5.5)]);
        }
        IconGlyph::Diagnostics => {
            line(vec![
                point(-7.0, 0.0),
                point(-4.0, 0.0),
                point(-3.0, -4.0),
                point(-1.5, 4.5),
                point(0.5, -3.0),
                point(2.0, 1.5),
                point(3.5, 0.0),
                point(7.0, 0.0),
            ]);
        }
        IconGlyph::SessionList => {
            for y in [-4.5, 0.0, 4.5] {
                painter.circle_filled(point(-5.0, y), 1.1, color);
                line(vec![point(-2.0, y), point(7.0, y)]);
            }
        }
        IconGlyph::Close => {
            line(vec![point(-3.5, -3.5), point(3.5, 3.5)]);
            line(vec![point(3.5, -3.5), point(-3.5, 3.5)]);
        }
    }
}

/// Ghost action with a caller-supplied stable widget id.
///
/// The id keeps keyboard focus and widget memory scoped to one workspace or one
/// card action instead of leaking between workspace tabs. `gap` is the surface
/// color under the control so the focus ring keeps the contract's two-pixel gap.
pub fn ghost_action(
    ui: &mut Ui,
    id: egui::Id,
    label: &str,
    selected: bool,
    height: f32,
    gap: Color32,
) -> ActionResponse {
    ghost_action_with(ui, id, label, selected, height, gap, true)
}

/// Ghost action with an explicit enabled state.
pub fn ghost_action_with(
    ui: &mut Ui,
    id: egui::Id,
    label: &str,
    selected: bool,
    height: f32,
    gap: Color32,
    enabled: bool,
) -> ActionResponse {
    let font = FontId::proportional(11.5);
    let text_width = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), TEXT_TERTIARY)
        .size()
        .x;
    let width = (text_width + 16.0).max(28.0);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(width, height.max(24.0)), egui::Sense::hover());
    let mut response = ui.interact(rect, id, egui::Sense::click());
    if !enabled {
        response = response.on_disabled_hover_text("Unavailable");
    }
    let hovered = enabled && response.hovered();
    let fill = if !enabled {
        Color32::TRANSPARENT
    } else if selected {
        BG_SELECTED
    } else if hovered {
        BG_HOVER
    } else {
        Color32::TRANSPARENT
    };
    let stroke = if selected {
        Stroke::new(1.0, BORDER_CONTROL)
    } else {
        Stroke::NONE
    };
    let color = if !enabled {
        TEXT_DISABLED
    } else if selected || hovered {
        TEXT_PRIMARY
    } else {
        TEXT_TERTIARY
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(6),
        fill,
        stroke,
        StrokeKind::Inside,
    );
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, color);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    if enabled && hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if enabled && response.has_focus() {
        paint_focus_ring(ui, rect, CornerRadius::same(6), gap);
    }
    let keyboard = enabled
        && response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        });
    let activated = enabled && (response.clicked() || keyboard);
    ActionResponse {
        response,
        activated,
    }
}

/// Secondary button with an explicit control boundary.
pub fn secondary_button(ui: &mut Ui, label: &str, height: f32) -> egui::Response {
    secondary_button_with(ui, label, height, true)
}

/// Secondary button with an explicit enabled state.
pub fn secondary_button_with(
    ui: &mut Ui,
    label: &str,
    height: f32,
    enabled: bool,
) -> egui::Response {
    filled_button(
        ui,
        label,
        height,
        ButtonColors {
            rest: if enabled { BG_ELEVATED } else { BG_CARD },
            hover: if enabled { BG_HOVER } else { BG_CARD },
            pressed: BG_SELECTED,
            text: if enabled {
                TEXT_SECONDARY
            } else {
                TEXT_DISABLED
            },
            stroke: Stroke::new(1.0, if enabled { BORDER_CONTROL } else { BORDER }),
        },
        enabled,
    )
}

/// Primary accent button. The only accent-filled control in the product.
pub fn primary_button(ui: &mut Ui, label: &str, height: f32, enabled: bool) -> egui::Response {
    filled_button(
        ui,
        label,
        height,
        ButtonColors {
            rest: if enabled {
                ACCENT
            } else {
                Color32::from_rgb(59, 48, 38)
            },
            hover: ACCENT_HOVER,
            pressed: ACCENT_PRESSED,
            text: if enabled {
                ACCENT_TEXT
            } else {
                Color32::from_rgb(140, 122, 99)
            },
            stroke: Stroke::NONE,
        },
        enabled,
    )
}

/// Primary action with a caller-supplied stable widget id.
pub fn primary_action(
    ui: &mut Ui,
    id: egui::Id,
    label: &str,
    height: f32,
    enabled: bool,
    gap: Color32,
) -> ActionResponse {
    filled_action(
        ui,
        id,
        label,
        height,
        ButtonColors {
            rest: if enabled {
                ACCENT
            } else {
                Color32::from_rgb(59, 48, 38)
            },
            hover: ACCENT_HOVER,
            pressed: ACCENT_PRESSED,
            text: if enabled {
                ACCENT_TEXT
            } else {
                Color32::from_rgb(140, 122, 99)
            },
            stroke: Stroke::NONE,
        },
        gap,
        enabled,
    )
}

/// Secondary action with a caller-supplied stable widget id.
pub fn secondary_action(
    ui: &mut Ui,
    id: egui::Id,
    label: &str,
    height: f32,
    enabled: bool,
    gap: Color32,
) -> ActionResponse {
    filled_action(
        ui,
        id,
        label,
        height,
        ButtonColors {
            rest: if enabled { BG_ELEVATED } else { BG_CARD },
            hover: if enabled { BG_HOVER } else { BG_CARD },
            pressed: BG_SELECTED,
            text: if enabled {
                TEXT_SECONDARY
            } else {
                TEXT_DISABLED
            },
            stroke: Stroke::new(1.0, if enabled { BORDER_CONTROL } else { BORDER }),
        },
        gap,
        enabled,
    )
}

fn filled_action(
    ui: &mut Ui,
    id: egui::Id,
    label: &str,
    height: f32,
    colors: ButtonColors,
    gap: Color32,
    enabled: bool,
) -> ActionResponse {
    let font = FontId::proportional(BUTTON_SIZE);
    let text_width = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), colors.text)
        .size()
        .x;
    let width = (text_width + 20.0).max(28.0);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(width, height.max(24.0)), egui::Sense::hover());
    let response = ui.interact(rect, id, egui::Sense::click());
    let hovered = enabled && response.hovered();
    let down = hovered && ui.ctx().input(|input| input.pointer.any_down());
    let fill = if !enabled {
        colors.rest
    } else if down {
        colors.pressed
    } else if hovered {
        colors.hover
    } else {
        colors.rest
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(6),
        fill,
        colors.stroke,
        StrokeKind::Inside,
    );
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, colors.text);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, colors.text);
    if enabled && response.has_focus() {
        paint_focus_ring(ui, rect, CornerRadius::same(6), gap);
    }
    let keyboard = enabled
        && response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        });
    let activated = enabled && (response.clicked() || keyboard);
    ActionResponse {
        response,
        activated,
    }
}

struct ButtonColors {
    rest: Color32,
    hover: Color32,
    pressed: Color32,
    text: Color32,
    stroke: Stroke,
}

fn filled_button(
    ui: &mut Ui,
    label: &str,
    height: f32,
    colors: ButtonColors,
    enabled: bool,
) -> egui::Response {
    let font = FontId::proportional(BUTTON_SIZE);
    let text_width = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), colors.text)
        .size()
        .x;
    let width = (text_width + 20.0).max(28.0);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(width, height.max(24.0)), egui::Sense::hover());
    let hovered = enabled && ui.rect_contains_pointer(rect);
    let down = hovered && ui.ctx().input(|input| input.pointer.any_down());
    let fill = if !enabled {
        colors.rest
    } else if down {
        colors.pressed
    } else if hovered {
        colors.hover
    } else {
        colors.rest
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(6),
        fill,
        colors.stroke,
        StrokeKind::Inside,
    );
    let mut builder = UiBuilder::new()
        .max_rect(rect)
        .layout(egui::Layout::centered_and_justified(
            egui::Direction::TopDown,
        ));
    if !enabled {
        builder = builder.disabled();
    }
    let mut child = ui.new_child(builder);
    let response = child.add(
        egui::Button::new(RichText::new(label).font(font).color(colors.text))
            .frame(false)
            .min_size(rect.size()),
    );
    if enabled && response.has_focus() {
        paint_focus_ring(ui, rect, CornerRadius::same(6), BG_CARD);
    }
    response
}

/// A small opaque status/read-only pill.
pub fn pill(ui: &mut Ui, text: &str, color: Color32, tint: Color32, height: f32) -> egui::Response {
    let font = FontId::proportional(10.0);
    let text_width = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), color)
        .size()
        .x;
    let size = egui::vec2((text_width + 14.0).max(18.0), height.max(14.0));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same((height / 2.0) as u8),
        tint,
        Stroke::new(1.0, BORDER),
        StrokeKind::Inside,
    );
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(pos, galley, color);
    response
}

// ── Theme installation ──────────────────────────────────────────────────────

/// Install the complete Calm Editor theme. Call once per context.
pub fn install(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.visuals = visuals();
        style.text_styles = text_styles();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        style.spacing.interact_size = egui::vec2(28.0, 26.0);
        style.spacing.window_margin = Margin::same(14);
        style.spacing.menu_margin = Margin::same(6);
        style.spacing.indent = 14.0;
        style.spacing.tooltip_width = 320.0;
        style.spacing.scroll = scroll_style();
        style.animation_time = 0.12;
    });
}

/// Dark visuals for the contract.
pub fn visuals() -> Visuals {
    let mut visuals = Visuals::dark();
    visuals.dark_mode = true;
    visuals.override_text_color = None;
    visuals.weak_text_color = Some(TEXT_TERTIARY);
    visuals.weak_text_alpha = 1.0;
    visuals.panel_fill = Color32::TRANSPARENT;
    visuals.window_fill = BG_CARD;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.window_shadow = SHADOW_FLOAT;
    visuals.menu_corner_radius = CornerRadius::same(6);
    visuals.popup_shadow = SHADOW_POPUP;
    visuals.extreme_bg_color = BG_DEEP;
    visuals.faint_bg_color = BG_ELEVATED;
    visuals.code_bg_color = BG_DEEP;
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = ERROR;
    visuals.hyperlink_color = ACCENT;
    visuals.selection.bg_fill = BG_SELECTED;
    visuals.selection.stroke = Stroke::new(1.0, TEXT_PRIMARY);
    visuals.window_highlight_topmost = false;
    visuals.striped = false;
    visuals.indent_has_left_vline = false;
    visuals.widgets = Widgets {
        noninteractive: WidgetVisuals {
            bg_fill: BG_CARD,
            weak_bg_fill: BG_CARD,
            bg_stroke: Stroke::new(1.0, BORDER),
            corner_radius: CornerRadius::same(6),
            fg_stroke: Stroke::new(1.0, TEXT_SECONDARY),
            expansion: 0.0,
        },
        inactive: WidgetVisuals {
            bg_fill: SCROLL_THUMB,
            weak_bg_fill: BG_ELEVATED,
            bg_stroke: Stroke::new(1.0, BORDER_CONTROL),
            corner_radius: CornerRadius::same(6),
            fg_stroke: Stroke::new(1.0, TEXT_SECONDARY),
            expansion: 0.0,
        },
        hovered: WidgetVisuals {
            bg_fill: SCROLL_THUMB_HOVER,
            weak_bg_fill: BG_HOVER,
            bg_stroke: Stroke::new(1.0, BORDER_CONTROL_HOVER),
            corner_radius: CornerRadius::same(6),
            fg_stroke: Stroke::new(1.0, TEXT_PRIMARY),
            expansion: 0.0,
        },
        active: WidgetVisuals {
            bg_fill: BG_SELECTED,
            weak_bg_fill: BG_SELECTED,
            bg_stroke: Stroke::new(2.0, FOCUS),
            corner_radius: CornerRadius::same(6),
            fg_stroke: Stroke::new(1.0, TEXT_PRIMARY),
            expansion: 0.0,
        },
        open: WidgetVisuals {
            bg_fill: BG_SELECTED,
            weak_bg_fill: BG_SELECTED,
            bg_stroke: Stroke::new(1.0, BORDER_CONTROL),
            corner_radius: CornerRadius::same(6),
            fg_stroke: Stroke::new(1.0, TEXT_PRIMARY),
            expansion: 0.0,
        },
    };
    visuals
}

/// Standard text styles. No style is below 10px.
pub fn text_styles() -> BTreeMap<TextStyle, FontId> {
    BTreeMap::from([
        (TextStyle::Small, FontId::proportional(SMALL_SIZE)),
        (TextStyle::Body, FontId::proportional(BODY_SIZE)),
        (TextStyle::Button, FontId::proportional(BUTTON_SIZE)),
        (TextStyle::Heading, FontId::proportional(20.0)),
        (TextStyle::Monospace, FontId::monospace(MONO_SIZE)),
    ])
}

/// Solid eight-pixel scrollbars with a 24px minimum handle.
pub fn scroll_style() -> egui::style::ScrollStyle {
    let mut scroll = egui::style::ScrollStyle::solid();
    scroll.bar_width = 8.0;
    scroll.handle_min_length = 24.0;
    scroll.bar_inner_margin = 2.0;
    scroll.bar_outer_margin = 0.0;
    scroll
}

/// Native clear color. The alpha is one when transparency is disabled.
///
/// This is the default in-app-glass clear color; callers that have a native
/// backdrop status should use [`clear_color_with_native`].
pub fn clear_color() -> [f32; 4] {
    clear_color_with_native(false)
}

/// Clear color for a window, accounting for a running native backdrop.
///
/// With native vibrancy/Mica active the lower alpha (150 instead of 190) lets
/// the OS material show through. Without it, the default alpha keeps the
/// in-app glass readable even when the compositor ignores transparency.
pub fn clear_color_with_native(native_backdrop: bool) -> [f32; 4] {
    if !TRANSLUCENT_WINDOW {
        return BG_CLEAR_SOLID.to_normalized_gamma_f32();
    }
    if native_backdrop {
        BG_CLEAR_NATIVE.to_normalized_gamma_f32()
    } else {
        BG_CLEAR.to_normalized_gamma_f32()
    }
}

/// Terminal palette from the contract.
pub fn terminal_palette() -> crate::render::GridPalette {
    crate::render::GridPalette {
        background: TERMINAL_BACKGROUND,
        foreground: TERMINAL_FOREGROUND,
        cursor: TERMINAL_CURSOR,
        selection_background: TERMINAL_SELECTION_BACKGROUND,
        selection_foreground: TERMINAL_SELECTION_FOREGROUND,
        ansi: {
            let mut ansi = crate::render::GridPalette::default().ansi;
            ansi[8] = TERMINAL_BRIGHT_BLACK;
            ansi
        },
    }
}

/// Truncate a string to `max` characters, appending an ellipsis.
pub fn truncate_end(text: &str, max: usize) -> String {
    crate::session_map::truncate_end(text, max)
}

/// Truncate a file name in the middle with an ellipsis.
pub fn truncate_middle(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max {
        return text.to_owned();
    }
    if max == 1 {
        return "…".to_owned();
    }
    let head = (max - 1) / 2;
    let tail = max - 1 - head;
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(count - tail).collect();
    format!("{start}…{end}")
}

/// Human-readable size for preview metadata.
pub fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let value = bytes as f64;
    if value < KIB {
        format!("{bytes} B")
    } else if value < KIB * 1024.0 {
        format!("{:.1} KB", value / KIB)
    } else {
        format!("{:.1} MB", value / (KIB * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_match_the_contract() {
        assert_eq!(BG_CARD, Color32::from_rgb(0x17, 0x1A, 0x20));
        assert_eq!(ACCENT, Color32::from_rgb(0xE9, 0xA7, 0x6A));
        assert_eq!(TERMINAL_BACKGROUND, [0x0B, 0x0D, 0x10]);
        assert_eq!(TERMINAL_BRIGHT_BLACK, [0x70, 0x7B, 0x8E]);
    }

    #[test]
    fn glass_tokens_are_translucent_and_tonal() {
        for (glass, opaque) in [
            (BG_CHROME_GLASS, BG_CHROME),
            (BG_CARD_GLASS, BG_CARD),
            (BG_HEADER_GLASS, BG_HEADER),
        ] {
            let [r, g, b, a] = glass.to_srgba_unmultiplied();
            let [opaque_r, opaque_g, opaque_b, _] = opaque.to_srgba_unmultiplied();
            assert!(
                r.abs_diff(opaque_r) <= 1 && g.abs_diff(opaque_g) <= 1 && b.abs_diff(opaque_b) <= 1,
                "glass tint must match the opaque tone within premultiplied rounding: {glass:?}"
            );
            assert!(
                (195..=215).contains(&a),
                "glass alpha must stay in the frosted range: {glass:?}"
            );
        }
        assert!(GLASS_HIGHLIGHT.a() < 20);
        assert!(GLASS_SHADOW.a() < 80);
    }

    #[test]
    fn wordmark_job_has_accent_initial_and_primary_rest() {
        let job = wordmark_job();
        assert_eq!(job.text, WORDMARK);
        assert_eq!(job.sections.len(), 2);
        let first = job.sections[0].byte_range.clone();
        let second = job.sections[1].byte_range.clone();
        assert_eq!(&job.text[first.start.0..first.end.0], "M");
        assert_eq!(&job.text[second.start.0..second.end.0], "ADE");
        assert_eq!(job.sections[0].format.color, ACCENT);
        assert_eq!(job.sections[1].format.color, TEXT_PRIMARY);
        assert_eq!(
            job.sections[0].format.font_id,
            FontId::monospace(WORDMARK_SIZE)
        );
        assert!(
            (job.sections[0].format.extra_letter_spacing - WORDMARK_SPACING).abs() < f32::EPSILON
        );
    }

    #[test]
    fn wordmark_galley_is_cached_per_context() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut first = None;
        let mut second = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            first = Some(wordmark_galley(ui));
            second = Some(wordmark_galley(ui));
        });
        output.textures_delta.clear();
        assert!(std::sync::Arc::ptr_eq(
            first.as_ref().unwrap(),
            second.as_ref().unwrap()
        ));
    }

    #[test]
    fn text_styles_keep_the_minimum_sizes() {
        let styles = text_styles();
        assert_eq!(styles[&TextStyle::Small].size, 10.0);
        assert_eq!(styles[&TextStyle::Body].size, 12.5);
        assert_eq!(styles[&TextStyle::Monospace].size, 13.0);
        assert!(styles.values().all(|font| font.size >= 10.0));
    }

    #[test]
    fn layout_switches_to_compact_below_nine_hundred() {
        let wide = Layout::for_window(1280.0, 800.0);
        assert!(!wide.compact);
        assert_eq!(wide.gutter, 10.0);
        assert_eq!(wide.right_card_width, 440.0);
        assert_eq!(wide.explorer_default_width(), 440.0);
        assert!(wide.explorer_max_width(wide.content_width()) > 440.0);
        assert_eq!(wide.max_tabs, 5);

        let compact = Layout::for_window(720.0, 480.0);
        assert!(compact.compact);
        assert_eq!(compact.gutter, 8.0);
        assert_eq!(compact.top_height, 36.0);
        assert_eq!(compact.right_card_width, 288.0);
        assert_eq!(
            compact.explorer_width(440.0, compact.content_width()),
            288.0
        );
        assert!(compact.explorer_max_width(compact.content_width()) < 440.0);
        assert!(compact.hide_preview_meta);
    }

    #[test]
    fn explorer_width_keeps_a_usable_left_surface() {
        let compact = Layout::for_window(720.0, 480.0);
        let max = compact.explorer_max_width(compact.content_width());
        assert!(max >= EXPLORER_MIN_WIDTH);
        assert!(
            compact.content_width() - compact.gap - max >= compact.min_left_width(),
            "left surface lost its usable width"
        );
        assert_eq!(
            compact.explorer_width(1_000.0, compact.content_width()),
            max
        );
        assert_eq!(
            compact.explorer_width(10.0, compact.content_width()),
            EXPLORER_MIN_WIDTH
        );

        let wide = Layout::for_window(1280.0, 800.0);
        let max = wide.explorer_max_width(wide.content_width());
        assert!(
            wide.content_width() - wide.gap - max >= wide.min_left_width(),
            "left surface lost its usable width"
        );
    }

    #[test]
    fn clear_color_respects_the_kill_switch_and_native_backdrop() {
        let in_app = clear_color();
        assert_eq!(in_app, clear_color_with_native(false));

        let expected_in_app_alpha = if TRANSLUCENT_WINDOW {
            190.0 / 255.0
        } else {
            1.0
        };
        assert!((in_app[3] - expected_in_app_alpha).abs() < 0.001);

        let native = clear_color_with_native(true);
        let expected_native_alpha = if TRANSLUCENT_WINDOW {
            150.0 / 255.0
        } else {
            1.0
        };
        assert!((native[3] - expected_native_alpha).abs() < 0.001);
        assert!(
            native[3] <= in_app[3],
            "a native backdrop must not make the clear color more opaque"
        );
    }

    #[test]
    fn card_frames_are_opaque_bordered_and_shadow_free() {
        let layout = Layout::for_window(1280.0, 800.0);
        let card = card_frame(&layout);
        assert_eq!(card.fill, BG_CARD);
        assert_eq!(card.stroke.color, BORDER);
        assert_eq!(card.shadow, Shadow::NONE);
        assert_eq!(card.corner_radius, CornerRadius::same(10));

        let floating = floating_frame();
        assert_eq!(floating.fill, BG_CARD);
        assert_eq!(floating.shadow, SHADOW_FLOAT);
    }

    #[test]
    fn card_and_primary_button_render_headless_shapes() {
        let ctx = egui::Context::default();
        install(&ctx);
        let layout = Layout::for_window(1280.0, 800.0);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 620.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            card_frame(&layout).show(ui, |ui| {
                ui.label("card body");
                primary_button(ui, "Use this folder", 28.0, true);
            });
        });
        let accent = output.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) => rect.fill == ACCENT,
            _ => false,
        });
        output.textures_delta.clear();
        assert!(!output.shapes.is_empty());
        assert!(accent, "primary button must be accent-filled");
    }

    #[test]
    fn middle_truncation_keeps_both_ends() {
        assert_eq!(truncate_middle("README.md", 20), "README.md");
        let truncated = truncate_middle("a-long-preview-filename.rs", 12);
        assert!(truncated.contains('…'));
        assert!(truncated.starts_with("a-lon"));
        assert!(truncated.ends_with(".rs"));
    }

    #[test]
    fn terminal_palette_uses_the_contract_background_and_bright_black() {
        let palette = terminal_palette();
        assert_eq!(palette.background, [11, 13, 16]);
        assert_eq!(palette.foreground, [215, 221, 231]);
        assert_eq!(palette.cursor, [242, 184, 127]);
        assert_eq!(palette.ansi[8], [112, 123, 142]);
        assert_eq!(palette.selection_background, [46, 58, 78]);
    }
}
