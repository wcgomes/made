//! egui surfaces for the Calm Editor design.
//!
//! The terminal is the hero: the floating header, full-height explorer, and
//! mutually exclusive terminal/preview surfaces are deliberately quiet. There
//! is no routine status bar or integration banner and no persistent header
//! band: wordmark, tabs, and icon actions sit on the clear/native backdrop.
//! The full-height Explorer card uses the theme's in-app frosted-glass
//! approximation; the terminal is glass on its primary screen and opaque for
//! alternate-screen TUIs, while preview and picker cards stay opaque. All
//! display text is sanitized and every control keeps a visible focus state.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, RichText, Sense, Stroke, StrokeKind, Ui,
    UiBuilder,
};

use crate::app::{AppModel, WorkspaceId};
use crate::diagnostics::{DiagnosticsPill, DiagnosticsSnapshot};
use crate::explorer_view::{ExplorerPane, PreviewUiAction};
use crate::integration_status::IntegrationStatus;
use crate::integrations::{
    rank_order, IntegrationList, IntegrationRef, Mark, Presentation, CLAUDE_CODE, GROK, OPENCODE,
};
use crate::sanitize::{sanitize_copy_text, sanitize_display, sanitize_text};
use crate::session_map::{truncate_end, truncate_path, SessionActivity, SessionRow};
use crate::terminal_view::TerminalUiOutput;
use crate::theme::{self, Layout};
use crate::tui::TerminalTarget;
use crate::updates::{UpdateAction, UpdatePanel};
use crate::workbench::{LeftSurface, Workbench};
use crate::{APP_DESCRIPTION, APP_NAME, APP_VERSION};
use made_platform::{Clipboard, LogIssueSeverity, PlatformError};

// ── Status tone mapping ─────────────────────────────────────────────────────

fn status_tone(status: IntegrationStatus) -> (Color32, Color32, &'static str) {
    match status {
        IntegrationStatus::Connected => (theme::SUCCESS, theme::SUCCESS_TINT, "●"),
        IntegrationStatus::Inactive => (theme::TEXT_TERTIARY, theme::BG_ELEVATED, "○"),
        IntegrationStatus::Degraded => (theme::WARNING, theme::WARNING_TINT, "!"),
        IntegrationStatus::Unsupported => (theme::ERROR, theme::ERROR_TINT, "×"),
    }
}

/// Visible OpenCode pill label. Connected, including after a reconnect, paints
/// `ready` in the connected success tone. [`IntegrationStatus::label`] stays
/// the machine code `connected`.
fn opencode_pill_label(status: IntegrationStatus) -> &'static str {
    match status {
        IntegrationStatus::Connected => "ready",
        IntegrationStatus::Inactive
        | IntegrationStatus::Degraded
        | IntegrationStatus::Unsupported => status.label(),
    }
}

fn status_pill_ui(
    ui: &mut Ui,
    status: IntegrationStatus,
    reason: &str,
    layout: &Layout,
) -> egui::Response {
    let (tone, tint, symbol) = status_tone(status);
    paint_status_pill(
        ui,
        symbol,
        opencode_pill_label(status),
        tone,
        tint,
        reason,
        layout,
    )
}

fn grok_status_pill_ui(
    ui: &mut Ui,
    pill: DiagnosticsPill,
    sentence: &str,
    layout: &Layout,
) -> egui::Response {
    let (tone, tint, symbol) = match pill {
        DiagnosticsPill::Inactive => status_tone(IntegrationStatus::Inactive),
        DiagnosticsPill::Degraded => status_tone(IntegrationStatus::Degraded),
        // Ready uses the connected pill's success tone, not its label.
        DiagnosticsPill::Ready => status_tone(IntegrationStatus::Connected),
    };
    paint_status_pill(ui, symbol, pill.label(), tone, tint, sentence, layout)
}

fn paint_status_pill(
    ui: &mut Ui,
    symbol: &str,
    label: &str,
    tone: Color32,
    tint: Color32,
    reason: &str,
    layout: &Layout,
) -> egui::Response {
    let text = format!("{symbol} {label}");
    theme::pill(ui, &text, tone, tint, layout.status_pill_height)
        .on_hover_text(sanitize_display(reason, 400))
}

// ── Picker ──────────────────────────────────────────────────────────────────

/// Folder picker shown when Made starts without a workspace.
pub fn picker_ui(model: &mut AppModel, ui: &mut Ui) {
    let layout = Layout::for_window(ui.available_width(), ui.available_height());
    egui::Frame::NONE
        .inner_margin(egui::Margin::same(layout.gutter as i8))
        .show(ui, |ui| picker_surface(model, ui, &layout));
}

/// Stable id for a picker action button.
pub fn picker_control_id(action: &str) -> egui::Id {
    egui::Id::unique(("made-picker-control", action))
}

/// The picker list's single focusable container.
pub fn picker_list_id() -> egui::Id {
    egui::Id::unique("made-picker-list-focus")
}

/// Stable id for the picker's current-path chip.
pub fn picker_path_id() -> egui::Id {
    egui::Id::unique("made-picker-path")
}

/// Current picker target used by the primary action and its tooltip.
pub fn picker_current_target(model: &AppModel) -> Option<String> {
    model
        .launch()
        .picker()
        .map(|picker| picker.path_target().display().to_string())
}

#[derive(Debug, Clone)]
struct PickerBackContext {
    label: String,
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PickerCacheKey {
    instance: u64,
    revision: u64,
    dir: PathBuf,
}

#[derive(Debug, Clone)]
struct PickerDisplayRow {
    index: usize,
    name: String,
    path: PathBuf,
    depth: usize,
    expanded: bool,
}

struct PickerEntryCache {
    key: PickerCacheKey,
    rows: Arc<Vec<PickerDisplayRow>>,
}

fn picker_cache_id() -> egui::Id {
    egui::Id::unique("made-picker-entry-cache")
}

fn invalidate_picker_cache(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<Arc<PickerEntryCache>>(picker_cache_id()));
}

fn picker_display_rows(
    ctx: &egui::Context,
    picker: &crate::workspace_picker::WorkspacePicker,
) -> Arc<Vec<PickerDisplayRow>> {
    let key = PickerCacheKey {
        instance: picker.instance_id(),
        revision: picker.revision(),
        dir: picker.current_dir().to_path_buf(),
    };
    if let Some(cache) = ctx.data(|data| data.get_temp::<Arc<PickerEntryCache>>(picker_cache_id()))
    {
        if cache.key == key {
            return cache.rows.clone();
        }
    }
    let rows = Arc::new(
        picker
            .rows()
            .iter()
            .enumerate()
            .map(|(index, row)| PickerDisplayRow {
                index,
                name: sanitize_display(row.name(), 200),
                path: row.path().to_path_buf(),
                depth: row.depth(),
                expanded: row.expanded(),
            })
            .collect::<Vec<_>>(),
    );
    ctx.data_mut(|data| {
        data.insert_temp(
            picker_cache_id(),
            Arc::new(PickerEntryCache {
                key,
                rows: rows.clone(),
            }),
        );
    });
    rows
}

fn picker_back_context(model: &AppModel) -> Option<PickerBackContext> {
    let id = model.active_workspace_id()?;
    let workbench = model.workspace(id)?;
    let path = workbench
        .launch_plan()
        .map(|plan| plan.workspace.path().to_path_buf())
        .unwrap_or_else(|| workbench.shell_cwd().to_path_buf());
    let label = workbench
        .launch_plan()
        .map(|plan| plan.workspace.display_name().to_owned())
        .unwrap_or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "workspace".to_owned())
        });
    Some(PickerBackContext { label, path })
}

fn picker_surface(model: &mut AppModel, ui: &mut Ui, layout: &Layout) {
    let mut command = None;
    let mut keyboard_move: Option<isize> = None;
    let mut keyboard_confirm = false;
    let mut keyboard_toggle = false;
    let mut keyboard_expand = false;
    let mut keyboard_collapse = false;
    let mut keyboard_up = false;
    let back = picker_back_context(model);

    brand_row(ui, layout, back.as_ref(), &mut command);

    let available = ui.available_size();
    let top_padding = ((available.y - 600.0) / 3.0).max(24.0);
    ui.add_space(top_padding);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new("Open a workspace")
                .font(FontId::proportional(theme::HERO_SIZE))
                .color(theme::TEXT_PRIMARY),
        );
        ui.label(theme::body(
            "Made starts its terminal and explorer only after you choose a folder.",
        ));
    });
    ui.add_space(14.0);

    let Some(picker) = model.launch().picker() else {
        return;
    };
    let target = picker.path_target().to_path_buf();
    let rows = picker_display_rows(ui.ctx(), picker);
    let selection = picker.selection();
    let picker_instance = picker.instance_id();
    let picker_revision = picker.revision();
    let picker_error = picker
        .error()
        .map(|error| sanitize_display(&error.to_string(), 600));
    let launch_error = model
        .launch()
        .error()
        .map(|error| sanitize_display(&error.message(), 600));
    let truncated = picker.truncated_entries();
    let notice_height = if truncated > 0 { 20.0 } else { 0.0 };
    let base_list_max = if layout.compact {
        160.0
    } else {
        (layout.window_height - 2.0 * layout.gutter - 280.0).clamp(120.0, 320.0)
    };
    let list_max = (base_list_max - notice_height).max(64.0);
    let card_width = (ui.available_width() - 48.0)
        .min(if layout.compact { 672.0 } else { 760.0 })
        .max(280.0);

    ui.horizontal(|ui| {
        let gutter = ((ui.available_width() - card_width) / 2.0).max(0.0);
        ui.add_space(gutter);
        ui.vertical(|ui| {
            ui.set_width(card_width);
            egui::Frame::new()
                .fill(theme::BG_CARD)
                .stroke(Stroke::new(1.0, theme::BORDER))
                .corner_radius(CornerRadius::same(12))
                .show(ui, |ui| {
                    ui.set_width(card_width);
                    picker_toolbar(ui, layout, &target, &mut command);
                    if truncated > 0 {
                        picker_truncated_notice(ui, layout, truncated);
                    }
                    picker_list(
                        ui,
                        layout,
                        &rows,
                        selection,
                        list_max,
                        (picker_instance, picker_revision),
                        &mut command,
                    );
                    if let Some(error) = picker_error.as_deref().or(launch_error.as_deref()) {
                        notice_strip(ui, layout, theme::ERROR_TINT, theme::ERROR, error, error);
                    }
                    picker_footer(ui, layout, selection, &target, &mut command);
                });
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                ui.label(theme::micro(
                    "Click a folder to expand or collapse it. Up/Down select, Right expands, Left collapses, Enter toggles, Ctrl/Cmd+Enter opens.",
                ));
            });
        });
    });

    let focused = ui.ctx().memory(|memory| memory.focused());
    let button_focused = focused.is_some_and(|id| id != picker_list_id());
    let input = ui.ctx().input(|input| input.events.clone());
    for event in input {
        if let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        {
            if key == egui::Key::Escape {
                command = Some(PickerCommand::Close);
                continue;
            }
            if key == egui::Key::Backspace {
                keyboard_up = true;
                continue;
            }
            if button_focused {
                // Focused buttons own Enter/Space and arrow keys. The global
                // list navigation must not hijack their activation.
                continue;
            }
            match key {
                egui::Key::ArrowDown => keyboard_move = Some(1),
                egui::Key::ArrowUp => keyboard_move = Some(-1),
                egui::Key::ArrowRight => keyboard_expand = true,
                egui::Key::ArrowLeft => keyboard_collapse = true,
                egui::Key::Enter if modifiers.command => keyboard_confirm = true,
                egui::Key::Enter => keyboard_toggle = true,
                _ => {}
            }
        }
    }

    if let Some(delta) = keyboard_move {
        model.picker_move(delta);
    }
    if keyboard_up {
        let _ = model.picker_up();
    }
    if keyboard_toggle {
        let _ = model.picker_toggle_selected();
    }
    if keyboard_expand {
        let _ = model.picker_expand_selected();
    }
    if keyboard_collapse {
        let _ = model.picker_collapse_selected();
    }
    if keyboard_confirm {
        let _ = model.confirm_picker();
    }
    match command {
        Some(PickerCommand::Up) => {
            let _ = model.picker_up();
        }
        Some(PickerCommand::Refresh) => {
            let _ = model.picker_refresh();
            invalidate_picker_cache(ui.ctx());
        }
        Some(PickerCommand::Toggle(index)) => {
            let _ = model.picker_toggle(index);
        }
        Some(PickerCommand::ConfirmCurrent) => {
            let _ = model.confirm_picker();
        }
        Some(PickerCommand::ConfirmRow(index)) => {
            let result = model
                .launch()
                .picker()
                .map(|picker| picker.confirm_row(index));
            match result {
                Some(Ok(plan)) => model.open_plan(plan),
                Some(Err(error)) => {
                    if let Some(picker) = model.launch_mut().picker_mut() {
                        picker.set_error(error);
                    }
                }
                None => {}
            }
        }
        Some(PickerCommand::Back) => {
            model.cancel_picker();
        }
        Some(PickerCommand::Close) => {
            model.close_picker();
        }
        None => {}
    }
}

fn brand_row(
    ui: &mut Ui,
    layout: &Layout,
    back: Option<&PickerBackContext>,
    command: &mut Option<PickerCommand>,
) {
    let height = if layout.compact { 32.0 } else { 36.0 };
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    theme::wordmark(&mut row);
    row.label(theme::micro(APP_VERSION));
    row.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let close = theme::icon_action(
            ui,
            picker_control_id("close"),
            theme::IconGlyph::Close,
            "Close picker",
            false,
            layout.button_height,
            theme::BG_CLEAR_SOLID,
        );
        if close.activated {
            *command = Some(PickerCommand::Close);
        }
        if let Some(back) = back {
            let label = format!(
                "Back to {}",
                truncate_end(
                    &sanitize_display(&back.label, 80),
                    if layout.compact { 14 } else { 24 }
                )
            );
            let action = theme::ghost_action(
                ui,
                picker_control_id("back"),
                &label,
                false,
                layout.button_height,
                theme::BG_CLEAR_SOLID,
            );
            action.response.on_hover_text(format!(
                "Return to {} without stopping its shell",
                sanitize_display(&back.path.display().to_string(), 900)
            ));
            if action.activated {
                *command = Some(PickerCommand::Back);
            }
        } else {
            ui.label(theme::micro("Choose a workspace"));
        }
    });
}

fn picker_toolbar(
    ui: &mut Ui,
    layout: &Layout,
    current: &std::path::Path,
    command: &mut Option<PickerCommand>,
) {
    let height = if layout.compact { 36.0 } else { 40.0 };
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let mut toolbar = ui.new_child(
        UiBuilder::new()
            .max_rect(
                rect.shrink2(egui::vec2(layout.header_padding_x, 0.0))
                    .shrink(1.0),
            )
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    let up = theme::ghost_action(
        &mut toolbar,
        picker_control_id("up"),
        "Up",
        false,
        layout.button_height,
        theme::BG_CARD,
    );
    up.response.on_hover_text("Go to the parent folder");
    if up.activated {
        *command = Some(PickerCommand::Up);
    }
    let refresh = theme::ghost_action(
        &mut toolbar,
        picker_control_id("refresh"),
        "Refresh",
        false,
        layout.button_height,
        theme::BG_CARD,
    );
    refresh.response.on_hover_text("Re-read the current folder");
    if refresh.activated {
        *command = Some(PickerCommand::Refresh);
    }
    let (divider, _) = toolbar.allocate_exact_size(egui::vec2(1.0, 14.0), Sense::hover());
    toolbar.painter().rect_filled(divider, 0.0, theme::BORDER);
    let full = current.display().to_string();
    let available = toolbar.available_width().max(0.0);
    if available > 24.0 {
        let max_text = (available - 12.0).max(1.0);
        let char_budget = (max_text / 6.3).floor().max(4.0) as usize;
        let display = truncate_path(&sanitize_display(&full, 900), char_budget);
        let galley = toolbar.painter().layout_job(theme::ellipsized_job(
            display,
            FontId::monospace(10.5),
            theme::TEXT_PRIMARY,
            max_text,
        ));
        let chip_width = (galley.size().x + 12.0).min(available).max(1.0);
        let (chip_rect, _) =
            toolbar.allocate_exact_size(egui::vec2(chip_width, 20.0), Sense::hover());
        let chip = toolbar.interact(chip_rect, picker_path_id(), Sense::hover());
        toolbar
            .painter()
            .rect_filled(chip_rect, CornerRadius::same(4), theme::BG_SELECTED);
        toolbar.painter().galley(
            egui::pos2(
                chip_rect.left() + 6.0,
                chip_rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            theme::TEXT_PRIMARY,
        );
        chip.on_hover_text(sanitize_display(&full, 900));
    }
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.bottom() - 0.5),
            egui::pos2(rect.right(), rect.bottom() - 0.5),
        ],
        Stroke::new(1.0, theme::BORDER_HEADER),
    );
}

fn picker_list(
    ui: &mut Ui,
    _layout: &Layout,
    rows: &[PickerDisplayRow],
    selection: Option<usize>,
    list_max: f32,
    (instance, revision): (u64, u64),
    command: &mut Option<PickerCommand>,
) {
    const ROW_HEIGHT: f32 = 32.0;
    let scroll_id = egui::Id::unique(("made-picker-last-selection", instance));
    let previous = ui
        .ctx()
        .data(|data| data.get_temp::<(Option<usize>, u64)>(scroll_id));
    let scroll_to_selection = previous != Some((selection, revision));
    if scroll_to_selection {
        ui.ctx()
            .data_mut(|data| data.insert_temp(scroll_id, (selection, revision)));
    }
    let content_height = rows.len() as f32 * ROW_HEIGHT;
    let list_height = if rows.is_empty() {
        64.0
    } else {
        content_height.min(list_max).max(ROW_HEIGHT)
    };
    let (list_rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), list_height),
        Sense::hover(),
    );
    let list_focus = picker_list_id();
    ui.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            list_focus,
            egui::EventFilter {
                tab: false,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: false,
            },
        );
    });
    let list_response = ui.interact(list_rect, list_focus, Sense::click());
    if list_response.clicked() {
        list_response.request_focus();
    }
    let mut list_ui = ui.new_child(UiBuilder::new().max_rect(list_rect));
    let visibility = if content_height > list_height + 0.5 {
        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded
    } else {
        egui::scroll_area::ScrollBarVisibility::AlwaysHidden
    };
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt(("made-picker-list", instance, revision))
        .auto_shrink([false, false])
        .scroll_bar_visibility(visibility)
        .max_height(list_height);
    if scroll_to_selection {
        if let Some(selected) = selection {
            // Reveal minimally with a bottom-edge window. This shows the
            // selected row and as many immediately preceding ancestor rows as
            // fit, instead of centering and pushing the root ancestor away.
            let rows_per_view = ((list_height / ROW_HEIGHT).floor() as usize).max(1);
            let first_visible = selected.saturating_sub(rows_per_view - 1);
            let max_offset = (content_height - list_height).max(0.0);
            let target = (first_visible as f32 * ROW_HEIGHT).min(max_offset);
            scroll = scroll.animated(false).vertical_scroll_offset(target);
        }
    }
    list_ui.spacing_mut().item_spacing.y = 0.0;
    scroll.show_rows(&mut list_ui, ROW_HEIGHT, rows.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for index in range {
            let Some(row) = rows.get(index) else {
                continue;
            };
            let (rect, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
            let row_id = egui::Id::unique(("made-picker-row", row.index));
            let response = ui.interact(rect, row_id, Sense::CLICK);
            let row_rect = rect.shrink2(egui::vec2(4.0, 1.0));
            let selected = selection == Some(row.index);
            if selected {
                ui.painter()
                    .rect_filled(row_rect, CornerRadius::same(6), theme::BG_SELECTED);
            } else if response.hovered() {
                ui.painter()
                    .rect_filled(row_rect, CornerRadius::same(6), theme::BG_HOVER);
            }
            let indent = 4.0 + row.depth as f32 * 14.0;
            let marker_color = if selected {
                theme::TEXT_PRIMARY
            } else {
                theme::TEXT_TERTIARY
            };
            theme::paint_disclosure(
                ui.painter(),
                egui::pos2(rect.left() + indent + 8.0, rect.center().y),
                row.expanded,
                marker_color,
            );
            let text_left = rect.left() + 24.0 + row.depth as f32 * 14.0;
            theme::paint_ellipsized_centered(
                ui.painter(),
                text_left,
                rect.center().y,
                &row.name,
                FontId::proportional(12.5),
                if selected {
                    theme::TEXT_PRIMARY
                } else {
                    theme::TEXT_SECONDARY
                },
                (rect.right() - text_left - 6.0).max(20.0),
            );
            let response =
                response.on_hover_text(sanitize_display(&row.path.display().to_string(), 900));
            if response.double_clicked() {
                // The first click already selected/toggled the row.
                // Ignore the second click so a double-click toggles once.
            } else if response.clicked() {
                ui.memory_mut(|memory| memory.request_focus(list_focus));
                *command = Some(PickerCommand::Toggle(row.index));
            }
        }
    });
    if rows.is_empty() {
        let mut empty = ui.new_child(
            UiBuilder::new()
                .max_rect(list_rect)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        empty.add_space(8.0);
        empty.label(theme::body("No child folders here."));
    }
}

fn picker_truncated_notice(ui: &mut Ui, layout: &Layout, truncated: usize) {
    let height = 20.0;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::WARNING_TINT);
    ui.painter().rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
        CornerRadius::same(2),
        theme::WARNING,
    );
    let text = format!(
        "Showing the first {} folders; {} more are not listed.",
        crate::workspace_picker::MAX_PICKER_ENTRIES,
        truncated
    );
    let full = format!(
        "This folder contains more than {} child folders. Use Up/Refresh or open it directly to narrow the list.",
        crate::workspace_picker::MAX_PICKER_ENTRIES
    );
    theme::paint_ellipsized(
        ui.painter(),
        egui::pos2(rect.left() + layout.header_padding_x, rect.center().y - 7.0),
        text,
        FontId::proportional(10.5),
        theme::WARNING,
        (rect.width() - layout.header_padding_x * 2.0).max(20.0),
    );
    response.on_hover_text(full);
}

fn picker_footer(
    ui: &mut Ui,
    layout: &Layout,
    selection: Option<usize>,
    target: &std::path::Path,
    command: &mut Option<PickerCommand>,
) {
    let height = if layout.compact { 48.0 } else { 52.0 };
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let mut footer = ui.new_child(
        UiBuilder::new()
            .max_rect(
                rect.shrink2(egui::vec2(layout.header_padding_x, 0.0))
                    .shrink(1.0),
            )
            .layout(egui::Layout::right_to_left(Align::Center)),
    );
    let primary = theme::primary_action(
        &mut footer,
        picker_control_id("current"),
        "Use current folder",
        layout.primary_height,
        true,
        theme::BG_CARD,
    );
    primary.response.on_hover_text(format!(
        "Open this folder: {}",
        sanitize_display(&target.display().to_string(), 900)
    ));
    if primary.activated {
        *command = Some(PickerCommand::ConfirmCurrent);
    }
    let enabled = selection.is_some();
    let secondary = theme::quiet_secondary_action(
        &mut footer,
        picker_control_id("selected"),
        "Open selected folder",
        layout.primary_height,
        enabled,
        theme::BG_CARD,
    );
    secondary.response.on_hover_text(if enabled {
        "Open the selected tree row in a new workspace tab"
    } else {
        "Select a folder to open it in a new tab"
    });
    if secondary.activated {
        if let Some(index) = selection {
            *command = Some(PickerCommand::ConfirmRow(index));
        }
    }
}

