//! “Calm Editor” design tokens and egui theme installation.
//!
//! The contract is `specs/changes/archive/20261029-made-ui-ux-v1/design.md`. The terminal card is glass on its
//! primary screen and opaque for alternate-screen TUIs; preview and picker surfaces stay opaque.
//! The Explorer card uses semi-transparent dark fills as an in-app frosted-glass approximation.
//! A native backdrop (macOS vibrancy or Windows Mica) can be layered behind the window when the
//! platform supports it; the in-app explorer glass remains legible on its own and stays the
//! default on every platform without that effect. The workspace header floats directly over the
//! clear/native backdrop with no band fill, border, shadow, or highlight. Keep the palette and
//! geometry here rather than scattering literals through the views.

use eframe::egui::style::{WidgetVisuals, Widgets};
use eframe::egui::text::LayoutJob;
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Frame, Margin, RichText, Shadow, Stroke, StrokeKind,
    TextFormat, TextStyle, Ui, UiBuilder, Visuals,
};
use egui_extras::loaders::svg_loader::SvgLoader;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Global kill switch for the translucent native window.
///
/// When `false`, no transparency is requested and [`clear_color`] is opaque.
/// The rest of the UI is identical; it simply renders on the solid backdrop.
pub const TRANSLUCENT_WINDOW: bool = true;

// ── Colors ──────────────────────────────────────────────────────────────────

// The fallback clear color is deliberately dark: alpha 215 keeps the window
// slightly translucent without making text chase a bright desktop when no
// native material is applied. When a native compositor backdrop is active,
// alpha 135 lets the OS material read through; the glass fills below carry the
// text contrast.
pub const BG_CLEAR: Color32 = Color32::from_rgba_unmultiplied_const(10, 11, 14, 215);
pub const BG_CLEAR_NATIVE: Color32 = Color32::from_rgba_unmultiplied_const(10, 11, 14, 135);
pub const BG_CLEAR_SOLID: Color32 = Color32::from_rgb(10, 11, 14);
pub const BG_CHROME: Color32 = Color32::from_rgb(20, 22, 27);
pub const BG_CARD: Color32 = Color32::from_rgb(23, 26, 32);
pub const BG_HEADER: Color32 = Color32::from_rgb(20, 23, 28);
pub const BG_DEEP: Color32 = Color32::from_rgb(18, 21, 26);
pub const BG_TERMINAL: Color32 = Color32::from_rgb(11, 13, 16);
/// Primary-screen terminal card fill: the `BG_TERMINAL` RGB at glass alpha.
/// This is the sole glass layer; body/chrome overlays are transparent.
pub const BG_TERMINAL_GLASS: Color32 = Color32::from_rgba_unmultiplied_const(11, 13, 16, 160);
pub const BG_ELEVATED: Color32 = Color32::from_rgb(27, 31, 38);
pub const BG_HOVER: Color32 = Color32::from_rgb(29, 33, 41);
pub const BG_SELECTED: Color32 = Color32::from_rgb(34, 42, 53);
/// Inactive workspace-tab fill: translucent black over the window backdrop.
pub const BG_TAB_IDLE: Color32 = Color32::from_rgba_unmultiplied_const(0, 0, 0, 84);
/// Workspace-tab hover fill: opaque dark green-gray. Inactive hover paints no stroke.
pub const BG_TAB_HOVER: Color32 = Color32::from_rgb(0x15, 0x1C, 0x18);
/// Selected workspace-tab fill: opaque dark gray with a green undertone.
pub const BG_TAB_SELECTED: Color32 = Color32::from_rgb(0x1B, 0x23, 0x1F);

// Frosted-glass approximation for the Explorer card. The RGB values preserve
// the opaque tonal colors. Alpha 188–194 leaves the dark backdrop visible while
// keeping primary/secondary text contrast intact; these fills are used with or
// without a native backdrop. The header has no persistent band fill.
pub const BG_CARD_GLASS: Color32 = Color32::from_rgba_unmultiplied_const(23, 26, 32, 188);
pub const BG_HEADER_GLASS: Color32 = Color32::from_rgba_unmultiplied_const(20, 23, 28, 194);
// Native-backdrop Explorer tints: very discreet so the macOS HudWindow or
// Windows Mica material reads through. The fallback glass above remains
// unchanged and is used whenever the native effect is not actually applied.
pub const BG_CARD_GLASS_NATIVE: Color32 = Color32::from_rgba_unmultiplied_const(23, 26, 32, 24);
pub const BG_HEADER_GLASS_NATIVE: Color32 = Color32::from_rgba_unmultiplied_const(20, 23, 28, 32);
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

/// Inactive header-tab label opacity.
pub const TAB_INACTIVE_OPACITY: f32 = 0.72;
/// Inactive header-icon opacity.
pub const ICON_INACTIVE_OPACITY: f32 = 0.80;

/// Secondary label tone for an inactive header tab.
pub fn inactive_tab_text() -> Color32 {
    TEXT_SECONDARY.gamma_multiply(TAB_INACTIVE_OPACITY)
}

/// Glyph tone for an inactive header action.
pub fn inactive_icon_color() -> Color32 {
    TEXT_TERTIARY.gamma_multiply(ICON_INACTIVE_OPACITY)
}

pub const ACCENT: Color32 = Color32::from_rgb(0x8B, 0xD5, 0xA7);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0xA3, 0xE3, 0xBA);
pub const ACCENT_PRESSED: Color32 = Color32::from_rgb(0x72, 0xBF, 0x8E);
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(0x08, 0x14, 0x0D);
pub const ACCENT_TINT: Color32 = Color32::from_rgb(0x26, 0x35, 0x2C);

pub const SUCCESS: Color32 = Color32::from_rgb(0x4F, 0xCF, 0xA8);
pub const SUCCESS_TINT: Color32 = Color32::from_rgb(35, 47, 47);
pub const WARNING: Color32 = Color32::from_rgb(233, 184, 114);
pub const WARNING_TINT: Color32 = Color32::from_rgb(48, 45, 42);
pub const ERROR: Color32 = Color32::from_rgb(229, 138, 138);
pub const ERROR_TINT: Color32 = Color32::from_rgb(48, 39, 45);
pub const INFO: Color32 = Color32::from_rgb(143, 184, 232);
pub const INFO_TINT: Color32 = Color32::from_rgb(37, 45, 56);
pub const FOCUS: Color32 = Color32::from_rgb(0xB8, 0xF0, 0xC9);

pub const SCROLL_TRACK: Color32 = Color32::from_rgb(20, 23, 29);
pub const SCROLL_THUMB: Color32 = Color32::from_rgb(98, 108, 125);
pub const SCROLL_THUMB_HOVER: Color32 = Color32::from_rgb(122, 132, 148);

