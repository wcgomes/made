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
use std::sync::{Arc, Mutex};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, RichText, Sense, Stroke, StrokeKind, Ui,
    UiBuilder,
};

use crate::app::{AppModel, WorkspaceId};
use crate::diagnostics::{DiagnosticsPill, DiagnosticsSnapshot};
use crate::explorer_view::{ExplorerPane, PreviewUiAction};
use crate::integration_status::IntegrationStatus;
use crate::integrations::{rank_order, IntegrationList, IntegrationRef, Mark, Presentation};
use crate::sanitize::{sanitize_copy_text, sanitize_display};
use crate::session_map::{truncate_end, truncate_path, SessionActivity, SessionRow};
use crate::terminal_view::TerminalUiOutput;
use crate::theme::{self, Layout};
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

fn integration_status_pill_ui(
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

// ── Open workspace screen ───────────────────────────────────────────────────

/// The Open workspace screen: shown for `+`, `--picker`, last-tab close and
/// the smoke surface without a folder.
pub fn picker_ui(model: &mut AppModel, ui: &mut Ui) {
    let layout = Layout::for_window(ui.available_width(), ui.available_height());
    open_workspace_surface(model, ui, &layout);
}

/// Stable id for an Open workspace screen control.
pub fn picker_control_id(action: &str) -> egui::Id {
    egui::Id::unique(("made-picker-control", action))
}

/// The folder tree's single focusable container.
pub fn picker_list_id() -> egui::Id {
    egui::Id::unique("made-picker-list-focus")
}

/// Stable id for the editable address bar.
pub fn picker_path_id() -> egui::Id {
    egui::Id::unique("made-picker-path")
}

/// Stable id for one host button of the host bar.
pub fn open_workspace_host_id(host: crate::open_workspace::HostId) -> egui::Id {
    match host {
        crate::open_workspace::HostId::Local => egui::Id::unique("made-host-local"),
        crate::open_workspace::HostId::Ssh(id) => {
            egui::Id::unique(("made-host-ssh", authority_key(id)))
        }
    }
}

/// Stable id for the `+ Add host` button.
pub fn open_workspace_add_host_id() -> egui::Id {
    egui::Id::unique("made-host-add")
}

/// Stable id for the `Host actions` (⋯) button of an SSH host.
pub fn host_actions_id(id: made_remote::protocol::AuthorityId) -> egui::Id {
    egui::Id::unique(("made-host-actions", authority_key(id)))
}

/// Stable id for one item of the `Host actions` menu.
pub fn host_action_item_id(
    id: made_remote::protocol::AuthorityId,
    action: crate::ssh_servers::HostAction,
) -> egui::Id {
    egui::Id::unique(("made-host-action", authority_key(id), action.label()))
}

/// The Worktrees list's single focusable container.
pub fn worktree_list_id() -> egui::Id {
    egui::Id::unique("made-worktree-list")
}

/// Stable id for one launch button: `terminal` or the TUI's menu item id.
pub fn launch_button_id(kind: crate::open_workspace::LaunchKind) -> egui::Id {
    egui::Id::unique(("made-launch", launch_key(kind)))
}

fn launch_key(kind: crate::open_workspace::LaunchKind) -> &'static str {
    match kind {
        crate::open_workspace::LaunchKind::Terminal => "terminal",
        crate::open_workspace::LaunchKind::Tui(tui) => tui.descriptor().menu_item_id,
    }
}

/// Stable id for one field of the host form.
pub fn host_form_field_id(field: crate::ssh_servers::HostField) -> egui::Id {
    egui::Id::unique(("made-host-form", format!("{field:?}")))
}

/// The effective launch path shown on the launch bar.
pub fn picker_current_target(model: &AppModel) -> Option<String> {
    model
        .open_workspace()
        .and_then(crate::open_workspace::OpenWorkspace::effective_launch_path)
        .or_else(|| {
            model
                .launch()
                .picker()
                .map(|picker| picker.path_target().to_path_buf())
        })
        .map(|path| path.display().to_string())
}

/// Folder rows and Worktrees rows are this tall (the Explorer row height).
pub const OPEN_WORKSPACE_ROW_HEIGHT: f32 = 24.0;
/// Horizontal and vertical window margins of the Open workspace card: 48/32
/// at the wide layout, 16/12 compact.
pub fn open_workspace_card_margins(compact: bool) -> (f32, f32) {
    if compact {
        (16.0, 12.0)
    } else {
        (48.0, 32.0)
    }
}

enum ScreenCommand {
    Up,
    Refresh,
    Toggle(usize),
    SelectHost(crate::open_workspace::HostId),
    AddHost,
    EditHost(made_remote::protocol::AuthorityId),
    TestHost(made_remote::protocol::AuthorityId),
    RemoveHost(made_remote::protocol::AuthorityId),
    SelectWorktree(usize),
    Launch(crate::open_workspace::LaunchKind),
    Navigate(String),
    Move(isize),
    Expand,
    Collapse,
    ToggleSelected,
    SaveForm,
    CancelForm,
    Removal(bool),
    Prompt(bool),
    Consent(bool),
    ForgetHostKey,
    DismissReport,
    DismissFailure,
    DismissNotice(crate::app::ScreenNotice),
    Close,
}

fn address_draft_id() -> egui::Id {
    egui::Id::unique("made-address-draft")
}

fn host_menu_id(id: made_remote::protocol::AuthorityId) -> egui::Id {
    egui::Id::unique(("made-host-menu", authority_key(id)))
}

fn host_icon_glyph(host: crate::open_workspace::HostId) -> theme::IconGlyph {
    match host {
        crate::open_workspace::HostId::Local => theme::IconGlyph::Computer,
        crate::open_workspace::HostId::Ssh(_) => theme::IconGlyph::Servers,
    }
}

fn connection_text(state: &crate::open_workspace::ConnectionState) -> String {
    use crate::open_workspace::ConnectionState as C;
    match state {
        C::NotConnected => "Not connected".to_owned(),
        C::Connecting => "Connecting".to_owned(),
        C::Connected => "Connected".to_owned(),
        C::HostKeyApprovalNeeded => "Host key approval needed".to_owned(),
        C::AuthenticationCancelled => "Authentication cancelled".to_owned(),
        C::Failed(reason) => format!("Connection failed: {}", sanitize_display(reason, 200)),
    }
}

fn connection_glyph(state: &crate::open_workspace::ConnectionState) -> theme::HostStatusGlyph {
    use crate::open_workspace::ConnectionState as C;
    match state {
        C::NotConnected => theme::HostStatusGlyph::NotConnected,
        C::Connecting => theme::HostStatusGlyph::Connecting,
        C::Connected => theme::HostStatusGlyph::Connected,
        C::HostKeyApprovalNeeded => theme::HostStatusGlyph::ApprovalNeeded,
        C::AuthenticationCancelled => theme::HostStatusGlyph::Cancelled,
        C::Failed(_) => theme::HostStatusGlyph::Failed,
    }
}

/// Tooltip and accessible name of a host button: kind, full name, full
/// address, connection state and detection.
pub fn host_accessible_label(host: &crate::app::ScreenHost) -> String {
    let kind = match host.id {
        crate::open_workspace::HostId::Local => "Local computer",
        crate::open_workspace::HostId::Ssh(_) => "SSH host",
    };
    let mut label = format!(
        "{kind}: {}, {}",
        sanitize_display(&host.name, 128),
        sanitize_display(&host.address, 300)
    );
    if let Some(state) = &host.connection {
        label.push_str(&format!(" — {}", connection_text(state)));
    }
    // Detection can still arrive only on Local or while connecting.
    let pending = matches!(
        host.connection,
        None | Some(
            crate::open_workspace::ConnectionState::Connecting
                | crate::open_workspace::ConnectionState::HostKeyApprovalNeeded
                | crate::open_workspace::ConnectionState::Connected
        )
    );
    match &host.detected {
        None if pending => label.push_str(" — detection pending"),
        None => {}
        Some(found) if !found.is_empty() => {
            let names: Vec<&str> = found.iter().map(|tui| tui.descriptor().label).collect();
            label.push_str(&format!(" — detected: {}", names.join(", ")));
        }
        Some(_) => {}
    }
    label
}

/// Colored dot of a change pill: the status color of its git group.
pub fn pill_dot_color(kind: made_remote::git::PillKind) -> Color32 {
    match kind {
        made_remote::git::PillKind::Conflicted => theme::ERROR,
        made_remote::git::PillKind::Staged => theme::INFO,
        made_remote::git::PillKind::Modified => theme::WARNING,
        made_remote::git::PillKind::Untracked => theme::SUCCESS,
    }
}

const PILL_HEIGHT: f32 = 18.0;
const PILL_GAP: f32 = 4.0;

fn pill_width(ui: &Ui, text: &str) -> f32 {
    let text = ui
        .painter()
        .layout_no_wrap(
            text.to_owned(),
            FontId::proportional(theme::SMALL_SIZE),
            theme::TEXT_SECONDARY,
        )
        .size()
        .x;
    6.0 + 6.0 + 5.0 + text + 7.0
}

fn pills_width(ui: &Ui, pills: &[(made_remote::git::PillKind, usize)]) -> f32 {
    pills
        .iter()
        .map(|(kind, count)| pill_width(ui, &format!("{count} {}", kind.label())))
        .sum::<f32>()
        + PILL_GAP * pills.len().saturating_sub(1) as f32
}

/// Paint change pills from `left` on the row centered at `center_y`.
fn paint_pills(
    ui: &Ui,
    left: f32,
    center_y: f32,
    pills: &[(made_remote::git::PillKind, usize)],
) -> f32 {
    let mut x = left;
    for (kind, count) in pills {
        let text = format!("{count} {}", kind.label());
        let width = pill_width(ui, &text);
        let rect = egui::Rect::from_min_size(
            egui::pos2(x, center_y - PILL_HEIGHT / 2.0),
            egui::vec2(width, PILL_HEIGHT),
        );
        ui.painter().rect(
            rect,
            CornerRadius::same((PILL_HEIGHT / 2.0) as u8),
            theme::BG_ELEVATED,
            Stroke::new(1.0, theme::BORDER),
            StrokeKind::Inside,
        );
        ui.painter().circle_filled(
            egui::pos2(rect.left() + 9.0, rect.center().y),
            3.0,
            pill_dot_color(*kind),
        );
        ui.painter().text(
            egui::pos2(rect.left() + 17.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(theme::SMALL_SIZE),
            theme::TEXT_SECONDARY,
        );
        x = rect.right() + PILL_GAP;
    }
    x
}

fn counts_pills(
    counts: &crate::open_workspace::CountsState,
) -> Vec<(made_remote::git::PillKind, usize)> {
    match counts {
        crate::open_workspace::CountsState::Ready(counts) => counts.nonzero(),
        _ => Vec::new(),
    }
}

fn counts_summary(counts: &crate::open_workspace::CountsState) -> Option<String> {
    match counts {
        crate::open_workspace::CountsState::Ready(counts) => counts.summary(),
        _ => None,
    }
}

/// Middle-ellipsize `text` to fit `width` in `font`.
fn middle_ellipsized(ui: &Ui, text: &str, font: &FontId, width: f32) -> String {
    let full = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), theme::TEXT_PRIMARY)
        .size()
        .x;
    if full <= width {
        return text.to_owned();
    }
    let chars = text.chars().count().max(1);
    let per_char = full / chars as f32;
    let budget = ((width / per_char).floor() as usize).max(3);
    theme::truncate_middle(text, budget)
}

/// Rows of the launch area, top to bottom: target row height (`Launch in`),
/// gap, button row height, and the top/bottom padding (wide 20/6/26, 8/10;
/// compact 18/4/24, 6/8).
pub fn launch_area_metrics(compact: bool) -> (f32, f32, f32, f32, f32) {
    if compact {
        (18.0, 4.0, 24.0, 6.0, 8.0)
    } else {
        (20.0, 6.0, 26.0, 8.0, 10.0)
    }
}