/// Label on the picker's only accent-filled control. Used by the headless test.
pub fn picker_primary_label() -> &'static str {
    "Use current folder"
}

enum PickerCommand {
    Up,
    Refresh,
    Toggle(usize),
    ConfirmCurrent,
    ConfirmRow(usize),
    Back,
    Close,
}

// ── Starting surface ────────────────────────────────────────────────────────

/// Shown for the one frame between a confirmed selection and runtime startup.
pub fn starting_ui(model: &AppModel, ui: &mut Ui) {
    let layout = Layout::for_window(ui.available_width(), ui.available_height());
    egui::Frame::NONE
        .inner_margin(egui::Margin::same(layout.gutter as i8))
        .show(ui, |ui| {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.add(egui::Spinner::new().size(16.0).color(theme::ACCENT));
                ui.label(theme::body("Starting the workspace…"));
                if let Some(error) = model.launch().error() {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(sanitize_display(&error.message(), 400))
                            .font(FontId::proportional(theme::BODY_SIZE))
                            .color(theme::ERROR),
                    );
                }
            });
        });
}

// ── Workspace strip and stable widget ids ──────────────────────────────────

/// Stable id for one workspace tab.
pub fn workspace_tab_id(id: WorkspaceId) -> egui::Id {
    egui::Id::unique(("made-workspace-tab", id.get()))
}

/// Stable id for one workspace tab's close affordance.
pub fn workspace_tab_close_id(id: WorkspaceId) -> egui::Id {
    egui::Id::unique(("made-workspace-close", id.get()))
}

fn workspace_tab_close_reveal_id(id: WorkspaceId) -> egui::Id {
    workspace_tab_id(id).with("close-reveal")
}

/// Stable id for a global top-bar control.
pub fn top_action_id(action: &str) -> egui::Id {
    egui::Id::unique(("made-top-action", action))
}

/// Stable id for one item in the `+` new-tab menu.
pub fn new_tab_menu_item_id(action: &str) -> egui::Id {
    egui::Id::unique(("made-new-tab-menu-item", action))
}

/// Stable id for a top-bar control whose state belongs to one workspace.
pub fn top_workspace_action_id(id: WorkspaceId, action: &str) -> egui::Id {
    egui::Id::unique(("made-top-workspace-action", action, id.get()))
}

/// Stable id for a workspace-scoped left-surface toggle.
pub fn left_surface_action_id(pane_id: &str, surface: &str) -> egui::Id {
    egui::Id::unique(("made-left-surface", pane_id, surface))
}

/// Stable id for the workspace explorer splitter.
pub fn explorer_splitter_id(pane_id: &str) -> egui::Id {
    egui::Id::unique(("made-explorer-splitter", pane_id))
}

/// Horizontal inset before the first tab glyph or label.
const WORKSPACE_TAB_INSET: f32 = 10.0;
/// Space kept from a label's right edge to the tab's right edge. This covers
/// the reserved 24px close target plus its optical gap.
const WORKSPACE_TAB_CLOSE_SLOT: f32 = 26.0;
/// 16px icon plus a 6px gap before the label.
const WORKSPACE_TAB_ICON_SIZE: f32 = 16.0;
const WORKSPACE_TAB_ICON_GAP: f32 = 6.0;
const WORKSPACE_TAB_ICON_SLOT: f32 = WORKSPACE_TAB_ICON_SIZE + WORKSPACE_TAB_ICON_GAP;
/// Extra right-edge allowance retained in the tab-width and label budget so the
/// approved geometry stays unchanged as indicators appear.
const WORKSPACE_TAB_RIGHT_EDGE_ALLOWANCE: f32 = 14.0;
/// Fixed text growth kept beyond the old layout after the icon slot and
/// right-edge allowance.
const WORKSPACE_TAB_TEXT_GROWTH: f32 = 12.0;
/// Corner inset for the right-aligned session status dots.
const WORKSPACE_TAB_STATUS_CORNER_INSET: f32 = 10.0;
/// Session status dot radius for a 4px diameter.
const WORKSPACE_TAB_STATUS_DOT_RADIUS: f32 = 2.0;
/// Gap between session status dots.
const WORKSPACE_TAB_STATUS_DOT_GAP: f32 = 3.0;
/// Gap kept between the dot row and the label.
const WORKSPACE_TAB_STATUS_LABEL_GAP: f32 = 8.0;
/// Maximum number of session status dots shown on one tab.
const WORKSPACE_TAB_MAX_SESSION_DOTS: usize = 8;
/// Existing right padding: close target plus its retained allowance.
const WORKSPACE_TAB_RIGHT_PADDING: f32 =
    WORKSPACE_TAB_CLOSE_SLOT + WORKSPACE_TAB_RIGHT_EDGE_ALLOWANCE;
/// Gentle busy pulse: 1.8s period and a 0.35 minimum opacity.
const WORKSPACE_TAB_BUSY_PULSE_PERIOD: f64 = 1.8;
const WORKSPACE_TAB_BUSY_PULSE_MIN: f32 = 0.35;
/// Crossfade time between session dots and the close glyph.
const WORKSPACE_TAB_CLOSE_FADE_TIME: f32 = 0.15;

fn fade_color(color: Color32, opacity: f32) -> Color32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(
        r,
        g,
        b,
        (a as f32 * opacity.clamp(0.0, 1.0))
            .round()
            .clamp(0.0, 255.0) as u8,
    )
}

/// Busy-dot opacity as an 8-bit alpha value. One full sine period is
/// [`WORKSPACE_TAB_BUSY_PULSE_PERIOD`], with no movement or size change.
fn workspace_busy_status_alpha(time: f64) -> u8 {
    let phase = (time * std::f64::consts::TAU / WORKSPACE_TAB_BUSY_PULSE_PERIOD).sin();
    let opacity = WORKSPACE_TAB_BUSY_PULSE_MIN
        + (1.0 - WORKSPACE_TAB_BUSY_PULSE_MIN) * (phase as f32 * 0.5 + 0.5);
    (opacity * 255.0).round().clamp(0.0, 255.0) as u8
}

fn workspace_tab_visible_session_count(sessions: &[SessionActivity]) -> usize {
    sessions.len().min(WORKSPACE_TAB_MAX_SESSION_DOTS)
}

fn workspace_tab_dot_row_width(session_count: usize) -> f32 {
    let shown = session_count.min(WORKSPACE_TAB_MAX_SESSION_DOTS);
    if shown == 0 {
        0.0
    } else {
        shown as f32 * WORKSPACE_TAB_STATUS_DOT_RADIUS * 2.0
            + (shown - 1) as f32 * WORKSPACE_TAB_STATUS_DOT_GAP
    }
}

/// Extra right-side room needed once the session dots outgrow the existing
/// close-target padding. Zero up to three dots.
fn workspace_tab_dot_excess(session_count: usize) -> f32 {
    let row_width = workspace_tab_dot_row_width(session_count);
    if row_width == 0.0 {
        return 0.0;
    }
    (WORKSPACE_TAB_STATUS_CORNER_INSET + row_width + WORKSPACE_TAB_STATUS_LABEL_GAP
        - WORKSPACE_TAB_RIGHT_PADDING)
        .max(0.0)
}

fn workspace_tab_base_width(label: &str, layout: &Layout) -> f32 {
    let chars = label.chars().count() as f32;
    (chars * 6.2
        + 38.0
        + WORKSPACE_TAB_ICON_SLOT
        + WORKSPACE_TAB_RIGHT_EDGE_ALLOWANCE
        + WORKSPACE_TAB_TEXT_GROWTH)
        .clamp(
            if layout.compact { 84.0 } else { 96.0 },
            if layout.compact { 198.0 } else { 238.0 },
        )
}

fn workspace_tab_width(label: &str, layout: &Layout, session_count: usize) -> f32 {
    workspace_tab_base_width(label, layout) + workspace_tab_dot_excess(session_count)
}

/// Text width left after the close slot, right-edge allowance, icon slot, and
/// any extra room reserved for the session dot row. The label area itself
/// stays unchanged as dots appear, so the row never overlaps it.
fn workspace_tab_label_width(width: f32, session_count: usize) -> f32 {
    let base_width = width - workspace_tab_dot_excess(session_count);
    (base_width
        - WORKSPACE_TAB_INSET
        - WORKSPACE_TAB_CLOSE_SLOT
        - WORKSPACE_TAB_RIGHT_EDGE_ALLOWANCE
        - WORKSPACE_TAB_ICON_SLOT)
        .max(8.0)
}

/// Whether OpenCode is active in a workspace tab.
///
/// A connected bridge covers both a directly launched TUI and a shell where
/// the user ran `opencode` manually. A directly launched TUI also counts while
/// its process is alive, even before the bridge handshakes.
pub(crate) fn workspace_opencode_active(
    target: TerminalTarget,
    connected: bool,
    running: bool,
) -> bool {
    connected || (target == TerminalTarget::Tui(OPENCODE) && running)
}

/// Whether a non-OpenCode tab shows the Grok Build label and mark.
///
/// A directly launched Grok TUI counts while its process is alive, even
/// before a session exists. A shell tab counts while that terminal still has
/// open Grok sessions. A remote tab counts the same way from its own server
/// state: a directly launched remote grok (`TerminalTarget::RemoteGrok`) or
/// any owned open Grok session in the tab's projection. An active OpenCode
/// tab never does: its label, icon, and dots stay OpenCode's.
pub(crate) fn workspace_grok_branded(
    opencode: bool,
    target: TerminalTarget,
    running: bool,
    open_grok_sessions: usize,
) -> bool {
    !opencode
        && (open_grok_sessions > 0
            || ((target == TerminalTarget::Tui(GROK)
                || matches!(target, TerminalTarget::RemoteGrok { .. }))
                && running))
}

/// Claude Code brands a tab while it is present, unless OpenCode owns the tab
/// or a remote tab already presents Grok Build under SSH-10.
fn workspace_claude_branded(opencode: bool, remote_grok_branded: bool, present: bool) -> bool {
    !opencode && !remote_grok_branded && present
}

/// Non-color summary across all open sessions. Counts include sessions beyond
/// the eight-dot visual cap.
fn opencode_sessions_summary(sessions: &[SessionActivity]) -> Option<String> {
    if sessions.is_empty() {
        return None;
    }
    let busy = sessions
        .iter()
        .filter(|activity| **activity == SessionActivity::Busy)
        .count();
    let attention = sessions
        .iter()
        .filter(|activity| **activity == SessionActivity::Attention)
        .count();
    let idle = sessions
        .iter()
        .filter(|activity| **activity == SessionActivity::Idle)
        .count();
    let mut parts = Vec::new();
    if busy > 0 {
        parts.push(format!("{busy} running"));
    }
    if attention > 0 {
        parts.push(if attention == 1 {
            "1 needs attention".to_owned()
        } else {
            format!("{attention} need attention")
        });
    }
    if idle > 0 {
        parts.push(format!("{idle} idle"));
    }
    Some(format!("Sessions: {}", parts.join(", ")))
}

/// Official Grok mark from `assets/grok.svg`, outside the Material icon inventory.
fn grok_mark() -> theme::MenuLeading {
    mark_leading(GROK.descriptor().mark)
}

/// Official Claude mark from `assets/claude-code.svg`, outside the Material
/// icon inventory. The Material `claude.svg` file icon is not this mark.
fn claude_mark() -> theme::MenuLeading {
    mark_leading(CLAUDE_CODE.descriptor().mark)
}

/// The vendored icon of a mark name, resolved once per name.
fn vendored_mark(name: &'static str) -> crate::explorer_icons::IconId {
    static RESOLVED: Mutex<Vec<(&'static str, crate::explorer_icons::IconId)>> =
        Mutex::new(Vec::new());
    let mut resolved = RESOLVED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, icon)) = resolved.iter().find(|(known, _)| *known == name) {
        return *icon;
    }
    let icon = crate::explorer_icons::icon_by_file_name(name)
        .expect("a vendored mark is part of the explorer icons");
    resolved.push((name, icon));
    icon
}

/// The menu leading image of an integration's declared mark.
fn mark_leading(mark: Mark) -> theme::MenuLeading {
    match mark {
        Mark::Vendored(name) => {
            let icon = vendored_mark(name);
            theme::MenuLeading::Image {
                uri: icon.uri(),
                bytes: icon.bytes(),
            }
        }
        Mark::Image { uri, bytes } => theme::MenuLeading::Image { uri, bytes },
    }
}

/// Accessible tab name, including the non-color session summary when present.
fn workspace_tab_accessible_label(label: &str, summary: Option<&str>) -> String {
    let label = sanitize_display(label, 80);
    match summary {
        Some(summary) => format!("{label} — {summary}"),
        None => label,
    }
}

/// Choose bounded visible tab indices while keeping the active tab visible.
///
/// `trailing_width` reserves a mandatory control immediately after the last
/// visible tab (the header's Open folder `+`). When the tabs still overflow,
/// the overflow trigger's width is reserved after it as well.
fn bounded_tab_indices(
    widths: &[f32],
    active: Option<usize>,
    available: f32,
    gap: f32,
    trailing_width: Option<f32>,
) -> (Vec<usize>, bool) {
    if widths.is_empty() {
        return (Vec::new(), false);
    }
    let total: f32 = widths.iter().sum::<f32>() + gap * (widths.len().saturating_sub(1) as f32);
    let trailing = trailing_width.unwrap_or(0.0);
    let trailing_gap = if trailing_width.is_some() { gap } else { 0.0 };
    if total + trailing_gap + trailing <= available {
        return ((0..widths.len()).collect(), false);
    }
    let overflow_width = 28.0 + gap;
    let budget = (available - trailing_gap - trailing - overflow_width).max(0.0);
    let active = active.filter(|index| *index < widths.len());
    let mut visible = Vec::new();
    let mut used = 0.0;
    // Reserve the active tab's real width before filling the remaining budget.
    // The previous heuristic could swap in a wide active tab after truncation
    // and overlap the fixed workspace controls.
    if let Some(active) = active {
        visible.push(active);
        used = widths[active];
    }
    for (index, width) in widths.iter().enumerate() {
        if Some(index) == active {
            continue;
        }
        let extra = if visible.is_empty() {
            *width
        } else {
            *width + gap
        };
        if used + extra <= budget {
            visible.push(index);
            used += extra;
        }
    }
    if visible.is_empty() {
        visible.push(active.unwrap_or(0));
    }
    visible.sort_unstable();
    (visible, true)
}

struct WorkspaceTabInfo {
    id: WorkspaceId,
    label: String,
    path: std::path::PathBuf,
    remote_path: Option<String>,
    /// The integration that brands the tab: its label, mark and dots.
    brand: Option<TabBrand>,
    sessions: Vec<SessionActivity>,
}

/// The integration that brands a workspace tab.
struct TabBrand {
    entry: IntegrationRef,
    /// Sanitized tooltip title, for an integration whose tooltip names its
    /// session.
    title: Option<String>,
}

/// The first integration of the tab's list, in rank order, whose presentation
/// brands the tab, with that presentation.
fn tab_brand(workbench: &Workbench) -> Option<(IntegrationRef, Presentation)> {
    let list = workbench.integrations();
    let generic = rank_order(list).into_iter().find_map(|index| {
        workbench
            .presentation_of(index)
            .map(|presentation| (list[index], presentation))
    });
    if workbench.is_remote() {
        // BEGIN remote-path (AIA-02)
        return remote_tab_brand(workbench, list, generic);
        // END remote-path
    }
    generic
}

// BEGIN remote-path (AIA-02)
/// The branding of a remote tab. OpenCode and Grok Build present from the
/// tab's own server state; an integration that presents from its tab state
/// yields to OpenCode and to a remote tab that already presents Grok Build
/// under SSH-10.
fn remote_tab_brand(
    workbench: &Workbench,
    list: IntegrationList,
    generic: Option<(IntegrationRef, Presentation)>,
) -> Option<(IntegrationRef, Presentation)> {
    let opencode = list.contains(&OPENCODE) && workbench.remote_opencode_active();
    let grok_dots = workbench.remote_grok_activities();
    let remote_grok = list.contains(&GROK)
        && workspace_grok_branded(
            opencode,
            workbench.terminal_target(),
            workbench.remote_grok_running(),
            grok_dots.len(),
        );
    let generic = generic.filter(|_| workspace_claude_branded(opencode, remote_grok, true));
    if opencode {
        let dots = workbench
            .session_map()
            .rows()
            .iter()
            .map(SessionRow::activity)
            .collect();
        Some((OPENCODE, Presentation { dots, title: None }))
    } else if generic.is_some() {
        generic
    } else if remote_grok {
        Some((
            GROK,
            Presentation {
                dots: grok_dots,
                title: None,
            },
        ))
    } else {
        None
    }
}
// END remote-path

/// The tab of one workspace.
fn workspace_tab_info(id: WorkspaceId, workbench: &Workbench) -> WorkspaceTabInfo {
    let path = workbench.shell_cwd().to_path_buf();
    let remote_path = workbench.remote_path_label();
    let branded = tab_brand(workbench);
    // A branded tab shows its integration's label; only an unbranded remote
    // tab falls back to its server name so SSH identity stays inspectable.
    let label = if let Some((entry, _)) = &branded {
        entry.descriptor().label.to_owned()
    } else if let Some(name) = workbench.remote_label() {
        name
    } else {
        workbench
            .launch_plan()
            .map(|plan| plan.workspace.display_name().to_owned())
            .unwrap_or_else(|| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Workspace".to_owned())
            })
    };
    // Session dots follow the brand, so stale rows cannot paint status after a
    // disconnect or fatal protocol failure.
    let (brand, sessions) = match branded {
        Some((entry, presentation)) => (
            Some(TabBrand {
                entry,
                title: presentation.title,
            }),
            presentation.dots,
        ),
        None => (None, Vec::new()),
    };
    WorkspaceTabInfo {
        id,
        label,
        path,
        remote_path,
        brand,
        sessions,
    }
}

/// The tooltip of a workspace tab: a branded tab names its session and the
/// sessions' summary and never the path; an unbranded tab with sessions shows
/// the summary, and one without shows its path.
fn workspace_tab_tooltip(tab: &WorkspaceTabInfo, summary: Option<&str>) -> Option<String> {
    match &tab.brand {
        Some(TabBrand {
            title: Some(title), ..
        }) => Some(match summary {
            Some(summary) => format!("{title} — {summary}"),
            None => title.clone(),
        }),
        Some(_) => summary.map(str::to_owned),
        None if !tab.sessions.is_empty() => summary.map(str::to_owned),
        None => Some(match &tab.remote_path {
            Some(path) => sanitize_display(path, 900),
            None => sanitize_display(&tab.path.display().to_string(), 900),
        }),
    }
}

/// What a workspace tab shows, for tests of the presentation seam.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabSummary {
    /// The workspace.
    pub id: WorkspaceId,
    /// Tab label.
    pub label: String,
    /// The mark painted in the tab, or `None` for the terminal glyph.
    pub mark: Option<Mark>,
    /// Session status dots.
    pub dots: Vec<SessionActivity>,
    /// Tooltip, or `None` when the tab has none.
    pub tooltip: Option<String>,
    /// Accessible name of the tab.
    pub accessible_name: String,
}

/// The tabs of the open workspaces as they paint, in strip order.
#[doc(hidden)]
pub fn workspace_tab_summaries(model: &AppModel) -> Vec<TabSummary> {
    model
        .workspace_ids()
        .filter_map(|id| {
            let tab = workspace_tab_info(id, model.workspace(id)?);
            let summary = opencode_sessions_summary(&tab.sessions);
            Some(TabSummary {
                id,
                mark: tab
                    .brand
                    .as_ref()
                    .map(|brand| brand.entry.descriptor().mark),
                tooltip: workspace_tab_tooltip(&tab, summary.as_deref()),
                accessible_name: workspace_tab_accessible_label(&tab.label, summary.as_deref()),
                dots: tab.sessions,
                label: tab.label,
            })
        })
        .collect()
}

/// The entries of the new-tab menu below Terminal: the integrations of the
/// model's list that PATH detection found, in list order.
#[doc(hidden)]
pub fn new_tab_menu_entries(model: &AppModel) -> Vec<IntegrationRef> {
    model
        .integrations()
        .iter()
        .copied()
        .filter(|entry| model.compatible_tuis().contains(entry))
        .collect()
}

/// The integration rows of the About card: the pill and the sentence of each
/// row of the snapshot, in list order.
#[doc(hidden)]
pub fn about_integration_rows(snapshot: &DiagnosticsSnapshot) -> Vec<(DiagnosticsPill, &str)> {
    snapshot
        .rows
        .iter()
        .map(|row| (row.pill, row.sentence.as_str()))
        .collect()
}

fn authority_key(id: made_remote::protocol::AuthorityId) -> String {
    id.0.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Stable id for the SSH servers window.
pub fn ssh_servers_window_id() -> egui::Id {
    egui::Id::unique("made-ssh-servers")
}

/// Stable id for one field in the SSH servers window.
pub fn ssh_field_id(field: &str) -> egui::Id {
    egui::Id::unique(("made-ssh-field", field))
}

fn ssh_field_hold_id() -> egui::Id {
    egui::Id::unique("made-ssh-field-hold")
}

fn ssh_window_rect_id() -> egui::Id {
    egui::Id::unique("made-ssh-window-rect")
}

fn ssh_window_was_open_id() -> egui::Id {
    egui::Id::unique("made-ssh-window-was-open")
}

fn ssh_window_layer_id() -> egui::Id {
    egui::Id::unique("made-ssh-window-layer")
}

fn ssh_pending_row_focus_id() -> egui::Id {
    egui::Id::unique("made-ssh-row-focus-next")
}

fn ssh_body_height_id() -> egui::Id {
    egui::Id::unique("made-ssh-window-body-height")
}

/// Stable id for one saved SSH server row.
pub fn ssh_server_row_id(id: made_remote::protocol::AuthorityId) -> egui::Id {
    egui::Id::unique(("made-ssh-server-row", authority_key(id)))
}

/// One bounded vertical scroll area in the shared floating-overlay style.
/// `fill_width` keeps the SSH window width fixed; menus shrink to their rows.
fn bounded_scroll_area(
    id_salt: impl Hash + std::fmt::Debug,
    max_height: f32,
    fill_width: bool,
    reset_scroll: bool,
) -> egui::ScrollArea {
    let scroll = egui::ScrollArea::vertical()
        .id_salt(id_salt)
        .max_height(max_height)
        .auto_shrink([!fill_width, true])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded);
    if reset_scroll {
        // The salt is stable across open/close so deliberate scrolling survives
        // within one open session. A fresh open starts at the top instead of
        // inheriting the last session's offset.
        scroll.vertical_scroll_offset(0.0)
    } else {
        scroll
    }
}

fn ssh_field_painted_pass_id(field: egui::Id) -> egui::Id {
    egui::Id::unique(("made-ssh-field-painted-pass", field))
}

fn mark_ssh_field_painted(ctx: &egui::Context, field: egui::Id) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| data.insert_temp(ssh_field_painted_pass_id(field), pass));
}

fn terminal_hit_rect_id() -> egui::Id {
    egui::Id::unique("made-terminal-hit-rect")
}

fn held_ssh_field(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|data| data.get_temp::<egui::Id>(ssh_field_hold_id()))
}

