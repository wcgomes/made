//! egui surfaces for the Calm Editor design.
//!
//! The terminal is the hero: the floating header, full-height explorer, and
//! mutually exclusive terminal/preview surfaces are deliberately quiet. There
//! is no routine status bar or integration banner and no persistent header
//! band: wordmark, tabs, and icon actions sit on the clear/native backdrop.
//! The full-height Explorer card uses the theme's in-app frosted-glass
//! approximation; terminal, preview, and picker cards stay opaque. All display
//! text is sanitized and every control keeps a visible focus state.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, RichText, Sense, Stroke, StrokeKind, Ui,
    UiBuilder,
};

use crate::app::{AppModel, WorkspaceId};
use crate::diagnostics::DiagnosticsSnapshot;
use crate::explorer_view::{ExplorerPane, PreviewUiAction};
use crate::integration_status::IntegrationStatus;
use crate::sanitize::{sanitize_copy_text, sanitize_display};
use crate::session_map::{truncate_end, truncate_path, SessionRow};
use crate::terminal_view::TerminalUiOutput;
use crate::theme::{self, Layout};
use crate::workbench::{LeftSurface, Workbench};
use crate::APP_VERSION;

// ── Status tone mapping ─────────────────────────────────────────────────────

fn status_tone(status: IntegrationStatus) -> (Color32, Color32, &'static str) {
    match status {
        IntegrationStatus::Connected => (theme::SUCCESS, theme::SUCCESS_TINT, "●"),
        IntegrationStatus::Inactive => (theme::TEXT_TERTIARY, theme::BG_ELEVATED, "○"),
        IntegrationStatus::Degraded => (theme::WARNING, theme::WARNING_TINT, "!"),
        IntegrationStatus::Unsupported => (theme::ERROR, theme::ERROR_TINT, "×"),
    }
}