fn open_workspace_surface(model: &mut AppModel, ui: &mut Ui, layout: &Layout) {
    let mut command: Option<ScreenCommand> = None;
    let screen = ui.max_rect();
    // The card grows with the window inside margins that let the backdrop
    // show around it.
    let (margin_x, margin_y) = open_workspace_card_margins(layout.compact);
    let card = egui::Rect::from_min_max(
        screen.min + egui::vec2(margin_x, margin_y),
        egui::pos2(
            (screen.right() - margin_x).max(screen.left() + margin_x + 1.0),
            (screen.bottom() - margin_y).max(screen.top() + margin_y + 1.0),
        ),
    );
    let native_backdrop_applied = model.native_backdrop().is_applied();
    theme::paint_glass_card(ui.painter(), card, layout, native_backdrop_applied);
    let Some(open) = model.open_workspace() else {
        return;
    };
    let selected_host = open.selected_host();
    let compact = layout.compact;
    let pad_x = if compact { 10.0 } else { 12.0 };
    let separator = |ui: &Ui, y: f32| {
        ui.painter().line_segment(
            [
                egui::pos2(card.left() + 1.0, y - 0.5),
                egui::pos2(card.right() - 1.0, y - 0.5),
            ],
            Stroke::new(1.0, theme::BORDER_HEADER),
        );
    };

    // Brand row; its controls register last for the Tab order.
    let brand_height = if compact { 32.0 } else { 40.0 };
    let brand = egui::Rect::from_min_size(card.min, egui::vec2(card.width(), brand_height));
    theme::paint_glass_card_header(ui, brand, layout, native_backdrop_applied);
    theme::paint_card_border(ui, card, layout);

    // Host bar.
    let host_pad_y = if compact { 6.0 } else { 8.0 };
    let host_height = if compact { 40.0 } else { 44.0 };
    let host_bar = egui::Rect::from_min_size(
        egui::pos2(card.left(), brand.bottom()),
        egui::vec2(card.width(), host_height + 2.0 * host_pad_y),
    );
    host_bar_ui(model, ui, host_bar, pad_x, host_height, &mut command);
    separator(ui, host_bar.bottom());

    // Everything below the host bar is laid out top-down in a child ui.
    let body = egui::Rect::from_min_max(
        egui::pos2(card.left() + 1.0, host_bar.bottom()),
        egui::pos2(card.right() - 1.0, card.bottom() - 1.0),
    );
    let mut body_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(body)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    body_ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);

    let (target_row, launch_gap, button_row, launch_top, launch_bottom) =
        launch_area_metrics(compact);
    let launch_height = launch_top + target_row + launch_gap + button_row + launch_bottom + 1.0;
    let worktrees_shown = model
        .open_workspace()
        .is_some_and(crate::open_workspace::OpenWorkspace::worktrees_shown);
    let worktree_count = model
        .open_workspace()
        .map_or(0, |open| open.worktrees().len());
    let wt_head = if compact { 22.0 } else { 26.0 };
    let wt_pad = if compact { 4.0 } else { 10.0 };
    let toolbar_height = if compact { 36.0 } else { 40.0 };
    let tree_pad = if compact { 2.0 } else { 4.0 };
    let min_tree = 4.0 * OPEN_WORKSPACE_ROW_HEIGHT + 2.0 * tree_pad;

    // Transient panels above the folder area: form, removal, prompt, report.
    let panels_budget = (body.height()
        - launch_height
        - toolbar_height
        - min_tree
        - if worktrees_shown {
            1.0 + wt_head + wt_pad + 2.0 * (OPEN_WORKSPACE_ROW_HEIGHT + 1.0)
        } else {
            0.0
        })
    .max(48.0);
    egui::ScrollArea::vertical()
        .id_salt("made-open-workspace-panels")
        .max_height(panels_budget)
        .auto_shrink([false, true])
        .show(&mut body_ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            screen_panels(model, ui, layout, selected_host, &mut command);
        });

    toolbar_ui(
        model,
        &mut body_ui,
        layout,
        toolbar_height,
        pad_x,
        &mut command,
    );
    let remaining = body_ui.available_height();
    let wt_rows_space = if worktrees_shown {
        let natural = worktree_count as f32 * (OPEN_WORKSPACE_ROW_HEIGHT + 1.0);
        let max = (remaining - launch_height - min_tree - 1.0 - wt_head - wt_pad)
            .max(2.0 * (OPEN_WORKSPACE_ROW_HEIGHT + 1.0));
        natural
            .min(max)
            .min(6.0 * (OPEN_WORKSPACE_ROW_HEIGHT + 1.0))
    } else {
        0.0
    };
    let wt_height = if worktrees_shown {
        1.0 + wt_head + wt_pad + wt_rows_space
    } else {
        0.0
    };
    let tree_height = (remaining - launch_height - wt_height).max(OPEN_WORKSPACE_ROW_HEIGHT);
    folder_area_ui(
        model,
        &mut body_ui,
        layout,
        tree_height,
        tree_pad,
        &mut command,
    );
    if worktrees_shown {
        worktrees_ui(
            model,
            &mut body_ui,
            layout,
            wt_head,
            wt_rows_space,
            &mut command,
        );
    }
    launch_bar_ui(model, &mut body_ui, layout, pad_x, &mut command);

    // Brand row controls last: Close ends the Tab order.
    brand_row_ui(ui, layout, brand, pad_x, &mut command);

    screen_keyboard(model, ui, &mut command);
    apply_screen_command(model, ui, command);
}

fn brand_row_ui(
    ui: &mut Ui,
    layout: &Layout,
    rect: egui::Rect,
    pad_x: f32,
    command: &mut Option<ScreenCommand>,
) {
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(egui::Rect::from_min_max(
                rect.min + egui::vec2(pad_x, 0.0),
                rect.max - egui::vec2(pad_x - 4.0, 0.0),
            ))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 8.0;
    theme::wordmark_brand(&mut row);
    row.label(theme::micro(APP_VERSION));
    let (divider, _) = row.allocate_exact_size(egui::vec2(1.0, 14.0), Sense::hover());
    row.painter().rect_filled(divider, 0.0, theme::BORDER);
    row.label(
        RichText::new("Open workspace")
            .font(FontId::proportional(13.0))
            .color(theme::TEXT_PRIMARY),
    );
    row.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let close = theme::icon_action(
            ui,
            picker_control_id("close"),
            theme::IconGlyph::Close,
            "Close",
            false,
            layout.button_height,
            theme::BG_CARD,
        );
        if close.activated {
            *command = Some(ScreenCommand::Close);
        }
    });
}

fn host_bar_ui(
    model: &mut AppModel,
    ui: &mut Ui,
    rect: egui::Rect,
    pad_x: f32,
    height: f32,
    command: &mut Option<ScreenCommand>,
) {
    let hosts = model.screen_hosts();
    let selected = model
        .open_workspace()
        .map(crate::open_workspace::OpenWorkspace::selected_host);
    let adding = model
        .screen_form()
        .is_some_and(|form| form.editing.is_none());
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.left() + pad_x, rect.top()),
        egui::pos2(rect.right() - pad_x, rect.bottom()),
    );
    let mut bar = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    let mut order: Vec<egui::Id> = hosts
        .iter()
        .map(|host| open_workspace_host_id(host.id))
        .collect();
    order.push(open_workspace_add_host_id());
    let focused = ui.ctx().memory(|memory| memory.focused());
    let step = ui.input(|input| {
        if input.key_pressed(egui::Key::ArrowRight) {
            1
        } else if input.key_pressed(egui::Key::ArrowLeft) {
            -1
        } else {
            0
        }
    });
    if let Some(index) = focused.and_then(|focused| order.iter().position(|id| *id == focused)) {
        if step != 0 {
            let next = (index as isize + step).clamp(0, order.len() as isize - 1) as usize;
            ui.ctx()
                .memory_mut(|memory| memory.request_focus(order[next]));
        }
    }
    egui::ScrollArea::horizontal()
        .id_salt("made-host-bar")
        .auto_shrink([false, false])
        .max_height(inner.height())
        .show(&mut bar, |ui| {
            ui.set_min_height(inner.height());
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for host in &hosts {
                    host_button_ui(ui, host, Some(host.id) == selected, height, command);
                }
                add_host_button_ui(ui, height, adding, command);
            });
        });
}

fn host_button_ui(
    ui: &mut Ui,
    host: &crate::app::ScreenHost,
    selected: bool,
    height: f32,
    command: &mut Option<ScreenCommand>,
) {
    let id = open_workspace_host_id(host.id);
    let ssh = match host.id {
        crate::open_workspace::HostId::Ssh(id) => Some(id),
        crate::open_workspace::HostId::Local => None,
    };
    let name = sanitize_display(&host.name, 128);
    let address = sanitize_display(&host.address, 300);
    let name_font = FontId::proportional(theme::BODY_SIZE);
    let address_font = FontId::monospace(10.5);
    let marks: Vec<IntegrationRef> = host.detected.clone().unwrap_or_default();
    let name_width = ui
        .painter()
        .layout_no_wrap(name.clone(), name_font.clone(), theme::TEXT_SECONDARY)
        .size()
        .x
        .min(220.0);
    let marks_width = if marks.is_empty() {
        0.0
    } else {
        2.0 + marks.len() as f32 * 16.0
    };
    let address_width = ui
        .painter()
        .layout_no_wrap(address.clone(), address_font.clone(), theme::TEXT_TERTIARY)
        .size()
        .x
        .min(220.0);
    let text_width = (name_width + marks_width).max(address_width);
    let actions_width = if ssh.is_some() { 6.0 + 24.0 - 4.0 } else { 0.0 };
    let width = 10.0 + 16.0 + 8.0 + text_width + actions_width + 10.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    ui.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            id,
            egui::EventFilter {
                tab: false,
                horizontal_arrows: true,
                vertical_arrows: false,
                escape: false,
            },
        );
    });
    let actions_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - 10.0 + 4.0 - 24.0, rect.center().y - 12.0),
        egui::vec2(24.0, 24.0),
    );
    let menu_open = ssh.is_some_and(|ssh| egui::Popup::is_id_open(ui.ctx(), host_menu_id(ssh)));
    let hovered = response.hovered() || ui.rect_contains_pointer(rect);
    let fill = if selected {
        theme::BG_SELECTED
    } else if hovered {
        theme::BG_HOVER
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    let text_color = if selected || hovered {
        theme::TEXT_PRIMARY
    } else {
        theme::TEXT_SECONDARY
    };
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 10.0, rect.center().y - 8.0),
        egui::Vec2::splat(16.0),
    );
    theme::paint_icon(
        ui.painter(),
        icon_rect,
        host_icon_glyph(host.id),
        if selected {
            theme::TEXT_PRIMARY
        } else {
            theme::TEXT_TERTIARY
        },
    );
    if let Some(state) = &host.connection {
        ui.painter().extend(theme::host_status_shapes(
            icon_rect.right_bottom() + egui::vec2(-2.0, -2.0),
            connection_glyph(state),
            if fill == Color32::TRANSPARENT {
                theme::BG_CARD
            } else {
                fill
            },
        ));
    }
    let text_left = icon_rect.right() + 8.0;
    let name_center = rect.center().y - 7.5;
    let address_center = rect.center().y + 7.5;
    theme::paint_ellipsized_centered(
        ui.painter(),
        text_left,
        name_center,
        &name,
        name_font,
        text_color,
        name_width.max(1.0),
    );
    let mut mark_x = text_left + name_width + 4.0;
    for tui in &marks {
        let leading = mark_leading(tui.descriptor().mark);
        let mark_rect = egui::Rect::from_center_size(
            egui::pos2(mark_x + 6.0, name_center),
            egui::Vec2::splat(12.0),
        );
        if let theme::MenuLeading::Image { uri, bytes } = leading {
            egui::Image::from_bytes(uri, bytes).paint_at(ui, mark_rect);
        }
        mark_x += 16.0;
    }
    theme::paint_ellipsized_centered(
        ui.painter(),
        text_left,
        address_center,
        &address,
        address_font,
        theme::TEXT_TERTIARY,
        address_width.max(1.0),
    );
    let label = host_accessible_label(host);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, true, selected, label.clone())
    });
    if response.has_focus() {
        theme::paint_focus_ring(ui, rect, CornerRadius::same(6), theme::BG_CARD);
        response.scroll_to_me_animation(None, egui::style::ScrollAnimation::none());
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let keyboard = response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        });
    if (response.clicked() || keyboard) && !selected {
        *command = Some(ScreenCommand::SelectHost(host.id));
    }
    let Some(ssh) = ssh else {
        response.on_hover_text(label);
        return;
    };
    let context_key = response.has_focus()
        && ui.input(|input| {
            (input.key_pressed(egui::Key::F10) && input.modifiers.shift)
                || input.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            key: egui::Key::F10,
                            pressed: true,
                            modifiers,
                            ..
                        } if modifiers.shift
                    )
                })
        });
    let response = response.on_hover_text(label);
    response.context_menu(|ui| host_actions_menu(ui, ssh, command));
    // The ⋯ button is revealed by hover or focus and stays a 24×24 target.
    let actions_focused = ui
        .ctx()
        .memory(|memory| memory.has_focus(host_actions_id(ssh)));
    let reveal = hovered || response.has_focus() || actions_focused || menu_open;
    let actions = ui.interact(actions_rect, host_actions_id(ssh), Sense::click());
    if reveal {
        let open = menu_open;
        let action_fill = if open || actions.hovered() {
            theme::BG_HOVER
        } else {
            Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(actions_rect, CornerRadius::same(6), action_fill);
        theme::paint_icon(
            ui.painter(),
            actions_rect,
            theme::IconGlyph::More,
            if open || actions.hovered() {
                theme::TEXT_PRIMARY
            } else {
                theme::TEXT_TERTIARY
            },
        );
    }
    if actions.has_focus() {
        theme::paint_focus_ring(ui, actions_rect, CornerRadius::same(6), fill);
    }
    let actions_keyboard = actions.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        });
    let toggle = actions.clicked() || actions_keyboard || context_key;
    let actions = actions.on_hover_text("Host actions");
    actions.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Host actions"));
    egui::Popup::menu(&actions)
        .id(host_menu_id(ssh))
        .open_memory(toggle.then_some(egui::SetOpenCommand::Toggle))
        .show(|ui| host_actions_menu(ui, ssh, command));
}

fn host_actions_menu(
    ui: &mut Ui,
    ssh: made_remote::protocol::AuthorityId,
    command: &mut Option<ScreenCommand>,
) {
    ui.set_min_width(168.0);
    for action in crate::ssh_servers::host_actions(crate::open_workspace::HostId::Ssh(ssh)) {
        if *action == crate::ssh_servers::HostAction::Remove {
            ui.separator();
        }
        let label = match action {
            crate::ssh_servers::HostAction::TestConnection => action.label().to_owned(),
            _ => format!("{}…", action.label()),
        };
        let item = theme::ghost_action(
            ui,
            host_action_item_id(ssh, *action),
            &label,
            false,
            26.0,
            theme::BG_CARD,
        );
        if item.activated {
            *command = Some(match action {
                crate::ssh_servers::HostAction::Edit => ScreenCommand::EditHost(ssh),
                crate::ssh_servers::HostAction::TestConnection => ScreenCommand::TestHost(ssh),
                crate::ssh_servers::HostAction::Remove => ScreenCommand::RemoveHost(ssh),
            });
            ui.close();
        }
    }
}

fn add_host_button_ui(ui: &mut Ui, height: f32, open: bool, command: &mut Option<ScreenCommand>) {
    let id = open_workspace_add_host_id();
    let font = FontId::proportional(theme::BUTTON_SIZE);
    let text = ui
        .painter()
        .layout_no_wrap("Add host".to_owned(), font.clone(), theme::TEXT_TERTIARY)
        .size()
        .x;
    let width = 8.0 + 16.0 + 6.0 + text + 12.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    ui.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            id,
            egui::EventFilter {
                tab: false,
                horizontal_arrows: true,
                vertical_arrows: false,
                escape: false,
            },
        );
    });
    let hovered = response.hovered();
    let fill = if open {
        theme::BG_SELECTED
    } else if hovered {
        theme::BG_HOVER
    } else {
        Color32::TRANSPARENT
    };
    let color = if open || hovered {
        theme::TEXT_PRIMARY
    } else {
        theme::TEXT_TERTIARY
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    let icon = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 8.0, rect.center().y - 8.0),
        egui::Vec2::splat(16.0),
    );
    theme::paint_icon(ui.painter(), icon, theme::IconGlyph::Plus, color);
    ui.painter().text(
        egui::pos2(icon.right() + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Add host",
        font,
        color,
    );
    if response.has_focus() {
        theme::paint_focus_ring(ui, rect, CornerRadius::same(6), theme::BG_CARD);
        response.scroll_to_me_animation(None, egui::style::ScrollAnimation::none());
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let keyboard = response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        });
    let response = response.on_hover_text("Add an SSH host");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Add host"));
    if response.clicked() || keyboard {
        *command = Some(ScreenCommand::AddHost);
    }
}