fn ssh_field_held(ctx: &egui::Context) -> bool {
    held_ssh_field(ctx).is_some()
}

fn hold_ssh_field(ctx: &egui::Context, id: egui::Id) {
    ctx.data_mut(|data| data.insert_temp(ssh_field_hold_id(), id));
}

/// Drop only the field hold. Tab/click into another control inside the window
/// and returning to the terminal both leave the window's rect, layer, and
/// measured body height intact.
fn release_ssh_field_hold(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<egui::Id>(ssh_field_hold_id());
    });
}

/// Full reset, only when the window actually closes or disappears.
fn reset_ssh_window_state(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<egui::Id>(ssh_field_hold_id());
        data.remove::<egui::Rect>(ssh_window_rect_id());
        data.remove::<egui::LayerId>(ssh_window_layer_id());
        data.remove::<egui::Id>(ssh_pending_row_focus_id());
        data.remove::<f32>(ssh_body_height_id());
    });
}

/// The terminal paints after this window and treats a click in its rect as its
/// own, including clicks on the centered window. Restore the field after that.
struct SshFieldFocusRestore;

impl egui::plugin::Plugin for SshFieldFocusRestore {
    fn debug_name(&self) -> &'static str {
        "ssh-field-focus"
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        apply_pending_ssh_row_focus(ui);
        restore_ssh_field_focus(ui);
    }
}

/// Tab from the action row enters the inventory after every widget in the
/// frame has registered, so the next frame's workbench widgets cannot take the
/// focus first.
fn apply_pending_ssh_row_focus(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let pending = ctx.data(|data| data.get_temp::<egui::Id>(ssh_pending_row_focus_id()));
    let Some(pending) = pending else {
        return;
    };
    ctx.data_mut(|data| data.remove::<egui::Id>(ssh_pending_row_focus_id()));
    if ctx.memory(|memory| memory.focused()) == Some(pending) {
        return;
    }
    ctx.memory_mut(|memory| memory.request_focus(pending));
}

fn restore_ssh_field_focus(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let Some(field) = ctx.data(|data| data.get_temp::<egui::Id>(ssh_field_hold_id())) else {
        return;
    };
    // Prompts and fields disappear without a per-call-site cleanup (answered,
    // cancelled, transport failure, tab close). `read_response` also exposes
    // the previous pass, so require this field's paint marker from this exact
    // pass before reclaiming focus.
    let painted = ctx.data(|data| data.get_temp::<u64>(ssh_field_painted_pass_id(field)));
    if painted != Some(ctx.cumulative_pass_nr()) {
        release_ssh_field_hold(&ctx);
        return;
    }
    // Tab/arrow focus that landed on another widget in this window is a
    // deliberate move: release the field hold instead of pulling focus back.
    let focused = ui.memory(|memory| memory.focused());
    if let Some(focused) = focused.filter(|focused| *focused != field) {
        let layer = ctx.data(|data| data.get_temp::<egui::LayerId>(ssh_window_layer_id()));
        let in_window = layer.is_some_and(|layer| {
            ctx.viewport(|viewport| {
                viewport
                    .this_pass
                    .widgets
                    .get(focused)
                    .is_some_and(|widget| widget.layer_id == layer && widget.sense.is_focusable())
            })
        });
        if in_window {
            release_ssh_field_hold(&ctx);
            return;
        }
    }
    let window = ctx.data(|data| data.get_temp::<egui::Rect>(ssh_window_rect_id()));
    let terminal = ctx.data(|data| data.get_temp::<egui::Rect>(terminal_hit_rect_id()));
    let returned_to_terminal = ctx.input(|input| {
        input.pointer.any_pressed()
            && input.pointer.interact_pos().is_some_and(|pos| {
                window.is_some_and(|window| !window.contains(pos))
                    && terminal.is_some_and(|terminal| terminal.contains(pos))
            })
    });
    if returned_to_terminal {
        release_ssh_field_hold(&ctx);
        return;
    }
    ui.memory_mut(|memory| memory.request_focus(field));
}

fn ssh_text_edit(ui: &mut Ui, field: &str, text: &mut String, password: bool) {
    let id = ssh_field_id(field);
    let mut edit = egui::TextEdit::singleline(text)
        .id(id)
        .desired_width(f32::INFINITY);
    if password {
        edit = edit.password(true);
    }
    let response = ui.add(edit);
    mark_ssh_field_painted(ui.ctx(), id);
    if response.clicked() {
        response.request_focus();
    }
    if response.clicked() || response.has_focus() {
        hold_ssh_field(ui.ctx(), id);
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: false,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: false,
                },
            );
        });
    }
}

/// Label sits 4px above its field. The window's `layout.gap` separates pairs.
fn ssh_field(ui: &mut Ui, label: &str, field: &str, text: &mut String) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(theme::micro(label));
        ssh_text_edit(ui, field, text, false);
    });
}

fn ssh_user_port_row(ui: &mut Ui, layout: &Layout, user: &mut String, port: &mut String) {
    let gap = layout.gap;
    let port_width = 96.0;
    let width = ui.available_width();
    ui.horizontal(|ui| {
        ui.set_min_width(width);
        ui.spacing_mut().item_spacing.x = gap;
        let user_width = (width - gap - port_width).max(80.0);
        ui.vertical(|ui| {
            ui.set_width(user_width);
            ssh_field(ui, "User", "user", user);
        });
        ui.vertical(|ui| {
            ui.set_width(port_width);
            ssh_field(ui, "Port", "port", port);
        });
    });
}

/// One saved server row with a stable id for focus and keyboard navigation.
fn ssh_server_row(
    ui: &mut Ui,
    id: made_remote::protocol::AuthorityId,
    selected: bool,
    label: &str,
    accessible_label: &str,
    height: f32,
) -> egui::Response {
    let font = FontId::proportional(theme::BUTTON_SIZE);
    let padding = ui.spacing().button_padding.x;
    let full_width = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), theme::TEXT_SECONDARY)
        .size()
        .x;
    let available = ui.available_width().max(1.0);
    let width = (full_width + 2.0 * padding).min(available);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(width, height.max(24.0)), egui::Sense::hover());
    let response = ui.interact(rect, ssh_server_row_id(id), egui::Sense::click());
    if response.clicked() {
        response.request_focus();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, true, selected, accessible_label)
    });
    let fill = if selected {
        theme::BG_SELECTED
    } else if response.hovered() {
        theme::BG_HOVER
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    let color = if selected || response.hovered() {
        theme::TEXT_PRIMARY
    } else {
        theme::TEXT_SECONDARY
    };
    theme::paint_ellipsized_centered(
        ui.painter(),
        rect.left() + 10.0,
        rect.center().y,
        label,
        font,
        color,
        (rect.width() - 20.0).max(1.0),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.has_focus() {
        theme::paint_focus_ring(ui, rect, egui::CornerRadius::same(6), theme::BG_CARD);
        response.scroll_to_me(Some(Align::Center));
    }
    response
}

/// SSH server registration, test, and remote launch confirmation.
pub fn ssh_servers_window(model: &mut AppModel, ctx: &egui::Context) {
    ctx.add_plugin(SshFieldFocusRestore);
    let open_now = model.ssh_open();
    let was_open = ctx
        .data(|data| data.get_temp::<bool>(ssh_window_was_open_id()))
        .unwrap_or(false);
    if !open_now {
        if was_open {
            reset_ssh_window_state(ctx);
            ctx.data_mut(|data| data.insert_temp(ssh_window_was_open_id(), false));
            if let Some(workbench) = model.workbench_mut() {
                workbench.request_terminal_focus(ctx);
            }
        }
        return;
    }
    ctx.data_mut(|data| data.insert_temp(ssh_window_was_open_id(), true));
    let just_opened = !was_open;
    let focus_before = ctx.memory(|memory| memory.focused());
    let tab_forward =
        ctx.input(|input| input.key_pressed(egui::Key::Tab) && !input.modifiers.shift);
    let body_max_height = (ctx.content_rect().height() - 100.0).max(96.0);
    let body_height = ctx
        .data(|data| data.get_temp::<f32>(ssh_body_height_id()))
        .unwrap_or(0.0)
        .clamp(0.0, body_max_height);
    let rows_tab_after_actions = model.ssh_prompt().is_none()
        && model.remote_launch().is_none()
        && !model.ssh_selected().is_some_and(|selected| {
            model
                .ssh_registry()
                .ok()
                .is_some_and(|registry| registry.can_launch_grok(selected))
        });
    let mut first_row = None;
    let mut open = true;
    let layout = Layout::for_window(ctx.content_rect().width(), ctx.content_rect().height());
    let screen = ctx.content_rect();
    let previous_size = ctx
        .data(|data| data.get_temp::<egui::Rect>(ssh_window_rect_id()))
        .map_or(egui::vec2(460.0, 120.0), |rect| rect.size());
    let window_position = egui::pos2(
        (screen.center().x - previous_size.x * 0.5).clamp(
            screen.left(),
            (screen.right() - previous_size.x).max(screen.left()),
        ),
        (screen.center().y - previous_size.y * 0.5).clamp(
            screen.top(),
            (screen.bottom() - previous_size.y).max(screen.top()),
        ),
    );
    let shown = egui::Window::new("Servidores SSH")
        .id(ssh_servers_window_id())
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .current_pos(window_position)
        .constrain_to(screen)
        .default_width(460.0)
        .frame(theme::floating_frame())
        .show(ctx, |ui| {
            ui.set_min_width(420.0);
            ui.spacing_mut().item_spacing.y = layout.gap;
            ui.spacing_mut().item_spacing.x = layout.gap;
            let mut content_height = 0.0;
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), body_height),
                egui::Layout::top_down(Align::Min),
                |ui| {
                    let body = bounded_scroll_area(
                        ("made-ssh-window-body",),
                        body_max_height,
                        true,
                        just_opened,
                    );
                    let output = body.show(ui, |ui| {
                        if let Err(error) = model.ssh_registry() {
                            ui.label(theme::body(sanitize_display(error, 300)));
                        }
                        if let Some(notice) = model.ssh_notice() {
                            ui.label(theme::micro(sanitize_display(notice, 240)));
                        }
                        if model.ssh_host_key_changed() {
                            ui.label(theme::micro(
                                "The stored key no longer matches this server. Forget it only if the server key change was expected.",
                            ));
                            if ui.button("Forget old host key").clicked() {
                                if let Err(error) = model.forget_selected_ssh_host_key() {
                                    model.set_ssh_notice(error);
                                }
                            }
                        }
                        let listed = model.ssh_registry().ok().map(|registry| {
                            registry
                                .servers()
                                .iter()
                                .map(|server| {
                                    let id = server.id();
                                    let probe = match registry.probe_state(id) {
                                        crate::ssh_servers::ProbeState::Complete(capabilities) => {
                                            let opencode = if capabilities.opencode {
                                                "OpenCode detected"
                                            } else {
                                                "OpenCode not detected"
                                            };
                                            if capabilities.grok {
                                                format!("{opencode} · grok detected")
                                            } else {
                                                opencode.to_owned()
                                            }
                                        }
                                        crate::ssh_servers::ProbeState::Failed(_) => "test failed".to_owned(),
                                        crate::ssh_servers::ProbeState::Running(_) => "testing".to_owned(),
                                        crate::ssh_servers::ProbeState::Cancelled => "cancelled".to_owned(),
                                        crate::ssh_servers::ProbeState::Idle => "not tested".to_owned(),
                                    };
                                    (id, server.metadata().display_name.clone(), probe)
                                })
                                .collect::<Vec<_>>()
                        });
                        if let Some(servers) = listed {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                for (id, name, probe) in servers {
                                    let selected = model.ssh_selected() == Some(id);
                                    let label = format!("{} · {probe}", sanitize_display(&name, 48));
                                    let accessible_label =
                                        sanitize_text(&format!("{name} · {probe}"));
                                    if first_row.is_none() {
                                        first_row = Some(ssh_server_row_id(id));
                                    }
                                    let response = ssh_server_row(
                                        ui,
                                        id,
                                        selected,
                                        &label,
                                        &accessible_label,
                                        layout.button_height,
                                    );
                                    let activated = response.clicked()
                                        || (response.has_focus()
                                            && ui.input(|input| {
                                                input.key_pressed(egui::Key::Enter)
                                                    || input.key_pressed(egui::Key::Space)
                                            }));
                                    if activated {
                                        model.select_ssh_server(id);
                                    }
                                }
                            });
                        }
                        {
                            let draft = model.ssh_draft();
                            ssh_field(ui, "Display name", "name", &mut draft.name);
                            ssh_field(ui, "Host or SSH alias", "host", &mut draft.host);
                            ssh_user_port_row(ui, &layout, &mut draft.user, &mut draft.port);
                            ssh_field(ui, "Key file reference", "key", &mut draft.key_file);
                            ssh_field(
                                ui,
                                "Initial remote directory",
                                "directory",
                                &mut draft.directory,
                            );
                        }
                        ui.label(theme::micro(
                            "Passwords, tokens, and private key bytes are not stored.",
                        ));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = layout.gap;
                            if theme::primary_action(
                                ui,
                                top_action_id("ssh-save"),
                                "Save",
                                layout.button_height,
                                true,
                                theme::BG_CARD,
                            )
                            .activated
                            {
                                if let Err(error) = model.save_ssh_server() {
                                    model.set_ssh_notice(error);
                                }
                            }
                            if theme::ghost_action(
                                ui,
                                top_action_id("ssh-test"),
                                "Test",
                                false,
                                layout.button_height,
                                theme::BG_CARD,
                            )
                            .activated
                            {
                                if let Err(error) = model.test_selected_ssh_server() {
                                    model.set_ssh_notice(error);
                                }
                            }
                            if theme::ghost_action(
                                ui,
                                top_action_id("ssh-remove"),
                                "Remove",
                                false,
                                layout.button_height,
                                theme::BG_CARD,
                            )
                            .activated
                            {
                                if let Err(error) = model.remove_selected_ssh_server() {
                                    model.set_ssh_notice(error);
                                }
                            }
                        });
                        if model.ssh_prompt().is_some() {
                            ui.separator();
                            let class = model
                                .ssh_prompt()
                                .map(|prompt| crate::remote_session::classify_prompt(&prompt.text));
                            let changed = matches!(
                                class,
                                Some(crate::remote_session::SshPromptClass::ChangedHostKey { .. })
                            );
                            if let Some(class) = class {
                                ssh_prompt_card(ui, &class, model.prompt_draft());
                            }
                            ui.horizontal(|ui| {
                                if !changed && ui.button("Approve").clicked() {
                                    model.submit_ssh_prompt(true);
                                }
                                if ui
                                    .button(if changed {
                                        "Refuse key"
                                    } else {
                                        "Cancel prompt"
                                    })
                                    .clicked()
                                {
                                    model.submit_ssh_prompt(false);
                                    if !changed {
                                        model.cancel_ssh_probe();
                                    }
                                }
                            });
                        }
                        let launch_title = model.remote_launch().map(|launch| {
                            if launch.opencode {
                                format!("OpenCode SSH — {}", sanitize_display(&launch.label, 48))
                            } else {
                                format!("Terminal SSH — {}", sanitize_display(&launch.label, 48))
                            }
                        });
                        grok_install_consent_card(model, ui, &layout);
                        if let Some(title) = launch_title {
                            ui.separator();
                            ui.label(theme::body(title));
                            if let Some(directory) = model.remote_launch_directory() {
                                ssh_field(ui, "Remote directory", "launch-directory", directory);
                            }
                            ui.horizontal(|ui| {
                                if theme::primary_action(
                                    ui,
                                    top_action_id("ssh-launch"),
                                    "Open remote workspace",
                                    layout.button_height,
                                    true,
                                    theme::BG_CARD,
                                )
                                .activated
                                {
                                    if let Err(error) = model.confirm_remote_launch() {
                                        model.set_ssh_notice(error);
                                    }
                                }
                                if ui.button("Cancel").clicked() {
                                    model.cancel_remote_launch();
                                }
                            });
                        }
                    });
                    content_height = output.content_size.y;
                },
            );
            ui.ctx().data_mut(|data| {
                data.insert_temp(ssh_body_height_id(), content_height);
            });
        });
    if let Some(shown) = shown {
        ctx.data_mut(|data| {
            data.insert_temp(ssh_window_rect_id(), shown.response.rect);
            data.insert_temp(ssh_window_layer_id(), shown.response.layer_id);
        });
    }
    if tab_forward && rows_tab_after_actions && focus_before == Some(top_action_id("ssh-remove")) {
        if let Some(first_row) = first_row {
            ctx.data_mut(|data| data.insert_temp(ssh_pending_row_focus_id(), first_row));
        }
    }
    let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));
    if escape && !egui::Popup::is_any_open(ctx) {
        open = false;
    }
    if !open {
        reset_ssh_window_state(ctx);
        ctx.data_mut(|data| data.insert_temp(ssh_window_was_open_id(), false));
        model.set_ssh_open(false);
        if let Some(workbench) = model.workbench_mut() {
            workbench.request_terminal_focus(ctx);
        }
    }
}