// Terminal palette (also exported to the grid renderer).
pub const TERMINAL_BACKGROUND: [u8; 3] = [11, 13, 16];
pub const TERMINAL_FOREGROUND: [u8; 3] = [215, 221, 231];
pub const TERMINAL_CURSOR: [u8; 3] = TERMINAL_FOREGROUND;
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
/// Inactive workspace/session tab label size.
pub const TAB_FONT_SIZE: f32 = 12.5;
/// Selected tab label size; the quiet non-color selection cue.
pub const TAB_SELECTED_FONT_SIZE: f32 = 13.0;

/// Monospace wordmark shown in workspace and picker chrome.
pub const WORDMARK: &str = "MADE";
/// Wordmark hover text.
pub const WORDMARK_TOOLTIP: &str = "Minimal Agentic Development Environment";
/// Wordmark font size.
pub const WORDMARK_SIZE: f32 = 14.0;
/// Wordmark tracking in points.
pub const WORDMARK_SPACING: f32 = 1.8;
/// Gap between the tab strip and the right-side wordmark, and between the
/// wordmark and the first right icon.
pub const WORDMARK_GAP_WIDE: f32 = 24.0;
/// Compact wordmark breathing gap on both sides.
pub const WORDMARK_GAP_COMPACT: f32 = 18.0;

/// Wordmark breathing gap for the current layout density.
pub fn wordmark_gap(compact: bool) -> f32 {
    if compact {
        WORDMARK_GAP_COMPACT
    } else {
        WORDMARK_GAP_WIDE
    }
}

/// Fixed font size of the workspace-tab close glyph.
pub const CLOSE_GLYPH_SIZE: f32 = 14.0;

/// How far the workspace-tab stroke sits below `BG_TAB_SELECTED`.
///
/// Twelve is the same strength as the previous lighter step, in the darker
/// direction. The selected fill is (27, 35, 31), so this does not clamp to black.
pub const WORKSPACE_TAB_STROKE_DARKEN: u8 = 12;

/// Alpha of the selected workspace-tab stroke.
///
/// 128 keeps the previous strength: a partial mark, not an opaque outline.
pub const WORKSPACE_TAB_STROKE_ALPHA: u8 = 128;

/// Faint 1px inside stroke painted only on the selected workspace tab.
pub fn workspace_tab_stroke() -> Stroke {
    let [r, g, b, _] = BG_TAB_SELECTED.to_srgba_unmultiplied();
    Stroke::new(
        1.0,
        Color32::from_rgba_unmultiplied(
            r.saturating_sub(WORKSPACE_TAB_STROKE_DARKEN),
            g.saturating_sub(WORKSPACE_TAB_STROKE_DARKEN),
            b.saturating_sub(WORKSPACE_TAB_STROKE_DARKEN),
            WORKSPACE_TAB_STROKE_ALPHA,
        ),
    )
}

/// Explorer splitter fade duration in seconds.
pub const SPLITTER_FADE_TIME: f32 = 0.14;
/// Peak hover opacity of the splitter line/grip.
pub const SPLITTER_HOVER_OPACITY: f32 = 0.55;
/// Peak drag opacity of the accent splitter line/grip.
pub const SPLITTER_DRAG_OPACITY: f32 = 0.70;

/// Splitter opacity after `progress` of the fade.
pub fn splitter_opacity(progress: f32, dragged: bool) -> f32 {
    let peak = if dragged {
        SPLITTER_DRAG_OPACITY
    } else {
        SPLITTER_HOVER_OPACITY
    };
    progress.clamp(0.0, 1.0) * peak
}

/// Splitter tone: muted control boundary while hovering, green accent while dragging.
pub fn splitter_tone(dragged: bool) -> Color32 {
    if dragged {
        ACCENT
    } else {
        BORDER_CONTROL
    }
}

/// Splitter line/stroke color with the fade opacity baked in.
pub fn splitter_line_color(progress: f32, dragged: bool) -> Color32 {
    splitter_tone(dragged).gamma_multiply(splitter_opacity(progress, dragged))
}

/// Splitter grip fill with the fade opacity baked in.
pub fn splitter_grip_fill(progress: f32, dragged: bool) -> Color32 {
    BG_ELEVATED.gamma_multiply(splitter_opacity(progress, dragged))
}

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

/// Paint ellipsized text vertically centered on `center_y`.
///
/// Returns the laid-out galley so callers can align app-drawn markers to the
/// same optical center.
pub fn paint_ellipsized_centered(
    painter: &egui::Painter,
    left: f32,
    center_y: f32,
    text: impl Into<String>,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let job = ellipsized_job(text, font, color, max_width);
    let galley = painter.layout_job(job);
    let top = center_y - galley.size().y / 2.0;
    painter.galley(egui::pos2(left, top), galley.clone(), color);
    galley
}