fn screen_panels(
    model: &mut AppModel,
    ui: &mut Ui,
    layout: &Layout,
    selected_host: crate::open_workspace::HostId,
    command: &mut Option<ScreenCommand>,
) {
    let pad_x = if layout.compact { 10.0 } else { 12.0 };
    if model.screen_form().is_some() {
        host_form_ui(model, ui, layout, command);
    }
    if let Some(message) = model.screen_removal().map(str::to_owned) {
        panel_frame(ui, pad_x, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(theme::body(sanitize_display(&message, 300)));
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    let remove = theme::ghost_action(
                        ui,
                        picker_control_id("remove-confirm"),
                        "Remove",
                        false,
                        layout.button_height,
                        theme::BG_HEADER,
                    );
                    if remove.activated {
                        *command = Some(ScreenCommand::Removal(true));
                    }
                    let cancel = theme::ghost_action(
                        ui,
                        picker_control_id("remove-cancel"),
                        "Cancel",
                        false,
                        layout.button_height,
                        theme::BG_HEADER,
                    );
                    if cancel.activated {
                        *command = Some(ScreenCommand::Removal(false));
                    }
                });
            });
        });
    }
    if let Some(text) = model.screen_prompt().map(|prompt| prompt.text.clone()) {
        let class = crate::remote_session::classify_prompt(&text);
        let changed = matches!(
            class,
            crate::remote_session::SshPromptClass::ChangedHostKey { .. }
        );
        panel_frame(ui, pad_x, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            match &class {
                crate::remote_session::SshPromptClass::ChangedHostKey { .. } => {
                    ui.label(theme::body(
                        "The host key of this server changed. Made refused it.",
                    ));
                }
                crate::remote_session::SshPromptClass::UnknownHost { text } => {
                    ui.label(theme::body(
                        "This host is not trusted yet. Approve only if the fingerprint matches.",
                    ));
                    ui.label(theme::mono(
                        sanitize_display(text, 700),
                        10.5,
                        theme::TEXT_SECONDARY,
                    ));
                }
                crate::remote_session::SshPromptClass::Secret { text } => {
                    ui.label(theme::body(sanitize_display(text, 400)));
                    let edit = egui::TextEdit::singleline(model.prompt_draft())
                        .id(ssh_field_id("screen-prompt"))
                        .password(true)
                        .desired_width(f32::INFINITY);
                    ui.add(edit);
                }
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if !changed {
                    let approve = theme::primary_action(
                        ui,
                        picker_control_id("prompt-approve"),
                        "Continue",
                        layout.button_height,
                        true,
                        theme::BG_HEADER,
                    );
                    if approve.activated {
                        *command = Some(ScreenCommand::Prompt(true));
                    }
                }
                let cancel = theme::ghost_action(
                    ui,
                    picker_control_id("prompt-cancel"),
                    if changed { "Refuse key" } else { "Cancel" },
                    false,
                    layout.button_height,
                    theme::BG_HEADER,
                );
                if cancel.activated {
                    *command = Some(ScreenCommand::Prompt(false));
                }
            });
        });
    }
    if let crate::open_workspace::HostId::Ssh(ssh) = selected_host {
        if model.ssh_selected() == Some(ssh) && model.ssh_host_key_changed() {
            panel_frame(ui, pad_x, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.label(theme::body(
                    "The stored key no longer matches this server. Forget it only if the server key change was expected.",
                ));
                let forget = theme::secondary_action(
                    ui,
                    picker_control_id("forget-host-key"),
                    "Forget old host key",
                    layout.button_height,
                    true,
                    theme::BG_HEADER,
                );
                if forget.activated {
                    *command = Some(ScreenCommand::ForgetHostKey);
                }
            });
        }
        if model.screen_consent_pending() {
            panel_frame(ui, pad_x, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.label(theme::body(
                    "Browsing this server needs Made's helper there. Approve to stage it for this server and version; no password is written to disk.",
                ));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let approve = theme::primary_action(
                        ui,
                        picker_control_id("consent-approve"),
                        "Approve",
                        layout.button_height,
                        true,
                        theme::BG_HEADER,
                    );
                    if approve.activated {
                        *command = Some(ScreenCommand::Consent(true));
                    }
                    let refuse = theme::ghost_action(
                        ui,
                        picker_control_id("consent-refuse"),
                        "Not now",
                        false,
                        layout.button_height,
                        theme::BG_HEADER,
                    );
                    if refuse.activated {
                        *command = Some(ScreenCommand::Consent(false));
                    }
                });
            });
        }
        // BEGIN remote-path (AIA-02)
        if model.ssh_selected() == Some(ssh) {
            panel_frame(ui, pad_x, |ui| grok_install_consent_card(model, ui, layout));
        }
        // END remote-path
    }
}

/// A panel row of the screen. A panel whose content added nothing is not
/// painted, so the screen never shows an empty notice line.
fn panel_frame(ui: &mut Ui, pad_x: f32, add: impl FnOnce(&mut Ui)) {
    let mut prepared = egui::Frame::new()
        .fill(theme::BG_HEADER)
        .inner_margin(egui::Margin::symmetric(pad_x as i8, 8))
        .begin(ui);
    prepared
        .content_ui
        .set_width(prepared.content_ui.available_width());
    add(&mut prepared.content_ui);
    if prepared.content_ui.min_rect().height() < 1.0 {
        return;
    }
    prepared.end(ui);
    let rect = ui.min_rect();
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.bottom() - 0.5),
            egui::pos2(rect.right(), rect.bottom() - 0.5),
        ],
        Stroke::new(1.0, theme::BORDER_HEADER),
    );
}

fn host_form_ui(
    model: &mut AppModel,
    ui: &mut Ui,
    layout: &Layout,
    command: &mut Option<ScreenCommand>,
) {
    use crate::ssh_servers::HostField;
    let Some(state) = model.screen_form().cloned() else {
        return;
    };
    let pad_x = if layout.compact { 10.0 } else { 16.0 };
    let mut test = false;
    panel_frame(ui, pad_x, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(12.0, 6.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if state.editing.is_some() {
                    "EDIT SSH HOST"
                } else {
                    "ADD SSH HOST"
                })
                .font(FontId::proportional(theme::SECTION_SIZE))
                .color(theme::TEXT_TERTIARY),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.label(theme::micro(
                    "Saved hosts keep no passwords, keys or prompt answers.",
                ));
            });
        });
        let fields: [(HostField, &str, bool); 6] = [
            (HostField::Name, "Display name", false),
            (HostField::Host, "Host or SSH alias", false),
            (HostField::User, "User", true),
            (HostField::Port, "Port", true),
            (HostField::KeyFile, "Key file", true),
            (HostField::Directory, "Initial remote directory", true),
        ];
        let columns = 3;
        let gap = 12.0;
        let width =
            ((ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32).max(80.0);
        for chunk in fields.chunks(columns) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (field, label, optional) in chunk {
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, 0.0),
                        egui::Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(width);
                            ui.spacing_mut().item_spacing.y = 4.0;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                ui.label(theme::micro(*label));
                                if *optional {
                                    ui.label(
                                        RichText::new("optional")
                                            .font(FontId::proportional(theme::SECTION_SIZE))
                                            .color(theme::TEXT_DISABLED),
                                    );
                                }
                            });
                            if let Some(form) = model.screen_form_mut() {
                                let text = match field {
                                    HostField::Name => &mut form.name,
                                    HostField::Host => &mut form.host,
                                    HostField::User => &mut form.user,
                                    HostField::Port => &mut form.port,
                                    HostField::KeyFile => &mut form.key_file,
                                    HostField::Directory => &mut form.directory,
                                };
                                let response = ui.add(
                                    egui::TextEdit::singleline(text)
                                        .id(host_form_field_id(*field))
                                        .desired_width(width)
                                        .margin(egui::Margin::symmetric(8, 6)),
                                );
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(egui::Role::TextInput, true, *label)
                                });
                                if response.lost_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter))
                                {
                                    *command = Some(ScreenCommand::SaveForm);
                                }
                            }
                            if let Some((_, error)) =
                                state.errors.iter().find(|(errored, _)| errored == field)
                            {
                                ui.label(
                                    RichText::new(*error)
                                        .font(FontId::proportional(theme::SECTION_SIZE))
                                        .color(theme::ERROR),
                                );
                            }
                        },
                    );
                }
            });
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if state.editing.is_some() {
                let action = theme::secondary_action(
                    ui,
                    picker_control_id("form-test"),
                    "Test connection",
                    layout.button_height,
                    true,
                    theme::BG_HEADER,
                );
                test = action.activated;
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let save = theme::primary_action(
                    ui,
                    picker_control_id("form-save"),
                    "Save host",
                    layout.button_height,
                    true,
                    theme::BG_HEADER,
                );
                if save.activated {
                    *command = Some(ScreenCommand::SaveForm);
                }
                let cancel = theme::ghost_action(
                    ui,
                    picker_control_id("form-cancel"),
                    "Cancel",
                    false,
                    layout.button_height,
                    theme::BG_HEADER,
                );
                if cancel.activated {
                    *command = Some(ScreenCommand::CancelForm);
                }
            });
        });
    });
    if test {
        if let Some(id) = state.editing {
            *command = Some(ScreenCommand::TestHost(id));
        }
    }
}

fn toolbar_ui(
    model: &mut AppModel,
    ui: &mut Ui,
    layout: &Layout,
    height: f32,
    pad_x: f32,
    command: &mut Option<ScreenCommand>,
) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let mut bar = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(pad_x, 0.0)))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    bar.spacing_mut().item_spacing.x = 4.0;
    let up = theme::icon_action(
        &mut bar,
        picker_control_id("up"),
        theme::IconGlyph::Up,
        "Go to the parent folder",
        false,
        layout.button_height,
        theme::BG_CARD,
    );
    if up.activated {
        *command = Some(ScreenCommand::Up);
    }
    let refresh = theme::icon_action(
        &mut bar,
        picker_control_id("refresh"),
        theme::IconGlyph::Refresh,
        "Refresh folders",
        false,
        layout.button_height,
        theme::BG_CARD,
    );
    if refresh.activated {
        *command = Some(ScreenCommand::Refresh);
    }
    bar.add_space(6.0);
    let (divider, _) = bar.allocate_exact_size(egui::vec2(1.0, 14.0), Sense::hover());
    bar.painter().rect_filled(divider, 0.0, theme::BORDER);
    bar.add_space(6.0);

    let target = model
        .open_workspace()
        .and_then(crate::open_workspace::OpenWorkspace::launch_target)
        .map(|path| sanitize_display(&path.display().to_string(), 900))
        .unwrap_or_default();
    let error = model.screen_address_error();
    let id = picker_path_id();
    let editing = bar.ctx().memory(|memory| memory.has_focus(id));
    let mut draft = bar
        .ctx()
        .data(|data| data.get_temp::<String>(address_draft_id()))
        .unwrap_or_default();
    if !editing && error.is_none() {
        draft = target.clone();
    }
    let field_height = if layout.compact { 24.0 } else { 26.0 };
    let (field, _) = bar.allocate_exact_size(
        egui::vec2(bar.available_width().max(40.0), field_height),
        Sense::hover(),
    );
    bar.painter()
        .rect_filled(field, CornerRadius::same(4), theme::BG_SELECTED);
    let text_rect = field.shrink2(egui::vec2(8.0, 0.0));
    let text_rect = if error.is_some() {
        egui::Rect::from_min_max(text_rect.min, text_rect.max - egui::vec2(20.0, 0.0))
    } else {
        text_rect
    };
    let mut field_ui = bar.new_child(
        UiBuilder::new()
            .max_rect(text_rect)
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    field_ui.spacing_mut().interact_size.y = field_height;
    let output = egui::TextEdit::singleline(&mut draft)
        .id(id)
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::ZERO)
        .align(Align2::LEFT_CENTER)
        .font(FontId::monospace(10.5))
        .text_color(theme::TEXT_PRIMARY)
        .desired_width(text_rect.width())
        .clip_text(true)
        .show(&mut field_ui);
    let response = output.response.response.clone();
    if response.gained_focus() {
        let mut state = output.state;
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(draft.chars().count()),
            )));
        state.store(bar.ctx(), id);
    }
    let enter = response.lost_focus() && bar.input(|input| input.key_pressed(egui::Key::Enter));
    let escape = response.lost_focus() && bar.input(|input| input.key_pressed(egui::Key::Escape));
    if enter {
        *command = Some(ScreenCommand::Navigate(draft.clone()));
        response.request_focus();
    }
    if escape {
        draft = target.clone();
        model.screen_clear_address_error();
        bar.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique("made-address-escape"), true));
    }
    bar.ctx()
        .data_mut(|data| data.insert_temp(address_draft_id(), draft));
    if response.has_focus() {
        theme::paint_focus_ring(&bar, field, CornerRadius::same(4), theme::BG_CARD);
    }
    if let Some(error) = error {
        let center = egui::pos2(field.right() - 12.0, field.center().y);
        bar.painter().circle_filled(center, 7.0, theme::ERROR);
        bar.painter().text(
            center,
            Align2::CENTER_CENTER,
            "!",
            FontId::monospace(11.0),
            theme::ACCENT_TEXT,
        );
        let _ = error;
    }
    let response = response.on_hover_text(if target.is_empty() {
        "Type an absolute folder path and press Enter".to_owned()
    } else {
        target.clone()
    });
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::TextInput, true, format!("Folder path {target}"))
    });
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.bottom() - 0.5),
            egui::pos2(rect.right(), rect.bottom() - 0.5),
        ],
        Stroke::new(1.0, theme::BORDER_HEADER),
    );
}