/// Global workspace tabs; OpenCode sessions remain a separate optional map.
pub fn workspace_strip(model: &mut AppModel, ui: &mut Ui) -> bool {
    let layout = Layout::for_window(ui.available_width(), ui.ctx().content_rect().height());
    let active = model.active_workspace_id();
    let update_ready = model.update_ready();
    let tabs: Vec<WorkspaceTabInfo> = model
        .workspace_ids()
        .filter_map(|id| Some(workspace_tab_info(id, model.workspace(id)?)))
        .collect();
    let has_workbench = model.workbench().is_some();
    let compatible_tuis = new_tab_menu_entries(model);
    let explorer_visible = model.workbench().is_some_and(Workbench::explorer_visible);
    let sessions_visible = model.workbench().is_some_and(Workbench::sessions_visible);
    let sessions_available = model.workbench().is_some_and(|workbench| {
        sessions_visible
            || !workbench.session_map().is_empty()
            || workbench.integration().status().is_connected()
            || workbench.remote_sessions_available()
    });
    if let (Some(active), Some(workbench)) = (active, model.workbench()) {
        let target = top_workspace_action_id(active, "explorer");
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::unique(("made-terminal-exit-target", workbench.pane_id())),
                target,
            );
        });
    }

    let mut activate = None;
    let mut close = None;
    let mut create = false;
    let mut launch_tui = None;
    let mut launch_remote_grok = None;
    let mut launch_remote_claude = None;
    let mut toggle_explorer = false;
    let mut toggle_sessions = false;
    let mut toggle_ssh = false;
    let mut about = false;
    let mut launch_remote: Option<(made_remote::protocol::AuthorityId, bool)> = None;

    let panel_height = layout.content_top();
    egui::Panel::top("made-workspaces")
        .exact_size(panel_height)
        .frame(egui::Frame::NONE)
        .show_separator_line(false)
        .show(ui, |ui| {
            let header = egui::Rect::from_min_size(
                egui::pos2(layout.gutter, layout.header_top()),
                egui::vec2(
                    (ui.available_width() - 2.0 * layout.gutter).max(1.0),
                    layout.top_height,
                ),
            );
            // No extra horizontal inset: the control row spans the card edges.
            let content = header;
            let control_gap = if layout.compact { 4.0 } else { 6.0 };
            let wordmark_gap = theme::wordmark_gap(layout.compact);

            let mut right_specs: Vec<(&'static str, theme::IconGlyph, &'static str, bool)> =
                Vec::new();
            if sessions_available {
                right_specs.push((
                    "sessions",
                    theme::IconGlyph::SessionList,
                    if sessions_visible {
                        "Hide sessions"
                    } else {
                        "Show sessions"
                    },
                    sessions_visible,
                ));
            }
            right_specs.push((
                "ssh-servers",
                theme::IconGlyph::Servers,
                "Servidores SSH",
                model.ssh_open(),
            ));
            if has_workbench {
                right_specs.push((
                    "explorer",
                    theme::IconGlyph::PanelRight,
                    if explorer_visible {
                        "Hide explorer"
                    } else {
                        "Show explorer"
                    },
                    explorer_visible,
                ));
            }
            let icon_extent = theme::icon_action_extent(layout.tab_height);
            let right_width = if right_specs.is_empty() {
                0.0
            } else {
                icon_extent * right_specs.len() as f32
                    + control_gap * (right_specs.len().saturating_sub(1) as f32)
            };
            let right_rect = egui::Rect::from_min_size(
                egui::pos2(
                    (content.right() - right_width).max(content.left()),
                    content.top(),
                ),
                egui::vec2(right_width.min(content.width()), content.height()),
            );
            let mut right = ui.new_child(
                UiBuilder::new()
                    .max_rect(right_rect)
                    .layout(egui::Layout::left_to_right(Align::Center)),
            );
            right.spacing_mut().item_spacing.x = control_gap;
            for (name, glyph, label, selected) in &right_specs {
                let id = match *name {
                    "explorer" | "sessions" => active
                        .map(|active| top_workspace_action_id(active, name))
                        .unwrap_or_else(|| top_action_id(name)),
                    _ => top_action_id(name),
                };
                let action = theme::icon_action(
                    &mut right,
                    id,
                    *glyph,
                    label,
                    *selected,
                    layout.tab_height,
                    theme::BG_CLEAR,
                );
                match *name {
                    "sessions" if action.activated => toggle_sessions = true,
                    "explorer" if action.activated => toggle_explorer = true,
                    "ssh-servers" if action.activated => toggle_ssh = true,
                    _ => {}
                }
            }

            // Tabs start at the header's left edge. The cached wordmark sits to
            // their right, immediately before the icon group, with a deliberate
            // breathing gap on both sides. It is not a tab and paints no fill.
            let wordmark_width = theme::wordmark_width(ui);
            let wordmark_rect = egui::Rect::from_min_size(
                egui::pos2(
                    (right_rect.left() - wordmark_gap - wordmark_width).max(content.left()),
                    content.top(),
                ),
                egui::vec2(wordmark_width.min(content.width()), content.height()),
            );
            let mut wordmark = ui.new_child(
                UiBuilder::new()
                    .max_rect(wordmark_rect)
                    .layout(egui::Layout::left_to_right(Align::Center)),
            );
            let trigger = theme::wordmark_about_action(
                &mut wordmark,
                layout.tab_height,
                theme::BG_CLEAR,
                update_ready && active.is_some(),
            );
            if trigger.activated && active.is_some() {
                about = true;
            }

            let tabs_rect = egui::Rect::from_min_max(
                content.min,
                egui::pos2(
                    (wordmark_rect.left() - wordmark_gap).max(content.left()),
                    content.bottom(),
                ),
            );
            let active_index = tabs.iter().position(|tab| Some(tab.id) == active);
            let widths: Vec<f32> = tabs
                .iter()
                .map(|tab| workspace_tab_width(&tab.label, &layout, tab.sessions.len()))
                .collect();
            let plus_extent = theme::icon_action_extent(layout.tab_height);
            let tabs_available = tabs_rect.width().max(0.0);
            let (visible, overflow) = bounded_tab_indices(
                &widths,
                active_index,
                tabs_available,
                control_gap,
                Some(plus_extent),
            );
            let mut tab_strip = ui.new_child(
                UiBuilder::new()
                    .max_rect(tabs_rect)
                    .layout(egui::Layout::left_to_right(Align::Center)),
            );
            tab_strip.spacing_mut().item_spacing.x = control_gap;
            for index in &visible {
                let tab = &tabs[*index];
                let width = widths[*index];
                let result = workspace_tab_ui(
                    &mut tab_strip,
                    tab,
                    width,
                    &layout,
                    Some(tab.id) == active,
                    &tabs,
                    &visible,
                );
                if result.0 {
                    activate = Some(tab.id);
                }
                if result.1 {
                    close = Some(tab.id);
                }
            }
            let open_folder = theme::icon_action(
                &mut tab_strip,
                top_action_id("open-folder"),
                theme::IconGlyph::Plus,
                "Open folder",
                false,
                layout.tab_height,
                theme::BG_CLEAR,
            );
            let has_ssh_servers = model
                .ssh_registry()
                .ok()
                .is_some_and(|registry| !registry.servers().is_empty());
            // No local TUI and no saved server: open the picker. A menu with
            // only Terminal is the same choice, and it hid the picker.
            let show_new_tab_menu = !compatible_tuis.is_empty() || has_ssh_servers;
            if open_folder.activated && !show_new_tab_menu {
                create = true;
            }
            if show_new_tab_menu {
                // Never reserve more rows than the viewport actually has left;
                // the scroll area shrinks a short menu and caps a taller one
                // without pushing controls past the screen edge.
                let menu_max_height =
                    (ui.ctx().content_rect().bottom() - open_folder.response.rect.bottom() - 8.0)
                        .max(0.0);
                let reset_menu_scroll = open_folder.activated;
                egui::Popup::menu(&open_folder.response)
                    .open_memory(
                        open_folder
                            .activated
                            .then_some(egui::SetOpenCommand::Toggle),
                    )
                    .show(|ui| {
                        let menu = bounded_scroll_area(
                            ("made-new-tab-menu",),
                            menu_max_height,
                            false,
                            reset_menu_scroll,
                        );
                        menu.show(ui, |ui| {
                            if new_tab_menu_item(
                                ui,
                                new_tab_menu_item_id("terminal"),
                                theme::MenuLeading::Glyph(theme::IconGlyph::Terminal),
                                "Terminal",
                                layout.button_height,
                            ) {
                                create = true;
                                ui.close();
                            }
                            for tui in &compatible_tuis {
                                let descriptor = tui.descriptor();
                                if new_tab_menu_item(
                                    ui,
                                    new_tab_menu_item_id(descriptor.menu_item_id),
                                    mark_leading(descriptor.mark),
                                    descriptor.label,
                                    layout.button_height,
                                ) {
                                    launch_tui = Some(*tui);
                                    ui.close();
                                }
                            }
                            if let Ok(registry) = model.ssh_registry() {
                                for server in registry.servers() {
                                    let name =
                                        sanitize_display(&server.metadata().display_name, 48);
                                    let terminal_label = format!("Terminal SSH — {name}");
                                    if new_tab_menu_item(
                                        ui,
                                        new_tab_menu_item_id(&format!(
                                            "ssh-terminal-{}",
                                            authority_key(server.id())
                                        )),
                                        theme::MenuLeading::Glyph(theme::IconGlyph::Servers),
                                        &terminal_label,
                                        layout.button_height,
                                    ) {
                                        launch_remote = Some((server.id(), false));
                                        ui.close();
                                    }
                                    // BEGIN remote-path (AIA-02)
                                    if registry.can_launch_opencode(server.id()) {
                                        let opencode_label = format!("OpenCode SSH — {name}");
                                        if new_tab_menu_item(
                                            ui,
                                            new_tab_menu_item_id(&format!(
                                                "ssh-opencode-{}",
                                                authority_key(server.id())
                                            )),
                                            theme::MenuLeading::Glyph(theme::IconGlyph::Terminal),
                                            &opencode_label,
                                            layout.button_height,
                                        ) {
                                            launch_remote = Some((server.id(), true));
                                            ui.close();
                                        }
                                    }
                                    if registry.can_launch_grok(server.id()) {
                                        let grok_label = format!("Grok SSH — {name}");
                                        if new_tab_menu_item(
                                            ui,
                                            new_tab_menu_item_id(&format!(
                                                "ssh-grok-{}",
                                                authority_key(server.id())
                                            )),
                                            grok_mark(),
                                            &grok_label,
                                            layout.button_height,
                                        ) {
                                            launch_remote_grok = Some(server.id());
                                            ui.close();
                                        }
                                    }
                                    if registry.can_launch_claude(server.id()) {
                                        let claude_label = format!("Claude Code SSH — {name}");
                                        if new_tab_menu_item(
                                            ui,
                                            new_tab_menu_item_id(&format!(
                                                "ssh-claude-{}",
                                                authority_key(server.id())
                                            )),
                                            claude_mark(),
                                            &claude_label,
                                            layout.button_height,
                                        ) {
                                            launch_remote_claude = Some(server.id());
                                            ui.close();
                                        }
                                    }
                                    // END remote-path
                                }
                            }
                        });
                    });
            }
            if overflow {
                let overflow_action = theme::ghost_action(
                    &mut tab_strip,
                    top_action_id("workspace-overflow"),
                    "…",
                    false,
                    layout.button_height,
                    theme::BG_CLEAR,
                );
                let overflow_response = overflow_action.response;
                egui::Popup::menu(&overflow_response).show(|ui| {
                    ui.set_max_width(420.0);
                    for tab in &tabs {
                        let is_active = Some(tab.id) == active;
                        if ui
                            .selectable_label(is_active, sanitize_display(&tab.label, 80))
                            .clicked()
                        {
                            activate = Some(tab.id);
                            ui.close();
                        }
                        ui.horizontal(|ui| {
                            let path_text = tab
                                .remote_path
                                .clone()
                                .unwrap_or_else(|| tab.path.display().to_string());
                            ui.label(theme::mono(
                                truncate_path(&sanitize_display(&path_text, 900), 72),
                                10.0,
                                theme::TEXT_TERTIARY,
                            ));
                            if ui.small_button("Close").clicked() {
                                close = Some(tab.id);
                                ui.close();
                            }
                        });
                    }
                });
                overflow_response.on_hover_text("All workspaces");
            }
        });

    if let Some(tui) = launch_tui {
        model.begin_tui_workspace(tui);
    }
    // BEGIN remote-path (AIA-02)
    if let Some((id, opencode)) = launch_remote {
        if let Err(error) = model.begin_remote_choice(id, opencode) {
            model.set_ssh_notice(error);
            model.set_ssh_open(true);
        }
    }
    if let Some(id) = launch_remote_grok {
        if let Err(error) = model.begin_remote_grok_launch(id) {
            model.set_ssh_notice(error);
            model.set_ssh_open(true);
        }
    }
    if let Some(id) = launch_remote_claude {
        if let Err(error) = model.begin_remote_claude_launch(id) {
            model.set_ssh_notice(error);
            model.set_ssh_open(true);
        }
    }
    // END remote-path
    if toggle_ssh {
        model.toggle_ssh();
    }
    if create {
        model.begin_new_workspace();
    }
    if let Some(id) = activate {
        model.switch_workspace(id);
    }
    if let Some(id) = close {
        model.close_workspace(id);
    }
    if let Some(notice) = model.remote_close_notice() {
        ui.label(theme::micro(sanitize_display(notice, 180)));
    }
    if toggle_explorer {
        if let Some(workbench) = model.workbench_mut() {
            workbench.set_explorer_visible(!explorer_visible);
        }
    }
    if toggle_sessions {
        if let Some(workbench) = model.workbench_mut() {
            workbench.set_sessions_visible(!sessions_visible);
        }
    }
    about
}

fn new_tab_menu_item(
    ui: &mut Ui,
    id: egui::Id,
    leading: theme::MenuLeading,
    label: &str,
    height: f32,
) -> bool {
    let action = theme::menu_labeled_action(ui, id, leading, label, height, theme::BG_CARD);
    if action.response.has_focus() {
        action.response.scroll_to_me(Some(Align::Center));
    }
    action
        .response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    action.activated
}

fn paint_workspace_tab_session_dots(
    ui: &mut Ui,
    tab: egui::Rect,
    sessions: &[SessionActivity],
    fade: f32,
) {
    let shown = workspace_tab_visible_session_count(sessions);
    if shown == 0 || fade <= 0.0 {
        return;
    }
    let fade = fade.clamp(0.0, 1.0);
    let busy_alpha = workspace_busy_status_alpha(ui.input(|input| input.time)) as f32 * fade;
    let mut has_busy = false;
    for (index, activity) in sessions.iter().take(shown).enumerate() {
        let offset = (shown - 1 - index) as f32
            * (WORKSPACE_TAB_STATUS_DOT_RADIUS * 2.0 + WORKSPACE_TAB_STATUS_DOT_GAP);
        let center = egui::pos2(
            tab.right() - WORKSPACE_TAB_STATUS_CORNER_INSET - offset,
            tab.center().y,
        );
        match activity {
            SessionActivity::Idle => {
                ui.painter().circle_filled(
                    center,
                    WORKSPACE_TAB_STATUS_DOT_RADIUS,
                    fade_color(theme::TEXT_TERTIARY, fade),
                );
            }
            SessionActivity::Attention => {
                ui.painter().circle_filled(
                    center,
                    WORKSPACE_TAB_STATUS_DOT_RADIUS,
                    fade_color(theme::WARNING, fade),
                );
            }
            SessionActivity::Busy => {
                has_busy = true;
                let [r, g, b, _] = theme::ACCENT.to_srgba_unmultiplied();
                ui.painter().circle_filled(
                    center,
                    WORKSPACE_TAB_STATUS_DOT_RADIUS,
                    Color32::from_rgba_unmultiplied(
                        r,
                        g,
                        b,
                        busy_alpha.round().clamp(0.0, 255.0) as u8,
                    ),
                );
            }
        }
    }
    if has_busy {
        ui.request_repaint();
    }
}

fn workspace_tab_ui(
    ui: &mut Ui,
    tab: &WorkspaceTabInfo,
    width: f32,
    layout: &Layout,
    active: bool,
    all_tabs: &[WorkspaceTabInfo],
    visible: &[usize],
) -> (bool, bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, layout.tab_height), Sense::hover());
    let response = ui.interact(rect, workspace_tab_id(tab.id), Sense::click());
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 13.0, rect.center().y),
        egui::vec2(24.0, layout.tab_height.max(24.0)),
    );
    let close_response = ui
        .interact(close_rect, workspace_tab_close_id(tab.id), Sense::click())
        .on_hover_text("Close workspace");
    // The close affordance is part of the tab's pointer-over state: hovering
    // only the X still applies the parent tab hover fill and label, but the parent
    // path tooltip is suppressed so the close tooltip is the one shown.
    let close_hovered = close_response.hovered();
    let hovered = response.hovered() || close_hovered;
    let close_visible = hovered || response.has_focus() || close_response.has_focus();
    let close_reveal = ui.ctx().animate_bool_with_time_and_easing(
        workspace_tab_close_reveal_id(tab.id),
        close_visible,
        WORKSPACE_TAB_CLOSE_FADE_TIME,
        egui::emath::easing::quadratic_out,
    );
    let fill = if active {
        theme::BG_TAB_SELECTED
    } else if hovered {
        theme::BG_TAB_HOVER
    } else {
        theme::BG_TAB_IDLE
    };
    let stroke = if active {
        theme::workspace_tab_stroke()
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
    let session_summary = opencode_sessions_summary(&tab.sessions);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::Role::Tab,
            true,
            active,
            workspace_tab_accessible_label(&tab.label, session_summary.as_deref()),
        )
    });
    let label_left = rect.left() + WORKSPACE_TAB_INSET + WORKSPACE_TAB_ICON_SLOT;
    let label_width = workspace_tab_label_width(rect.width(), tab.sessions.len());
    let color = if active || hovered {
        theme::TEXT_PRIMARY
    } else {
        theme::inactive_tab_text()
    };
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(
            rect.left() + WORKSPACE_TAB_INSET + WORKSPACE_TAB_ICON_SIZE * 0.5,
            rect.center().y,
        ),
        egui::Vec2::splat(WORKSPACE_TAB_ICON_SIZE),
    );
    if let Some(brand) = &tab.brand {
        match mark_leading(brand.entry.descriptor().mark) {
            theme::MenuLeading::Image { uri, bytes } => {
                egui::Image::from_bytes(uri, bytes).paint_at(ui, icon_rect);
            }
            theme::MenuLeading::Glyph(glyph) => {
                theme::paint_icon(ui.painter(), icon_rect, glyph, color);
            }
        }
    } else {
        theme::paint_icon(ui.painter(), icon_rect, theme::IconGlyph::Terminal, color);
    }
    let font = FontId::proportional(if active {
        theme::TAB_SELECTED_FONT_SIZE
    } else {
        theme::TAB_FONT_SIZE
    });
    let job = theme::ellipsized_job(sanitize_display(&tab.label, 80), font, color, label_width);
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        egui::pos2(label_left, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    let dots_fade = 1.0 - close_reveal;
    if dots_fade > 0.0 {
        paint_workspace_tab_session_dots(ui, rect, &tab.sessions, dots_fade);
    }
    if close_reveal > 0.0 {
        let close_color = if close_hovered {
            theme::ERROR
        } else {
            theme::inactive_icon_color()
        };
        ui.painter().text(
            close_rect.center(),
            Align2::CENTER_CENTER,
            "×",
            FontId::proportional(theme::CLOSE_GLYPH_SIZE),
            fade_color(close_color, close_reveal),
        );
    }
    if response.has_focus() {
        theme::paint_focus_ring(ui, rect, CornerRadius::same(6), theme::BG_CLEAR);
    }
    if close_response.has_focus() {
        theme::paint_inside_focus_ring(ui.painter(), close_rect.shrink(1.0), CornerRadius::same(4));
    }
    // A branded tab omits the workspace path. With no sessions the summary is
    // empty, so the tab has no parent tooltip.
    let tooltip = workspace_tab_tooltip(tab, session_summary.as_deref());
    let tab_response = if close_hovered {
        response
    } else if let Some(tooltip) = tooltip {
        response.on_hover_text(tooltip)
    } else {
        response
    };
    let keyboard = tab_response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        });
    let close_keyboard = close_response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        });
    if tab_response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
        })
    {
        return (false, true);
    }
    if tab_response.has_focus() {
        let step = if ui.input(|input| input.key_pressed(egui::Key::ArrowLeft)) {
            Some(-1isize)
        } else if ui.input(|input| input.key_pressed(egui::Key::ArrowRight)) {
            Some(1isize)
        } else {
            None
        };
        if let Some(step) = step {
            if let Some(position) = visible.iter().position(|visible_index| {
                all_tabs
                    .get(*visible_index)
                    .is_some_and(|item| item.id == tab.id)
            }) {
                let next = (position as isize + step).rem_euclid(visible.len() as isize) as usize;
                if let Some(next_tab) = visible.get(next).and_then(|index| all_tabs.get(*index)) {
                    ui.memory_mut(|memory| memory.request_focus(workspace_tab_id(next_tab.id)));
                }
            }
        }
    }
    (
        (tab_response.clicked() || keyboard) && !close_keyboard,
        close_response.clicked() || close_keyboard,
    )
}

struct DiagnosticsUiCache {
    signature: u64,
    snapshot: DiagnosticsSnapshot,
    bridge_stats: String,
    log_path: String,
    copy: String,
}

struct DiagnosticsMetricsCache {
    bucket: u64,
    text: String,
}

fn diagnostics_cache_id(pane_id: &str) -> egui::Id {
    egui::Id::unique(("made-diagnostics-cache", pane_id))
}

fn about_open_id() -> egui::Id {
    egui::Id::unique("made-about-open")
}

/// Smooth reveal time for the About diagnostics detail.
const ABOUT_REVEAL_TIME: f32 = 0.22;

fn diagnostics_metrics_cache_id(pane_id: &str) -> egui::Id {
    egui::Id::unique(("made-diagnostics-metrics", pane_id))
}

fn diagnostics_metrics_bucket(ctx: &egui::Context) -> u64 {
    (ctx.input(|input| input.time) * 4.0).floor().max(0.0) as u64
}

fn diagnostics_previous_log_bucket(ctx: &egui::Context) -> u64 {
    ctx.input(|input| input.time).floor().max(0.0) as u64
}

fn heartbeat_age(
    last: Option<std::time::Instant>,
    now: std::time::Instant,
) -> Option<std::time::Duration> {
    last.map(|last| now.saturating_duration_since(last))
}

/// Stable id for the About diagnostics reveal toggle in one workspace.
pub fn about_diagnostics_toggle_id(pane_id: &str) -> egui::Id {
    egui::Id::unique(("made-about-diagnostics-toggle", pane_id))
}

/// Stable id for the diagnostics copy action in one workspace.
pub fn diagnostics_copy_id(pane_id: &str) -> egui::Id {
    egui::Id::unique(("made-diagnostics-copy", pane_id))
}

/// Stable id for the previous-log copy action in one workspace.
pub fn diagnostics_previous_log_copy_id(pane_id: &str) -> egui::Id {
    egui::Id::unique(("made-diagnostics-previous-log-copy", pane_id))
}

/// Stable id for the previous-log path label in one workspace.
pub fn diagnostics_previous_log_path_id(pane_id: &str) -> egui::Id {
    egui::Id::unique(("made-diagnostics-previous-log-path", pane_id))
}

/// Minimal clipboard adapter so UI copy actions use the diagnostics API.
struct UiClipboard {
    ctx: egui::Context,
}

impl Clipboard for UiClipboard {
    fn get_text(&mut self) -> Result<String, PlatformError> {
        Err(PlatformError::message(
            "clipboard read is not available from the diagnostics window",
        ))
    }

    fn set_text(&mut self, text: &str) -> Result<(), PlatformError> {
        self.ctx.copy_text(text.to_owned());
        Ok(())
    }
}