fn status_pill_ui(
    ui: &mut Ui,
    status: IntegrationStatus,
    reason: &str,
    layout: &Layout,
) -> egui::Response {
    let (tone, tint, symbol) = status_tone(status);
    let text = format!("{symbol} {}", status.label());
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

/// Stable id for a global top-bar control.
pub fn top_action_id(action: &str) -> egui::Id {
    egui::Id::unique(("made-top-action", action))
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

fn workspace_tab_width(label: &str, layout: &Layout) -> f32 {
    let chars = label.chars().count() as f32;
    (chars * 6.2 + 38.0).clamp(
        if layout.compact { 84.0 } else { 96.0 },
        if layout.compact { 150.0 } else { 190.0 },
    )
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
}

/// Global workspace tabs; OpenCode sessions remain a separate optional map.
pub fn workspace_strip(model: &mut AppModel, ui: &mut Ui) -> bool {
    let layout = Layout::for_window(ui.available_width(), ui.ctx().content_rect().height());
    let active = model.active_workspace_id();
    let tabs: Vec<WorkspaceTabInfo> = model
        .workspace_ids()
        .filter_map(|id| {
            let workbench = model.workspace(id)?;
            let path = workbench.shell_cwd().to_path_buf();
            let label = workbench
                .launch_plan()
                .map(|plan| plan.workspace.display_name().to_owned())
                .unwrap_or_else(|| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Workspace".to_owned())
                });
            Some(WorkspaceTabInfo { id, label, path })
        })
        .collect();
    let has_workbench = model.workbench().is_some();
    let explorer_visible = model.workbench().is_some_and(Workbench::explorer_visible);
    let sessions_visible = model.workbench().is_some_and(Workbench::sessions_visible);
    let sessions_available = model.workbench().is_some_and(|workbench| {
        sessions_visible
            || !workbench.session_map().is_empty()
            || workbench.integration().status().is_connected()
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
    let mut toggle_explorer = false;
    let mut toggle_sessions = false;
    let mut diagnostics = false;

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
            if has_workbench {
                right_specs.push((
                    "diagnostics",
                    theme::IconGlyph::Diagnostics,
                    "Diagnostics",
                    false,
                ));
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
                    "diagnostics" if action.activated => diagnostics = true,
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
            theme::wordmark(&mut wordmark);

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
                .map(|tab| workspace_tab_width(&tab.label, &layout))
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
            if open_folder.activated {
                create = true;
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
                            ui.label(theme::mono(
                                truncate_path(
                                    &sanitize_display(&tab.path.display().to_string(), 900),
                                    72,
                                ),
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

    if create {
        model.begin_new_workspace();
    }
    if let Some(id) = activate {
        model.switch_workspace(id);
    }
    if let Some(id) = close {
        model.close_workspace(id);
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
    diagnostics
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
    let close_scale = theme::close_glyph_scale(ui.ctx().animate_bool_with_time_and_easing(
        egui::Id::unique(("made-close-scale", tab.id.get())),
        close_hovered,
        theme::CLOSE_HOVER_SCALE_TIME,
        egui::emath::easing::quadratic_out,
    ));
    let fill = if active {
        theme::BG_TAB_SELECTED
    } else if hovered {
        theme::BG_TAB_HOVER
    } else {
        theme::BG_TAB_IDLE
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(6),
        fill,
        Stroke::NONE,
        StrokeKind::Inside,
    );
    let label_width = (rect.width() - 36.0).max(8.0);
    let color = if active || hovered {
        theme::TEXT_PRIMARY
    } else {
        theme::inactive_tab_text()
    };
    let font = FontId::proportional(if active {
        theme::TAB_SELECTED_FONT_SIZE
    } else {
        theme::TAB_FONT_SIZE
    });
    let job = theme::ellipsized_job(sanitize_display(&tab.label, 80), font, color, label_width);
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    ui.painter().text(
        close_rect.center(),
        Align2::CENTER_CENTER,
        "×",
        FontId::proportional(theme::CLOSE_GLYPH_SIZE * close_scale),
        if close_hovered {
            theme::ERROR
        } else {
            theme::inactive_icon_color()
        },
    );
    if response.has_focus() {
        theme::paint_focus_ring(ui, rect, CornerRadius::same(6), theme::BG_CLEAR);
    }
    if close_response.has_focus() {
        theme::paint_inside_focus_ring(ui.painter(), close_rect.shrink(1.0), CornerRadius::same(4));
    }
    let tab_response = if close_hovered {
        response
    } else {
        response.on_hover_text(sanitize_display(&tab.path.display().to_string(), 900))
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

fn diagnostics_open_id() -> egui::Id {
    egui::Id::unique("made-diagnostics-open")
}

fn diagnostics_metrics_cache_id(pane_id: &str) -> egui::Id {
    egui::Id::unique(("made-diagnostics-metrics", pane_id))
}

fn diagnostics_metrics_bucket(ctx: &egui::Context) -> u64 {
    (ctx.input(|input| input.time) * 4.0).floor().max(0.0) as u64
}

fn heartbeat_age(
    last: Option<std::time::Instant>,
    now: std::time::Instant,
) -> Option<std::time::Duration> {
    last.map(|last| now.saturating_duration_since(last))
}

/// Stable id for the diagnostics copy action in one workspace.
pub fn diagnostics_copy_id(pane_id: &str) -> egui::Id {
    egui::Id::unique(("made-diagnostics-copy", pane_id))
}

impl Workbench {
    /// Draw the complete workspace surface.
    pub fn ui(&mut self, ui: &mut Ui, show_sessions: &mut bool, show_diagnostics: &mut bool) {
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
            self.workspace_ui(ui, &layout, show_diagnostics);
        });
        self.set_sessions_visible(*show_sessions);
    }

    fn workspace_ui(&mut self, ui: &mut Ui, layout: &Layout, show_diagnostics: &mut bool) {
        self.sync_terminal_visibility(ui.ctx());
        self.main_panels(ui, layout);
        self.apply_terminal_focus_boost(ui.ctx());
        let ctx = ui.ctx().clone();
        self.toast(&ctx, layout);
        self.diagnostics_window(&ctx, show_diagnostics, layout);
    }

    fn sync_terminal_visibility(&mut self, ctx: &egui::Context) {
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
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::unique(("made-terminal-focus-boost", self.pane_id())),
                4u8,
            );
        });
    }

    fn apply_terminal_focus_boost(&mut self, ctx: &egui::Context) {
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

    fn main_panels(&mut self, ui: &mut Ui, layout: &Layout) {
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
            if self.left_surface() == LeftSurface::Terminal {
                self.request_terminal_focus(ui.ctx());
            }
        }
        ui.ctx()
            .data_mut(|data| data.insert_temp(visibility_id, explorer_visible));
        let available = ui.available_width().max(1.0);
        let has_content = explorer.is_some() || self.workspace_error.is_some();
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
                            explorer.tree_ui(&mut content_ui, self.diag.log_mut(), layout);
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
            }
        }

        let mut preview_loaded = explorer
            .as_ref()
            .is_some_and(|pane| pane.preview_path().is_some());
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
                        }
                    }
                }
            });
        self.explorer = explorer;
    }

    fn terminal_card(&mut self, ui: &mut Ui, layout: &Layout, preview_loaded: bool) -> bool {
        let card_rect = ui.available_rect_before_wrap();
        let mut card_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(card_rect)
                .id_salt(("made-terminal-card", self.pane_id())),
        );
        let ui = &mut card_ui;
        ui.painter().rect_filled(
            card_rect,
            CornerRadius::same(layout.card_radius),
            theme::BG_TERMINAL,
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
                self.terminal_path_line(ui, path_rect, row, layout);
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
            ui.painter().rect_filled(slot_rect, 0.0, theme::BG_TERMINAL);
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
                theme::BG_TERMINAL,
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
        if let Some(terminal) = terminal.as_mut() {
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
        } else {
            terminal_unavailable_card(
                &mut body_ui,
                layout,
                terminal_error
                    .as_deref()
                    .unwrap_or("The terminal could not be started."),
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
                match terminal.relaunch(&self.platform, self.diag.log_mut()) {
                    Ok(()) => self.set_notice("Shell restarted."),
                    Err(error) => {
                        self.diag.log_mut().record(
                            crate::diagnostics::LogLevel::Error,
                            format!("shell relaunch failed: {error}"),
                        );
                        self.set_notice(format!("Shell relaunch failed: {error}"));
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

    fn terminal_path_line(&self, ui: &mut Ui, rect: egui::Rect, row: &SessionRow, layout: &Layout) {
        ui.painter().rect_filled(rect, 0.0, theme::BG_TERMINAL);
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

    fn workspace_diagnostics(&self) -> crate::diagnostics::WorkspaceDiagnostics {
        let label = self
            .plan
            .as_ref()
            .map(|plan| plan.workspace.display_name().to_owned())
            .unwrap_or_else(|| "Workspace unavailable".to_owned());
        let path = self
            .plan
            .as_ref()
            .map(|plan| plan.workspace.path().to_path_buf());
        let capability = self.shell_cwd_capability();
        crate::diagnostics::WorkspaceDiagnostics {
            identity: self.pane_id().to_owned(),
            label,
            path,
            shell_cwd: Some(self.shell_cwd().to_path_buf()),
            cwd_supported: capability.map(made_platform::ShellCwdCapability::is_supported),
            cwd_reason: capability.map(|capability| capability.reason.clone()),
            root_source: Some(self.root_source().to_owned()),
            watcher: self.explorer.as_ref().map(|explorer| {
                if explorer.has_watcher() {
                    "live".to_owned()
                } else if let Some(error) = explorer.watcher_error() {
                    format!("unavailable: {}", sanitize_display(error, 300))
                } else if explorer.watcher_loading() {
                    "starting".to_owned()
                } else {
                    "unavailable".to_owned()
                }
            }),
        }
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
        self.explorer_root().hash(&mut hasher);
        if let Some(worktree) = self.worktree() {
            worktree.status.label().hash(&mut hasher);
            worktree.branch.hash(&mut hasher);
            worktree.worktree_label.hash(&mut hasher);
            worktree.is_linked_worktree.hash(&mut hasher);
        }
        self.session_map.len().hash(&mut hasher);
        self.session_map.active_session_id().hash(&mut hasher);
        if let Some(bridge) = self.bridge.as_ref() {
            bridge.current_connection_id().hash(&mut hasher);
            bridge.dropped_events().hash(&mut hasher);
            bridge.coalesced_states().hash(&mut hasher);
        }
        if let Some(explorer) = self.explorer.as_ref() {
            explorer.has_watcher().hash(&mut hasher);
            explorer.watcher_loading().hash(&mut hasher);
            explorer.watcher_error().hash(&mut hasher);
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
        let mut snapshot = self.diagnostics_snapshot();
        snapshot.attach_workspace(self.workspace_diagnostics());
        let bridge_stats = self
            .bridge
            .as_ref()
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
        let copy = sanitize_copy_text(&snapshot.to_text(), self.token.as_ref(), 1024 * 1024);
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

    fn diagnostics_window(
        &mut self,
        ctx: &egui::Context,
        show_diagnostics: &mut bool,
        layout: &Layout,
    ) {
        if !*show_diagnostics {
            if ctx.data(|data| {
                data.get_temp::<bool>(diagnostics_open_id())
                    .unwrap_or(false)
            }) {
                ctx.data_mut(|data| data.insert_temp(diagnostics_open_id(), false));
            }
            return;
        }
        let was_open = ctx.data(|data| {
            data.get_temp::<bool>(diagnostics_open_id())
                .unwrap_or(false)
        });
        if !was_open {
            ctx.data_mut(|data| {
                data.remove::<Arc<DiagnosticsUiCache>>(diagnostics_cache_id(self.pane_id()));
            });
        }
        let cache = self.diagnostics_cache(ctx);
        let metrics_cache = self.diagnostics_metrics_cache(ctx);
        let snapshot = &cache.snapshot;
        let performance = metrics_cache.text.as_str();
        let bridge_stats = cache.bridge_stats.as_str();
        let heartbeat_age =
            heartbeat_age(self.integration.last_heartbeat(), std::time::Instant::now());
        let config_failure = self.config_failure.clone();
        let log_write_error = self.log_write_error.clone();
        let log_path = cache.log_path.as_str();
        let reason_full = snapshot.reason.clone();
        let reason = sanitize_display(&reason_full, 320);
        let content = ctx.content_rect();
        let width = (content.width() - 64.0).clamp(440.0, 680.0);
        let height = (content.height() - 64.0).clamp(340.0, 560.0);
        let position = content.center() - egui::vec2(width / 2.0, height / 2.0);
        let mut open = *show_diagnostics;
        let mut copy_requested = false;

        egui::Window::new("Diagnostics")
            .id(egui::Id::unique((
                "made-diagnostics-window",
                self.pane_id(),
            )))
            .open(&mut open)
            .resizable(true)
            .collapsible(false)
            .default_pos(position)
            .default_size(egui::vec2(width, height))
            .min_size(egui::vec2(420.0, 320.0))
            .max_width(width)
            .max_height(height)
            .frame(theme::floating_frame())
            .show(ctx, |ui| {
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
                if let Some(error) = &log_write_error {
                    notice_strip(
                        ui,
                        layout,
                        theme::ERROR_TINT,
                        theme::ERROR,
                        &format!("log: {}", sanitize_display(error, 400)),
                        &sanitize_display(error, 700),
                    );
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height((height - 150.0).max(120.0))
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
            });

        *show_diagnostics = open;
        if copy_requested {
            ctx.data_mut(|data| {
                data.remove::<Arc<DiagnosticsUiCache>>(diagnostics_cache_id(self.pane_id()));
            });
            let fresh = self.diagnostics_cache(ctx);
            ctx.copy_text(fresh.copy.clone());
        }
        ctx.data_mut(|data| data.insert_temp(diagnostics_open_id(), open));
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

fn terminal_unavailable_card(ui: &mut Ui, layout: &Layout, message: &str) {
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
            ui.label(theme::micro(
                "The explorer remains available; Made never blocks input on a terminal failure.",
            ));
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
                "Optional OpenCode integration is inactive.",
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
    fn compact_tab_width_and_picker_label_stay_within_targets() {
        let layout = Layout::for_window(720.0, 480.0);
        let row = row(0, true);
        assert!((112.0..=184.0).contains(&tab_width(&row, &layout)));
        assert_eq!(theme::format_bytes(4_300), "4.2 KB");
    }
}