fn folder_area_ui(
    model: &mut AppModel,
    ui: &mut Ui,
    layout: &Layout,
    height: f32,
    pad: f32,
    command: &mut Option<ScreenCommand>,
) {
    let (area, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let mut area_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(area)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    area_ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    let pad_x = if layout.compact { 10.0 } else { 12.0 };
    // Fill, tone, text, tooltip and the dismiss control with its command.
    let mut notices: Vec<(Color32, Color32, String, String, Option<NoticeDismiss>)> = Vec::new();
    let mut error = |text: String, dismiss: NoticeDismiss| {
        notices.push((
            theme::ERROR_TINT,
            theme::ERROR,
            text.clone(),
            text,
            Some(dismiss),
        ));
    };
    if let Some(failure) = model.screen_connection_failure() {
        error(sanitize_display(failure, 600), NoticeDismiss::Failure);
    }
    if let Some((tested, report)) = model.screen_test_report() {
        let name = model
            .screen_hosts()
            .into_iter()
            .find(|host| host.id == crate::open_workspace::HostId::Ssh(tested))
            .map(|host| sanitize_display(&host.name, 128))
            .unwrap_or_default();
        let report = sanitize_display(report, 600);
        error(
            if name.is_empty() {
                report
            } else {
                format!("{name}: {report}")
            },
            NoticeDismiss::Report,
        );
    }
    for (kind, text) in model.screen_visible_notices() {
        let text = match kind {
            crate::app::ScreenNotice::Address => format!(
                "{} · Press Esc to restore {}.",
                text,
                sanitize_display(&picker_current_target(model).unwrap_or_default(), 300)
            ),
            _ => sanitize_display(&text, 600),
        };
        error(text, NoticeDismiss::Notice(kind));
    }
    let truncated = model.screen_truncated();
    if truncated > 0 {
        let text = if model
            .open_workspace()
            .is_some_and(|open| open.selected_host() == crate::open_workspace::HostId::Local)
        {
            format!(
                "Showing the first {} folders; {} more are not listed.",
                crate::workspace_picker::MAX_PICKER_ENTRIES,
                truncated
            )
        } else {
            format!(
                "Showing the first {} entries; more are not listed.",
                crate::workspace_picker::MAX_PICKER_ENTRIES
            )
        };
        notices.push((
            theme::WARNING_TINT,
            theme::WARNING,
            text.clone(),
            text,
            None,
        ));
    }
    // A notice line never renders empty.
    notices.retain(|(_, _, text, ..)| !text.trim().is_empty());
    for (fill, tone, text, tooltip, dismiss) in &notices {
        area_ui.add_space(6.0);
        let mut strip = area_ui.new_child(
            UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(
                    egui::pos2(area.left() + pad_x, area_ui.cursor().top()),
                    egui::pos2(area.right() - pad_x, area.bottom()),
                ))
                .layout(egui::Layout::top_down(Align::Min)),
        );
        let id = dismiss.map(NoticeDismiss::id);
        if notice_strip(&mut strip, layout, *fill, *tone, text, tooltip, id) {
            if let Some(dismiss) = dismiss {
                *command = Some(dismiss.command());
            }
        }
        area_ui.add_space(strip.min_rect().height());
    }
    let list_rect = egui::Rect::from_min_max(
        egui::pos2(
            area.left() + (if layout.compact { 6.0 } else { 8.0 }),
            area_ui.cursor().top() + pad,
        ),
        egui::pos2(
            area.right() - (if layout.compact { 6.0 } else { 8.0 }),
            area.bottom() - pad,
        ),
    );
    let rows = model.screen_rows();
    let selection = model.screen_selection();
    let empty = model.screen_empty_text();
    tree_ui(ui, list_rect, rows.as_slice(), selection, empty, command);
}

fn tree_ui(
    ui: &mut Ui,
    list_rect: egui::Rect,
    rows: &[crate::app::ScreenRow],
    selection: Option<usize>,
    empty_text: Option<&str>,
    command: &mut Option<ScreenCommand>,
) {
    const ROW: f32 = OPEN_WORKSPACE_ROW_HEIGHT;
    if list_rect.height() < 1.0 {
        return;
    }
    let list_focus = picker_list_id();
    let root_key = rows.first().map(|row| row.path.clone());
    let scroll_id = egui::Id::unique("made-picker-last-selection");
    let previous = ui
        .ctx()
        .data(|data| data.get_temp::<(Option<usize>, usize, Option<String>)>(scroll_id));
    let current = (selection, rows.len(), root_key.clone());
    let changed = previous.as_ref() != Some(&current);
    let rerooted = previous
        .as_ref()
        .is_some_and(|previous| previous.2 != root_key);
    if changed {
        ui.ctx()
            .data_mut(|data| data.insert_temp(scroll_id, current));
    }
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
    list_response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Tree, true, "Folders"));
    let content_height = rows.len() as f32 * ROW;
    let mut list_ui = ui.new_child(UiBuilder::new().max_rect(list_rect));
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt("made-picker-list")
        .auto_shrink([false, false])
        .max_height(list_rect.height());
    if rerooted {
        scroll = scroll.animated(false).vertical_scroll_offset(0.0);
    } else if changed {
        if let Some(selected) = selection {
            // Minimal reveal: keep the row and as many ancestors as fit.
            let offset = ui
                .ctx()
                .data(|data| data.get_temp::<f32>(egui::Id::unique("made-picker-offset")))
                .unwrap_or(0.0);
            let top = selected as f32 * ROW;
            let bottom = top + ROW;
            let max_offset = (content_height - list_rect.height()).max(0.0);
            let target = if top < offset {
                top
            } else if bottom > offset + list_rect.height() {
                bottom - list_rect.height()
            } else {
                offset
            };
            scroll = scroll
                .animated(false)
                .vertical_scroll_offset(target.clamp(0.0, max_offset));
        }
    }
    list_ui.spacing_mut().item_spacing.y = 0.0;
    let output = scroll.show_rows(&mut list_ui, ROW, rows.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for index in range {
            let Some(row) = rows.get(index) else {
                continue;
            };
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), Sense::hover());
            let response = ui.interact(
                rect,
                egui::Id::unique(("made-picker-row", index)),
                Sense::CLICK,
            );
            let fill_rect = rect.shrink2(egui::vec2(4.0, 1.0));
            let selected = selection == Some(index);
            let hovered = response.hovered();
            if selected {
                ui.painter()
                    .rect_filled(fill_rect, CornerRadius::same(6), theme::BG_SELECTED);
            } else if hovered {
                ui.painter()
                    .rect_filled(fill_rect, CornerRadius::same(6), theme::BG_HOVER);
            }
            let indent = 4.0 + row.depth as f32 * 14.0;
            theme::paint_disclosure(
                ui.painter(),
                egui::pos2(rect.left() + indent + 8.0, rect.center().y),
                row.expanded,
                if selected || hovered {
                    theme::TEXT_SECONDARY
                } else {
                    theme::TEXT_TERTIARY
                },
            );
            let text_left = rect.left() + 24.0 + row.depth as f32 * 14.0;
            theme::paint_ellipsized_centered(
                ui.painter(),
                text_left,
                rect.center().y,
                &row.name,
                FontId::proportional(theme::BODY_SIZE),
                if selected {
                    theme::TEXT_PRIMARY
                } else {
                    theme::TEXT_SECONDARY
                },
                (rect.right() - text_left - 6.0).max(20.0),
            );
            let response = response.on_hover_text(sanitize_display(&row.path, 900));
            if response.double_clicked() {
                // The first click already toggled; a double-click toggles once.
            } else if response.clicked() {
                ui.memory_mut(|memory| memory.request_focus(list_focus));
                *command = Some(ScreenCommand::Toggle(index));
            }
        }
    });
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            egui::Id::unique("made-picker-offset"),
            output.state.clamped_offset().y,
        )
    });
    if let (true, Some(text)) = (rows.is_empty(), empty_text) {
        let mut empty = ui.new_child(
            UiBuilder::new()
                .max_rect(list_rect)
                .layout(egui::Layout::left_to_right(Align::Min)),
        );
        empty.add_space(8.0);
        empty.label(theme::body(text));
    }
}

fn worktrees_ui(
    model: &mut AppModel,
    ui: &mut Ui,
    layout: &Layout,
    head: f32,
    rows_height: f32,
    command: &mut Option<ScreenCommand>,
) {
    const ROW: f32 = OPEN_WORKSPACE_ROW_HEIGHT;
    let Some(open) = model.open_workspace() else {
        return;
    };
    let items = open.worktrees().to_vec();
    let selection = open.worktree_selection();
    let compact = layout.compact;
    let (pad_top, pad_bottom, pad_x) = if compact {
        (2.0, 2.0, 6.0)
    } else {
        (4.0, 6.0, 8.0)
    };
    let total = 1.0 + pad_top + head + rows_height + pad_bottom;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), total), Sense::hover());
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.top() + 0.5),
            egui::pos2(rect.right(), rect.top() + 0.5),
        ],
        Stroke::new(1.0, theme::BORDER_HEADER),
    );
    let head_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + pad_x + 4.0, rect.top() + 1.0 + pad_top),
        egui::vec2(rect.width() - 2.0 * pad_x, head),
    );
    let galley = ui.painter().layout_job(theme::section_job("WORKTREES"));
    let label_width = galley.size().x;
    ui.painter().galley(
        egui::pos2(
            head_rect.left(),
            head_rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        theme::TEXT_TERTIARY,
    );
    let count = items.len().to_string();
    let count_galley = ui.painter().layout_no_wrap(
        count,
        FontId::proportional(theme::SMALL_SIZE),
        theme::TEXT_TERTIARY,
    );
    let count_rect = egui::Rect::from_min_size(
        egui::pos2(
            head_rect.left() + label_width + 6.0,
            head_rect.center().y - 7.0,
        ),
        egui::vec2(count_galley.size().x + 10.0, 14.0),
    );
    ui.painter()
        .rect_filled(count_rect, CornerRadius::same(7), theme::BG_ELEVATED);
    ui.painter().galley(
        count_rect.center() - count_galley.size() / 2.0,
        count_galley,
        theme::TEXT_TERTIARY,
    );
    let list_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + pad_x, head_rect.bottom()),
        egui::vec2(rect.width() - 2.0 * pad_x, rows_height),
    );
    let focus = worktree_list_id();
    ui.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            focus,
            egui::EventFilter {
                tab: false,
                horizontal_arrows: false,
                vertical_arrows: true,
                escape: false,
            },
        );
    });
    let list_response = ui.interact(list_rect, focus, Sense::click());
    list_response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::List, true, "Worktrees"));
    let has_focus = list_response.has_focus();
    if has_focus {
        let (up, down, pick) = ui.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space),
            )
        });
        let current = selection.unwrap_or(0);
        let step = |from: usize, delta: isize| -> Option<usize> {
            let mut index = from as isize + delta;
            while index >= 0 && (index as usize) < items.len() {
                if !items[index as usize].missing {
                    return Some(index as usize);
                }
                index += delta;
            }
            None
        };
        if up {
            if let Some(next) = step(current, -1) {
                *command = Some(ScreenCommand::SelectWorktree(next));
            }
        } else if down {
            if let Some(next) = step(current, 1) {
                *command = Some(ScreenCommand::SelectWorktree(next));
            }
        } else if pick {
            if let Some(index) = selection {
                *command = Some(ScreenCommand::SelectWorktree(index));
            }
        }
    }
    let mut list_ui = ui.new_child(UiBuilder::new().max_rect(list_rect));
    egui::ScrollArea::vertical()
        .id_salt("made-worktree-list")
        .auto_shrink([false, false])
        .max_height(rows_height)
        .show(&mut list_ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (index, item) in items.iter().enumerate() {
                let (row, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), Sense::hover());
                let response = ui.interact(
                    row,
                    egui::Id::unique(("made-worktree-row", index)),
                    if item.missing {
                        Sense::hover()
                    } else {
                        Sense::CLICK
                    },
                );
                let fill_rect = row.shrink2(egui::vec2(4.0, 0.0));
                let selected = selection == Some(index);
                let hovered = response.hovered() && !item.missing;
                if selected {
                    ui.painter()
                        .rect_filled(fill_rect, CornerRadius::same(6), theme::BG_SELECTED);
                } else if hovered {
                    ui.painter()
                        .rect_filled(fill_rect, CornerRadius::same(6), theme::BG_HOVER);
                }
                if selected && has_focus {
                    response.scroll_to_me_animation(None, egui::style::ScrollAnimation::none());
                }
                let color = if item.missing {
                    theme::TEXT_DISABLED
                } else if selected {
                    theme::TEXT_PRIMARY
                } else {
                    theme::TEXT_SECONDARY
                };
                let icon = egui::Rect::from_min_size(
                    egui::pos2(fill_rect.left() + 8.0, row.center().y - 8.0),
                    egui::Vec2::splat(16.0),
                );
                theme::paint_icon(
                    ui.painter(),
                    icon,
                    if index == 0 {
                        theme::IconGlyph::GitRepository
                    } else {
                        theme::IconGlyph::LinkedWorktree
                    },
                    if selected {
                        theme::TEXT_PRIMARY
                    } else {
                        theme::TEXT_TERTIARY
                    },
                );
                let pills = counts_pills(&item.counts);
                let pills_w = if pills.is_empty() {
                    0.0
                } else {
                    pills_width(ui, &pills)
                };
                let right =
                    fill_rect.right() - 8.0 - pills_w - if pills.is_empty() { 0.0 } else { 8.0 };
                let path = sanitize_display(&item.path.display().to_string(), 900);
                let text_left = icon.right() + 8.0;
                let label = if item.missing {
                    Some("missing".to_owned())
                } else {
                    item.label.as_ref().map(|label| sanitize_display(label, 80))
                };
                let label_width = label.as_ref().map_or(0.0, |label| {
                    ui.painter()
                        .layout_no_wrap(
                            label.clone(),
                            FontId::monospace(10.5),
                            theme::TEXT_TERTIARY,
                        )
                        .size()
                        .x
                        + 8.0
                });
                let path_galley = theme::paint_ellipsized_centered(
                    ui.painter(),
                    text_left,
                    row.center().y,
                    &path,
                    FontId::proportional(theme::BODY_SIZE),
                    color,
                    (right - text_left - label_width).max(40.0),
                );
                if let Some(label) = &label {
                    ui.painter().text(
                        egui::pos2(text_left + path_galley.size().x + 8.0, row.center().y),
                        Align2::LEFT_CENTER,
                        label,
                        FontId::monospace(10.5),
                        theme::TEXT_TERTIARY,
                    );
                }
                if !pills.is_empty() {
                    paint_pills(
                        ui,
                        fill_rect.right() - 8.0 - pills_w,
                        row.center().y,
                        &pills,
                    );
                }
                let mut tooltip = path.clone();
                if let Some(label) = &label {
                    tooltip.push_str(&format!(" ({label})"));
                }
                if let Some(summary) = counts_summary(&item.counts) {
                    tooltip.push_str(&format!(" — {summary}"));
                }
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::Role::ListItem,
                        !item.missing,
                        selected,
                        tooltip.clone(),
                    )
                });
                let response = response.on_hover_text(tooltip);
                if response.clicked() && !item.missing {
                    ui.memory_mut(|memory| memory.request_focus(focus));
                    *command = Some(ScreenCommand::SelectWorktree(index));
                }
            }
        });
}