impl Workbench {
    /// Draw the complete workspace surface.
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        show_sessions: &mut bool,
        show_about: &mut bool,
        updates: &mut UpdatePanel,
        native_backdrop_applied: bool,
    ) {
        if !self.is_visible() {
            return;
        }
        self.record_paint();
        *show_sessions = self.sessions_visible();
        let layout = Layout::for_window(ui.available_width(), ui.ctx().content_rect().height());
        let margin = egui::Margin {
            left: layout.gutter as i8,
            right: layout.gutter as i8,
            top: 0,
            bottom: layout.gutter as i8,
        };
        egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
            self.workspace_ui(ui, &layout, show_about, updates, native_backdrop_applied);
        });
        self.set_sessions_visible(*show_sessions);
    }

    fn workspace_ui(
        &mut self,
        ui: &mut Ui,
        layout: &Layout,
        show_about: &mut bool,
        updates: &mut UpdatePanel,
        native_backdrop_applied: bool,
    ) {
        ui.ctx()
            .data_mut(|data| data.remove::<egui::Rect>(terminal_hit_rect_id()));
        self.sync_terminal_visibility(ui.ctx());
        self.main_panels(ui, layout, native_backdrop_applied);
        self.apply_terminal_focus_boost(ui.ctx());
        let ctx = ui.ctx().clone();
        self.toast(&ctx, layout);
        self.about_window(&ctx, show_about, updates, layout);
    }

    pub(crate) fn sync_terminal_visibility(&mut self, ctx: &egui::Context) {
        let visible = self.is_visible() && self.left_surface() == LeftSurface::Terminal;
        if let Some(terminal) = self.terminal.as_mut() {
            terminal.set_visible(visible);
            terminal.sync_visibility(ctx, self.diag.log_mut());
        }
    }

    /// Explicitly move keyboard focus into the visible terminal after a surface
    /// or panel transition. Focus is re-applied for a few frames because the
    /// first terminal frame after a hidden preview can legitimately allocate a
    /// new auto widget id.
    fn request_terminal_focus(&mut self, ctx: &egui::Context) {
        if ssh_field_held(ctx) {
            return;
        }
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::unique(("made-terminal-focus-boost", self.pane_id())),
                4u8,
            );
        });
    }

    fn apply_terminal_focus_boost(&mut self, ctx: &egui::Context) {
        if ssh_field_held(ctx) {
            return;
        }
        let boost_id = egui::Id::unique(("made-terminal-focus-boost", self.pane_id()));
        if self.left_surface() != LeftSurface::Terminal {
            ctx.data_mut(|data| data.remove::<u8>(boost_id));
            return;
        }
        let mut boost = ctx.data(|data| data.get_temp::<u8>(boost_id)).unwrap_or(0);
        if boost == 0 {
            return;
        }
        if let Some(focus_id) = self
            .terminal
            .as_ref()
            .and_then(|terminal| terminal.focus_id())
        {
            ctx.memory_mut(|memory| memory.request_focus(focus_id));
        }
        boost = boost.saturating_sub(1);
        ctx.data_mut(|data| data.insert_temp(boost_id, boost));
    }

    fn main_panels(&mut self, ui: &mut Ui, layout: &Layout, native_backdrop_applied: bool) {
        let mut explorer = self.explorer.take();
        if let Some(pane) = explorer.as_mut() {
            pane.set_ui_scope(egui::Id::unique(("made-explorer", self.pane_id())));
        }
        let explorer_visible = self.explorer_visible();
        let visibility_id = egui::Id::unique(("made-explorer-visible", self.pane_id()));
        let was_explorer_visible = ui
            .ctx()
            .data(|data| data.get_temp::<bool>(visibility_id).unwrap_or(true));
        if was_explorer_visible && !explorer_visible {
            if let Some(pane) = explorer.as_ref() {
                pane.release_focus(ui.ctx());
            }
            if self.is_remote() {
                let tree_focus =
                    egui::Id::unique(("made-explorer", self.pane_id())).with("tree-focus");
                if ui.ctx().memory(|memory| memory.has_focus(tree_focus)) {
                    ui.ctx()
                        .memory_mut(|memory| memory.surrender_focus(tree_focus));
                }
            }
            if self.left_surface() == LeftSurface::Terminal {
                self.request_terminal_focus(ui.ctx());
            }
        }
        ui.ctx()
            .data_mut(|data| data.insert_temp(visibility_id, explorer_visible));
        let available = ui.available_width().max(1.0);
        let has_content = explorer.is_some() || self.workspace_error.is_some() || self.is_remote();
        let target_visible = self.explorer_visible() && has_content;
        let animation_id = egui::Id::unique(("made-explorer-animation", self.pane_id()));
        let progress = ui
            .ctx()
            .animate_bool_with_time_and_easing(
                animation_id,
                target_visible,
                0.15,
                egui::emath::easing::quadratic_out,
            )
            .clamp(0.0, 1.0);
        if has_content && (target_visible || progress > 0.0) {
            let max_width = layout.explorer_max_width(available).max(0.0);
            let width = layout
                .explorer_width(self.explorer_width(), available)
                .clamp(0.0, available);
            let panel_width = ((width + layout.gap) * progress).clamp(0.0, available);
            // The panel's inner left margin is the gap; keep the content
            // rectangle non-negative during the first/last animation frames.
            if panel_width > layout.gap + 0.5 {
                egui::Panel::right(egui::Id::unique(("made-explorer-panel", self.pane_id())))
                    .resizable(false)
                    .exact_size(panel_width)
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                        left: layout.gap as i8,
                        right: 0,
                        top: 0,
                        bottom: 0,
                    }))
                    .show_separator_line(false)
                    .show(ui, |ui| {
                        let rect = ui.max_rect();
                        let splitter = egui::Rect::from_center_size(
                            egui::pos2(rect.left() - layout.gap * 0.5, rect.center().y),
                            egui::vec2(8.0, rect.height()),
                        );
                        let response = ui.interact(
                            splitter,
                            explorer_splitter_id(self.pane_id()),
                            Sense::DRAG,
                        );
                        if response.dragged() {
                            let delta = ui.input(|input| input.pointer.delta().x);
                            self.set_explorer_width(
                                (width - delta).clamp(theme::EXPLORER_MIN_WIDTH, max_width),
                            );
                        }
                        if response.hovered() || response.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        }
                        let splitter_fade = ui.ctx().animate_bool_with_time_and_easing(
                            egui::Id::unique(("made-splitter-fade", self.pane_id())),
                            response.hovered() || response.dragged(),
                            theme::SPLITTER_FADE_TIME,
                            egui::emath::easing::quadratic_out,
                        );

                        // Lay the Explorer out at its full width and clip it to
                        // the animated panel. This slides the card in/out
                        // without reflowing the tree every frame.
                        let content_rect = egui::Rect::from_min_max(
                            egui::pos2(rect.left(), rect.top()),
                            egui::pos2(rect.left() + width, rect.bottom()),
                        );
                        let mut content_ui = ui.new_child(
                            UiBuilder::new()
                                .max_rect(content_rect)
                                .id_salt(("made-explorer-content", self.pane_id())),
                        );
                        content_ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                        if let Some(explorer) = explorer.as_mut() {
                            explorer.set_native_backdrop_applied(native_backdrop_applied);
                            explorer.tree_ui(&mut content_ui, self.diag.log_mut(), layout);
                        } else if self.is_remote() {
                            let root = self.remote_path_label().and_then(|path| {
                                made_remote::protocol::RemotePath::new(path.into_bytes()).ok()
                            });
                            let actions = if let Some(root) = root {
                                self.remote_explorer_ui(
                                    &mut content_ui,
                                    &root,
                                    layout,
                                    egui::Id::unique(("made-explorer", self.pane_id())),
                                    native_backdrop_applied,
                                )
                            } else {
                                Vec::new()
                            };
                            self.apply_remote_actions(ui.ctx(), actions);
                        } else if let Some(error) = self.workspace_error.clone() {
                            unavailable_card(
                                &mut content_ui,
                                layout,
                                "Explorer unavailable",
                                &error.message(),
                            );
                        }

                        // Paint the divider after the card so the grip cannot
                        // be covered by the sliding content.
                        if splitter_fade > 0.0 {
                            let dragged = response.dragged();
                            let opacity = theme::splitter_opacity(splitter_fade, dragged);
                            if opacity > 0.001 {
                                let tone = theme::splitter_line_color(splitter_fade, dragged);
                                ui.painter().rect_filled(
                                    egui::Rect::from_center_size(
                                        splitter.center(),
                                        egui::vec2(1.0, splitter.height()),
                                    ),
                                    0.0,
                                    tone,
                                );
                                ui.painter().rect(
                                    egui::Rect::from_center_size(
                                        splitter.center(),
                                        egui::vec2(6.0, 32.0),
                                    ),
                                    CornerRadius::same(2),
                                    theme::splitter_grip_fill(splitter_fade, dragged),
                                    Stroke::new(1.0, tone),
                                    StrokeKind::Inside,
                                );
                            }
                        }
                        response.on_hover_text("Drag to resize explorer");
                    });
            }
        }

        let opened_source_control = explorer
            .as_mut()
            .is_some_and(ExplorerPane::take_source_control_opened);
        if explorer
            .as_mut()
            .is_some_and(ExplorerPane::take_preview_opened)
        {
            self.set_left_surface(LeftSurface::Preview);
            self.sync_terminal_visibility(ui.ctx());
        }
        if self.left_surface() == LeftSurface::Preview
            && ui.ctx().memory(|memory| memory.focused().is_none())
        {
            if let Some(pane) = explorer
                .as_mut()
                .filter(|pane| pane.preview_path().is_some())
            {
                pane.focus_preview();
            } else if self.is_remote() && self.remote_has_preview() {
                self.remote_preview_focus_pending = true;
            }
        }

        let mut preview_loaded = explorer
            .as_ref()
            .is_some_and(|pane| pane.preview_path().is_some())
            || self.remote_has_preview();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let mut show_preview = self.left_surface() == LeftSurface::Preview;
                if show_preview {
                    if let Some(pane) = explorer.as_mut() {
                        match pane.preview_ui(ui, self.diag.log_mut(), layout) {
                            PreviewUiAction::Close => {
                                self.close_preview();
                                self.sync_terminal_visibility(ui.ctx());
                                self.request_terminal_focus(ui.ctx());
                                preview_loaded = false;
                                show_preview = false;
                            }
                            PreviewUiAction::ShowTerminal => {
                                self.set_left_surface(LeftSurface::Terminal);
                                self.sync_terminal_visibility(ui.ctx());
                                pane.release_preview_focus(ui.ctx());
                                self.request_terminal_focus(ui.ctx());
                                show_preview = false;
                            }
                            PreviewUiAction::None => {}
                        }
                    } else if self.is_remote() {
                        match self.remote_preview_ui(ui, layout) {
                            PreviewUiAction::Close => {
                                self.close_preview();
                                self.sync_terminal_visibility(ui.ctx());
                                self.request_terminal_focus(ui.ctx());
                                preview_loaded = false;
                                show_preview = false;
                            }
                            PreviewUiAction::ShowTerminal => {
                                self.set_left_surface(LeftSurface::Terminal);
                                self.sync_terminal_visibility(ui.ctx());
                                self.request_terminal_focus(ui.ctx());
                                show_preview = false;
                            }
                            PreviewUiAction::None => {}
                        }
                    } else {
                        self.close_preview();
                        self.sync_terminal_visibility(ui.ctx());
                        preview_loaded = false;
                        show_preview = false;
                    }
                }
                if !show_preview {
                    let file_clicked = self.terminal_card(ui, layout, preview_loaded);
                    if file_clicked {
                        self.set_left_surface(LeftSurface::Preview);
                        self.sync_terminal_visibility(ui.ctx());
                        if let Some(pane) = explorer.as_mut() {
                            pane.focus_preview();
                        } else if self.is_remote() {
                            self.remote_preview_focus_pending = true;
                        }
                    }
                }
            });
        self.explorer = explorer;
        if opened_source_control {
            self.refresh_git_status_if_stale();
            ui.ctx()
                .request_repaint_after(crate::metrics::ACTIVE_REPAINT_INTERVAL);
        }
    }

    fn terminal_card(&mut self, ui: &mut Ui, layout: &Layout, preview_loaded: bool) -> bool {
        let card_rect = ui.available_rect_before_wrap();
        ui.ctx()
            .data_mut(|data| data.insert_temp(terminal_hit_rect_id(), card_rect));
        let alternate_screen = self
            .terminal
            .as_ref()
            .is_none_or(|terminal| terminal.alternate_screen());
        let terminal_fill = theme::terminal_surface(alternate_screen);
        let chrome_fill = theme::terminal_overlay_surface(alternate_screen);
        let mut card_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(card_rect)
                .id_salt(("made-terminal-card", self.pane_id())),
        );
        let ui = &mut card_ui;
        ui.painter().rect_filled(
            card_rect,
            CornerRadius::same(layout.card_radius),
            terminal_fill,
        );
        let mut top = card_rect.top();
        let focus_error = self
            .focus
            .last_error()
            .map(|error| sanitize_display(&error.message, 300));
        let show_sessions = self.sessions_visible();

        if show_sessions {
            let strip_rect = egui::Rect::from_min_size(
                egui::pos2(card_rect.left(), top),
                egui::vec2(card_rect.width(), layout.tab_strip_height),
            );
            let rows = self.session_map.rows();
            let active = self.session_map.active_session_id();
            let selected = self.session_map.selected_session_id();
            let state = TabStripState {
                active,
                selected,
                in_flight: self.focus.is_in_flight(),
            };
            let output = tab_strip_ui(ui, strip_rect, layout, rows, &state);
            if let Some(session_id) = output.keyboard_selected {
                self.session_map.select(&session_id);
            }
            if let Some(session_id) = output.clicked {
                self.activate_session_tab(&session_id);
            }
            top += layout.tab_strip_height;

            if let Some(error) = focus_error.as_deref() {
                let error_rect = egui::Rect::from_min_size(
                    egui::pos2(card_rect.left(), top),
                    egui::vec2(card_rect.width(), 24.0),
                );
                if focus_error_strip_ui(ui, error_rect, error, layout) {
                    self.dismiss_focus_error();
                }
                top += 24.0;
            }

            if let Some(row) = self.focused_tab_row() {
                let path_rect = egui::Rect::from_min_size(
                    egui::pos2(card_rect.left(), top),
                    egui::vec2(card_rect.width(), if layout.compact { 20.0 } else { 22.0 }),
                );
                self.terminal_path_line(ui, path_rect, row, layout, chrome_fill);
                top += path_rect.height();
            }
        }

        let mut file_clicked = false;
        if preview_loaded {
            let slot_height = layout.button_height + 4.0;
            let slot_rect = egui::Rect::from_min_size(
                egui::pos2(card_rect.left(), top),
                egui::vec2(card_rect.width(), slot_height),
            );
            if let Some(fill) = chrome_fill {
                ui.painter().rect_filled(slot_rect, 0.0, fill);
            }
            ui.painter().line_segment(
                [
                    egui::pos2(slot_rect.left(), slot_rect.bottom() - 0.5),
                    egui::pos2(slot_rect.right(), slot_rect.bottom() - 0.5),
                ],
                Stroke::new(1.0, theme::BORDER_HEADER),
            );
            let mut slot_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(slot_rect.shrink2(egui::vec2(layout.header_padding_x, 0.0)))
                    .layout(egui::Layout::right_to_left(Align::Center)),
            );
            let action = theme::ghost_action(
                &mut slot_ui,
                left_surface_action_id(self.pane_id(), "file"),
                "File",
                false,
                layout.button_height,
                chrome_fill.unwrap_or(Color32::TRANSPARENT),
            );
            action
                .response
                .on_hover_text("Show the loaded file preview (preview stays open)");
            file_clicked = action.activated;
            top += slot_height;
        }

        let body_rect = egui::Rect::from_min_max(egui::pos2(card_rect.left(), top), card_rect.max);
        let mut body_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(body_rect)
                .id_salt(("made-terminal-body", self.pane_id())),
        );
        let terminal_error = self.terminal_error.clone();
        let mut terminal = self.terminal.take();
        let mut output = TerminalUiOutput::default();
        if self.is_remote()
            && (self.remote_needs_consent()
                || self.remote_needs_update()
                || self.remote_needs_host_approval()
                || self.remote_auth().is_some()
                || self.remote_can_reconnect())
        {
            remote_open_card(&mut body_ui, layout, self);
        } else if let Some(terminal) = terminal.as_mut() {
            let produced = terminal.ui(
                &mut body_ui,
                &self.platform,
                &mut self.metrics,
                self.diag.log_mut(),
            );
            output.restart_requested = produced.restart_requested;
            output.input_sent = produced.input_sent;
            output.notice = produced.notice;
            output.focus_exit_requested = produced.focus_exit_requested;
        } else if self.is_remote() {
            let detail = self.remote_detail().unwrap_or(
                "Remote terminal input is paused until SSH attaches. No local shell was started.",
            );
            terminal_unavailable_card(&mut body_ui, layout, detail, false);
        } else {
            terminal_unavailable_card(
                &mut body_ui,
                layout,
                terminal_error
                    .as_deref()
                    .unwrap_or("The terminal could not be started."),
                true,
            );
        }
        self.terminal = terminal;

        if output.focus_exit_requested {
            let target = ui
                .ctx()
                .data(|data| {
                    data.get_temp::<egui::Id>(egui::Id::unique((
                        "made-terminal-exit-target",
                        self.pane_id(),
                    )))
                })
                .unwrap_or_else(|| top_action_id("open-folder"));
            if let Some(terminal_id) = self
                .terminal
                .as_ref()
                .and_then(|terminal| terminal.focus_id())
            {
                ui.ctx().data_mut(|data| {
                    data.remove::<u8>(egui::Id::unique((
                        "made-terminal-focus-boost",
                        self.pane_id(),
                    )));
                });
                ui.ctx().memory_mut(|memory| {
                    memory.surrender_focus(terminal_id);
                    memory.request_focus(target);
                });
            }
        }

        ui.painter().rect_stroke(
            card_rect,
            CornerRadius::same(layout.card_radius),
            Stroke::new(1.0, theme::BORDER),
            StrokeKind::Inside,
        );

        if output.input_sent {
            self.last_activity = std::time::Instant::now();
        }
        if let Some(notice) = output.notice {
            self.set_notice(notice);
        }
        if output.restart_requested {
            if let Some(terminal) = self.terminal.as_mut() {
                let label = self.terminal_target.program_label();
                match terminal.relaunch(&self.platform, self.diag.log_mut()) {
                    Ok(()) => self.set_notice(format!("{label} restarted.")),
                    Err(error) => {
                        self.diag.log_mut().record(
                            crate::diagnostics::LogLevel::Error,
                            format!(
                                "{} relaunch failed: {error}",
                                self.terminal_target.log_label()
                            ),
                        );
                        self.set_notice(format!("{label} relaunch failed: {error}"));
                    }
                }
            }
        }

        file_clicked
    }

    fn activate_session_tab(&mut self, session_id: &str) {
        let active = self.session_map.active_session_id().map(str::to_owned);
        self.session_map.select(session_id);
        if active.as_deref() != Some(session_id) {
            self.focus_session(session_id);
        }
    }

    fn focused_tab_row(&self) -> Option<&SessionRow> {
        self.session_map.selected().or_else(|| {
            self.session_map
                .active_session_id()
                .and_then(|id| self.session_map.row(id))
        })
    }

    fn terminal_path_line(
        &self,
        ui: &mut Ui,
        rect: egui::Rect,
        row: &SessionRow,
        layout: &Layout,
        fill: Option<Color32>,
    ) {
        if let Some(fill) = fill {
            ui.painter().rect_filled(rect, 0.0, fill);
        }
        ui.painter().line_segment(
            [
                egui::pos2(rect.left(), rect.bottom() - 0.5),
                egui::pos2(rect.right(), rect.bottom() - 0.5),
            ],
            Stroke::new(1.0, theme::BORDER_HEADER),
        );
        let full = row
            .directory
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "path unavailable".to_owned());
        let unavailable = row.directory.is_none();
        let tone = if unavailable {
            theme::WARNING
        } else {
            theme::TEXT_TERTIARY
        };
        let text = if unavailable {
            "! path unavailable".to_owned()
        } else {
            format!(
                "▸ {}",
                truncate_path(&sanitize_display(&full, 900), layout.path_line_chars)
            )
        };
        ui.painter().text(
            egui::pos2(rect.left() + layout.header_padding_x, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::monospace(10.5),
            tone,
        );
        ui.interact(
            rect,
            egui::Id::unique(("made-terminal-path", self.pane_id(), &row.session_id)),
            Sense::hover(),
        )
        .on_hover_text(sanitize_display(&full, 900));
    }

    fn toast(&mut self, ctx: &egui::Context, layout: &Layout) {
        let Some(notice) = self.notice().map(str::to_owned) else {
            return;
        };
        egui::Area::new(egui::Id::unique(("made-toast", self.pane_id())))
            .anchor(
                Align2::RIGHT_BOTTOM,
                egui::vec2(-layout.gutter, layout.toast_bottom_offset()),
            )
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_max_width(if layout.compact { 300.0 } else { 360.0 });
                egui::Frame::new()
                    .fill(theme::BG_ELEVATED)
                    .stroke(Stroke::new(1.0, theme::BORDER_CONTROL))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .shadow(theme::SHADOW_TOAST)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(sanitize_display(&notice, 240))
                                .font(FontId::proportional(11.0))
                                .color(theme::TEXT_SECONDARY),
                        );
                    });
            });
    }

    fn diagnostics_signature(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.pane_id().hash(&mut hasher);
        self.diag.log().revision().hash(&mut hasher);
        self.integration.status().label().hash(&mut hasher);
        self.integration.reason().code.code().hash(&mut hasher);
        self.integration.reason().detail.hash(&mut hasher);
        self.integration
            .connection()
            .map(|connection| connection.connection_id)
            .hash(&mut hasher);
        self.integration.last_revision().hash(&mut hasher);
        self.integration.active_session_id().hash(&mut hasher);
        self.integration.active_session_title().hash(&mut hasher);
        self.shell_cwd().hash(&mut hasher);
        self.root_source().hash(&mut hasher);
        if let Some(view) = self
            .follow
            .as_ref()
            .and_then(|follow| follow.view_directory())
        {
            view.directory().hash(&mut hasher);
            view.available.hash(&mut hasher);
            view.reason.hash(&mut hasher);
        }
        self.explorer_root().hash(&mut hasher);
        if let Some(worktree) = self.worktree() {
            worktree.status.label().hash(&mut hasher);
            worktree.branch.hash(&mut hasher);
            worktree.worktree_label.hash(&mut hasher);
            worktree.is_linked_worktree.hash(&mut hasher);
        }
        self.session_map.len().hash(&mut hasher);
        self.session_map.active_session_id().hash(&mut hasher);
        if let Some(bridge) = self.bridge() {
            bridge.current_connection_id().hash(&mut hasher);
            bridge.dropped_events().hash(&mut hasher);
            bridge.coalesced_states().hash(&mut hasher);
        }
        if let Some(explorer) = self.explorer.as_ref() {
            explorer.has_watcher().hash(&mut hasher);
            explorer.watcher_loading().hash(&mut hasher);
            explorer.watcher_error().hash(&mut hasher);
        }
        for (key, report) in self.row_reports() {
            key.hash(&mut hasher);
            report.pill.label().hash(&mut hasher);
            report.sentence.hash(&mut hasher);
        }
        if let Some(report) = self.remote_opencode_diagnostics() {
            report.status.label().hash(&mut hasher);
            report.code.hash(&mut hasher);
            report.reason.hash(&mut hasher);
        }
        hasher.finish()
    }

    fn diagnostics_cache(&self, ctx: &egui::Context) -> Arc<DiagnosticsUiCache> {
        let signature = self.diagnostics_signature();
        let id = diagnostics_cache_id(self.pane_id());
        if let Some(cache) = ctx.data(|data| data.get_temp::<Arc<DiagnosticsUiCache>>(id)) {
            if cache.signature == signature {
                return cache;
            }
        }
        // `diagnostics_snapshot` already attaches the selected workspace
        // context through `workspace_diagnostics_context`.
        let snapshot = self.diagnostics_snapshot();
        let bridge_stats = self
            .bridge()
            .map(|bridge| {
                format!(
                    "connection={:?} dropped={} coalesced={}",
                    bridge.current_connection_id(),
                    bridge.dropped_events(),
                    bridge.coalesced_states()
                )
            })
            .unwrap_or_else(|| "bridge unavailable".to_owned());
        let log_path = self.sanitized_log_path();
        let copy = sanitize_copy_text(&snapshot.to_text(), self.token(), 1024 * 1024);
        let cache = Arc::new(DiagnosticsUiCache {
            signature,
            snapshot,
            bridge_stats,
            log_path,
            copy,
        });
        ctx.data_mut(|data| data.insert_temp(id, cache.clone()));
        cache
    }

    /// Previous-log existence, statting at most once per second while open.
    fn diagnostics_previous_log_exists(&self, ctx: &egui::Context) -> bool {
        let bucket = diagnostics_previous_log_bucket(ctx);
        let id = egui::Id::unique(("made-diagnostics-previous-log-exists", self.pane_id()));
        if let Some((cached_bucket, exists)) = ctx.data(|data| data.get_temp::<(u64, bool)>(id)) {
            if cached_bucket == bucket {
                return exists;
            }
        }
        let exists = self.diag.previous_log_exists();
        ctx.data_mut(|data| data.insert_temp(id, (bucket, exists)));
        exists
    }

    fn diagnostics_metrics_cache(&self, ctx: &egui::Context) -> Arc<DiagnosticsMetricsCache> {
        let bucket = diagnostics_metrics_bucket(ctx);
        let id = diagnostics_metrics_cache_id(self.pane_id());
        if let Some(cache) = ctx.data(|data| data.get_temp::<Arc<DiagnosticsMetricsCache>>(id)) {
            if cache.bucket == bucket {
                return cache;
            }
        }
        let cache = Arc::new(DiagnosticsMetricsCache {
            bucket,
            text: self.metrics.report().to_text(),
        });
        ctx.data_mut(|data| data.insert_temp(id, cache.clone()));
        cache
    }

    fn about_window(
        &mut self,
        ctx: &egui::Context,
        show_about: &mut bool,
        updates: &mut UpdatePanel,
        layout: &Layout,
    ) {
        if !*show_about {
            if ctx.data(|data| data.get_temp::<bool>(about_open_id()).unwrap_or(false)) {
                ctx.data_mut(|data| data.insert_temp(about_open_id(), false));
            }
            return;
        }
        let was_open = ctx.data(|data| data.get_temp::<bool>(about_open_id()).unwrap_or(false));
        if !was_open {
            self.about_diagnostics_visible = false;
            self.about_open_generation = self.about_open_generation.wrapping_add(1);
            ctx.data_mut(|data| {
                data.remove::<Arc<DiagnosticsUiCache>>(diagnostics_cache_id(self.pane_id()));
            });
        }
        let cache = self.diagnostics_cache(ctx);
        let previous_log_exists = self.diagnostics_previous_log_exists(ctx);
        let metrics_cache = self.diagnostics_metrics_cache(ctx);
        let snapshot = &cache.snapshot;
        let performance = metrics_cache.text.as_str();
        let bridge_stats = cache.bridge_stats.as_str();
        let heartbeat_age =
            heartbeat_age(self.integration.last_heartbeat(), std::time::Instant::now());
        let config_failure = self.config_failure.clone();
        let log_write_issue = self.log_write_issue.clone();
        let log_path = cache.log_path.as_str();
        let pane_id = self.pane_id().to_owned();
        let reason_full = snapshot.reason.clone();
        let reason = sanitize_display(&reason_full, 320);
        let content = ctx.content_rect();
        let width = (content.width() - 64.0).clamp(440.0, 680.0);
        let mut open = *show_about;
        let mut copy_requested = false;
        let mut previous_copy_requested = false;
        let mut detail_visible = self.about_diagnostics_visible;
        // The compact card fits the Updates section and its confirmation.
        let collapsed_height = 220.0 + updates_section_height(updates);
        let expanded_height = (content.height() - 48.0).max(360.0);
        let reveal_id = egui::Id::unique((
            "made-about-diagnostics-reveal",
            self.pane_id(),
            self.about_open_generation,
        ));
        let reveal = ctx.animate_bool_with_time_and_easing(
            reveal_id,
            self.about_diagnostics_visible,
            ABOUT_REVEAL_TIME,
            egui::emath::easing::quadratic_out,
        );
        let window_height = collapsed_height + (expanded_height - collapsed_height) * reveal;

        egui::Window::new("About")
            .id(egui::Id::unique(("made-about-window", self.pane_id())))
            .open(&mut open)
            .resizable(false)
            .movable(false)
            .anchor(Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .fixed_size(egui::vec2(width, window_height))
            .collapsible(false)
            .frame(theme::floating_frame())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    theme::wordmark_brand(ui);
                    ui.label(theme::mono(
                        format!("{APP_NAME} {APP_VERSION}"),
                        10.5,
                        theme::TEXT_TERTIARY,
                    ));
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        let label = if detail_visible {
                            "Hide diagnostics"
                        } else {
                            "Show diagnostics"
                        };
                        let toggle = if detail_visible {
                            theme::primary_action(
                                ui,
                                about_diagnostics_toggle_id(&pane_id),
                                label,
                                layout.button_height,
                                true,
                                theme::BG_CARD,
                            )
                        } else {
                            theme::secondary_action(
                                ui,
                                about_diagnostics_toggle_id(&pane_id),
                                label,
                                layout.button_height,
                                true,
                                theme::BG_CARD,
                            )
                        };
                        toggle.response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::Button, true, label)
                        });
                        if toggle.activated {
                            detail_visible = !detail_visible;
                        }
                    });
                });
                ui.label(theme::body("Minimal Agentic Development Environment"));
                ui.label(theme::micro(APP_DESCRIPTION));
                ui.add_space(6.0);
                updates_section(ui, updates, layout);
                ui.add_space(6.0);
                if reveal <= 0.001 {
                    let remaining = ui.available_height().max(0.0);
                    ui.add_space(remaining);
                    return;
                }
                ui.multiply_opacity(reveal);
                ui.separator();
                ui.horizontal(|ui| {
                    status_pill_ui(ui, snapshot.status, &reason_full, layout);
                    ui.label(theme::micro(&reason)).on_hover_text(&reason_full);
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        let copy_action = theme::secondary_action(
                            ui,
                            diagnostics_copy_id(self.pane_id()),
                            "Copy diagnostics",
                            layout.button_height,
                            true,
                            theme::BG_CARD,
                        );
                        copy_action
                            .response
                            .on_hover_text("Copy the redacted diagnostics report");
                        if copy_action.activated {
                            copy_requested = true;
                        }
                    });
                });
                for (pill, sentence) in about_integration_rows(snapshot) {
                    ui.horizontal(|ui| {
                        grok_status_pill_ui(ui, pill, sentence, layout);
                        ui.label(theme::micro(sentence)).on_hover_text(sentence);
                    });
                }
                if let Some(failure) = &config_failure {
                    notice_strip(
                        ui,
                        layout,
                        theme::WARNING_TINT,
                        theme::WARNING,
                        &format!("configuration: {}", sanitize_display(failure, 400)),
                        &sanitize_display(failure, 700),
                    );
                }
                if let Some(issue) = &log_write_issue {
                    let (fill, tone) = log_issue_tone(issue.severity);
                    let message = crate::diagnostics::redact_hex_secrets(&sanitize_display(
                        &issue.message,
                        700,
                    ));
                    notice_strip(
                        ui,
                        layout,
                        fill,
                        tone,
                        &format!("log: {}", sanitize_display(&message, 400)),
                        &message,
                    );
                }

                let detail_height = (ui.available_height() - 90.0).max(48.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(detail_height)
                    .show(ui, |ui| {
                        diagnostics_section(ui, "WORKSPACE");
                        diagnostics_rows(
                            ui,
                            "made-diag-workspace",
                            &[
                                (
                                    "Identity",
                                    snapshot
                                        .workspace_identity
                                        .clone()
                                        .unwrap_or_else(|| "unavailable".to_owned()),
                                ),
                                (
                                    "Label",
                                    snapshot
                                        .workspace_label
                                        .clone()
                                        .unwrap_or_else(|| "unavailable".to_owned()),
                                ),
                                (
                                    "Path",
                                    snapshot
                                        .workspace_path
                                        .as_ref()
                                        .map(|path| path.display().to_string())
                                        .unwrap_or_else(|| "unavailable".to_owned()),
                                ),
                                (
                                    "Shell cwd",
                                    snapshot
                                        .shell_cwd
                                        .as_ref()
                                        .map(|path| path.display().to_string())
                                        .unwrap_or_else(|| "unknown".to_owned()),
                                ),
                                ("Cwd reporting", cwd_reporting_text(snapshot)),
                                (
                                    "Effective root",
                                    snapshot
                                        .active_root
                                        .as_ref()
                                        .map(|path| path.display().to_string())
                                        .unwrap_or_else(|| "none".to_owned()),
                                ),
                                (
                                    "Root source",
                                    snapshot
                                        .root_source
                                        .clone()
                                        .unwrap_or_else(|| "unknown".to_owned()),
                                ),
                                (
                                    "View directory",
                                    match (
                                        &snapshot.view_directory,
                                        snapshot.view_directory_available,
                                    ) {
                                        (Some(path), Some(false)) => format!(
                                            "{} (unavailable{})",
                                            path.display(),
                                            snapshot
                                                .view_directory_reason
                                                .as_ref()
                                                .map(|reason| format!(": {reason}"))
                                                .unwrap_or_default()
                                        ),
                                        (Some(path), _) => path.display().to_string(),
                                        (None, _) => "none".to_owned(),
                                    },
                                ),
                                (
                                    "Watcher",
                                    snapshot
                                        .watcher
                                        .clone()
                                        .unwrap_or_else(|| "unavailable".to_owned()),
                                ),
                            ],
                        );

                        diagnostics_section(ui, "CONNECTION");
                        diagnostics_rows(
                            ui,
                            "made-diag-connection",
                            &[
                                ("Reason code", snapshot.reason_code.to_owned()),
                                ("Bridge", bridge_stats.to_owned()),
                                (
                                    "Protocol",
                                    snapshot
                                        .protocol_version
                                        .map(|version| version.to_string())
                                        .unwrap_or_else(|| "none".to_owned()),
                                ),
                                (
                                    "Last revision",
                                    snapshot
                                        .last_revision
                                        .map(|revision| revision.to_string())
                                        .unwrap_or_else(|| "none".to_owned()),
                                ),
                                (
                                    "Heartbeat/state age",
                                    heartbeat_age
                                        .map(|age| format!("{:.3}s", age.as_secs_f64()))
                                        .unwrap_or_else(|| "none".to_owned()),
                                ),
                                (
                                    "Tabs enabled",
                                    if snapshot.tabs_enabled { "yes" } else { "no" }.to_owned(),
                                ),
                                (
                                    "Token fingerprint",
                                    snapshot
                                        .token_fingerprint
                                        .clone()
                                        .unwrap_or_else(|| "none".to_owned()),
                                ),
                            ],
                        );

                        diagnostics_section(ui, "SESSION");
                        let mut session_rows = vec![
                            (
                                "Active session",
                                snapshot
                                    .active_session_id
                                    .clone()
                                    .unwrap_or_else(|| "none".to_owned()),
                            ),
                            (
                                "Active title",
                                snapshot
                                    .active_session_title
                                    .clone()
                                    .unwrap_or_else(|| "none".to_owned()),
                            ),
                        ];
                        if let Some(subpath) = &snapshot.active_subpath {
                            session_rows.push(("Subpath", subpath.clone()));
                        }
                        session_rows.push((
                            "Active root",
                            snapshot
                                .active_root
                                .as_ref()
                                .map(|root| root.display().to_string())
                                .unwrap_or_else(|| "none".to_owned()),
                        ));
                        diagnostics_rows(ui, "made-diag-session", &session_rows);

                        diagnostics_section(ui, "WORKTREE");
                        let mut worktree_rows = vec![(
                            "Status",
                            snapshot
                                .worktree_status
                                .map(|status| status.label().to_owned())
                                .unwrap_or_else(|| "none".to_owned()),
                        )];
                        if let Some(branch) = &snapshot.worktree_branch {
                            worktree_rows.push(("Branch", branch.clone()));
                        }
                        if let Some(label) = &snapshot.worktree_label {
                            worktree_rows.push(("Label", label.clone()));
                        }
                        if snapshot.worktree_is_linked {
                            worktree_rows.push(("Worktree kind", "linked worktree".to_owned()));
                        }
                        if let Some(reason) = &snapshot.worktree_reason {
                            worktree_rows.push(("Reason", reason.clone()));
                        }
                        diagnostics_rows(ui, "made-diag-worktree", &worktree_rows);

                        diagnostics_section(ui, "VERSIONS");
                        diagnostics_rows(
                            ui,
                            "made-diag-versions",
                            &[
                                (
                                    "Plugin",
                                    snapshot
                                        .plugin_version
                                        .clone()
                                        .unwrap_or_else(|| "none".to_owned()),
                                ),
                                (
                                    "OpenCode",
                                    snapshot
                                        .opencode_version
                                        .clone()
                                        .unwrap_or_else(|| "none".to_owned()),
                                ),
                                ("Made", snapshot.app_version.clone()),
                                (
                                    "Last command-line apply",
                                    snapshot
                                        .update_apply
                                        .clone()
                                        .unwrap_or_else(|| "none".to_owned()),
                                ),
                            ],
                        );

                        diagnostics_section(ui, "METRICS");
                        egui::Frame::new()
                            .fill(theme::BG_DEEP)
                            .corner_radius(CornerRadius::same(6))
                            .inner_margin(egui::Margin::same(8))
                            .show(ui, |ui| {
                                ui.label(theme::mono(performance, 10.5, theme::TEXT_SECONDARY));
                            });

                        diagnostics_section(ui, "LOG");
                        egui::Frame::new()
                            .fill(theme::BG_DEEP)
                            .corner_radius(CornerRadius::same(6))
                            .inner_margin(egui::Margin::same(8))
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .id_salt("made-diagnostics-log")
                                    .max_height(160.0)
                                    .show(ui, |ui| {
                                        for entry in &snapshot.entries {
                                            let (tone, _) = log_level_tone(entry.level);
                                            let message = sanitize_display(
                                                &format!(
                                                    "{}{}{}",
                                                    entry.message,
                                                    entry
                                                        .reason
                                                        .as_ref()
                                                        .map(|reason| format!(" reason={reason}"))
                                                        .unwrap_or_default(),
                                                    entry
                                                        .session_id
                                                        .as_ref()
                                                        .map(|session| format!(
                                                            " session={session}"
                                                        ))
                                                        .unwrap_or_default(),
                                                ),
                                                2000,
                                            );
                                            let mut job = egui::text::LayoutJob::default();
                                            job.append(
                                                &format!("#{} ", entry.sequence),
                                                0.0,
                                                egui::TextFormat {
                                                    font_id: FontId::monospace(10.0),
                                                    color: theme::TEXT_TERTIARY,
                                                    ..Default::default()
                                                },
                                            );
                                            job.append(
                                                &format!("[{}] ", entry.level.label()),
                                                0.0,
                                                egui::TextFormat {
                                                    font_id: FontId::monospace(10.0),
                                                    color: tone,
                                                    ..Default::default()
                                                },
                                            );
                                            job.append(
                                                &message,
                                                0.0,
                                                egui::TextFormat {
                                                    font_id: FontId::monospace(10.5),
                                                    color: theme::TEXT_SECONDARY,
                                                    ..Default::default()
                                                },
                                            );
                                            ui.add(egui::Label::new(job).selectable(false));
                                        }
                                    });
                            });
                    });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(theme::mono("Previous log", 10.0, theme::TEXT_TERTIARY));
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        let exists = previous_log_exists;
                        let action = theme::quiet_secondary_action(
                            ui,
                            diagnostics_previous_log_copy_id(&pane_id),
                            "Copy previous log path",
                            layout.button_height,
                            exists,
                            theme::BG_DEEP,
                        );
                        action.response.on_hover_text("Copy the previous log path");
                        if action.activated && snapshot.previous_log_location.is_some() {
                            previous_copy_requested = true;
                        }
                        let path = snapshot
                            .previous_log_location
                            .as_deref()
                            .unwrap_or("unavailable");
                        let response = ui.add(
                            egui::Label::new(theme::mono(
                                sanitize_display(path, 512),
                                10.0,
                                theme::TEXT_TERTIARY,
                            ))
                            .truncate(),
                        );
                        ui.interact(
                            response.rect,
                            diagnostics_previous_log_path_id(&pane_id),
                            Sense::hover(),
                        )
                        .on_hover_text(sanitize_display(path, 2048));
                    });
                });
                if !previous_log_exists {
                    ui.label(theme::mono("not yet rotated", 10.0, theme::TEXT_TERTIARY));
                }

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(theme::mono(log_path, 10.0, theme::TEXT_TERTIARY));
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if theme::ghost_button(ui, "Copy log path", false, layout.button_height)
                            .clicked()
                        {
                            ctx.copy_text(log_path.to_owned());
                        }
                    });
                });
                let remaining = ui.available_height().max(0.0);
                ui.add_space(remaining);
            });

        if !open && updates.confirming {
            updates.actions.push(UpdateAction::Dismiss);
        }
        *show_about = open;
        self.about_diagnostics_visible = detail_visible;
        if previous_copy_requested {
            let mut clipboard = UiClipboard { ctx: ctx.clone() };
            if let Err(error) = self.diag.copy_previous_log_path(&mut clipboard) {
                eprintln!("made: could not copy previous log path: {error}");
            }
        }
        if copy_requested {
            ctx.data_mut(|data| {
                data.remove::<Arc<DiagnosticsUiCache>>(diagnostics_cache_id(self.pane_id()));
            });
            let fresh = self.diagnostics_cache(ctx);
            ctx.copy_text(fresh.copy.clone());
        }
        ctx.data_mut(|data| data.insert_temp(about_open_id(), open));
    }
}