/// Paint a small app-drawn directory disclosure triangle.
pub fn paint_disclosure(
    painter: &egui::Painter,
    center: egui::Pos2,
    expanded: bool,
    color: Color32,
) {
    let points = if expanded {
        vec![
            center + egui::vec2(-4.0, -3.0),
            center + egui::vec2(4.0, -3.0),
            center + egui::vec2(0.0, 3.0),
        ]
    } else {
        vec![
            center + egui::vec2(-3.5, -4.0),
            center + egui::vec2(3.5, 0.0),
            center + egui::vec2(-3.5, 4.0),
        ]
    };
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

// ── Geometry ────────────────────────────────────────────────────────────────

/// Narrowest usable full-height explorer card.
pub const EXPLORER_MIN_WIDTH: f32 = 200.0;

/// Wide-layout default explorer card width.
pub const EXPLORER_DEFAULT_WIDTH_WIDE: f32 = 360.0;

/// Compact-layout default explorer card width.
pub const EXPLORER_DEFAULT_WIDTH_COMPACT: f32 = 288.0;

/// Pre-2026-10-27 wide default retained for stored-width migration.
pub const EXPLORER_LEGACY_DEFAULT_WIDTH: f32 = 440.0;

/// Interim Phase 12 wide default retained for stored-width migration.
pub const EXPLORER_INTERIM_DEFAULT_WIDTH: f32 = 400.0;

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
    /// Session-map tabs keep their previous height when the workspace header
    /// shrinks.
    pub session_tab_height: f32,
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
            EXPLORER_DEFAULT_WIDTH_WIDE
        } else if width >= 900.0 {
            (content_width * 0.35)
                .clamp(EXPLORER_DEFAULT_WIDTH_COMPACT, EXPLORER_DEFAULT_WIDTH_WIDE)
        } else {
            EXPLORER_DEFAULT_WIDTH_COMPACT
        };
        let narrow_right = right_card_width < 360.0;
        Self {
            compact,
            window_width: width,
            window_height: height,
            gutter,
            gap: if compact { 6.0 } else { 8.0 },
            top_height: if compact { 24.0 } else { 26.0 },
            card_radius: if compact { 8 } else { 10 },
            card_header_height: if compact { 30.0 } else { 34.0 },
            header_padding_x: if compact { 10.0 } else { 12.0 },
            content_padding: if compact { 10.0 } else { 12.0 },
            tab_strip_height: if compact { 34.0 } else { 36.0 },
            tab_height: if compact { 24.0 } else { 26.0 },
            session_tab_height: if compact { 26.0 } else { 28.0 },
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

    /// Top edge of the floating workspace header: one pane gap below the window.
    pub fn header_top(&self) -> f32 {
        self.gap
    }

    /// Top edge of the main content cards: one pane gap below the header strip.
    pub fn content_top(&self) -> f32 {
        self.header_top() + self.top_height + self.gap
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
    ///
    /// `440` is the pre-2026-10-27 wide default, `400` is the interim Phase 12
    /// default, and `360` is the current constructor default. All three mean
    /// the splitter has not been dragged, so they resolve to the responsive
    /// default (360 wide, 35% at mid widths, 288 compact). Other stored widths
    /// are deliberate and only clamp.
    pub fn explorer_width(&self, stored: f32, available_width: f32) -> f32 {
        let stored = if (stored - EXPLORER_LEGACY_DEFAULT_WIDTH).abs() < 0.5
            || (stored - EXPLORER_INTERIM_DEFAULT_WIDTH).abs() < 0.5
            || (stored - EXPLORER_DEFAULT_WIDTH_WIDE).abs() < 0.5
        {
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

/// Terminal card fill: glass on the primary screen, opaque for TUIs that use
/// the VT alternate screen. This is the sole primary-screen glass layer.
pub fn terminal_surface(alternate_screen: bool) -> Color32 {
    if alternate_screen {
        BG_TERMINAL
    } else {
        BG_TERMINAL_GLASS
    }
}

/// Overlay fill for terminal-owned body/chrome that sits on the terminal card.
/// Primary screen: no fill, so the card's single glass layer shows through.
/// Alternate screen: opaque `BG_TERMINAL` so TUI cells stay on an opaque surface.
pub fn terminal_overlay_surface(alternate_screen: bool) -> Option<Color32> {
    alternate_screen.then_some(BG_TERMINAL)
}

/// Terminal card frame. The terminal body is inset so its square grid fill
/// never reaches the rounded outer corners.
pub fn terminal_frame(layout: &Layout, alternate_screen: bool) -> Frame {
    Frame::new()
        .fill(terminal_surface(alternate_screen))
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(layout.card_radius))
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

/// Explorer card fill: native tint when the backdrop is actually applied,
/// otherwise the darker fallback glass.
pub fn explorer_card_glass(native_backdrop_applied: bool) -> Color32 {
    if native_backdrop_applied {
        BG_CARD_GLASS_NATIVE
    } else {
        BG_CARD_GLASS
    }
}

/// Explorer header fill: native tint when the backdrop is actually applied,
/// otherwise the darker fallback glass.
pub fn explorer_header_glass(native_backdrop_applied: bool) -> Color32 {
    if native_backdrop_applied {
        BG_HEADER_GLASS_NATIVE
    } else {
        BG_HEADER_GLASS
    }
}

/// In-app frosted fill for the full-height Explorer card.
///
/// Preview and picker cards stay opaque; the terminal card has its own conditional fill. Only the
/// Explorer surface uses this glass fill.
pub fn paint_glass_card(
    painter: &egui::Painter,
    rect: egui::Rect,
    layout: &Layout,
    native_backdrop_applied: bool,
) {
    let fill = explorer_card_glass(native_backdrop_applied);
    painter.rect_filled(
        rect.translate(egui::vec2(0.0, 2.0)).expand(1.0),
        CornerRadius::same(layout.card_radius.saturating_add(1)),
        GLASS_SHADOW,
    );
    painter.rect(
        rect,
        CornerRadius::same(layout.card_radius),
        fill,
        Stroke::new(1.0, BORDER),
        StrokeKind::Inside,
    );
}

/// Paint the Explorer header with the glass tone and top highlight.
pub fn paint_glass_card_header(
    ui: &Ui,
    rect: egui::Rect,
    layout: &Layout,
    native_backdrop_applied: bool,
) {
    let radius = CornerRadius {
        nw: layout.card_radius,
        ne: layout.card_radius,
        sw: 0,
        se: 0,
    };
    let fill = explorer_header_glass(native_backdrop_applied);
    ui.painter().rect_filled(rect, radius, fill);
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
    let color = if !enabled {
        TEXT_DISABLED
    } else if selected || hovered {
        TEXT_PRIMARY
    } else {
        TEXT_TERTIARY
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
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

/// Monochrome app-drawn glyphs for header actions and new-tab menu rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconGlyph {
    /// Simple app-drawn plus: open another workspace.
    Plus,
    /// Outlined panel with a right column: show/hide the explorer.
    PanelRight,
    /// Heartbeat line: workspace and integration diagnostics.
    Diagnostics,
    /// List rows with markers: optional OpenCode sessions.
    SessionList,
    /// Small X: dismiss the picker or a chrome surface.
    Close,
    /// Small terminal window with a prompt chevron.
    Terminal,
    /// Branch with three nodes: read-only source control.
    SourceControl,
    /// Two offset rounded sheets: the file tree, beside source control.
    Folders,
    /// Forked chevrons: linked-worktree mark in the REPOS header.
    LinkedWorktree,
}

/// Linked-worktree mark beside the worktree name. Not the source-control toggle.
pub const LINKED_WORKTREE_ICON: f32 = 12.0;

/// Widest app-drawn header glyph (Explorer panel/Diagnostics).
pub const ICON_GLYPH_WIDTH: f32 = 14.0;

/// Square hit target for an icon-only action at `height`.
pub fn icon_action_extent(height: f32) -> f32 {
    height.max(24.0)
}

/// Optional colors for the three source-control rings.
///
/// `None` keeps the icon's current color, including hover and selected.
/// The stem and curve are not part of this and always use that color.
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceControlRings {
    /// Highest ring, about y=-4.25. New changes use [`SUCCESS`].
    pub highest: Option<Color32>,
    /// Vertically middle ring, the branch tip. Modified changes use [`WARNING`].
    pub middle: Option<Color32>,
    /// Lowest ring, about y=4.25. Deleted and conflicted changes use [`ERROR`].
    pub lowest: Option<Color32>,
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
    icon_action_with(ui, id, glyph, label, selected, height, gap, None)
}

/// Source-control toggle. Ring overrides do not replace the stem, the curve,
/// or a ring whose change type is absent.
pub fn source_control_action(
    ui: &mut Ui,
    id: egui::Id,
    label: &str,
    selected: bool,
    height: f32,
    gap: Color32,
    rings: SourceControlRings,
) -> ActionResponse {
    icon_action_with(
        ui,
        id,
        IconGlyph::SourceControl,
        label,
        selected,
        height,
        gap,
        Some(rings),
    )
}

fn icon_action_with(
    ui: &mut Ui,
    id: egui::Id,
    glyph: IconGlyph,
    label: &str,
    selected: bool,
    height: f32,
    gap: Color32,
    rings: Option<SourceControlRings>,
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
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    let color = if selected || hovered {
        TEXT_PRIMARY
    } else {
        inactive_icon_color()
    };
    paint_icon_with(ui.painter(), rect, glyph, color, rings, hovered, selected);
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

pub(crate) fn paint_icon(
    painter: &egui::Painter,
    rect: egui::Rect,
    glyph: IconGlyph,
    color: Color32,
) {
    paint_icon_with(painter, rect, glyph, color, None, false, false);
}

fn paint_icon_with(
    painter: &egui::Painter,
    rect: egui::Rect,
    glyph: IconGlyph,
    color: Color32,
    rings: Option<SourceControlRings>,
    hovered: bool,
    selected: bool,
) {
    let center = rect.center();
    let point = |x: f32, y: f32| center + egui::vec2(x, y);
    let stroke = Stroke::new(1.5, color);
    let line = |points: Vec<egui::Pos2>| {
        painter.add(egui::Shape::line(points, stroke));
    };
    match glyph {
        IconGlyph::Plus => {
            line(vec![point(-6.0, 0.0), point(6.0, 0.0)]);
            line(vec![point(0.0, -6.0), point(0.0, 6.0)]);
        }
        IconGlyph::PanelRight => {
            let panel = egui::Rect::from_center_size(center, egui::vec2(ICON_GLYPH_WIDTH, 11.0));
            painter.rect_stroke(panel, CornerRadius::same(2), stroke, StrokeKind::Middle);
            line(vec![point(2.0, -5.5), point(2.0, 5.5)]);
        }
        IconGlyph::Diagnostics => {
            let half = ICON_GLYPH_WIDTH / 2.0;
            line(vec![
                point(-half, 0.0),
                point(-4.0, 0.0),
                point(-3.0, -4.0),
                point(-1.5, 4.5),
                point(0.5, -3.0),
                point(2.0, 1.5),
                point(3.5, 0.0),
                point(half, 0.0),
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
        IconGlyph::Terminal => {
            let panel = egui::Rect::from_center_size(center, egui::vec2(ICON_GLYPH_WIDTH, 11.0));
            painter.rect_stroke(panel, CornerRadius::same(2), stroke, StrokeKind::Middle);
            line(vec![point(-3.5, -2.5), point(-1.0, 0.0), point(-3.5, 2.5)]);
            line(vec![point(-0.25, 2.5), point(2.75, 2.5)]);
        }
        IconGlyph::SourceControl => {
            // Hollow rings. `circle_stroke` paints outside the radius, so this
            // inner radius keeps the 1.5 stroke inside the 14px glyph box.
            let ring = 1.25;
            let join = ring + stroke.width * 0.5;
            let top = point(-3.25, -4.25);
            let bottom = point(-3.25, 4.25);
            let right = point(3.75, -1.0);
            // Stem only between the top and bottom rings, stopping in the ring
            // stroke so a cap cannot cross the hollow center.
            line(vec![point(-3.25, -4.25 + join), point(-3.25, 4.25 - join)]);
            // Branch leaves the middle of the stem and meets the right ring
            // along its radius, one bend, no extra node.
            let start = point(-3.25 + stroke.width * 0.5, 0.35);
            let outward = start - right;
            let outward = outward / outward.length();
            let end = right + outward * join;
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                [
                    start,
                    start + egui::vec2(2.2, -0.15),
                    end + outward * 2.0,
                    end,
                ],
                false,
                Color32::TRANSPARENT,
                stroke,
            ));
            // Stem and curve stay on `stroke`. Only the three existing rings
            // may take a change color; an absent type keeps the icon color.
            // A status color fills the interior while hovered or selected.
            // The 1.5 stroke sits outside this radius, so the fill stops at
            // the hollow edge. An absent type stays hollow.
            let rings = rings.unwrap_or_default();
            let paint_ring = |at: egui::Pos2, tint: Option<Color32>| {
                let ring_color = tint.unwrap_or(color);
                if (hovered || selected) && tint.is_some() {
                    painter.circle_filled(at, ring, ring_color);
                }
                painter.circle_stroke(at, ring, Stroke::new(stroke.width, ring_color));
            };
            paint_ring(top, rings.highest);
            paint_ring(right, rings.middle);
            paint_ring(bottom, rings.lowest);
        }
        IconGlyph::Folders => {
            // Two portrait pages. `front` is already absolute; do not pass
            // its edges back through `point()`.
            let front = egui::Rect::from_min_max(point(-2.5, -3.5), point(5.5, 5.5));
            let back = front.translate(egui::vec2(-2.0, -2.0));
            let pad = 2.0;
            let left_of_front = egui::Rect::from_min_max(
                egui::pos2(back.left() - pad, back.top() - pad),
                egui::pos2(front.left(), back.bottom() + pad),
            );
            let above_front = egui::Rect::from_min_max(
                egui::pos2(front.left(), back.top() - pad),
                egui::pos2(back.right() + pad, front.top()),
            );
            let radius = CornerRadius::same(2);
            painter.with_clip_rect(left_of_front).rect_stroke(
                back,
                radius,
                stroke,
                StrokeKind::Middle,
            );
            painter.with_clip_rect(above_front).rect_stroke(
                back,
                radius,
                stroke,
                StrokeKind::Middle,
            );
            painter.rect_stroke(front, radius, stroke, StrokeKind::Middle);
            let fold = 2.2;
            let right = front.right();
            let top = front.top();
            painter.line_segment(
                [egui::pos2(right - fold, top), egui::pos2(right, top + fold)],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(right - fold, top),
                    egui::pos2(right - fold, top + fold),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(right - fold, top + fold),
                    egui::pos2(right, top + fold),
                ],
                stroke,
            );
        }
        IconGlyph::LinkedWorktree => {
            // Worktree-compact: a short stem forks into two right-pointing
            // chevrons. Cap-sized dots stand in for round ends; egui lines
            // have no cap style. Not the three-ring source-control toggle.
            let cap = stroke.width * 0.5;
            let fork = point(-0.7, 0.0);
            line(vec![point(-4.6, 0.0), fork]);
            painter.circle_filled(point(-4.6, 0.0), cap, color);
            painter.circle_filled(fork, cap, color);
            for through_y in [-2.2, 2.2] {
                let through = egui::vec2(2.0, through_y);
                let from_fork = through - egui::vec2(-0.7, 0.0);
                let unit = from_fork / from_fork.length();
                let perp = egui::vec2(-unit.y, unit.x);
                let tip = through + unit * 3.2;
                let back = tip - unit * 1.7;
                line(vec![fork, point(through.x, through.y), point(tip.x, tip.y)]);
                for barb in [back + perp * 1.15, back - perp * 1.15] {
                    line(vec![point(barb.x, barb.y), point(tip.x, tip.y)]);
                    painter.circle_filled(point(barb.x, barb.y), cap, color);
                }
                painter.circle_filled(point(tip.x, tip.y), cap, color);
            }
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
    let color = if !enabled {
        TEXT_DISABLED
    } else if selected || hovered {
        TEXT_PRIMARY
    } else {
        TEXT_TERTIARY
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
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

/// Leading mark on a new-tab menu row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuLeading {
    /// App-drawn chrome glyph, colored with the label.
    Glyph(IconGlyph),
    /// Vendored SVG. Painted at 16px with the default white tint, so hover does not recolor it.
    Image {
        /// `bytes://` URI that keys the texture cache.
        uri: &'static str,
        /// Embedded SVG document.
        bytes: &'static [u8],
    },
}

/// Galley origin that puts the letterform body on `center_y`.
///
/// The body runs from the tight ink top to the baseline. `mesh_bounds` also
/// extends through descenders, so centering that box would lift "OpenCode"
/// above "Terminal".
fn menu_label_origin_y(galley: &egui::Galley, center_y: f32) -> f32 {
    let Some(row) = galley.rows.first() else {
        return center_y - galley.size().y * 0.5;
    };
    let Some(glyph) = row.glyphs.first() else {
        return center_y - galley.size().y * 0.5;
    };
    let baseline = row.pos.y + glyph.pos.y;
    let ink_top = galley.mesh_bounds.min.y;
    let body_center = if ink_top.is_finite() {
        (ink_top + baseline) * 0.5
    } else {
        baseline - glyph.font_face_ascent * 0.5
    };
    center_y - body_center
}

/// Full-width menu row: leading mark and left-aligned label in one click target.
///
/// [`ghost_action`] stays centered and icon-free. This row fills a justified
/// menu with an 8px inset and an 8px gap after the 14px icon slot. Both labels
/// share one baseline, and the letterform body — not the galley line box — is
/// centered on the icon.
pub(crate) fn menu_labeled_action(
    ui: &mut Ui,
    id: egui::Id,
    leading: MenuLeading,
    label: &str,
    height: f32,
    gap: Color32,
) -> ActionResponse {
    const PAD_X: f32 = 8.0;
    const ICON_GAP: f32 = 8.0;
    let font = FontId::proportional(11.5);
    let text_width = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), TEXT_TERTIARY)
        .size()
        .x;
    let height = height.max(24.0);
    let width = (PAD_X + ICON_GLYPH_WIDTH + ICON_GAP + text_width + PAD_X).max(24.0);
    // `allocate_at_least` keeps the justified menu width. During the popup's
    // sizing pass, cross-justify is off, so the menu measures the widest row.
    let (rect, _) = ui.allocate_at_least(egui::vec2(width, height), egui::Sense::hover());
    let response = ui.interact(rect, id, egui::Sense::click());
    let hovered = response.hovered();
    let fill = if hovered {
        BG_HOVER
    } else {
        Color32::TRANSPARENT
    };
    let color = if hovered { TEXT_PRIMARY } else { TEXT_TERTIARY };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(
            rect.left() + PAD_X + ICON_GLYPH_WIDTH * 0.5,
            rect.center().y,
        ),
        egui::vec2(ICON_GLYPH_WIDTH, ICON_GLYPH_WIDTH),
    );
    match leading {
        MenuLeading::Glyph(glyph) => paint_icon(ui.painter(), icon_rect, glyph, color),
        MenuLeading::Image { uri, bytes } => {
            let image_rect =
                egui::Rect::from_center_size(icon_rect.center(), egui::Vec2::splat(16.0));
            egui::Image::from_bytes(uri, bytes).paint_at(ui, image_rect);
        }
    }
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, color);
    ui.painter().galley(
        egui::pos2(
            icon_rect.right() + ICON_GAP,
            menu_label_origin_y(&galley, rect.center().y),
        ),
        galley,
        color,
    );
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

/// Borderless secondary action used by the folder picker.
pub fn quiet_secondary_action(
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
            stroke: Stroke::NONE,
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
    install_icon_loader(ctx);
}

/// Install the SVG loader that decodes the embedded Explorer icons, once per
/// context.
///
/// Only [`SvgLoader`] is added. No file, HTTP, or raster loader is installed,
/// so icon bytes come only from the embedded vendored assets.
fn install_icon_loader(ctx: &egui::Context) {
    if !ctx.is_loader_installed(SvgLoader::ID) {
        ctx.add_image_loader(Arc::new(SvgLoader::default()));
    }
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
            bg_fill: SCROLL_THUMB_HOVER,
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

/// Floating overlay scrollbars that stay hidden at rest and fade in on hover.
///
/// The bars allocate zero width, so content never reflows when one appears.
pub fn scroll_style() -> egui::style::ScrollStyle {
    let mut scroll = egui::style::ScrollStyle::floating();
    scroll.bar_width = 8.0;
    scroll.floating_width = 3.0;
    scroll.floating_allocated_width = 0.0;
    scroll.handle_min_length = 24.0;
    scroll.bar_inner_margin = 2.0;
    scroll.bar_outer_margin = 0.0;
    scroll.foreground_color = false;
    scroll.dormant_background_opacity = 0.0;
    scroll.active_background_opacity = 0.0;
    scroll.interact_background_opacity = 0.35;
    scroll.dormant_handle_opacity = 0.0;
    scroll.active_handle_opacity = 0.60;
    scroll.interact_handle_opacity = 1.0;
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
/// With native vibrancy/Mica active the lower alpha (135 instead of 215) lets
/// the OS material show through. Without it, the darker fallback alpha keeps
/// the in-app glass readable even when the compositor ignores transparency.
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
        assert_eq!(ACCENT, Color32::from_rgb(0x8B, 0xD5, 0xA7));
        assert_eq!(ACCENT_HOVER, Color32::from_rgb(0xA3, 0xE3, 0xBA));
        assert_eq!(ACCENT_PRESSED, Color32::from_rgb(0x72, 0xBF, 0x8E));
        assert_eq!(ACCENT_TEXT, Color32::from_rgb(0x08, 0x14, 0x0D));
        assert_eq!(ACCENT_TINT, Color32::from_rgb(0x26, 0x35, 0x2C));
        assert_eq!(FOCUS, Color32::from_rgb(0xB8, 0xF0, 0xC9));
        assert_eq!(SUCCESS, Color32::from_rgb(0x4F, 0xCF, 0xA8));
        assert_eq!(
            BG_TERMINAL_GLASS,
            Color32::from_rgba_unmultiplied_const(11, 13, 16, 160)
        );
        assert_eq!(TERMINAL_BACKGROUND, [0x0B, 0x0D, 0x10]);
        assert_eq!(TERMINAL_BRIGHT_BLACK, [0x70, 0x7B, 0x8E]);
    }

    #[test]
    fn mint_accent_stays_distinct_from_semantic_status() {
        let accent = ACCENT.to_srgba_unmultiplied();
        assert!(
            accent[1] > accent[0] && accent[1] > accent[2],
            "accent must be a calm green: {accent:?}"
        );
        assert_ne!(ACCENT, SUCCESS, "success must not reuse the accent tone");
        let success = SUCCESS.to_srgba_unmultiplied();
        assert!(
            success[1] > success[2] && success[2] > success[0],
            "success must stay a cool teal distinct from the mint accent: {success:?}"
        );
        assert_ne!(ACCENT, WARNING, "warning must not reuse the accent tone");
        assert_ne!(ACCENT, ERROR, "error must not reuse the accent tone");
        assert_ne!(ACCENT, INFO, "info must not reuse the accent tone");
    }

    #[test]
    fn splitter_fade_helper_is_bounded_and_discreet() {
        assert!((SPLITTER_FADE_TIME - 0.14).abs() < f32::EPSILON);
        assert!(splitter_opacity(0.0, false).abs() < f32::EPSILON);
        assert!((splitter_opacity(1.0, false) - SPLITTER_HOVER_OPACITY).abs() < f32::EPSILON);
        assert!((splitter_opacity(1.0, true) - SPLITTER_DRAG_OPACITY).abs() < f32::EPSILON);
        assert_eq!(splitter_tone(false), BORDER_CONTROL);
        assert_eq!(splitter_tone(true), ACCENT);

        let hover_line = splitter_line_color(1.0, false).to_srgba_unmultiplied();
        let border = BORDER_CONTROL.to_srgba_unmultiplied();
        assert!(hover_line[..3]
            .iter()
            .zip(&border[..3])
            .all(|(actual, expected)| actual.abs_diff(*expected) <= 1));
        assert!(hover_line[3] > 0 && hover_line[3] < 255);
        let drag_line = splitter_line_color(1.0, true).to_srgba_unmultiplied();
        let accent = ACCENT.to_srgba_unmultiplied();
        assert!(drag_line[..3]
            .iter()
            .zip(&accent[..3])
            .all(|(actual, expected)| actual.abs_diff(*expected) <= 1));
        assert!(drag_line[3] > 0 && drag_line[3] < 255);
        let grip = splitter_grip_fill(1.0, false).to_srgba_unmultiplied();
        let elevated = BG_ELEVATED.to_srgba_unmultiplied();
        assert!(grip[..3]
            .iter()
            .zip(&elevated[..3])
            .all(|(actual, expected)| actual.abs_diff(*expected) <= 1));
        assert!(grip[3] > 0 && grip[3] < 255);
    }

    #[test]
    fn glass_tokens_are_translucent_and_tonal() {
        for (glass, opaque, alpha) in [
            (BG_CARD_GLASS, BG_CARD, 188),
            (BG_HEADER_GLASS, BG_HEADER, 194),
        ] {
            let [r, g, b, a] = glass.to_srgba_unmultiplied();
            let [opaque_r, opaque_g, opaque_b, _] = opaque.to_srgba_unmultiplied();
            assert!(
                r.abs_diff(opaque_r) <= 1 && g.abs_diff(opaque_g) <= 1 && b.abs_diff(opaque_b) <= 1,
                "glass tint must match the opaque tone within premultiplied rounding: {glass:?}"
            );
            assert_eq!(
                a, alpha,
                "glass alpha must match the contract value: {glass:?}"
            );
        }

        let terminal_glass = BG_TERMINAL_GLASS.to_srgba_unmultiplied();
        let terminal = BG_TERMINAL.to_srgba_unmultiplied();
        assert!(
            terminal_glass[..3]
                .iter()
                .zip(&terminal[..3])
                .all(|(actual, expected)| actual.abs_diff(*expected) <= 1),
            "terminal glass must preserve the BG_TERMINAL RGB: {terminal_glass:?} vs {terminal:?}"
        );
        assert_eq!(terminal_glass[3], 160);
        assert_eq!(terminal_surface(false), BG_TERMINAL_GLASS);
        assert_eq!(terminal_surface(true), BG_TERMINAL);
        assert_eq!(terminal_overlay_surface(false), None);
        assert_eq!(terminal_overlay_surface(true), Some(BG_TERMINAL));
        let layout = Layout::for_window(1280.0, 800.0);
        assert_eq!(terminal_frame(&layout, false).fill, BG_TERMINAL_GLASS);
        assert_eq!(terminal_frame(&layout, true).fill, BG_TERMINAL);
        assert!(
            BG_CLEAR_NATIVE.a() < BG_TERMINAL_GLASS.a()
                && BG_TERMINAL_GLASS.a() < BG_CARD_GLASS.a()
                && BG_CARD_GLASS.a() < BG_HEADER_GLASS.a()
                && BG_HEADER_GLASS.a() < BG_CLEAR.a(),
            "the documented alpha order must hold"
        );
        assert!(GLASS_HIGHLIGHT.a() < 20);
        assert!(GLASS_SHADOW.a() < 80);
    }

    #[test]
    fn explorer_glass_selects_native_or_fallback_tokens() {
        assert_eq!(explorer_card_glass(true), BG_CARD_GLASS_NATIVE);
        assert_eq!(explorer_card_glass(false), BG_CARD_GLASS);
        assert_eq!(explorer_header_glass(true), BG_HEADER_GLASS_NATIVE);
        assert_eq!(explorer_header_glass(false), BG_HEADER_GLASS);

        assert_eq!(
            BG_CARD_GLASS_NATIVE,
            Color32::from_rgba_unmultiplied_const(23, 26, 32, 24)
        );
        assert_eq!(
            BG_CARD_GLASS,
            Color32::from_rgba_unmultiplied_const(23, 26, 32, 188)
        );
        assert_eq!(
            BG_HEADER_GLASS_NATIVE,
            Color32::from_rgba_unmultiplied_const(20, 23, 28, 32)
        );
        assert_eq!(
            BG_HEADER_GLASS,
            Color32::from_rgba_unmultiplied_const(20, 23, 28, 194)
        );
        assert!(BG_CARD_GLASS_NATIVE.a() < BG_HEADER_GLASS_NATIVE.a());
        assert!(BG_CARD_GLASS.a() < BG_HEADER_GLASS.a());
    }

    #[test]
    fn fallback_explorer_glass_text_meets_aa_over_bright_desktop() {
        fn composite(over: Color32, under: [f32; 3]) -> [f32; 3] {
            let [r, g, b, a] = over.to_srgba_unmultiplied();
            let alpha = f32::from(a) / 255.0;
            [
                f32::from(r) / 255.0 * alpha + under[0] * (1.0 - alpha),
                f32::from(g) / 255.0 * alpha + under[1] * (1.0 - alpha),
                f32::from(b) / 255.0 * alpha + under[2] * (1.0 - alpha),
            ]
        }
        fn linear(channel: f32) -> f32 {
            if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        }
        fn luminance(rgb: [f32; 3]) -> f32 {
            0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])
        }
        fn contrast(a: [f32; 3], b: [f32; 3]) -> f32 {
            let (a, b) = (luminance(a), luminance(b));
            let (hi, lo) = if a > b { (a, b) } else { (b, a) };
            (hi + 0.05) / (lo + 0.05)
        }
        fn text_color(color: Color32) -> [f32; 3] {
            let [r, g, b, _] = color.to_srgba_unmultiplied();
            [
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
            ]
        }

        let clear = composite(BG_CLEAR, [1.0, 1.0, 1.0]);
        let card = composite(BG_CARD_GLASS, clear);
        let header = composite(BG_HEADER_GLASS, clear);
        for (name, color) in [
            ("primary", TEXT_PRIMARY),
            ("secondary", TEXT_SECONDARY),
            ("tertiary", TEXT_TERTIARY),
        ] {
            let ratio = contrast(text_color(color), card);
            assert!(
                ratio >= 4.5,
                "{name} text on fallback Explorer card must meet 4.5:1, got {ratio:.2}"
            );
            let ratio = contrast(text_color(color), header);
            assert!(
                ratio >= 4.5,
                "{name} text on fallback Explorer header must meet 4.5:1, got {ratio:.2}"
            );
        }
    }

    #[test]
    fn inactive_header_tokens_use_the_documented_opacity() {
        let tab = inactive_tab_text().to_srgba_unmultiplied();
        let icon = inactive_icon_color().to_srgba_unmultiplied();
        let tab_base = TEXT_SECONDARY.to_srgba_unmultiplied();
        let icon_base = TEXT_TERTIARY.to_srgba_unmultiplied();
        assert!((TAB_INACTIVE_OPACITY - 0.72).abs() < f32::EPSILON);
        assert!((ICON_INACTIVE_OPACITY - 0.80).abs() < f32::EPSILON);
        assert!(
            tab[0].abs_diff(tab_base[0]) <= 1
                && tab[1].abs_diff(tab_base[1]) <= 1
                && tab[2].abs_diff(tab_base[2]) <= 1,
            "inactive tab tone must preserve the secondary hue: {tab:?} vs {tab_base:?}"
        );
        assert!(
            icon[0].abs_diff(icon_base[0]) <= 1
                && icon[1].abs_diff(icon_base[1]) <= 1
                && icon[2].abs_diff(icon_base[2]) <= 1,
            "inactive icon tone must preserve the tertiary hue: {icon:?} vs {icon_base:?}"
        );
        assert!(tab[3] < tab_base[3]);
        assert!(icon[3] < icon_base[3]);
    }

    #[test]
    fn idle_workspace_tab_token_is_translucent_black() {
        let [r, g, b, a] = BG_TAB_IDLE.to_srgba_unmultiplied();
        assert_eq!([r, g, b], [0, 0, 0]);
        assert_eq!(
            a, 84,
            "idle tab fill must stay the documented translucent black"
        );
        assert!(a > 0 && a < 255, "idle tab fill must be translucent: {a}");
        assert_eq!(BG_TAB_HOVER, Color32::from_rgb(0x15, 0x1C, 0x18));
        assert_eq!(BG_TAB_SELECTED, Color32::from_rgb(0x1B, 0x23, 0x1F));
        for (name, tone) in [("hover", BG_TAB_HOVER), ("selected", BG_TAB_SELECTED)] {
            let [r, g, b, a] = tone.to_srgba_unmultiplied();
            assert_eq!(a, 255, "{name} workspace tab must be opaque");
            assert!(
                g >= r && g >= b,
                "{name} workspace tab must keep a green-gray undertone: {tone:?}"
            );
        }
        let hover: u16 =
            BG_TAB_HOVER.r() as u16 + BG_TAB_HOVER.g() as u16 + BG_TAB_HOVER.b() as u16;
        let selected: u16 =
            BG_TAB_SELECTED.r() as u16 + BG_TAB_SELECTED.g() as u16 + BG_TAB_SELECTED.b() as u16;
        assert!(
            selected > hover,
            "selected tab must be a luminance step above hover"
        );
        let stroke = workspace_tab_stroke();
        assert_eq!(stroke.width, 1.0);
        let [r, g, b, a] = stroke.color.to_srgba_unmultiplied();
        let selected = BG_TAB_SELECTED.to_srgba_unmultiplied();
        assert!(
            a > 64 && a < 255,
            "workspace-tab stroke must be stronger than alpha 64 and still not opaque"
        );
        assert_eq!(a, WORKSPACE_TAB_STROKE_ALPHA);
        assert!(
            selected[0] > WORKSPACE_TAB_STROKE_DARKEN
                && selected[1] > WORKSPACE_TAB_STROKE_DARKEN
                && selected[2] > WORKSPACE_TAB_STROKE_DARKEN,
            "darkening the selected fill must not clamp to black"
        );
        assert_eq!(
            stroke.color,
            Color32::from_rgba_unmultiplied(
                selected[0] - WORKSPACE_TAB_STROKE_DARKEN,
                selected[1] - WORKSPACE_TAB_STROKE_DARKEN,
                selected[2] - WORKSPACE_TAB_STROKE_DARKEN,
                WORKSPACE_TAB_STROKE_ALPHA,
            )
        );
        assert_ne!(
            (r, g, b),
            (39, 47, 43),
            "the previous lighter stroke must not return"
        );
        assert_ne!(stroke.color, Color32::from_rgb(0x2B, 0x33, 0x2F));
        assert!(
            g >= r && g >= b,
            "workspace-tab stroke must keep the tab green-gray family: {stroke:?}"
        );
        assert!(r.abs_diff(selected[0] - WORKSPACE_TAB_STROKE_DARKEN) <= 1);
        assert_ne!(stroke.color, ACCENT);
        assert_ne!(stroke.color, FOCUS);
        assert_ne!(stroke.color, BORDER);
        assert_ne!(stroke.color, BORDER_CONTROL);
    }

    #[test]
    fn scroll_style_is_floating_and_hidden_at_rest() {
        let style = scroll_style();
        assert!(style.floating);
        assert_eq!(style.allocated_width(), 0.0);
        assert_eq!(style.dormant_handle_opacity, 0.0);
        assert_eq!(style.dormant_background_opacity, 0.0);
        assert!(style.active_handle_opacity > 0.0);
        assert!(style.interact_handle_opacity >= style.active_handle_opacity);
        assert!(style.bar_width > style.floating_width);
        assert!(style.handle_min_length >= 24.0);
        assert!(!style.foreground_color);
        assert_eq!(visuals().widgets.inactive.bg_fill, SCROLL_THUMB);
        assert_eq!(visuals().widgets.hovered.bg_fill, SCROLL_THUMB_HOVER);
        assert_eq!(visuals().widgets.active.bg_fill, SCROLL_THUMB_HOVER);
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
        assert_eq!(wide.right_card_width, 360.0);
        assert_eq!(wide.explorer_default_width(), 360.0);
        assert!(wide.explorer_max_width(wide.content_width()) > 360.0);
        let mid = Layout::for_window(1000.0, 800.0);
        assert!(
            (mid.right_card_width - mid.content_width() * 0.35).abs() < 0.01,
            "900–1099 explorer default must stay at 35% of content width"
        );
        let compact_edge = Layout::for_window(899.0, 800.0);
        assert_eq!(compact_edge.right_card_width, 288.0);
        let mid_edge = Layout::for_window(1099.0, 800.0);
        assert_eq!(mid_edge.right_card_width, 360.0);
        assert_eq!(wide.max_tabs, 5);
        assert_eq!(wide.top_height, 26.0);
        assert_eq!(wide.tab_height, 26.0);
        assert_eq!(wide.session_tab_height, 28.0);
        assert_eq!(wide.tab_strip_height, 36.0);
        assert_eq!(wide.header_top(), 8.0);
        assert_eq!(wide.content_top(), 42.0);

        let compact = Layout::for_window(720.0, 480.0);
        assert!(compact.compact);
        assert_eq!(compact.gutter, 8.0);
        assert_eq!(compact.top_height, 24.0);
        assert_eq!(compact.tab_height, 24.0);
        assert_eq!(compact.session_tab_height, 26.0);
        assert_eq!(compact.tab_strip_height, 34.0);
        assert_eq!(compact.header_top(), 6.0);
        assert_eq!(compact.content_top(), 36.0);
        assert_eq!(compact.right_card_width, 288.0);
        assert_eq!(
            compact.explorer_width(440.0, compact.content_width()),
            288.0
        );
        assert_eq!(
            compact.explorer_width(400.0, compact.content_width()),
            288.0
        );
        assert_eq!(
            compact.explorer_width(360.0, compact.content_width()),
            288.0
        );
        assert!(compact.explorer_max_width(compact.content_width()) > compact.right_card_width);
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
        assert_eq!(wide.explorer_default_width(), 360.0);
        assert_eq!(
            wide.explorer_width(440.0, wide.content_width()),
            360.0,
            "the pre-Phase-12 440 default must migrate to the new 360 default"
        );
        assert_eq!(
            wide.explorer_width(400.0, wide.content_width()),
            360.0,
            "the interim Phase 12 400 default must migrate to the new 360 default"
        );
        assert_eq!(wide.explorer_width(360.0, wide.content_width()), 360.0);
        assert_eq!(wide.explorer_width(300.0, wide.content_width()), 300.0);
        assert_eq!(wide.explorer_width(320.0, wide.content_width()), 320.0);
        assert_eq!(wide.explorer_width(380.0, wide.content_width()), 380.0);
        assert_eq!(wide.explorer_width(420.0, wide.content_width()), 420.0);
        assert_eq!(
            compact.explorer_width(420.0, compact.content_width()),
            compact.explorer_max_width(compact.content_width())
        );
        let max = wide.explorer_max_width(wide.content_width());
        assert!(
            wide.content_width() - wide.gap - max >= wide.min_left_width(),
            "left surface lost its usable width"
        );
    }

    #[test]
    fn clear_color_respects_the_kill_switch_and_native_backdrop() {
        let fallback = BG_CLEAR.to_srgba_unmultiplied();
        let native_token = BG_CLEAR_NATIVE.to_srgba_unmultiplied();
        assert!(
            fallback[0].abs_diff(10) <= 1
                && fallback[1].abs_diff(11) <= 1
                && fallback[2].abs_diff(14) <= 1
        );
        assert_eq!(fallback[3], 215);
        assert!(
            native_token[0].abs_diff(10) <= 1
                && native_token[1].abs_diff(11) <= 1
                && native_token[2].abs_diff(14) <= 1
        );
        assert_eq!(native_token[3], 135);
        assert!(
            fallback[3] > native_token[3],
            "the no-native fallback must be darker than the native clear"
        );
        assert_eq!(BG_CLEAR_SOLID, Color32::from_rgb(10, 11, 14));

        let in_app = clear_color();
        assert_eq!(in_app, clear_color_with_native(false));

        let expected_in_app_alpha = if TRANSLUCENT_WINDOW {
            215.0 / 255.0
        } else {
            1.0
        };
        assert!((in_app[3] - expected_in_app_alpha).abs() < 0.001);

        let native = clear_color_with_native(true);
        let expected_native_alpha = if TRANSLUCENT_WINDOW {
            135.0 / 255.0
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
        assert_eq!(palette.cursor, [215, 221, 231]);
        assert_eq!(palette.ansi[8], [112, 123, 142]);
        assert_eq!(palette.selection_background, [46, 58, 78]);
    }
}