fn launch_bar_ui(
    model: &mut AppModel,
    ui: &mut Ui,
    layout: &Layout,
    pad_x: f32,
    command: &mut Option<ScreenCommand>,
) {
    let compact = layout.compact;
    let (target_row, gap, button_row, top, bottom) = launch_area_metrics(compact);
    let height = 1.0 + top + target_row + gap + button_row + bottom;
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.top() + 0.5),
            egui::pos2(rect.right(), rect.top() + 0.5),
        ],
        Stroke::new(1.0, theme::BORDER_HEADER),
    );
    let Some(open) = model.open_workspace() else {
        return;
    };
    let target_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + pad_x, rect.top() + 1.0 + top),
        egui::vec2(rect.width() - 2.0 * pad_x, target_row),
    );
    let path = open
        .effective_launch_path()
        .map(|path| sanitize_display(&path.display().to_string(), 900));
    let indicator = open.git_indicator();
    let pills_counts = match open.git_state() {
        Some(crate::open_workspace::GitState::Repository { counts, .. })
            if !open.worktrees_shown() =>
        {
            *counts
        }
        _ => crate::open_workspace::CountsState::Pending,
    };
    let buttons = open.launch_buttons();

    // Target row, left-aligned at the content inset: `Launch in`, git
    // indicator, path, pills.
    let center_y = target_rect.center().y;
    let label_galley = ui.painter().layout_no_wrap(
        "Launch in".to_owned(),
        FontId::proportional(theme::META_SIZE),
        theme::TEXT_TERTIARY,
    );
    let mut x = target_rect.left();
    ui.painter().galley(
        egui::pos2(x, center_y - label_galley.size().y / 2.0),
        label_galley.clone(),
        theme::TEXT_TERTIARY,
    );
    x += label_galley.size().x + 8.0;
    let mut pills = counts_pills(&pills_counts);
    let summary = counts_summary(&pills_counts);
    let path_font = FontId::monospace(11.0);
    let indicator_width = if indicator { 16.0 + 8.0 } else { 0.0 };
    let path_natural = path.as_ref().map_or(0.0, |path| {
        ui.painter()
            .layout_no_wrap(path.clone(), path_font.clone(), theme::TEXT_PRIMARY)
            .size()
            .x
    });
    let available = target_rect.right() - x - indicator_width;
    // The path shortens first; pills that still do not fit hide from the end.
    let min_path = path_natural.min(160.0);
    let mut hidden: Vec<(made_remote::git::PillKind, usize)> = Vec::new();
    while !pills.is_empty() && available - (pills_width(ui, &pills) + 8.0) < min_path {
        if let Some(pill) = pills.pop() {
            hidden.insert(0, pill);
        }
    }
    let pills_space = if pills.is_empty() {
        0.0
    } else {
        pills_width(ui, &pills) + 8.0
    };
    if indicator {
        let icon =
            egui::Rect::from_min_size(egui::pos2(x, center_y - 8.0), egui::Vec2::splat(16.0));
        theme::paint_icon(
            ui.painter(),
            icon,
            theme::IconGlyph::GitRepository,
            theme::TEXT_SECONDARY,
        );
        let mut tooltip = "Git repository".to_owned();
        if !hidden.is_empty() {
            let parts: Vec<String> = hidden
                .iter()
                .map(|(kind, count)| format!("{count} {}", kind.label()))
                .collect();
            tooltip = format!("Git repository — {}", parts.join(" · "));
        }
        let response = ui.interact(icon, picker_control_id("git-indicator"), Sense::hover());
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, tooltip.clone()));
        response.on_hover_text(tooltip);
        x = icon.right() + 8.0;
    }
    let path_width = (target_rect.right() - x - pills_space).max(0.0);
    if let Some(path) = &path {
        let shown = middle_ellipsized(ui, path, &path_font, path_width);
        let galley = ui
            .painter()
            .layout_no_wrap(shown, path_font.clone(), theme::TEXT_PRIMARY);
        let path_rect = egui::Rect::from_min_size(
            egui::pos2(x, center_y - galley.size().y / 2.0),
            egui::vec2(galley.size().x.min(path_width), galley.size().y),
        );
        ui.painter()
            .with_clip_rect(path_rect.expand2(egui::vec2(0.0, 2.0)))
            .galley(path_rect.min, galley, theme::TEXT_PRIMARY);
        let response = ui.interact(path_rect, picker_control_id("launch-path"), Sense::hover());
        let name = format!("Launch in {path}");
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, name.clone()));
        response.on_hover_text(path.clone());
        x = path_rect.right() + 8.0;
    } else {
        ui.painter().text(
            egui::pos2(x, center_y),
            Align2::LEFT_CENTER,
            "No folder yet",
            FontId::proportional(theme::META_SIZE),
            theme::TEXT_TERTIARY,
        );
    }
    if !pills.is_empty() {
        let pills_rect = egui::Rect::from_min_size(
            egui::pos2(x, center_y - PILL_HEIGHT / 2.0),
            egui::vec2(pills_width(ui, &pills), PILL_HEIGHT),
        );
        paint_pills(ui, x, center_y, &pills);
        let response = ui.interact(
            pills_rect,
            picker_control_id("launch-pills"),
            Sense::hover(),
        );
        if let Some(summary) = summary {
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Label, true, summary.clone())
            });
            response.on_hover_text(summary);
        }
    }

    // Button row: centered in the footer, Terminal first; the path never
    // moves it.
    let labels: Vec<(crate::open_workspace::LaunchButton, f32)> = buttons
        .iter()
        .map(|button| {
            (
                *button,
                theme::launch_button_width(ui, button.kind.label(), button_row),
            )
        })
        .collect();
    let total: f32 = labels.iter().map(|(_, width)| width).sum::<f32>()
        + 6.0 * labels.len().saturating_sub(1) as f32;
    let inner_left = rect.left() + pad_x;
    let left = ((rect.center().x - total / 2.0).round()).max(inner_left);
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(egui::Rect::from_min_size(
                egui::pos2(left, target_rect.bottom() + gap),
                egui::vec2((rect.right() - pad_x - left).max(total), button_row),
            ))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 6.0;
    let ids: Vec<egui::Id> = buttons
        .iter()
        .map(|button| launch_button_id(button.kind))
        .collect();
    let focused = ui.ctx().memory(|memory| memory.focused());
    if let Some(index) = focused.and_then(|focused| ids.iter().position(|id| *id == focused)) {
        let step = ui.input(|input| {
            if input.key_pressed(egui::Key::ArrowRight) {
                1
            } else if input.key_pressed(egui::Key::ArrowLeft) {
                -1
            } else {
                0
            }
        });
        if step != 0 {
            let next = (index as isize + step).clamp(0, ids.len() as isize - 1) as usize;
            ui.ctx()
                .memory_mut(|memory| memory.request_focus(ids[next]));
        }
    }
    for (button, _) in labels {
        let id = launch_button_id(button.kind);
        row.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: false,
                    horizontal_arrows: true,
                    vertical_arrows: false,
                    escape: false,
                },
            );
        });
        let leading = match button.kind {
            crate::open_workspace::LaunchKind::Terminal => {
                theme::MenuLeading::Glyph(theme::IconGlyph::Terminal)
            }
            crate::open_workspace::LaunchKind::Tui(tui) => mark_leading(tui.descriptor().mark),
        };
        let tooltip = match (button.disabled_reason, &path) {
            (Some(reason), _) => reason.to_owned(),
            (None, Some(path)) => {
                let base = format!("Open {} in {}", button.kind.label(), path);
                if button.kind == crate::open_workspace::LaunchKind::Terminal {
                    format!("{base} (Cmd/Ctrl+Enter)")
                } else {
                    base
                }
            }
            (None, None) => button.kind.label().to_owned(),
        };
        let action = theme::launch_action(
            &mut row,
            id,
            leading,
            button.kind.label(),
            button_row,
            button.enabled,
            &tooltip,
        );
        if action.activated {
            *command = Some(ScreenCommand::Launch(button.kind));
        }
    }
}

fn screen_keyboard(model: &AppModel, ui: &mut Ui, command: &mut Option<ScreenCommand>) {
    let ctx = ui.ctx().clone();
    let focused = ctx.memory(|memory| memory.focused());
    let address_escape = ctx.data_mut(|data| {
        data.remove_temp::<bool>(egui::Id::unique("made-address-escape"))
            .unwrap_or(false)
    });
    let text_focused = focused.is_some_and(|id| {
        id == picker_path_id()
            || id == ssh_field_id("screen-prompt")
            || [
                crate::ssh_servers::HostField::Name,
                crate::ssh_servers::HostField::Host,
                crate::ssh_servers::HostField::User,
                crate::ssh_servers::HostField::Port,
                crate::ssh_servers::HostField::KeyFile,
                crate::ssh_servers::HostField::Directory,
            ]
            .into_iter()
            .any(|field| host_form_field_id(field) == id)
    });
    let tree_keys = focused.is_none() || focused == Some(picker_list_id());
    // Backspace belongs to folder navigation: the tree and its Up/Refresh
    // toolbar, never the host bar, launch bar or other controls.
    let folder_keys = tree_keys
        || focused
            .is_some_and(|id| id == picker_control_id("up") || id == picker_control_id("refresh"));
    let events = ctx.input(|input| input.events.clone());
    for event in events {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        else {
            continue;
        };
        if key == egui::Key::Enter && modifiers.command {
            // Terminal is the Cmd/Ctrl+Enter target, even over a focused
            // launch button; a text field or another action owning this Enter
            // (Navigate, Save, a prompt or consent answer) is left alone.
            let free = command.is_none() || matches!(command, Some(ScreenCommand::Launch(_)));
            if !text_focused && free {
                *command = Some(ScreenCommand::Launch(
                    crate::open_workspace::LaunchKind::Terminal,
                ));
            }
            continue;
        }
        if key == egui::Key::Escape {
            if address_escape || text_focused || egui::Popup::is_any_open(&ctx) {
                continue;
            }
            if let Some(dismiss) = focused
                .and_then(|focused| NoticeDismiss::ALL.into_iter().find(|d| d.id() == focused))
            {
                // Escape on a dismiss control closes only its notice.
                *command = Some(dismiss.command());
            } else if model.screen_form().is_some() {
                *command = Some(ScreenCommand::CancelForm);
            } else {
                *command = Some(ScreenCommand::Close);
            }
            continue;
        }
        if key == egui::Key::Backspace && folder_keys && command.is_none() {
            *command = Some(ScreenCommand::Up);
            continue;
        }
        if !tree_keys || command.is_some() {
            continue;
        }
        *command = match key {
            egui::Key::ArrowDown => Some(ScreenCommand::Move(1)),
            egui::Key::ArrowUp => Some(ScreenCommand::Move(-1)),
            egui::Key::ArrowRight => Some(ScreenCommand::Expand),
            egui::Key::ArrowLeft => Some(ScreenCommand::Collapse),
            egui::Key::Enter => Some(ScreenCommand::ToggleSelected),
            _ => None,
        };
    }
}