/// Stable id of About's `Check for updates` button.
pub fn about_update_check_id() -> egui::Id {
    egui::Id::unique("made-about-update-check")
}

/// Stable id of About's `Check for updates automatically` toggle.
pub fn about_update_automatic_id() -> egui::Id {
    egui::Id::unique("made-about-update-automatic")
}

/// Stable id of About's `Restart to update` button.
pub fn about_update_restart_id() -> egui::Id {
    egui::Id::unique("made-about-update-restart")
}

/// Stable id of the inline confirmation's `Restart now` button.
pub fn about_update_restart_now_id() -> egui::Id {
    egui::Id::unique("made-about-update-restart-now")
}

/// Stable id of the inline confirmation's `Not now` button.
pub fn about_update_not_now_id() -> egui::Id {
    egui::Id::unique("made-about-update-not-now")
}

/// Wording of the inline restart confirmation.
pub fn restart_confirmation_text(version: &str) -> String {
    format!(
        "Restart into Made {version}? Every open workspace, terminal and running session, \
         including OpenCode, Grok and SSH sessions, will close and will not be restored."
    )
}

fn updates_section_height(updates: &UpdatePanel) -> f32 {
    let mut height = 24.0;
    if updates.controls {
        height += 34.0;
    }
    if updates.confirming {
        height += 74.0;
    }
    height
}

/// The About Updates section: one status line, and for an installed build
/// the check button, the automatic-check toggle, the restart action and its
/// inline confirmation. A non-installed build shows only its status line.
fn updates_section(ui: &mut Ui, updates: &mut UpdatePanel, layout: &Layout) {
    ui.horizontal(|ui| {
        ui.label(theme::mono("UPDATES", 10.0, theme::TEXT_TERTIARY));
        ui.label(theme::micro(
            updates.notice.as_deref().unwrap_or(&updates.status_line),
        ));
    });
    if !updates.controls {
        return;
    }
    ui.horizontal(|ui| {
        let check = theme::secondary_action(
            ui,
            about_update_check_id(),
            "Check for updates",
            layout.button_height,
            !updates.busy,
            theme::BG_CARD,
        );
        check.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, !updates.busy, "Check for updates")
        });
        if check.activated && !updates.busy {
            updates.actions.push(UpdateAction::CheckNow);
        }
        let toggle = theme::toggle_action(
            ui,
            about_update_automatic_id(),
            "Check for updates automatically",
            updates.automatic,
            layout.button_height,
            theme::BG_CARD,
        );
        if toggle.activated {
            updates
                .actions
                .push(UpdateAction::SetAutomatic(!updates.automatic));
        }
        if updates.ready.is_some() && !updates.confirming {
            let restart = theme::primary_action(
                ui,
                about_update_restart_id(),
                "Restart to update",
                layout.button_height,
                true,
                theme::BG_CARD,
            );
            restart.response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, "Restart to update")
            });
            if restart.activated {
                updates.actions.push(UpdateAction::RequestRestart);
            }
        }
    });
    if let (true, Some(version)) = (updates.confirming, updates.ready.clone()) {
        egui::Frame::NONE
            .fill(theme::BG_ELEVATED)
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(theme::micro(restart_confirmation_text(&version)));
                ui.horizontal(|ui| {
                    let now = theme::primary_action(
                        ui,
                        about_update_restart_now_id(),
                        "Restart now",
                        layout.button_height,
                        true,
                        theme::BG_ELEVATED,
                    );
                    now.response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, "Restart now")
                    });
                    if now.activated {
                        updates.actions.push(UpdateAction::RestartNow);
                    }
                    let later = theme::secondary_action(
                        ui,
                        about_update_not_now_id(),
                        "Not now",
                        layout.button_height,
                        true,
                        theme::BG_ELEVATED,
                    );
                    later.response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, "Not now")
                    });
                    if later.activated {
                        updates.actions.push(UpdateAction::Dismiss);
                    }
                });
            });
    }
}

fn cwd_reporting_text(snapshot: &crate::diagnostics::DiagnosticsSnapshot) -> String {
    let state = match snapshot.cwd_supported {
        Some(true) => "supported",
        Some(false) => "unsupported",
        None => "unknown",
    };
    match &snapshot.cwd_reason {
        Some(reason) => format!("{state} — {reason}"),
        None => state.to_owned(),
    }
}

// ── Session tab strip ───────────────────────────────────────────────────────

struct TabStripState<'a> {
    active: Option<&'a str>,
    selected: Option<&'a str>,
    in_flight: bool,
}

#[derive(Debug, Default)]
struct TabStripOutput {
    clicked: Option<String>,
    keyboard_selected: Option<String>,
    #[cfg(test)]
    overflow: bool,
}

/// Bound a tab candidate set to the configured maximum while keeping the
/// active tab present.
fn candidate_tab_indices(count: usize, active: Option<usize>, max: usize) -> Vec<usize> {
    if count <= max {
        return (0..count).collect();
    }
    let max = max.max(1);
    let mut candidates: Vec<usize> = (0..max.saturating_sub(1)).collect();
    if let Some(active) = active {
        if !candidates.contains(&active) {
            candidates.push(active);
        }
    }
    let mut next = 0;
    while candidates.len() < max && next < count {
        if !candidates.contains(&next) {
            candidates.push(next);
        }
        next += 1;
    }
    candidates.sort_unstable();
    candidates
}

fn tab_strip_ui(
    ui: &mut Ui,
    strip: egui::Rect,
    layout: &Layout,
    rows: &[SessionRow],
    state: &TabStripState<'_>,
) -> TabStripOutput {
    ui.painter().rect_filled(
        strip,
        CornerRadius {
            nw: layout.card_radius,
            ne: layout.card_radius,
            sw: 0,
            se: 0,
        },
        theme::BG_HEADER,
    );
    ui.painter().line_segment(
        [
            egui::pos2(strip.left(), strip.bottom() - 0.5),
            egui::pos2(strip.right(), strip.bottom() - 0.5),
        ],
        Stroke::new(1.0, theme::BORDER_HEADER),
    );

    let strip_padding = if layout.compact { 3.0 } else { 4.0 };
    let tabs_height = layout.session_tab_height;
    let tabs_top = strip.top() + (layout.tab_strip_height - tabs_height) / 2.0;
    let inflight_width = if state.in_flight { 18.0 } else { 0.0 };
    let active_index = state
        .active
        .and_then(|active| rows.iter().position(|row| row.session_id == active));
    let candidates = candidate_tab_indices(rows.len(), active_index, layout.max_tabs);
    let candidate_widths: Vec<f32> = candidates
        .iter()
        .map(|index| tab_width(&rows[*index], layout))
        .collect();
    let overflow_width = if candidates.len() < rows.len() {
        30.0
    } else {
        0.0
    };
    let tabs_right =
        (strip.right() - overflow_width - inflight_width - 8.0).max(strip.left() + 40.0);
    let tabs_available = (tabs_right - strip.left() - 2.0 * strip_padding).max(40.0);
    let active_candidate = active_index
        .and_then(|active| candidates.iter().position(|candidate| *candidate == active));
    let (visible_positions, _width_overflow) = bounded_tab_indices(
        &candidate_widths,
        active_candidate,
        tabs_available,
        4.0,
        None,
    );
    let visible: Vec<usize> = visible_positions
        .iter()
        .filter_map(|position| candidates.get(*position).copied())
        .collect();
    let overflow = visible.len() < rows.len();

    if rows.is_empty() {
        let mut empty = ui.new_child(
            UiBuilder::new()
                .max_rect(
                    egui::Rect::from_min_size(
                        egui::pos2(strip.left() + 10.0, strip.top()),
                        egui::vec2(
                            (tabs_right - strip.left() - 14.0).max(40.0),
                            layout.tab_strip_height,
                        ),
                    )
                    .shrink2(egui::vec2(0.0, 2.0)),
                )
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        if layout.compact {
            empty
                .label(theme::micro("No open sessions"))
                .on_hover_text("Open OpenCode in the terminal.");
        } else {
            empty.label(theme::micro("No open sessions"));
            empty.label(theme::micro("Open OpenCode in the terminal."));
        }
    } else {
        let tabs_rect = egui::Rect::from_min_max(
            egui::pos2(strip.left() + strip_padding, tabs_top),
            egui::pos2(tabs_right, tabs_top + tabs_height),
        );
        let mut tabs_ui = ui.new_child(UiBuilder::new().max_rect(tabs_rect));
        let mut responses: Vec<(String, egui::Response)> = Vec::new();
        let mut clicked = None;
        tabs_ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for index in &visible {
                let Some(row) = rows.get(*index) else {
                    continue;
                };
                let tab_width = tab_width(row, layout);
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(tab_width, tabs_height), Sense::click());
                let is_active = state.active == Some(row.session_id.as_str());
                let is_selected = state.selected == Some(row.session_id.as_str());
                let selected = is_active || is_selected;
                let fill = if selected {
                    theme::BG_SELECTED
                } else if response.hovered() {
                    theme::BG_HOVER
                } else {
                    Color32::TRANSPARENT
                };
                if fill != Color32::TRANSPARENT {
                    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
                }

                let mut reserve = 8.0;
                if row.busy {
                    reserve += 14.0;
                }
                if row.attention {
                    reserve += 14.0;
                }
                if row.unread || row.prompt_pulse > 0 {
                    reserve += if row.unread && row.prompt_pulse == 0 {
                        10.0
                    } else {
                        22.0
                    };
                }
                if row.renaming {
                    reserve += 10.0;
                }
                let title = sanitize_display(&row.display_title(layout.title_chars), 200);
                let title_width = (rect.width() - 18.0 - reserve).max(16.0);
                let color = if selected || response.hovered() {
                    theme::TEXT_PRIMARY
                } else {
                    theme::inactive_tab_text()
                };
                let font = FontId::proportional(if selected {
                    theme::TAB_SELECTED_FONT_SIZE
                } else {
                    theme::TAB_FONT_SIZE
                });
                let job = theme::ellipsized_job(title, font, color, title_width);
                let galley = ui.painter().layout_job(job);
                ui.painter().galley(
                    egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
                    galley,
                    color,
                );

                let mut x = rect.right() - 8.0;
                if row.renaming {
                    x -= 8.0;
                    ui.painter().text(
                        egui::pos2(x, rect.center().y),
                        Align2::CENTER_CENTER,
                        "*",
                        FontId::proportional(12.0),
                        theme::TEXT_SECONDARY,
                    );
                }
                if row.unread || row.prompt_pulse > 0 {
                    if row.unread && row.prompt_pulse == 0 {
                        x -= 8.0;
                        ui.painter().circle_filled(
                            egui::pos2(x, rect.center().y),
                            3.0,
                            theme::INFO,
                        );
                    } else {
                        x -= 22.0;
                        let badge = egui::Rect::from_min_size(
                            egui::pos2(x, rect.center().y - 7.0),
                            egui::vec2(18.0, 14.0),
                        );
                        ui.painter()
                            .rect_filled(badge, CornerRadius::same(7), theme::INFO_TINT);
                        ui.painter().text(
                            badge.center(),
                            Align2::CENTER_CENTER,
                            format!("{}", row.prompt_pulse.min(99)),
                            FontId::monospace(10.0),
                            theme::INFO,
                        );
                    }
                }
                if row.attention {
                    x -= 14.0;
                    let badge = egui::Rect::from_center_size(
                        egui::pos2(x, rect.center().y),
                        egui::vec2(13.0, 13.0),
                    );
                    ui.painter()
                        .circle_filled(badge.center(), 6.5, theme::WARNING_TINT);
                    ui.painter().circle_stroke(
                        badge.center(),
                        6.5,
                        Stroke::new(1.0, theme::WARNING),
                    );
                    ui.painter().text(
                        badge.center(),
                        Align2::CENTER_CENTER,
                        "!",
                        FontId::proportional(10.0),
                        theme::WARNING,
                    );
                }
                if row.busy {
                    x -= 12.0;
                    ui.put(
                        egui::Rect::from_center_size(
                            egui::pos2(x, rect.center().y),
                            egui::vec2(10.0, 10.0),
                        ),
                        egui::Spinner::new().size(10.0).color(theme::ACCENT),
                    );
                }

                if response.has_focus() {
                    theme::paint_focus_ring(ui, rect, CornerRadius::same(6), theme::BG_HEADER);
                }
                let response = response.on_hover_text(format!(
                    "{}\n{}",
                    sanitize_display(&row.title, 240),
                    sanitize_display(&row.display_path(240), 300)
                ));
                if response.clicked() {
                    response.request_focus();
                    clicked = Some(row.session_id.clone());
                }
                responses.push((row.session_id.clone(), response));
            }
        });

        let mut output = TabStripOutput {
            clicked,
            #[cfg(test)]
            overflow,
            ..Default::default()
        };

        if let Some(index) = responses
            .iter()
            .position(|(_, response)| response.has_focus())
        {
            let events = ui.ctx().input(|input| input.events.clone());
            for event in events {
                if let egui::Event::Key {
                    key, pressed: true, ..
                } = event
                {
                    match key {
                        egui::Key::Enter | egui::Key::Space => {
                            output.clicked = Some(responses[index].0.clone());
                        }
                        egui::Key::ArrowLeft => {
                            let next = (index + responses.len() - 1) % responses.len();
                            responses[next].1.request_focus();
                            responses[next].1.scroll_to_me(Some(Align::Center));
                            output.keyboard_selected = Some(responses[next].0.clone());
                        }
                        egui::Key::ArrowRight => {
                            let next = (index + 1) % responses.len();
                            responses[next].1.request_focus();
                            responses[next].1.scroll_to_me(Some(Align::Center));
                            output.keyboard_selected = Some(responses[next].0.clone());
                        }
                        _ => {}
                    }
                }
            }
        }

        if state.in_flight {
            let spinner_rect = egui::Rect::from_center_size(
                egui::pos2(strip.right() - overflow_width - 10.0, strip.center().y),
                egui::vec2(12.0, 12.0),
            );
            ui.put(
                spinner_rect,
                egui::Spinner::new().size(10.0).color(theme::ACCENT),
            );
        }

        if overflow {
            let overflow_rect = egui::Rect::from_center_size(
                egui::pos2(strip.right() - 16.0, strip.center().y),
                egui::vec2(28.0, layout.button_height.max(24.0)),
            );
            let mut menu_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(overflow_rect)
                    .layout(egui::Layout::left_to_right(Align::Center)),
            );
            let action = theme::ghost_action(
                &mut menu_ui,
                ui.scope_id().with("made-session-overflow"),
                "…",
                false,
                layout.button_height,
                theme::BG_HEADER,
            );
            let response = action.response;
            egui::Popup::menu(&response).show(|ui| {
                ui.set_max_width(420.0);
                for row in rows {
                    let is_active = state.active == Some(row.session_id.as_str());
                    let title = sanitize_display(&row.display_title(64), 64);
                    if ui.selectable_label(is_active, title).clicked() {
                        output.clicked = Some(row.session_id.clone());
                        ui.close();
                    }
                    ui.label(theme::mono(
                        sanitize_display(&row.display_path(120), 140),
                        10.0,
                        theme::TEXT_TERTIARY,
                    ));
                }
            });
            response.on_hover_text("All OpenCode sessions");
        }

        return output;
    }

    TabStripOutput {
        #[cfg(test)]
        overflow,
        ..Default::default()
    }
}

fn tab_width(row: &SessionRow, layout: &Layout) -> f32 {
    let mut width = row.display_title(layout.title_chars).chars().count() as f32 * 6.2 + 42.0;
    if row.busy {
        width += 14.0;
    }
    if row.attention {
        width += 14.0;
    }
    if row.unread || row.prompt_pulse > 0 {
        width += if row.unread && row.prompt_pulse == 0 {
            10.0
        } else {
            22.0
        };
    }
    if row.renaming {
        width += 10.0;
    }
    width.clamp(112.0, if layout.compact { 184.0 } else { 220.0 })
}

fn focus_error_strip_ui(ui: &mut Ui, rect: egui::Rect, error: &str, layout: &Layout) -> bool {
    ui.painter().rect_filled(rect, 0.0, theme::ERROR_TINT);
    ui.painter().rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
        CornerRadius::same(2),
        theme::ERROR,
    );
    let mut dismissed = false;
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(layout.header_padding_x, 0.0)))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    let text = format!("! Focus failed — {}", sanitize_display(error, 260));
    row.label(
        RichText::new(truncate_end(&text, if layout.compact { 46 } else { 120 }))
            .font(FontId::proportional(10.5))
            .color(theme::ERROR),
    );
    row.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
        if theme::ghost_button(ui, "Dismiss", false, layout.button_height).clicked() {
            dismissed = true;
        }
    });
    dismissed
}

fn paint_header(ui: &Ui, rect: egui::Rect, layout: &Layout, top_rounded: bool) {
    let radius = if top_rounded {
        CornerRadius {
            nw: layout.card_radius,
            ne: layout.card_radius,
            sw: 0,
            se: 0,
        }
    } else {
        CornerRadius::ZERO
    };
    ui.painter().rect_filled(rect, radius, theme::BG_HEADER);
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.bottom() - 0.5),
            egui::pos2(rect.right(), rect.bottom() - 0.5),
        ],
        Stroke::new(1.0, theme::BORDER_HEADER),
    );
}

// ── Shared cards ────────────────────────────────────────────────────────────

fn unavailable_card(ui: &mut Ui, layout: &Layout, title: &str, message: &str) {
    let rect = ui.max_rect();
    theme::paint_card(ui.painter(), rect, layout, theme::BG_CARD);
    let header = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(rect.width(), layout.card_header_height),
    );
    paint_header(ui, header, layout, true);
    let mut header_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(header.shrink2(egui::vec2(layout.header_padding_x, 0.0)))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    theme::section_label(&mut header_ui, "EXPLORER");
    theme::paint_card_border(ui, rect, layout);
    let body = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 10.0, header.bottom() + 10.0),
        egui::pos2(rect.right() - 10.0, rect.bottom() - 10.0),
    );
    let mut body_ui = ui.new_child(UiBuilder::new().max_rect(body));
    egui::Frame::new()
        .fill(theme::ERROR_TINT)
        .stroke(Stroke::new(1.0, theme::ERROR))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(10))
        .show(&mut body_ui, |ui| {
            ui.label(
                RichText::new(title)
                    .font(FontId::proportional(13.0))
                    .color(theme::ERROR),
            );
            ui.label(theme::body(sanitize_display(message, 600)));
        });
}

/// The per-server/version Grok plugin install consent card.
///
/// The card exists only where that server's own probe detected grok. Nothing
/// is staged, invoked, or written before an explicit approval; refusal stores
/// nothing; withdrawal clears the recorded consent without uninstalling.
fn grok_install_consent_card(model: &mut crate::app::AppModel, ui: &mut Ui, layout: &Layout) {
    let Some(selected) = model.ssh_selected() else {
        return;
    };
    let detected = model
        .ssh_registry()
        .ok()
        .is_some_and(|registry| registry.can_launch_grok(selected));
    if !detected {
        return;
    }
    let consented = model.remote_grok_consent_granted(selected).unwrap_or(false);
    let running = model.remote_grok_install_running(selected);
    let state = model.remote_grok_install_state(selected);
    let refused = model.remote_grok_install_refused(selected);
    let ready = model.remote_grok_install_ready(selected);
    ui.separator();
    ui.label(theme::body("Grok Build plugin install"));
    if running {
        ui.label(theme::micro(
            "Installing the official Grok Build plugin for this server and version.",
        ));
        return;
    }
    if consented {
        ui.label(theme::micro(format!(
            "Approved for this server and version {}.",
            crate::APP_VERSION
        )));
        match state {
            Some(crate::app::RemoteGrokInstallState::Succeeded) => {
                ui.label(theme::micro(
                    "The plugin is installed. Plugin hooks apply only to Grok sessions started after the install.",
                ));
            }
            Some(crate::app::RemoteGrokInstallState::Failed { reason }) => match reason {
                Some(reason) => {
                    ui.label(theme::micro(sanitize_display(
                        &format!("The last install failed. The server reported: {reason}."),
                        200,
                    )));
                }
                None => {
                    ui.label(theme::micro(
                        "The last install failed. Session follow keeps the remote inventory tier.",
                    ));
                }
            },
            _ => {
                ui.label(theme::micro("The install has not run on this server yet."));
            }
        }
        if !ready {
            ui.label(theme::micro(
                "The private companion is not provisioned or verified in this session. Open this server's remote workspace first; the install stays unavailable until that succeeds.",
            ));
        }
        if !matches!(state, Some(crate::app::RemoteGrokInstallState::Succeeded))
            && theme::ghost_action_with(
                ui,
                top_action_id("ssh-grok-install"),
                "Install plugin",
                false,
                layout.button_height,
                theme::BG_CARD,
                ready,
            )
            .activated
        {
            if let Err(error) = model.approve_remote_grok_install(selected) {
                model.set_ssh_notice(error);
            }
        }
        if theme::ghost_action(
            ui,
            top_action_id("ssh-grok-withdraw"),
            "Withdraw consent",
            false,
            layout.button_height,
            theme::BG_CARD,
        )
        .activated
        {
            if let Err(error) = model.withdraw_remote_grok_install(selected) {
                model.set_ssh_notice(error);
            }
        }
        return;
    }
    ui.label(theme::micro(
        "Approving runs the official `grok plugin install <staged-plugin> --trust` against this server's already provisioned or reused private companion and the grok detection found here. It does not stage the companion under this approval.",
    ));
    if !ready {
        ui.label(theme::micro(
            "Open this server's remote workspace first so Made can provision or verify its private companion; approval stays unavailable until that succeeds.",
        ));
    }
    ui.label(theme::micro(
        "Plugin hooks apply only to Grok sessions started after the install. Uninstall stays your own `grok plugin uninstall grok-made --confirm`.",
    ));
    if refused {
        ui.label(theme::micro(
            "Refused. Nothing was installed or written; session follow uses the remote inventory.",
        ));
    }
    ui.horizontal(|ui| {
        if theme::primary_action(
            ui,
            top_action_id("ssh-grok-approve"),
            "Approve",
            layout.button_height,
            ready,
            theme::BG_CARD,
        )
        .activated
        {
            if let Err(error) = model.approve_remote_grok_install(selected) {
                model.set_ssh_notice(error);
            }
        }
        if theme::ghost_action(
            ui,
            top_action_id("ssh-grok-refuse"),
            "Refuse",
            false,
            layout.button_height,
            theme::BG_CARD,
        )
        .activated
        {
            model.refuse_remote_grok_install(selected);
        }
    });
}

fn ssh_prompt_card(ui: &mut Ui, class: &crate::remote_session::SshPromptClass, draft: &mut String) {
    match class {
        crate::remote_session::SshPromptClass::ChangedHostKey { .. } => {
            ui.label(theme::body("the remote host key changed and was refused"));
        }
        crate::remote_session::SshPromptClass::UnknownHost { text } => {
            ui.label(theme::body(
                "This host is not trusted yet. Approve only if the fingerprint matches.",
            ));
            ui.label(theme::body(sanitize_display(text, 700)));
            ui.label(theme::micro(
                "Approve sends yes. Reject cancels the attempt.",
            ));
        }
        crate::remote_session::SshPromptClass::Secret { text } => {
            ui.label(theme::body(sanitize_display(text, 400)));
            ssh_text_edit(ui, "prompt", draft, true);
        }
    }
}

fn remote_open_card(ui: &mut Ui, layout: &Layout, workbench: &mut Workbench) {
    let available = ui.max_rect();
    let width = (available.width() - 48.0).clamp(220.0, 560.0);
    let rect = egui::Rect::from_min_size(
        egui::pos2(available.left() + 24.0, available.top() + 24.0),
        egui::vec2(width, 220.0),
    );
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    let needs_consent = workbench.remote_needs_consent();
    let needs_update = workbench.remote_needs_update();
    let update_failed = workbench.remote_update_failed();
    let auth = workbench.remote_auth();
    let disconnected = workbench.remote_can_reconnect();
    let detail = workbench
        .remote_detail()
        .unwrap_or("Remote session")
        .to_owned();
    egui::Frame::new()
        .fill(theme::BG_CARD)
        .stroke(Stroke::new(1.0, theme::BORDER_HEADER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(12))
        .show(&mut child, |ui| {
            ui.label(
                RichText::new(if disconnected {
                    "Connection lost"
                } else if needs_update {
                    "Companion update"
                } else {
                    "SSH"
                })
                .font(FontId::proportional(13.0))
                .color(if disconnected {
                    theme::ERROR
                } else {
                    theme::TEXT_PRIMARY
                }),
            );
            ui.label(theme::body(sanitize_display(&detail, 500)));
            if workbench.remote_needs_host_approval() {
                ui.label(theme::body(
                    "This host is not in the trust file. Approve only the fingerprint below. Nothing is sent until you do.",
                ));
                match workbench.remote_host_fingerprint() {
                    Some(fingerprint) => {
                        ui.label(theme::body(sanitize_display(fingerprint, 120)));
                    }
                    None => {
                        ui.label(theme::micro(
                            "No fingerprint was provided. Approval is still required, and none was assumed.",
                        ));
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Approve host").clicked() {
                        workbench.approve_unknown_host();
                    }
                    if ui.button("Cancel").clicked() {
                        workbench.deny_unknown_host();
                    }
                });
            }
            if needs_consent {
                ui.label(theme::micro(
                    "Cancel writes nothing. Approval is stored for this server and version only.",
                ));
                ui.horizontal(|ui| {
                    if ui.button("Approve companion").clicked() {
                        workbench.grant_remote_consent();
                    }
                    if ui.button("Cancel").clicked() {
                        workbench.deny_remote_consent();
                    }
                });
            }
            if needs_update {
                ui.label(theme::body(
                    "The installed companion for this version differs from the packaged files. Updating replaces only Made's private companion directory.",
                ));
                let version = sanitize_display(crate::APP_VERSION, 40);
                let target = workbench
                    .remote_update_target()
                    .map(|target| sanitize_display(target, 80))
                    .unwrap_or_else(|| "the detected target".to_owned());
                ui.label(theme::micro(format!("Version {version} · {target}")));
                ui.label(theme::micro(
                    "This approval applies to this attempt only. Running sessions keep their processes; nothing else is written or deleted.",
                ));
                ui.horizontal(|ui| {
                    let label = if update_failed {
                        "Retry update"
                    } else {
                        "Update companion"
                    };
                    if ui.button(label).clicked() {
                        if update_failed {
                            workbench.retry_remote_update();
                        } else {
                            workbench.approve_remote_update();
                        }
                    }
                    if ui.button("Not now").clicked() {
                        workbench.refuse_remote_update();
                    }
                });
            }
            if let Some(class) = auth {
                // A prompt appears while the terminal still owns focus. Focus the
                // secret field once so typing does not go to the terminal and
                // submit an empty password. The flag prevents re-focusing after
                // the user deliberately clicks away.
                if matches!(
                    class,
                    crate::remote_session::SshPromptClass::Secret { .. }
                ) {
                    if let Some(prompt_id) = workbench.remote_prompt_id() {
                        let key = egui::Id::unique(("made-ssh-prompt-focus", prompt_id));
                        let first = ui
                            .ctx()
                            .data(|data| data.get_temp::<bool>(key).is_none());
                        let field = ssh_field_id("prompt");
                        // A live prompt hidden for a pass loses its hold.
                        // Re-hold when the field was not painted last pass, but
                        // not after the user deliberately clicked away from it
                        // or while another live SSH field owns the hold.
                        let repainted = ui.ctx().read_response(field).is_none();
                        let other_field_held =
                            held_ssh_field(ui.ctx()).is_some_and(|held| held != field);
                        if first || (repainted && !other_field_held) {
                            ui.memory_mut(|memory| memory.request_focus(field));
                            hold_ssh_field(ui.ctx(), field);
                            ui.ctx()
                                .data_mut(|data| data.insert_temp(key, true));
                        }
                    }
                }
                if let Some(draft) = workbench.remote_prompt_draft() {
                    ssh_prompt_card(ui, &class, draft);
                }
                let changed = matches!(
                    class,
                    crate::remote_session::SshPromptClass::ChangedHostKey { .. }
                );
                ui.horizontal(|ui| {
                    if !changed && ui.button("Approve").clicked() {
                        workbench.submit_remote_auth(true);
                    }
                    if ui
                        .button(if changed { "Refuse key" } else { "Cancel" })
                        .clicked()
                    {
                        workbench.submit_remote_auth(false);
                    }
                });
            }
            if workbench.remote_can_reconnect() && ui.button("Reconnect").clicked() {
                let _ = workbench.reconnect_remote();
            }
            let _ = layout;
        });
}

fn terminal_unavailable_card(
    ui: &mut Ui,
    layout: &Layout,
    message: &str,
    explorer_available: bool,
) {
    let available = ui.max_rect();
    let width = (available.width() - 48.0).clamp(200.0, 520.0);
    let rect = egui::Rect::from_min_size(
        egui::pos2(available.left() + 24.0, available.top() + 24.0),
        egui::vec2(width, 104.0),
    );
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    egui::Frame::new()
        .fill(theme::ERROR_TINT)
        .stroke(Stroke::new(1.0, theme::ERROR))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(12))
        .show(&mut child, |ui| {
            ui.label(
                RichText::new("Terminal unavailable")
                    .font(FontId::proportional(13.0))
                    .color(theme::ERROR),
            );
            ui.label(theme::body(sanitize_display(message, 500)));
            if explorer_available {
                ui.label(theme::micro(
                    "The explorer remains available; Made never blocks input on a terminal failure.",
                ));
            } else {
                ui.label(theme::micro(
                    "The remote folder did not open. Reconnect and retry after the companion is available.",
                ));
            }
            let _ = layout;
        });
}

fn notice_strip(
    ui: &mut Ui,
    layout: &Layout,
    fill: Color32,
    tone: Color32,
    text: &str,
    tooltip: &str,
) {
    let height = if layout.compact { 26.0 } else { 30.0 };
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
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
    response.on_hover_text(sanitize_display(tooltip, 700));
}

fn log_level_tone(level: crate::diagnostics::LogLevel) -> (Color32, Color32) {
    match level {
        crate::diagnostics::LogLevel::Info => (theme::INFO, theme::INFO_TINT),
        crate::diagnostics::LogLevel::Warn => (theme::WARNING, theme::WARNING_TINT),
        crate::diagnostics::LogLevel::Error => (theme::ERROR, theme::ERROR_TINT),
    }
}

fn log_issue_tone(severity: LogIssueSeverity) -> (Color32, Color32) {
    match severity {
        LogIssueSeverity::Warning => (theme::WARNING_TINT, theme::WARNING),
        LogIssueSeverity::Error => (theme::ERROR_TINT, theme::ERROR),
    }
}

fn diagnostics_section(ui: &mut Ui, label: &str) {
    ui.add_space(8.0);
    theme::section_label(ui, label);
}

fn diagnostics_rows(ui: &mut Ui, id: &str, rows: &[(&str, String)]) {
    if ui.available_width() < 560.0 {
        for (label, value) in rows {
            ui.label(theme::micro(*label));
            ui.add(
                egui::Label::new(theme::mono(
                    sanitize_display(value, 900),
                    10.5,
                    theme::TEXT_SECONDARY,
                ))
                .wrap(),
            )
            .on_hover_text(sanitize_display(value, 900));
            ui.add_space(2.0);
        }
        return;
    }
    let value_width = (ui.available_width() - 130.0).max(120.0);
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([18.0, 4.0])
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.label(theme::micro(*label));
                ui.allocate_ui_with_layout(
                    egui::vec2(value_width, 16.0),
                    egui::Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.add(
                            egui::Label::new(theme::mono(
                                sanitize_display(value, 900),
                                10.5,
                                theme::TEXT_SECONDARY,
                            ))
                            .truncate(),
                        )
                        .on_hover_text(sanitize_display(value, 900));
                    },
                );
                ui.end_row();
            }
        });
}