fn apply_screen_command(model: &mut AppModel, ui: &Ui, command: Option<ScreenCommand>) {
    let Some(command) = command else {
        return;
    };
    match command {
        ScreenCommand::Up => model.screen_up(),
        ScreenCommand::Refresh => model.screen_refresh(),
        ScreenCommand::Toggle(index) => model.screen_toggle(index),
        ScreenCommand::SelectHost(host) => model.screen_select_host(host),
        ScreenCommand::AddHost => model.screen_add_host(),
        ScreenCommand::EditHost(id) => model.screen_edit_host(id),
        ScreenCommand::TestHost(id) => model.screen_test_host(id),
        ScreenCommand::RemoveHost(id) => model.screen_request_remove(id),
        ScreenCommand::SelectWorktree(index) => {
            model.screen_select_worktree(index);
        }
        ScreenCommand::Launch(kind) => model.screen_launch(kind),
        ScreenCommand::Navigate(text) => {
            if model.screen_navigate(&text).is_ok() {
                ui.ctx()
                    .memory_mut(|memory| memory.surrender_focus(picker_path_id()));
            }
        }
        ScreenCommand::Move(delta) => model.screen_move(delta),
        ScreenCommand::Expand => model.screen_expand(),
        ScreenCommand::Collapse => model.screen_collapse(),
        ScreenCommand::ToggleSelected => model.screen_toggle_selected(),
        ScreenCommand::SaveForm => {
            model.screen_save_form();
        }
        ScreenCommand::CancelForm => model.screen_cancel_form(),
        ScreenCommand::Removal(confirm) => model.screen_answer_removal(confirm),
        ScreenCommand::Prompt(accept) => model.screen_answer_prompt(accept),
        ScreenCommand::Consent(approve) => model.screen_answer_consent(approve),
        ScreenCommand::ForgetHostKey => {
            if let Err(error) = model.forget_selected_ssh_host_key() {
                model.set_ssh_notice(error);
            }
        }
        ScreenCommand::DismissReport => model.screen_dismiss_test_report(),
        ScreenCommand::DismissFailure => model.screen_dismiss_connection_failure(),
        ScreenCommand::DismissNotice(kind) => model.screen_dismiss_notice(kind),
        ScreenCommand::Close => {
            model.close_picker();
        }
    }
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

/// Stable id for a top-bar control whose state belongs to one workspace.
pub fn top_workspace_action_id(id: WorkspaceId, action: &str) -> egui::Id {
    egui::Id::unique(("made-top-workspace-action", action, id.get()))
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
            if layout.compact { 230.0 } else { 280.0 },
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

/// Non-color summary across all open sessions. Counts include sessions beyond
/// the eight-dot visual cap.
fn sessions_summary(sessions: &[SessionActivity]) -> Option<String> {
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
    let label = sanitize_display(label, 300);
    match summary {
        Some(summary) => format!("{label} — {summary}"),
        None => label,
    }
}

/// The accessible name of a tab: a branded tab names its program, then the
/// session title its label shows; a shell tab names its label.
fn workspace_tab_name(tab: &WorkspaceTabInfo, summary: Option<&str>) -> String {
    let label = match &tab.brand {
        Some(brand) => match &brand.shown {
            Some(title) => format!("{} — {title}", brand.entry.descriptor().label),
            None => brand.entry.descriptor().label.to_owned(),
        },
        None => tab.label.clone(),
    };
    let label = match &tab.ssh {
        Some(host) => format!("{label} on SSH host {}", sanitize_display(&host.name, 128)),
        None => label,
    };
    workspace_tab_accessible_label(&label, summary)
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
    /// The SSH host of a remote tab: its badge, tooltip line and accessible
    /// name.
    ssh: Option<TabHost>,
    /// The integration that brands the tab: its label, mark and dots.
    brand: Option<TabBrand>,
    sessions: Vec<SessionActivity>,
}

/// Center of the SSH host badge: the bottom-right of the leading icon slot.
pub fn ssh_tab_badge_center(icon_rect: egui::Rect) -> egui::Pos2 {
    icon_rect.max - egui::vec2(3.5, 3.5)
}

/// The SSH host a workspace tab is bound to.
struct TabHost {
    /// Saved display name.
    name: String,
    /// `[user@]host[:port]`.
    address: String,
    /// The host bar state shape while the transport is not connected.
    state: Option<theme::HostStatusGlyph>,
}

/// The integration that brands a workspace tab.
struct TabBrand {
    entry: IntegrationRef,
    /// Sanitized tooltip title, for an integration whose tooltip names its
    /// session.
    title: Option<String>,
    /// The session title the label shows, when it does not fall back to the
    /// catalog label.
    shown: Option<String>,
}

/// Whether the session map yields to another running program: the tab is
/// branded by an integration that does not own the session channel.
pub(crate) fn sessions_outranked(workbench: &Workbench) -> bool {
    tab_brand(workbench)
        .is_some_and(|(entry, _)| entry.follow() != crate::integrations::FollowKind::SessionChannel)
}

/// `{lead} {owner} sessions`, or `{lead} sessions` without a session channel.
fn sessions_label(lead: &str, owner: Option<&str>) -> String {
    match owner {
        Some(owner) => format!("{lead} {owner} sessions"),
        None => format!("{lead} sessions"),
    }
}

/// The first integration of the tab's list, in rank order, whose presentation
/// brands the tab, with that presentation.
pub(crate) fn tab_brand(workbench: &Workbench) -> Option<(IntegrationRef, Presentation)> {
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
/// The branding of a remote tab, in the local order. OpenCode and Grok Build
/// present from the tab's own server state; an integration that presents from
/// its tab state (Claude Code) yields to OpenCode and ranks above Grok Build.
fn remote_tab_brand(
    workbench: &Workbench,
    list: IntegrationList,
    generic: Option<(IntegrationRef, Presentation)>,
) -> Option<(IntegrationRef, Presentation)> {
    let opencode =
        list.contains(&crate::integrations::OPENCODE) && workbench.remote_opencode_active();
    let grok_dots = workbench.remote_grok_activities();
    let remote_grok = list.contains(&crate::integrations::GROK)
        && crate::integrations::grok::workspace_grok_branded(
            opencode,
            workbench.terminal_target(),
            workbench.remote_grok_running(),
            grok_dots.len(),
        );
    let generic = generic
        .filter(|_| crate::integrations::claude_code::workspace_claude_branded(opencode, true));
    if opencode {
        let dots = workbench
            .session_map()
            .rows()
            .iter()
            .map(SessionRow::activity)
            .collect();
        Some((
            crate::integrations::OPENCODE,
            Presentation { dots, title: None },
        ))
    } else if generic.is_some() {
        generic
    } else if remote_grok {
        Some((
            crate::integrations::GROK,
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
/// Longest tab label before sanitizing truncates it; the tab ellipsizes the
/// rest to its width.
const WORKSPACE_TAB_LABEL_MAX_CHARS: usize = 120;

/// The last component of a slash-separated path; `/` names the root.
fn folder_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .map_or_else(|| path.to_owned(), str::to_owned)
}

/// A sanitized, bounded label, or `None` when nothing printable remains.
fn bounded_label(text: &str) -> Option<String> {
    let label = sanitize_display(text.trim(), WORKSPACE_TAB_LABEL_MAX_CHARS);
    let label = label.trim();
    (!label.is_empty()).then(|| label.to_owned())
}

/// The session title a branded tab shows: the active session of the session
/// channel's owner, or the title its presentation names. `None` when the tab
/// falls back to the program's catalog label.
fn branded_session_title(
    workbench: &Workbench,
    entry: IntegrationRef,
    presentation: &Presentation,
) -> Option<String> {
    let title = if entry.follow() == crate::integrations::FollowKind::SessionChannel {
        let map = workbench.session_map();
        map.rows()
            .iter()
            .find(|row| Some(row.session_id.as_str()) == map.active_session_id())
            .map(|row| row.title.clone())
            .filter(|title| title != crate::session_map::UNTITLED)
    } else {
        presentation.title.clone()
    };
    title
        .and_then(|title| bounded_label(&title))
        .filter(|title| title != entry.descriptor().label)
}

/// The label of a workspace tab: a branded tab names its active session,
/// falling back to the program's catalog label; a shell tab names its
/// shell's current directory (the remote cwd for an SSH tab).
pub(crate) fn workspace_tab_label(workbench: &Workbench) -> String {
    let label = match tab_brand(workbench) {
        Some((entry, presentation)) => branded_session_title(workbench, entry, &presentation)
            .unwrap_or_else(|| entry.descriptor().label.to_owned()),
        None => match workbench.remote_cwd_label() {
            Some(cwd) => folder_name(&cwd),
            None => folder_name(&workbench.shell_cwd().display().to_string()),
        },
    };
    bounded_label(&label).unwrap_or_else(|| "Workspace".to_owned())
}

fn workspace_tab_info(id: WorkspaceId, workbench: &Workbench) -> WorkspaceTabInfo {
    let path = workbench.shell_cwd().to_path_buf();
    let remote_path = workbench.remote_cwd_label();
    let label = workspace_tab_label(workbench);
    let branded = tab_brand(workbench);
    // Session dots follow the brand, so stale rows cannot paint status after a
    // disconnect or fatal protocol failure.
    let (brand, sessions) = match branded {
        Some((entry, presentation)) => {
            let shown = branded_session_title(workbench, entry, &presentation);
            (
                Some(TabBrand {
                    entry,
                    title: shown.clone().or(presentation.title),
                    shown,
                }),
                presentation.dots,
            )
        }
        None => (None, Vec::new()),
    };
    WorkspaceTabInfo {
        id,
        label,
        path,
        remote_path,
        ssh: workbench.remote_host().map(|(name, address)| TabHost {
            name,
            address,
            state: workbench.remote_transport_glyph(),
        }),
        brand,
        sessions,
    }
}

/// The tooltip of a workspace tab: a branded tab names its session and the
/// sessions' summary and never the path; an unbranded tab with sessions shows
/// the summary, and one without shows its path.
fn workspace_tab_tooltip(tab: &WorkspaceTabInfo, summary: Option<&str>) -> Option<String> {
    let content = match &tab.brand {
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
    };
    // An SSH tab names its host first.
    match &tab.ssh {
        Some(host) => {
            let line = format!(
                "SSH: {} ({})",
                sanitize_display(&host.name, 128),
                sanitize_display(&host.address, 300)
            );
            Some(match content {
                Some(content) => format!("{line}\n{content}"),
                None => line,
            })
        }
        None => content,
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
    /// The SSH host badge: `None` on a local tab, else the host bar state
    /// shape it paints (`None` inside while connected).
    pub ssh_badge: Option<Option<theme::HostStatusGlyph>>,
}

/// The tabs of the open workspaces as they paint, in strip order.
#[doc(hidden)]
pub fn workspace_tab_summaries(model: &AppModel) -> Vec<TabSummary> {
    model
        .workspace_ids()
        .filter_map(|id| {
            let tab = workspace_tab_info(id, model.workspace(id)?);
            let summary = sessions_summary(&tab.sessions);
            Some(TabSummary {
                id,
                mark: tab
                    .brand
                    .as_ref()
                    .map(|brand| brand.entry.descriptor().mark),
                tooltip: workspace_tab_tooltip(&tab, summary.as_deref()),
                accessible_name: workspace_tab_name(&tab, summary.as_deref()),
                ssh_badge: tab.ssh.as_ref().map(|host| host.state),
                dots: tab.sessions,
                label: tab.label,
            })
        })
        .collect()
}

/// The pill of one About integration row: the primary integration keeps its
/// status pill; declared rows keep their row pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AboutPill {
    Status(IntegrationStatus),
    Row(DiagnosticsPill),
}

impl AboutPill {
    /// 2 for unsupported, 1 for degraded or failed, 0 otherwise.
    fn severity(self) -> u8 {
        match self {
            Self::Status(IntegrationStatus::Unsupported) => 2,
            Self::Status(IntegrationStatus::Degraded) | Self::Row(DiagnosticsPill::Degraded) => 1,
            _ => 0,
        }
    }
}

/// One integration row of the About card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AboutRow<'a> {
    /// The integration the row belongs to; `None` for the neutral headline.
    pub entry: Option<IntegrationRef>,
    pub pill: AboutPill,
    pub sentence: &'a str,
}

/// The About card's integration rows and headline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutIntegrations<'a> {
    /// The tab's overall state: the most severe actionable row (catalog order
    /// breaks ties), else the active program's row, else a neutral inactive
    /// sentence.
    pub headline: AboutRow<'a>,
    /// Every integration of `list` with diagnostics, as ordinary rows in list
    /// order.
    pub rows: Vec<AboutRow<'a>>,
}

/// Sentence of the headline when no program integration is active.
pub const NO_PROGRAM_INTEGRATION: &str = "No program integration is active.";

/// Build the About rows and headline of `snapshot` for a tab over `list`
/// whose active program is `active`.
pub fn about_integrations<'a>(
    snapshot: &'a DiagnosticsSnapshot,
    list: IntegrationList,
    active: Option<IntegrationRef>,
) -> AboutIntegrations<'a> {
    use crate::integrations::DiagnosticsDecl;
    let rows: Vec<AboutRow<'a>> = list
        .iter()
        .filter_map(|entry| match entry.diagnostics() {
            DiagnosticsDecl::Primary => Some(AboutRow {
                entry: Some(*entry),
                pill: AboutPill::Status(snapshot.status),
                sentence: snapshot.reason.as_str(),
            }),
            DiagnosticsDecl::Row(decl) => snapshot.row(decl.status_key).map(|row| AboutRow {
                entry: Some(*entry),
                pill: AboutPill::Row(row.pill),
                sentence: row.sentence.as_str(),
            }),
            DiagnosticsDecl::None => None,
        })
        .collect();
    // A program that is merely absent is actionable only as the active one.
    let severity = |row: &AboutRow<'_>| match row.entry {
        Some(entry)
            if row.entry != active
                && matches!(row.pill, AboutPill::Status(_))
                && entry.reason_is_absence(snapshot.reason_code) =>
        {
            0
        }
        _ => row.pill.severity(),
    };
    let worst = rows.iter().map(severity).max().unwrap_or(0);
    let headline = rows
        .iter()
        .find(|row| worst > 0 && severity(row) == worst)
        .or_else(|| {
            rows.iter()
                .find(|row| active.is_some() && row.entry == active)
        })
        .copied()
        .unwrap_or(AboutRow {
            entry: None,
            pill: AboutPill::Row(DiagnosticsPill::Inactive),
            sentence: NO_PROGRAM_INTEGRATION,
        });
    AboutIntegrations { headline, rows }
}

/// A section that carries only the session-channel owner's bridge data.
fn owner_section(owner: Option<&str>, section: &str) -> String {
    match owner {
        Some(owner) => format!("{} {section}", owner.to_uppercase()),
        None => section.to_owned(),
    }
}

/// The About rows and headline of `workbench`, whose active program is the
/// integration branding the tab.
fn workbench_about<'a>(
    workbench: &Workbench,
    snapshot: &'a DiagnosticsSnapshot,
) -> AboutIntegrations<'a> {
    about_integrations(
        snapshot,
        workbench.integrations(),
        tab_brand(workbench).map(|(entry, _)| entry),
    )
}

/// The About card headline sentence of `workbench`.
pub fn about_headline(workbench: &Workbench) -> String {
    let snapshot = workbench.diagnostics_snapshot();
    workbench_about(workbench, &snapshot)
        .headline
        .sentence
        .to_owned()
}

/// The integration rows under the headline; the headline row is shown once.
fn about_rows_ui(ui: &mut Ui, about: &AboutIntegrations<'_>, layout: &Layout) {
    for row in about.rows.iter().filter(|row| **row != about.headline) {
        ui.horizontal(|ui| {
            about_pill_ui(ui, row, layout);
            ui.label(theme::micro(row.sentence))
                .on_hover_text(row.sentence);
        });
    }
}

fn about_pill_ui(ui: &mut Ui, row: &AboutRow<'_>, layout: &Layout) -> egui::Response {
    match row.pill {
        AboutPill::Status(status) => status_pill_ui(ui, status, row.sentence, layout),
        AboutPill::Row(pill) => integration_status_pill_ui(ui, pill, row.sentence, layout),
    }
}

fn authority_key(id: made_remote::protocol::AuthorityId) -> String {
    id.0.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Stable id for one SSH prompt field.
pub fn ssh_field_id(field: &str) -> egui::Id {
    egui::Id::unique(("made-ssh-field", field))
}

fn ssh_field_hold_id() -> egui::Id {
    egui::Id::unique("made-ssh-field-hold")
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

/// Drop the field hold.
fn release_ssh_field_hold(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<egui::Id>(ssh_field_hold_id());
    });
}

/// The terminal paints after a prompt card and treats a click in its rect as
/// its own. Restore the held prompt field after that.
struct SshFieldFocusRestore;

impl egui::plugin::Plugin for SshFieldFocusRestore {
    fn debug_name(&self) -> &'static str {
        "ssh-field-focus"
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        restore_ssh_field_focus(ui);
    }
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