/// Session-map unread indicator. The API's pulse can be zero while unread is
/// true; render the plain word rather than "unread×0".
pub fn unread_label(unread: bool, prompt_pulse: u32) -> Option<String> {
    if unread {
        if prompt_pulse == 0 {
            Some("unread".to_owned())
        } else {
            Some(format!("unread×{prompt_pulse}"))
        }
    } else if prompt_pulse > 0 {
        Some(format!("unread×{prompt_pulse}"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_map::SessionRow;
    use std::ffi::OsString;
    use std::sync::MutexGuard;

    static APP_PATH_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn every_vendored_mark_of_the_product_resolves() {
        let mut vendored = 0;
        for entry in crate::integrations::PRODUCT {
            if let Mark::Vendored(name) = entry.descriptor().mark {
                vendored += 1;
                assert!(
                    crate::explorer_icons::icon_by_file_name(name).is_some(),
                    "{name} is not an explorer icon"
                );
                // Resolution is cached: the second lookup returns the same icon.
                assert_eq!(vendored_mark(name), vendored_mark(name));
            }
        }
        assert!(vendored > 0, "no product integration has a vendored mark");
    }

    /// Serializes and restores app-path environment variables for unit tests
    /// that must construct a real `Workbench`. `Drop` restores every variable
    /// even if the test panics.
    struct IsolatedAppPaths {
        _guard: MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<OsString>)>,
        _temp: tempfile::TempDir,
    }

    impl IsolatedAppPaths {
        fn new() -> Self {
            let guard = APP_PATH_ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let temp = tempfile::tempdir().expect("isolated app-path tempdir");
            let home = temp.path().join("home");
            let tmp = temp.path().join("tmp");
            let runtime = temp.path().join("runtime");
            let roaming = temp.path().join("appdata").join("roaming");
            let local = temp.path().join("appdata").join("local");
            for directory in [&home, &tmp, &runtime, &roaming, &local] {
                std::fs::create_dir_all(directory).expect("isolated app-path directory");
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700))
                    .expect("runtime permissions");
            }

            let variables = [
                "HOME",
                "USERPROFILE",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_STATE_HOME",
                "XDG_CACHE_HOME",
                "TMPDIR",
                "XDG_RUNTIME_DIR",
                "APPDATA",
                "LOCALAPPDATA",
            ];
            let saved = variables
                .iter()
                .map(|key| (*key, std::env::var_os(key)))
                .collect();
            std::env::set_var("HOME", &home);
            std::env::set_var("USERPROFILE", &home);
            std::env::set_var("XDG_CONFIG_HOME", temp.path().join("config"));
            std::env::set_var("XDG_DATA_HOME", temp.path().join("data"));
            std::env::set_var("XDG_STATE_HOME", temp.path().join("state"));
            std::env::set_var("XDG_CACHE_HOME", temp.path().join("cache"));
            std::env::set_var("TMPDIR", temp.path());
            std::env::set_var("XDG_RUNTIME_DIR", &runtime);
            std::env::set_var("APPDATA", &roaming);
            std::env::set_var("LOCALAPPDATA", &local);

            Self {
                _guard: guard,
                saved,
                _temp: temp,
            }
        }
    }

    impl Drop for IsolatedAppPaths {
        fn drop(&mut self) {
            for (key, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn row(index: usize, active: bool) -> SessionRow {
        SessionRow {
            session_id: format!("s{index}"),
            title: format!("Session {index}"),
            directory: Some(std::path::PathBuf::from(format!("/tmp/workspace-{index}"))),
            subpath: None,
            active,
            busy: index == 1,
            attention: index == 2,
            unread: index == 3,
            prompt_pulse: if index == 3 { 2 } else { 0 },
            renaming: index == 4,
        }
    }

    fn text_rect(output: &egui::FullOutput, needle: &str) -> Option<egui::Rect> {
        fn visit(shape: &egui::Shape, needle: &str, best: &mut Option<egui::Rect>) {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text.contains(needle) => {
                    *best = Some(egui::Rect::from_min_size(text.pos, text.galley.size()));
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        visit(shape, needle, best);
                    }
                }
                _ => {}
            }
        }
        let mut best = None;
        for clipped in &output.shapes {
            visit(&clipped.shape, needle, &mut best);
        }
        best
    }

    fn fill_rects(output: &egui::FullOutput) -> Vec<(egui::Rect, egui::Color32)> {
        fn visit(shape: &egui::Shape, rects: &mut Vec<(egui::Rect, egui::Color32)>) {
            match shape {
                egui::Shape::Rect(rect) => rects.push((rect.rect, rect.fill)),
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
    fn unread_with_zero_pulse_renders_the_word() {
        assert_eq!(unread_label(true, 0).as_deref(), Some("unread"));
        assert_eq!(unread_label(true, 3).as_deref(), Some("unread×3"));
        assert_eq!(unread_label(false, 2).as_deref(), Some("unread×2"));
        assert_eq!(unread_label(false, 0), None);
    }

    #[test]
    fn tab_strip_uses_selected_background_without_underline_or_marker_circles() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let layout = Layout::for_window(720.0, 480.0);
        let mut rows: Vec<SessionRow> = (0..6).map(|index| row(index, index == 0)).collect();
        // Keep the unread and attention badges inside the compact candidate set.
        rows[1].busy = false;
        rows[1].unread = true;
        rows[1].prompt_pulse = 2;
        rows[1].attention = false;
        rows[2].unread = false;
        rows[2].prompt_pulse = 0;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(720.0, 480.0),
            )),
            ..Default::default()
        };
        let mut strip_output = None;
        let mut output = ctx.run_ui(raw, |ui| {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(520.0, layout.tab_strip_height), Sense::hover());
            let state = TabStripState {
                active: Some("s0"),
                selected: Some("s1"),
                in_flight: true,
            };
            strip_output = Some(tab_strip_ui(ui, rect, &layout, &rows, &state));
        });
        let strip_output = strip_output.expect("strip rendered");
        let accent_circle = output.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Circle(circle) => circle.fill == theme::ACCENT && circle.radius <= 4.0,
            _ => false,
        });
        let accent_underline = output.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect_shape) => {
                rect_shape.fill == theme::ACCENT
                    && rect_shape.rect.height() <= 3.0
                    && rect_shape.rect.width() > 20.0
            }
            _ => false,
        });
        let selected_fill = output.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect_shape) => {
                rect_shape.fill == theme::BG_SELECTED
                    && (rect_shape.rect.height() - layout.session_tab_height).abs() < 1.0
            }
            _ => false,
        });
        let active_text = text_rect(&output, "Session 0").expect("active tab text");
        let selected_text = text_rect(&output, "Session 1").expect("selected tab text");
        let fills = fill_rects(&output);
        output.textures_delta.clear();
        assert!(!output.shapes.is_empty());
        assert!(
            strip_output.overflow,
            "six tabs must offer the overflow menu"
        );
        assert!(
            !accent_circle,
            "session tabs must not paint active/hollow marker circles"
        );
        assert!(
            !accent_underline,
            "selected session tab must not paint an underline"
        );
        assert!(
            selected_fill,
            "selected session tab must use the BG_SELECTED fill"
        );
        for (name, text) in [("active", active_text), ("selected", selected_text)] {
            assert!(
                fills.iter().any(|(rect, fill)| {
                    *fill == theme::BG_SELECTED && rect.contains_rect(text)
                }),
                "{name} session tab must use the BG_SELECTED fill"
            );
        }
        let attention_badge = output.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Circle(circle) => {
                circle.fill == theme::WARNING_TINT && circle.radius >= 6.0
            }
            _ => false,
        });
        let unread_badge = output.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect_shape) => rect_shape.fill == theme::INFO_TINT,
            _ => false,
        });
        assert!(attention_badge, "attention badge must survive tab polish");
        assert!(unread_badge, "unread badge must survive tab polish");
    }

    #[test]
    fn bounded_tabs_keep_the_active_tab_and_offer_overflow() {
        let widths = vec![120.0, 120.0, 120.0, 120.0];
        let (visible, overflow) = bounded_tab_indices(&widths, Some(0), 380.0, 4.0, None);
        assert!(overflow);
        assert!(visible.contains(&0));
        assert!(visible.len() < widths.len());

        let (visible, overflow) = bounded_tab_indices(&widths, Some(3), 380.0, 4.0, None);
        assert!(overflow);
        assert!(visible.contains(&3), "active tab must stay visible");

        let (visible, overflow) = bounded_tab_indices(&widths, Some(3), 1_000.0, 4.0, None);
        assert!(!overflow);
        assert_eq!(visible, vec![0, 1, 2, 3]);
    }

    #[test]
    fn bounded_tabs_reserve_the_trailing_plus_and_overflow_trigger() {
        let widths = vec![120.0, 120.0, 120.0];
        // 3 tabs + gaps leave no room for the trailing plus: overflow is required
        // and the plus width/gap must still be reserved from the tab budget.
        let (visible, overflow) = bounded_tab_indices(&widths, Some(0), 380.0, 4.0, Some(28.0));
        assert!(overflow);
        assert_eq!(visible, vec![0, 1]);
        // 3 tabs + gaps + plus fits without an overflow trigger.
        let (visible, overflow) = bounded_tab_indices(&widths, Some(0), 420.0, 4.0, Some(28.0));
        assert!(!overflow);
        assert_eq!(visible, vec![0, 1, 2]);
    }

    #[test]
    fn candidate_tabs_prefer_the_active_tab_at_the_limit() {
        let candidates = candidate_tab_indices(9, Some(8), 3);
        assert_eq!(candidates.len(), 3);
        assert!(candidates.contains(&8));
    }

    #[test]
    fn picker_rows_are_cached_until_the_tree_or_refresh_changes() {
        let temp = tempfile::tempdir().unwrap();
        for index in 0..100 {
            std::fs::create_dir(temp.path().join(format!("dir-{index:03}"))).unwrap();
        }
        let mut picker = crate::workspace_picker::WorkspacePicker::new(temp.path()).unwrap();
        let ctx = egui::Context::default();
        let first = picker_display_rows(&ctx, &picker);
        let second = picker_display_rows(&ctx, &picker);
        assert_eq!(first.len(), 100);
        assert!(
            Arc::ptr_eq(&first, &second),
            "unchanged tree must reuse sanitized rows"
        );
        std::fs::rename(temp.path().join("dir-050"), temp.path().join("renamed-050")).unwrap();
        let revision = picker.revision();
        picker.refresh().unwrap();
        assert!(picker.revision() > revision);
        let third = picker_display_rows(&ctx, &picker);
        assert!(!Arc::ptr_eq(&first, &third));
        assert!(third.iter().any(|row| row.name == "renamed-050"));
        assert!(third.iter().all(|row| row.name != "dir-050"));
        picker.select(0).unwrap();
        picker.toggle_selected().unwrap();
        let expanded = picker_display_rows(&ctx, &picker);
        assert!(!Arc::ptr_eq(&third, &expanded));
        let other = crate::workspace_picker::WorkspacePicker::new(temp.path()).unwrap();
        assert_ne!(picker.instance_id(), other.instance_id());
        let fourth = picker_display_rows(&ctx, &other);
        assert!(!Arc::ptr_eq(&expanded, &fourth));
    }

    #[test]
    fn picker_target_names_the_current_folder() {
        let temp = tempfile::tempdir().unwrap();
        let picker = crate::workspace_picker::WorkspacePicker::new(temp.path()).unwrap();
        let model = AppModel::from_launch(
            crate::workspace::WorkspaceLaunch::Picker {
                picker,
                error: None,
            },
            crate::app::AppOptions::new(),
        );
        let target = picker_current_target(&model).expect("picker target");
        assert_eq!(
            target,
            temp.path().canonicalize().unwrap().display().to_string()
        );
    }

    #[test]
    fn diagnostics_metrics_bucket_and_heartbeat_age_advance() {
        let ctx = egui::Context::default();
        let mut first_output = ctx.run_ui(
            egui::RawInput {
                time: Some(1.0),
                ..Default::default()
            },
            |_| {},
        );
        first_output.textures_delta.clear();
        let first = diagnostics_metrics_bucket(&ctx);
        let mut second_output = ctx.run_ui(
            egui::RawInput {
                time: Some(1.3),
                ..Default::default()
            },
            |_| {},
        );
        second_output.textures_delta.clear();
        let second = diagnostics_metrics_bucket(&ctx);
        assert!(second > first, "metrics cache bucket must advance");

        let now = std::time::Instant::now();
        let last = now - std::time::Duration::from_millis(1500);
        let age = heartbeat_age(Some(last), now).expect("heartbeat age");
        assert!(age >= std::time::Duration::from_millis(1500));
    }

    #[test]
    fn diagnostics_metrics_cache_refreshes_counters_while_open() {
        let _app_paths = IsolatedAppPaths::new();
        let temp = tempfile::tempdir().unwrap();
        let mut model = AppModel::from_launch(
            crate::workspace::WorkspaceLaunch::waiting_for_picker(),
            crate::app::AppOptions::new(),
        );
        model.open_plan(
            crate::workspace::LaunchPlan::open(
                temp.path(),
                crate::workspace::WorkspaceSource::Picker,
            )
            .unwrap(),
        );
        model.ensure_workbench();
        let id = model.active_workspace_id().expect("active workspace");
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let raw = |time: f64| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            time: Some(time),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw(1.0), |ui| model.ui(ui));
        output.textures_delta.clear();
        model.logic(&ctx);
        let workbench = model.workspace(id).unwrap();
        let first = workbench.diagnostics_metrics_cache(&ctx);
        assert!(first.text.contains("counters:"));

        model.workspace_mut(id).unwrap().set_explorer_width(380.0);
        let mut output = ctx.run_ui(raw(1.3), |ui| model.ui(ui));
        output.textures_delta.clear();
        model.logic(&ctx);
        let second = model.workspace(id).unwrap().diagnostics_metrics_cache(&ctx);
        assert!(
            !Arc::ptr_eq(&first, &second),
            "a new time bucket must rebuild the metrics report"
        );
        assert_eq!(
            model
                .workspace(id)
                .unwrap()
                .metrics()
                .counters()
                .explorer_layout_changes,
            1,
            "the layout change must reach the live report"
        );
        assert!(second.text.contains("explorer_layout_changes=1"));
        model.shutdown();
    }

    #[test]
    fn shutdown_window_close_frame_does_not_paint_the_workspace() {
        let _app_paths = IsolatedAppPaths::new();
        let temp = tempfile::tempdir().unwrap();
        let mut model = AppModel::from_launch(
            crate::workspace::WorkspaceLaunch::waiting_for_picker(),
            crate::app::AppOptions::new(),
        );
        model.open_plan(
            crate::workspace::LaunchPlan::open(
                temp.path(),
                crate::workspace::WorkspaceSource::Picker,
            )
            .unwrap(),
        );
        model.ensure_workbench();
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw(), |ui| model.ui(ui));
        output.textures_delta.clear();
        assert!(
            !output.shapes.is_empty(),
            "the live workspace must render before shutdown"
        );
        assert!(
            text_rect(&output, "Terminal unavailable").is_none(),
            "the live terminal must not show the unavailable card"
        );

        // eframe runs `logic` (which observes the close request and shuts the
        // app down) and then `ui` in the same frame. The shutdown frame must
        // not paint the now-removed terminal's error card.
        model.shutdown();
        let frames = model.ui_frame_count();
        let mut output = ctx.run_ui(raw(), |ui| model.ui(ui));
        output.textures_delta.clear();
        assert!(
            text_rect(&output, "Terminal unavailable").is_none(),
            "shutdown must not paint the terminal-unavailable card"
        );
        assert!(
            !fill_rects(&output)
                .iter()
                .any(|(_, fill)| *fill == theme::ERROR_TINT),
            "shutdown must not paint an error-tinted card"
        );
        assert!(
            output.shapes.is_empty(),
            "shutdown must not render the workspace"
        );
        assert_eq!(
            model.ui_frame_count(),
            frames,
            "a shutdown UI pass must not count as a rendered frame"
        );
    }

    #[test]
    fn diagnostics_status_pill_renders_shapes() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let layout = Layout::for_window(1280.0, 800.0);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            let (pill, _) = ui
                .allocate_exact_size(egui::vec2(100.0, layout.status_pill_height), Sense::hover());
            let mut pill_ui = ui.new_child(UiBuilder::new().max_rect(pill));
            status_pill_ui(
                &mut pill_ui,
                IntegrationStatus::Inactive,
                "OpenCode is not running.",
                &layout,
            );
        });
        assert!(!output.shapes.is_empty());
        output.textures_delta.clear();
    }

    #[test]
    fn picker_primary_action_is_the_accent_control() {
        assert_eq!(picker_primary_label(), "Use current folder");
    }

    #[test]
    fn log_issue_severity_selects_warning_or_error_tone() {
        assert_eq!(
            log_issue_tone(LogIssueSeverity::Warning),
            (theme::WARNING_TINT, theme::WARNING)
        );
        assert_eq!(
            log_issue_tone(LogIssueSeverity::Error),
            (theme::ERROR_TINT, theme::ERROR)
        );
    }

    #[test]
    fn workspace_tab_icon_follows_connection_or_a_live_direct_tui() {
        let opencode = TerminalTarget::Tui(OPENCODE);
        assert!(
            !workspace_opencode_active(TerminalTarget::Shell, false, true),
            "a running shell is not OpenCode"
        );
        assert!(
            workspace_opencode_active(TerminalTarget::Shell, true, false),
            "a connected integration covers a manually launched OpenCode"
        );
        assert!(
            workspace_opencode_active(opencode, false, true),
            "a directly launched TUI shows the icon before the bridge connects"
        );
        assert!(
            !workspace_opencode_active(opencode, false, false),
            "the icon disappears when a direct TUI exits or the integration disconnects"
        );
    }

    #[test]
    fn workspace_busy_status_pulse_is_subtle_and_periodic() {
        let period = WORKSPACE_TAB_BUSY_PULSE_PERIOD;
        assert!(
            (1.5..=2.0).contains(&period),
            "the busy pulse period must stay gentle: {period}"
        );
        let bright = workspace_busy_status_alpha(period * 0.25);
        let dim = workspace_busy_status_alpha(period * 0.75);
        assert_eq!(bright, 255, "the pulse must reach full opacity");
        let dim_opacity = dim as f32 / 255.0;
        assert!(
            (dim_opacity - WORKSPACE_TAB_BUSY_PULSE_MIN).abs() < 0.01,
            "the pulse minimum must stay around 0.35: {dim_opacity}"
        );
        assert!(dim < bright, "the pulse must change opacity");
        let sample = 0.137;
        assert_eq!(
            workspace_busy_status_alpha(sample),
            workspace_busy_status_alpha(sample + period),
            "the pulse must repeat with its period"
        );
    }

    #[test]
    fn workspace_tab_width_keeps_long_labels_wider_than_before() {
        assert_eq!(workspace_tab_dot_excess(0), 0.0);
        assert_eq!(workspace_tab_dot_excess(3), 0.0);
        assert_eq!(workspace_tab_dot_excess(4), 3.0);
        assert_eq!(workspace_tab_dot_excess(8), 31.0);
        assert_eq!(
            workspace_tab_dot_excess(9),
            31.0,
            "the eighth dot caps the width excess"
        );

        for (width, height, base_max, expected_max, old_label) in [
            (720.0, 480.0, 198.0, 229.0, 114.0),
            (1280.0, 800.0, 238.0, 269.0, 154.0),
        ] {
            let layout = Layout::for_window(width, height);
            let long_label = "workspace-with-a-long-label-that-clamps";
            assert!(
                (workspace_tab_base_width(long_label, &layout) - base_max).abs() < f32::EPSILON,
                "{width}px: long label base must clamp at exactly {base_max}"
            );
            let tab_width = workspace_tab_width(long_label, &layout, 8);
            assert!(
                (tab_width - expected_max).abs() < f32::EPSILON,
                "{width}px: eight dots must clamp at exactly {expected_max}: {tab_width}"
            );
            let label_width = workspace_tab_label_width(tab_width, 8);
            let base_label_width =
                workspace_tab_label_width(workspace_tab_base_width(long_label, &layout), 0);
            assert!(
                (label_width - base_label_width).abs() < f32::EPSILON,
                "{width}px: eight dots must not shrink the label area: {label_width} vs {base_label_width}"
            );
            assert!(
                label_width - old_label >= 12.0 - f32::EPSILON,
                "{width}px: eight dots must leave at least 12px more text: {label_width} vs {old_label}"
            );

            let typical_label = "typical-label";
            let chars = typical_label.chars().count() as f32;
            let typical_width = workspace_tab_width(typical_label, &layout, 8);
            let old_typical_label = chars * 6.2 + 38.0 - 36.0;
            let typical_label_width = workspace_tab_label_width(typical_width, 8);
            assert!(
                typical_label_width - old_typical_label >= 12.0 - f32::EPSILON,
                "{width}px: a typical label must gain at least 12px of text: {typical_label_width} vs {old_typical_label}"
            );
        }
    }

    #[test]
    fn workspace_tab_accessible_name_includes_session_summary() {
        let busy = opencode_sessions_summary(&[SessionActivity::Busy, SessionActivity::Idle])
            .expect("busy summary");
        assert_eq!(busy, "Sessions: 1 running, 1 idle");

        let attention = opencode_sessions_summary(&[
            SessionActivity::Attention,
            SessionActivity::Idle,
            SessionActivity::Busy,
        ])
        .expect("attention summary");
        assert_eq!(attention, "Sessions: 1 running, 1 needs attention, 1 idle");
        assert_eq!(
            opencode_sessions_summary(&[SessionActivity::Attention]),
            Some("Sessions: 1 needs attention".to_owned())
        );
        assert_eq!(
            opencode_sessions_summary(&[
                SessionActivity::Attention,
                SessionActivity::Attention,
                SessionActivity::Busy,
            ]),
            Some("Sessions: 1 running, 2 need attention".to_owned())
        );
        assert_eq!(
            opencode_sessions_summary(&[SessionActivity::Busy; 9]),
            Some("Sessions: 9 running".to_owned()),
            "the summary counts sessions beyond the eight painted dots"
        );
        assert_eq!(opencode_sessions_summary(&[]), None);

        let accessible = workspace_tab_accessible_label("OpenCode", Some(&attention));
        assert_eq!(accessible, format!("OpenCode — {attention}"));
        assert!(
            accessible.contains("OpenCode — Sessions: 1 running"),
            "{accessible}"
        );
        assert!(
            accessible.contains("1 needs attention"),
            "attention summary missing: {accessible}"
        );
        assert_eq!(workspace_tab_accessible_label("alpha", None), "alpha");
        assert_eq!(
            workspace_tab_accessible_label(GROK.descriptor().label, Some(&attention)),
            format!("Grok Build — {attention}"),
            "a branded Grok tab uses Grok Build and the same session summary"
        );
        assert_eq!(
            workspace_tab_accessible_label("project-alpha", Some(&attention)),
            format!("project-alpha — {attention}"),
            "a tab with Grok closed keeps the folder name and the same session summary"
        );
    }

    #[test]
    fn grok_build_brand_follows_a_running_tui_or_open_sessions_and_yields_to_opencode() {
        let grok = TerminalTarget::Tui(GROK);
        let shell = TerminalTarget::Shell;
        let opencode = TerminalTarget::Tui(OPENCODE);
        let authority = made_remote::protocol::AuthorityId([7; 16]);
        let remote_grok = TerminalTarget::RemoteGrok { authority };
        let remote_shell = TerminalTarget::RemoteShell { authority };
        assert!(workspace_grok_branded(false, grok, true, 0));
        assert!(workspace_grok_branded(false, shell, true, 2));
        assert!(workspace_grok_branded(false, grok, false, 1));
        assert!(!workspace_grok_branded(false, grok, false, 0));
        assert!(!workspace_grok_branded(false, shell, true, 0));
        assert!(!workspace_grok_branded(false, opencode, true, 0));
        assert!(!workspace_grok_branded(true, grok, true, 3));
        assert!(!workspace_grok_branded(true, shell, true, 3));
        // A remote tab's branding comes from its own server state: a directly
        // launched remote grok is branded before any session, an owned open
        // session brands a remote shell tab, and OpenCode keeps precedence.
        assert!(workspace_grok_branded(false, remote_grok, true, 0));
        assert!(!workspace_grok_branded(false, remote_grok, false, 0));
        assert!(workspace_grok_branded(false, remote_shell, false, 2));
        assert!(!workspace_grok_branded(true, remote_grok, true, 3));
    }

    #[test]
    fn claude_code_brand_yields_to_opencode_and_to_remote_grok_presentation() {
        // Present Claude Code brands the tab on its own.
        assert!(workspace_claude_branded(false, false, true));
        assert!(!workspace_claude_branded(false, false, false));
        // OpenCode keeps its label, icon and dots.
        assert!(!workspace_claude_branded(true, false, true));
        // A remote tab already presenting Grok Build under SSH-10 keeps it.
        assert!(!workspace_claude_branded(false, true, true));
    }

    #[test]
    fn compact_tab_width_and_picker_label_stay_within_targets() {
        let layout = Layout::for_window(720.0, 480.0);
        let row = row(0, true);
        assert!((112.0..=184.0).contains(&tab_width(&row, &layout)));
        assert_eq!(theme::format_bytes(4_300), "4.2 KB");
    }
}