/// Keep a held SSH prompt field focused across the terminal's own focus
/// requests. Installed every frame; it does nothing without a held field.
pub fn install_ssh_field_focus(ctx: &egui::Context) {
    ctx.add_plugin(SshFieldFocusRestore);
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
    let explorer_visible = model.workbench().is_some_and(Workbench::explorer_visible);
    let sessions_visible = model.workbench().is_some_and(Workbench::sessions_visible);
    let sessions_available = model.workbench().is_some_and(|workbench| {
        !sessions_outranked(workbench)
            && (sessions_visible
                || !workbench.session_map().is_empty()
                || workbench.integration().status().is_connected()
                || workbench.remote_sessions_available())
    });
    let sessions_owner = model
        .workbench()
        .and_then(|workbench| crate::integrations::session_channel_label(workbench.integrations()));
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
    let mut about = false;

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

            let mut right_specs: Vec<(&'static str, theme::IconGlyph, String, bool)> = Vec::new();
            if sessions_available {
                right_specs.push((
                    "sessions",
                    theme::IconGlyph::SessionList,
                    sessions_label(
                        if sessions_visible { "Hide" } else { "Show" },
                        sessions_owner,
                    ),
                    sessions_visible,
                ));
            }
            if has_workbench {
                right_specs.push((
                    "explorer",
                    theme::IconGlyph::PanelRight,
                    if explorer_visible {
                        "Hide explorer"
                    } else {
                        "Show explorer"
                    }
                    .to_owned(),
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
                "Open workspace",
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
                            let path_text = tab
                                .remote_path
                                .clone()
                                .unwrap_or_else(|| tab.path.display().to_string());
                            let path_text = match &tab.ssh {
                                Some(host) => format!("SSH: {} · {path_text}", host.name),
                                None => path_text,
                            };
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
    let session_summary = sessions_summary(&tab.sessions);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::Role::Tab,
            true,
            active,
            workspace_tab_name(tab, session_summary.as_deref()),
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
    if let Some(host) = &tab.ssh {
        // The cutout ring takes the tab's own fill; the idle fill is
        // translucent, so its opaque backdrop stands in.
        let cutout = if active {
            theme::BG_TAB_SELECTED
        } else if hovered {
            theme::BG_TAB_HOVER
        } else {
            theme::BG_CLEAR_SOLID
        };
        ui.painter().extend(theme::ssh_tab_badge_shapes(
            ssh_tab_badge_center(icon_rect),
            host.state,
            cutout,
            color,
        ));
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
            let panel_width = ((width + layout.pane_gap) * progress).clamp(0.0, available);
            // The panel's inner left margin is the pane gap; keep the content
            // rectangle non-negative during the first/last animation frames.
            if panel_width > layout.pane_gap + 0.5 {
                egui::Panel::right(egui::Id::unique(("made-explorer-panel", self.pane_id())))
                    .resizable(false)
                    .exact_size(panel_width)
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                        left: layout.pane_gap as i8,
                        right: 0,
                        top: 0,
                        bottom: 0,
                    }))
                    .show_separator_line(false)
                    .show(ui, |ui| {
                        let rect = ui.max_rect();
                        let splitter = egui::Rect::from_center_size(
                            egui::pos2(rect.left() - layout.pane_gap * 0.5, rect.center().y),
                            egui::vec2(theme::SPLITTER_HIT_WIDTH, rect.height()),
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

                        // Paint the capsule after the card so it cannot be
                        // covered by the sliding content.
                        if splitter_fade > 0.0 {
                            let dragged = response.dragged();
                            let opacity = theme::splitter_opacity(splitter_fade, dragged);
                            if opacity > 0.001 {
                                ui.painter().rect_filled(
                                    theme::splitter_capsule_rect(splitter.center()),
                                    CornerRadius::same(
                                        (theme::SPLITTER_CAPSULE_SIZE.x / 2.0) as u8,
                                    ),
                                    theme::splitter_capsule_color(splitter_fade, dragged),
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
                                show_preview = false;
                            }
                            PreviewUiAction::None => {}
                        }
                    } else {
                        self.close_preview();
                        self.sync_terminal_visibility(ui.ctx());
                        show_preview = false;
                    }
                }
                if !show_preview {
                    self.terminal_card(ui, layout);
                }
            });
        self.explorer = explorer;
        if opened_source_control {
            self.refresh_git_status_if_stale();
            ui.ctx()
                .request_repaint_after(crate::metrics::ACTIVE_REPAINT_INTERVAL);
        }
    }

    fn terminal_card(&mut self, ui: &mut Ui, layout: &Layout) {
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
        // The map is hidden, reserving no space, while another program runs.
        let show_sessions = self.sessions_visible() && !sessions_outranked(self);

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
                owner: crate::integrations::session_channel_label(self.integrations()),
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

        let body_rect = egui::Rect::from_min_max(egui::pos2(card_rect.left(), top), card_rect.max);
        let mut body_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(body_rect)
                .id_salt(("made-terminal-body", self.pane_id())),
        );
        // Painting the URL dialog arms its Open from this frame. While it is
        // shown the terminal leaves pointer input over the whole body alone.
        let url_card = self.url_card_painted(std::time::Instant::now());
        let terminal_had_focus = self
            .terminal
            .as_ref()
            .is_some_and(|terminal| terminal.is_focused());
        let terminal_error = self.terminal_error.clone();
        let mut terminal = self.terminal.take();
        if let Some(terminal) = terminal.as_mut() {
            terminal.set_pointer_exclusion(url_card.as_ref().map(|_| body_rect));
        }
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

        // Drawn over the terminal body after it, so it is on top: showing or
        // hiding it never changes the terminal grid, so the child gets no
        // resize.
        if let Some(card) = url_card {
            let dialog = url_dialog_layout(ui, body_rect, layout, &card);
            self.set_url_card_reviewable(dialog.reviewable);
            // Read back: becoming reviewable restarts the arming delay.
            let card = self.url_card().cloned().unwrap_or(card);
            if let Some(action) =
                url_dialog_ui(ui, body_rect, &dialog, layout, &card, self.pane_id())
            {
                if let Some(text) = self.act_on_url_card(action) {
                    ui.ctx().copy_text(text);
                }
            }
            // A click on the dialog or its scrim must not cost the terminal
            // its keyboard focus: the dialog's controls take it only for the
            // click, and typing keeps going to the terminal.
            let dialog_pointer = ui.input(|input| {
                (input.pointer.any_pressed()
                    || input.pointer.any_released()
                    || input.pointer.any_down())
                    && input
                        .pointer
                        .interact_pos()
                        .is_some_and(|pos| body_rect.contains(pos))
            });
            if terminal_had_focus && dialog_pointer {
                if let Some(terminal_id) = self
                    .terminal
                    .as_ref()
                    .and_then(|terminal| terminal.focus_id())
                {
                    ui.ctx()
                        .memory_mut(|memory| memory.request_focus(terminal_id));
                }
            }
        }

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
        self.focus_unavailable.hash(&mut hasher);
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
        // BEGIN remote-path (AIA-02)
        if let Some(report) = self.remote_opencode_diagnostics() {
            report.status.label().hash(&mut hasher);
            report.code.hash(&mut hasher);
            report.reason.hash(&mut hasher);
        }
        // END remote-path
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
        let about = workbench_about(self, snapshot);
        let reason_full = about.headline.sentence.to_owned();
        let reason = sanitize_display(&reason_full, 320);
        let owner = crate::integrations::session_channel_label(self.integrations());
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
                    about_pill_ui(ui, &about.headline, layout);
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
                // Compact cards scroll the rows with the detail so the log
                // footer stays inside the window.
                if !layout.compact {
                    about_rows_ui(ui, &about, layout);
                }
                if let Some(failure) = &config_failure {
                    notice_strip(
                        ui,
                        layout,
                        theme::WARNING_TINT,
                        theme::WARNING,
                        &format!("configuration: {}", sanitize_display(failure, 400)),
                        &sanitize_display(failure, 700),
                        None,
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
                        None,
                    );
                }

                let detail_height = (ui.available_height() - 90.0).max(48.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(detail_height)
                    .show(ui, |ui| {
                        if layout.compact {
                            about_rows_ui(ui, &about, layout);
                        }
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

                        diagnostics_section(ui, &owner_section(owner, "CONNECTION"));
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

                        diagnostics_section(ui, &owner_section(owner, "SESSION"));
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
                        let owner_plugin = owner
                            .map_or_else(|| "Plugin".to_owned(), |owner| format!("{owner} plugin"));
                        diagnostics_rows(
                            ui,
                            "made-diag-versions",
                            &[
                                (
                                    owner_plugin.as_str(),
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
        "Restart into Made {version}? Every open workspace and terminal, including running \
         programs and SSH sessions, will close and will not be restored."
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
    /// The session-channel owner's label, which the map belongs to.
    owner: Option<&'static str>,
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
        let none = sessions_label("No", state.owner);
        if layout.compact {
            empty
                .label(theme::micro(&none))
                .on_hover_text("Open OpenCode in the terminal.");
        } else {
            empty.label(theme::micro(&none));
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

pub fn url_card_action_id(pane_id: &str, action: &str) -> egui::Id {
    egui::Id::unique(("made-url-card", pane_id, action))
}

/// Rows enough for the longest host (255 characters) even at one character
/// per row, so wrapping never elides any of it.
const URL_DIALOG_MAX_ROWS: usize = 255;
/// Space kept between the dialog and the edges of the pane body.
const URL_DIALOG_MARGIN: f32 = 16.0;
const URL_DIALOG_MAX_WIDTH: f32 = 560.0;
/// `theme::floating_frame`'s inner margin and corner radius.
const URL_DIALOG_PADDING: f32 = 14.0;
const URL_DIALOG_RADIUS: u8 = 10;
/// Narrowest content column that still reviews a host; below it Open stays
/// disabled.
const URL_DIALOG_MIN_TEXT_WIDTH: f32 = 160.0;
/// Tallest the full-URL block grows before it scrolls.
const URL_DIALOG_URL_MAX_HEIGHT: f32 = 180.0;
/// Inner padding of the full-URL block.
const URL_DIALOG_URL_PADDING: f32 = 8.0;
/// Dims the pane behind the dialog, like the floating shadows' black.
const URL_DIALOG_SCRIM: Color32 = Color32::from_black_alpha(120);
const URL_DIALOG_REASON: &str = "Widen the pane to review this link";

/// The laid-out dialog for one frame.
pub(crate) struct UrlDialogLayout {
    /// The dialog, centered over the pane body.
    pub rect: egui::Rect,
    title: Arc<egui::Galley>,
    host: Arc<egui::Galley>,
    url: Arc<egui::Galley>,
    /// Visible height of the full-URL block; longer URLs scroll in it.
    url_height: f32,
    /// Whether the whole host fits in the dialog; Open needs it.
    pub reviewable: bool,
}

fn url_dialog_job(text: &str, font: FontId, color: Color32, width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: width,
            max_rows: URL_DIALOG_MAX_ROWS,
            break_anywhere: true,
            overflow_character: None,
        },
        ..Default::default()
    };
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job
}

/// Lays out the dialog for `body`. Nothing in it is ever shortened: the host
/// wraps, and the full URL wraps inside a block that scrolls once it is
/// taller than [`URL_DIALOG_URL_MAX_HEIGHT`] or the room the pane leaves.
pub(crate) fn url_dialog_layout(
    ui: &Ui,
    body: egui::Rect,
    layout: &Layout,
    card: &crate::url_open::UrlCard,
) -> UrlDialogLayout {
    use crate::url_open::UrlOrigin;
    let width = (body.width() - 2.0 * URL_DIALOG_MARGIN)
        .clamp(2.0 * URL_DIALOG_PADDING + 1.0, URL_DIALOG_MAX_WIDTH);
    let inner = width - 2.0 * URL_DIALOG_PADDING;
    let title = match &card.origin {
        UrlOrigin::Local => "Open link in browser?".to_owned(),
        UrlOrigin::Remote(server) => format!(
            "Open link in browser? \u{2014} from {}",
            sanitize_display(server, 40)
        ),
    };
    let galley = |text: &str, font: FontId, color: Color32, width: f32| {
        ui.painter()
            .layout_job(url_dialog_job(text, font, color, width))
    };
    let title = galley(
        &title,
        FontId::proportional(13.0),
        theme::TEXT_SECONDARY,
        inner,
    );
    let host = galley(
        &sanitize_display(card.url.host(), 255),
        FontId::proportional(16.0),
        theme::TEXT_PRIMARY,
        inner,
    );
    let url = galley(
        &sanitize_display(card.url.as_str(), 2048),
        FontId::monospace(11.5),
        theme::TEXT_SECONDARY,
        inner - 2.0 * URL_DIALOG_URL_PADDING,
    );
    let url_row = url.rows.first().map_or(url.size().y, |row| row.height());
    let block = |height: f32| height + 2.0 * URL_DIALOG_URL_PADDING;
    // Everything but the URL block's scrolling part.
    let fixed = 2.0 * URL_DIALOG_PADDING
        + title.size().y
        + host.size().y
        + layout.button_height.max(24.0)
        + 3.0 * layout.gap;
    let available = (body.height() - 2.0 * URL_DIALOG_MARGIN).max(0.0);
    let reviewable = inner >= URL_DIALOG_MIN_TEXT_WIDTH && fixed + block(url_row) <= available;
    let reason = if reviewable { 0.0 } else { layout.gap + 14.0 };
    let url_height = url
        .size()
        .y
        .min(URL_DIALOG_URL_MAX_HEIGHT)
        .min(available - fixed - reason - block(0.0))
        .max(url_row);
    let height = fixed + reason + block(url_height);
    let top = if height <= body.height() {
        body.center().y - height / 2.0
    } else {
        body.top()
    };
    let rect = egui::Rect::from_min_size(
        egui::pos2(body.center().x - width / 2.0, top),
        egui::vec2(width, height),
    );
    UrlDialogLayout {
        rect,
        title,
        host,
        url,
        url_height,
        reviewable,
    }
}

/// The dialog over the pane that asked to open a URL: a scrim over the pane
/// body that takes every pointer event, the title, the whole host, the full
/// URL in a scrolling block, and the only controls that act on it. Open and
/// Copy wait for the arming delay; Dismiss is immediate. It never takes
/// keyboard input.
fn url_dialog_ui(
    ui: &mut Ui,
    body: egui::Rect,
    dialog: &UrlDialogLayout,
    layout: &Layout,
    card: &crate::url_open::UrlCard,
    pane_id: &str,
) -> Option<crate::url_open::UrlCardAction> {
    use crate::url_open::UrlCardAction;
    // The scrim claims pointer input over the whole pane body, so a click
    // meant for the dialog never reaches the terminal or the program in it.
    ui.interact(
        body,
        url_card_action_id(pane_id, "scrim"),
        Sense::click_and_drag(),
    );
    let bottom = layout.card_radius;
    ui.painter().rect_filled(
        body,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: bottom,
            se: bottom,
        },
        URL_DIALOG_SCRIM,
    );
    let rect = dialog.rect;
    ui.interact(
        rect,
        url_card_action_id(pane_id, "card"),
        Sense::click_and_drag(),
    );
    let radius = CornerRadius::same(URL_DIALOG_RADIUS);
    // A dialog taller than a short pane must not paint over the chrome.
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(body));
    painter.add(theme::SHADOW_FLOAT.as_shape(rect, radius));
    painter.rect(
        rect,
        radius,
        theme::BG_CARD,
        Stroke::new(1.0, theme::BORDER),
        StrokeKind::Inside,
    );
    let now = std::time::Instant::now();
    let armed = card.armed(now);
    let settled = card.settled(now);
    if let Some(left) = card.arming_left(now) {
        ui.ctx().request_repaint_after(left);
    }
    let inner = rect.shrink(URL_DIALOG_PADDING);
    let mut content = ui.new_child(
        UiBuilder::new()
            .id_salt(("made-url-dialog", pane_id))
            .max_rect(inner)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    content.set_clip_rect(rect.intersect(body));
    content.spacing_mut().item_spacing = egui::vec2(layout.gap, layout.gap);
    content.add(egui::Label::new(dialog.title.clone()).selectable(false));
    if !dialog.reviewable {
        // First after the title, so it stays visible in a short pane.
        content.add(egui::Label::new(theme::micro(URL_DIALOG_REASON)).truncate());
    }
    content.add(egui::Label::new(dialog.host.clone()));
    egui::Frame::NONE
        .fill(theme::BG_ELEVATED)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(URL_DIALOG_URL_PADDING as i8))
        .show(&mut content, |ui| {
            ui.set_width(inner.width() - 2.0 * URL_DIALOG_URL_PADDING);
            egui::ScrollArea::vertical()
                .id_salt(("made-url-dialog-url", pane_id))
                .max_height(dialog.url_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add(egui::Label::new(dialog.url.clone()));
                });
        });
    let mut action = None;
    content.allocate_ui_with_layout(
        egui::vec2(inner.width(), layout.button_height.max(24.0)),
        egui::Layout::right_to_left(Align::Center),
        |row| {
            let open = theme::primary_action(
                row,
                url_card_action_id(pane_id, "open"),
                "Open",
                layout.button_height,
                armed,
                theme::BG_CARD,
            );
            open.response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, armed, "Open link in browser")
            });
            if open.activated && armed {
                action = Some(UrlCardAction::Open);
            }
            let copy = theme::secondary_action(
                row,
                url_card_action_id(pane_id, "copy"),
                "Copy",
                layout.button_height,
                settled,
                theme::BG_CARD,
            );
            if copy.activated && settled {
                action = Some(UrlCardAction::Copy);
            }
            let dismiss = theme::ghost_action(
                row,
                url_card_action_id(pane_id, "dismiss"),
                "Dismiss",
                false,
                layout.button_height,
                theme::BG_CARD,
            );
            if dismiss.activated {
                action = Some(UrlCardAction::Dismiss);
            }
        },
    );
    action
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

// BEGIN remote-path (AIA-02)
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
// END remote-path

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

/// The dismiss control of one notice-line failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeDismiss {
    Failure,
    Report,
    Notice(crate::app::ScreenNotice),
}

impl NoticeDismiss {
    const ALL: [Self; 6] = [
        Self::Failure,
        Self::Report,
        Self::Notice(crate::app::ScreenNotice::Address),
        Self::Notice(crate::app::ScreenNotice::Notice),
        Self::Notice(crate::app::ScreenNotice::Tree),
        Self::Notice(crate::app::ScreenNotice::Launch),
    ];

    fn id(self) -> egui::Id {
        picker_control_id(match self {
            Self::Failure => "dismiss-failure",
            Self::Report => "dismiss-report",
            Self::Notice(crate::app::ScreenNotice::Address) => "dismiss-address",
            Self::Notice(crate::app::ScreenNotice::Notice) => "dismiss-notice",
            Self::Notice(crate::app::ScreenNotice::Tree) => "dismiss-tree",
            Self::Notice(crate::app::ScreenNotice::Launch) => "dismiss-launch",
        })
    }

    fn command(self) -> ScreenCommand {
        match self {
            Self::Failure => ScreenCommand::DismissFailure,
            Self::Report => ScreenCommand::DismissReport,
            Self::Notice(kind) => ScreenCommand::DismissNotice(kind),
        }
    }
}

/// One contextual notice line; with `dismiss`, an icon-only Dismiss control
/// sits at its end. Returns whether that control was activated.
fn notice_strip(
    ui: &mut Ui,
    layout: &Layout,
    fill: Color32,
    tone: Color32,
    text: &str,
    tooltip: &str,
    dismiss: Option<egui::Id>,
) -> bool {
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
    let control = if dismiss.is_some() {
        theme::icon_action_extent(24.0) + 4.0
    } else {
        0.0
    };
    theme::paint_ellipsized(
        ui.painter(),
        egui::pos2(rect.left() + 20.0, rect.center().y - 7.0),
        text,
        FontId::proportional(10.5),
        tone,
        (rect.width() - 28.0 - control).max(20.0),
    );
    response.on_hover_text(sanitize_display(tooltip, 700));
    let Some(id) = dismiss else {
        return false;
    };
    let extent = theme::icon_action_extent(24.0);
    let mut slot = ui.new_child(
        UiBuilder::new()
            .max_rect(egui::Rect::from_center_size(
                egui::pos2(rect.right() - 3.0 - extent / 2.0, rect.center().y),
                egui::Vec2::splat(extent),
            ))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    dismiss_action(&mut slot, id, fill)
}

/// Icon-only Dismiss control of a screen notice. Escape while it is focused
/// dismisses only that notice (see `screen_keyboard`).
fn dismiss_action(ui: &mut Ui, id: egui::Id, gap: Color32) -> bool {
    ui.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            id,
            egui::EventFilter {
                tab: false,
                horizontal_arrows: false,
                vertical_arrows: false,
                escape: true,
            },
        );
    });
    theme::icon_action(ui, id, theme::IconGlyph::Close, "Dismiss", false, 24.0, gap).activated
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
    use crate::integrations::claude_code::workspace_claude_branded;
    use crate::integrations::grok::workspace_grok_branded;
    use crate::integrations::{GROK, OPENCODE};
    use crate::session_map::SessionRow;
    use crate::tui::TerminalTarget;
    use std::ffi::OsString;
    use std::sync::MutexGuard;

    static APP_PATH_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn command_enter_never_overrides_another_action_that_took_enter() {
        use crate::open_workspace::LaunchKind;
        let temp = tempfile::tempdir().unwrap();
        let picker = crate::workspace_picker::WorkspacePicker::new(temp.path()).unwrap();
        let model = AppModel::from_launch(
            crate::workspace::WorkspaceLaunch::Picker {
                picker,
                error: None,
            },
            crate::app::AppOptions::new(),
        );
        let ctx = egui::Context::default();
        let keyboard = |command: &mut Option<ScreenCommand>| {
            let raw = egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }],
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| screen_keyboard(&model, ui, command));
            output.textures_delta.clear();
        };
        // Another action owns this Enter: it is not replaced by a launch.
        let mut command = Some(ScreenCommand::SaveForm);
        keyboard(&mut command);
        assert!(matches!(command, Some(ScreenCommand::SaveForm)));
        let mut command = Some(ScreenCommand::Navigate("/tmp".to_owned()));
        keyboard(&mut command);
        assert!(matches!(command, Some(ScreenCommand::Navigate(_))));
        // A focused TUI button took the Enter too: Terminal is still the
        // Cmd/Ctrl+Enter target, and only one launch runs.
        let mut command = Some(ScreenCommand::Launch(LaunchKind::Tui(OPENCODE)));
        keyboard(&mut command);
        assert!(matches!(
            command,
            Some(ScreenCommand::Launch(LaunchKind::Terminal))
        ));
        // Otherwise Cmd/Ctrl+Enter opens Terminal.
        let mut command = None;
        keyboard(&mut command);
        assert!(matches!(
            command,
            Some(ScreenCommand::Launch(LaunchKind::Terminal))
        ));
    }

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
                owner: None,
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
    fn screen_rows_are_cached_until_the_tree_or_refresh_changes() {
        let temp = tempfile::tempdir().unwrap();
        for index in 0..100 {
            std::fs::create_dir(temp.path().join(format!("dir-{index:03}"))).unwrap();
        }
        let picker = crate::workspace_picker::WorkspacePicker::new(temp.path()).unwrap();
        let mut model = AppModel::from_launch(
            crate::workspace::WorkspaceLaunch::Picker {
                picker,
                error: None,
            },
            crate::app::AppOptions::new(),
        );
        let ctx = egui::Context::default();
        model.sync_screen(&ctx);
        let first = model.screen_rows();
        let second = model.screen_rows();
        assert_eq!(first.len(), 100);
        assert!(
            Arc::ptr_eq(&first, &second),
            "unchanged tree must reuse sanitized rows"
        );
        std::fs::rename(temp.path().join("dir-050"), temp.path().join("renamed-050")).unwrap();
        model.screen_refresh();
        let third = model.screen_rows();
        assert!(!Arc::ptr_eq(&first, &third));
        assert!(third.iter().any(|row| row.name == "renamed-050"));
        assert!(third.iter().all(|row| row.name != "dir-050"));
        model.screen_toggle(0);
        let expanded = model.screen_rows();
        assert!(!Arc::ptr_eq(&third, &expanded));
        model.shutdown();
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
            (720.0, 480.0, 230.0, 261.0, 114.0),
            (1280.0, 800.0, 280.0, 311.0, 154.0),
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
    fn tab_labels_are_sanitized_bounded_and_named_after_folders() {
        let hostile = format!("a\nb\u{202e}c{}", "x".repeat(300));
        let label = bounded_label(&hostile).unwrap();
        assert!(
            !label.contains('\n') && !label.contains('\u{202e}'),
            "{label:?}"
        );
        assert!(
            label.chars().count() <= WORKSPACE_TAB_LABEL_MAX_CHARS,
            "{label}"
        );
        assert!(label.starts_with('a'), "{label}");
        assert_eq!(bounded_label(" \u{202e} "), None);
        assert_eq!(folder_name("/work/made/crates/made-app"), "made-app");
        assert_eq!(folder_name("/var/log/"), "log");
        assert_eq!(folder_name("/"), "/");
        assert_eq!(folder_name("/home/me/dir com espaço"), "dir com espaço");
    }

    #[test]
    fn workspace_tab_accessible_name_includes_session_summary() {
        let busy = sessions_summary(&[SessionActivity::Busy, SessionActivity::Idle])
            .expect("busy summary");
        assert_eq!(busy, "Sessions: 1 running, 1 idle");

        let attention = sessions_summary(&[
            SessionActivity::Attention,
            SessionActivity::Idle,
            SessionActivity::Busy,
        ])
        .expect("attention summary");
        assert_eq!(attention, "Sessions: 1 running, 1 needs attention, 1 idle");
        assert_eq!(
            sessions_summary(&[SessionActivity::Attention]),
            Some("Sessions: 1 needs attention".to_owned())
        );
        assert_eq!(
            sessions_summary(&[
                SessionActivity::Attention,
                SessionActivity::Attention,
                SessionActivity::Busy,
            ]),
            Some("Sessions: 1 running, 2 need attention".to_owned())
        );
        assert_eq!(
            sessions_summary(&[SessionActivity::Busy; 9]),
            Some("Sessions: 9 running".to_owned()),
            "the summary counts sessions beyond the eight painted dots"
        );
        assert_eq!(sessions_summary(&[]), None);

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
    fn claude_code_brand_yields_to_opencode_but_not_to_remote_grok_presentation() {
        // Present Claude Code brands the tab on its own, locally and remotely,
        // including over open remote Grok Build sessions.
        assert!(workspace_claude_branded(false, true));
        assert!(!workspace_claude_branded(false, false));
        // OpenCode keeps its label, icon and dots.
        assert!(!workspace_claude_branded(true, true));
    }

    #[test]
    fn a_remote_claude_tab_shows_owned_session_dots_and_the_local_tooltip_title() {
        use crate::integrations::claude_code::follow::{ClaudeActivity, ClaudeTab};
        use made_remote::protocol::{ClaudeActivity as Remote, ClaudeSessionInfo, RemotePath};
        use made_terminal::{ProtocolEvent, ProtocolState};

        let session = |id: &str, recency, activity, title: &str| ClaudeSessionInfo {
            session_id: id.to_owned(),
            pid: 40 + recency as u32,
            cwd: Some(RemotePath::new(format!("/srv/{id}").into_bytes()).unwrap()),
            activity,
            title: title.to_owned(),
            recency,
            nested: false,
        };
        let mut tab = ClaudeTab::new(0, false);
        tab.apply_remote_sessions(&[
            session("old", 1, Remote::Busy, "Older"),
            session("new", 2, Remote::Attention, "Newest\u{202e}"),
        ]);
        // One dot per live owned session, in attachment order, with the
        // hook-derived status.
        assert_eq!(
            tab.dots(),
            vec![ClaudeActivity::Busy, ClaudeActivity::Attention]
        );
        // Without a protocol status the latest session's sanitized hook title
        // is the tooltip title; the remote terminal title is not read.
        assert_eq!(tab.tooltip_title(Some("Remote title")), "Newest");
        assert_eq!(tab.directory().as_deref(), Some("/srv/new"));
        // A live protocol status applies to the latest session, and the tab's
        // own remote terminal title leads the tooltip.
        tab.note_protocol(ProtocolEvent::Status {
            state: ProtocolState::Working,
        });
        assert_eq!(tab.dots(), vec![ClaudeActivity::Busy, ClaudeActivity::Busy]);
        assert_eq!(tab.tooltip_title(Some("Remote title")), "Remote title");
        // A nested session never takes the tab over.
        let mut nested = session("nested", 3, Remote::Idle, "Nested");
        nested.nested = true;
        tab.apply_remote_sessions(&[session("new", 2, Remote::Idle, "Newest"), nested]);
        assert_eq!(tab.directory().as_deref(), Some("/srv/new"));
        // No session and no protocol: the default title.
        tab.apply_remote_sessions(&[]);
        tab.note_shell_prompt();
        assert!(!tab.present());
        assert_eq!(tab.tooltip_title(Some("Remote title")), "Claude Code");
    }

    #[test]
    fn compact_tab_width_and_picker_label_stay_within_targets() {
        let layout = Layout::for_window(720.0, 480.0);
        let row = row(0, true);
        assert!((112.0..=184.0).contains(&tab_width(&row, &layout)));
        assert_eq!(theme::format_bytes(4_300), "4.2 KB");
    }
}
