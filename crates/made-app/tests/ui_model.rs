//! Headless egui UI-model test: picker -> selected folder -> workspace
//! surfaces, without a display, GPU, window, PTY, or bridge.

mod support;

use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use made_app::app::{AppModel, AppOptions, WorkspaceId};
use made_app::cli::LaunchArgs;
use made_app::explorer_icons;
use made_app::explorer_view::{ExplorerPane, PreviewAction};
use made_app::integrations::claude_code;
use made_app::integrations::grok::{self, GrokDiagnosticsHold, GrokRunningHold};
use made_app::integrations::{IntegrationRef, CLAUDE_CODE, GROK, OPENCODE};
use made_app::ui::{
    about_diagnostics_toggle_id, diagnostics_copy_id, diagnostics_previous_log_copy_id,
    diagnostics_previous_log_path_id, explorer_splitter_id, left_surface_action_id,
    new_tab_menu_item_id, picker_control_id, picker_current_target, picker_list_id, picker_path_id,
    ssh_field_id, ssh_server_row_id, ssh_servers_window_id, top_action_id, top_workspace_action_id,
    workspace_tab_close_id, workspace_tab_id,
};
use made_app::workbench::LeftSurface;
use made_app::workspace::{
    LaunchPlan, Surfaces, WorkspaceLaunch, WorkspaceSelection, WorkspaceSource,
};
use made_app::workspace_picker::WorkspacePicker;
use made_app::{TerminalTarget, Workbench};
use made_platform::native_backdrop::{NativeBackdropKind, NativeBackdropStatus};
use serde_json::{json, Value};
use tungstenite::{Message, WebSocket};

fn raw_input() -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 620.0),
        )),
        ..Default::default()
    }
}

#[test]
fn egui_win_does_not_swallow_terminal_clipboard_keys() {
    assert!(
        !made_app::native_options().clipboard_shortcuts,
        "egui-winit must not consume Ctrl/Cmd+C/X/V before the terminal encoder"
    );
}

#[test]
fn app_model_without_a_workspace_opens_the_default_home_not_the_picker() {
    let mut model = AppModel::new(
        LaunchArgs {
            workspace: WorkspaceSelection::None,
            smoke_test: None,
        },
        AppOptions::new(),
    );
    assert!(!model.is_picker());
    assert_eq!(model.launch().source(), WorkspaceSource::DefaultHome);
    model.shutdown();
}

#[test]
fn app_model_smoke_without_a_workspace_stays_on_the_picker() {
    let mut model = AppModel::new(
        LaunchArgs {
            workspace: WorkspaceSelection::None,
            smoke_test: Some(Duration::from_millis(50)),
        },
        AppOptions::new(),
    );
    assert!(model.is_picker());
    assert_eq!(model.launch().source(), WorkspaceSource::Picker);
    assert!(!model.start_pending());
    model.shutdown();
}

#[test]
fn app_model_smoke_with_an_explicit_workspace_opens_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut model = AppModel::new(
        LaunchArgs {
            workspace: WorkspaceSelection::Explicit(temp.path().to_path_buf()),
            smoke_test: Some(Duration::from_millis(50)),
        },
        AppOptions::new(),
    );
    assert!(!model.is_picker());
    assert_eq!(model.launch().source(), WorkspaceSource::LaunchArgument);
    model.shutdown();
}

#[test]
fn headless_ui_installs_the_calm_theme_and_transparency_request() {
    let temp = tempfile::tempdir().unwrap();
    let picker = WorkspacePicker::new(temp.path()).unwrap();
    let launch = WorkspaceLaunch::Picker {
        picker,
        error: None,
    };
    let mut model = AppModel::from_launch(launch, AppOptions::new());
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(raw_input(), |ui| model.ui(ui));
    output.textures_delta.clear();

    let style = ctx.style_of(egui::Theme::Dark);
    assert_eq!(style.text_styles[&egui::TextStyle::Small].size, 10.0);
    assert_eq!(style.text_styles[&egui::TextStyle::Body].size, 12.5);
    assert_eq!(style.text_styles[&egui::TextStyle::Monospace].size, 13.0);
    assert_eq!(style.visuals.window_fill, made_app::theme::BG_CARD);
    assert!(style.visuals.weak_text_color.is_some());
    assert_eq!(
        made_app::native_options().viewport.transparent,
        Some(made_app::theme::TRANSLUCENT_WINDOW)
    );
    assert_eq!(made_app::ui::picker_primary_label(), "Use current folder");
    model.shutdown();
}

#[test]
fn picker_selection_transitions_to_workspace_surfaces_without_a_display() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("alpha")).unwrap();
    fs::create_dir(temp.path().join("beta")).unwrap();
    fs::write(temp.path().join("README.md"), "fixture").unwrap();

    let picker = WorkspacePicker::new(temp.path()).unwrap();
    let launch = WorkspaceLaunch::Picker {
        picker,
        error: None,
    };
    let mut model = AppModel::from_launch(launch, AppOptions::new());

    assert!(model.is_picker());
    assert_eq!(model.surfaces(), Surfaces::Picker);
    assert!(model.workbench().is_none());

    // Render the picker headlessly. This exercises real egui layout and text
    // without creating a window or a graphics context.
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(raw_input(), |ui| model.ui(ui));
    assert!(
        !output.shapes.is_empty(),
        "the picker rendered no egui shapes"
    );
    // Headless test: discard the texture upload the real renderer would apply.
    output.textures_delta.clear();

    // Select the first child folder and confirm it as the workspace. The plan
    // is validated by the same model the real picker uses.
    model.picker_select(0).unwrap();
    let selected_name = model
        .launch()
        .picker()
        .unwrap()
        .selected_row()
        .unwrap()
        .name()
        .to_owned();
    assert_eq!(selected_name, "alpha");
    let plan = model.launch().picker().unwrap().confirm_row(0).unwrap();
    let expected_root = plan.explorer_root.clone();
    model.open_plan(plan);

    assert!(!model.is_picker());
    assert_eq!(model.surfaces(), Surfaces::full());
    assert!(model.surfaces().has_terminal());
    assert!(model.surfaces().has_explorer());
    assert!(model.start_pending());
    assert!(
        model.workbench().is_none(),
        "headless test must not spawn runtime"
    );
    assert_eq!(model.launch().terminal_cwd(), Some(expected_root.as_path()));
    assert_eq!(
        model.launch().explorer_root(),
        Some(expected_root.as_path())
    );

    // The next UI pass shows the pre-runtime starting surface, still without a
    // display or child process.
    let mut output = ctx.run_ui(raw_input(), |ui| model.ui(ui));
    assert!(!output.shapes.is_empty());
    output.textures_delta.clear();
    assert!(model.workbench().is_none());

    model.shutdown();
}

#[test]
fn confirming_the_current_picker_folder_selects_both_surfaces() {
    let temp = tempfile::tempdir().unwrap();
    let picker = WorkspacePicker::new(temp.path()).unwrap();
    let launch = WorkspaceLaunch::Picker {
        picker,
        error: None,
    };
    let mut model = AppModel::from_launch(launch, AppOptions::new());

    model.confirm_picker().expect("confirm current folder");
    assert_eq!(model.surfaces(), Surfaces::full());
    assert!(model.start_pending());
    let expected = fs::canonicalize(temp.path()).unwrap();
    assert_eq!(model.launch().terminal_cwd(), Some(expected.as_path()));
    assert_eq!(model.launch().explorer_root(), Some(expected.as_path()));
    model.shutdown();
}

#[test]
fn an_invalid_workspace_keeps_the_terminal_surface_and_reports_the_error() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("does-not-exist");
    let launch = WorkspaceLaunch::from_arg(Some(missing.as_os_str()));
    let model = AppModel::from_launch(launch, AppOptions::new());

    assert_eq!(model.surfaces(), Surfaces::terminal_only());
    assert!(model.surfaces().has_terminal());
    assert!(!model.surfaces().has_explorer());
    assert!(model.launch().error().is_some());
    assert!(model.launch().terminal_cwd().is_some());
}

// ── Headless GUI-model harness for the quiet workspace contract ────────────

struct Headless {
    ctx: egui::Context,
    model: AppModel,
    width: f32,
    height: f32,
    started: Instant,
    frame: u64,
    last_time: f64,
}

impl Headless {
    fn new(model: AppModel, width: f32, height: f32) -> Self {
        let ctx = egui::Context::default();
        made_app::theme::install(&ctx);
        Self {
            ctx,
            model,
            width,
            height,
            started: Instant::now(),
            frame: 0,
            last_time: 0.0,
        }
    }

    fn raw(&mut self, events: Vec<egui::Event>) -> egui::RawInput {
        self.frame = self.frame.saturating_add(1);
        let wall = self.started.elapsed().as_secs_f64();
        let time = wall.max(self.last_time);
        self.last_time = time;
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(self.width, self.height),
            )),
            time: Some(time),
            events,
            ..Default::default()
        }
    }

    fn render(&mut self) -> egui::FullOutput {
        self.model.logic(&self.ctx);
        let raw = self.raw(Vec::new());
        let mut output = self.ctx.run_ui(raw, |ui| self.model.ui(ui));
        output.textures_delta.clear();
        output
    }

    fn render_events(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.model.logic(&self.ctx);
        let raw = self.raw(events);
        let mut output = self.ctx.run_ui(raw, |ui| self.model.ui(ui));
        output.textures_delta.clear();
        output
    }

    fn render_events_at(&mut self, events: Vec<egui::Event>, time: f64) -> egui::FullOutput {
        self.model.logic(&self.ctx);
        let min_time = self.last_time + 1.0 / 60.0;
        let mut raw = self.raw(events);
        let time = time.max(min_time);
        self.last_time = time;
        raw.time = Some(time);
        let mut output = self.ctx.run_ui(raw, |ui| self.model.ui(ui));
        output.textures_delta.clear();
        output
    }

    /// Render the app UI without running model logic, keeping startup pending.
    fn render_ui_only(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        let raw = self.raw(events);
        let mut output = self.ctx.run_ui(raw, |ui| self.model.ui(ui));
        output.textures_delta.clear();
        output
    }

    fn response(&self, id: egui::Id) -> egui::Response {
        self.ctx
            .read_response(id)
            .unwrap_or_else(|| panic!("widget {id:?} was not rendered"))
    }

    fn click(&mut self, id: egui::Id) -> egui::FullOutput {
        // Two passes settle panel sizes after a tab changes, then the pointer
        // sequence hits the freshly registered widget.
        self.render();
        self.render();
        let position = self.response(id).rect.center();
        self.render_events(vec![egui::Event::PointerMoved(position)]);
        self.render_events(vec![pointer_button(position, true)]);
        self.render_events(vec![pointer_button(position, false)])
    }

    fn drag(&mut self, id: egui::Id, delta: egui::Vec2) {
        self.render();
        self.render();
        let start = self.response(id).rect.center();
        let end = start + delta;
        self.render_events(vec![egui::Event::PointerMoved(start)]);
        self.render_events(vec![pointer_button(start, true)]);
        self.render_events(vec![egui::Event::PointerMoved(end)]);
        self.render_events(vec![pointer_button(end, false)]);
    }

    fn focus(&mut self, id: egui::Id) {
        self.ctx.memory_mut(|memory| memory.request_focus(id));
        // One pass registers focus; a second lets the widget install its
        // arrow-key focus-lock filter before any test key is injected.
        self.render();
        self.render();
    }

    fn hover(&mut self, id: egui::Id) -> egui::FullOutput {
        self.render();
        self.render();
        // Leave the previous icon first. Moving directly between adjacent
        // icons can keep the old tooltip's Area alive for one pass, which
        // hides the new tooltip in that frame.
        self.render_events(vec![egui::Event::PointerMoved(egui::pos2(
            4.0,
            self.height - 4.0,
        ))]);
        self.render();
        let position = self.response(id).rect.center();
        self.render_events(vec![egui::Event::PointerMoved(position)]);
        self.render_events(vec![egui::Event::PointerMoved(position)]);
        // Tooltip Areas are created on the pass after hover begins.
        self.render()
    }

    /// Hover until `ready` accepts the rendered text. The assertions remain
    /// exact; this only tolerates tooltip-Area creation lag under a loaded
    /// runner.
    fn hover_until(
        &mut self,
        id: egui::Id,
        mut ready: impl FnMut(&str) -> bool,
    ) -> egui::FullOutput {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let output = self.hover(id);
            if ready(&output_text(&output)) || Instant::now() >= deadline {
                return output;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Hover until the tooltip text appears. The assertion remains exact; this
    /// only tolerates tooltip-Area creation lag under a loaded runner.
    fn hover_until_text(&mut self, id: egui::Id, needle: &str) -> egui::FullOutput {
        self.hover_until(id, |text| text.contains(needle))
    }

    fn press_hold(&mut self, id: egui::Id) -> egui::FullOutput {
        self.render();
        self.render();
        let position = self.response(id).rect.center();
        self.render_events(vec![egui::Event::PointerMoved(position)]);
        self.render_events(vec![pointer_button(position, true)])
    }

    fn release(&mut self, id: egui::Id) {
        let position = self.response(id).rect.center();
        self.render_events(vec![pointer_button(position, false)]);
    }

    fn press_key(&mut self, key: egui::Key) {
        self.press_key_with_modifiers(key, egui::Modifiers::NONE);
    }

    fn press_key_with_modifiers(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        self.render_events(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
        self.render_events(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        }]);
    }

    fn settle_animation(&mut self) {
        for _ in 0..12 {
            self.render();
            std::thread::sleep(Duration::from_millis(16));
        }
    }

    /// Advance realtime-driven egui animations by rendering fixed 1/60s frames.
    /// `stable_dt` clamps a single large jump, so fade tests need several frames.
    fn advance(&mut self, seconds: f64) {
        let base = self.last_time;
        let frames = ((seconds / (1.0 / 60.0)).ceil() as usize).max(1);
        for frame in 1..=frames {
            self.render_events_at(Vec::new(), base + frame as f64 / 60.0);
        }
    }

    fn pump_until(&mut self, mut predicate: impl FnMut(&AppModel) -> bool) {
        // Hosted CI runners, especially macOS arm64, can take longer than the
        // local machine to spawn the fixture PTY. The predicate still has to
        // become true; only the observation window grows.
        let deadline = Instant::now() + Duration::from_secs(20);
        while !predicate(&self.model) && Instant::now() < deadline {
            self.model.logic(&self.ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            predicate(&self.model),
            "runtime did not reach expected state"
        );
    }

    fn pump_until_rows(&mut self, id: WorkspaceId, rows: usize) {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            self.render();
            let visible = self
                .model
                .workspace_mut(id)
                .and_then(Workbench::explorer_mut)
                .map(ExplorerPane::visible_row_count)
                .unwrap_or(0);
            if visible >= rows {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "explorer did not reach {rows} rows"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Headless {
    fn drop(&mut self) {
        self.model.shutdown();
    }
}

fn picker_headless(start: &Path, width: f32, height: f32) -> Headless {
    let picker = WorkspacePicker::new(start).unwrap();
    let launch = WorkspaceLaunch::Picker {
        picker,
        error: None,
    };
    Headless::new(
        AppModel::from_launch(launch, AppOptions::new()),
        width,
        height,
    )
}

fn pointer_button(position: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn typed_events(text: &str) -> Vec<egui::Event> {
    let mut events = Vec::new();
    for character in text.trim_end_matches('\n').chars() {
        events.push(egui::Event::Text(character.to_string()));
    }
    events.push(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    events
}

fn type_terminal_command(workbench: &mut Workbench, command: &str) {
    let mut log = made_app::DiagnosticLog::new();
    let terminal = workbench.terminal_mut().unwrap();
    for ch in command.chars() {
        terminal.send_key(
            made_terminal::Key::Char(ch),
            made_terminal::Modifiers::default(),
            &mut log,
        );
    }
    terminal.send_key(
        made_terminal::Key::Enter,
        made_terminal::Modifiers::default(),
        &mut log,
    );
}

fn headless_set_width(model: &mut AppModel, id: WorkspaceId, width: f32) {
    model
        .workspace_mut(id)
        .expect("live workspace")
        .set_explorer_width(width);
}

fn bridge_options(workbench: &Workbench) -> Option<Value> {
    let content = workbench
        .terminal()?
        .options()
        .env
        .iter()
        .find(|(key, _)| key == made_bridge::config::CONFIG_CONTENT_ENV)?
        .1
        .to_str()?;
    let config: Value = serde_json::from_str(content).ok()?;
    config["plugins"]
        .as_array()?
        .iter()
        .find_map(|entry| entry.get("options")?.get("made").cloned())
}

fn bridge_token(workbench: &Workbench) -> Option<String> {
    bridge_options(workbench)?
        .get("token")?
        .as_str()
        .map(str::to_owned)
}

fn send(socket: &mut WebSocket<TcpStream>, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .unwrap();
}

fn read(socket: &mut WebSocket<TcpStream>) -> Value {
    for _ in 0..16 {
        if let Message::Text(text) = socket.read().unwrap() {
            let value: Value = serde_json::from_str(&text).unwrap();
            if value["type"] == "ping" {
                send(socket, json!({"type":"pong", "nonce":value["nonce"]}));
            } else {
                return value;
            }
        }
    }
    panic!("too many control frames");
}

fn connect(options: &Value) -> WebSocket<TcpStream> {
    let url = options["bridgeUrl"].as_str().unwrap();
    let address = url
        .strip_prefix("ws://")
        .unwrap()
        .strip_suffix("/made")
        .unwrap();
    let stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let (mut socket, _) = tungstenite::client(url, stream).unwrap();
    send(
        &mut socket,
        json!({"type":"hello", "protocolVersion":1, "token":options["token"], "paneId":options["paneId"], "pluginVersion":"test", "opencodeVersion":"2.1.4", "pid":123, "cwd":"/"}),
    );
    assert_eq!(read(&mut socket)["type"], "welcome");
    socket
}

fn send_state(socket: &mut WebSocket<TcpStream>, revision: u64, directory: &Path) {
    send(
        socket,
        json!({"type":"state", "revision":revision, "activeSessionID":"ui-session", "tabsEnabled":true,
        "sessions":[{"sessionID":"ui-session", "title":"Owned session", "active":true, "busy":false, "attention":false, "unread":false,
        "promptPulse":0, "renaming":false, "directory":directory, "subpath":null, "projectID":null}]}),
    );
}

/// Keep the fake plugin's bridge connection alive on a worker.
///
/// The bridge server drops a client that misses three heartbeat pings, so a
/// test that stops reading its socket after `send_state` must keep answering
/// the protocol's JSON pings while it pumps the model; under load a slow run
/// otherwise loses the session state the assertions render.
fn service_bridge_heartbeat(socket: WebSocket<TcpStream>) {
    std::thread::spawn(move || {
        let mut socket = socket;
        let _ = socket
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(250)));
        loop {
            match socket.read() {
                Ok(Message::Text(text)) => {
                    let Ok(value) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    if value["type"] == "ping" {
                        let reply = Message::Text(
                            json!({"type":"pong", "nonce":value["nonce"]})
                                .to_string()
                                .into(),
                        );
                        if socket.send(reply).is_err() {
                            return;
                        }
                    }
                }
                Ok(_) => continue,
                Err(tungstenite::Error::Io(error)) => match error.kind() {
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => continue,
                    _ => return,
                },
                Err(_) => return,
            }
        }
    });
}

/// Send a session snapshot with explicit `(id, busy, attention)` rows.
fn send_sessions_state(
    socket: &mut WebSocket<TcpStream>,
    revision: u64,
    directory: &Path,
    sessions: &[(&str, bool, bool)],
) {
    let active_session_id = sessions.first().map(|(id, _, _)| *id);
    let sessions: Vec<Value> = sessions
        .iter()
        .enumerate()
        .map(|(index, (id, busy, attention))| {
            json!({
                "sessionID": id,
                "title": format!("Session {index}"),
                "active": index == 0,
                "busy": busy,
                "attention": attention,
                "unread": false,
                "promptPulse": 0,
                "renaming": false,
                "directory": directory,
                "subpath": null,
                "projectID": null
            })
        })
        .collect();
    send(
        socket,
        json!({
            "type": "state",
            "revision": revision,
            "activeSessionID": active_session_id,
            "tabsEnabled": true,
            "sessions": sessions
        }),
    );
}

fn assert_text_clipped_inside(output: &egui::FullOutput, screen: egui::Rect) {
    fn visit(shape: &egui::Shape, clip: egui::Rect, screen: egui::Rect) {
        match shape {
            egui::Shape::Text(_) => {
                assert!(
                    screen.contains_rect(clip),
                    "text clip outside screen: {clip:?}"
                );
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, clip, screen);
                }
            }
            _ => {}
        }
    }
    for clipped in &output.shapes {
        visit(&clipped.shape, clipped.clip_rect, screen);
    }
}

fn output_text(output: &egui::FullOutput) -> String {
    fn visit(shape: &egui::Shape, text: &mut String) {
        match shape {
            egui::Shape::Text(text_shape) => {
                text.push_str(&text_shape.galley.job.text);
                text.push('\n');
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, text);
                }
            }
            _ => {}
        }
    }
    let mut text = String::new();
    for clipped in &output.shapes {
        visit(&clipped.shape, &mut text);
    }
    text
}

fn text_shape_count(output: &egui::FullOutput) -> usize {
    fn visit(shape: &egui::Shape, count: &mut usize) {
        match shape {
            egui::Shape::Text(_) => *count += 1,
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, count);
                }
            }
            _ => {}
        }
    }
    let mut count = 0;
    for clipped in &output.shapes {
        visit(&clipped.shape, &mut count);
    }
    count
}

fn has_stroke_color(output: &egui::FullOutput, color: egui::Color32) -> bool {
    fn visit(shape: &egui::Shape, color: egui::Color32) -> bool {
        match shape {
            egui::Shape::Rect(rect) => rect.stroke.color == color,
            egui::Shape::LineSegment { stroke, .. } => stroke.color == color,
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| visit(shape, color)),
            _ => false,
        }
    }
    output
        .shapes
        .iter()
        .any(|clipped| visit(&clipped.shape, color))
}

fn shape_hits_point(output: &egui::FullOutput, point: egui::Pos2) -> bool {
    fn visit(shape: &egui::Shape, point: egui::Pos2) -> bool {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| visit(shape, point)),
            egui::Shape::Noop | egui::Shape::Callback(_) => false,
            _ => shape.visual_bounding_rect().contains(point),
        }
    }
    output
        .shapes
        .iter()
        .any(|clipped| visit(&clipped.shape, point))
}

fn largest_rect(output: &egui::FullOutput, fill: egui::Color32) -> Option<egui::Rect> {
    fn visit(shape: &egui::Shape, fill: egui::Color32, best: &mut Option<egui::Rect>) {
        match shape {
            egui::Shape::Rect(rect_shape) if rect_shape.fill == fill => {
                let area = rect_shape.rect.width() * rect_shape.rect.height();
                if best.is_none_or(|current| area > current.width() * current.height()) {
                    *best = Some(rect_shape.rect);
                }
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, fill, best);
                }
            }
            _ => {}
        }
    }
    let mut best = None;
    for clipped in &output.shapes {
        visit(&clipped.shape, fill, &mut best);
    }
    best
}

fn rects_with_fill(output: &egui::FullOutput, fill: egui::Color32) -> Vec<egui::Rect> {
    fn visit(shape: &egui::Shape, fill: egui::Color32, rects: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Rect(rect) if rect.fill == fill => rects.push(rect.rect),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, fill, rects);
                }
            }
            _ => {}
        }
    }
    let mut rects = Vec::new();
    for clipped in &output.shapes {
        visit(&clipped.shape, fill, &mut rects);
    }
    rects
}

fn rect_shapes(output: &egui::FullOutput) -> Vec<egui::epaint::RectShape> {
    fn visit(shape: &egui::Shape, rects: &mut Vec<egui::epaint::RectShape>) {
        match shape {
            egui::Shape::Rect(rect) => rects.push(rect.clone()),
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

fn path_shapes(output: &egui::FullOutput) -> Vec<egui::epaint::PathShape> {
    fn visit(shape: &egui::Shape, paths: &mut Vec<egui::epaint::PathShape>) {
        match shape {
            egui::Shape::Path(path) => paths.push(path.clone()),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, paths);
                }
            }
            _ => {}
        }
    }
    let mut paths = Vec::new();
    for clipped in &output.shapes {
        visit(&clipped.shape, &mut paths);
    }
    paths
}

fn circle_shapes(output: &egui::FullOutput) -> Vec<egui::epaint::CircleShape> {
    fn visit(shape: &egui::Shape, circles: &mut Vec<egui::epaint::CircleShape>) {
        match shape {
            egui::Shape::Circle(circle) => circles.push(*circle),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, circles);
                }
            }
            _ => {}
        }
    }
    let mut circles = Vec::new();
    for clipped in &output.shapes {
        visit(&clipped.shape, &mut circles);
    }
    circles
}

/// Center of the last (rightmost) session status dot.
fn status_corner_center(tab: egui::Rect) -> egui::Pos2 {
    egui::pos2(tab.right() - 10.0, tab.center().y)
}

/// Session dots painted in the tab's right corner, ordered left to right.
fn tab_session_dots(output: &egui::FullOutput, tab: egui::Rect) -> Vec<egui::epaint::CircleShape> {
    let corner = status_corner_center(tab);
    let mut dots: Vec<_> = circle_shapes(output)
        .into_iter()
        .filter(|circle| {
            (circle.radius - 2.0).abs() < 0.1
                && (circle.center.y - corner.y).abs() < 0.75
                && circle.center.x <= corner.x + 0.75
                && circle.center.x >= corner.x - 7.0 * 7.0 - 0.75
        })
        .collect();
    dots.sort_by(|left, right| left.center.x.total_cmp(&right.center.x));
    dots
}

/// Center of dot `index` in a right-aligned row of `count` dots.
fn session_dot_center(tab: egui::Rect, index: usize, count: usize) -> egui::Pos2 {
    egui::pos2(
        tab.right() - 10.0 - (count - 1 - index) as f32 * 7.0,
        tab.center().y,
    )
}

fn session_dot_rgb(dot: &egui::epaint::CircleShape) -> (u8, u8, u8) {
    let [r, g, b, _] = dot.fill.to_srgba_unmultiplied();
    (r, g, b)
}

fn session_dot_alpha(dot: &egui::epaint::CircleShape) -> u8 {
    dot.fill.to_srgba_unmultiplied()[3]
}

fn assert_session_dot_color(dot: &egui::epaint::CircleShape, tone: egui::Color32, label: &str) {
    let (r, g, b) = session_dot_rgb(dot);
    let [expected_r, expected_g, expected_b, _] = tone.to_srgba_unmultiplied();
    assert!(
        (r as i32 - expected_r as i32).abs() <= 1
            && (g as i32 - expected_g as i32).abs() <= 1
            && (b as i32 - expected_b as i32).abs() <= 1,
        "{label} must use its documented tone: ({r},{g},{b}) vs ({expected_r},{expected_g},{expected_b})"
    );
}

fn path_center(path: &egui::epaint::PathShape) -> egui::Pos2 {
    egui::Rect::from_points(&path.points).center()
}

/// The 16px icon slot of a 24px explorer row: the 8px slot start plus 14px
/// per depth, centered vertically on the row.
fn icon_slot(row: egui::Rect, depth: usize) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(row.left() + 8.0 + 14.0 * depth as f32, row.center().y - 8.0),
        egui::vec2(16.0, 16.0),
    )
}

/// A texture drawn by `egui::Image`, with the geometry and paint it carries.
#[derive(Debug, Clone, Copy)]
struct PaintedImage {
    rect: egui::Rect,
    texture: egui::TextureId,
    tint: egui::Color32,
    stroke: egui::Stroke,
    corner_radius: egui::CornerRadius,
}

fn painted_images(output: &egui::FullOutput) -> Vec<PaintedImage> {
    fn visit(shape: &egui::Shape, images: &mut Vec<PaintedImage>) {
        match shape {
            egui::Shape::Rect(rect) if rect.fill_texture_id() != egui::TextureId::default() => {
                images.push(PaintedImage {
                    rect: rect.rect,
                    texture: rect.fill_texture_id(),
                    tint: rect.fill,
                    stroke: rect.stroke,
                    corner_radius: rect.corner_radius,
                });
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, images);
                }
            }
            _ => {}
        }
    }
    let mut images = Vec::new();
    for clipped in &output.shapes {
        visit(&clipped.shape, &mut images);
    }
    images
}

/// The painted 16px icon whose slot is centered in `row` at `depth`.
fn icon_in_slot(output: &egui::FullOutput, row: egui::Rect, depth: usize) -> Option<PaintedImage> {
    let slot = icon_slot(row, depth);
    painted_images(output).into_iter().find(|image| {
        (image.rect.center() - slot.center()).length() < 0.75
            && (image.rect.width() - 16.0).abs() < 0.75
            && (image.rect.height() - 16.0).abs() < 0.75
    })
}

/// The texture egui uses for one icon at explorer size. It loads through the
/// same URI, texture options, and size hint that `egui::Image` paints with.
fn icon_texture(ctx: &egui::Context, icon: explorer_icons::IconId) -> egui::TextureId {
    icon_texture_px(ctx, icon, 16, 16)
}

/// Same as [`icon_texture`] at an explicit pixel size. `Image::paint_at` asks
/// for `pixels_per_point * rect`, so a 16px icon is not always a 16px texture.
fn icon_texture_px(
    ctx: &egui::Context,
    icon: explorer_icons::IconId,
    width: u32,
    height: u32,
) -> egui::TextureId {
    ctx.include_bytes(icon.uri(), icon.bytes());
    let hint = egui::load::SizeHint::Size {
        width,
        height,
        maintain_aspect_ratio: false,
    };
    match ctx.try_load_texture(icon.uri(), egui::TextureOptions::default(), hint) {
        Ok(egui::load::TexturePoll::Ready { texture }) => texture.id,
        Ok(egui::load::TexturePoll::Pending { .. }) => {
            panic!("{} is still loading", icon.name())
        }
        Err(error) => panic!("{} failed to load: {error:?}", icon.name()),
    }
}

/// Texture for an SVG embedded outside the Material inventory. `uri` is the
/// same `bytes://` key `Image::paint_at` uses, at the pixel size it requests.
fn embedded_svg_texture(
    ctx: &egui::Context,
    uri: &'static str,
    bytes: &'static [u8],
    width: u32,
    height: u32,
) -> egui::TextureId {
    ctx.include_bytes(uri, bytes);
    let hint = egui::load::SizeHint::Size {
        width,
        height,
        maintain_aspect_ratio: false,
    };
    match ctx.try_load_texture(uri, egui::TextureOptions::default(), hint) {
        Ok(egui::load::TexturePoll::Ready { texture }) => texture.id,
        Ok(egui::load::TexturePoll::Pending { .. }) => panic!("{uri} is still loading"),
        Err(error) => panic!("{uri} failed to load: {error:?}"),
    }
}

/// The vendored icon whose SVG file is `name`. Expectations name the file the
/// theme maps, so they do not simply echo whatever the resolver returns.
fn icon_named(name: &str) -> explorer_icons::IconId {
    explorer_icons::icon_ids()
        .find(|id| id.name() == name)
        .unwrap_or_else(|| panic!("{name} is not a vendored icon"))
}

/// Single-glyph text marks: the glyph's rect and its resolved color.
fn glyph_marks(output: &egui::FullOutput, glyph: &str) -> Vec<(egui::Rect, egui::Color32)> {
    fn visit(shape: &egui::Shape, glyph: &str, marks: &mut Vec<(egui::Rect, egui::Color32)>) {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text == glyph => {
                let color =
                    text.galley
                        .job
                        .sections
                        .first()
                        .map_or(text.fallback_color, |section| {
                            if section.format.color == egui::Color32::PLACEHOLDER {
                                text.fallback_color
                            } else {
                                section.format.color
                            }
                        });
                marks.push((
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                    color,
                ));
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, glyph, marks);
                }
            }
            _ => {}
        }
    }
    let mut marks = Vec::new();
    for clipped in &output.shapes {
        visit(&clipped.shape, glyph, &mut marks);
    }
    marks
}

/// Stable widget id of the tree row at `index`.
fn explorer_row_id(headless: &Headless, id: WorkspaceId, index: usize) -> egui::Id {
    headless
        .model
        .workspace(id)
        .and_then(Workbench::explorer)
        .expect("explorer")
        .row_widget_id(index)
}

/// Rect of the tree row at `index`, as laid out in the last frame.
fn explorer_row_rect(headless: &Headless, id: WorkspaceId, index: usize) -> egui::Rect {
    headless.response(explorer_row_id(headless, id, index)).rect
}

/// Restores a directory's mode when dropped so the temporary tree can be removed.
#[cfg(unix)]
struct RestoreMode(PathBuf);

#[cfg(unix)]
impl Drop for RestoreMode {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}

fn focus_ring_rects(output: &egui::FullOutput, bounds: egui::Rect) -> Vec<egui::Rect> {
    rect_shapes(output)
        .into_iter()
        .filter(|shape| {
            shape.stroke.color == made_app::theme::FOCUS && shape.rect.intersects(bounds)
        })
        .map(|shape| shape.rect)
        .collect()
}

fn text_colors(output: &egui::FullOutput, needle: &str, max_y: f32) -> Vec<egui::Color32> {
    fn visit(shape: &egui::Shape, needle: &str, max_y: f32, colors: &mut Vec<egui::Color32>) {
        match shape {
            egui::Shape::Text(text)
                if text.pos.y <= max_y && text.galley.job.text.contains(needle) =>
            {
                for section in &text.galley.job.sections {
                    let color = if section.format.color == egui::Color32::PLACEHOLDER {
                        text.fallback_color
                    } else {
                        section.format.color
                    };
                    colors.push(color);
                }
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, needle, max_y, colors);
                }
            }
            _ => {}
        }
    }
    let mut colors = Vec::new();
    for clipped in &output.shapes {
        visit(&clipped.shape, needle, max_y, &mut colors);
    }
    colors
}

fn text_font_sizes(output: &egui::FullOutput, needle: &str, max_y: f32) -> Vec<f32> {
    fn visit(shape: &egui::Shape, needle: &str, max_y: f32, sizes: &mut Vec<f32>) {
        match shape {
            egui::Shape::Text(text)
                if text.pos.y <= max_y && text.galley.job.text.contains(needle) =>
            {
                for section in &text.galley.job.sections {
                    sizes.push(section.format.font_id.size);
                }
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, needle, max_y, sizes);
                }
            }
            _ => {}
        }
    }
    let mut sizes = Vec::new();
    for clipped in &output.shapes {
        visit(&clipped.shape, needle, max_y, &mut sizes);
    }
    sizes
}

fn text_rects(output: &egui::FullOutput, needle: &str, max_y: f32) -> Vec<egui::Rect> {
    fn visit(shape: &egui::Shape, needle: &str, max_y: f32, rects: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Text(text)
                if text.pos.y <= max_y && text.galley.job.text.contains(needle) =>
            {
                rects.push(egui::Rect::from_min_size(text.pos, text.galley.size()));
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, needle, max_y, rects);
                }
            }
            _ => {}
        }
    }
    let mut rects = Vec::new();
    for clipped in &output.shapes {
        visit(&clipped.shape, needle, max_y, &mut rects);
    }
    rects
}

/// One painted menu label: the galley line box, the shared baseline, and the
/// tight ink. The ink bottom includes descenders; the letterform body does not.
struct PaintedMenuLabel {
    line: egui::Rect,
    baseline_y: f32,
    ink: egui::Rect,
}

fn painted_menu_label(
    output: &egui::FullOutput,
    needle: &str,
    row: egui::Rect,
) -> PaintedMenuLabel {
    fn find(shape: &egui::Shape, needle: &str, row: egui::Rect) -> Option<PaintedMenuLabel> {
        match shape {
            egui::Shape::Text(text) => {
                let line = egui::Rect::from_min_size(text.pos, text.galley.size());
                if !text.galley.job.text.contains(needle) || !row.intersects(line) {
                    return None;
                }
                let placed = text
                    .galley
                    .rows
                    .first()
                    .unwrap_or_else(|| panic!("{needle} label has no text row"));
                let glyph = placed
                    .glyphs
                    .first()
                    .unwrap_or_else(|| panic!("{needle} label has no glyphs"));
                Some(PaintedMenuLabel {
                    line,
                    baseline_y: text.pos.y + placed.pos.y + glyph.pos.y,
                    ink: text.galley.mesh_bounds.translate(text.pos.to_vec2()),
                })
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, needle, row)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|clipped| find(&clipped.shape, needle, row))
        .unwrap_or_else(|| panic!("{needle} label was not painted in {row:?}"))
}

/// Midpoint of the ink top and the baseline. Ignores descenders below the baseline.
fn letterform_center_y(label: &PaintedMenuLabel) -> f32 {
    (label.ink.top() + label.baseline_y) * 0.5
}

/// Set by the isolated-child helper so fixture helpers that persist app data
/// can prove they resolve their app-data directory under the test temp root.
const ISOLATED_TEST_HOME_ENV: &str = "MADE_UI_MODEL_TEST_HOME";

fn isolated(name: &str) -> bool {
    if std::env::var_os("MADE_UI_MODEL_CHILD").is_some() {
        return false;
    }
    let home = tempfile::tempdir().unwrap();
    let tmp = home.path().join("tmp");
    let runtime = home.path().join("runtime");
    let roaming = home.path().join("AppData").join("Roaming");
    let local = home.path().join("AppData").join("Local");
    #[cfg(unix)]
    let bin = home.path().join("bin");
    for directory in [&tmp, &runtime, &roaming, &local] {
        std::fs::create_dir_all(directory).unwrap();
    }
    #[cfg(unix)]
    std::fs::create_dir(&bin).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", name, "--nocapture"])
        .env("MADE_UI_MODEL_CHILD", "1")
        .env(ISOLATED_TEST_HOME_ENV, home.path())
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env("TMPDIR", &tmp)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("APPDATA", &roaming)
        .env("LOCALAPPDATA", &local);
    command.env_remove(made_bridge::config::CONFIG_CONTENT_ENV);
    command.env_remove(made_platform::SHELL_ENV);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let executable = bin.join("opencode");
        std::fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::var_os("PATH").unwrap_or_default();
        // A host `claude` must not leak into detection: drop host PATH entries
        // that hold one. Tests that need claude put a fake in the isolated bin.
        let host = std::env::split_paths(&path).filter(|dir| !dir.join("claude").exists());
        let paths = std::iter::once(bin).chain(host);
        command.env("PATH", std::env::join_paths(paths).unwrap());
    }
    if std::path::Path::new("/bin/bash").exists() {
        command.env("SHELL", "/bin/bash");
    }
    // Hand the child an explicit fixture plugin package staged inside the
    // install root, which release builds require the plugin override to
    // canonicalize into; cargo never packages a plugin beside the test
    // binary.
    let plugins = support::StagedPlugins::stage("ui-model");
    command.env(
        made_platform::PLUGIN_DIR_ENV,
        plugins.package("opencode-made"),
    );
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

/// Refuse to touch the SSH registry unless app-data resolves under the temp
/// root handed to an isolated-child run. `add_ssh_metadata` persists
/// immediately, so a direct, unisolated caller would otherwise write the
/// developer's real registry.
fn assert_ssh_fixture_is_isolated(context: &str) {
    use made_platform::Platform;
    let test_root = std::env::var_os(ISOLATED_TEST_HOME_ENV).unwrap_or_else(|| {
        panic!("{context} requires an isolated temporary HOME (run through isolated())")
    });
    let app_data = made_platform::host_platform()
        .app_paths()
        .expect("test app paths")
        .data_dir;
    assert!(
        app_data.starts_with(&test_root),
        "{context} would write the host app-data registry at {}",
        app_data.display()
    );
}

fn registry_file_exists(root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name() == Some(std::ffi::OsStr::new(made_app::ssh_servers::REGISTRY_FILE)) {
            return true;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) && registry_file_exists(&path) {
            return true;
        }
    }
    false
}

fn canary_environment(home: &Path) -> std::process::Command {
    let tmp = home.join("tmp");
    let runtime = home.join("runtime");
    let roaming = home.join("AppData").join("Roaming");
    let local = home.join("AppData").join("Local");
    for directory in [&tmp, &runtime, &roaming, &local] {
        std::fs::create_dir_all(directory).unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("TMPDIR", &tmp)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("APPDATA", &roaming)
        .env("LOCALAPPDATA", &local);
    command.env_remove(ISOLATED_TEST_HOME_ENV);
    command.env_remove("MADE_UI_MODEL_CHILD");
    command.env_remove(made_bridge::config::CONFIG_CONTENT_ENV);
    command.env_remove(made_platform::SHELL_ENV);
    command
}

/// The guard must reject an unisolated caller before it builds an AppModel or
/// persists the private SSH registry.
#[test]
fn unisolated_ui_model_helper_guard_child() {
    if std::env::var_os("MADE_UI_MODEL_CANARY_CHILD").is_none() {
        return;
    }
    let workspace = tempfile::tempdir().unwrap();
    let remote = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = local_and_remote_headless(workspace.path(), "unisolated-helper");
    }));
    assert!(
        remote.is_err(),
        "local_and_remote_headless must reject an unisolated app-data root"
    );

    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let inventory = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = add_ssh_inventory(&mut model, 1);
    }));
    assert!(
        inventory.is_err(),
        "add_ssh_inventory must reject an unisolated app-data root"
    );
    model.shutdown();

    let home = std::env::var_os("HOME").expect("canary HOME");
    assert!(
        !registry_file_exists(Path::new(&home)),
        "an unisolated caller wrote the private SSH registry"
    );
}

/// Non-destructive canary: run one helper in a child HOME with no isolation
/// marker. The helper must panic before writing anything, leaving the canary
/// registry absent.
#[test]
fn unisolated_ui_model_helpers_fail_before_writing_the_registry() {
    if std::env::var_os("MADE_UI_MODEL_CHILD").is_some()
        || std::env::var_os("MADE_UI_MODEL_CANARY_CHILD").is_some()
    {
        return;
    }
    let canary = tempfile::tempdir().unwrap();
    let output = canary_environment(canary.path())
        .args([
            "--exact",
            "unisolated_ui_model_helper_guard_child",
            "--nocapture",
        ])
        .env("MADE_UI_MODEL_CANARY_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !registry_file_exists(canary.path()),
        "the unisolated helper wrote {} under the canary root",
        made_app::ssh_servers::REGISTRY_FILE
    );
}

fn open_workspace(model: &mut AppModel, path: &Path) -> WorkspaceId {
    model.open_plan(LaunchPlan::open(path, WorkspaceSource::Picker).unwrap());
    model.ensure_workbench();
    model.active_workspace_id().expect("active workspace")
}

fn isolated_log_path() -> PathBuf {
    use made_platform::Platform;
    made_platform::host_platform()
        .app_paths()
        .expect("isolated app paths")
        .log_dir
        .join("integration.log")
}

fn wait_explorer_ready(headless: &mut Headless, id: WorkspaceId) {
    headless.pump_until(|model| {
        model
            .workspace(id)
            .and_then(Workbench::explorer)
            .is_some_and(made_app::ExplorerPane::is_root_ready)
    });
}

/// Open About from the header wordmark and reveal its diagnostics detail.
fn open_about_diagnostics(headless: &mut Headless, id: WorkspaceId) {
    headless.click(made_app::theme::wordmark_id());
    let pane_id = headless.model.workspace(id).unwrap().pane_id().to_owned();
    headless.click(about_diagnostics_toggle_id(&pane_id));
    settle_about_animation(headless);
}

/// Advance the About reveal animation to its target.
fn settle_about_animation(headless: &mut Headless) {
    let mut time = headless.last_time;
    for _ in 0..24 {
        time += 1.0 / 60.0;
        headless.render_events_at(Vec::new(), time);
    }
}

fn fixture_workspace(temp: &Path, name: &str, file: &str) -> PathBuf {
    let path = temp.join(name);
    fs::create_dir(&path).unwrap();
    fs::write(path.join(file), "fixture contents\n").unwrap();
    path.canonicalize().unwrap()
}

fn authority_hex(id: made_remote::protocol::AuthorityId) -> String {
    id.0.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn text_in_rect(output: &egui::FullOutput, rect: egui::Rect) -> String {
    fn visit(shape: &egui::Shape, rect: egui::Rect, text: &mut String) {
        match shape {
            egui::Shape::Text(run) => {
                let line = egui::Rect::from_min_size(run.pos, run.galley.size());
                if rect.intersects(line) {
                    text.push_str(&run.galley.job.text);
                    text.push('\n');
                }
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, rect, text);
                }
            }
            _ => {}
        }
    }
    let mut text = String::new();
    for clipped in &output.shapes {
        visit(&clipped.shape, rect, &mut text);
    }
    text
}

fn accesskit_row_state(
    output: &egui::FullOutput,
    id: egui::Id,
) -> (egui::Role, Option<String>, Option<egui::accesskit::Toggled>) {
    let update = output
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("accesskit update was not produced");
    let (_, node) = update
        .nodes
        .iter()
        .find(|(node_id, _)| *node_id == id.accesskit_id())
        .unwrap_or_else(|| panic!("no accessible node for {id:?}"));
    (node.role(), node.label().map(str::to_owned), node.toggled())
}

fn add_ssh_inventory(
    model: &mut AppModel,
    count: usize,
) -> Vec<made_remote::protocol::AuthorityId> {
    use made_app::ssh_servers::ServerMetadata;
    assert_ssh_fixture_is_isolated("add_ssh_inventory");
    (0..count)
        .map(|index| {
            model
                .add_ssh_metadata(ServerMetadata {
                    display_name: format!("Plant {index:03}"),
                    host: format!("plant-{index:03}.example"),
                    user: None,
                    port: None,
                    key_file: None,
                    initial_directory: None,
                })
                .expect("server metadata saves")
        })
        .collect()
}

fn wheel_over(headless: &mut Headless, position: egui::Pos2, delta: egui::Vec2) {
    headless.render_events(vec![egui::Event::PointerMoved(position)]);
    headless.render_events(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: egui::Modifiers::NONE,
    }]);
    headless.advance(0.6);
}

#[test]
fn quiet_workspace_chrome_has_no_default_technical_status() {
    if isolated("quiet_workspace_chrome_has_no_default_technical_status") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "alpha", "notes.txt");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    let output = headless.render();
    let text = output_text(&output);
    for expected in ["MADE", "EXPLORER", "notes.txt"] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
    for forbidden in [
        "No active session",
        "No open sessions",
        "waiting",
        "degraded",
        "running",
        "Integration",
        "Sessions",
        "PREVIEW",
        "Select a file",
        "TERMINAL",
        "Open folder",
        "Explorer",
        "Diagnostics",
        "Wider",
        "Narrower",
        "Reload",
        "Hide",
        "Explorer actions",
    ] {
        assert!(
            !text.contains(forbidden),
            "routine chrome leaked {forbidden:?} in {text}"
        );
    }
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
    for icon in [
        top_action_id("open-folder"),
        top_workspace_action_id(id, "explorer"),
        made_app::theme::wordmark_id(),
    ] {
        let response = headless.response(icon);
        assert!(
            screen.contains_rect(response.rect),
            "header icon clipped: {icon:?} {:?}",
            response.rect
        );
        assert!(
            response.rect.width() >= 24.0 && response.rect.height() >= 24.0,
            "header icon target below 24px: {icon:?} {:?}",
            response.rect
        );
    }
    assert!(
        headless
            .ctx
            .read_response(top_workspace_action_id(id, "sessions"))
            .is_none(),
        "optional Sessions icon must stay absent without session data"
    );
    assert!(
        headless
            .ctx
            .read_response(top_action_id("diagnostics"))
            .is_none(),
        "the header must not render a Diagnostics icon"
    );
    let workbench = headless.model.workbench().unwrap();
    assert_eq!(workbench.left_surface(), LeftSurface::Terminal);
    assert!(!workbench.sessions_visible());
    assert!(workbench.explorer_visible());
    assert!(workbench.explorer().unwrap().preview_path().is_none());
    assert_eq!(workbench.explorer().unwrap().root(), Some(root.as_path()));

    let active_tab = headless.response(workspace_tab_id(id)).rect;
    assert!(
        rect_shapes(&output)
            .iter()
            .any(|shape| shape.rect == active_tab
                && shape.fill == made_app::theme::BG_TAB_SELECTED
                && shape.stroke == made_app::theme::workspace_tab_stroke()),
        "active workspace tab must use the opaque selected fill and the shared inside stroke"
    );
    assert!(active_tab.width() > 40.0);
    assert!(
        text_colors(&output, "alpha", 55.0).contains(&made_app::theme::TEXT_PRIMARY),
        "selected workspace tab label must use TEXT_PRIMARY"
    );
    assert!(
        !output.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Circle(circle) => {
                circle.center.y < 70.0
                    && (circle.fill == made_app::theme::ACCENT
                        || circle.fill == made_app::theme::TEXT_TERTIARY)
                    && circle.radius <= 4.0
            }
            _ => false,
        }),
        "workspace tabs must not paint dot markers"
    );
    assert!(
        !rects_with_fill(&output, made_app::theme::ACCENT)
            .iter()
            .any(|rect| rect.top() < 70.0 && rect.height() <= 3.0 && rect.width() > 20.0),
        "workspace tabs must not paint an accent underline"
    );
    let header_bottom = made_app::theme::Layout::for_window(1280.0, 800.0).gutter
        + made_app::theme::Layout::for_window(1280.0, 800.0).top_height;
    assert!(
        !rect_shapes(&output).iter().any(|rect| {
            rect.rect.top() < header_bottom
                && rect.fill != egui::Color32::TRANSPARENT
                && rect.rect.width() > 500.0
        }),
        "floating header must not paint a persistent band fill"
    );
    assert!(
        !rect_shapes(&output).iter().any(|rect| {
            rect.rect.top() < header_bottom
                && (rect.fill == made_app::theme::GLASS_HIGHLIGHT
                    || rect.fill == made_app::theme::GLASS_SHADOW)
        }),
        "floating header must not paint glass highlight or shadow"
    );
    assert!(
        !rect_shapes(&output).iter().any(|rect| {
            rect.rect.top() < header_bottom
                && rect.fill == made_app::theme::BORDER
                && rect.rect.width() <= 1.5
                && rect.rect.height() >= 12.0
        }),
        "wordmark must not paint a divider in the header"
    );
    assert!(
        !rect_shapes(&output).iter().any(|rect| {
            rect.rect.top() < header_bottom
                && rect.stroke.color == made_app::theme::BORDER
                && rect.rect.width() > 500.0
        }),
        "floating header must not paint a band border"
    );

    let explorer_action = headless
        .response(top_workspace_action_id(id, "explorer"))
        .rect;
    assert!(
        rects_with_fill(&output, made_app::theme::BG_SELECTED)
            .iter()
            .any(|rect| rect.contains_rect(explorer_action)),
        "selected Explorer toggle must use the selected background"
    );
    assert!(
        !rect_shapes(&output).iter().any(|rect| {
            rect.rect.expand(1.0).contains_rect(explorer_action)
                && rect.stroke.color == made_app::theme::BORDER_CONTROL
        }),
        "selected Explorer toggle must not paint a border"
    );
    assert!(
        !rects_with_fill(&output, made_app::theme::BG_HEADER_GLASS).is_empty(),
        "explorer header must use the glass header tone"
    );
    let explorer_card =
        largest_rect(&output, made_app::theme::BG_CARD_GLASS).expect("explorer card must paint");
    assert!(
        (explorer_card.height() - 748.0).abs() < 2.0,
        "explorer must fill the main content height: {explorer_card:?}"
    );

    let wordmark = output_text(&headless.hover_until_text(made_app::theme::wordmark_id(), "About"));
    assert!(
        wordmark.contains("About"),
        "wordmark About tooltip missing: {wordmark}"
    );
}

#[test]
fn workspace_tabs_use_non_color_selection_and_reduced_inactive_opacity() {
    if isolated("workspace_tabs_use_non_color_selection_and_reduced_inactive_opacity") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let alpha = fixture_workspace(temp.path(), "alpha", "a.txt");
    let beta = fixture_workspace(temp.path(), "beta", "b.txt");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let alpha_id = open_workspace(&mut model, &alpha);
    let beta_id = open_workspace(&mut model, &beta);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, alpha_id);
    wait_explorer_ready(&mut headless, beta_id);
    let output = headless.render();

    let active_tab = headless.response(workspace_tab_id(beta_id)).rect;
    let selected_shape = rect_shapes(&output)
        .into_iter()
        .find(|shape| shape.rect == active_tab && shape.fill == made_app::theme::BG_TAB_SELECTED)
        .expect("selected workspace tab must use the opaque selected fill");
    assert_eq!(
        selected_shape.stroke,
        made_app::theme::workspace_tab_stroke(),
        "selected workspace tab must paint the shared discreet inside stroke"
    );
    assert_ne!(selected_shape.stroke.color, made_app::theme::ACCENT);
    assert_ne!(selected_shape.stroke.color, made_app::theme::FOCUS);
    assert!(
        !rects_with_fill(&output, made_app::theme::BG_SELECTED)
            .iter()
            .any(|rect| rect.contains_rect(active_tab)),
        "selected workspace tab must not use the row BG_SELECTED fill"
    );
    assert!(
        !rects_with_fill(&output, made_app::theme::BG_CARD_GLASS)
            .iter()
            .any(|rect| rect.intersects(active_tab)),
        "selected workspace tab must not use the Explorer glass fill"
    );
    assert!(
        rect_shapes(&output).iter().any(|shape| {
            shape.rect == active_tab && shape.stroke == made_app::theme::workspace_tab_stroke()
        }),
        "selected workspace tab must paint the shared inside stroke"
    );
    assert!(
        text_colors(&output, "beta", 55.0).contains(&made_app::theme::TEXT_PRIMARY),
        "selected workspace tab label must be primary"
    );
    assert!(
        text_font_sizes(&output, "beta", 55.0).contains(&made_app::theme::TAB_SELECTED_FONT_SIZE),
        "selected workspace tab must use the 13px non-color cue"
    );

    let inactive_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    let idle_shape = rect_shapes(&output)
        .into_iter()
        .find(|shape| shape.rect == inactive_tab && shape.fill == made_app::theme::BG_TAB_IDLE)
        .expect("inactive workspace tab must use the translucent black idle fill");
    assert_eq!(
        idle_shape.stroke,
        egui::Stroke::NONE,
        "inactive workspace tab must paint no stroke at rest"
    );
    assert!(
        text_colors(&output, "alpha", 55.0).contains(&made_app::theme::inactive_tab_text()),
        "inactive workspace tab label must use the reduced-opacity token"
    );
    assert!(
        text_font_sizes(&output, "alpha", 55.0).contains(&made_app::theme::TAB_FONT_SIZE),
        "inactive workspace tab must keep the 12.5px label"
    );
    assert!(
        !rects_with_fill(&output, made_app::theme::BG_SELECTED)
            .iter()
            .any(|rect| rect.contains_rect(inactive_tab)),
        "inactive workspace tab must not use the selected fill"
    );

    let hovered = headless.hover(workspace_tab_id(alpha_id));
    assert!(
        !rects_with_fill(&hovered, made_app::theme::BG_HOVER)
            .iter()
            .any(|rect| rect.intersects(inactive_tab)),
        "hovered workspace tab must keep its idle fill instead of BG_HOVER"
    );
    let hover_shape = rect_shapes(&hovered)
        .into_iter()
        .find(|shape| shape.rect == inactive_tab && shape.fill == made_app::theme::BG_TAB_HOVER)
        .expect("hovered workspace tab must use the opaque hover fill");
    assert_eq!(
        hover_shape.stroke,
        egui::Stroke::NONE,
        "hovered inactive workspace tab must paint no stroke"
    );
    let hovered_colors = text_colors(&hovered, "alpha", 55.0);
    assert!(
        hovered_colors.contains(&made_app::theme::TEXT_PRIMARY),
        "hover must restore the workspace-tab label to full opacity"
    );
    assert!(
        !hovered_colors.contains(&made_app::theme::inactive_tab_text()),
        "hover must not keep the reduced-opacity label token"
    );
    assert!(
        !rect_shapes(&hovered).iter().any(|shape| {
            shape.rect == inactive_tab
                && (shape.stroke.color == made_app::theme::ACCENT
                    || shape.stroke.color == made_app::theme::FOCUS)
        }),
        "hovered workspace tab must not paint an accent or focus stroke"
    );

    let selected_hover = headless.hover(workspace_tab_id(beta_id));
    let selected_hover_shape = rect_shapes(&selected_hover)
        .into_iter()
        .find(|shape| shape.rect == active_tab && shape.fill == made_app::theme::BG_TAB_SELECTED)
        .expect("hovered selected workspace tab must keep the selected fill");
    assert_eq!(
        selected_hover_shape.stroke,
        made_app::theme::workspace_tab_stroke(),
        "hovered selected workspace tab must keep the faint inside stroke"
    );
}

#[test]
fn workspace_tab_close_hover_lights_parent_and_reddens_icon() {
    if isolated("workspace_tab_close_hover_lights_parent_and_reddens_icon") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let alpha = fixture_workspace(temp.path(), "alpha", "a.txt");
    let beta = fixture_workspace(temp.path(), "beta", "b.txt");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let alpha_id = open_workspace(&mut model, &alpha);
    let beta_id = open_workspace(&mut model, &beta);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, alpha_id);
    wait_explorer_ready(&mut headless, beta_id);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    headless.render();

    let inactive_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    let full_path = fs::canonicalize(&alpha).unwrap().display().to_string();
    headless.hover_until_text(workspace_tab_close_id(alpha_id), "Close workspace");
    headless.advance(0.2);
    let output = headless.render();
    assert!(
        output_text(&output).contains("Close workspace"),
        "close affordance must keep its tooltip"
    );
    assert!(
        !output_text(&output).contains(&full_path),
        "close hover must suppress the parent full-path tooltip"
    );
    assert!(
        text_colors(&output, "×", 55.0).contains(&made_app::theme::ERROR),
        "close icon hover must use the soft ERROR color"
    );
    assert!(
        !rects_with_fill(&output, made_app::theme::BG_HOVER)
            .iter()
            .any(|rect| rect.intersects(inactive_tab)),
        "close-icon hover must not paint a hover fill"
    );
    let parent = rect_shapes(&output)
        .into_iter()
        .find(|shape| shape.rect == inactive_tab && shape.fill == made_app::theme::BG_TAB_HOVER)
        .expect("close hover must apply the parent tab hover fill");
    assert_eq!(
        parent.stroke,
        egui::Stroke::NONE,
        "close hover on an inactive tab must not paint a stroke"
    );
    assert!(
        text_colors(&output, "alpha", 55.0).contains(&made_app::theme::TEXT_PRIMARY),
        "close-icon hover must also light the parent tab label"
    );

    // Hovering the close X on the already-selected tab must keep the selected
    // fill, not switch to the hover fill or paint any border/X background.
    let active_tab = headless.response(workspace_tab_id(beta_id)).rect;
    let active_path = fs::canonicalize(&beta).unwrap().display().to_string();
    let active_close = headless.response(workspace_tab_close_id(beta_id)).rect;
    headless.hover_until_text(workspace_tab_close_id(beta_id), "Close workspace");
    headless.advance(0.2);
    let active_output = headless.render();
    let active_text = output_text(&active_output);
    assert!(
        active_text.contains("Close workspace"),
        "selected-tab close hover must keep the close tooltip"
    );
    assert!(
        !active_text.lines().any(|line| line.trim() == active_path),
        "selected-tab close hover must suppress the parent path tooltip"
    );
    assert!(
        text_colors(&active_output, "×", 55.0).contains(&made_app::theme::ERROR),
        "selected-tab close hover must use the red ERROR glyph"
    );
    assert!(
        !rects_with_fill(&active_output, made_app::theme::BG_HOVER)
            .iter()
            .any(|rect| rect.intersects(active_tab) || rect.intersects(active_close)),
        "selected-tab close hover must not paint an X background"
    );
    let active_parent = rect_shapes(&active_output)
        .into_iter()
        .find(|shape| shape.rect == active_tab && shape.fill == made_app::theme::BG_TAB_SELECTED)
        .expect("selected-tab close hover must keep BG_TAB_SELECTED");
    assert_eq!(
        active_parent.stroke,
        made_app::theme::workspace_tab_stroke(),
        "selected-tab close hover must keep the faint inside stroke"
    );
    assert!(
        !rect_shapes(&active_output).iter().any(|shape| {
            shape.rect == active_tab && shape.fill == made_app::theme::BG_TAB_HOVER
        }),
        "selected-tab close hover must not switch to BG_TAB_HOVER"
    );

    assert_eq!(
        text_rects(&active_output, "×", 55.0).len(),
        1,
        "selected-tab close hover must show one close glyph"
    );
    assert!(
        text_font_sizes(&active_output, "×", 55.0)
            .iter()
            .all(|size| (size - made_app::theme::CLOSE_GLYPH_SIZE).abs() < 0.1),
        "selected-tab close hover must keep the fixed close glyph size"
    );
    assert!(
        text_colors(&active_output, "×", 55.0).contains(&made_app::theme::ERROR),
        "selected-tab close hover must keep the red glyph"
    );
}

#[test]
fn workspace_tab_opencode_activity_updates_live_for_background_workspaces() {
    if isolated("workspace_tab_opencode_activity_updates_live_for_background_workspaces") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let alpha = fixture_workspace(
        temp.path(),
        "opencode-activity-alpha-with-a-long-name",
        "a.txt",
    );
    let beta = fixture_workspace(temp.path(), "activity-beta", "b.txt");
    let alpha_path = alpha.display().to_string();
    let beta_path = beta.display().to_string();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let alpha_id = open_workspace(&mut model, &alpha);
    let beta_id = open_workspace(&mut model, &beta);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, alpha_id);
    wait_explorer_ready(&mut headless, beta_id);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    assert_eq!(
        headless.model.active_workspace_id(),
        Some(beta_id),
        "the OpenCode workspace must stay in the background"
    );
    let initial_output = headless.render();
    let initial_alpha_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    let initial_beta_tab = headless.response(workspace_tab_id(beta_id)).rect;
    for tab in [initial_alpha_tab, initial_beta_tab] {
        assert!(
            tab_session_dots(&initial_output, tab).is_empty(),
            "an idle session map must paint no status dot: {tab:?}"
        );
    }
    assert!(
        text_rects(
            &initial_output,
            "opencode-activity-alpha-with-a-long-name",
            55.0,
        )
        .iter()
        .any(|rect| initial_alpha_tab.contains_rect(*rect)),
        "a closed OpenCode tab must keep its folder label"
    );

    let options =
        bridge_options(headless.model.workspace(alpha_id).unwrap()).expect("bridge options");
    let mut socket = connect(&options);
    headless.pump_until(|model| model.workspace(alpha_id).unwrap().is_connected());
    headless.render();
    let connected_idle_output = headless.render();
    let alpha_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    assert!(
        tab_session_dots(&connected_idle_output, alpha_tab).is_empty(),
        "a connected OpenCode tab with no sessions must paint no status dot"
    );
    assert!(
        text_rects(&connected_idle_output, "OpenCode", 55.0)
            .iter()
            .any(|rect| alpha_tab.contains_rect(*rect)),
        "a connected OpenCode tab must be labeled OpenCode"
    );
    headless.hover(workspace_tab_id(alpha_id));
    headless.advance(0.2);
    let no_session_hover = headless.render();
    let no_session_text = output_text(&no_session_hover);
    assert!(
        !no_session_text.contains(&alpha_path) && !no_session_text.contains("Sessions:"),
        "an OpenCode tab with no sessions must show no parent tooltip: {no_session_text}"
    );
    headless.render_events(vec![egui::Event::PointerMoved(egui::pos2(
        4.0,
        headless.height - 4.0,
    ))]);
    headless.advance(0.2);

    send_sessions_state(
        &mut socket,
        1,
        &alpha,
        &[
            ("busy-session", true, false),
            ("idle-session", false, false),
        ],
    );
    headless.pump_until(|model| {
        model
            .workspace(alpha_id)
            .unwrap()
            .session_map()
            .rows()
            .len()
            == 2
    });

    // The extra pass also lets the lazily loaded SVG reach the texture cache.
    headless.render();
    let busy_output = headless.render_events_at(Vec::new(), headless.last_time + 0.5);
    let alpha_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    let beta_tab = headless.response(workspace_tab_id(beta_id)).rect;
    let busy_dots = tab_session_dots(&busy_output, alpha_tab);
    assert_eq!(
        busy_dots.len(),
        2,
        "busy and idle sessions must each paint one dot"
    );
    assert!(
        tab_session_dots(&busy_output, beta_tab).is_empty(),
        "a plain shell workspace must stay free of OpenCode session dots"
    );

    let opencode_icon = icon_named("opencode.svg");
    let icon_image = painted_images(&busy_output)
        .into_iter()
        .find(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && alpha_tab.contains_rect(image.rect)
        })
        .expect("a connected workspace must paint opencode.svg in its tab");
    assert_eq!(
        icon_image.texture,
        icon_texture(&headless.ctx, opencode_icon),
        "the tab must reuse the vendored opencode.svg bytes"
    );
    assert_eq!(
        icon_image.tint,
        egui::Color32::WHITE,
        "the tab icon keeps the shipped SVG colors"
    );
    let alpha_label = text_rects(&busy_output, "OpenCode", 55.0)
        .into_iter()
        .find(|rect| alpha_tab.contains_rect(*rect))
        .expect("the OpenCode label must be painted");
    assert!(
        icon_image.rect.right() < alpha_label.left(),
        "the OpenCode icon must precede the label: {:?} vs {:?}",
        icon_image.rect,
        alpha_label
    );
    assert!(
        (alpha_label.left() - (alpha_tab.left() + 10.0 + 22.0)).abs() < 1.0,
        "the label follows the 16px icon and its 6px gap: {:?} in {alpha_tab:?}",
        alpha_label
    );
    let busy_dot = &busy_dots[0];
    let idle_dot = &busy_dots[1];
    assert!(
        (busy_dot.radius - 2.0).abs() < 0.1 && (idle_dot.radius - 2.0).abs() < 0.1,
        "session dots must keep the 4px diameter: {busy_dot:?} {idle_dot:?}"
    );
    assert!(
        (busy_dot.center - session_dot_center(alpha_tab, 0, 2)).length() < 0.75
            && (idle_dot.center - session_dot_center(alpha_tab, 1, 2)).length() < 0.75,
        "dots must be right-aligned in row order: {busy_dot:?} {idle_dot:?}"
    );
    assert_session_dot_color(busy_dot, made_app::theme::ACCENT, "the busy dot");
    assert_session_dot_color(idle_dot, made_app::theme::TEXT_TERTIARY, "the idle dot");
    let busy_opacity = session_dot_alpha(busy_dot) as f32 / 255.0;
    assert!(
        (0.34..=1.0).contains(&busy_opacity),
        "the busy dot opacity must stay between the pulse bounds: {busy_opacity}"
    );
    assert_eq!(
        session_dot_alpha(idle_dot),
        255,
        "the idle dot must be static"
    );
    let last_inset = alpha_tab.right() - (idle_dot.center.x + idle_dot.radius);
    assert!(
        (last_inset - 8.0).abs() < 0.75,
        "the last dot must keep the documented right inset: {last_inset}"
    );
    assert!(
        busy_dot.center.x - busy_dot.radius >= alpha_label.right() - 0.75,
        "the dot row must not overlap the label: dot {:?}, label {alpha_label:?}",
        busy_dot.center
    );
    assert!(
        !text_rects(&busy_output, "×", 55.0)
            .iter()
            .any(|rect| alpha_tab.contains_rect(*rect)),
        "the close glyph must stay hidden at rest while session dots are visible"
    );

    let period = 1.8_f64;
    let next_period = ((headless.last_time / period).floor() + 1.0) * period;
    let bright_output = headless.render_events_at(Vec::new(), next_period + period * 0.25);
    let bright_dots = tab_session_dots(&bright_output, alpha_tab);
    let bright_alpha = session_dot_alpha(&bright_dots[0]);
    let dim_output = headless.render_events_at(Vec::new(), next_period + period * 0.75);
    let dim_dots = tab_session_dots(&dim_output, alpha_tab);
    let dim_alpha = session_dot_alpha(&dim_dots[0]);
    assert!(
        bright_alpha > dim_alpha,
        "the busy session dot must pulse in opacity: bright {bright_alpha}, dim {dim_alpha}"
    );
    assert!(
        (bright_alpha as i32 - 255).abs() <= 1,
        "the busy dot must reach full opacity: {bright_alpha}"
    );
    let dim_opacity = dim_alpha as f32 / 255.0;
    assert!(
        (dim_opacity - 0.35).abs() < 0.02,
        "the busy dot must dim to about 0.35 opacity: {dim_opacity}"
    );
    assert_eq!(
        session_dot_alpha(&bright_dots[1]),
        session_dot_alpha(&dim_dots[1]),
        "non-busy session dots must not pulse"
    );

    let close_target = headless.response(workspace_tab_close_id(alpha_id)).rect;
    assert!(
        close_target.width() >= 24.0 && close_target.height() >= 24.0,
        "the close target must stay at least 24x24: {close_target:?}"
    );
    assert!(
        (close_target.center().x - (alpha_tab.right() - 13.0)).abs() < 0.75
            && (close_target.center().y - alpha_tab.center().y).abs() < 0.75,
        "the close target must stay centered at right - 13: target {close_target:?}, tab {alpha_tab:?}"
    );
    assert!(
        (alpha_tab.width() - 135.6).abs() < 0.75,
        "two dots on the OpenCode label must keep the base width: {alpha_tab:?}"
    );
    assert!(
        (initial_alpha_tab.width() - 238.0).abs() < 0.75,
        "the disconnected folder tab must keep its long-label clamp: {initial_alpha_tab:?}"
    );
    // Usable label area ends 26px (close slot) + 14px (right-edge allowance) from the right.
    let label_area = alpha_tab.right() - 40.0 - alpha_label.left();
    assert!(
        (label_area - 63.6).abs() < 0.75,
        "the OpenCode label must keep its base text area: {label_area} (tab {alpha_tab:?}, label {alpha_label:?})"
    );
    assert!(
        !painted_images(&busy_output).into_iter().any(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && beta_tab.contains_rect(image.rect)
        }),
        "a plain shell workspace must not paint the OpenCode icon"
    );
    let beta_label = text_rects(&busy_output, "activity-beta", 55.0)
        .into_iter()
        .find(|rect| beta_tab.contains_rect(*rect))
        .expect("the shell tab label");
    assert!(
        (beta_label.left() - (beta_tab.left() + 10.0 + 22.0)).abs() < 1.0,
        "the shell label must start after the leading icon slot: {beta_label:?} in {beta_tab:?}"
    );
    let beta_expected_width = "activity-beta".chars().count() as f32 * 6.2 + 86.0;
    assert!(
        (beta_tab.width() - beta_expected_width).abs() < 0.75,
        "the shell tab width must not change for its icon: {beta_tab:?}"
    );
    let terminal_window = rect_shapes(&busy_output)
        .into_iter()
        .find(|shape| {
            (shape.rect.width() - 14.0).abs() < 0.75
                && (shape.rect.height() - 11.0).abs() < 0.75
                && (shape.stroke.width - 1.5).abs() < 0.05
                && shape.stroke.color == made_app::theme::TEXT_PRIMARY
                && beta_tab.contains_rect(shape.rect)
                && shape.rect.right() <= beta_label.left() - 2.0
        })
        .map(|shape| shape.rect)
        .expect("shell tab terminal window glyph");
    assert!(
        (terminal_window.center().x - (beta_tab.left() + 10.0 + 8.0)).abs() < 1.0
            && (terminal_window.center().y - beta_tab.center().y).abs() < 1.0,
        "the terminal glyph must be centered in the 16px slot: {terminal_window:?} in {beta_tab:?}"
    );
    let terminal_paths: Vec<_> = path_shapes(&busy_output)
        .into_iter()
        .filter(|path| {
            let bounds = egui::Rect::from_points(&path.points);
            beta_tab.contains_rect(bounds)
                && bounds.right() <= beta_label.left() - 1.0
                && matches!(
                    path.stroke.color,
                    egui::epaint::ColorMode::Solid(color)
                        if color == made_app::theme::TEXT_PRIMARY
                )
        })
        .collect();
    assert_eq!(
        terminal_paths.len(),
        2,
        "the shell tab terminal glyph must keep its chevron and cursor: {terminal_paths:?}"
    );
    assert!(
        terminal_window.contains_rect(egui::Rect::from_points(&terminal_paths[0].points))
            && terminal_window.contains_rect(egui::Rect::from_points(&terminal_paths[1].points)),
        "the shell tab prompt strokes must sit inside its terminal window"
    );

    headless.hover_until_text(workspace_tab_id(alpha_id), "Sessions: 1 running, 1 idle");
    headless.advance(0.2);
    let busy_hover = headless.render();
    let busy_text = output_text(&busy_hover);
    assert!(
        busy_text.contains("Sessions: 1 running, 1 idle"),
        "the busy tab tooltip must summarize every session: {busy_text}"
    );
    assert!(
        !busy_text.contains(&alpha_path),
        "the OpenCode tab tooltip must omit the workspace path: {busy_text}"
    );
    let alpha_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    assert!(
        tab_session_dots(&busy_hover, alpha_tab).is_empty(),
        "hover must hide all session dots"
    );
    assert!(
        text_rects(&busy_hover, "×", 55.0)
            .iter()
            .any(|rect| alpha_tab.contains_rect(*rect)),
        "hover must show only the close glyph in place of the dots"
    );

    let beta_hover = headless.hover_until_text(workspace_tab_id(beta_id), &beta_path);
    assert!(
        output_text(&beta_hover).contains(&beta_path),
        "a plain shell tab must keep its full-path tooltip"
    );

    send_sessions_state(
        &mut socket,
        2,
        &alpha,
        &[
            ("attention-session", false, true),
            ("idle-session", false, false),
            ("busy-session", true, false),
        ],
    );
    headless.pump_until(|model| {
        model
            .workspace(alpha_id)
            .unwrap()
            .session_map()
            .rows()
            .len()
            == 3
    });
    headless.render_events(vec![egui::Event::PointerMoved(egui::pos2(
        4.0,
        headless.height - 4.0,
    ))]);
    headless.advance(0.2);
    let attention_output = headless.render_events_at(Vec::new(), headless.last_time + 0.25);
    let alpha_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    let attention_dots = tab_session_dots(&attention_output, alpha_tab);
    assert_eq!(
        attention_dots.len(),
        3,
        "each of attention, idle, and busy must paint one dot"
    );
    for (index, dot) in attention_dots.iter().enumerate() {
        assert!(
            (dot.center - session_dot_center(alpha_tab, index, 3)).length() < 0.75,
            "dot {index} must follow session order: {dot:?}"
        );
        assert!(
            (dot.radius - 2.0).abs() < 0.1,
            "dot {index} must keep the 4px diameter"
        );
    }
    assert_session_dot_color(
        &attention_dots[0],
        made_app::theme::WARNING,
        "the attention dot",
    );
    assert_session_dot_color(
        &attention_dots[1],
        made_app::theme::TEXT_TERTIARY,
        "the idle dot",
    );
    assert_session_dot_color(&attention_dots[2], made_app::theme::ACCENT, "the busy dot");
    assert!(
        (0.34..=1.0).contains(&(session_dot_alpha(&attention_dots[2]) as f32 / 255.0)),
        "attention must not suppress the busy dot"
    );
    assert!(
        !text_rects(&attention_output, "×", 55.0)
            .iter()
            .any(|rect| alpha_tab.contains_rect(*rect)),
        "the close glyph must stay hidden at rest while session dots are visible"
    );

    // Crossfade mid-frame: both the dots and the X are partially painted.
    headless.render_events_at(
        vec![egui::Event::PointerMoved(alpha_tab.center())],
        headless.last_time + 1.0 / 60.0,
    );
    let mid_transition = headless.render_events_at(Vec::new(), headless.last_time + 0.05);
    let mid_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    let mid_dots = tab_session_dots(&mid_transition, mid_tab);
    assert_eq!(
        mid_dots.len(),
        3,
        "mid-transition must keep the session dots painted"
    );
    assert!(
        mid_dots.iter().all(|dot| {
            let alpha = session_dot_alpha(dot);
            alpha > 0 && alpha < 255
        }),
        "mid-transition dots must be partially faded: {mid_dots:?}"
    );
    let mid_close_alpha = text_colors(&mid_transition, "×", 55.0)
        .into_iter()
        .find(|color| color.a() > 0)
        .expect("mid-transition close glyph");
    let full_close_alpha = made_app::theme::inactive_icon_color().a();
    assert!(
        mid_close_alpha.a() < full_close_alpha,
        "mid-transition X must be partially faded: {} vs {full_close_alpha}",
        mid_close_alpha.a()
    );

    headless.advance(0.2);
    let settled = headless.render();
    let settled_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    assert!(
        tab_session_dots(&settled, settled_tab).is_empty(),
        "after the transition only the close glyph may remain"
    );
    assert!(
        text_colors(&settled, "×", 55.0).contains(&made_app::theme::inactive_icon_color()),
        "the settled close glyph must paint at full opacity"
    );

    headless.render_events_at(
        vec![egui::Event::PointerMoved(egui::pos2(
            4.0,
            headless.height - 4.0,
        ))],
        headless.last_time + 1.0 / 60.0,
    );
    headless.advance(0.2);
    let returned = headless.render();
    let returned_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    let returned_dots = tab_session_dots(&returned, returned_tab);
    assert_eq!(
        returned_dots.len(),
        3,
        "leaving the tab must fade every session dot back in"
    );
    assert!(
        returned_dots
            .iter()
            .take(2)
            .all(|dot| session_dot_alpha(dot) == 255),
        "non-busy dots must return to full opacity"
    );
    assert!(
        !text_rects(&returned, "×", 55.0)
            .iter()
            .any(|rect| returned_tab.contains_rect(*rect)),
        "the close glyph must fade out after leaving the tab"
    );

    headless.hover_until_text(
        workspace_tab_id(alpha_id),
        "Sessions: 1 running, 1 needs attention, 1 idle",
    );
    headless.advance(0.2);
    let attention_hover = headless.render();
    let attention_text = output_text(&attention_hover);
    assert!(
        attention_text.contains("Sessions: 1 running, 1 needs attention, 1 idle"),
        "the attention tooltip must summarize every session: {attention_text}"
    );
    assert!(
        !attention_text.contains(&alpha_path),
        "the OpenCode attention tooltip must omit the workspace path: {attention_text}"
    );
    let alpha_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    assert!(
        tab_session_dots(&attention_hover, alpha_tab).is_empty(),
        "hover must hide all session dots for mixed activity"
    );
    assert!(
        text_rects(&attention_hover, "×", 55.0)
            .iter()
            .any(|rect| alpha_tab.contains_rect(*rect)),
        "hover must show only the close glyph for mixed activity"
    );

    let many_sessions = [
        ("many-busy-0", true, false),
        ("many-busy-1", true, false),
        ("many-busy-2", true, false),
        ("many-busy-3", true, false),
        ("many-busy-4", true, false),
        ("many-busy-5", true, false),
        ("many-busy-6", true, false),
        ("many-busy-7", true, false),
        ("many-attention-8", false, true),
    ];
    send_sessions_state(&mut socket, 3, &alpha, &many_sessions);
    headless.pump_until(|model| {
        model
            .workspace(alpha_id)
            .unwrap()
            .session_map()
            .rows()
            .len()
            == 9
    });
    headless.render_events(vec![egui::Event::PointerMoved(egui::pos2(
        4.0,
        headless.height - 4.0,
    ))]);
    headless.advance(0.2);
    let many_output = headless.render_events_at(Vec::new(), headless.last_time + 0.25);
    let alpha_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    let many_dots = tab_session_dots(&many_output, alpha_tab);
    assert_eq!(
        many_dots.len(),
        8,
        "only the first eight session dots may be visible"
    );
    for (index, dot) in many_dots.iter().enumerate() {
        assert!(
            (dot.center - session_dot_center(alpha_tab, index, 8)).length() < 0.75,
            "capped dot {index} must keep session order: {dot:?}"
        );
        assert_session_dot_color(dot, made_app::theme::ACCENT, "a capped busy dot");
    }
    assert!(
        many_dots
            .iter()
            .all(|dot| session_dot_rgb(dot) != session_dot_rgb(&attention_dots[0])),
        "the ninth attention session must stay out of the eight-dot row"
    );
    assert!(
        (alpha_tab.width() - 166.6).abs() < 0.75,
        "eight dots must grow the OpenCode tab to exactly 166.6px: {alpha_tab:?}"
    );
    let many_label = text_rects(&many_output, "OpenCode", 55.0)
        .into_iter()
        .find(|rect| alpha_tab.contains_rect(*rect))
        .expect("the capped-dot tab must keep its label");
    let many_label_area = alpha_tab.right() - 40.0 - 31.0 - many_label.left();
    assert!(
        (many_label_area - label_area).abs() < 0.75,
        "eight dots must grow the tab without shrinking the label: {many_label_area} vs {label_area}"
    );
    let many_hover = {
        headless.hover_until_text(
            workspace_tab_id(alpha_id),
            "Sessions: 8 running, 1 needs attention",
        );
        headless.advance(0.2);
        headless.render()
    };
    let many_text = output_text(&many_hover);
    assert!(
        many_text.contains("Sessions: 8 running, 1 needs attention"),
        "the tooltip summary must count sessions beyond the dot cap: {many_text}"
    );
    assert!(
        !many_text.contains(&alpha_path),
        "the capped OpenCode tooltip must omit the workspace path: {many_text}"
    );
    let alpha_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    assert!(
        tab_session_dots(&many_hover, alpha_tab).is_empty(),
        "hover must hide the capped dot row"
    );
    assert!(
        text_rects(&many_hover, "×", 55.0)
            .iter()
            .any(|rect| alpha_tab.contains_rect(*rect)),
        "hover must show only the close glyph for the capped dot row"
    );

    headless.render_events(vec![egui::Event::PointerMoved(egui::pos2(
        4.0,
        headless.height - 4.0,
    ))]);
    headless.focus(workspace_tab_id(alpha_id));
    headless.advance(0.2);
    let focused = headless.render();
    let alpha_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    assert!(
        tab_session_dots(&focused, alpha_tab).is_empty(),
        "keyboard focus must hide all session dots"
    );
    assert!(
        text_rects(&focused, "×", 55.0)
            .iter()
            .any(|rect| alpha_tab.contains_rect(*rect)),
        "keyboard focus must show only the close glyph in place of the dots"
    );

    send(
        &mut socket,
        json!({"type":"goodbye", "reason":"normal exit"}),
    );
    headless.pump_until(|model| !model.workspace(alpha_id).unwrap().is_connected());
    headless.render();
    let disconnected = headless.render();
    let alpha_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    assert!(
        tab_session_dots(&disconnected, alpha_tab).is_empty(),
        "a disconnected OpenCode tab must paint no session dots"
    );
    assert!(
        !painted_images(&disconnected).into_iter().any(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && alpha_tab.contains_rect(image.rect)
        }),
        "the icon must disappear when the integration disconnects"
    );
    assert!(
        text_rects(
            &disconnected,
            "opencode-activity-alpha-with-a-long-name",
            55.0,
        )
        .iter()
        .any(|rect| alpha_tab.contains_rect(*rect)),
        "the disconnected tab must revert to its folder label"
    );
    assert!(
        (alpha_tab.width() - 238.0).abs() < 0.75,
        "the reverted folder label must restore the long-label width: {alpha_tab:?}"
    );
    headless.hover_until_text(workspace_tab_id(alpha_id), &alpha_path);
    headless.advance(0.2);
    let idle_hover = headless.render();
    let alpha_tab = headless.response(workspace_tab_id(alpha_id)).rect;
    assert!(
        text_rects(&idle_hover, "×", 55.0)
            .iter()
            .any(|rect| alpha_tab.contains_rect(*rect)),
        "the idle close glyph must become visible on hover"
    );
    assert!(
        tab_session_dots(&idle_hover, alpha_tab).is_empty(),
        "an idle tab must keep the no-status close behavior"
    );
    assert!(
        output_text(&idle_hover).contains(&alpha_path),
        "the reverted folder tab must show its full-path tooltip"
    );
}

#[test]
fn explorer_rows_use_unified_metrics_and_open_folder_treatment() {
    if isolated("explorer_rows_use_unified_metrics_and_open_folder_treatment") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("rows");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.render();
    headless.click(
        headless
            .model
            .workspace(id)
            .unwrap()
            .explorer()
            .unwrap()
            .row_widget_id(0),
    );
    headless.pump_until_rows(id, 2);
    headless.render();
    headless.click(
        headless
            .model
            .workspace(id)
            .unwrap()
            .explorer()
            .unwrap()
            .row_widget_id(1),
    );
    let output = headless.render();
    let layout = made_app::theme::Layout::for_window(1280.0, 800.0);
    let explorer_left = 1280.0 - layout.gutter - layout.right_card_width;
    let body_top = layout.gap + layout.top_height + layout.gap + layout.card_header_height;
    let selected = rects_with_fill(&output, made_app::theme::BG_SELECTED)
        .into_iter()
        .find(|rect| rect.left() > explorer_left && rect.top() > body_top)
        .expect("selected explorer row fill");
    assert!((selected.height() - 24.0).abs() < 0.5);
    let selected_shape = rect_shapes(&output)
        .into_iter()
        .find(|shape| shape.fill == made_app::theme::BG_SELECTED && shape.rect == selected)
        .expect("selected explorer row shape");
    assert_eq!(selected_shape.corner_radius, egui::CornerRadius::same(6));
    assert!(
        (selected.left() - (explorer_left + 12.0)).abs() < 1.0,
        "tree row must inset 4px inside the 8px body padding: {selected:?}"
    );
    assert!(
        !rects_with_fill(&output, made_app::theme::ACCENT)
            .iter()
            .any(|rect| rect.left() > explorer_left
                && rect.top() > body_top
                && rect.width() <= 3.0
                && (20.0..=25.0).contains(&rect.height())),
        "selected explorer row must not paint an accent side bar"
    );
    let expanded_colors = text_colors(&output, "src", body_top + 24.0);
    assert!(
        expanded_colors.contains(&made_app::theme::TEXT_PRIMARY),
        "expanded folder label must be brighter: {expanded_colors:?}"
    );

    let folder_row = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .row_widget_id(0);
    let file_row = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .row_widget_id(1);
    let folder_rect = headless.response(folder_row).rect;
    let file_rect = headless.response(file_row).rect;
    assert!(
        !rect_shapes(&output).iter().any(|shape| {
            (shape.fill == made_app::theme::BG_SELECTED || shape.fill == made_app::theme::BG_HOVER)
                && shape.rect.intersect(folder_rect).height() > 0.5
        }),
        "an expanded but unselected folder must paint no row fill"
    );

    let folder_text = text_rects(&output, "src", body_top + 24.0)
        .into_iter()
        .find(|rect| rect.top() > body_top)
        .expect("expanded folder label");
    assert!(
        (folder_text.left() - (explorer_left + 36.0)).abs() < 1.0,
        "depth-0 text must start 28px from the row edge: {folder_text:?}"
    );
    assert!(
        (folder_text.center().y - folder_rect.center().y).abs() < 0.75,
        "folder label must be vertically centered in the row: {folder_text:?} vs {folder_rect:?}"
    );
    let file_text = text_rects(&output, "main.rs", body_top + 48.0)
        .into_iter()
        .find(|rect| rect.top() > body_top)
        .expect("child file label");
    assert!(
        (file_text.left() - (folder_text.left() + 14.0)).abs() < 1.0,
        "depth must add exactly 14px: {file_text:?} vs {folder_text:?}"
    );
    assert!(
        (file_text.center().y - file_rect.center().y).abs() < 0.75,
        "file label must be vertically centered in the row: {file_text:?} vs {file_rect:?}"
    );
    assert!(
        !output_text(&output).contains('▾') && !output_text(&output).contains('▸'),
        "the folder disclosure triangle must not paint as a font glyph"
    );
    assert!(
        !output_text(&output).contains('·'),
        "the file bullet must not paint as a font glyph"
    );
    let folder_icon =
        icon_in_slot(&output, folder_rect, 0).expect("expanded folder Material icon in its slot");
    assert_eq!(
        folder_icon.texture,
        icon_texture(&headless.ctx, icon_named("folder-src-open.svg"),),
        "an expanded folder must show its open icon"
    );
    assert!(
        (folder_icon.rect.center().y - folder_text.center().y).abs() < 0.75,
        "folder icon must share the label's optical center: {folder_icon:?} vs {folder_text:?}"
    );
    let file_icon =
        icon_in_slot(&output, file_rect, 1).expect("file Material icon in its depth-1 slot");
    assert_eq!(
        file_icon.texture,
        icon_texture(&headless.ctx, icon_named("rust.svg"),),
        "main.rs must show its Rust icon"
    );
    assert!(
        (file_icon.rect.center().y - file_text.center().y).abs() < 0.75,
        "file icon must share the label's optical center: {file_icon:?} vs {file_text:?}"
    );
    assert!(
        (file_icon.rect.center().x - (folder_icon.rect.center().x + 14.0)).abs() < 1.5,
        "depth must shift the icon slot by 14px: {file_icon:?} vs {folder_icon:?}"
    );

    let hovered = headless.hover(folder_row);
    let hover_fill = rects_with_fill(&hovered, made_app::theme::BG_HOVER)
        .into_iter()
        .find(|rect| rect.left() > explorer_left && rect.top() > body_top)
        .expect("hovered explorer row fill");
    assert!((hover_fill.height() - 24.0).abs() < 0.5);
    assert_eq!(
        rect_shapes(&hovered)
            .into_iter()
            .find(|shape| shape.fill == made_app::theme::BG_HOVER && shape.rect == hover_fill)
            .expect("hovered explorer row shape")
            .corner_radius,
        egui::CornerRadius::same(6)
    );
}

#[test]
fn explorer_rows_render_material_icons_in_the_icon_slot() {
    if isolated("explorer_rows_render_material_icons_in_the_icon_slot") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("icons");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.pump_until_rows(id, 2);
    let output = headless.render();

    // Directories sort first: the collapsed `src` folder, then `main.rs`.
    let folder = icon_in_slot(&output, explorer_row_rect(&headless, id, 0), 0)
        .expect("folder icon in its 16px slot");
    let file = icon_in_slot(&output, explorer_row_rect(&headless, id, 1), 0)
        .expect("file icon in its 16px slot");
    assert_eq!(
        folder.texture,
        icon_texture(&headless.ctx, icon_named("folder-src.svg"),),
        "a collapsed src folder must show its closed icon"
    );
    assert_eq!(
        file.texture,
        icon_texture(&headless.ctx, icon_named("rust.svg"),),
        "main.rs must show its Rust icon"
    );
    assert_ne!(folder.texture, file.texture, "folder and file icons differ");
    for image in [folder, file] {
        assert_eq!(
            image.tint,
            egui::Color32::WHITE,
            "icons keep the neutral tint"
        );
        assert_eq!(image.stroke, egui::Stroke::NONE, "icons paint no border");
        assert_eq!(
            image.corner_radius,
            egui::CornerRadius::ZERO,
            "icons paint no rounding"
        );
    }
}

#[test]
fn explorer_folder_icon_changes_between_closed_and_open() {
    if isolated("explorer_folder_icon_changes_between_closed_and_open") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("named-folder");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    let output = headless.render();
    let closed = icon_in_slot(&output, explorer_row_rect(&headless, id, 0), 0)
        .expect("collapsed folder icon");
    assert_eq!(
        closed.texture,
        icon_texture(&headless.ctx, icon_named("folder-src.svg"),),
        "a collapsed folder must show its closed icon"
    );

    headless.click(explorer_row_id(&headless, id, 0));
    headless.pump_until_rows(id, 2);
    let output = headless.render();
    let open = icon_in_slot(&output, explorer_row_rect(&headless, id, 0), 0)
        .expect("expanded folder icon");
    assert_eq!(
        open.texture,
        icon_texture(&headless.ctx, icon_named("folder-src-open.svg"),),
        "an expanded folder must show its open icon"
    );
    assert_ne!(
        closed.texture, open.texture,
        "closed and open folder icons must differ"
    );
}

#[test]
fn explorer_default_folder_icon_changes_between_closed_and_open() {
    if isolated("explorer_default_folder_icon_changes_between_closed_and_open") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("default-folder");
    fs::create_dir_all(root.join("zz-plain")).unwrap();
    fs::write(root.join("zz-plain/inner.txt"), "inner\n").unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    let output = headless.render();
    let closed = icon_in_slot(&output, explorer_row_rect(&headless, id, 0), 0)
        .expect("collapsed default folder icon");
    assert_eq!(
        closed.texture,
        icon_texture(&headless.ctx, icon_named("folder.svg"),),
        "an unmapped collapsed folder must show the theme's default closed icon"
    );

    headless.click(explorer_row_id(&headless, id, 0));
    headless.pump_until_rows(id, 2);
    let output = headless.render();
    let open = icon_in_slot(&output, explorer_row_rect(&headless, id, 0), 0)
        .expect("expanded default folder icon");
    assert_eq!(
        open.texture,
        icon_texture(&headless.ctx, icon_named("folder-open.svg"),),
        "an unmapped expanded folder must show the theme's default open icon"
    );
    assert_ne!(
        closed.texture, open.texture,
        "default closed and open folder icons must differ"
    );
}

#[test]
fn explorer_unknown_file_uses_default_icon() {
    if isolated("explorer_unknown_file_uses_default_icon") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("unknown-file");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("notes.zzunknown"), "notes\n").unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    let output = headless.render();
    let icon = icon_in_slot(&output, explorer_row_rect(&headless, id, 0), 0)
        .expect("unknown file icon in its 16px slot");
    assert_eq!(
        icon.texture,
        icon_texture(&headless.ctx, icon_named("file.svg")),
        "an unmapped file must show the theme's default file icon"
    );
}

#[test]
fn explorer_rows_keep_text_offsets_and_vertical_centering() {
    if isolated("explorer_rows_keep_text_offsets_and_vertical_centering") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("layout");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    for (width, height) in [(1280.0, 800.0), (720.0, 480.0)] {
        let mut model =
            AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
        let id = open_workspace(&mut model, &root);
        let mut headless = Headless::new(model, width, height);
        wait_explorer_ready(&mut headless, id);
        headless.render();
        headless.click(explorer_row_id(&headless, id, 0));
        headless.pump_until_rows(id, 2);
        let output = headless.render();

        let layout = made_app::theme::Layout::for_window(width, height);
        let explorer_left = width - layout.gutter - layout.right_card_width;
        let body_top = layout.gap + layout.top_height + layout.gap + layout.card_header_height;
        let folder_row = explorer_row_rect(&headless, id, 0);
        let file_row = explorer_row_rect(&headless, id, 1);
        assert!(
            (folder_row.height() - 24.0).abs() < 0.5 && (file_row.height() - 24.0).abs() < 0.5,
            "{width}x{height}: rows keep 24px height: {folder_row:?} {file_row:?}"
        );
        assert!(
            (folder_row.left() - (explorer_left + 8.0)).abs() < 1.0,
            "{width}x{height}: rows keep the 4px inset inside the body padding: {folder_row:?}"
        );

        // Text starts 28px from the row edge at depth 0, plus 14px per depth.
        let folder_text = text_rects(&output, "src", body_top + 24.0)
            .into_iter()
            .find(|rect| rect.top() > body_top)
            .expect("folder label");
        let file_text = text_rects(&output, "main.rs", body_top + 48.0)
            .into_iter()
            .find(|rect| rect.top() > body_top)
            .expect("file label");
        assert!(
            (folder_text.left() - (folder_row.left() + 28.0)).abs() < 1.0,
            "{width}x{height}: depth-0 text must start 28px from the row edge: {folder_text:?}"
        );
        assert!(
            (file_text.left() - (folder_row.left() + 42.0)).abs() < 1.0,
            "{width}x{height}: depth 1 must add exactly 14px: {file_text:?}"
        );

        // Each 16px icon sits in the inset slot, 4px before its label, and
        // shares the label's vertical optical center.
        for (row, depth, text) in [(folder_row, 0, folder_text), (file_row, 1, file_text)] {
            let icon = icon_in_slot(&output, row, depth).expect("icon in its slot");
            assert!(
                (icon.rect.left() - (row.left() + 8.0 + 14.0 * depth as f32)).abs() < 1.0,
                "{width}x{height}: icon must start in the inset slot: {icon:?} vs {row:?}"
            );
            assert!(
                (text.left() - icon.rect.right() - 4.0).abs() < 1.0,
                "{width}x{height}: label keeps its 4px gap after the icon: {icon:?} vs {text:?}"
            );
            assert!(
                (icon.rect.center().y - text.center().y).abs() < 0.75
                    && (icon.rect.center().y - row.center().y).abs() < 0.75,
                "{width}x{height}: icon and label share the row's optical center: {icon:?} vs {text:?}"
            );
        }

        // The former disclosure triangle and bullet are not painted in the tree.
        let tree_body = egui::Rect::from_min_max(
            egui::pos2(explorer_left, body_top),
            egui::pos2(width, height),
        );
        assert!(
            path_shapes(&output)
                .iter()
                .all(|path| !tree_body.contains(path_center(path))),
            "{width}x{height}: the app-drawn disclosure triangle must not paint"
        );
        assert!(
            circle_shapes(&output)
                .iter()
                .all(|circle| !tree_body.contains(circle.center)),
            "{width}x{height}: the app-drawn file bullet must not paint"
        );

        // The unselected file row gains no fill, border, outline, or accent.
        let slot = icon_slot(file_row, 1);
        let extra: Vec<egui::Rect> = rect_shapes(&output)
            .into_iter()
            .filter(|shape| {
                shape.fill_texture_id() == egui::TextureId::default()
                    && shape.rect.height() <= 24.5
                    && shape.rect.intersects(slot)
            })
            .map(|shape| shape.rect)
            .collect();
        assert!(
            extra.is_empty(),
            "{width}x{height}: the icon must not add a fill, border, or accent: {extra:?}"
        );
    }
}

#[test]
fn explorer_selected_row_fill_keeps_icon_inset() {
    if isolated("explorer_selected_row_fill_keeps_icon_inset") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "icon-inset", "entry.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.render();
    let row_id = explorer_row_id(&headless, id, 0);
    headless.click(row_id);
    let output = headless.render();

    let row = headless.response(row_id).rect;
    let fill = rects_with_fill(&output, made_app::theme::BG_SELECTED)
        .into_iter()
        .find(|rect| rect.intersects(row) && (rect.height() - 24.0).abs() < 0.5)
        .expect("selected row fill");
    let icon = icon_in_slot(&output, row, 0).expect("selected row icon");
    assert!(
        (fill.left() - (row.left() + 4.0)).abs() < 0.5,
        "the fill must keep its 4px horizontal inset: {fill:?} vs {row:?}"
    );
    assert!(
        icon.rect.left() >= fill.left() + 3.5,
        "the icon must sit at least 3.5px inside the fill's left edge: {icon:?} vs {fill:?}"
    );
    assert!(
        icon.rect.left() >= fill.left() && icon.rect.right() <= fill.right(),
        "the icon must stay fully inside the fill horizontally: {icon:?} vs {fill:?}"
    );
}

#[test]
fn explorer_icons_keep_full_color_on_hover_and_selection() {
    if isolated("explorer_icons_keep_full_color_on_hover_and_selection") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("states");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.pump_until_rows(id, 2);
    let tree_id = headless
        .model
        .workspace(id)
        .and_then(Workbench::explorer)
        .expect("explorer")
        .tree_focus_id();

    // At rest, the file row has no state fill.
    let output = headless.render();
    let row = explorer_row_rect(&headless, id, 1);
    let rest = icon_in_slot(&output, row, 0).expect("file icon at rest");
    assert_eq!(
        rest.texture,
        icon_texture(&headless.ctx, icon_named("rust.svg"),)
    );
    assert!(
        rects_with_fill(&output, made_app::theme::BG_HOVER)
            .iter()
            .chain(rects_with_fill(&output, made_app::theme::BG_SELECTED).iter())
            .all(|rect| !rect.intersects(row)),
        "the file row must start without a state fill"
    );

    // Hover changes the row fill, never the icon.
    let hovered_output = headless.hover(explorer_row_id(&headless, id, 1));
    let hovered = icon_in_slot(&hovered_output, row, 0).expect("file icon while hovered");
    assert!(
        rects_with_fill(&hovered_output, made_app::theme::BG_HOVER)
            .iter()
            .any(|rect| rect.intersects(row)),
        "hover must keep the row fill"
    );
    assert_eq!(
        (hovered.texture, hovered.tint),
        (rest.texture, rest.tint),
        "hover must not recolor the icon"
    );

    // Keyboard focus selects the file row: the fill changes, the icon does not.
    headless.render_events(vec![egui::Event::PointerMoved(egui::pos2(4.0, 796.0))]);
    headless.focus(tree_id);
    headless.press_key(egui::Key::ArrowDown);
    let selected_output = headless.render();
    assert_eq!(
        headless
            .model
            .workspace(id)
            .and_then(Workbench::explorer)
            .and_then(ExplorerPane::selected_path),
        Some(fs::canonicalize(root.join("main.rs")).unwrap().as_path()),
        "arrow navigation must select the file row"
    );
    let selected = icon_in_slot(&selected_output, row, 0).expect("file icon while selected");
    assert!(
        rects_with_fill(&selected_output, made_app::theme::BG_SELECTED)
            .iter()
            .any(|rect| rect.intersects(row)),
        "selection must keep the row fill"
    );
    for image in [rest, hovered, selected] {
        assert_eq!(
            (image.texture, image.tint),
            (rest.texture, egui::Color32::WHITE),
            "rest, hover, and selection must all render the same full-color icon"
        );
    }
}

#[cfg(unix)]
#[test]
fn explorer_special_rows_keep_explicit_markers() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    if isolated("explorer_special_rows_keep_explicit_markers") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("markers");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("target.txt"), "target\n").unwrap();
    symlink(root.join("target.txt"), root.join("link.txt")).unwrap();
    let mkfifo = std::process::Command::new("mkfifo")
        .arg(root.join("pipe"))
        .status()
        .expect("mkfifo");
    assert!(mkfifo.success(), "mkfifo must create the special entry");
    // An unreadable folder shows an error row once expanded.
    let locked = root.join("locked");
    fs::create_dir(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let _restore = RestoreMode(locked);

    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.render();
    headless.click(explorer_row_id(&headless, id, 0));
    headless.pump_until_rows(id, 5);
    let output = headless.render();

    // Rows: the expanded folder, its error row, then symlink, special, and file.
    let locked_row = explorer_row_rect(&headless, id, 0);
    let error_row = explorer_row_rect(&headless, id, 1);
    let link_row = explorer_row_rect(&headless, id, 2);
    let pipe_row = explorer_row_rect(&headless, id, 3);
    let target_row = explorer_row_rect(&headless, id, 4);
    for (label, row) in [
        ("link.txt", link_row),
        ("pipe", pipe_row),
        ("target.txt", target_row),
    ] {
        assert!(
            text_rects(&output, label, 1000.0)
                .iter()
                .any(|rect| (rect.center().y - row.center().y).abs() < 0.75),
            "{label} must occupy row {row:?}"
        );
    }

    let marker = |glyph: &str, row: egui::Rect, depth: usize| {
        glyph_marks(&output, glyph)
            .into_iter()
            .find(|(rect, _)| {
                (rect.center().y - row.center().y).abs() < 0.75
                    && (rect.center().x - (row.left() + 16.0 + 14.0 * depth as f32)).abs() < 1.5
            })
            .unwrap_or_else(|| panic!("{glyph} marker must sit in its icon slot of {row:?}"))
            .1
    };
    assert_eq!(
        marker("@", link_row, 0),
        made_app::theme::TEXT_TERTIARY,
        "symlink rows keep the @ marker in the tertiary tone"
    );
    assert_eq!(
        marker("?", pipe_row, 0),
        made_app::theme::TEXT_TERTIARY,
        "special rows keep the ? marker in the tertiary tone"
    );
    assert_eq!(
        marker("!", error_row, 1),
        made_app::theme::ERROR,
        "error rows keep the ! marker in the error tone"
    );
    for (row, depth) in [(link_row, 0), (pipe_row, 0), (error_row, 1)] {
        assert!(
            icon_in_slot(&output, row, depth).is_none(),
            "special rows must not show a theme icon: {row:?}"
        );
    }

    // Folders and regular files keep their theme icons.
    assert_eq!(
        icon_in_slot(&output, locked_row, 0)
            .expect("expanded folder icon")
            .texture,
        icon_texture(&headless.ctx, icon_named("folder-open.svg"),)
    );
    assert_eq!(
        icon_in_slot(&output, target_row, 0)
            .expect("file icon")
            .texture,
        icon_texture(&headless.ctx, icon_named("document.svg"),)
    );
}

#[test]
fn explorer_selection_and_tree_focus_paint_no_outline() {
    if isolated("explorer_selection_and_tree_focus_paint_no_outline") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("outline");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.render();
    let folder_row = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .row_widget_id(0);
    headless.click(folder_row);
    headless.pump_until_rows(id, 2);
    let tree_id = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .tree_focus_id();
    assert!(
        headless.ctx.memory(|memory| memory.has_focus(tree_id)),
        "clicking a tree row must focus the tree container"
    );
    let output = headless.render();

    let layout = made_app::theme::Layout::for_window(1280.0, 800.0);
    let explorer_left = 1280.0 - layout.gutter - layout.right_card_width;
    let explorer = egui::Rect::from_min_max(
        egui::pos2(explorer_left, layout.gap + layout.top_height + layout.gap),
        egui::pos2(1280.0, 800.0),
    );
    let folder_rect = headless.response(folder_row).rect;
    assert!(
        rects_with_fill(&output, made_app::theme::BG_SELECTED)
            .iter()
            .any(|rect| rect.intersects(folder_rect)),
        "pointer selection must keep the inset BG_SELECTED fill"
    );
    assert!(
        text_colors(&output, "src", 250.0).contains(&made_app::theme::TEXT_PRIMARY),
        "pointer selection must keep the primary label"
    );
    assert!(
        focus_ring_rects(&output, explorer).is_empty(),
        "pointer focus must not paint a row or container outline: {:?}",
        focus_ring_rects(&output, explorer)
    );

    headless.focus(tree_id);
    let focused = headless.render();
    assert!(
        headless.ctx.memory(|memory| memory.has_focus(tree_id)),
        "tree must still own keyboard focus"
    );
    assert!(
        focus_ring_rects(&focused, explorer).is_empty(),
        "keyboard tree focus must not paint a row or container outline: {:?}",
        focus_ring_rects(&focused, explorer)
    );
    assert!(
        rects_with_fill(&focused, made_app::theme::BG_SELECTED)
            .iter()
            .any(|rect| rect.intersects(folder_rect)),
        "keyboard focus must keep the selection fill"
    );
}

#[test]
fn tree_focus_with_no_selection_selects_first_row_without_preview() {
    if isolated("tree_focus_with_no_selection_selects_first_row_without_preview") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("tree-focus");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("alpha.txt"), "a\n").unwrap();
    fs::write(root.join("beta.txt"), "b\n").unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.render();
    assert_eq!(
        headless
            .model
            .workspace(id)
            .unwrap()
            .explorer()
            .unwrap()
            .selected_path(),
        None,
        "the tree must start without a selection"
    );

    let tree_id = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .tree_focus_id();
    let first_row = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .row_widget_id(0);
    // This is the Tab/programmatic focus path; no pointer click occurs.
    headless.focus(tree_id);
    let output = headless.render();

    let workbench = headless.model.workspace(id).unwrap();
    let explorer = workbench.explorer().unwrap();
    assert_eq!(
        explorer.selected_path(),
        Some(fs::canonicalize(root.join("alpha.txt")).unwrap().as_path()),
        "focusing the tree must select its first visible row"
    );
    assert!(
        explorer.preview_path().is_none(),
        "focus must not auto-open a first-row file"
    );
    let row_rect = headless.response(first_row).rect;
    assert!(
        rects_with_fill(&output, made_app::theme::BG_SELECTED)
            .iter()
            .any(|rect| rect.intersects(row_rect)),
        "first-row focus selection must use the inset BG_SELECTED fill"
    );
    assert!(
        text_colors(&output, "alpha.txt", 250.0).contains(&made_app::theme::TEXT_PRIMARY),
        "first-row focus selection must use primary text"
    );
    let layout = made_app::theme::Layout::for_window(1280.0, 800.0);
    let explorer_rect = egui::Rect::from_min_max(
        egui::pos2(1280.0 - layout.gutter - layout.right_card_width, 0.0),
        egui::pos2(1280.0, 800.0),
    );
    assert!(
        focus_ring_rects(&output, explorer_rect).is_empty(),
        "first-row focus must not paint a container/row outline: {:?}",
        focus_ring_rects(&output, explorer_rect)
    );
}

#[test]
fn icon_only_header_actions_expose_tooltips_and_visible_focus() {
    if isolated("icon_only_header_actions_expose_tooltips_and_visible_focus") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "icons", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    headless.render();
    for action in [
        top_action_id("open-folder"),
        top_workspace_action_id(id, "explorer"),
        made_app::theme::wordmark_id(),
    ] {
        let response = headless.response(action);
        assert!(
            response.rect.width() >= 24.0 && response.rect.height() >= 24.0,
            "header action target below 24px: {action:?} {:?}",
            response.rect
        );
    }
    assert!(
        headless
            .ctx
            .read_response(top_action_id("diagnostics"))
            .is_none(),
        "the header must not render a Diagnostics icon"
    );
    let open_folder_rect = headless.response(top_action_id("open-folder")).rect;
    let hovered_output = headless.hover_until_text(top_action_id("open-folder"), "Open folder");
    let hovered = output_text(&hovered_output);
    assert!(
        hovered.contains("Open folder"),
        "Open folder tooltip missing: {hovered}"
    );
    assert!(
        rects_with_fill(&hovered_output, made_app::theme::BG_HOVER)
            .iter()
            .any(|rect| rect.contains_rect(open_folder_rect)),
        "hovered header icon must use BG_HOVER"
    );
    let hovered = output_text(
        &headless.hover_until_text(top_workspace_action_id(id, "explorer"), "Hide explorer"),
    );
    assert!(
        hovered.contains("Hide explorer"),
        "Explorer tooltip missing: {hovered}"
    );
    let hovered = headless.hover_until_text(made_app::theme::wordmark_id(), "About");
    let hovered_text = output_text(&hovered);
    assert!(
        hovered_text.contains("About"),
        "wordmark About tooltip missing: {hovered_text}"
    );
    let wordmark_rect = headless.response(made_app::theme::wordmark_id()).rect;
    assert!(
        !rects_with_fill(&hovered, made_app::theme::BG_HOVER)
            .iter()
            .any(|rect| rect.intersects(wordmark_rect)),
        "the wordmark trigger must not paint a hover fill"
    );
    headless.focus(top_action_id("open-folder"));
    let focused = headless.render();
    assert!(
        has_stroke_color(&focused, made_app::theme::FOCUS),
        "focused header icon must paint a visible focus ring"
    );
    headless.focus(made_app::theme::wordmark_id());
    let focused = headless.render();
    assert!(
        has_stroke_color(&focused, made_app::theme::FOCUS),
        "focused wordmark trigger must paint a visible focus ring"
    );
}

#[test]
fn wordmark_about_trigger_opens_only_with_an_active_workspace() {
    if isolated("wordmark_about_trigger_opens_only_with_an_active_workspace") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "wordmark-about", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    let closed = output_text(&headless.render());
    assert!(
        !closed.contains("Show diagnostics"),
        "About must start closed: {closed}"
    );

    headless.click(made_app::theme::wordmark_id());
    let opened = output_text(&headless.render());
    assert!(
        opened.contains("Show diagnostics"),
        "pointer activation must open About: {opened}"
    );

    headless.focus(made_app::theme::wordmark_id());
    headless.press_key(egui::Key::Enter);
    let closed = output_text(&headless.render());
    assert!(
        !closed.contains("Show diagnostics"),
        "Enter must close About: {closed}"
    );

    headless.focus(made_app::theme::wordmark_id());
    headless.press_key(egui::Key::Space);
    let opened = output_text(&headless.render());
    assert!(
        opened.contains("Show diagnostics"),
        "Space must open About: {opened}"
    );

    // Pre-start shell: a launch plan exists, but no workspace runtime is active.
    let plan = LaunchPlan::open(&root, WorkspaceSource::Picker).unwrap();
    let model = AppModel::from_launch(WorkspaceLaunch::from_plan(plan), AppOptions::new());
    assert!(model.active_workspace_id().is_none());
    let mut headless = Headless::new(model, 1280.0, 800.0);
    headless.render_ui_only(Vec::new());
    headless.render_ui_only(Vec::new());
    let position = headless
        .response(made_app::theme::wordmark_id())
        .rect
        .center();
    headless.render_ui_only(vec![egui::Event::PointerMoved(position)]);
    headless.render_ui_only(vec![pointer_button(position, true)]);
    let output = headless.render_ui_only(vec![pointer_button(position, false)]);
    assert!(headless.model.active_workspace_id().is_none());
    assert!(
        !output_text(&output).contains("Show diagnostics"),
        "activation without an active workspace must not open About"
    );
}

#[test]
fn about_reveals_diagnostics_on_demand_and_resets_when_reopened() {
    if isolated("about_reveals_diagnostics_on_demand_and_resets_when_reopened") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "about-screen", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);

    headless.click(made_app::theme::wordmark_id());
    let pane_id = headless.model.workspace(id).unwrap().pane_id().to_owned();
    let toggle_id = about_diagnostics_toggle_id(&pane_id);
    let window_id = egui::Id::unique(("made-about-window", pane_id.as_str()));
    let default_output = headless.render();
    let text = output_text(&default_output);
    for expected in [
        "MADE",
        &format!("{} {}", made_app::APP_NAME, made_app::APP_VERSION),
        "Minimal Agentic Development Environment",
        made_app::APP_DESCRIPTION,
        "Show diagnostics",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
    assert!(
        rects_with_fill(&default_output, made_app::theme::ACCENT).is_empty(),
        "the hidden diagnostics toggle must not use the active accent fill"
    );
    for hidden in [
        "Copy diagnostics",
        "Previous log",
        "Copy log path",
        "Effective root",
    ] {
        assert!(!text.contains(hidden), "About leaked {hidden:?}: {text}");
    }
    let collapsed_window = headless
        .ctx
        .memory(|memory| memory.area_rect(window_id))
        .expect("About window rect");
    assert!(
        collapsed_window.height() < 300.0,
        "collapsed About must stay compact: {collapsed_window:?}"
    );

    headless.click(toggle_id);
    headless.render_events_at(Vec::new(), headless.last_time + 1.0 / 60.0);
    let mid_window = headless
        .ctx
        .memory(|memory| memory.area_rect(window_id))
        .expect("About window rect");
    assert!(
        mid_window.height() > collapsed_window.height() && mid_window.height() < 700.0,
        "diagnostics reveal must animate through an intermediate height: collapsed {collapsed_window:?}, mid {mid_window:?}"
    );
    settle_about_animation(&mut headless);
    let revealed_output = headless.render();
    let text = output_text(&revealed_output);
    for expected in [
        "Hide diagnostics",
        "Copy diagnostics",
        "Previous log",
        "Effective root",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
    assert!(!text.contains("Show diagnostics"));
    assert!(
        !rects_with_fill(&revealed_output, made_app::theme::ACCENT).is_empty(),
        "the revealed diagnostics toggle must use the active accent fill"
    );
    let expanded_window = headless
        .ctx
        .memory(|memory| memory.area_rect(window_id))
        .expect("About window rect");
    assert!(
        expanded_window.height() > collapsed_window.height() + 100.0,
        "revealing diagnostics must grow About: collapsed {collapsed_window:?}, expanded {expanded_window:?}"
    );
    assert!(
        expanded_window.height() >= 720.0,
        "revealed About must use the available app height: {expanded_window:?}"
    );

    headless.click(toggle_id);
    settle_about_animation(&mut headless);
    let hidden_output = headless.render();
    let text = output_text(&hidden_output);
    assert!(text.contains("Show diagnostics"));
    assert!(!text.contains("Copy diagnostics"));
    assert!(
        rects_with_fill(&hidden_output, made_app::theme::ACCENT).is_empty(),
        "the closed diagnostics toggle must return to the secondary treatment"
    );
    let collapsed_again = headless
        .ctx
        .memory(|memory| memory.area_rect(window_id))
        .expect("About window rect");
    assert!(
        collapsed_again.height() < 300.0,
        "hiding diagnostics must return About to its compact height: {collapsed_again:?}"
    );

    let about_rect = headless
        .ctx
        .memory(|memory| {
            memory.area_rect(egui::Id::unique(("made-about-window", pane_id.as_str())))
        })
        .expect("About window rect");
    headless.focus(toggle_id);
    let focused = headless.render();
    assert!(
        !focus_ring_rects(&focused, about_rect.expand(4.0)).is_empty(),
        "About diagnostics toggle must paint a visible focus ring"
    );
    headless.focus(toggle_id);
    headless.press_key(egui::Key::Enter);
    assert!(output_text(&headless.render()).contains("Hide diagnostics"));
    headless.focus(toggle_id);
    headless.press_key(egui::Key::Space);
    assert!(output_text(&headless.render()).contains("Show diagnostics"));

    headless.click(made_app::theme::wordmark_id());
    headless.render();
    headless.click(made_app::theme::wordmark_id());
    let text = output_text(&headless.render());
    assert!(text.contains("Show diagnostics"));
    assert!(!text.contains("Copy diagnostics"));
}

#[test]
fn header_places_tabs_left_and_wordmark_before_icons() {
    if isolated("header_places_tabs_left_and_wordmark_before_icons") {
        return;
    }
    for (width, height, gap, top_height, tab_height) in [
        (1280.0, 800.0, 24.0, 26.0, 26.0),
        (720.0, 480.0, 18.0, 24.0, 24.0),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let alpha = fixture_workspace(temp.path(), "alpha", "a.txt");
        let beta = fixture_workspace(temp.path(), "beta", "b.txt");
        let mut model =
            AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
        let alpha_id = open_workspace(&mut model, &alpha);
        let beta_id = open_workspace(&mut model, &beta);
        let mut headless = Headless::new(model, width, height);
        headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.interaction.tooltip_delay = 0.0
        });
        wait_explorer_ready(&mut headless, alpha_id);
        wait_explorer_ready(&mut headless, beta_id);
        let output = headless.render();
        let layout = made_app::theme::Layout::for_window(width, height);

        let wordmark = headless.response(made_app::theme::wordmark_id()).rect;
        let first_tab = headless.response(workspace_tab_id(alpha_id)).rect;
        let last_tab = headless.response(workspace_tab_id(beta_id)).rect;
        let plus = headless.response(top_action_id("open-folder")).rect;
        let ssh = headless.response(top_action_id("ssh-servers")).rect;
        let explorer = headless
            .response(top_workspace_action_id(beta_id, "explorer"))
            .rect;
        let content_left = layout.gutter;
        let content_right = width - layout.gutter;

        assert!(
            headless
                .ctx
                .read_response(top_action_id("diagnostics"))
                .is_none(),
            "{width}px: the header must not render a Diagnostics icon"
        );
        assert!(
            wordmark.width() >= 24.0 && wordmark.height() >= 24.0,
            "{width}px: the wordmark trigger target must be at least 24x24: {wordmark:?}"
        );

        assert_eq!(layout.top_height, top_height);
        assert_eq!(layout.tab_height, tab_height);
        assert!(
            (first_tab.left() - content_left).abs() < 0.75,
            "{width}px: the first tab must start at the card left edge: {first_tab:?} vs {content_left}"
        );
        assert!(
            (first_tab.height() - tab_height).abs() < 0.5,
            "{width}px: tab height must be {tab_height}: {first_tab:?}"
        );
        assert!(
            (first_tab.top() - layout.gap).abs() < 0.5,
            "{width}px: header top must be one pane gap below the window edge: {first_tab:?}"
        );
        assert!(
            (explorer.right() - content_right).abs() < 0.75,
            "{width}px: the last icon target must end at the card right edge: explorer {explorer:?}, content right {content_right}"
        );
        assert!(
            explorer.right() <= width,
            "{width}px: the last icon target must remain inside the window: {explorer:?}"
        );
        let explorer_glyph = rect_shapes(&output)
            .into_iter()
            .find(|shape| {
                (shape.rect.width() - 14.0).abs() < 1.0
                    && (shape.rect.height() - 11.0).abs() < 1.0
                    && shape.rect.top() < layout.gap + layout.top_height
            })
            .map(|shape| shape.rect)
            .expect("Explorer panel glyph");
        let left_padding = explorer_glyph.left() - explorer.left();
        let right_padding = explorer.right() - explorer_glyph.right();
        assert!(
            (left_padding - right_padding).abs() < 0.75,
            "{width}px: the last glyph must be centered in its target: glyph {explorer_glyph:?}, target {explorer:?}, padding {left_padding}/{right_padding}"
        );
        assert!(
            (explorer_glyph.center().x - explorer.center().x).abs() < 0.75,
            "{width}px: the last glyph center must match the target center: glyph {explorer_glyph:?}, target {explorer:?}"
        );
        assert!(
            explorer_glyph.right() < content_right - 0.5,
            "{width}px: the centered glyph must sit inside the card right edge: {explorer_glyph:?} vs {content_right}"
        );
        assert!(
            explorer.contains_rect(explorer_glyph),
            "{width}px: the glyph must stay inside its hit target: glyph {explorer_glyph:?}, target {explorer:?}"
        );
        let is_explorer_fill = |rect: &egui::Rect| {
            (rect.width() - tab_height).abs() < 0.75
                && (rect.height() - tab_height).abs() < 0.75
                && rect.intersects(explorer)
        };
        let explorer_fill = rects_with_fill(&output, made_app::theme::BG_SELECTED)
            .into_iter()
            .find(is_explorer_fill)
            .expect("selected Explorer fill");
        assert!(
            (explorer_fill.right() - content_right).abs() < 0.75,
            "{width}px: the selected Explorer fill must end at the card right edge: {explorer_fill:?}"
        );
        assert!(
            plus.left() >= last_tab.right() - 0.5 && plus.left() - last_tab.right() <= 8.0,
            "{width}px: Open folder plus must sit immediately after the last tab: tab {last_tab:?}, plus {plus:?}"
        );
        assert!(
            wordmark.left() - plus.right() >= gap - 0.75,
            "{width}px: tab strip must leave a {gap}px gap before the wordmark: plus {plus:?}, wordmark {wordmark:?}"
        );
        let control_gap = if layout.compact { 4.0 } else { 6.0 };
        assert!(
            (ssh.left() - wordmark.right() - gap).abs() < 0.75,
            "{width}px: wordmark-to-first-icon gap must be {gap}px: wordmark {wordmark:?}, ssh {ssh:?}"
        );
        assert!(
            (explorer.left() - ssh.right() - control_gap).abs() < 1.0,
            "{width}px: Servidores SSH must sit immediately left of Explorer: ssh {ssh:?}, explorer {explorer:?}"
        );
        assert!(
            explorer.right() > ssh.right() && explorer.right() > wordmark.right(),
            "{width}px: Explorer toggle must be rightmost: wordmark {wordmark:?}, ssh {ssh:?}, explorer {explorer:?}"
        );
        assert!(
            plus.width() >= 24.0 && plus.height() >= 24.0,
            "{width}px: Open folder target below 24px: {plus:?}"
        );
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
        for (name, rect) in [
            ("wordmark", wordmark),
            ("plus", plus),
            ("explorer", explorer),
            (
                "active tab",
                headless.response(workspace_tab_id(beta_id)).rect,
            ),
        ] {
            assert!(
                screen.contains_rect(rect),
                "{width}px: {name} is clipped: {rect:?}"
            );
        }
        assert!(
            !rects_with_fill(&output, made_app::theme::BG_TAB_SELECTED)
                .iter()
                .any(|rect| rect.intersects(wordmark))
                && !rects_with_fill(&output, made_app::theme::BG_TAB_HOVER)
                    .iter()
                    .any(|rect| rect.intersects(wordmark)),
            "{width}px: the wordmark must not paint a tab fill"
        );
        let icon_cluster_left = explorer.left();
        assert!(
            wordmark.right() < icon_cluster_left,
            "{width}px: wordmark must sit before the icon group"
        );
        let card = largest_rect(&output, made_app::theme::BG_CARD_GLASS).expect("explorer card");
        let panel_bottom = layout.gap + layout.top_height + layout.gap;
        assert!(
            (card.top() - panel_bottom).abs() < 1.0,
            "{width}px: compact header must move card top to {panel_bottom}px: {card:?}"
        );
        assert!(
            (card.top() - first_tab.bottom() - layout.gap).abs() < 1.0,
            "{width}px: the space above and below the strip must both equal the pane gap"
        );

        let wordmark_hover = headless.hover_until_text(made_app::theme::wordmark_id(), "About");
        assert!(
            output_text(&wordmark_hover).contains("About"),
            "{width}px: wordmark About tooltip missing"
        );
        for (name, fill) in [
            ("BG_HOVER", made_app::theme::BG_HOVER),
            ("BG_SELECTED", made_app::theme::BG_SELECTED),
            ("BG_TAB_HOVER", made_app::theme::BG_TAB_HOVER),
            ("BG_TAB_SELECTED", made_app::theme::BG_TAB_SELECTED),
        ] {
            assert!(
                !rects_with_fill(&wordmark_hover, fill)
                    .iter()
                    .any(|rect| rect.intersects(wordmark)),
                "{width}px: hovering the wordmark must not paint a {name} fill"
            );
        }
        headless.focus(made_app::theme::wordmark_id());
        let focused = headless.render();
        assert!(
            has_stroke_color(&focused, made_app::theme::FOCUS),
            "{width}px: focused wordmark trigger must paint a visible focus ring"
        );

        // The selected Explorer keeps BG_SELECTED on hover; hide it to exercise
        // the unselected BG_HOVER fill on the same flush target.
        let explorer_id = top_workspace_action_id(beta_id, "explorer");
        headless.click(explorer_id);
        headless.render();
        let hover_output = headless.hover(explorer_id);
        let hover_fill = rects_with_fill(&hover_output, made_app::theme::BG_HOVER)
            .into_iter()
            .find(is_explorer_fill)
            .expect("hovered Explorer fill");
        assert!(
            (hover_fill.right() - content_right).abs() < 0.75,
            "{width}px: the hovered Explorer fill must end at the card right edge: {hover_fill:?}"
        );

        headless.pump_until(|model| {
            model
                .workspace(beta_id)
                .and_then(Workbench::terminal)
                .is_some_and(made_app::TerminalPane::is_running)
        });
        let terminal_pid = headless
            .model
            .workspace(beta_id)
            .and_then(Workbench::terminal)
            .and_then(made_app::TerminalPane::pid);
        headless.focus(top_action_id("open-folder"));
        headless.press_key(egui::Key::Enter);
        headless.render();
        // Enter already opened the picker when nothing else is offered. Do not
        // send the menu chord into that picker: ArrowDown/Enter would navigate
        // the folder list, not a terminal row that was never shown.
        let menu_open = !headless.model.is_picker()
            && headless
                .ctx
                .read_response(new_tab_menu_item_id("terminal"))
                .is_some();
        if menu_open {
            headless.press_key(egui::Key::ArrowDown);
            headless.render();
            headless.press_key(egui::Key::Enter);
            headless.render();
        }
        assert!(
            headless.model.is_picker(),
            "{width}px: + must open the picker directly when nothing else is offered, or via Terminal when a menu is open"
        );
        let terminal = headless
            .model
            .workspace(beta_id)
            .and_then(Workbench::terminal);
        assert!(
            terminal.is_some_and(made_app::TerminalPane::is_running),
            "{width}px: opening the picker must leave the local terminal running (pid {terminal_pid:?}, status {})",
            terminal
                .map(made_app::TerminalPane::shell_status_label)
                .unwrap_or_else(|| "absent".to_owned())
        );
    }
}

#[test]
fn header_plus_precedes_overflow_and_both_stay_reachable() {
    if isolated("header_plus_precedes_overflow_and_both_stay_reachable") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let mut ids = Vec::new();
    for index in 0..4 {
        let root = fixture_workspace(
            temp.path(),
            &format!("workspace-with-a-long-name-{index}"),
            "main.rs",
        );
        ids.push(open_workspace(&mut model, &root));
    }
    let mut headless = Headless::new(model, 720.0, 480.0);
    for id in &ids {
        wait_explorer_ready(&mut headless, *id);
    }
    headless.render();

    let plus = headless.response(top_action_id("open-folder")).rect;
    let overflow = headless.response(top_action_id("workspace-overflow")).rect;
    let wordmark = headless.response(made_app::theme::wordmark_id()).rect;
    let ssh = headless.response(top_action_id("ssh-servers")).rect;
    let layout = made_app::theme::Layout::for_window(720.0, 480.0);
    let gap = made_app::theme::wordmark_gap(true);
    let active = headless
        .model
        .active_workspace_id()
        .expect("active workspace");
    assert!(
        headless
            .ctx
            .read_response(workspace_tab_id(active))
            .is_some(),
        "the active workspace tab must remain visible under overflow"
    );
    assert!(
        plus.right() <= overflow.left() + 0.5,
        "the plus must precede the overflow trigger: plus {plus:?}, overflow {overflow:?}"
    );
    assert!(
        overflow.right() <= wordmark.left() - gap + 0.75,
        "overflow trigger must leave the wordmark breathing gap: overflow {overflow:?}, wordmark {wordmark:?}, gap {gap}"
    );
    let first_visible = ids
        .iter()
        .filter_map(|id| {
            headless
                .ctx
                .read_response(workspace_tab_id(*id))
                .map(|response| response.rect)
        })
        .min_by(|left, right| left.left().total_cmp(&right.left()))
        .expect("visible tab");
    assert!(
        (first_visible.left() - layout.gutter).abs() < 0.75,
        "first visible tab must start at the header content left: {first_visible:?}"
    );
    let visible_tabs: Vec<egui::Rect> = ids
        .iter()
        .filter_map(|id| {
            headless
                .ctx
                .read_response(workspace_tab_id(*id))
                .map(|response| response.rect)
        })
        .collect();
    assert!(!visible_tabs.is_empty());
    assert!(
        visible_tabs
            .iter()
            .all(|tab| tab.right() <= plus.left() + 0.5),
        "no visible tab may overlap the plus: tabs {visible_tabs:?}, plus {plus:?}"
    );
    for (name, rect) in [("plus", plus), ("overflow", overflow), ("ssh", ssh)] {
        assert!(
            rect.width() >= 24.0 && rect.height() >= 24.0,
            "{name} target below 24px or clipped: {rect:?}"
        );
    }
    for (name, rect) in [
        ("plus", plus),
        ("overflow", overflow),
        ("ssh", ssh),
        ("wordmark", wordmark),
    ] {
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(720.0, 480.0))
                .contains_rect(rect),
            "{name} must remain inside the compact window: {rect:?}"
        );
    }
}

#[test]
fn workspace_tab_close_is_hidden_until_hover_or_focus_without_layout_growth() {
    if isolated("workspace_tab_close_is_hidden_until_hover_or_focus_without_layout_growth") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let alpha = fixture_workspace(temp.path(), "alpha", "a.txt");
    let beta = fixture_workspace(temp.path(), "beta", "b.txt");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let alpha_id = open_workspace(&mut model, &alpha);
    let beta_id = open_workspace(&mut model, &beta);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, alpha_id);
    wait_explorer_ready(&mut headless, beta_id);

    let alpha_tab_id = workspace_tab_id(alpha_id);
    let beta_tab_id = workspace_tab_id(beta_id);
    let alpha_close_id = workspace_tab_close_id(alpha_id);
    let beta_close_id = workspace_tab_close_id(beta_id);
    let rest = headless.render();
    assert!(
        text_rects(&rest, "×", 55.0).is_empty(),
        "the close glyph must be absent at rest for idle and selected tabs"
    );
    let alpha_tab = headless.response(alpha_tab_id).rect;
    let beta_tab = headless.response(beta_tab_id).rect;
    let alpha_close = headless.response(alpha_close_id).rect;
    let beta_close = headless.response(beta_close_id).rect;
    for (name, rect) in [("idle close", alpha_close), ("selected close", beta_close)] {
        assert!(
            rect.width() >= 24.0 && rect.height() >= 24.0,
            "{name} target must remain at least 24×24 while hidden: {rect:?}"
        );
    }
    let alpha_label = text_rects(&rest, "alpha", 55.0)
        .into_iter()
        .find(|rect| alpha_tab.contains_rect(*rect))
        .expect("idle tab label");

    headless.hover(alpha_tab_id);
    headless.advance(0.2);
    let hovered_tab = headless.render();
    let hovered_glyphs = text_rects(&hovered_tab, "×", 55.0);
    assert_eq!(hovered_glyphs.len(), 1, "tab hover must reveal one X");
    assert!(
        alpha_close.contains_rect(hovered_glyphs[0]),
        "the revealed X must stay in the idle tab close target"
    );
    assert!(
        text_font_sizes(&hovered_tab, "×", 55.0)
            .iter()
            .all(|size| (size - made_app::theme::CLOSE_GLYPH_SIZE).abs() < 0.1),
        "the revealed X must use the fixed close glyph size"
    );
    assert!(
        !text_colors(&hovered_tab, "×", 55.0).contains(&made_app::theme::ERROR),
        "tab hover alone must not paint the close glyph red"
    );
    assert_eq!(headless.response(alpha_tab_id).rect, alpha_tab);
    let hovered_label = text_rects(&hovered_tab, "alpha", 55.0)
        .into_iter()
        .find(|rect| alpha_tab.contains_rect(*rect))
        .expect("hovered tab label");
    assert_eq!(
        hovered_label, alpha_label,
        "showing the X must not move or resize the label"
    );

    headless.hover(alpha_close_id);
    headless.advance(0.2);
    let hovered_close = headless.render();
    let close_glyphs = text_rects(&hovered_close, "×", 55.0);
    assert_eq!(close_glyphs.len(), 1, "close hover must keep one X visible");
    assert!(
        alpha_close.contains_rect(close_glyphs[0]),
        "close hover must keep the X in its target"
    );
    assert!(
        text_font_sizes(&hovered_close, "×", 55.0)
            .iter()
            .all(|size| (size - made_app::theme::CLOSE_GLYPH_SIZE).abs() < 0.1),
        "close hover must not scale the glyph"
    );
    assert!(
        text_colors(&hovered_close, "×", 55.0).contains(&made_app::theme::ERROR),
        "direct close hover must paint ERROR"
    );
    assert!(
        !rects_with_fill(&hovered_close, made_app::theme::BG_HOVER)
            .iter()
            .any(|rect| rect.intersects(alpha_close)),
        "direct close hover must not paint an X background"
    );

    headless.render_events(vec![egui::Event::PointerMoved(egui::pos2(
        4.0,
        headless.height - 4.0,
    ))]);
    headless.focus(alpha_tab_id);
    headless.advance(0.2);
    let focused_tab = headless.render();
    assert_eq!(
        text_rects(&focused_tab, "×", 55.0).len(),
        1,
        "tab focus must reveal the X"
    );
    assert!(
        text_font_sizes(&focused_tab, "×", 55.0)
            .iter()
            .all(|size| (size - made_app::theme::CLOSE_GLYPH_SIZE).abs() < 0.1),
        "focused tab X must keep the fixed size"
    );
    assert!(
        !focus_ring_rects(&focused_tab, alpha_tab).is_empty(),
        "focused tab must keep its focus ring"
    );

    headless.focus(alpha_close_id);
    let focused_close = headless.render();
    assert_eq!(
        text_rects(&focused_close, "×", 55.0).len(),
        1,
        "close focus must keep the X visible"
    );
    assert!(
        text_font_sizes(&focused_close, "×", 55.0)
            .iter()
            .all(|size| (size - made_app::theme::CLOSE_GLYPH_SIZE).abs() < 0.1),
        "focused close X must keep the fixed size"
    );
    assert!(
        !focus_ring_rects(&focused_close, alpha_close).is_empty(),
        "focused close must paint its inside focus ring"
    );
    assert_eq!(headless.response(alpha_tab_id).rect, alpha_tab);
    assert_eq!(headless.response(beta_tab_id).rect, beta_tab);

    headless.press_key(egui::Key::Enter);
    headless.render();
    assert!(
        headless.model.workspace(alpha_id).is_none(),
        "Enter on the focused close must close the workspace"
    );
}

#[test]
fn workspace_strip_creates_switches_and_closes_tabs_to_picker() {
    if isolated("workspace_strip_creates_switches_and_closes_tabs_to_picker") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let alpha = fixture_workspace(temp.path(), "alpha", "a.txt");
    let beta = fixture_workspace(temp.path(), "beta", "b.txt");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let alpha_id = open_workspace(&mut model, &alpha);
    let beta_id = open_workspace(&mut model, &beta);
    let mut headless = Headless::new(model, 1100.0, 700.0);
    wait_explorer_ready(&mut headless, alpha_id);
    wait_explorer_ready(&mut headless, beta_id);
    assert_eq!(headless.model.active_workspace_id(), Some(beta_id));

    headless.click(workspace_tab_id(alpha_id));
    assert_eq!(headless.model.active_workspace_id(), Some(alpha_id));

    headless.click(workspace_tab_close_id(beta_id));
    assert!(headless.model.workspace(beta_id).is_none());
    assert_eq!(headless.model.active_workspace_id(), Some(alpha_id));

    headless.click(top_action_id("open-folder"));
    // No local TUI and no saved server: + opens the picker and does not draw
    // a Terminal item. When a menu is open, Terminal is still that choice.
    if headless
        .ctx
        .read_response(new_tab_menu_item_id("terminal"))
        .is_some()
    {
        headless.click(new_tab_menu_item_id("terminal"));
    }
    assert!(headless.model.is_picker());
    assert_eq!(headless.model.workspace_count(), 1);
    let picker_text = output_text(&headless.render());
    assert!(picker_text.contains("Open a workspace"));

    assert!(headless.model.cancel_picker());
    assert_eq!(headless.model.active_workspace_id(), Some(alpha_id));
    headless.render();
    headless.click(workspace_tab_close_id(alpha_id));
    assert!(headless.model.is_picker());
    assert_eq!(headless.model.workspace_count(), 0);
    assert!(headless.model.workbench().is_none());
    let final_text = output_text(&headless.render());
    assert!(final_text.contains("Open a workspace"));
}

#[cfg(unix)]
#[test]
fn background_tui_detection_adopts_the_isolated_opencode() {
    if isolated("background_tui_detection_adopts_the_isolated_opencode") {
        return;
    }
    // The isolated PATH still contains the real grok after the fake bin. A
    // no-op grok there keeps plugin install from running the user's binary.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin = std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
            .next()
            .expect("isolated bin");
        let grok = bin.join("grok");
        std::fs::write(&grok, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&grok, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tui_detection(),
    );
    assert!(
        model.compatible_tuis().is_empty(),
        "detection must not block construction"
    );
    let mut headless = Headless::new(model, 1100.0, 700.0);
    headless.pump_until(|model| model.compatible_tuis().contains(&OPENCODE));
    let detected = headless.model.compatible_tuis();
    assert!(
        detected == [OPENCODE] || detected == [OPENCODE, GROK],
        "the background worker must adopt catalog order from the isolated PATH: {detected:?}"
    );
}

#[test]
fn header_plus_without_a_detected_tui_opens_the_folder_picker() {
    if isolated("header_plus_without_a_detected_tui_opens_the_folder_picker") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "fallback", "main.rs");
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([]),
    );
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1100.0, 700.0);
    wait_explorer_ready(&mut headless, id);
    headless.pump_until(|model| {
        model
            .workspace(id)
            .and_then(Workbench::terminal)
            .is_some_and(made_app::TerminalPane::is_running)
    });

    headless.click(top_action_id("open-folder"));
    assert!(
        headless.model.is_picker(),
        "without a local TUI and without a saved SSH server the + opens the picker"
    );
    assert!(headless
        .ctx
        .read_response(new_tab_menu_item_id("terminal"))
        .is_none());
    assert_eq!(headless.model.workspace_count(), 1);
    assert!(headless.model.workspace(id).is_some());
    assert!(
        headless
            .model
            .workspace(id)
            .and_then(Workbench::terminal)
            .is_some_and(made_app::TerminalPane::is_running),
        "opening the picker must leave the local terminal running"
    );
}

#[test]
fn ssh_header_registration_and_remote_tab_keep_local_work() {
    if isolated("ssh_header_registration_and_remote_tab_keep_local_work") {
        return;
    }
    use made_app::remote_workspace::RemoteWorkspace;
    use made_app::ssh_servers::ServerMetadata;
    use made_remote::protocol::{
        DirectoryEntry, EntryKind, Identity, Operation, RemotePath, TabCapability, WireEnvelope,
    };

    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "local-stays", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let local_id = open_workspace(&mut model, &root);
    let id = model
        .add_ssh_metadata(ServerMetadata {
            display_name: "Development".into(),
            host: "server-a".into(),
            user: None,
            port: Some(22),
            key_file: Some("/keys/reference-only".into()),
            initial_directory: None,
        })
        .expect("server metadata saves without credentials");
    let registry_path = {
        use made_platform::Platform;
        made_platform::host_platform()
            .app_paths()
            .expect("isolated app paths")
            .data_dir
            .join("ssh-servers.json")
    };
    let encoded = std::fs::read_to_string(&registry_path).expect("registry file");
    for forbidden in ["password", "token", "private_key", "prompt"] {
        assert!(!encoded.contains(forbidden), "{encoded}");
    }
    assert!(encoded.contains("server-a"));

    let mut workspace = RemoteWorkspace::new(
        Identity {
            authority: id,
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        },
        RemotePath::new(b"/project".to_vec()).unwrap(),
    )
    .unwrap();
    let list_path = RemotePath::new(b"/project".to_vec()).unwrap();
    let request = workspace
        .request(
            Operation::List {
                path: list_path.clone(),
            },
            None,
        )
        .unwrap();
    workspace
        .accept(WireEnvelope::new(
            request.identity.clone(),
            request.request_id,
            Operation::Listed {
                path: list_path,
                entries: vec![DirectoryEntry {
                    path: RemotePath::new(b"/project/readme.txt".to_vec()).unwrap(),
                    kind: EntryKind::File,
                }],
                truncated: false,
                done: true,
            },
        ))
        .unwrap();
    model
        .attach_remote_model(id, workspace, false)
        .expect("remote model attaches without a host shell");
    let remote_id = model.active_workspace_id().unwrap();
    assert_ne!(remote_id, local_id);
    assert!(model.workspace(local_id).is_some());
    assert!(model.workspace(remote_id).unwrap().is_remote());
    assert!(model
        .workspace(remote_id)
        .unwrap()
        .terminal_target()
        .is_remote());

    let mut headless = Headless::new(model, 1280.0, 800.0);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    headless.advance(0.3);
    let output = headless.render();
    let text = output_text(&output);
    assert!(text.contains("EXPLORER"), "{text}");
    assert!(!text.contains("SSH connected"), "{text}");
    assert!(!text.contains("integration unknown"), "{text}");
    assert!(text.contains("readme.txt"), "{text}");
    let ssh = headless.response(top_action_id("ssh-servers"));
    assert!(ssh.rect.width() >= 24.0 && ssh.rect.height() >= 24.0);
    let explorer = headless
        .response(top_workspace_action_id(remote_id, "explorer"))
        .rect;
    assert!(
        ssh.rect.right() <= explorer.left() && explorer.right() > ssh.rect.right(),
        "Servidores SSH must sit beside Explorer: {:?} {explorer:?}",
        ssh.rect
    );
    let hovered =
        output_text(&headless.hover_until_text(top_action_id("ssh-servers"), "Servidores SSH"));
    assert!(hovered.contains("Servidores SSH"), "{hovered}");
    headless.click(top_action_id("open-folder"));
    let menu = output_text(&headless.render());
    assert!(menu.contains("Terminal"), "{menu}");
    assert!(menu.contains("Terminal SSH"), "{menu}");
    assert!(
        !menu.contains("OpenCode SSH"),
        "OpenCode SSH must wait for this server's detection: {menu}"
    );
}

#[test]
fn remote_opencode_tab_shows_local_label_icon_and_session_dots() {
    if isolated("remote_opencode_tab_shows_local_label_icon_and_session_dots") {
        return;
    }
    use made_app::remote_workspace::RemoteWorkspace;
    use made_app::ssh_servers::ServerMetadata;
    use made_bridge::state::SessionSnapshot;
    use made_remote::protocol::{Identity, Operation, RemotePath, TabCapability, WireEnvelope};

    let name = "remote-opencode-parity";
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let authority = model
        .add_ssh_metadata(ServerMetadata {
            display_name: name.into(),
            host: name.into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("server metadata saves");
    model
        .attach_remote_model(
            authority,
            RemoteWorkspace::new(
                Identity {
                    authority,
                    tab: TabCapability::random().unwrap(),
                    epoch: 1,
                    generation: 1,
                },
                RemotePath::new(b"/project".to_vec()).unwrap(),
            )
            .unwrap(),
            false,
        )
        .expect("remote model attaches");
    let remote_id = model.active_workspace_id().unwrap();
    let mut headless = Headless::new(model, 1280.0, 800.0);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    headless.render();
    let initial_output = headless.render();
    let initial_tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&initial_output, name, 55.0)
            .iter()
            .any(|rect| initial_tab.contains_rect(*rect)),
        "a remote shell tab without OpenCode keeps the server-name label"
    );
    assert!(
        tab_session_dots(&initial_output, initial_tab).is_empty(),
        "a remote shell tab without OpenCode paints no session dots"
    );

    // The real current-attach snapshot path: an unsolicited session snapshot
    // riding the attach request id connects the owning remote integration and
    // populates that tab's remote session map.
    let snapshot: SessionSnapshot = serde_json::from_value(json!({
        "revision": 1,
        "activeSessionID": "busy-1",
        "tabsEnabled": true,
        "sessions": [
            {
                "sessionID": "busy-1",
                "title": "Busy",
                "active": true,
                "busy": true,
                "directory": "/project"
            },
            {
                "sessionID": "atten-1",
                "title": "Attention",
                "active": false,
                "attention": true,
                "directory": "/project"
            },
            {
                "sessionID": "idle-1",
                "title": "Idle",
                "active": false,
                "directory": "/project"
            }
        ]
    }))
    .unwrap();
    {
        let workbench = headless.model.workspace_mut(remote_id).unwrap();
        let remote = workbench.remote_workspace_mut().unwrap();
        let identity = remote.identity().clone();
        remote
            .accept_session_event(WireEnvelope::new(
                identity,
                1,
                Operation::SessionSnapshot { snapshot },
            ))
            .expect("the current-attach session snapshot applies");
    }
    headless.pump_until(|model| {
        model
            .workspace(remote_id)
            .unwrap()
            .session_map()
            .rows()
            .len()
            == 3
    });
    // The extra pass settles the tab geometry after the branding switch.
    headless.render();

    let output = headless.render_events_at(Vec::new(), headless.last_time + 0.5);
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, "OpenCode", 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "a connected remote OpenCode tab must be labeled OpenCode"
    );
    assert!(
        text_rects(&output, name, 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "the server name must not be the tab label while the integration is connected"
    );
    let opencode_icon = icon_named("opencode.svg");
    let icon_image = painted_images(&output)
        .into_iter()
        .find(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && tab.contains_rect(image.rect)
        })
        .expect("a connected remote OpenCode tab must paint opencode.svg in its tab");
    assert_eq!(
        icon_image.texture,
        icon_texture(&headless.ctx, opencode_icon),
        "the tab must reuse the vendored opencode.svg bytes"
    );
    let dots = tab_session_dots(&output, tab);
    assert_eq!(
        dots.len(),
        3,
        "busy, attention and idle remote sessions each paint one dot"
    );
    for (index, dot) in dots.iter().enumerate() {
        assert!(
            (dot.center - session_dot_center(tab, index, 3)).length() < 0.75,
            "dots must be right-aligned in row order: {dot:?}"
        );
    }
    assert_session_dot_color(&dots[0], made_app::theme::ACCENT, "the busy dot");
    assert_session_dot_color(&dots[1], made_app::theme::WARNING, "the attention dot");
    assert_session_dot_color(&dots[2], made_app::theme::TEXT_TERTIARY, "the idle dot");

    headless.hover(workspace_tab_id(remote_id));
    headless.advance(0.2);
    let hovered = output_text(&headless.render());
    for expected in ["Sessions: 1 running", "1 needs attention", "1 idle"] {
        assert!(
            hovered.contains(expected),
            "the remote OpenCode tab tooltip must summarize its remote sessions: {hovered}"
        );
    }
}

#[test]
fn remote_opencode_direct_launch_keeps_branding_until_exit_or_disconnect() {
    if isolated("remote_opencode_direct_launch_keeps_branding_until_exit_or_disconnect") {
        return;
    }
    use made_app::remote_workspace::{IntegrationState as RemoteIntegrationState, RemoteWorkspace};
    use made_app::ssh_servers::ServerMetadata;
    use made_remote::protocol::{Identity, RemotePath, TabCapability};

    let name = "remote-opencode-direct";
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let authority = model
        .add_ssh_metadata(ServerMetadata {
            display_name: name.into(),
            host: name.into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("server metadata saves");
    model
        .attach_remote_model(
            authority,
            RemoteWorkspace::new(
                Identity {
                    authority,
                    tab: TabCapability::random().unwrap(),
                    epoch: 1,
                    generation: 1,
                },
                RemotePath::new(b"/project".to_vec()).unwrap(),
            )
            .unwrap(),
            true,
        )
        .expect("remote model attaches");
    let remote_id = model.active_workspace_id().unwrap();
    assert_eq!(
        model.workspace(remote_id).unwrap().terminal_target(),
        TerminalTarget::RemoteOpenCode { authority },
        "the direct launch must own the OpenCode SSH target"
    );
    let mut headless = Headless::new(model, 1280.0, 800.0);
    headless.render();

    // The directly launched remote OpenCode keeps the local OpenCode
    // presentation while its remote process is alive, even before the first
    // integration handshake.
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, "OpenCode", 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "a directly launched remote OpenCode tab keeps the OpenCode label before the first handshake"
    );
    let opencode_icon = icon_named("opencode.svg");
    let icon_image = painted_images(&output)
        .into_iter()
        .find(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && tab.contains_rect(image.rect)
        })
        .expect("the directly launched remote OpenCode tab must paint opencode.svg");
    assert_eq!(
        icon_image.texture,
        icon_texture(&headless.ctx, opencode_icon),
        "the tab must reuse the vendored opencode.svg bytes"
    );
    assert!(
        tab_session_dots(&output, tab).is_empty(),
        "no remote sessions are open yet, so no dots may paint"
    );

    // The remote OpenCode exits normally: the owning tab returns to the
    // server-name presentation and the shared session map stays empty.
    {
        let workbench = headless.model.workspace_mut(remote_id).unwrap();
        let remote = workbench.remote_workspace_mut().unwrap();
        let identity = remote.identity().clone();
        remote
            .integration_changed(&identity, RemoteIntegrationState::Unavailable)
            .expect("the remote integration change applies");
    }
    headless.pump_until(|model| {
        model
            .workspace(remote_id)
            .unwrap()
            .session_map()
            .rows()
            .is_empty()
    });
    // The extra pass settles the tab geometry after the presentation reverts.
    headless.render();
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, name, 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "the tab returns to the server-name presentation after the remote OpenCode exits"
    );
    assert!(
        text_rects(&output, "OpenCode", 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "no OpenCode label may survive the remote exit"
    );
    assert!(
        tab_session_dots(&output, tab).is_empty(),
        "stale session dots must clear with the remote exit"
    );
    assert_eq!(
        headless
            .model
            .workspace(remote_id)
            .unwrap()
            .terminal_target(),
        TerminalTarget::RemoteOpenCode { authority },
        "the exit must not restart or replace the remote terminal target"
    );

    // A connected integration restores the presentation, and a transport
    // disconnect reverts it again without touching the tab.
    {
        let workbench = headless.model.workspace_mut(remote_id).unwrap();
        let remote = workbench.remote_workspace_mut().unwrap();
        let identity = remote.identity().clone();
        remote
            .integration_changed(&identity, RemoteIntegrationState::Connected)
            .expect("the remote integration reconnect applies");
    }
    headless.render();
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, "OpenCode", 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "a connected remote integration restores the OpenCode presentation"
    );
    {
        let workbench = headless.model.workspace_mut(remote_id).unwrap();
        workbench.remote_workspace_mut().unwrap().disconnect();
    }
    headless.render();
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, name, 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "the tab returns to the server-name presentation after the transport disconnects"
    );
    assert!(
        tab_session_dots(&output, tab).is_empty(),
        "the disconnect must leave no session dots"
    );
}

#[test]
fn remote_grok_tab_shows_grok_build_branding_and_dots() {
    if isolated("remote_grok_tab_shows_grok_build_branding_and_dots") {
        return;
    }
    use made_app::remote_workspace::RemoteWorkspace;
    use made_app::ssh_servers::ServerMetadata;
    use made_remote::protocol::{
        GrokActivity, GrokSessionInfo, GrokSessionSnapshot, Identity, Operation, RemotePath,
        TabCapability, WireEnvelope,
    };

    let name = "remote-grok-parity";
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let authority = model
        .add_ssh_metadata(ServerMetadata {
            display_name: name.into(),
            host: name.into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("server metadata saves");
    // A remote shell tab where the user typed `grok` manually: the target is
    // the shell, so branding comes from owned sessions, not from a process.
    model
        .attach_remote_model(
            authority,
            RemoteWorkspace::new(
                Identity {
                    authority,
                    tab: TabCapability::random().unwrap(),
                    epoch: 1,
                    generation: 1,
                },
                RemotePath::new(b"/project".to_vec()).unwrap(),
            )
            .unwrap(),
            false,
        )
        .expect("remote model attaches");
    let remote_id = model.active_workspace_id().unwrap();
    let mut headless = Headless::new(model, 1280.0, 800.0);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    headless.render();

    let session = |id: &str, cwd: &str, opened_at: u64, activity: GrokActivity| GrokSessionInfo {
        session_id: id.to_owned(),
        cwd: Some(RemotePath::new(cwd.as_bytes().to_vec()).unwrap()),
        opened_at,
        activity,
    };
    let push = |headless: &mut Headless, snapshot: GrokSessionSnapshot| {
        let workbench = headless.model.workspace_mut(remote_id).unwrap();
        let remote = workbench.remote_workspace_mut().unwrap();
        let identity = remote.identity().clone();
        remote
            .accept_grok_push(WireEnvelope::new(
                identity,
                1,
                Operation::GrokSnapshot { snapshot },
            ))
            .expect("the current-attach Grok snapshot applies");
    };

    // A living sessionless grok process does not brand a shell tab: only the
    // tab's own managed grok process earns the label before any session.
    push(
        &mut headless,
        GrokSessionSnapshot {
            revision: 1,
            running: false,
            hooks_ready: false,
            sessions: vec![],
        },
    );
    headless.render();
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, name, 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "a remote shell tab stays on the server name until a Grok session opens"
    );
    assert!(
        tab_session_dots(&output, tab).is_empty(),
        "a remote shell tab paints no Grok dots before a session"
    );

    // One owned session is enough: the tab is branded Grok Build with the
    // official mark and the local dot treatment.
    push(
        &mut headless,
        GrokSessionSnapshot {
            revision: 2,
            running: false,
            hooks_ready: false,
            sessions: vec![session("a", "/grok-a", 1, GrokActivity::Idle)],
        },
    );
    headless.render();
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, "Grok Build", 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "an owned remote Grok session must brand the shell tab Grok Build"
    );
    assert!(
        text_rects(&output, name, 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "the server name must not be the label while a Grok session is open"
    );
    let grok_image = painted_images(&output)
        .into_iter()
        .find(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && tab.contains_rect(image.rect)
        })
        .expect("a branded remote Grok tab must paint the official grok.svg mark");
    let grok_px = (output.pixels_per_point * grok_image.rect.size()).round();
    assert_eq!(
        grok_image.texture,
        embedded_svg_texture(
            &headless.ctx,
            "bytes://grok.svg",
            include_bytes!("../assets/grok.svg"),
            grok_px.x as u32,
            grok_px.y as u32,
        ),
        "the leading slot must use grok.svg"
    );
    headless.hover(workspace_tab_id(remote_id));
    headless.advance(0.2);
    let hovered = output_text(&headless.render());
    assert!(
        hovered.contains("1 idle") && !hovered.contains("/project"),
        "a branded Grok tab tooltip summarizes sessions and omits the path: {hovered}"
    );

    // Owned sessions in three states paint the local dots in attachment
    // order: idle, working (busy tone), attention.
    push(
        &mut headless,
        GrokSessionSnapshot {
            revision: 3,
            running: false,
            hooks_ready: false,
            sessions: vec![
                session("a", "/grok-a", 1, GrokActivity::Idle),
                session("b", "/grok-b", 2, GrokActivity::Working),
                session("c", "/grok-c", 3, GrokActivity::Attention),
            ],
        },
    );
    // Move the pointer off the tab so the close reveal fades and the dots
    // paint at full strength, as they do without a pointer over the tab.
    headless.render_events(vec![egui::Event::PointerMoved(egui::pos2(
        4.0,
        headless.height - 4.0,
    ))]);
    headless.advance(0.3);
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    let dots = tab_session_dots(&output, tab);
    assert_eq!(dots.len(), 3, "each owned Grok session paints one dot");
    for (index, dot) in dots.iter().enumerate() {
        assert!(
            (dot.center - session_dot_center(tab, index, 3)).length() < 0.75,
            "dots must be right-aligned in attachment order: {dot:?}"
        );
    }
    assert_session_dot_color(&dots[0], made_app::theme::TEXT_TERTIARY, "the idle dot");
    assert_session_dot_color(&dots[1], made_app::theme::ACCENT, "the working dot");
    assert_session_dot_color(&dots[2], made_app::theme::WARNING, "the attention dot");
    headless.hover(workspace_tab_id(remote_id));
    headless.advance(0.2);
    let hovered = output_text(&headless.render());
    for expected in ["Sessions: 1 running", "1 needs attention", "1 idle"] {
        assert!(
            hovered.contains(expected),
            "the remote Grok tab tooltip must summarize its sessions: {hovered}"
        );
    }
    assert!(
        !hovered.contains("/project"),
        "a Grok Build tab tooltip must omit the workspace path: {hovered}"
    );
}

#[test]
fn remote_grok_direct_launch_brands_before_a_session_and_exits() {
    if isolated("remote_grok_direct_launch_brands_before_a_session_and_exits") {
        return;
    }
    use made_app::remote_workspace::RemoteWorkspace;
    use made_app::ssh_servers::ServerMetadata;
    use made_remote::protocol::{
        GrokSessionSnapshot, Identity, Operation, RemotePath, TabCapability, WireEnvelope,
    };

    let name = "remote-grok-direct";
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let authority = model
        .add_ssh_metadata(ServerMetadata {
            display_name: name.into(),
            host: name.into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("server metadata saves");
    model
        .attach_remote_grok_model(
            authority,
            RemoteWorkspace::new(
                Identity {
                    authority,
                    tab: TabCapability::random().unwrap(),
                    epoch: 1,
                    generation: 1,
                },
                RemotePath::new(b"/project".to_vec()).unwrap(),
            )
            .unwrap(),
        )
        .expect("remote model attaches");
    let remote_id = model.active_workspace_id().unwrap();
    assert_eq!(
        model.workspace(remote_id).unwrap().terminal_target(),
        TerminalTarget::RemoteGrok { authority },
        "the direct launch must own the Grok SSH target"
    );
    let mut headless = Headless::new(model, 1280.0, 800.0);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    headless.render();

    let push = |headless: &mut Headless, snapshot: GrokSessionSnapshot| {
        let workbench = headless.model.workspace_mut(remote_id).unwrap();
        let remote = workbench.remote_workspace_mut().unwrap();
        let identity = remote.identity().clone();
        remote
            .accept_grok_push(WireEnvelope::new(
                identity,
                1,
                Operation::GrokSnapshot { snapshot },
            ))
            .expect("the current-attach Grok snapshot applies");
    };

    // The tab's remote target is a living grok process: the label is
    // `Grok Build` with the official mark before any session exists.
    push(
        &mut headless,
        GrokSessionSnapshot {
            revision: 1,
            running: true,
            hooks_ready: false,
            sessions: vec![],
        },
    );
    headless.render();
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, "Grok Build", 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "a directly launched remote grok must be labeled Grok Build before any session"
    );
    assert!(
        text_rects(&output, name, 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "the server name must not be the label while the remote grok process runs"
    );
    let grok_image = painted_images(&output)
        .into_iter()
        .find(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && tab.contains_rect(image.rect)
        })
        .expect("a branded remote Grok tab must paint the official grok.svg mark");
    let grok_px = (output.pixels_per_point * grok_image.rect.size()).round();
    assert_eq!(
        grok_image.texture,
        embedded_svg_texture(
            &headless.ctx,
            "bytes://grok.svg",
            include_bytes!("../assets/grok.svg"),
            grok_px.x as u32,
            grok_px.y as u32,
        ),
        "the leading slot must use grok.svg"
    );
    assert!(
        tab_session_dots(&output, tab).is_empty(),
        "a sessionless Grok tab paints no dots"
    );
    headless.hover(workspace_tab_id(remote_id));
    headless.advance(0.2);
    let hovered = output_text(&headless.render());
    assert!(
        !hovered.contains("/project") && !hovered.contains("Sessions:"),
        "a sessionless Grok Build tab must omit the workspace path tooltip: {hovered}"
    );

    // When the grok process is gone and no session remains, the tab returns
    // to the server-name label and terminal glyph.
    push(
        &mut headless,
        GrokSessionSnapshot {
            revision: 2,
            running: false,
            hooks_ready: false,
            sessions: vec![],
        },
    );
    headless.render();
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, name, 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "an exited remote grok with no sessions restores the server-name label"
    );
    assert!(
        text_rects(&output, "Grok Build", 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "the Grok Build label must drop when the process exits with no sessions"
    );
    assert!(
        !painted_images(&output).into_iter().any(|image| {
            (image.rect.width() - 16.0).abs() < 0.75 && tab.contains_rect(image.rect)
        }),
        "the official grok mark must drop when the process exits"
    );
    assert!(
        tab_session_dots(&output, tab).is_empty(),
        "an exited remote grok leaves no stale dots"
    );
}

#[test]
fn remote_grok_exit_disconnect_and_opencode_precedence() {
    if isolated("remote_grok_exit_disconnect_and_opencode_precedence") {
        return;
    }
    use made_app::remote_workspace::{IntegrationState as RemoteIntegrationState, RemoteWorkspace};
    use made_app::ssh_servers::ServerMetadata;
    use made_remote::protocol::{
        GrokActivity, GrokSessionInfo, GrokSessionSnapshot, Identity, Operation, RemotePath,
        TabCapability, WireEnvelope,
    };

    let name = "remote-grok-exit";
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let authority = model
        .add_ssh_metadata(ServerMetadata {
            display_name: name.into(),
            host: name.into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("server metadata saves");
    model
        .attach_remote_model(
            authority,
            RemoteWorkspace::new(
                Identity {
                    authority,
                    tab: TabCapability::random().unwrap(),
                    epoch: 1,
                    generation: 1,
                },
                RemotePath::new(b"/project".to_vec()).unwrap(),
            )
            .unwrap(),
            false,
        )
        .expect("remote model attaches");
    let remote_id = model.active_workspace_id().unwrap();
    let mut headless = Headless::new(model, 1280.0, 800.0);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    headless.render();

    let push = |headless: &mut Headless, snapshot: GrokSessionSnapshot| {
        let workbench = headless.model.workspace_mut(remote_id).unwrap();
        let remote = workbench.remote_workspace_mut().unwrap();
        let identity = remote.identity().clone();
        remote
            .accept_grok_push(WireEnvelope::new(
                identity,
                1,
                Operation::GrokSnapshot { snapshot },
            ))
            .expect("the Grok snapshot applies");
    };
    let session = |id: &str, cwd: &str, opened_at: u64, activity: GrokActivity| GrokSessionInfo {
        session_id: id.to_owned(),
        cwd: Some(RemotePath::new(cwd.as_bytes().to_vec()).unwrap()),
        opened_at,
        activity,
    };
    let pointer_away = |headless: &mut Headless| {
        headless.render_events(vec![egui::Event::PointerMoved(egui::pos2(
            4.0,
            headless.height - 4.0,
        ))]);
        headless.advance(0.3);
    };

    push(
        &mut headless,
        GrokSessionSnapshot {
            revision: 1,
            running: false,
            hooks_ready: false,
            sessions: vec![session("a", "/grok-a", 1, GrokActivity::Working)],
        },
    );
    headless.render();
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, "Grok Build", 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "the fixture starts branded Grok Build with one open session"
    );

    // OpenCode keeps precedence: a connected remote integration owns the
    // label, icon and dots even while Grok sessions are open.
    {
        let workbench = headless.model.workspace_mut(remote_id).unwrap();
        let remote = workbench.remote_workspace_mut().unwrap();
        let identity = remote.identity().clone();
        remote
            .integration_changed(&identity, RemoteIntegrationState::Connected)
            .expect("the remote integration connects");
    }
    headless.render();
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, "OpenCode", 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "a connected remote OpenCode integration owns the tab presentation"
    );
    assert!(
        text_rects(&output, "Grok Build", 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "the Grok Build label must yield to the connected OpenCode integration"
    );
    push(
        &mut headless,
        GrokSessionSnapshot {
            revision: 2,
            running: true,
            hooks_ready: false,
            sessions: vec![session("a", "/grok-a", 1, GrokActivity::Working)],
        },
    );
    pointer_away(&mut headless);
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, "OpenCode", 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "OpenCode keeps the label even when Grok sessions exist"
    );
    let opencode_icon = icon_named("opencode.svg");
    let icon_image = painted_images(&output)
        .into_iter()
        .find(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && tab.contains_rect(image.rect)
        })
        .expect("the OpenCode tab must keep opencode.svg");
    assert_eq!(
        icon_image.texture,
        icon_texture(&headless.ctx, opencode_icon),
        "the tab must keep the vendored opencode.svg bytes"
    );
    assert!(
        tab_session_dots(&output, tab).is_empty(),
        "no OpenCode sessions are open, so Grok dots must not paint either"
    );

    // The integration becomes unavailable again: the owned Grok session
    // retakes the presentation above the shell fallback.
    {
        let workbench = headless.model.workspace_mut(remote_id).unwrap();
        let remote = workbench.remote_workspace_mut().unwrap();
        let identity = remote.identity().clone();
        remote
            .integration_changed(&identity, RemoteIntegrationState::Unavailable)
            .expect("the remote integration becomes unavailable");
    }
    pointer_away(&mut headless);
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, "Grok Build", 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "the open Grok session retakes the presentation when OpenCode leaves"
    );
    assert_eq!(
        tab_session_dots(&output, tab).len(),
        1,
        "the owned Grok session paints its dot again"
    );

    // The grok process exits and no session remains: the tab returns to the
    // server-name label and terminal glyph, with no stale dots.
    push(
        &mut headless,
        GrokSessionSnapshot {
            revision: 3,
            running: false,
            hooks_ready: false,
            sessions: vec![],
        },
    );
    pointer_away(&mut headless);
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, name, 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "an exited remote grok with no open sessions restores the server-name label"
    );
    assert!(
        text_rects(&output, "Grok Build", 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "the Grok Build label must drop when grok exits with no sessions"
    );
    assert!(
        !painted_images(&output).into_iter().any(|image| {
            (image.rect.width() - 16.0).abs() < 0.75 && tab.contains_rect(image.rect)
        }),
        "the official grok mark must drop when grok exits"
    );
    assert!(
        tab_session_dots(&output, tab).is_empty(),
        "an exited remote grok leaves no stale dots"
    );

    // A previous-attachment snapshot with a stale identity is discarded and
    // must not restore activity after the exit.
    {
        let workbench = headless.model.workspace_mut(remote_id).unwrap();
        let remote = workbench.remote_workspace_mut().unwrap();
        let mut stale = remote.identity().clone();
        stale.epoch = 9;
        assert!(
            remote
                .accept_grok_push(WireEnvelope::new(
                    stale,
                    1,
                    Operation::GrokSnapshot {
                        snapshot: GrokSessionSnapshot {
                            revision: 4,
                            running: true,
                            hooks_ready: false,
                            sessions: vec![session("a", "/grok-a", 1, GrokActivity::Working)],
                        },
                    },
                ))
                .is_err(),
            "a stale-epoch Grok snapshot must be discarded",
        );
    }
    pointer_away(&mut headless);
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        tab_session_dots(&output, tab).is_empty(),
        "stale-epoch Grok state must not repaint dots"
    );
    assert!(
        text_rects(&output, name, 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "discarded stale state keeps the server-name presentation"
    );

    // A transport disconnect clears Grok activity without touching the
    // terminal presentation.
    push(
        &mut headless,
        GrokSessionSnapshot {
            revision: 5,
            running: true,
            hooks_ready: false,
            sessions: vec![session("a", "/grok-a", 1, GrokActivity::Working)],
        },
    );
    headless.render();
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, "Grok Build", 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "the fixture is branded again before the transport disconnect"
    );
    {
        let workbench = headless.model.workspace_mut(remote_id).unwrap();
        workbench.remote_workspace_mut().unwrap().disconnect();
    }
    pointer_away(&mut headless);
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(remote_id)).rect;
    assert!(
        text_rects(&output, name, 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "the disconnect restores the server-name presentation"
    );
    assert!(
        tab_session_dots(&output, tab).is_empty(),
        "the disconnect clears stale Grok dots"
    );
}

#[test]
fn remote_open_prompt_refuses_changed_key_without_a_secret_field() {
    if isolated("remote_open_prompt_refuses_changed_key_without_a_secret_field") {
        return;
    }
    use made_app::remote_workspace::RemoteWorkspace;
    use made_app::ssh_servers::ServerMetadata;
    use made_remote::protocol::{Identity, RemotePath, TabCapability};

    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = model
        .add_ssh_metadata(ServerMetadata {
            display_name: "Changed".into(),
            host: "server-b".into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("metadata saves");
    let workspace = RemoteWorkspace::new(
        Identity {
            authority: id,
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        },
        RemotePath::new(b"/project".to_vec()).unwrap(),
    )
    .unwrap();
    model
        .attach_remote_model(id, workspace, false)
        .expect("remote model");
    let remote_id = model.active_workspace_id().unwrap();
    model
        .workspace_mut(remote_id)
        .unwrap()
        .present_open_failure(
            made_app::remote_session::SshFailureClass::ChangedHostKey,
            "the remote host key changed and was refused",
            Some("SHA256:must-not-appear"),
        );
    let mut headless = Headless::new(model, 1100.0, 700.0);
    headless.advance(0.2);
    let text = output_text(&headless.render());
    assert!(
        text.contains("the remote host key changed and was refused"),
        "changed key must use the static refusal: {text}"
    );
    assert!(
        !text.contains("SHA256:must-not-appear"),
        "a changed key must not show a fingerprint field: {text}"
    );
    assert!(
        !text.contains("Approve host"),
        "a changed key must not offer acceptance: {text}"
    );
    assert!(!text.contains("password"), "no secret field copy: {text}");
}

fn local_and_remote_headless(temp: &Path, name: &str) -> (Headless, WorkspaceId, WorkspaceId) {
    use made_app::remote_workspace::RemoteWorkspace;
    use made_app::ssh_servers::ServerMetadata;
    use made_remote::protocol::{Identity, RemotePath, TabCapability};

    assert_ssh_fixture_is_isolated("local_and_remote_headless");
    let root = fixture_workspace(temp, name, "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let local_id = open_workspace(&mut model, &root);
    let authority = model
        .add_ssh_metadata(ServerMetadata {
            display_name: name.into(),
            host: name.into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("metadata saves");
    let workspace = RemoteWorkspace::new(
        Identity {
            authority,
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        },
        RemotePath::new(b"/project".to_vec()).unwrap(),
    )
    .unwrap();
    model
        .attach_remote_model(authority, workspace, false)
        .expect("remote model attaches");
    let remote_id = model.active_workspace_id().unwrap();
    assert_ne!(remote_id, local_id);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, local_id);
    (headless, local_id, remote_id)
}

#[test]
fn ssh_prompt_answer_releases_hold_and_local_terminal_takes_each_frame_input() {
    if isolated("ssh_prompt_answer_releases_hold_and_local_terminal_takes_each_frame_input") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let (mut headless, local_id, remote_id) = local_and_remote_headless(temp.path(), "ssh-hold");
    assert!(headless
        .model
        .workspace_mut(remote_id)
        .unwrap()
        .present_remote_prompt("user@server's password:"));
    let field = ssh_field_id("prompt");
    for _ in 0..2 {
        headless.render();
        assert_eq!(
            headless.ctx.memory(|memory| memory.focused()),
            Some(field),
            "a live SSH prompt must keep its secret field focused"
        );
    }

    // Leave the prompt live but hidden for one pass so the end-pass plugin has
    // to reject its previous-pass response, then answer and type immediately.
    headless.model.switch_workspace(local_id);
    headless.render();
    headless
        .model
        .workspace_mut(remote_id)
        .unwrap()
        .submit_remote_auth(true);
    assert_terminal_takes_each_frame_input(&mut headless, local_id);
}

#[test]
fn ssh_prompt_cancel_and_tab_close_clear_the_field_focus_hold() {
    if isolated("ssh_prompt_cancel_and_tab_close_clear_the_field_focus_hold") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let (mut headless, local_id, remote_id) =
        local_and_remote_headless(temp.path(), "ssh-hold-close");
    let field = ssh_field_id("prompt");

    assert!(headless
        .model
        .workspace_mut(remote_id)
        .unwrap()
        .present_remote_prompt("user@server's password:"));
    for _ in 0..2 {
        headless.render();
    }
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(field),
        "a live SSH prompt must keep its secret field focused"
    );
    headless.model.switch_workspace(local_id);
    headless.render();
    headless
        .model
        .workspace_mut(remote_id)
        .unwrap()
        .submit_remote_auth(false);
    assert!(headless
        .model
        .workspace(remote_id)
        .unwrap()
        .remote_auth()
        .is_none());
    assert_terminal_takes_each_frame_input(&mut headless, local_id);

    headless.model.switch_workspace(remote_id);
    assert!(headless
        .model
        .workspace_mut(remote_id)
        .unwrap()
        .present_remote_prompt("user@server's password:"));
    for _ in 0..2 {
        headless.render();
    }
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(field),
        "the repainted prompt must hold its field again"
    );
    assert!(headless.model.close_workspace(remote_id));
    assert_terminal_takes_each_frame_input(&mut headless, local_id);
}

#[test]
fn ssh_prompt_hidden_passes_repaint_regains_focus_and_dismissal_is_stable() {
    if isolated("ssh_prompt_hidden_passes_repaint_regains_focus_and_dismissal_is_stable") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let (mut headless, local_id, remote_id) =
        local_and_remote_headless(temp.path(), "ssh-hold-repaint");
    let field = ssh_field_id("prompt");

    assert!(headless
        .model
        .workspace_mut(remote_id)
        .unwrap()
        .present_remote_prompt("user@server's password:"));
    for _ in 0..2 {
        headless.render();
    }
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(field),
        "a live SSH prompt must keep its secret field focused"
    );

    headless.click(workspace_tab_id(local_id));
    for _ in 0..3 {
        headless.render();
    }
    assert!(
        headless.ctx.read_response(field).is_none(),
        "the live prompt must stay unpainted for at least two completed passes"
    );

    headless.click(workspace_tab_id(remote_id));
    let mut regained = false;
    for _ in 0..4 {
        headless.render();
        if headless.ctx.memory(|memory| memory.focused()) == Some(field) {
            regained = true;
            break;
        }
    }
    assert!(
        regained,
        "a repainted live SSH prompt must regain its secret field focus without a click"
    );

    headless.model.switch_workspace(local_id);
    headless.render();
    headless
        .model
        .workspace_mut(remote_id)
        .unwrap()
        .submit_remote_auth(true);
    assert_terminal_takes_each_frame_input(&mut headless, local_id);
}

#[test]
fn ssh_prompt_repaint_keeps_another_live_field_focused() {
    if isolated("ssh_prompt_repaint_keeps_another_live_field_focused") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let (mut headless, local_id, remote_id) =
        local_and_remote_headless(temp.path(), "ssh-hold-other-field");
    let prompt = ssh_field_id("prompt");
    let name = ssh_field_id("name");

    assert!(headless
        .model
        .workspace_mut(remote_id)
        .unwrap()
        .present_remote_prompt("user@server's password:"));
    for _ in 0..2 {
        headless.render();
    }
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(prompt),
        "the first prompt paint must focus its secret field"
    );

    headless.click(workspace_tab_id(local_id));
    for _ in 0..3 {
        headless.render();
    }
    assert!(headless.ctx.read_response(prompt).is_none());

    headless.click(top_action_id("ssh-servers"));
    headless.render();
    headless.click(name);
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(name),
        "the SSH servers field must own focus before the prompt repaints"
    );

    headless.click(workspace_tab_id(remote_id));
    for _ in 0..3 {
        headless.render();
        assert_eq!(
            headless.ctx.memory(|memory| memory.focused()),
            Some(name),
            "a repainted prompt must not steal focus from another live SSH field"
        );
    }
    assert!(
        headless.ctx.read_response(prompt).is_some(),
        "the live prompt must actually repaint while the other field stays focused"
    );
}

fn assert_terminal_takes_each_frame_input(headless: &mut Headless, id: WorkspaceId) {
    let alphabet = "abcdefghijklmnopqrstuvwxyz";
    let mut focus_per_frame = Vec::new();
    for character in alphabet.chars() {
        headless.render_events(vec![egui::Event::Text(character.to_string())]);
        focus_per_frame.push(
            headless
                .model
                .workspace(id)
                .unwrap()
                .terminal()
                .unwrap()
                .is_focused(),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let echoed = loop {
        let flat: String = output_text(&headless.render())
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        if flat.contains(alphabet) || Instant::now() >= deadline {
            break flat;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(
        echoed.contains(alphabet),
        "the terminal dropped typed characters (focus per frame: {focus_per_frame:?}): {echoed}"
    );
    assert!(
        focus_per_frame.iter().all(|focused| *focused),
        "the terminal must keep focus on every frame while typing: {focus_per_frame:?}"
    );
}

#[test]
fn remote_open_unknown_host_shows_failure_fingerprint_and_requires_approval() {
    if isolated("remote_open_unknown_host_shows_failure_fingerprint_and_requires_approval") {
        return;
    }
    use made_app::remote_session::SshFailureClass;
    use made_app::remote_workspace::RemoteWorkspace;
    use made_app::ssh_servers::ServerMetadata;
    use made_remote::protocol::{Identity, RemotePath, TabCapability};

    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = model
        .add_ssh_metadata(ServerMetadata {
            display_name: "Unknown".into(),
            host: "server-c".into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("metadata saves");
    let workspace = RemoteWorkspace::new(
        Identity {
            authority: id,
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        },
        RemotePath::new(b"/project".to_vec()).unwrap(),
    )
    .unwrap();
    model
        .attach_remote_model(id, workspace, false)
        .expect("remote model");
    let remote_id = model.active_workspace_id().unwrap();
    model
        .workspace_mut(remote_id)
        .unwrap()
        .present_open_failure(
            SshFailureClass::UnknownHost,
            "the remote host key is not in the app trust file",
            Some("SHA256:abcDEF123"),
        );
    let mut headless = Headless::new(model, 1100.0, 700.0);
    headless.advance(0.2);
    let text = output_text(&headless.render());
    assert!(
        text.contains("the remote host key is not in the app trust file"),
        "unknown host must show the static detail: {text}"
    );
    assert!(
        text.contains("SHA256:abcDEF123"),
        "unknown host must show the failure fingerprint: {text}"
    );
    assert!(
        text.contains("Approve host"),
        "approval must be explicit: {text}"
    );
    assert!(!text.contains("password"), "no secret field: {text}");
}

#[test]
fn stage_error_does_not_spawn_the_companion() {
    if isolated("stage_error_does_not_spawn_the_companion") {
        return;
    }
    use made_app::remote_session::SshFailureClass;
    use made_app::remote_workspace::RemoteWorkspace;
    use made_app::ssh_servers::{AttemptError, ServerMetadata};
    use made_remote::protocol::{Identity, RemotePath, TabCapability};

    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = model
        .add_ssh_metadata(ServerMetadata {
            display_name: "Stage".into(),
            host: "server-d".into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("metadata saves");
    let workspace = RemoteWorkspace::new(
        Identity {
            authority: id,
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        },
        RemotePath::new(b"/project".to_vec()).unwrap(),
    )
    .unwrap();
    model
        .attach_remote_model(id, workspace, false)
        .expect("remote model");
    let remote_id = model.active_workspace_id().unwrap();
    model
        .workspace_mut(remote_id)
        .unwrap()
        .deliver_stage_result(Err(AttemptError {
            class: SshFailureClass::UnknownHost,
            detail: "the remote host key is not in the app trust file".to_owned(),
            fingerprint: Some("SHA256:stageOnly".to_owned()),
        }));
    let workbench = model.workspace(remote_id).unwrap();
    assert!(
        !workbench.companion_spawn_requested(),
        "a stage error must not start the companion"
    );
    assert!(workbench.remote_needs_host_approval());
    let mut headless = Headless::new(model, 1100.0, 700.0);
    headless.advance(0.2);
    let text = output_text(&headless.render());
    assert!(
        text.contains("Approve host"),
        "host approval must stay visible after a stage error: {text}"
    );
    assert!(text.contains("SHA256:stageOnly"), "{text}");
    assert!(
        !text.contains("Opening the authenticated session"),
        "stage failure must not continue into the companion: {text}"
    );

    headless
        .model
        .workspace_mut(remote_id)
        .unwrap()
        .deliver_stage_result(Err(AttemptError {
            class: SshFailureClass::ChangedHostKey,
            detail: "the remote host key changed and was refused".to_owned(),
            fingerprint: Some("SHA256:must-not-appear".to_owned()),
        }));
    let workbench = headless.model.workspace(remote_id).unwrap();
    assert!(!workbench.companion_spawn_requested());
    assert!(!workbench.remote_needs_host_approval());
    let text = output_text(&headless.render());
    assert!(
        text.contains("the remote host key changed and was refused"),
        "{text}"
    );
    assert!(!text.contains("Approve host"), "{text}");
    assert!(!text.contains("SHA256:must-not-appear"), "{text}");
}

#[test]
fn same_version_update_card_is_actionable_and_explorer_is_bounded() {
    if isolated("same_version_update_card_is_actionable_and_explorer_is_bounded") {
        return;
    }
    use made_app::remote_session::SshFailureClass;
    use made_app::remote_workspace::RemoteWorkspace;
    use made_app::ssh_servers::{AttemptError, ServerMetadata};
    use made_remote::protocol::{Identity, RemotePath, TabCapability};

    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = model
        .add_ssh_metadata(ServerMetadata {
            display_name: "Update".into(),
            host: "update-server".into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("metadata saves");
    let workspace = RemoteWorkspace::new(
        Identity {
            authority: id,
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        },
        RemotePath::new(b"/project".to_vec()).unwrap(),
    )
    .unwrap();
    model
        .attach_remote_model(id, workspace, false)
        .expect("remote model");
    let remote_id = model.active_workspace_id().unwrap();
    let workbench = model.workspace_mut(remote_id).unwrap();
    workbench.hold_remote_target("x86_64-unknown-linux-gnu");
    workbench.deliver_stage_result(Err(AttemptError {
        class: SshFailureClass::UpdateRequired,
        detail: "the installed companion must be updated with explicit approval".to_owned(),
        fingerprint: None,
    }));

    let mut headless = Headless::new(model, 1100.0, 700.0);
    headless.advance(0.2);
    let text = output_text(&headless.render());
    assert!(text.contains("Companion update"), "{text}");
    assert!(
        text.contains("the installed companion must be updated"),
        "the terminal card keeps the real reason: {text}"
    );
    assert!(text.contains("Update companion"), "{text}");
    assert!(
        text.contains(made_app::APP_VERSION),
        "the update card names the sanitized version: {text}"
    );
    assert!(
        text.contains("x86_64-unknown-linux-gnu"),
        "the update card names the sanitized target: {text}"
    );
    assert!(
        !text.contains("Approve companion"),
        "update approval is separate from initial staging consent: {text}"
    );
    assert!(
        !text.contains("Loading folder…"),
        "a pre-attach failure must not leave a perpetual loading explorer: {text}"
    );
    assert!(
        text.contains("Explorer unavailable"),
        "the explorer shows the bounded failure: {text}"
    );
    assert!(
        !text.contains("The explorer remains available"),
        "a failed pre-attach stage must not claim the explorer is usable: {text}"
    );
    assert!(
        !text.to_ascii_lowercase().contains("bump"),
        "no app-version bump advice is allowed: {text}"
    );
}

#[test]
fn header_plus_menu_lists_terminal_before_detected_tuis() {
    if isolated("header_plus_menu_lists_terminal_before_detected_tuis") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "menu", "main.rs");
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([OPENCODE]),
    );
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1100.0, 700.0);
    wait_explorer_ready(&mut headless, id);

    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    let hovered =
        output_text(&headless.hover_until_text(top_action_id("open-folder"), "Open folder"));
    assert!(
        hovered.contains("Open folder"),
        "Open folder tooltip missing while a TUI is detected: {hovered}"
    );

    headless.click(top_action_id("open-folder"));
    let output = headless.render();
    let terminal = headless.response(new_tab_menu_item_id("terminal"));
    let opencode = headless.response(new_tab_menu_item_id("opencode"));
    assert!(
        terminal.rect.top() < opencode.rect.top(),
        "Terminal must be the first menu item"
    );
    assert!(
        terminal.rect.width() >= 24.0 && terminal.rect.height() >= 24.0,
        "menu targets must stay at least 24x24: {:?}",
        terminal.rect
    );
    let text = output_text(&output);
    assert!(text.contains("Terminal"), "missing Terminal item: {text}");
    assert!(text.contains("OpenCode"), "missing OpenCode item: {text}");
    assert!(!headless.model.is_picker());
    assert_eq!(headless.model.workspace_count(), 1);
}

#[test]
fn new_tab_menu_terminal_item_runs_the_add_workspace_flow() {
    if isolated("new_tab_menu_terminal_item_runs_the_add_workspace_flow") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "terminal-menu", "main.rs");
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([OPENCODE]),
    );
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1100.0, 700.0);
    wait_explorer_ready(&mut headless, id);

    headless.click(top_action_id("open-folder"));
    headless.click(new_tab_menu_item_id("terminal"));
    assert!(headless.model.is_picker());
    assert_eq!(headless.model.workspace_count(), 1);
    assert!(headless.model.workspace(id).is_some());
}

#[test]
fn new_tab_menu_opencode_starts_a_home_tab_without_stopping_the_current_one() {
    if isolated("new_tab_menu_opencode_starts_a_home_tab_without_stopping_the_current_one") {
        return;
    }
    use made_platform::Platform;

    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "current", "main.rs");
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([OPENCODE]),
    );
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1100.0, 700.0);
    wait_explorer_ready(&mut headless, id);
    let home = made_platform::host_platform()
        .home_dir()
        .expect("test home")
        .canonicalize()
        .expect("canonical test home");

    headless.click(top_action_id("open-folder"));
    headless.click(new_tab_menu_item_id("opencode"));

    assert_eq!(headless.model.active_workspace_id(), Some(id));
    assert_eq!(headless.model.workspace_count(), 1);
    assert!(
        headless.model.start_pending(),
        "the new TUI tab must be scheduled for background preparation"
    );
    assert_eq!(
        headless.model.terminal_target(),
        TerminalTarget::Tui(OPENCODE)
    );
    assert_eq!(
        headless.model.launch().source(),
        WorkspaceSource::DefaultHome
    );
    assert_eq!(headless.model.launch().terminal_cwd(), Some(home.as_path()));
    assert!(headless
        .model
        .workspace(id)
        .and_then(Workbench::terminal)
        .is_some());
}

#[test]
fn new_tab_menu_items_are_keyboard_reachable_with_visible_focus() {
    if isolated("new_tab_menu_items_are_keyboard_reachable_with_visible_focus") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "keyboard-menu", "main.rs");
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([OPENCODE]),
    );
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1100.0, 700.0);
    wait_explorer_ready(&mut headless, id);

    headless.render();
    headless.focus(top_action_id("open-folder"));
    headless.press_key(egui::Key::Enter);
    headless.render();
    headless.press_key(egui::Key::ArrowDown);
    headless.render();
    // The popup fades in over `animation_time`; let the open animation settle so
    // the focus ring is painted at full opacity rather than mid-fade.
    headless.settle_animation();
    let terminal_id = new_tab_menu_item_id("terminal");
    let terminal = headless.response(terminal_id);
    assert!(
        terminal.has_focus(),
        "ArrowDown must focus the first menu item"
    );
    let focused = headless.render();
    let terminal = headless.response(terminal_id);
    assert!(
        !focus_ring_rects(&focused, terminal.rect).is_empty(),
        "the focused menu item must paint a visible focus ring"
    );

    headless.press_key(egui::Key::Enter);
    assert!(headless.model.is_picker());
}

#[test]
fn new_tab_menu_items_paint_leading_icons() {
    if isolated("new_tab_menu_items_paint_leading_icons") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "menu-icons", "main.rs");
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([OPENCODE]),
    );
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1100.0, 700.0);
    // At 1× this font's ink center lands on the galley line box, so centering
    // that box looks correct. 2× keeps the ink in the upper part of the box.
    headless.ctx.set_pixels_per_point(2.0);
    wait_explorer_ready(&mut headless, id);

    headless.click(top_action_id("open-folder"));
    // The menu fades in; settle so glyph colors are the label tone, not a fade multiply.
    headless.advance(0.3);
    let output = headless.render();
    let text = output_text(&output);
    assert!(text.contains("Terminal"), "missing Terminal item: {text}");
    assert!(text.contains("OpenCode"), "missing OpenCode item: {text}");

    let terminal = headless.response(new_tab_menu_item_id("terminal")).rect;
    let opencode = headless.response(new_tab_menu_item_id("opencode")).rect;
    assert!(
        terminal.top() < opencode.top(),
        "Terminal must stay the first menu item"
    );
    assert!(
        terminal.width() >= 24.0
            && terminal.height() >= 24.0
            && opencode.width() >= 24.0
            && opencode.height() >= 24.0,
        "menu targets must stay at least 24x24: {terminal:?} {opencode:?}"
    );
    assert!(
        (terminal.left() - opencode.left()).abs() < 0.75
            && (terminal.width() - opencode.width()).abs() < 0.75,
        "both rows must fill the same menu width: {terminal:?} {opencode:?}"
    );

    let terminal_label = painted_menu_label(&output, "Terminal", terminal);
    let opencode_label = painted_menu_label(&output, "OpenCode", opencode);
    assert!(
        (terminal_label.line.left() - opencode_label.line.left()).abs() < 1.0,
        "labels must share the same left inset: {:?} {:?}",
        terminal_label.line,
        opencode_label.line
    );
    // 8px row inset + 14px slot + 8px gap.
    assert!(
        (terminal_label.line.left() - (terminal.left() + 30.0)).abs() < 1.0,
        "the 14px slot keeps an 8px gap before the label: {:?} in {terminal:?}",
        terminal_label.line
    );
    let terminal_baseline = terminal_label.baseline_y - terminal.center().y;
    let opencode_baseline = opencode_label.baseline_y - opencode.center().y;
    assert!(
        (terminal_baseline - opencode_baseline).abs() < 0.2,
        "Terminal and OpenCode must share one baseline: {terminal_baseline} vs {opencode_baseline}"
    );
    assert!(
        opencode_label.ink.bottom() > terminal_label.ink.bottom() + 0.5,
        "OpenCode keeps the p descender below Terminal's ink"
    );
    for (row, label) in [(terminal, &terminal_label), (opencode, &opencode_label)] {
        assert!(
            label.line.left() > row.left() + 16.0 && label.line.right() < row.right() - 4.0,
            "label must be left-aligned inside the row with right padding: {:?} {row:?}",
            label.line
        );
        let body_y = letterform_center_y(label);
        assert!(
            (body_y - row.center().y).abs() < 0.2,
            "letterforms must sit on the icon, not in the upper part of the galley box: body {body_y} ink {:?} baseline {} row {row:?}",
            label.ink,
            label.baseline_y
        );
        assert!(
            label.line.center().y > row.center().y + 0.15,
            "centering the galley line box leaves the ink above the icon: line {:?} row {row:?}",
            label.line
        );
    }

    let window = rect_shapes(&output)
        .into_iter()
        .find(|shape| {
            (shape.rect.width() - 14.0).abs() < 0.75
                && (shape.rect.height() - 11.0).abs() < 0.75
                && (shape.stroke.width - 1.5).abs() < 0.05
                && shape.stroke.color == made_app::theme::TEXT_TERTIARY
                && shape.corner_radius == egui::CornerRadius::same(2)
                && terminal.contains_rect(shape.rect)
                && shape.rect.right() <= terminal_label.line.left() - 2.0
        })
        .map(|shape| shape.rect)
        .expect("terminal window glyph");
    assert!(
        (window.center().y - terminal.center().y).abs() < 1.0
            && (window.left() - (terminal.left() + 8.0)).abs() < 1.0,
        "terminal glyph must sit in the left slot: {window:?} {terminal:?}"
    );
    let terminal_paths: Vec<_> = path_shapes(&output)
        .into_iter()
        .filter(|path| {
            let bounds = egui::Rect::from_points(&path.points);
            terminal.contains_rect(bounds) && bounds.right() <= terminal_label.line.left() - 1.0
        })
        .collect();
    assert_eq!(
        terminal_paths.len(),
        2,
        "terminal glyph is a chevron plus cursor, not a sparkle: {terminal_paths:?}"
    );
    let chevron = terminal_paths
        .iter()
        .find(|path| path.points.len() == 3)
        .expect("prompt chevron");
    assert!(
        chevron.points[1].x > chevron.points[0].x + 1.0
            && chevron.points[1].x > chevron.points[2].x + 1.0
            && (chevron.points[0].x - chevron.points[2].x).abs() < 0.5,
        "chevron must point right: {:?}",
        chevron.points
    );
    let cursor = terminal_paths
        .iter()
        .find(|path| path.points.len() == 2)
        .expect("prompt cursor");
    assert!(
        (cursor.points[0].y - cursor.points[1].y).abs() < 0.5
            && cursor.points[1].x > cursor.points[0].x + 1.0,
        "cursor must be a short underscore: {:?}",
        cursor.points
    );
    for path in [&chevron, &cursor] {
        assert!(
            window.contains_rect(egui::Rect::from_points(&path.points)),
            "prompt strokes must sit inside the terminal window"
        );
        assert!(
            (path.stroke.width - 1.5).abs() < 0.05
                && matches!(
                    path.stroke.color,
                    egui::epaint::ColorMode::Solid(color)
                        if color == made_app::theme::TEXT_TERTIARY
                ),
            "prompt strokes must use the 1.5px tertiary stroke"
        );
    }

    let opencode_icon = explorer_icons::icon_by_file_name("opencode.svg")
        .expect("opencode.svg is vendored with the explorer icons");
    let opencode_image = painted_images(&output)
        .into_iter()
        .find(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && opencode.contains_rect(image.rect)
                && image.rect.right() <= opencode_label.line.left() - 1.0
        })
        .expect("OpenCode row must paint opencode.svg left of the label");
    let image_px = (output.pixels_per_point * opencode_image.rect.size()).round();
    assert!(
        opencode_label.line.left() - opencode_image.rect.right() >= 6.0 - 0.75,
        "the 16px image keeps at least 6px before the label: {:?} vs {:?}",
        opencode_image.rect,
        opencode_label.line
    );
    assert_eq!(
        opencode_image.texture,
        icon_texture_px(
            &headless.ctx,
            opencode_icon,
            image_px.x as u32,
            image_px.y as u32,
        ),
        "the menu must use the embedded opencode.svg bytes"
    );
    assert_eq!(
        opencode_image.tint,
        egui::Color32::WHITE,
        "the SVG keeps its shipped colors"
    );
    assert!(
        (opencode_image.rect.center().y - opencode.center().y).abs() < 1.0,
        "the SVG must be vertically centered in the row: {:?}",
        opencode_image.rect
    );
    for other in [
        "opencode_light.svg",
        "folder-opencode.svg",
        "folder-opencode-open.svg",
    ] {
        assert_ne!(
            opencode_image.texture,
            icon_texture_px(
                &headless.ctx,
                icon_named(other),
                image_px.x as u32,
                image_px.y as u32,
            ),
            "the menu must not use {other}"
        );
    }

    let hovered = headless.hover(new_tab_menu_item_id("terminal"));
    let hovered_row = headless.response(new_tab_menu_item_id("terminal")).rect;
    assert!(
        rect_shapes(&hovered).iter().any(|shape| {
            shape.fill == made_app::theme::BG_HOVER
                && (shape.rect.left() - hovered_row.left()).abs() < 1.0
                && (shape.rect.right() - hovered_row.right()).abs() < 1.0
                && (shape.rect.top() - hovered_row.top()).abs() < 1.0
                && (shape.rect.bottom() - hovered_row.bottom()).abs() < 1.0
        }),
        "hover fill must cover the full Terminal row: {hovered_row:?}"
    );
    assert!(
        rect_shapes(&hovered).iter().any(|shape| {
            (shape.rect.width() - 14.0).abs() < 0.75
                && (shape.rect.height() - 11.0).abs() < 0.75
                && shape.stroke.color == made_app::theme::TEXT_PRIMARY
                && hovered_row.contains_rect(shape.rect)
        }),
        "hovered terminal glyph must use the primary label color"
    );
}

#[test]
fn file_preview_roundtrip_keeps_the_live_terminal_and_blocks_hidden_input() {
    if isolated("file_preview_roundtrip_keeps_the_live_terminal_and_blocks_hidden_input") {
        return;
    }
    if !std::path::Path::new("/bin/bash").exists() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "preview", "note.txt");
    let marker = temp.path().join("hidden-input-marker");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.pump_until(|model| {
        model
            .workspace(id)
            .and_then(Workbench::terminal)
            .is_some_and(made_app::TerminalPane::is_running)
    });
    type_terminal_command(
        headless.model.workspace_mut(id).unwrap(),
        "while :; do echo tick; sleep 0.01; done",
    );
    headless.render();
    let file_row = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .row_widget_id(0);
    headless.click(file_row);
    assert_eq!(
        headless.model.workspace(id).unwrap().left_surface(),
        LeftSurface::Preview
    );
    let preview_path = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .preview_path()
        .unwrap()
        .to_path_buf();
    assert_eq!(preview_path, root.join("note.txt"));
    headless.pump_until(|model| {
        model
            .workspace(id)
            .and_then(Workbench::explorer)
            .and_then(made_app::ExplorerPane::preview_document)
            .is_some()
    });
    let output = headless.render();
    let text = output_text(&output);
    assert!(text.contains("PREVIEW"));
    assert!(text.contains("read-only"));
    assert!(text.contains("Close"));
    let workbench = headless.model.workspace(id).unwrap();
    let pid = workbench.terminal().unwrap().pid();
    let before_output = workbench.terminal().unwrap().output_bytes();
    let before_paints = workbench.terminal().unwrap().grid_paint_count();
    assert!(!workbench.terminal().unwrap().is_focused());
    headless.render();
    assert_eq!(
        headless
            .model
            .workspace(id)
            .unwrap()
            .terminal()
            .unwrap()
            .grid_paint_count(),
        before_paints,
        "hidden terminal must not paint its grid"
    );
    headless.pump_until(|model| {
        model
            .workspace(id)
            .and_then(Workbench::terminal)
            .is_some_and(|terminal| terminal.output_bytes() > before_output)
    });

    let at_leak = headless
        .model
        .workspace(id)
        .unwrap()
        .terminal()
        .unwrap()
        .output_bytes();
    headless.render_events(typed_events(&format!(
        "printf leaked > '{}'\n",
        marker.display()
    )));
    // Pump until the shell has consumed fresh output after the keystrokes:
    // the same passes that would deliver a leak have run, so the marker's
    // absence below is proven, not timed.
    headless.pump_until(|model| {
        model
            .workspace(id)
            .and_then(Workbench::terminal)
            .is_some_and(|terminal| terminal.output_bytes() > at_leak)
    });
    assert!(
        !marker.exists(),
        "preview keystrokes must never reach the hidden shell"
    );

    let pane_id = headless.model.workspace(id).unwrap().pane_id().to_owned();
    headless.click(top_workspace_action_id(id, "explorer"));
    assert!(!headless.model.workspace(id).unwrap().explorer_visible());
    headless.settle_animation();
    assert_eq!(
        headless.model.workspace(id).unwrap().left_surface(),
        LeftSurface::Preview
    );
    assert!(headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .preview_path()
        .is_some());
    assert!(!output_text(&headless.render()).contains("EXPLORER"));
    headless.click(top_workspace_action_id(id, "explorer"));
    assert!(headless.model.workspace(id).unwrap().explorer_visible());
    headless.settle_animation();
    assert!(output_text(&headless.render()).contains("EXPLORER"));
    let terminal_id = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .preview_terminal_id();
    headless.click(terminal_id);
    assert_eq!(
        headless.model.workspace(id).unwrap().left_surface(),
        LeftSurface::Terminal
    );
    for _ in 0..4 {
        headless.render();
        if headless
            .model
            .workspace(id)
            .unwrap()
            .terminal()
            .unwrap()
            .is_focused()
        {
            break;
        }
    }
    assert!(headless
        .model
        .workspace(id)
        .unwrap()
        .terminal()
        .unwrap()
        .is_focused());
    assert!(headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .preview_path()
        .is_some());
    assert_eq!(
        headless
            .model
            .workspace(id)
            .unwrap()
            .terminal()
            .unwrap()
            .pid(),
        pid
    );

    headless.click(left_surface_action_id(&pane_id, "file"));
    assert_eq!(
        headless.model.workspace(id).unwrap().left_surface(),
        LeftSurface::Preview
    );
    assert!(headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .preview_document()
        .is_some());
    let preview_focus = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .preview_focus_id();
    headless.render();
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(preview_focus),
        "File affordance must give the preview explicit focus"
    );
    let terminal_id = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .preview_terminal_id();
    headless.click(terminal_id);
    let selected_file = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .row_widget_id(0);
    headless.click(selected_file);
    assert_eq!(
        headless.model.workspace(id).unwrap().left_surface(),
        LeftSurface::Preview,
        "clicking the selected file must reopen the loaded preview"
    );
    let close_id = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .preview_action_id(PreviewAction::Close);
    headless.click(close_id);
    assert_eq!(
        headless.model.workspace(id).unwrap().left_surface(),
        LeftSurface::Terminal
    );
    assert!(headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .preview_path()
        .is_none());
    assert_eq!(
        headless
            .model
            .workspace(id)
            .unwrap()
            .terminal()
            .unwrap()
            .pid(),
        pid
    );
}

#[test]
fn primary_screen_terminal_card_uses_glass_fill() {
    if isolated("primary_screen_terminal_card_uses_glass_fill") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "terminal-glass", "notes.txt");
    for (width, height) in [(1280.0, 800.0), (720.0, 480.0)] {
        let mut model =
            AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
        let id = open_workspace(&mut model, &root);
        let mut headless = Headless::new(model, width, height);
        wait_explorer_ready(&mut headless, id);
        let output = headless.render();
        let glass = rects_with_fill(&output, made_app::theme::BG_TERMINAL_GLASS);
        assert_eq!(
            glass.len(),
            1,
            "{width}x{height} primary terminal must paint exactly one glass layer: {glass:?}"
        );
        let card = glass[0];
        assert!(
            card.width() > 300.0 && card.height() > 200.0,
            "{width}x{height} terminal card must occupy the left surface: {card:?}"
        );
        assert!(
            card.left() < 200.0,
            "{width}x{height} terminal card must be on the left: {card:?}"
        );
        headless.model.shutdown();
    }
}

#[cfg(unix)]
#[test]
fn alternate_screen_terminal_surface_is_opaque_and_has_no_glass() {
    use std::os::unix::fs::PermissionsExt;

    if isolated("alternate_screen_terminal_surface_is_opaque_and_has_no_glass") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "terminal-alt", "notes.txt");
    let shell = temp.path().join("alternate-shell");
    fs::write(
        &shell,
        "#!/bin/sh\nprintf '\\033[?1049h'\nwhile :; do sleep 1; done\n",
    )
    .unwrap();
    fs::set_permissions(&shell, fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var(made_platform::SHELL_ENV, &shell);

    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    headless.pump_until(|model| {
        model
            .workspace(id)
            .and_then(Workbench::terminal)
            .is_some_and(|terminal| terminal.alternate_screen())
    });
    let output = headless.render();
    let glass = rects_with_fill(&output, made_app::theme::BG_TERMINAL_GLASS);
    assert!(
        glass.is_empty(),
        "alternate-screen terminal must not paint a glass layer: {glass:?}"
    );
    let opaque = rects_with_fill(&output, made_app::theme::BG_TERMINAL);
    assert!(
        opaque
            .iter()
            .any(|rect| rect.width() > 500.0 && rect.height() > 300.0),
        "alternate-screen terminal card/body must be opaque BG_TERMINAL: {opaque:?}"
    );
    headless.model.shutdown();
}

#[test]
fn explorer_glass_uses_native_tokens_only_when_backdrop_is_applied() {
    if isolated("explorer_glass_uses_native_tokens_only_when_backdrop_is_applied") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "explorer-glass", "notes.txt");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);

    let fallback = headless.render();
    assert!(largest_rect(&fallback, made_app::theme::BG_CARD_GLASS).is_some());
    assert!(largest_rect(&fallback, made_app::theme::BG_HEADER_GLASS).is_some());
    assert!(largest_rect(&fallback, made_app::theme::BG_CARD_GLASS_NATIVE).is_none());
    assert!(largest_rect(&fallback, made_app::theme::BG_HEADER_GLASS_NATIVE).is_none());

    for (kind, reason) in [
        (NativeBackdropKind::MacosVibrancy, "test macOS vibrancy"),
        (NativeBackdropKind::WindowsAcrylic, "test Windows Acrylic"),
    ] {
        headless.model.set_native_backdrop(NativeBackdropStatus {
            kind,
            applied: true,
            reason: reason.to_owned(),
        });
        let output = headless.render();
        assert!(
            largest_rect(&output, made_app::theme::BG_CARD_GLASS_NATIVE).is_some(),
            "{kind:?} applied must paint the native Explorer card tint"
        );
        assert!(
            largest_rect(&output, made_app::theme::BG_HEADER_GLASS_NATIVE).is_some(),
            "{kind:?} applied must paint the native Explorer header tint"
        );
        assert!(
            largest_rect(&output, made_app::theme::BG_CARD_GLASS).is_none(),
            "{kind:?} applied must not paint the fallback Explorer card"
        );
        assert!(
            largest_rect(&output, made_app::theme::BG_HEADER_GLASS).is_none(),
            "{kind:?} applied must not paint the fallback Explorer header"
        );
    }

    headless
        .model
        .set_native_backdrop(NativeBackdropStatus::default());
    let fallback = headless.render();
    assert!(largest_rect(&fallback, made_app::theme::BG_CARD_GLASS).is_some());
    assert!(largest_rect(&fallback, made_app::theme::BG_HEADER_GLASS).is_some());
    assert!(largest_rect(&fallback, made_app::theme::BG_CARD_GLASS_NATIVE).is_none());
    assert!(largest_rect(&fallback, made_app::theme::BG_HEADER_GLASS_NATIVE).is_none());
    headless.model.shutdown();
}

#[test]
fn explorer_hide_restore_and_resize_keep_state_and_reclaim_width() {
    if isolated("explorer_hide_restore_and_resize_keep_state_and_reclaim_width") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "layout", "visible.txt");
    fs::create_dir(root.join("sub")).unwrap();
    fs::write(root.join("sub/nested.txt"), "nested\n").unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    let output = headless.render();
    let visible_terminal =
        largest_rect(&output, made_app::theme::BG_TERMINAL_GLASS).expect("terminal card painted");
    let explorer_width = headless.model.workspace(id).unwrap().explorer_width();
    assert!(explorer_width >= made_app::theme::EXPLORER_MIN_WIDTH);

    let directory_row = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .row_widget_id(0);
    headless.click(directory_row);
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut expanded_text = String::new();
    while Instant::now() < deadline {
        expanded_text = output_text(&headless.render());
        if expanded_text.contains("nested.txt") {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        expanded_text.contains("nested.txt"),
        "directory did not expand: {expanded_text}"
    );
    assert_eq!(
        headless
            .model
            .workspace(id)
            .unwrap()
            .explorer()
            .unwrap()
            .selected_path()
            .map(Path::to_path_buf),
        Some(root.join("sub"))
    );

    headless.click(top_workspace_action_id(id, "explorer"));
    assert!(!headless.model.workspace(id).unwrap().explorer_visible());
    headless.settle_animation();
    assert!(
        headless
            .model
            .workspace(id)
            .unwrap()
            .terminal()
            .unwrap()
            .is_focused(),
        "hiding the explorer must not steal terminal keyboard focus"
    );
    let hidden = headless.render();
    let hidden_text = output_text(&hidden);
    assert!(!hidden_text.contains("EXPLORER"));
    let hidden_terminal = largest_rect(&hidden, made_app::theme::BG_TERMINAL_GLASS)
        .expect("terminal card painted while hidden");
    assert!(
        hidden_terminal.width() > visible_terminal.width() + 300.0,
        "hiding the explorer must reclaim left width"
    );

    headless.click(top_workspace_action_id(id, "explorer"));
    assert!(headless.model.workspace(id).unwrap().explorer_visible());
    headless.settle_animation();
    let restored = headless.render();
    let restored_text = output_text(&restored);
    assert!(restored_text.contains("EXPLORER"));
    assert!(restored_text.contains("visible.txt"));
    assert!(
        restored_text.contains("nested.txt"),
        "expansion must survive hide/restore: {restored_text}"
    );
    assert_eq!(
        headless
            .model
            .workspace(id)
            .unwrap()
            .explorer()
            .unwrap()
            .selected_path(),
        Some(root.join("sub").as_path()),
        "selection must survive hide/restore"
    );

    let splitter = explorer_splitter_id(headless.model.workspace(id).unwrap().pane_id());
    let before_drag = headless.model.workspace(id).unwrap().explorer_width();
    headless.drag(splitter, egui::vec2(-20.0, 0.0));
    let after_drag = headless.model.workspace(id).unwrap().explorer_width();
    assert!(
        after_drag > before_drag,
        "dragging the splitter left must widen the explorer"
    );
    assert!(
        (after_drag - 380.0).abs() < 1.0,
        "the deliberate 380px drag width must survive away from the default sentinels: {after_drag}"
    );
    headless.render();
    headless.click(top_workspace_action_id(id, "explorer"));
    headless.render();
    headless.click(top_workspace_action_id(id, "explorer"));
    assert_eq!(
        headless.model.workspace(id).unwrap().explorer_width(),
        after_drag,
        "hide/restore must retain the chosen width"
    );
}

#[test]
fn each_workspace_keeps_its_own_preview_width_and_surface() {
    if isolated("each_workspace_keeps_its_own_preview_width_and_surface") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let alpha = fixture_workspace(temp.path(), "alpha", "a.txt");
    let beta = fixture_workspace(temp.path(), "beta", "b.txt");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let alpha_id = open_workspace(&mut model, &alpha);
    let beta_id = open_workspace(&mut model, &beta);
    headless_set_width(&mut model, alpha_id, 333.0);
    headless_set_width(&mut model, beta_id, 300.0);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, alpha_id);
    wait_explorer_ready(&mut headless, beta_id);

    headless.click(workspace_tab_id(alpha_id));
    headless.render();
    let row = headless
        .model
        .workspace(alpha_id)
        .unwrap()
        .explorer()
        .unwrap()
        .row_widget_id(0);
    headless.click(row);
    assert_eq!(
        headless.model.workspace(alpha_id).unwrap().left_surface(),
        LeftSurface::Preview
    );

    headless.click(workspace_tab_id(beta_id));
    let beta_workbench = headless.model.workspace(beta_id).unwrap();
    assert_eq!(beta_workbench.left_surface(), LeftSurface::Terminal);
    assert!(beta_workbench.explorer().unwrap().preview_path().is_none());
    assert_eq!(beta_workbench.explorer_width(), 300.0);

    let alpha_pid = headless
        .model
        .workspace(alpha_id)
        .unwrap()
        .terminal()
        .unwrap()
        .pid();
    let beta_pid = headless
        .model
        .workspace(beta_id)
        .unwrap()
        .terminal()
        .unwrap()
        .pid();
    headless
        .model
        .workspace_mut(beta_id)
        .unwrap()
        .set_explorer_visible(false);
    headless.click(workspace_tab_id(alpha_id));
    let alpha_workbench = headless.model.workspace(alpha_id).unwrap();
    assert_eq!(alpha_workbench.left_surface(), LeftSurface::Preview);
    assert_eq!(alpha_workbench.explorer_width(), 333.0);
    assert!(alpha_workbench.explorer_visible());
    assert_eq!(
        alpha_workbench.explorer().unwrap().preview_path().unwrap(),
        alpha.join("a.txt")
    );
    assert_eq!(alpha_workbench.terminal().unwrap().pid(), alpha_pid);
    let preview_focus = alpha_workbench.explorer().unwrap().preview_focus_id();
    headless.render();
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(preview_focus),
        "restored preview must receive explicit focus"
    );
    headless.click(workspace_tab_id(beta_id));
    let beta_workbench = headless.model.workspace(beta_id).unwrap();
    assert_eq!(beta_workbench.explorer_width(), 300.0);
    assert!(!beta_workbench.explorer_visible());
    assert_eq!(beta_workbench.terminal().unwrap().pid(), beta_pid);
    let alpha_scope = headless
        .model
        .workspace(alpha_id)
        .unwrap()
        .explorer()
        .unwrap()
        .row_widget_id(0);
    let beta_scope = headless
        .model
        .workspace(beta_id)
        .unwrap()
        .explorer()
        .unwrap()
        .row_widget_id(0);
    assert_ne!(
        alpha_scope, beta_scope,
        "explorer widget ids must be workspace-scoped"
    );
}

#[test]
fn explorer_animation_slides_without_reflow_and_stops_capturing_when_closed() {
    if isolated("explorer_animation_slides_without_reflow_and_stops_capturing_when_closed") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "animation", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    let output = headless.render();
    let pane = headless.model.workspace(id).unwrap().pane_id().to_owned();
    let splitter = explorer_splitter_id(&pane);
    let row = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .row_widget_id(0);
    let full_left = headless.response(splitter).rect.left();
    let full_card =
        largest_rect(&output, made_app::theme::BG_CARD_GLASS).expect("explorer glass card");

    headless
        .model
        .workspace_mut(id)
        .unwrap()
        .set_explorer_visible(false);
    let mut closing = None;
    let mut frame_time = headless.last_time;
    for _ in 0..40 {
        frame_time += 1.0 / 60.0;
        let output = headless.render_events_at(Vec::new(), frame_time);
        if let Some(response) = headless.ctx.read_response(splitter) {
            let left = response.rect.left();
            if left > full_left + 4.0 && left < 1270.0 {
                closing = Some(output);
                break;
            }
        } else {
            break;
        }
    }
    let closing = closing.expect("closing panel should expose an intermediate width frame");
    let closing_card =
        largest_rect(&closing, made_app::theme::BG_CARD_GLASS).expect("closing glass card");
    assert!(
        (closing_card.width() - full_card.width()).abs() < 2.0,
        "explorer content must not reflow during the slide: {:?} vs {:?}",
        closing_card,
        full_card
    );

    headless.settle_animation();
    let closed = headless.render();
    assert!(
        headless.ctx.read_response(splitter).is_none(),
        "closed animation must remove the splitter"
    );
    assert!(
        headless.ctx.read_response(row).is_none(),
        "closed animation must remove tree row input"
    );
    assert!(!output_text(&closed).contains("EXPLORER"));

    headless
        .model
        .workspace_mut(id)
        .unwrap()
        .set_explorer_visible(true);
    let mut opening = None;
    let mut frame_time = headless.last_time;
    for _ in 0..60 {
        frame_time += 1.0 / 60.0;
        let output = headless.render_events_at(Vec::new(), frame_time);
        if let Some(response) = headless.ctx.read_response(splitter) {
            let left = response.rect.left();
            if left > full_left + 4.0 && left < 1270.0 {
                opening = Some(output);
                break;
            }
        }
    }
    let opening = opening.expect("opening panel should expose an intermediate width frame");
    let opening_card =
        largest_rect(&opening, made_app::theme::BG_CARD_GLASS).expect("opening glass card");
    assert!((opening_card.width() - full_card.width()).abs() < 2.0);

    headless.settle_animation();
    let opened = headless.render();
    assert!(
        (headless.response(splitter).rect.left() - full_left).abs() < 2.0,
        "opening animation must finish at the stored width"
    );
    assert!(output_text(&opened).contains("EXPLORER"));

    // Interrupt a close with a show; the final state must still settle open.
    headless
        .model
        .workspace_mut(id)
        .unwrap()
        .set_explorer_visible(false);
    headless.render();
    std::thread::sleep(Duration::from_millis(40));
    headless.render();
    headless
        .model
        .workspace_mut(id)
        .unwrap()
        .set_explorer_visible(true);
    headless.render();
    headless.settle_animation();
    assert!(
        (headless.response(splitter).rect.left() - full_left).abs() < 2.0,
        "repeated toggle must settle at the full width"
    );
    let settled = headless.render();
    let settled_card =
        largest_rect(&settled, made_app::theme::BG_CARD_GLASS).expect("settled glass card");
    assert!((settled_card.width() - full_card.width()).abs() < 2.0);
}

#[test]
fn explorer_splitter_is_invisible_at_rest_and_reveals_on_hover_and_drag() {
    if isolated("explorer_splitter_is_invisible_at_rest_and_reveals_on_hover_and_drag") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "splitter", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    let output = headless.render();
    let splitter = explorer_splitter_id(headless.model.workspace(id).unwrap().pane_id());
    let splitter_response = headless.response(splitter);
    assert!(
        !splitter_response.sense.is_focusable(),
        "the resize splitter must not be a Tab stop"
    );
    assert!(
        splitter_response.rect.width() >= 8.0,
        "the resize splitter must keep an 8px pointer hit area"
    );
    let position = splitter_response.rect.center();
    assert!(
        !shape_hits_point(&output, position),
        "the splitter must paint no line or grip at rest"
    );

    let before_hover = headless.model.workspace(id).unwrap().explorer_width();
    let hover_start = headless.render_events_at(
        vec![egui::Event::PointerMoved(position)],
        headless.last_time + 1.0 / 60.0,
    );
    let early_alpha = rect_shapes(&hover_start)
        .into_iter()
        .find(|shape| {
            shape.rect.contains(position)
                && (shape.rect.width() - 1.0).abs() < 0.01
                && shape.rect.height() > 100.0
        })
        .map(|shape| shape.fill.to_srgba_unmultiplied()[3])
        .unwrap_or(0);
    headless.advance(0.20);
    let hovered = headless.render();
    let line = rect_shapes(&hovered)
        .into_iter()
        .find(|shape| {
            shape.rect.contains(position)
                && (shape.rect.width() - 1.0).abs() < 0.01
                && shape.rect.height() > 100.0
        })
        .expect("hovered splitter must fade in a 1px line");
    let line_rgba = line.fill.to_srgba_unmultiplied();
    let border_rgba = made_app::theme::BORDER_CONTROL.to_srgba_unmultiplied();
    assert!(
        line_rgba[..3]
            .iter()
            .zip(&border_rgba[..3])
            .all(|(actual, expected)| actual.abs_diff(*expected) <= 1),
        "hover line must use muted BORDER_CONTROL"
    );
    assert!(
        line_rgba[3] > 0 && line_rgba[3] < 255,
        "hover line must be partial opacity, not a bright permanent line: {line_rgba:?}"
    );
    assert!(
        line_rgba[3] > early_alpha,
        "splitter must fade in rather than snap: early {early_alpha}, settled {}",
        line_rgba[3]
    );
    let grip = rect_shapes(&hovered)
        .into_iter()
        .find(|shape| {
            shape.rect.contains(position)
                && (shape.rect.width() - 6.0).abs() < 0.01
                && (shape.rect.height() - 32.0).abs() < 0.01
        })
        .expect("hovered splitter must fade in a 6×32 grip");
    let grip_stroke = grip.stroke.color.to_srgba_unmultiplied();
    assert!(
        grip_stroke[..3]
            .iter()
            .zip(&border_rgba[..3])
            .all(|(actual, expected)| actual.abs_diff(*expected) <= 1),
        "hover grip must use muted BORDER_CONTROL"
    );
    assert!(
        grip_stroke[3] > 0 && grip_stroke[3] < 255,
        "hover grip must be partial opacity: {grip_stroke:?}"
    );
    assert!(
        output_text(&hovered).contains("Drag to resize explorer"),
        "splitter tooltip missing"
    );
    assert!(
        shape_hits_point(&hovered, position),
        "hovered splitter must paint at the boundary"
    );
    assert_eq!(
        headless.model.workspace(id).unwrap().explorer_width(),
        before_hover,
        "hover alone must not change the stored width"
    );

    let start = position;
    let end = start + egui::vec2(-48.0, 0.0);
    headless.render_events(vec![egui::Event::PointerMoved(start)]);
    headless.render_events(vec![pointer_button(start, true)]);
    headless.render_events(vec![egui::Event::PointerMoved(end)]);
    headless.advance(0.20);
    let dragged = headless.render();
    let dragged_center = headless.response(splitter).rect.center();
    let drag_line = rect_shapes(&dragged)
        .into_iter()
        .find(|shape| {
            shape.rect.contains(dragged_center)
                && (shape.rect.width() - 1.0).abs() < 0.01
                && shape.rect.height() > 100.0
        })
        .expect("dragging splitter must keep the faded line");
    let drag_rgba = drag_line.fill.to_srgba_unmultiplied();
    let accent_rgba = made_app::theme::ACCENT.to_srgba_unmultiplied();
    assert!(
        drag_rgba[..3]
            .iter()
            .zip(&accent_rgba[..3])
            .all(|(actual, expected)| actual.abs_diff(*expected) <= 1),
        "dragging splitter must use the green accent"
    );
    assert!(
        drag_rgba[3] > 0 && drag_rgba[3] < 255,
        "dragging accent must stay restrained: {drag_rgba:?}"
    );
    assert!(
        headless.model.workspace(id).unwrap().explorer_width() > before_hover,
        "dragging left must widen the explorer"
    );

    headless.render_events(vec![pointer_button(dragged_center, false)]);
    let away = egui::pos2(100.0, 400.0);
    // Frame-paced clock for the fade-out check: a loaded runner's wall clock
    // can pass the whole ~140 ms fade between two frames.
    let left_at = headless.last_time;
    headless.render_events_at(vec![egui::Event::PointerMoved(away)], left_at + 1.0 / 60.0);
    let leaving_center = headless.response(splitter).rect.center();
    let leaving = headless.render_events_at(Vec::new(), left_at + 2.0 / 60.0);
    assert!(
        shape_hits_point(&leaving, leaving_center),
        "leaving the splitter must fade out rather than vanish instantly"
    );
    headless.advance(0.20);
    let rest = headless.render();
    let rested_center = headless.response(splitter).rect.center();
    assert!(
        !shape_hits_point(&rest, rested_center),
        "the splitter paint must disappear after the fade-out"
    );
    assert!(
        !has_stroke_color(&rest, made_app::theme::BORDER_CONTROL_HOVER),
        "no permanent hover line may remain at rest"
    );
    assert!(
        !rect_shapes(&rest).iter().any(|shape| {
            shape.rect.contains(rested_center)
                && ((shape.rect.width() - 1.0).abs() < 0.01
                    || (shape.rect.width() - 6.0).abs() < 0.01)
                && (shape.fill.to_srgba_unmultiplied()[3] > 0
                    && shape.fill.to_srgba_unmultiplied()[3] < 255)
        }),
        "no faded splitter line or grip may remain at rest"
    );
}

#[test]
fn preview_fills_the_entire_left_pane_without_an_extra_row() {
    for (width, height) in [(1280.0, 800.0), (720.0, 480.0)] {
        if isolated("preview_fills_the_entire_left_pane_without_an_extra_row") {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let root = fixture_workspace(temp.path(), "preview-fill", "main.rs");
        let mut model =
            AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
        let id = open_workspace(&mut model, &root);
        let mut headless = Headless::new(model, width, height);
        wait_explorer_ready(&mut headless, id);
        headless.render();
        let row = headless
            .model
            .workspace(id)
            .unwrap()
            .explorer()
            .unwrap()
            .row_widget_id(0);
        headless.click(row);
        headless.pump_until(|model| {
            model
                .workspace(id)
                .and_then(Workbench::explorer)
                .and_then(made_app::ExplorerPane::preview_document)
                .is_some()
        });
        let output = headless.render();
        assert_eq!(
            headless.model.workspace(id).unwrap().explorer_width(),
            360.0,
            "the workbench must initialize at the new wide default"
        );
        let layout = made_app::theme::Layout::for_window(width, height);
        let explorer_width = layout.explorer_width(
            headless.model.workspace(id).unwrap().explorer_width(),
            layout.content_width(),
        );
        let (expected_explorer_width, expected_left_width) = if width >= 1100.0 {
            (360.0, 892.0)
        } else {
            (288.0, 410.0)
        };
        assert_eq!(layout.right_card_width, expected_explorer_width);
        assert_eq!(explorer_width, expected_explorer_width);
        assert!((layout.explorer_default_width() - expected_explorer_width).abs() < 0.01);
        let expected_left = layout.content_width() - layout.gap - explorer_width;
        assert!((expected_left - expected_left_width).abs() < 2.0);
        let expected_height =
            height - (layout.gap + layout.top_height + layout.gap) - layout.gutter;
        let explorer_card =
            largest_rect(&output, made_app::theme::BG_CARD_GLASS).expect("explorer card");
        let expected_explorer_left = width - layout.gutter - expected_explorer_width;
        assert!(
            (explorer_card.width() - expected_explorer_width).abs() < 2.0
                && (explorer_card.left() - expected_explorer_left).abs() < 2.0
                && (explorer_card.right() - (width - layout.gutter)).abs() < 2.0,
            "{width}x{height} explorer card {explorer_card:?} expected {expected_explorer_width} at {expected_explorer_left}"
        );
        let cards = rects_with_fill(&output, made_app::theme::BG_CARD);
        let left_card = cards
            .iter()
            .filter(|rect| rect.left() < width / 2.0 && rect.height() > 300.0)
            .max_by(|a, b| a.area().total_cmp(&b.area()))
            .expect("left preview card");
        assert!(
            (left_card.left() - layout.gutter).abs() < 1.0
                && (left_card.width() - expected_left).abs() < 2.0
                && (left_card.height() - expected_height).abs() < 2.0,
            "{width}x{height} preview card {left_card:?} expected width {expected_left} height {expected_height}"
        );
        assert!(
            output_text(&output).contains("Terminal"),
            "preview header must provide the return-to-terminal action"
        );
    }
}

#[test]
fn root_change_clears_a_loaded_preview_and_returns_terminal() {
    if isolated("root_change_clears_a_loaded_preview_and_returns_terminal") {
        return;
    }
    if !std::path::Path::new("/bin/bash").exists() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "root-a", "note.txt");
    let target = fixture_workspace(temp.path(), "root-b", "other.txt");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.render();
    let row = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .row_widget_id(0);
    headless.click(row);
    headless.pump_until(|model| {
        model
            .workspace(id)
            .and_then(Workbench::explorer)
            .and_then(made_app::ExplorerPane::preview_document)
            .is_some()
    });
    headless.click(top_workspace_action_id(id, "explorer"));
    assert!(!headless.model.workspace(id).unwrap().explorer_visible());
    headless.settle_animation();
    assert_eq!(
        headless.model.workspace(id).unwrap().left_surface(),
        LeftSurface::Preview
    );
    assert!(headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .preview_path()
        .is_some());
    assert!(!output_text(&headless.render()).contains("EXPLORER"));
    headless.click(top_workspace_action_id(id, "explorer"));
    assert!(headless.model.workspace(id).unwrap().explorer_visible());
    headless.settle_animation();
    assert!(output_text(&headless.render()).contains("EXPLORER"));
    let terminal_id = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .preview_terminal_id();
    headless.click(terminal_id);
    assert!(headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .preview_path()
        .is_some());
    type_terminal_command(
        headless.model.workspace_mut(id).unwrap(),
        &format!("cd '{}'", target.display()),
    );
    headless
        .pump_until(|model| model.workspace(id).unwrap().explorer_root() == Some(target.as_path()));
    let workbench = headless.model.workspace(id).unwrap();
    assert_eq!(workbench.left_surface(), LeftSurface::Terminal);
    assert!(workbench.explorer().unwrap().preview_path().is_none());
}

#[test]
fn compact_workspace_overflow_keeps_the_active_tab_visible() {
    if isolated("compact_workspace_overflow_keeps_the_active_tab_visible") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let mut ids = Vec::new();
    for index in 0..5 {
        let root = fixture_workspace(
            temp.path(),
            &format!("workspace-with-a-long-name-{index}"),
            "file.txt",
        );
        ids.push(open_workspace(&mut model, &root));
    }
    let mut headless = Headless::new(model, 720.0, 480.0);
    for id in &ids {
        wait_explorer_ready(&mut headless, *id);
    }
    let active = *ids.last().unwrap();
    headless.render();
    headless.render();
    assert_eq!(headless.model.active_workspace_id(), Some(active));
    let active_response = headless.response(workspace_tab_id(active));
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(720.0, 480.0));
    assert!(screen.contains_rect(active_response.rect));
    assert!(
        headless
            .ctx
            .read_response(workspace_tab_id(ids[3]))
            .is_none(),
        "a non-active tab should move into the labeled overflow at 720px"
    );
    headless.click(top_action_id("workspace-overflow"));
    headless.render();
    let menu = output_text(&headless.render());
    assert!(
        menu.contains("workspace-with-a-long-name"),
        "overflow menu should list workspace labels: {menu}"
    );
    assert!(
        !menu.contains('●') && !menu.contains('○'),
        "overflow labels must not carry marker circles: {menu}"
    );
}

#[test]
fn f6_leaves_terminal_focus_for_workspace_chrome_and_focus_returns() {
    if isolated("f6_leaves_terminal_focus_for_workspace_chrome_and_focus_returns") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "focus", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    for _ in 0..3 {
        headless.render();
    }
    assert!(headless
        .model
        .workspace(id)
        .unwrap()
        .terminal()
        .unwrap()
        .is_focused());
    headless.press_key(egui::Key::F6);
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(top_workspace_action_id(id, "explorer")),
        "F6 must move focus to the workspace chrome"
    );
    headless.render();
    assert!(!headless
        .model
        .workspace(id)
        .unwrap()
        .terminal()
        .unwrap()
        .is_focused());

    let term_id = headless
        .model
        .workspace(id)
        .unwrap()
        .terminal()
        .unwrap()
        .focus_id()
        .expect("terminal focus id");
    headless.click(term_id);
    for _ in 0..3 {
        headless.render();
        if headless
            .model
            .workspace(id)
            .unwrap()
            .terminal()
            .unwrap()
            .is_focused()
        {
            break;
        }
    }
    assert!(headless
        .model
        .workspace(id)
        .unwrap()
        .terminal()
        .unwrap()
        .is_focused());
}

#[test]
fn workspace_strip_budget_keeps_controls_inside_and_non_overlapping() {
    if isolated("workspace_strip_budget_keeps_controls_inside_and_non_overlapping") {
        return;
    }
    let short: Vec<String> = (0..11)
        .map(|index| char::from(b'a' + index as u8).to_string())
        .collect();
    let long: Vec<String> = (0..11)
        .map(|index| format!("long-workspace-label-{index:02}"))
        .collect();
    let cases: Vec<(f32, f32, Vec<String>)> = vec![
        (
            720.0,
            480.0,
            vec![
                "a".to_owned(),
                "b".to_owned(),
                "c".to_owned(),
                "long-active-workspace-name".to_owned(),
            ],
        ),
        (
            720.0,
            480.0,
            vec![
                "a".to_owned(),
                "b".to_owned(),
                "c".to_owned(),
                "d".to_owned(),
                "long-active-workspace-name".to_owned(),
            ],
        ),
        (1280.0, 800.0, short),
        (1280.0, 800.0, long),
    ];
    let mut saw_fit = false;
    let mut saw_overflow_at_1280 = false;
    for (width, height, names) in cases {
        let temp = tempfile::tempdir().unwrap();
        let mut model =
            AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
        let mut ids = Vec::new();
        for name in &names {
            let root = fixture_workspace(temp.path(), name, "file.txt");
            ids.push(open_workspace(&mut model, &root));
        }
        let active = *ids.last().unwrap();
        let mut headless = Headless::new(model, width, height);
        headless.render();
        headless.render();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
        let overflow = headless
            .ctx
            .read_response(top_action_id("workspace-overflow"));
        let mut controls = vec![
            headless.response(top_action_id("open-folder")).rect,
            headless
                .response(top_workspace_action_id(active, "explorer"))
                .rect,
            headless.response(made_app::theme::wordmark_id()).rect,
        ];
        if let Some(response) = &overflow {
            controls.push(response.rect);
        }
        for control in &controls {
            assert!(
                screen.contains_rect(*control),
                "{width}x{height} control clipped: {control:?}"
            );
        }
        for (index, first) in controls.iter().enumerate() {
            for second in &controls[index + 1..] {
                assert!(
                    !first.intersects(*second),
                    "{width}x{height} controls overlap: {first:?} vs {second:?}"
                );
            }
        }

        let active_tab = headless.response(workspace_tab_id(active)).rect;
        assert!(screen.contains_rect(active_tab));
        let mut visible_tabs = Vec::new();
        for id in &ids {
            if let Some(response) = headless.ctx.read_response(workspace_tab_id(*id)) {
                assert!(
                    screen.contains_rect(response.rect),
                    "{width}x{height} tab clipped: {:?}",
                    response.rect
                );
                for control in &controls {
                    assert!(
                        !response.rect.intersects(*control),
                        "{width}x{height} tab overlaps fixed control"
                    );
                }
                visible_tabs.push(response.rect);
            }
        }
        assert!(visible_tabs.contains(&active_tab));
        for (index, first) in visible_tabs.iter().enumerate() {
            for second in &visible_tabs[index + 1..] {
                assert!(
                    !first.intersects(*second),
                    "{width}x{height} tabs overlap: {first:?} vs {second:?}"
                );
            }
        }
        let all_tabs_rendered = visible_tabs.len() == ids.len();
        assert_eq!(
            overflow.is_some(),
            !all_tabs_rendered,
            "{width}x{height} overflow button must appear exactly when tabs do not all fit              (rendered {}/{})",
            visible_tabs.len(),
            ids.len()
        );
        if overflow.is_some() {
            saw_overflow_at_1280 |= width >= 1280.0;
        } else {
            saw_fit = true;
        }
    }
    assert!(
        saw_fit,
        "one case must prove the compact header can fit all tabs"
    );
    assert!(
        saw_overflow_at_1280,
        "one 1280px case must still force the overflow menu"
    );
}

#[test]
fn diagnostics_expose_workspace_shell_and_inactive_integration() {
    if isolated("diagnostics_expose_workspace_shell_and_inactive_integration") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "diagnostics", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    let token = bridge_token(headless.model.workspace(id).unwrap());
    headless.render();
    open_about_diagnostics(&mut headless, id);
    let text = output_text(&headless.render());
    for expected in [
        "WORKSPACE",
        "Identity",
        "Shell cwd",
        "Cwd reporting",
        "Effective root",
        "Root source",
        "Watcher",
        "inactive",
        "Copy diagnostics",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
    assert!(!text.contains("Waiting means"));
    if let Some(token) = token.as_deref() {
        assert!(!text.contains(token), "diagnostics leaked the bridge token");
    }
    let copy_id = diagnostics_copy_id(headless.model.workspace(id).unwrap().pane_id());
    let copied = headless.click(copy_id);
    let copied_text = copied
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .expect("copy diagnostics command");
    assert!(copied_text.contains("Made diagnostics"));
    assert!(copied_text.contains("workspace identity"));
    assert!(copied_text.contains("cwd reporting"));
    if let Some(token) = token {
        assert!(!copied_text.contains(token.as_str()));
    }
}

#[test]
fn diagnostics_previous_log_row_reflects_existence_and_copies() {
    if isolated("diagnostics_previous_log_row_reflects_existence_and_copies") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "previous-log", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 720.0, 480.0);
    wait_explorer_ready(&mut headless, id);
    let snapshot = headless.model.workspace(id).unwrap().diagnostics_snapshot();
    let previous = snapshot
        .previous_log_location
        .clone()
        .expect("previous log path");
    assert!(!snapshot.previous_log_exists);
    headless.render();
    open_about_diagnostics(&mut headless, id);
    let text = output_text(&headless.render());
    for expected in ["Previous log", "Copy previous log path", "not yet rotated"] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
    let pane_id = headless.model.workspace(id).unwrap().pane_id();
    let previous_copy_id = diagnostics_previous_log_copy_id(pane_id);
    let response = headless.response(previous_copy_id);
    assert!(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(720.0, 480.0))
            .contains_rect(response.rect),
        "previous-log action clipped at 720x480: {:?}",
        response.rect
    );
    let output = headless.click(previous_copy_id);
    assert!(
        !output
            .platform_output
            .commands
            .iter()
            .any(|command| matches!(command, egui::OutputCommand::CopyText(_))),
        "copy action must not copy without a previous log"
    );

    std::fs::write(&previous, b"previous record\n").unwrap();
    // The `.1` existence probe is bucketed to at most once per second.
    headless.render_events_at(Vec::new(), headless.last_time + 1.1);
    let text = output_text(&headless.render());
    assert!(!text.contains("not yet rotated"), "{text}");
    assert!(headless.response(previous_copy_id).enabled());

    let pane_id = headless.model.workspace(id).unwrap().pane_id();
    let path_id = diagnostics_previous_log_path_id(pane_id);
    let window_id = egui::Id::unique(("made-about-window", pane_id));
    let window = headless
        .ctx
        .memory(|memory| memory.area_rect(window_id))
        .expect("diagnostics window rect");
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(720.0, 480.0));
    assert!(screen.contains_rect(window), "window clipped: {window:?}");
    assert!(window.contains_rect(headless.response(path_id).rect));
    assert!(window.contains_rect(headless.response(previous_copy_id).rect));

    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    headless.render_events_at(Vec::new(), headless.last_time + 1.0);
    let before_tooltip = output_text(&headless.render()).matches(&previous).count();
    let hovered = headless.hover_until(path_id, |text| {
        text.matches(&previous).count() > before_tooltip
    });
    let after_tooltip = output_text(&hovered).matches(&previous).count();
    assert!(
        after_tooltip > before_tooltip,
        "previous-log full-path tooltip did not appear: {before_tooltip} -> {after_tooltip}"
    );

    let output = headless.click(previous_copy_id);
    let copied = output
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .expect("copy previous log command");
    assert_eq!(copied, previous);
}

#[test]
fn diagnostics_degraded_state_keeps_previous_log_row_at_720() {
    if isolated("diagnostics_degraded_state_keeps_previous_log_row_at_720") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "degraded-diagnostics", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 720.0, 480.0);
    wait_explorer_ready(&mut headless, id);
    let options = bridge_options(headless.model.workspace(id).unwrap()).expect("bridge options");
    let mut socket = connect(&options);
    send(
        &mut socket,
        json!({"type":"state", "revision":1, "activeSessionID":"ui-session", "tabsEnabled":false,
        "sessions":[{"sessionID":"ui-session", "title":"Owned session", "active":true, "busy":false, "attention":false, "unread":false,
        "promptPulse":0, "renaming":false, "directory":root, "subpath":null, "projectID":null}]}),
    );
    headless.pump_until(|model| {
        model
            .workspace(id)
            .is_some_and(|workspace| workspace.diagnostics_snapshot().status_label == "degraded")
    });

    headless.render();
    open_about_diagnostics(&mut headless, id);
    let output = headless.render();
    let text = output_text(&output);
    for expected in ["degraded", "Previous log", "not yet rotated"] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
    let pane_id = headless.model.workspace(id).unwrap().pane_id();
    let copy_id = diagnostics_previous_log_copy_id(pane_id);
    let window = headless
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::unique(("made-about-window", pane_id))))
        .expect("diagnostics window rect");
    assert!(window.contains_rect(headless.response(copy_id).rect));
    assert_text_clipped_inside(
        &output,
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(720.0, 480.0)),
    );
}

#[test]
fn diagnostics_warn_about_unsafe_previous_and_lock_paths_at_startup() {
    if isolated("diagnostics_warn_about_unsafe_previous_and_lock_paths_at_startup") {
        return;
    }
    let log = isolated_log_path();
    let backup = PathBuf::from(format!("{}.1", log.display()));
    let lock = PathBuf::from(format!("{}.lock", log.display()));
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(&log, b"active record\n").unwrap();
    fs::create_dir(&backup).unwrap();
    fs::create_dir(&lock).unwrap();

    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "unsafe-log-paths", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 720.0, 480.0);
    wait_explorer_ready(&mut headless, id);

    let warnings: Vec<String> = headless
        .model
        .workspace(id)
        .unwrap()
        .diagnostics_snapshot()
        .entries
        .iter()
        .filter(|entry| entry.level == made_app::LogLevel::Warn)
        .map(|entry| entry.message.clone())
        .collect();
    let joined = warnings.join("\n");
    assert!(
        joined.contains("previous diagnostics log path is not a regular file"),
        "{joined}"
    );
    assert!(
        joined.contains("diagnostics log lock path is not a regular file"),
        "{joined}"
    );

    headless.render();
    open_about_diagnostics(&mut headless, id);
    let text = output_text(&headless.render());
    assert!(text.contains("previous diagnostics log path is not a regular file"));
    assert!(text.contains("diagnostics log lock path is not a regular file"));

    headless.model.shutdown();
    assert_eq!(
        fs::read(&log).expect("active log must be untouched by a refused append"),
        b"active record\n"
    );
    assert!(backup.is_dir(), "unsafe previous path must be untouched");
    assert!(lock.is_dir(), "unsafe lock path must be untouched");
}

#[test]
fn oversized_legacy_log_warns_with_previous_row_at_720_and_rotates_on_shutdown() {
    if isolated("oversized_legacy_log_warns_with_previous_row_at_720_and_rotates_on_shutdown") {
        return;
    }
    let log = isolated_log_path();
    let backup = PathBuf::from(format!("{}.1", log.display()));
    let lock = PathBuf::from(format!("{}.lock", log.display()));
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(&backup, b"previous record\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&backup, fs::Permissions::from_mode(0o644)).unwrap();
    }
    {
        let file = fs::File::create(&log).unwrap();
        file.set_len(made_app::diagnostics::MAX_LOG_FILE_BYTES + 1)
            .unwrap();
    }

    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "legacy-log", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 720.0, 480.0);
    wait_explorer_ready(&mut headless, id);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let active_mode = fs::metadata(&log).unwrap().permissions().mode() & 0o777;
        let previous_mode = fs::metadata(&backup).unwrap().permissions().mode() & 0o777;
        assert_eq!(active_mode, 0o600, "active log mode after preflight");
        assert_eq!(
            previous_mode, 0o600,
            "existing previous log must be tightened at startup"
        );
    }

    let snapshot = headless.model.workspace(id).unwrap().diagnostics_snapshot();
    assert!(
        !snapshot
            .entries
            .iter()
            .any(|entry| entry.level == made_app::LogLevel::Error
                && entry.message.contains("will not be written")),
        "legacy size must not refuse future writes: {:?}",
        snapshot.entries
    );
    assert!(
        snapshot.entries.iter().any(|entry| {
            entry.level == made_app::LogLevel::Warn
                && entry.message.contains("above the 1 MiB threshold")
        }),
        "missing legacy warning: {:?}",
        snapshot.entries
    );

    headless.render();
    open_about_diagnostics(&mut headless, id);
    let output = headless.render();
    let text = output_text(&output);
    for expected in [
        "above the 1 MiB threshold",
        "Previous log",
        "Copy previous log path",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
    assert!(!text.contains("not yet rotated"), "{text}");
    assert!(!text.contains("will not be written"), "{text}");

    let pane_id = headless.model.workspace(id).unwrap().pane_id();
    let path_id = diagnostics_previous_log_path_id(pane_id);
    let copy_id = diagnostics_previous_log_copy_id(pane_id);
    let window_id = egui::Id::unique(("made-about-window", pane_id));
    let window = headless
        .ctx
        .memory(|memory| memory.area_rect(window_id))
        .expect("diagnostics window rect");
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(720.0, 480.0));
    assert!(window.contains_rect(headless.response(path_id).rect));
    assert!(window.contains_rect(headless.response(copy_id).rect));
    assert_text_clipped_inside(&output, screen);
    assert!(
        screen.expand(16.0).contains_rect(window),
        "diagnostics window shadow/content clipped: {window:?}"
    );

    headless.render_events_at(Vec::new(), headless.last_time + 1.0);
    let previous = backup.to_string_lossy();
    let before_tooltip = output_text(&headless.render())
        .matches(previous.as_ref())
        .count();
    let hovered = headless.hover_until(path_id, |text| {
        text.matches(previous.as_ref()).count() > before_tooltip
    });
    let after_tooltip = output_text(&hovered).matches(previous.as_ref()).count();
    assert!(
        after_tooltip > before_tooltip,
        "previous-log full-path tooltip did not appear: {before_tooltip} -> {after_tooltip}"
    );

    let copied = headless.click(copy_id);
    let copied_path = copied
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .expect("copy previous log command");
    assert_eq!(copied_path, backup.to_string_lossy());

    headless.model.shutdown();

    assert_eq!(
        fs::metadata(&backup).expect("rotated previous log").len(),
        made_app::diagnostics::MAX_LOG_FILE_BYTES + 1
    );
    assert!(
        fs::metadata(&log).expect("new active log").len()
            < made_app::diagnostics::MAX_LOG_FILE_BYTES
    );
    assert!(lock.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [&log, &backup, &lock] {
            assert_eq!(
                fs::metadata(path)
                    .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
                    .permissions()
                    .mode()
                    & 0o777,
                0o600,
                "{} must be 0600",
                path.display()
            );
        }
    }
}

#[test]
fn file_control_sits_below_session_strip_and_path() {
    if isolated("file_control_sits_below_session_strip_and_path") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "file-sessions", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.render();
    assert!(
        headless
            .ctx
            .read_response(top_workspace_action_id(id, "sessions"))
            .is_none(),
        "Sessions control must be absent without session data"
    );
    let options = bridge_options(headless.model.workspace(id).unwrap()).expect("bridge options");
    let mut socket = connect(&options);
    send_state(&mut socket, 1, &root);
    service_bridge_heartbeat(socket);
    headless.pump_until(|model| !model.workspace(id).unwrap().session_map().is_empty());
    headless
        .model
        .workspace_mut(id)
        .unwrap()
        .set_sessions_visible(true);
    headless.render();
    let row = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .row_widget_id(0);
    headless.click(row);
    headless.pump_until(|model| {
        model
            .workspace(id)
            .and_then(Workbench::explorer)
            .and_then(made_app::ExplorerPane::preview_document)
            .is_some()
    });
    let terminal_id = headless
        .model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .preview_terminal_id();
    headless.click(terminal_id);
    let output = headless.render();
    let text = output_text(&output);
    assert!(text.contains("Owned session"));
    let glass = rects_with_fill(&output, made_app::theme::BG_TERMINAL_GLASS);
    assert_eq!(
        glass.len(),
        1,
        "session/path/file-slot primary state must keep exactly one terminal glass layer: {glass:?}"
    );
    assert!(
        glass[0].width() > 500.0 && glass[0].height() > 300.0,
        "the sole glass rect must be the terminal card: {glass:?}"
    );
    assert!(
        rects_with_fill(&output, made_app::theme::BG_TERMINAL).is_empty(),
        "primary terminal body/path/file-slot chrome must not paint opaque BG_TERMINAL"
    );
    assert!(
        !rects_with_fill(&output, made_app::theme::BG_HEADER).is_empty(),
        "the optional session strip/header must remain opaque BG_HEADER"
    );
    let pane_id = headless.model.workspace(id).unwrap().pane_id().to_owned();
    let file_rect = headless
        .response(left_surface_action_id(&pane_id, "file"))
        .rect;
    let layout = made_app::theme::Layout::for_window(1280.0, 800.0);
    let card_top = layout.gap + layout.top_height + layout.gap;
    let path_line = 22.0;
    assert!(
        file_rect.top() >= card_top + layout.tab_strip_height + path_line - 1.0,
        "File control overlapped the session strip/path: {file_rect:?}"
    );
}

#[test]
fn add_workspace_picker_keeps_context_and_back_or_escape_returns() {
    if isolated("add_workspace_picker_keeps_context_and_back_or_escape_returns") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "ctx", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    model.begin_new_workspace();
    let mut headless = Headless::new(model, 1100.0, 700.0);
    let text = output_text(&headless.render());
    assert!(text.contains("Back to ctx"), "missing back context: {text}");
    assert!(!text.contains("Return to workspace"));
    headless.click(picker_control_id("back"));
    assert!(!headless.model.is_picker());
    assert_eq!(headless.model.active_workspace_id(), Some(id));

    headless.model.begin_new_workspace();
    headless.render();
    headless.press_key(egui::Key::Escape);
    assert!(!headless.model.is_picker());
    assert_eq!(headless.model.active_workspace_id(), Some(id));

    headless.model.begin_new_workspace();
    headless.render();
    headless.click(picker_control_id("close"));
    assert!(!headless.model.is_picker());
    assert_eq!(headless.model.active_workspace_id(), Some(id));
}

#[test]
fn picker_rows_and_secondary_action_use_unified_states() {
    if isolated("picker_rows_and_secondary_action_use_unified_states") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("alpha")).unwrap();
    fs::create_dir(temp.path().join("beta")).unwrap();
    let mut picker = picker_headless(temp.path(), 900.0, 620.0);
    picker.model.picker_select(0).unwrap();
    let output = picker.render();
    let secondary = picker.response(picker_control_id("selected")).rect;
    assert!(
        rects_with_fill(&output, made_app::theme::BG_ELEVATED)
            .iter()
            .any(|rect| rect.contains_rect(secondary)),
        "secondary action rest fill must be BG_ELEVATED"
    );

    let row_id = egui::Id::unique(("made-picker-row", 1));
    let row_rect = picker.response(row_id).rect;
    let hovered = picker.hover(row_id);
    assert!(
        rects_with_fill(&hovered, made_app::theme::BG_HOVER)
            .iter()
            .any(|rect| rect.intersects(row_rect)),
        "hovered picker row must use BG_HOVER"
    );

    let hovered = picker.hover(picker_control_id("selected"));
    assert!(
        rects_with_fill(&hovered, made_app::theme::BG_HOVER)
            .iter()
            .any(|rect| rect.contains_rect(secondary)),
        "hovered secondary action must use BG_HOVER"
    );
    let pressed = picker.press_hold(picker_control_id("selected"));
    assert!(
        rects_with_fill(&pressed, made_app::theme::BG_SELECTED)
            .iter()
            .any(|rect| rect.contains_rect(secondary)),
        "pressed secondary action must use BG_SELECTED"
    );
    picker.release(picker_control_id("selected"));
}

#[test]
fn picker_rows_have_no_focus_outline_but_picker_controls_keep_focus_rings() {
    if isolated("picker_rows_have_no_focus_outline_but_picker_controls_keep_focus_rings") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    for name in ["alpha", "beta"] {
        fs::create_dir(temp.path().join(name)).unwrap();
    }
    let mut picker = picker_headless(temp.path(), 900.0, 620.0);
    picker.model.picker_select(0).unwrap();
    picker.render();
    let row_id = egui::Id::unique(("made-picker-row", 0));
    let selected_row = picker.response(row_id).rect;
    assert!(
        rects_with_fill(&picker.render(), made_app::theme::BG_SELECTED)
            .iter()
            .any(|rect| rect.intersects(selected_row)),
        "selected picker row must keep the inset BG_SELECTED fill"
    );

    picker.focus(picker_list_id());
    let focused = picker.render();
    assert!(
        focus_ring_rects(&focused, selected_row).is_empty(),
        "selected picker row must not paint a focus outline: {:?}",
        focus_ring_rects(&focused, selected_row)
    );
    assert!(
        text_colors(&focused, "alpha", 400.0).contains(&made_app::theme::TEXT_PRIMARY),
        "selected picker row must keep primary text"
    );
    let row_rect = picker.response(row_id).rect;
    let label = text_rects(&focused, "alpha", 400.0)
        .into_iter()
        .find(|rect| rect.intersects(row_rect))
        .expect("selected picker label");
    assert!(
        (label.left() - (row_rect.left() + 24.0)).abs() < 1.0,
        "picker text must start 24px from the row edge: {label:?}"
    );
    assert!(
        (label.center().y - row_rect.center().y).abs() < 0.75,
        "picker label must be vertically centered: {label:?} vs {row_rect:?}"
    );
    let marker = path_shapes(&focused)
        .into_iter()
        .filter(|path| path.points.len() == 3 && path.fill == made_app::theme::TEXT_PRIMARY)
        .find(|path| row_rect.contains(path_center(path)))
        .expect("picker app-drawn disclosure triangle");
    let marker_rect = egui::Rect::from_points(&marker.points);
    assert!(
        (marker_rect.center().x - (row_rect.left() + 12.0)).abs() < 1.5,
        "picker 16px icon slot center must sit 12px from the row edge: {marker_rect:?}"
    );
    assert!(
        (marker_rect.center().y - label.center().y).abs() < 0.75,
        "picker triangle must share the label's optical center: {marker_rect:?} vs {label:?}"
    );

    let current = picker.response(picker_control_id("current")).rect;
    picker.focus(picker_control_id("current"));
    let button_focused = picker.render();
    assert!(
        !focus_ring_rects(&button_focused, current).is_empty(),
        "picker buttons must keep their visible focus ring"
    );
}

#[test]
fn picker_click_expands_tree_keeps_ancestors_and_updates_target() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(root.join("alpha/nested")).unwrap();
    fs::create_dir(root.join("beta")).unwrap();
    fs::write(root.join("not-a-folder.txt"), "x").unwrap();
    let alpha = fs::canonicalize(root.join("alpha")).unwrap();
    let root = fs::canonicalize(&root).unwrap();
    let mut picker = picker_headless(&root, 900.0, 620.0);
    picker.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    picker.render();
    let row_id = egui::Id::unique(("made-picker-row", 0));
    picker.click(row_id);

    let current = picker.model.launch().picker().expect("picker remains");
    assert_eq!(
        current.current_dir(),
        root,
        "expanding must not replace the root rows"
    );
    assert_eq!(current.selection(), Some(0));
    assert_eq!(current.path_target(), alpha);
    assert_eq!(
        current
            .rows()
            .iter()
            .map(|row| (row.name(), row.depth(), row.expanded()))
            .collect::<Vec<_>>(),
        vec![("alpha", 0, true), ("nested", 1, false), ("beta", 0, false)]
    );
    assert!(picker.model.is_picker());
    assert_eq!(picker.model.surfaces(), Surfaces::Picker);
    assert!(picker.model.workbench().is_none());
    assert_eq!(picker.model.workspace_count(), 0);
    assert!(!picker.model.start_pending());
    assert_eq!(
        picker_current_target(&picker.model),
        Some(alpha.display().to_string()),
        "the clicked folder must be the current target"
    );

    let output = picker.render();
    let text = output_text(&output);
    assert!(
        text.contains("Click a folder to expand or collapse it"),
        "picker help must describe tree expansion: {text}"
    );
    assert!(
        !text.contains("Click a folder to enter it"),
        "stale enter-to-replace help must be removed: {text}"
    );
    assert!(text.contains("nested"), "child rows must be listed: {text}");
    let nested_row = picker
        .response(egui::Id::unique(("made-picker-row", 1)))
        .rect;
    let nested_label = text_rects(&output, "nested", 500.0)
        .into_iter()
        .find(|rect| rect.intersects(nested_row))
        .expect("nested label");
    assert!(
        (nested_label.left() - (nested_row.left() + 24.0 + 14.0)).abs() < 1.0,
        "depth-1 label must indent 14px: {nested_label:?} vs {nested_row:?}"
    );
    let nested_marker = path_shapes(&output)
        .into_iter()
        .filter(|path| path.points.len() == 3)
        .find(|path| nested_row.contains(path_center(path)))
        .expect("nested disclosure marker");
    assert!(
        (path_center(&nested_marker).x - (nested_row.left() + 12.0 + 14.0)).abs() < 1.5,
        "depth-1 marker must indent 14px"
    );
    let selected_row = picker.response(row_id).rect;
    assert!(
        rects_with_fill(&output, made_app::theme::BG_SELECTED)
            .iter()
            .any(|rect| rect.intersects(selected_row)),
        "the clicked folder row must stay selected"
    );
    let list_rect = picker.response(picker_list_id()).rect;
    assert!(
        focus_ring_rects(&output, list_rect).is_empty(),
        "pointer focus in the picker must not paint a row or container outline: {:?}",
        focus_ring_rects(&output, list_rect)
    );
    let chip = picker.response(picker_path_id()).rect;
    assert!(
        rects_with_fill(&output, made_app::theme::BG_SELECTED)
            .iter()
            .any(|rect| rect.intersects(chip)),
        "the path chip must use the selected row target: {chip:?}"
    );
    let hovered =
        output_text(&picker.hover_until_text(picker_path_id(), &alpha.display().to_string()));
    assert!(
        hovered.contains(&alpha.display().to_string()),
        "the path chip tooltip must expose the selected row path: {hovered}"
    );

    picker.advance(0.4);
    picker.click(egui::Id::unique(("made-picker-row", 1)));
    let nested_target = picker.model.launch().picker().expect("picker remains");
    assert_eq!(nested_target.selection(), Some(1));
    assert_eq!(
        nested_target.path_target(),
        fs::canonicalize(root.join("alpha/nested")).unwrap()
    );
    let nested = fs::canonicalize(root.join("alpha/nested"))
        .unwrap()
        .display()
        .to_string();
    let hovered = output_text(&picker.hover_until_text(picker_path_id(), &nested));
    assert!(
        hovered.contains(&nested),
        "the path chip must follow the selected child row: {hovered}"
    );

    picker.advance(0.4);
    picker.click(row_id);
    let collapsed = picker.model.launch().picker().expect("picker remains");
    assert_eq!(
        collapsed.current_dir(),
        root,
        "collapsing must not re-root the picker"
    );
    assert_eq!(collapsed.selection(), Some(0));
    assert!(!collapsed.rows()[0].expanded());
    assert_eq!(
        collapsed
            .rows()
            .iter()
            .map(|row| row.name())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta"],
        "collapsing removes only the expanded subtree"
    );
    assert_eq!(collapsed.path_target(), alpha);
}

#[test]
fn picker_keyboard_tree_navigation_and_confirmation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(root.join("alpha/nested/deep")).unwrap();
    fs::create_dir(root.join("beta")).unwrap();
    let root = fs::canonicalize(&root).unwrap();

    let mut confirm_root = picker_headless(&root, 900.0, 620.0);
    confirm_root.focus(picker_list_id());
    confirm_root.press_key_with_modifiers(egui::Key::Enter, egui::Modifiers::COMMAND);
    assert!(
        !confirm_root.model.is_picker(),
        "Ctrl/Cmd+Enter with no selection must confirm the root"
    );
    assert_eq!(
        confirm_root.model.launch().explorer_root(),
        Some(root.as_path())
    );

    let mut tree = picker_headless(&root, 900.0, 620.0);
    tree.focus(picker_list_id());
    tree.press_key(egui::Key::ArrowDown);
    assert_eq!(
        tree.model.launch().picker().unwrap().selection(),
        Some(0),
        "the first Down from no selection must select the first row"
    );
    tree.press_key(egui::Key::ArrowRight);
    let expanded = tree.model.launch().picker().unwrap();
    assert!(expanded.rows()[0].expanded());
    assert_eq!(expanded.path_target(), root.join("alpha"));
    tree.press_key(egui::Key::ArrowDown);
    assert_eq!(
        tree.model.launch().picker().unwrap().path_target(),
        root.join("alpha/nested")
    );
    let keyboard_selected = tree.render();
    let selected_rect = tree.response(egui::Id::unique(("made-picker-row", 1))).rect;
    assert!(
        rects_with_fill(&keyboard_selected, made_app::theme::BG_SELECTED)
            .iter()
            .any(|rect| rect.intersects(selected_rect)),
        "keyboard selection must be visible on the child row"
    );
    tree.press_key(egui::Key::ArrowLeft);
    assert_eq!(
        tree.model.launch().picker().unwrap().path_target(),
        root.join("alpha"),
        "Left on a collapsed child selects its parent row"
    );
    tree.press_key(egui::Key::ArrowRight);
    assert_eq!(
        tree.model.launch().picker().unwrap().path_target(),
        root.join("alpha/nested"),
        "Right on an expanded folder selects its first child"
    );
    tree.press_key(egui::Key::Enter);
    assert!(
        tree.model
            .launch()
            .picker()
            .unwrap()
            .rows()
            .iter()
            .any(|row| row.name() == "deep"),
        "Enter expands the selected folder"
    );
    tree.press_key(egui::Key::Enter);
    assert!(
        !tree
            .model
            .launch()
            .picker()
            .unwrap()
            .rows()
            .iter()
            .any(|row| row.name() == "deep"),
        "Enter collapses the selected folder"
    );
    tree.press_key(egui::Key::ArrowRight);
    tree.press_key(egui::Key::ArrowRight);
    assert_eq!(
        tree.model.launch().picker().unwrap().path_target(),
        root.join("alpha/nested/deep"),
        "Right expands then selects the first child"
    );
    tree.press_key_with_modifiers(egui::Key::Enter, egui::Modifiers::COMMAND);
    assert!(
        !tree.model.is_picker(),
        "Ctrl/Cmd+Enter must confirm the selected tree row"
    );
    assert_eq!(
        tree.model.launch().explorer_root(),
        Some(root.join("alpha/nested/deep").as_path())
    );

    let mut enter = picker_headless(&root, 900.0, 620.0);
    enter.focus(picker_list_id());
    enter.press_key(egui::Key::ArrowDown);
    enter.press_key(egui::Key::Enter);
    assert!(enter.model.is_picker(), "Enter must only toggle expansion");
    assert!(enter
        .model
        .launch()
        .picker()
        .unwrap()
        .rows()
        .iter()
        .any(|row| row.name() == "nested"));
    enter.press_key(egui::Key::Enter);
    assert!(!enter
        .model
        .launch()
        .picker()
        .unwrap()
        .rows()
        .iter()
        .any(|row| row.name() == "nested"));
    assert_eq!(
        enter.model.launch().picker().unwrap().current_dir(),
        root,
        "Enter must not re-root or start a workspace"
    );

    let mut selected = picker_headless(&root, 900.0, 620.0);
    selected.focus(picker_list_id());
    selected.press_key(egui::Key::ArrowDown);
    selected.press_key(egui::Key::ArrowRight);
    selected.press_key(egui::Key::ArrowDown);
    assert_eq!(
        selected.model.launch().picker().unwrap().path_target(),
        root.join("alpha/nested")
    );
    selected.click(picker_control_id("selected"));
    assert!(
        !selected.model.is_picker(),
        "Open selected folder must open the selected row"
    );
    assert_eq!(
        selected.model.launch().explorer_root(),
        Some(root.join("alpha/nested").as_path())
    );
    assert!(selected.model.start_pending());
    assert!(selected.model.workbench().is_none());
}

#[test]
fn picker_compact_keyboard_deep_leaf_keeps_ancestor_chain_visible() {
    if isolated("picker_compact_keyboard_deep_leaf_keeps_ancestor_chain_visible") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(root.join("alpha/nested/deep/leaf")).unwrap();
    fs::create_dir(root.join("alpha/sibling-inside")).unwrap();
    fs::create_dir(root.join("beta")).unwrap();
    fs::create_dir(root.join("gamma")).unwrap();
    let root = fs::canonicalize(&root).unwrap();
    let mut picker = picker_headless(&root, 720.0, 480.0);
    picker.focus(picker_list_id());
    picker.press_key(egui::Key::ArrowDown);
    picker.press_key(egui::Key::ArrowRight);
    picker.press_key(egui::Key::ArrowDown);
    picker.press_key(egui::Key::ArrowRight);
    picker.press_key(egui::Key::ArrowDown);
    picker.press_key(egui::Key::ArrowRight);
    picker.press_key(egui::Key::ArrowDown);
    picker.render();
    let output = picker.render();

    let model = picker.model.launch().picker().expect("picker remains");
    assert_eq!(
        model.path_target(),
        root.join("alpha/nested/deep/leaf"),
        "keyboard traversal must select the deep leaf"
    );
    assert_eq!(model.selection(), Some(3));
    let list_rect = picker.response(picker_list_id()).rect;
    for (index, name) in [(0, "alpha"), (1, "nested"), (2, "deep"), (3, "leaf")] {
        let row = picker
            .response(egui::Id::unique(("made-picker-row", index)))
            .rect;
        assert!(
            list_rect.intersects(row),
            "{name} row must remain visible in the compact list: {row:?} vs {list_rect:?}"
        );
    }
    let leaf_row = picker
        .response(egui::Id::unique(("made-picker-row", 3)))
        .rect;
    assert!(
        rects_with_fill(&output, made_app::theme::BG_SELECTED)
            .iter()
            .any(|rect| rect.intersects(leaf_row)),
        "the selected leaf row must keep its fill"
    );
    assert!(
        focus_ring_rects(&output, list_rect).is_empty(),
        "the compact tree must not paint a row/container outline"
    );
}

#[test]
fn picker_double_click_toggles_once() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(root.join("alpha/nested")).unwrap();
    let root = fs::canonicalize(&root).unwrap();
    let mut picker = picker_headless(&root, 900.0, 620.0);
    picker.render();
    picker.render();
    let row_id = egui::Id::unique(("made-picker-row", 0));
    let position = picker.response(row_id).rect.center();

    // First click expands/selects alpha. The second click is a genuine
    // double-click and must not collapse the same row again.
    picker.render_events_at(vec![egui::Event::PointerMoved(position)], 1.0);
    picker.render_events_at(vec![pointer_button(position, true)], 1.0);
    picker.render_events_at(vec![pointer_button(position, false)], 1.0);
    let after_first = picker.model.launch().picker().expect("picker remains");
    assert_eq!(after_first.current_dir(), root);
    assert!(after_first.rows()[0].expanded());
    assert_eq!(after_first.selection(), Some(0));
    assert!(after_first.rows().iter().any(|row| row.name() == "nested"));

    picker.render_events_at(vec![egui::Event::PointerMoved(position)], 1.05);
    picker.render_events_at(vec![pointer_button(position, true)], 1.1);
    picker.render_events_at(vec![pointer_button(position, false)], 1.1);
    let after_double = picker.model.launch().picker().expect("picker remains");
    assert_eq!(after_double.current_dir(), root);
    assert!(
        after_double.rows()[0].expanded(),
        "a double-click must not toggle the row twice"
    );
    assert!(after_double.rows().iter().any(|row| row.name() == "nested"));
    assert_eq!(after_double.selection(), Some(0));
    assert!(picker.model.is_picker());
    assert!(picker.model.workbench().is_none());
}

#[test]
fn picker_current_path_chip_is_selected_and_fits() {
    let temp = tempfile::tempdir().unwrap();
    let mut start = temp.path().to_path_buf();
    for index in 0..5 {
        start.push(format!("segment-{index}-with-a-long-name"));
    }
    fs::create_dir_all(&start).unwrap();
    let full = fs::canonicalize(&start).unwrap().display().to_string();
    for (width, height) in [(1280.0, 800.0), (720.0, 480.0)] {
        let mut picker = picker_headless(&start, width, height);
        picker.ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.interaction.tooltip_delay = 0.0
        });
        let output = picker.render();
        let chip = picker.response(picker_path_id()).rect;
        let refresh = picker.response(picker_control_id("refresh")).rect;
        assert!(
            chip.left() > refresh.right(),
            "path chip must not overlap Up/Refresh at {width}x{height}: {chip:?} vs {refresh:?}"
        );
        let card = largest_rect(&output, made_app::theme::BG_CARD).expect("picker card");
        assert!(
            card.contains_rect(chip),
            "path chip must stay inside the card at {width}x{height}: {chip:?} vs {card:?}"
        );
        let shape = rect_shapes(&output)
            .into_iter()
            .find(|shape| shape.rect == chip && shape.fill == made_app::theme::BG_SELECTED)
            .expect("path chip must use the inset BG_SELECTED fill");
        assert_eq!(shape.corner_radius, egui::CornerRadius::same(4));
        assert_eq!(
            shape.stroke,
            egui::Stroke::NONE,
            "path chip must not paint a border"
        );
        assert!(
            text_colors(&output, "segment", 250.0).contains(&made_app::theme::TEXT_PRIMARY),
            "path chip text must be primary at {width}x{height}"
        );
        let hovered = output_text(&picker.hover_until_text(picker_path_id(), &full));
        assert!(
            hovered.contains(&full),
            "path chip must keep the full-path tooltip at {width}x{height}: {hovered}"
        );
    }
}

#[test]
fn picker_up_reroots_list_to_top_and_clears_tree_state() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("parent");
    let root = parent.join("root");
    fs::create_dir_all(&root).unwrap();
    for index in 0..40 {
        fs::create_dir(root.join(format!("dir-{index:02}"))).unwrap();
        fs::create_dir(parent.join(format!("sibling-{index:02}"))).unwrap();
    }
    for index in 0..20 {
        fs::create_dir(root.join(format!("dir-20/child-{index:02}"))).unwrap();
    }
    let root = fs::canonicalize(&root).unwrap();
    let parent = fs::canonicalize(&parent).unwrap();
    let mut picker = picker_headless(&root, 900.0, 620.0);
    picker.render();
    picker.model.picker_select(20).unwrap();
    picker.render();
    picker.render();
    let target_row = egui::Id::unique(("made-picker-row", 20));
    assert!(
        picker.ctx.read_response(target_row).is_some(),
        "scroll-to-selection must expose the target row"
    );
    picker.click(target_row);
    let expanded = picker.model.launch().picker().expect("picker remains");
    assert!(expanded.rows()[20].expanded());
    assert!(expanded.rows().iter().any(|row| row.name() == "child-00"));
    let list_rect = picker.response(picker_list_id()).rect;
    let expanded_row = picker.response(target_row).rect;
    assert!(
        list_rect.intersects(expanded_row),
        "expanding must scroll the selected row into view: {expanded_row:?} vs {list_rect:?}"
    );

    picker.click(picker_control_id("up"));
    let rerooted = picker.model.launch().picker().expect("picker remains");
    assert_eq!(rerooted.current_dir(), parent);
    assert_eq!(rerooted.selection(), None);
    assert!(rerooted.rows().iter().all(|row| row.depth() == 0));
    assert!(rerooted.rows().iter().all(|row| !row.expanded()));

    let output = picker.render();
    let list_rect = picker.response(picker_list_id()).rect;
    let first_row = egui::Id::unique(("made-picker-row", 0));
    let first_rect = picker.response(first_row).rect;
    assert!(
        (first_rect.top() - list_rect.top()).abs() < 1.5,
        "the re-rooted list must start at the top: {first_rect:?} vs {list_rect:?}"
    );
    let text = output_text(&output);
    assert!(
        text.contains("sibling-00"),
        "the parent's first children must be rendered: {text}"
    );

    let mut backspace = picker_headless(&root, 900.0, 620.0);
    backspace.focus(picker_list_id());
    backspace.press_key(egui::Key::ArrowDown);
    backspace.press_key(egui::Key::Backspace);
    let keyed = backspace.model.launch().picker().expect("picker remains");
    assert_eq!(keyed.current_dir(), parent);
    assert_eq!(keyed.selection(), None);
    assert!(keyed.rows().iter().all(|row| row.depth() == 0));
}

#[test]
fn picker_close_without_workspace_falls_back_to_home() {
    if isolated("picker_close_without_workspace_falls_back_to_home") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("alpha")).unwrap();
    let mut closed = picker_headless(temp.path(), 900.0, 620.0);
    closed.model.picker_select(0).unwrap();
    closed.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    let picker_output = closed.render();
    let text = output_text(&picker_output);
    assert!(
        !rects_with_fill(&picker_output, made_app::theme::BG_SELECTED).is_empty(),
        "selected picker row must keep its rounded background"
    );
    let secondary = closed.response(picker_control_id("selected")).rect;
    assert!(
        rects_with_fill(&picker_output, made_app::theme::BG_ELEVATED)
            .iter()
            .any(|rect| rect.intersects(secondary)),
        "quiet secondary action must use the elevated background"
    );
    assert!(
        !rect_shapes(&picker_output).iter().any(|rect| {
            rect.rect.intersects(secondary) && rect.stroke.color == made_app::theme::BORDER_CONTROL
        }),
        "Open selected folder must be borderless"
    );
    let selected_row = rects_with_fill(&picker_output, made_app::theme::BG_SELECTED)
        .into_iter()
        .max_by(|left, right| left.area().total_cmp(&right.area()))
        .expect("selected picker row");
    assert!((selected_row.height() - 30.0).abs() < 1.0);
    assert_eq!(
        rect_shapes(&picker_output)
            .into_iter()
            .find(|shape| shape.rect == selected_row)
            .expect("selected picker row shape")
            .corner_radius,
        egui::CornerRadius::same(6)
    );
    assert!(
        !rects_with_fill(&picker_output, made_app::theme::ACCENT)
            .iter()
            .any(|rect| rect.width() <= 3.0 && rect.height() > 20.0),
        "selected picker row must not paint a left accent bar"
    );
    assert!(
        text.contains(made_app::theme::WORDMARK),
        "picker wordmark missing"
    );
    assert!(
        text.contains(made_app::APP_VERSION),
        "picker version missing"
    );
    let close = closed.response(picker_control_id("close"));
    assert!(
        close.rect.width() >= 24.0 && close.rect.height() >= 24.0,
        "picker close target below 24px: {:?}",
        close.rect
    );
    let hovered = output_text(&closed.hover_until_text(picker_control_id("close"), "Close picker"));
    assert!(
        hovered.contains("Close picker"),
        "picker close tooltip missing: {hovered}"
    );
    closed.click(picker_control_id("close"));
    assert!(!closed.model.is_picker(), "Close must dismiss the picker");
    assert_eq!(
        closed.model.launch().source(),
        WorkspaceSource::DefaultHome,
        "Close without a workspace must open the default home"
    );
    assert!(
        closed.model.launch().surfaces().has_terminal(),
        "home fallback must have a terminal surface"
    );
    assert!(closed.model.start_pending(), "home fallback must start");

    let mut escaped = picker_headless(temp.path(), 900.0, 620.0);
    escaped.render();
    escaped.press_key(egui::Key::Escape);
    assert!(!escaped.model.is_picker(), "Escape must dismiss the picker");
    assert_eq!(
        escaped.model.launch().source(),
        WorkspaceSource::DefaultHome,
        "Escape without a workspace must open the default home"
    );
    assert!(escaped.model.start_pending());
}

#[test]
fn optional_session_map_appears_only_after_session_data() {
    if isolated("optional_session_map_appears_only_after_session_data") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "sessions", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    wait_explorer_ready(&mut headless, id);
    headless.render();
    assert!(
        headless
            .ctx
            .read_response(top_workspace_action_id(id, "sessions"))
            .is_none(),
        "Sessions control must be absent without session data"
    );
    assert!(!headless.model.workspace(id).unwrap().sessions_visible());

    let options = bridge_options(headless.model.workspace(id).unwrap()).expect("bridge options");
    let mut socket = connect(&options);
    send_state(&mut socket, 1, &root);
    service_bridge_heartbeat(socket);
    headless.pump_until(|model| !model.workspace(id).unwrap().session_map().is_empty());
    headless.render();
    assert!(
        headless
            .ctx
            .read_response(top_workspace_action_id(id, "sessions"))
            .is_some(),
        "Sessions control must appear when session data exists"
    );
    let sessions = headless
        .response(top_workspace_action_id(id, "sessions"))
        .rect;
    let ssh = headless.response(top_action_id("ssh-servers")).rect;
    let explorer = headless
        .response(top_workspace_action_id(id, "explorer"))
        .rect;
    assert!(
        headless
            .ctx
            .read_response(top_action_id("diagnostics"))
            .is_none(),
        "the header must not render a Diagnostics icon"
    );
    assert!(
        sessions.right() < explorer.left() && explorer.left() > sessions.left(),
        "right icon order must be Sessions then Explorer (Explorer rightmost): sessions {sessions:?}, explorer {explorer:?}"
    );
    assert!(
        sessions.right() <= ssh.left(),
        "Sessions must not overlap the SSH control: sessions {sessions:?}, ssh {ssh:?}"
    );
    assert!(
        ssh.right() <= explorer.left(),
        "the SSH control must precede Explorer without overlap: ssh {ssh:?}, explorer {explorer:?}"
    );
    let hovered = output_text(
        &headless.hover_until_text(top_workspace_action_id(id, "sessions"), "Show sessions"),
    );
    assert!(
        hovered.contains("Show sessions"),
        "Sessions tooltip missing: {hovered}"
    );
    headless.click(top_workspace_action_id(id, "sessions"));
    assert!(headless.model.workspace(id).unwrap().sessions_visible());
    let text = output_text(&headless.render());
    assert!(
        text.contains("Owned session"),
        "session title missing: {text}"
    );
    assert!(!text.contains("No open sessions"));

    let beta = fixture_workspace(temp.path(), "sessions-beta", "beta.rs");
    let beta_id = open_workspace(&mut headless.model, &beta);
    wait_explorer_ready(&mut headless, beta_id);
    headless.render();
    assert_eq!(headless.model.active_workspace_id(), Some(beta_id));
    assert!(!headless
        .model
        .workspace(beta_id)
        .unwrap()
        .sessions_visible());
    assert!(
        headless
            .ctx
            .read_response(top_workspace_action_id(beta_id, "sessions"))
            .is_none(),
        "another workspace must not inherit session data or the map control"
    );
    let beta_text = output_text(&headless.render());
    assert!(!beta_text.contains("Owned session"));
    headless.click(workspace_tab_id(id));
    assert!(headless.model.workspace(id).unwrap().sessions_visible());
    let alpha_text = output_text(&headless.render());
    assert!(alpha_text.contains("Owned session"));
}

#[test]
fn compact_layout_keeps_essential_controls_inside_the_window() {
    if isolated("compact_layout_keeps_essential_controls_inside_the_window") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "compact", "file.txt");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 720.0, 480.0);
    wait_explorer_ready(&mut headless, id);
    let output = headless.render();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(720.0, 480.0));
    let open_folder = headless.response(top_action_id("open-folder"));
    assert!(
        screen.contains_rect(open_folder.rect),
        "open-folder control is clipped: {:?}",
        open_folder.rect
    );
    assert!(open_folder.rect.width() >= 24.0 && open_folder.rect.height() >= 24.0);
    assert!(
        headless
            .ctx
            .read_response(top_action_id("diagnostics"))
            .is_none(),
        "the header must not render a Diagnostics icon"
    );
    let wordmark = headless.response(made_app::theme::wordmark_id());
    assert!(
        screen.contains_rect(wordmark.rect),
        "wordmark trigger is clipped: {:?}",
        wordmark.rect
    );
    assert!(wordmark.rect.width() >= 24.0 && wordmark.rect.height() >= 24.0);
    {
        let response = headless.response(top_workspace_action_id(id, "explorer"));
        assert!(
            screen.contains_rect(response.rect),
            "explorer control is clipped: {:?}",
            response.rect
        );
        assert!(response.rect.width() >= 24.0 && response.rect.height() >= 24.0);
    }
    let tab = headless.response(workspace_tab_id(id));
    assert!(
        screen.contains_rect(tab.rect),
        "tab clipped: {:?}",
        tab.rect
    );
    let explorer_card = largest_rect(&output, made_app::theme::BG_CARD_GLASS).unwrap();
    assert!(
        (explorer_card.height() - 436.0).abs() < 2.0,
        "compact explorer must fill main height: {explorer_card:?}"
    );
    assert!(
        (explorer_card.width() - 288.0).abs() < 2.0,
        "compact explorer default width: {explorer_card:?}"
    );
}

#[test]
fn picker_focused_buttons_own_enter_and_list_keys_navigate_only_from_the_list() {
    let temp = tempfile::tempdir().unwrap();
    let temp_root = temp.path().canonicalize().unwrap();
    let alpha = temp_root.join("alpha");
    fs::create_dir(&alpha).unwrap();

    let mut up = picker_headless(&alpha, 900.0, 620.0);
    up.render();
    up.focus(picker_control_id("up"));
    up.press_key(egui::Key::Enter);
    assert_eq!(up.model.launch().picker().unwrap().current_dir(), temp_root);
    assert!(up.model.is_picker());

    let mut refresh = picker_headless(&temp_root, 900.0, 620.0);
    refresh.render();
    let gamma = temp_root.join("gamma");
    fs::create_dir(&gamma).unwrap();
    refresh.focus(picker_control_id("refresh"));
    refresh.press_key(egui::Key::Enter);
    let refreshed = output_text(&refresh.render());
    assert!(
        refreshed.contains("gamma"),
        "refresh did not re-list: {refreshed}"
    );

    let mut backspace_button = picker_headless(&alpha, 900.0, 620.0);
    backspace_button.render();
    backspace_button.focus(picker_control_id("up"));
    backspace_button.press_key(egui::Key::Backspace);
    assert_eq!(
        backspace_button
            .model
            .launch()
            .picker()
            .unwrap()
            .current_dir(),
        temp_root,
        "Backspace must re-root even when a picker button has focus"
    );

    let mut list = picker_headless(&temp_root, 900.0, 620.0);
    list.render();
    list.focus(picker_list_id());
    list.press_key(egui::Key::ArrowDown);
    assert_eq!(list.model.launch().picker().unwrap().selection(), Some(0));
    list.press_key(egui::Key::Enter);
    let toggled = list.model.launch().picker().expect("picker remains");
    assert_eq!(
        toggled.current_dir(),
        temp_root,
        "Enter must not re-root the picker"
    );
    assert!(toggled.rows()[0].expanded());

    let mut button_switch = picker_headless(&temp_root, 900.0, 620.0);
    button_switch.render();
    button_switch.focus(picker_control_id("up"));
    button_switch.press_key(egui::Key::ArrowDown);
    assert_eq!(
        button_switch.model.launch().picker().unwrap().selection(),
        None,
        "a focused button must not let global arrows move the list"
    );

    let mut selected = picker_headless(&temp_root, 900.0, 620.0);
    selected.model.picker_select(0).unwrap();
    selected.render();
    selected.focus(picker_control_id("selected"));
    selected.press_key(egui::Key::Enter);
    assert!(!selected.model.is_picker());
    assert_eq!(
        selected.model.launch().explorer_root(),
        Some(alpha.as_path())
    );

    let mut current = picker_headless(&temp_root, 900.0, 620.0);
    current.render();
    current.focus(picker_control_id("current"));
    current.press_key(egui::Key::Enter);
    assert!(!current.model.is_picker());
    assert_eq!(
        current.model.launch().explorer_root(),
        Some(temp_root.as_path())
    );
}

#[test]
fn picker_virtualizes_thousands_of_entries() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..3_000 {
        fs::create_dir(temp.path().join(format!("folder-{index:04}"))).unwrap();
    }
    let mut headless = picker_headless(temp.path(), 720.0, 480.0);
    let first = headless.render();
    let first_time = headless.started.elapsed();
    let second = headless.render();
    let second_time = headless.started.elapsed().saturating_sub(first_time);
    assert!(
        text_shape_count(&first) < 100,
        "picker rendered too many rows: {} text shapes",
        text_shape_count(&first)
    );
    assert_eq!(text_shape_count(&first), text_shape_count(&second));
    assert!(
        first_time < Duration::from_secs(2) && second_time < Duration::from_secs(1),
        "picker frames were slow: first={first_time:?} second={second_time:?}"
    );
    let text = output_text(&first);
    assert!(text.contains("folder-0000"));
    assert!(
        !text.contains("folder-0500"),
        "off-screen rows inside the cap must not be rendered"
    );
    assert!(text.contains("more are not listed"));
    let picker = headless.model.launch().picker().unwrap();
    assert_eq!(
        picker.entries().len(),
        made_app::workspace_picker::MAX_PICKER_ENTRIES
    );
    assert_eq!(
        picker.truncated_entries(),
        3_000 - made_app::workspace_picker::MAX_PICKER_ENTRIES
    );
    headless.model.picker_select(2).unwrap();
    let selected_text = output_text(&headless.render());
    assert!(selected_text.contains("folder-0002"));
    headless.focus(picker_list_id());
    headless.press_key(egui::Key::ArrowDown);
    assert_eq!(
        headless.model.launch().picker().unwrap().selection(),
        Some(3)
    );
    headless.press_key(egui::Key::ArrowUp);
    assert_eq!(
        headless.model.launch().picker().unwrap().selection(),
        Some(2)
    );
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(720.0, 480.0));
    let list_rect = headless.response(picker_list_id()).rect;
    assert!(
        list_rect.height() <= 160.5,
        "short-window picker list cap was {}",
        list_rect.height()
    );
    for action in ["up", "refresh", "current", "selected"] {
        let rect = headless.response(picker_control_id(action)).rect;
        assert!(
            screen.contains_rect(rect),
            "picker {action} control clipped: {rect:?}"
        );
    }
}

#[test]
fn picker_primary_names_the_current_target_and_empty_folders_confirm() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("alpha")).unwrap();
    let mut picker = picker_headless(temp.path(), 720.0, 480.0);
    let output = picker.render();
    let text = output_text(&output);
    assert!(text.contains("Use current folder"));
    assert!(text.contains("Open selected folder"));
    assert_eq!(
        picker_current_target(&picker.model),
        Some(temp.path().canonicalize().unwrap().display().to_string())
    );
    let secondary = picker.response(picker_control_id("selected")).rect;
    assert!(
        text_colors(&output, "Open selected folder", 400.0)
            .contains(&made_app::theme::TEXT_DISABLED),
        "Open selected folder must be disabled with no selected row"
    );
    picker.click(picker_control_id("selected"));
    assert!(
        picker.model.is_picker(),
        "a disabled Open selected folder must not start a workspace"
    );
    picker.model.picker_select(0).unwrap();
    picker.render_events(vec![egui::Event::PointerMoved(egui::pos2(4.0, 4.0))]);
    let enabled = picker.render();
    assert!(
        rects_with_fill(&enabled, made_app::theme::BG_ELEVATED)
            .iter()
            .any(|rect| rect.contains_rect(secondary)),
        "Open selected folder must use BG_ELEVATED once a row is selected"
    );
    assert!(
        text_colors(&enabled, "Open selected folder", 400.0)
            .contains(&made_app::theme::TEXT_SECONDARY),
        "Open selected folder must leave the disabled tone when enabled"
    );

    let empty = tempfile::tempdir().unwrap();
    let mut empty_picker = picker_headless(empty.path(), 720.0, 480.0);
    empty_picker.render();
    assert_eq!(
        empty_picker.model.launch().picker().unwrap().selection(),
        None
    );
    empty_picker.focus(picker_control_id("current"));
    empty_picker.press_key(egui::Key::Enter);
    assert!(!empty_picker.model.is_picker());
    assert_eq!(
        empty_picker.model.launch().explorer_root(),
        Some(empty.path().canonicalize().unwrap().as_path())
    );
}

#[test]
fn ssh_servers_window_keeps_field_focus_and_has_no_collapse() {
    if isolated("ssh_servers_window_keeps_field_focus_and_has_no_collapse") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "ssh-focus", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.click(top_action_id("ssh-servers"));
    let opened = headless.render();
    let text = output_text(&opened);
    assert!(text.contains("Servidores SSH"), "{text}");
    assert!(text.contains("Display name"), "{text}");
    assert!(text.contains("Host or SSH alias"), "{text}");
    let window = headless
        .ctx
        .memory(|memory| memory.area_rect(ssh_servers_window_id()))
        .expect("SSH window rect");
    let title =
        egui::Rect::from_min_max(window.min, egui::pos2(window.right(), window.top() + 36.0));
    assert!(
        triangles_in(&opened, title).is_empty(),
        "Servidores SSH must not paint a collapse triangle"
    );
    let close = short_segments_in(&opened, title)
        .into_iter()
        .find(|segment| segment[0].x > window.center().x && segment[1].x > window.center().x)
        .expect("close button still paints in the title bar");

    let name = ssh_field_id("name");
    let host = ssh_field_id("host");
    let user = ssh_field_id("user");
    let port = ssh_field_id("port");
    let key = ssh_field_id("key");
    let directory = ssh_field_id("directory");
    let name_rect = headless.response(name).rect;
    let host_rect = headless.response(host).rect;
    let user_rect = headless.response(user).rect;
    let port_rect = headless.response(port).rect;
    let key_rect = headless.response(key).rect;
    let directory_rect = headless.response(directory).rect;
    for rect in [host_rect, user_rect, key_rect, directory_rect] {
        assert!(
            (rect.left() - name_rect.left()).abs() < 1.0,
            "fields must share a left edge: {name_rect:?} vs {rect:?}"
        );
    }
    assert!(
        (port_rect.right() - name_rect.right()).abs() < 1.0,
        "Port must align to the form's right edge: {port_rect:?} vs {name_rect:?}"
    );
    let pair_gap = host_rect.top() - name_rect.bottom();
    let later_gap = directory_rect.top() - key_rect.bottom();
    assert!(
        pair_gap >= 8.0 && (pair_gap - later_gap).abs() < 2.0,
        "field pairs must keep one Calm Editor rhythm: {pair_gap} vs {later_gap}"
    );

    headless.click(name);
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(name),
        "clicking Display name must focus that field"
    );
    for _ in 0..3 {
        headless.render();
        assert_eq!(
            headless.ctx.memory(|memory| memory.focused()),
            Some(name),
            "the terminal must not reclaim a focused SSH field"
        );
    }
    let pane = headless.model.workspace(id).unwrap().pane_id().to_owned();
    headless.ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::unique(("made-terminal-focus-boost", pane.as_str())),
            4u8,
        );
    });
    headless.render();
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(name),
        "a terminal focus boost must not steal an active SSH field"
    );
    headless.click(top_workspace_action_id(id, "explorer"));
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(name),
        "hiding the explorer must not hand focus back to the terminal"
    );

    headless.press_key(egui::Key::Tab);
    headless.render();
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(host),
        "Tab must move into the next SSH field and stay there"
    );

    let close_at = egui::pos2(
        (close[0].x + close[1].x) * 0.5,
        (close[0].y + close[1].y) * 0.5,
    );
    headless.render_events(vec![egui::Event::PointerMoved(close_at)]);
    headless.render_events(vec![pointer_button(close_at, true)]);
    headless.render_events(vec![pointer_button(close_at, false)]);
    // The close click and the terminal focus handback can settle on different
    // frames depending on the host. Render settling frames, bounded, until the
    // handback has happened before checking.
    headless.render();
    let mut closed = headless.render();
    for _ in 0..30 {
        let handed_back = headless
            .model
            .workspace(id)
            .and_then(|workbench| workbench.terminal())
            .is_some_and(|terminal| terminal.is_focused());
        if handed_back && !output_text(&closed).contains("Display name") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        closed = headless.render();
    }
    assert!(
        !output_text(&closed).contains("Display name"),
        "the window must still close: {}",
        output_text(&closed)
    );
    assert!(
        headless
            .model
            .workspace(id)
            .unwrap()
            .terminal()
            .unwrap()
            .is_focused(),
        "closing the window must return focus to the terminal"
    );
}

fn assert_large_ssh_window_scrolls(width: f32, height: f32) {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "ssh-large", "main.rs");
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([]),
    );
    let id = open_workspace(&mut model, &root);
    let servers = add_ssh_inventory(&mut model, 80);
    let last = *servers.last().expect("inventory");
    let first = servers[0];
    let mut headless = Headless::new(model, width, height);
    wait_explorer_ready(&mut headless, id);
    headless.click(top_action_id("ssh-servers"));
    headless.render();
    headless.render();

    let screen = headless.ctx.content_rect();
    let window = headless
        .ctx
        .memory(|memory| memory.area_rect(ssh_servers_window_id()))
        .expect("SSH window rect");
    assert!(
        screen.contains_rect(window),
        "{width}x{height}: the SSH window must stay inside the viewport: {window:?} vs {screen:?}"
    );

    // The bottom actions are reachable by scrolling the bounded body.
    wheel_over(&mut headless, window.center(), egui::vec2(0.0, -20_000.0));
    for action in ["ssh-save", "ssh-test", "ssh-remove"] {
        let rect = headless.response(top_action_id(action)).rect;
        assert!(
            window.contains_rect(rect) && screen.contains_rect(rect),
            "{width}x{height}: {action} must be reachable inside the window: {rect:?} vs {window:?}"
        );
    }

    // Keyboard focus on the last row scrolls it into the bounded body.
    headless.focus(ssh_server_row_id(last));
    headless.advance(0.6);
    let last_rect = headless.response(ssh_server_row_id(last)).rect;
    assert!(
        window.contains_rect(last_rect),
        "{width}x{height}: the last server row must scroll into the window: {last_rect:?} vs {window:?}"
    );

    // Shift+Tab from the first text field enters the inventory at the end;
    // arrow keys then move between rows without the old focus hold stealing it
    // back.
    headless.focus(ssh_field_id("name"));
    headless.render();
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(ssh_field_id("name")),
        "{width}x{height}: the name field must own focus"
    );
    headless.press_key_with_modifiers(egui::Key::Tab, egui::Modifiers::SHIFT);
    headless.render();
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(ssh_server_row_id(last)),
        "{width}x{height}: Shift+Tab from the fields must enter the rows"
    );
    headless.press_key(egui::Key::ArrowUp);
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(ssh_server_row_id(servers[servers.len() - 2])),
        "{width}x{height}: ArrowUp must move to the previous server row"
    );

    // Forward Tab from the action row enters the inventory instead of leaving
    // the dialog.
    headless.focus(top_action_id("ssh-remove"));
    headless.press_key(egui::Key::Tab);
    headless.render();
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(ssh_server_row_id(first)),
        "{width}x{height}: Tab from the actions must enter the first server row"
    );

    // Escape closes the management window and returns focus to the terminal.
    headless.press_key(egui::Key::Escape);
    headless.render();
    headless.render();
    assert!(
        !output_text(&headless.render()).contains("Display name"),
        "{width}x{height}: Escape must close the SSH management window"
    );
    assert!(
        headless
            .model
            .workspace(id)
            .unwrap()
            .terminal()
            .unwrap()
            .is_focused(),
        "{width}x{height}: closing the window must return focus to the terminal"
    );

    // Reopen: the management window starts at the top, so the first server row
    // is visible instead of inheriting the previous session's scroll offset.
    headless.click(top_action_id("ssh-servers"));
    headless.render();
    headless.render();
    let reopened = headless
        .ctx
        .memory(|memory| memory.area_rect(ssh_servers_window_id()))
        .expect("reopened SSH window rect");
    let first_rect = headless.response(ssh_server_row_id(first)).rect;
    assert!(
        reopened.contains_rect(first_rect) && first_rect.top() < reopened.top() + 96.0,
        "{width}x{height}: reopening must show the first server row at the top: {first_rect:?} vs {reopened:?}"
    );
}

fn ssh_window_rect(headless: &Headless) -> egui::Rect {
    headless
        .ctx
        .memory(|memory| memory.area_rect(ssh_servers_window_id()))
        .expect("SSH window rect")
}

/// Releasing a text-field hold into Save/Test/Remove or a server row must not
/// forget the window rect, recenter it, or zero its measured body height.
fn assert_ssh_window_stays_put_on_hold_release(width: f32, height: f32) {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "ssh-hold-release", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let servers = add_ssh_inventory(&mut model, 6);
    let first = servers[0];
    let mut headless = Headless::new(model, width, height);
    wait_explorer_ready(&mut headless, id);
    headless.click(top_action_id("ssh-servers"));
    headless.render();
    headless.render();
    let before = ssh_window_rect(&headless);
    assert!(before.height() > 0.0, "{width}x{height}: {before:?}");

    // Click a server row while its field owns focus.
    let name = ssh_field_id("name");
    headless.focus(name);
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(name),
        "{width}x{height}: the name field must own focus"
    );
    headless.click(ssh_server_row_id(first));
    let after_row_click = ssh_window_rect(&headless);
    assert_eq!(
        after_row_click, before,
        "{width}x{height}: clicking a server row must not collapse or recenter the window"
    );

    // Tab from the field into Save, then Test, Remove, and the inventory row.
    headless.focus(name);
    headless.render();
    let mut reached = Vec::new();
    for _ in 0..16 {
        headless.press_key(egui::Key::Tab);
        headless.render();
        let focused = headless.ctx.memory(|memory| memory.focused());
        if let Some(focused) = focused {
            if focused == top_action_id("ssh-save") {
                reached.push("save");
            } else if focused == top_action_id("ssh-test") {
                reached.push("test");
            } else if focused == top_action_id("ssh-remove") {
                reached.push("remove");
            }
        }
        if reached.len() == 3 {
            break;
        }
        assert!(
            headless
                .ctx
                .memory(|memory| memory.area_rect(ssh_servers_window_id()))
                .is_some_and(|current| current == before),
            "{width}x{height}: Tab release changed the window rect to {:?} from {before:?}",
            ssh_window_rect(&headless)
        );
    }
    assert_eq!(
        reached,
        vec!["save", "test", "remove"],
        "{width}x{height}: Tab must reach each action without losing the window"
    );
    assert_eq!(
        ssh_window_rect(&headless),
        before,
        "{width}x{height}: tabbing through actions must not collapse or recenter the window"
    );

    // The next Tab enters the inventory and keeps the same window rect.
    headless.press_key(egui::Key::Tab);
    headless.render();
    assert_eq!(
        headless.ctx.memory(|memory| memory.focused()),
        Some(ssh_server_row_id(first)),
        "{width}x{height}: Tab from Remove must enter the first server row"
    );
    assert_eq!(
        ssh_window_rect(&headless),
        before,
        "{width}x{height}: entering a row must not collapse or recenter the window"
    );
}

#[test]
fn ssh_servers_window_keeps_rect_on_field_hold_release_at_1280x800() {
    if isolated("ssh_servers_window_keeps_rect_on_field_hold_release_at_1280x800") {
        return;
    }
    assert_ssh_window_stays_put_on_hold_release(1280.0, 800.0);
}

#[test]
fn ssh_servers_window_keeps_rect_on_field_hold_release_at_720x480() {
    if isolated("ssh_servers_window_keeps_rect_on_field_hold_release_at_720x480") {
        return;
    }
    assert_ssh_window_stays_put_on_hold_release(720.0, 480.0);
}

#[test]
fn ssh_server_rows_expose_accessible_button_label_and_selection() {
    if isolated("ssh_server_rows_expose_accessible_button_label_and_selection") {
        return;
    }
    use made_app::ssh_servers::ServerMetadata;

    let format_controls = "\u{202e}\u{200b}\u{feff}\u{2066}";
    let long = format!(
        "Provisioning {format_controls}{}",
        "long-server-name-".repeat(6)
    );
    assert!(long.chars().count() <= 128);
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "ssh-accessible", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let long_id = model
        .add_ssh_metadata(ServerMetadata {
            display_name: long.clone(),
            host: "long-server.example".to_owned(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("long server saves");
    let other_id = model
        .add_ssh_metadata(ServerMetadata {
            display_name: "Short server".to_owned(),
            host: "short-server.example".to_owned(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("short server saves");
    model.select_ssh_server(long_id);

    let mut headless = Headless::new(model, 1280.0, 800.0);
    headless.ctx.enable_accesskit();
    wait_explorer_ready(&mut headless, id);
    headless.click(top_action_id("ssh-servers"));
    headless.render();
    let output = headless.render();

    let row_id = ssh_server_row_id(long_id);
    let row_rect = headless.response(row_id).rect;
    let painted = text_in_rect(&output, row_rect);
    let truncated = format!(
        "{} · not tested",
        made_app::sanitize::sanitize_display(&long, 48)
    );
    assert!(
        painted.contains(&truncated) && painted.contains('…'),
        "a long server name must ellipsize in the row: {painted:?}"
    );
    assert!(
        !painted.contains(&long),
        "the row must not paint the unbounded long name: {painted:?}"
    );

    let (role, label, selected) = accesskit_row_state(&output, row_id);
    assert_eq!(
        role,
        egui::Role::Button,
        "custom server rows must expose the Button role"
    );
    let expected_label = format!("{} · not tested", made_app::sanitize::sanitize_text(&long));
    assert_eq!(
        label.as_deref(),
        Some(expected_label.as_str()),
        "the accessible label carries the full untruncated name"
    );
    for (surface, text) in [
        ("visible", painted.as_str()),
        ("accessible", label.as_deref().expect("accessible label")),
    ] {
        // The painted-text collector joins rows with a trailing newline that is
        // not part of the row's name; every other control is a real failure.
        let checked = text.strip_suffix('\n').unwrap_or(text);
        assert_eq!(
            made_app::sanitize::sanitize_text(checked),
            checked,
            "the {surface} server name must not carry control or format characters"
        );
        for control in ['\u{202e}', '\u{200b}', '\u{feff}', '\u{2066}'] {
            assert!(
                !text.contains(control),
                "the {surface} server name must not carry {control:?}: {text:?}"
            );
        }
    }
    use egui::accesskit::Toggled;

    assert_eq!(
        selected,
        Some(Toggled::True),
        "the selected server row must expose selected state"
    );

    let (_, _, other_selected) = accesskit_row_state(&output, ssh_server_row_id(other_id));
    assert_eq!(
        other_selected,
        Some(Toggled::False),
        "an unselected server row must expose selected = false"
    );

    headless.model.select_ssh_server(other_id);
    let output = headless.render();
    let (_, _, long_selected) = accesskit_row_state(&output, row_id);
    let (_, _, other_selected) = accesskit_row_state(&output, ssh_server_row_id(other_id));
    assert_eq!(
        long_selected,
        Some(Toggled::False),
        "selection must move off the long row"
    );
    assert_eq!(
        other_selected,
        Some(Toggled::True),
        "selection must move to the chosen row"
    );
}

#[test]
fn ssh_server_rows_activate_with_enter_and_space_and_load_metadata() {
    if isolated("ssh_server_rows_activate_with_enter_and_space_and_load_metadata") {
        return;
    }
    use made_app::ssh_servers::ServerMetadata;

    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "ssh-activation", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    // Distinct host aliases prove which selected record loaded into the form.
    let servers = (0..3)
        .map(|index| {
            model
                .add_ssh_metadata(ServerMetadata {
                    display_name: format!("Plant {index:03}"),
                    host: format!("plant-{index:03}.example"),
                    user: None,
                    port: None,
                    key_file: None,
                    initial_directory: None,
                })
                .expect("server metadata saves")
        })
        .collect::<Vec<_>>();
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.click(top_action_id("ssh-servers"));
    headless.render();
    headless.render();

    headless.focus(ssh_server_row_id(servers[1]));
    headless.press_key(egui::Key::Enter);
    headless.render();
    let enter_text = output_text(&headless.render());
    assert!(
        enter_text.contains("plant-001.example"),
        "Enter must load the selected server metadata into the form: {enter_text}"
    );

    headless.focus(ssh_server_row_id(servers[0]));
    headless.press_key(egui::Key::Space);
    headless.render();
    let space_text = output_text(&headless.render());
    assert!(
        space_text.contains("plant-000.example"),
        "Space must load the selected server metadata into the form: {space_text}"
    );
    assert!(
        !space_text.contains("plant-001.example"),
        "Space must replace the previous selection: {space_text}"
    );
}

#[test]
fn ssh_management_actions_save_test_remove_have_model_effects() {
    if isolated("ssh_management_actions_save_test_remove_have_model_effects") {
        return;
    }
    use made_remote::protocol::{Capabilities, RemoteTarget};

    let capabilities = |opencode| Capabilities {
        target: RemoteTarget::LinuxX64,
        tmux: false,
        opencode,
        shell_cwd: false,
        helper_version: "probe".to_owned(),
        grok: false,
        python3: false,
    };
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "ssh-actions", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let servers = add_ssh_inventory(&mut model, 2);
    let first = servers[0];
    let selected = servers[1];
    model
        .hold_ssh_probe_result(first, Ok(capabilities(true)))
        .expect("first probe result records");
    model
        .hold_ssh_probe_result(selected, Ok(capabilities(false)))
        .expect("second probe result records");
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.click(top_action_id("ssh-servers"));
    headless.render();
    headless.render();

    // Each server paints its own probe result instead of a global guess.
    let probe_text = output_text(&headless.render());
    assert!(
        probe_text.contains("Plant 000 · OpenCode detected"),
        "the first server keeps its own probe text: {probe_text}"
    );
    assert!(
        probe_text.contains("Plant 001 · OpenCode not detected"),
        "the second server keeps its own probe text: {probe_text}"
    );

    // Save persists the edited draft through the model, not just the widget.
    headless.model.select_ssh_server(selected);
    headless.render();
    let name = ssh_field_id("name");
    headless.click(name);
    headless.press_key_with_modifiers(egui::Key::A, egui::Modifiers::COMMAND);
    headless.render_events(vec![egui::Event::Text("Renamed Plant".to_owned())]);
    headless.render();
    headless.click(top_action_id("ssh-save"));
    assert_eq!(
        headless.model.ssh_notice(),
        Some("Saved. Open workspaces keep their original server."),
        "Save must report the model's edit"
    );
    let saved = output_text(&headless.render());
    assert!(
        saved.contains("Renamed Plant · not tested"),
        "Save must persist the edited server record: {saved}"
    );

    // Test starts the bounded probe through the model and says so.
    headless.click(top_action_id("ssh-test"));
    assert_eq!(
        headless.model.ssh_notice(),
        Some("Testing the connection. OpenCode will not be started."),
        "Test must start the selected server's probe"
    );

    // Remove deletes the selected record and resets the selection.
    headless.click(top_action_id("ssh-remove"));
    assert_eq!(
        headless.model.ssh_notice(),
        Some("Server removed. Open workspaces were not closed."),
        "Remove must report the model's deletion"
    );
    let removed = output_text(&headless.render());
    assert!(
        !removed.contains("Renamed Plant"),
        "Remove must delete the saved server row: {removed}"
    );
    assert!(
        removed.contains("Plant 000"),
        "Remove must keep the other server: {removed}"
    );
    assert!(
        !removed.contains("plant-000.example"),
        "Remove must clear the selected server's draft: {removed}"
    );
}

#[test]
fn ssh_servers_window_scrolls_large_inventory_at_1280x800() {
    if isolated("ssh_servers_window_scrolls_large_inventory_at_1280x800") {
        return;
    }
    assert_large_ssh_window_scrolls(1280.0, 800.0);
}

#[test]
fn ssh_servers_window_scrolls_large_inventory_at_720x480() {
    if isolated("ssh_servers_window_scrolls_large_inventory_at_720x480") {
        return;
    }
    assert_large_ssh_window_scrolls(720.0, 480.0);
}

fn assert_large_new_tab_menu_scrolls(width: f32, height: f32) {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "menu-large", "main.rs");
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([OPENCODE]),
    );
    let id = open_workspace(&mut model, &root);
    let servers = add_ssh_inventory(&mut model, 80);
    let last = *servers.last().expect("inventory");
    let last_menu = new_tab_menu_item_id(&format!("ssh-terminal-{}", authority_hex(last)));
    let mut headless = Headless::new(model, width, height);
    wait_explorer_ready(&mut headless, id);

    headless.click(top_action_id("open-folder"));
    headless.settle_animation();
    let screen = headless.ctx.content_rect();
    let popup = headless
        .ctx
        .memory(|memory| memory.area_rect(top_action_id("open-folder").with("popup")))
        .expect("menu area");
    assert!(
        screen.contains_rect(popup),
        "{width}x{height}: the large menu must be capped inside the viewport: {popup:?} vs {screen:?}"
    );
    let terminal = headless.response(new_tab_menu_item_id("terminal")).rect;
    let opencode = headless.response(new_tab_menu_item_id("opencode")).rect;
    assert!(
        terminal.top() < opencode.top(),
        "{width}x{height}: Terminal must stay first"
    );
    assert!(
        (terminal.height() - opencode.height()).abs() < 0.5
            && (terminal.width() - opencode.width()).abs() < 0.75,
        "{width}x{height}: Terminal and OpenCode rows must keep their geometry: {terminal:?} {opencode:?}"
    );

    // A wheel over the menu scrolls every remote row into view.
    wheel_over(&mut headless, terminal.center(), egui::vec2(0.0, -20_000.0));
    let last_rect = headless.response(last_menu).rect;
    assert!(
        screen.contains_rect(last_rect),
        "{width}x{height}: the wheel must scroll the last remote row into view: {last_rect:?} vs {screen:?}"
    );

    // Reopen and walk the menu with the keyboard to the last remote row.
    headless.press_key(egui::Key::Escape);
    headless.render();
    headless.focus(top_action_id("open-folder"));
    headless.press_key(egui::Key::Enter);
    headless.render();
    let reopened_terminal = headless.response(new_tab_menu_item_id("terminal")).rect;
    let reopened_opencode = headless.response(new_tab_menu_item_id("opencode")).rect;
    assert!(
        screen.contains_rect(reopened_terminal) && screen.contains_rect(reopened_opencode),
        "{width}x{height}: reopening must show Terminal and OpenCode at the top: {reopened_terminal:?} {reopened_opencode:?} vs {screen:?}"
    );
    assert!(
        reopened_terminal.top() < reopened_opencode.top(),
        "{width}x{height}: reopening must keep Terminal above OpenCode"
    );
    headless.press_key(egui::Key::ArrowDown);
    headless.render();
    let mut reached = false;
    for _ in 0..servers.len() + 3 {
        if headless.ctx.memory(|memory| memory.focused()) == Some(last_menu) {
            reached = true;
            break;
        }
        headless.press_key(egui::Key::ArrowDown);
        headless.render();
    }
    assert!(
        reached,
        "{width}x{height}: arrow keys must reach the last remote row"
    );
    headless.advance(0.6);
    let last_rect = headless.response(last_menu).rect;
    assert!(
        screen.contains_rect(last_rect),
        "{width}x{height}: keyboard focus must scroll the last remote row into view: {last_rect:?} vs {screen:?}"
    );
}

#[test]
fn new_tab_menu_scrolls_large_inventory_at_1280x800() {
    if isolated("new_tab_menu_scrolls_large_inventory_at_1280x800") {
        return;
    }
    assert_large_new_tab_menu_scrolls(1280.0, 800.0);
}

#[test]
fn new_tab_menu_scrolls_large_inventory_at_720x480() {
    if isolated("new_tab_menu_scrolls_large_inventory_at_720x480") {
        return;
    }
    assert_large_new_tab_menu_scrolls(720.0, 480.0);
}

#[test]
fn new_tab_menu_scrolls_large_inventory_at_short_viewport() {
    if isolated("new_tab_menu_scrolls_large_inventory_at_short_viewport") {
        return;
    }
    // A shorter supported viewport must bound the menu to the space that is
    // actually left instead of reserving three rows past the bottom edge.
    assert_large_new_tab_menu_scrolls(720.0, 360.0);
}

#[test]
fn new_tab_menu_short_inventory_stays_compact() {
    if isolated("new_tab_menu_short_inventory_stays_compact") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "menu-short", "main.rs");
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([OPENCODE]),
    );
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1100.0, 700.0);
    wait_explorer_ready(&mut headless, id);

    headless.click(top_action_id("open-folder"));
    headless.settle_animation();
    let screen = headless.ctx.content_rect();
    let terminal = headless.response(new_tab_menu_item_id("terminal")).rect;
    let opencode = headless.response(new_tab_menu_item_id("opencode")).rect;
    let popup = headless
        .ctx
        .memory(|memory| memory.area_rect(top_action_id("open-folder").with("popup")))
        .expect("menu area");
    assert!(
        (terminal.height() - opencode.height()).abs() < 0.5
            && (terminal.width() - opencode.width()).abs() < 0.75,
        "the short menu must keep its row geometry: {terminal:?} {opencode:?}"
    );
    assert!(
        popup.height() < terminal.height() * 3.0 + 40.0,
        "a short menu must not reserve an empty scroll viewport: {popup:?}"
    );
    assert!(
        screen.contains_rect(popup),
        "a short menu must stay compact inside the viewport: {popup:?} vs {screen:?}"
    );
}

fn triangles_in(output: &egui::FullOutput, bounds: egui::Rect) -> Vec<egui::Rect> {
    fn visit(shape: &egui::Shape, bounds: egui::Rect, found: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Path(path) if path.points.len() == 3 => {
                let rect = egui::Rect::from_points(&path.points);
                if bounds.contains_rect(rect) {
                    found.push(rect);
                }
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, bounds, found);
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &output.shapes {
        visit(&clipped.shape, bounds, &mut found);
    }
    found
}

fn short_segments_in(output: &egui::FullOutput, bounds: egui::Rect) -> Vec<[egui::Pos2; 2]> {
    fn visit(shape: &egui::Shape, bounds: egui::Rect, found: &mut Vec<[egui::Pos2; 2]>) {
        match shape {
            egui::Shape::LineSegment { points, .. } => {
                let rect = egui::Rect::from_two_pos(points[0], points[1]);
                let length = (points[1] - points[0]).length();
                if length < 20.0 && bounds.intersects(rect) {
                    found.push(*points);
                }
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, bounds, found);
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &output.shapes {
        visit(&clipped.shape, bounds, &mut found);
    }
    found
}

fn grok_home_dir() -> PathBuf {
    let dir = PathBuf::from(std::env::var_os("HOME").expect("isolated home")).join(".grok");
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_grok_inventory(rows: &[Value]) {
    fs::write(
        grok_home_dir().join("active_sessions.json"),
        serde_json::to_string(rows).unwrap(),
    )
    .unwrap();
}

fn append_grok_hook(line: &Value) {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(grok_home_dir().join("made-grok-status.jsonl"))
        .unwrap();
    writeln!(file, "{}", serde_json::to_string(line).unwrap()).unwrap();
}

#[test]
fn grok_tab_paints_working_attention_and_idle_dots_with_grok_build() {
    if isolated("grok_tab_paints_working_attention_and_idle_dots_with_grok_build") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "project-alpha", "main.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    let pid = headless
        .model
        .workspace(id)
        .and_then(Workbench::terminal)
        .and_then(made_app::TerminalPane::pid)
        .expect("shell pid");
    let row = |session_id: &str, opened_at: &str| {
        json!({
            "pid": pid,
            "session_id": session_id,
            "cwd": "/tmp/made-grok-missing",
            "opened_at": opened_at
        })
    };
    write_grok_inventory(&[
        row("work", "2024-01-01T00:00:01Z"),
        row("ask", "2024-01-01T00:00:02Z"),
        row("idle", "2024-01-01T00:00:03Z"),
    ]);
    append_grok_hook(&json!({
        "sessionId": "history",
        "cwd": "/tmp/made-grok-history",
        "event": "UserPromptSubmit",
        "status": "working"
    }));
    // The first read skips lines already on disk. Later lines are the live ones.
    headless.render();

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        headless.render();
        let tab = headless.response(workspace_tab_id(id)).rect;
        if tab_session_dots(&headless.render(), tab).len() == 3 || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let idle_output = headless.render();
    let tab = headless.response(workspace_tab_id(id)).rect;
    let idle_dots = tab_session_dots(&idle_output, tab);
    assert_eq!(
        idle_dots.len(),
        3,
        "inventory-only sessions must show one idle dot each, and history must not add one"
    );
    for dot in &idle_dots {
        assert_session_dot_color(dot, made_app::theme::TEXT_TERTIARY, "an inventory idle dot");
        assert!((dot.radius - 2.0).abs() < 0.1, "dot diameter must stay 4px");
    }

    append_grok_hook(&json!({
        "sessionId": "work",
        "cwd": "/tmp/made-grok-missing",
        "event": "UserPromptSubmit",
        "status": "working"
    }));
    append_grok_hook(&json!({
        "sessionId": "ask",
        "cwd": "/tmp/made-grok-missing",
        "event": "PreToolUse",
        "status": "attention"
    }));
    append_grok_hook(&json!({
        "sessionId": "idle",
        "cwd": "/tmp/made-grok-missing",
        "event": "SessionStart",
        "status": "idle"
    }));
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(id)).rect;
    let dots = tab_session_dots(&output, tab);
    assert_eq!(dots.len(), 3, "each open Grok session paints one dot");
    assert_session_dot_color(&dots[0], made_app::theme::ACCENT, "the working dot");
    assert_session_dot_color(&dots[1], made_app::theme::WARNING, "the attention dot");
    assert_session_dot_color(&dots[2], made_app::theme::TEXT_TERTIARY, "the idle dot");
    for (index, dot) in dots.iter().enumerate() {
        assert!(
            (dot.center - session_dot_center(tab, index, 3)).length() < 0.75,
            "dot {index} must keep the OpenCode row placement"
        );
    }
    let label = text_rects(&output, "Grok Build", 55.0)
        .into_iter()
        .find(|rect| tab.contains_rect(*rect))
        .expect("an open Grok tab must be labeled Grok Build");
    assert!(
        (label.left() - (tab.left() + 10.0 + 22.0)).abs() < 1.0,
        "the label follows the 16px icon and its 6px gap: {label:?} in {tab:?}"
    );
    assert!(
        text_rects(&output, "project-alpha", 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "an open Grok tab must not keep the folder label"
    );
    assert!(
        text_rects(&output, "OpenCode", 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "a Grok tab must not take the OpenCode label"
    );
    let grok_image = painted_images(&output)
        .into_iter()
        .find(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && tab.contains_rect(image.rect)
                && image.rect.right() <= label.left() - 1.0
        })
        .expect("an open Grok tab must paint grok.svg in the 16px leading slot");
    let grok_px = (output.pixels_per_point * grok_image.rect.size()).round();
    assert_eq!(
        grok_image.texture,
        embedded_svg_texture(
            &headless.ctx,
            "bytes://grok.svg",
            include_bytes!("../assets/grok.svg"),
            grok_px.x as u32,
            grok_px.y as u32,
        ),
        "the tab's 16px image must match grok.svg bytes"
    );
    assert_eq!(
        grok_image.tint,
        egui::Color32::WHITE,
        "the Grok mark keeps its shipped colors"
    );
    assert!(
        (grok_image.rect.center().x - (tab.left() + 10.0 + 8.0)).abs() < 1.0
            && (grok_image.rect.center().y - tab.center().y).abs() < 1.0,
        "grok.svg must be centered in the leading slot: {:?}",
        grok_image.rect
    );
    assert!(
        !rect_shapes(&output).into_iter().any(|shape| {
            (shape.rect.width() - 14.0).abs() < 0.75
                && (shape.rect.height() - 11.0).abs() < 0.75
                && tab.contains_rect(shape.rect)
                && shape.rect.right() <= label.left() - 2.0
        }),
        "an open Grok tab must not keep the terminal glyph"
    );

    let period = 1.8_f64;
    let next_period = ((headless.last_time / period).floor() + 1.0) * period;
    let bright = session_dot_alpha(
        &tab_session_dots(
            &headless.render_events_at(Vec::new(), next_period + period * 0.25),
            tab,
        )[0],
    );
    let dim = session_dot_alpha(
        &tab_session_dots(
            &headless.render_events_at(Vec::new(), next_period + period * 0.75),
            tab,
        )[0],
    );
    assert!(
        bright > dim,
        "the working dot must pulse: bright {bright}, dim {dim}"
    );

    let path = root.display().to_string();
    let hovered = output_text(&headless.hover_until_text(
        workspace_tab_id(id),
        "Sessions: 1 running, 1 needs attention, 1 idle",
    ));
    assert!(
        hovered.contains("Sessions: 1 running, 1 needs attention, 1 idle"),
        "the tooltip must count working, attention, and idle: {hovered}"
    );
    assert!(
        !hovered.contains(&path),
        "a Grok Build tab must omit the workspace path: {hovered}"
    );

    append_grok_hook(&json!({
        "sessionId": "ask",
        "cwd": "/tmp/made-grok-missing",
        "event": "SessionEnd",
        "status": "idle"
    }));
    std::thread::sleep(Duration::from_millis(1100));
    let ended = headless.render();
    let tab = headless.response(workspace_tab_id(id)).rect;
    let ended_dots = tab_session_dots(&ended, tab);
    assert_eq!(
        ended_dots.len(),
        2,
        "session end removes that session's dot and inventory must not put it back"
    );
    assert!(
        text_rects(&ended, "Grok Build", 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "a tab that still has Grok sessions stays labeled Grok Build"
    );
    assert_session_dot_color(
        &ended_dots[0],
        made_app::theme::ACCENT,
        "work stays working",
    );
    assert_session_dot_color(
        &ended_dots[1],
        made_app::theme::TEXT_TERTIARY,
        "idle stays idle",
    );

    append_grok_hook(&json!({
        "sessionId": "idle",
        "cwd": "/tmp/made-grok-missing",
        "event": "StopFailure",
        "status": "error"
    }));
    let parallel = headless.render();
    let tab = headless.response(workspace_tab_id(id)).rect;
    let parallel_dots = tab_session_dots(&parallel, tab);
    assert_eq!(parallel_dots.len(), 2);
    assert_session_dot_color(
        &parallel_dots[0],
        made_app::theme::ACCENT,
        "another session's event must not change the working dot",
    );
    assert_session_dot_color(
        &parallel_dots[1],
        made_app::theme::WARNING,
        "only the idle session becomes attention",
    );

    append_grok_hook(&json!({
        "sessionId": "work",
        "cwd": "/tmp/made-grok-missing",
        "event": "SessionEnd",
        "status": "idle"
    }));
    append_grok_hook(&json!({
        "sessionId": "idle",
        "cwd": "/tmp/made-grok-missing",
        "event": "SessionEnd",
        "status": "idle"
    }));
    std::thread::sleep(Duration::from_millis(1100));
    let closed = headless.render();
    let tab = headless.response(workspace_tab_id(id)).rect;
    assert!(
        tab_session_dots(&closed, tab).is_empty(),
        "ending every Grok session removes the dots"
    );
    let folder = text_rects(&closed, "project-alpha", 55.0)
        .into_iter()
        .find(|rect| tab.contains_rect(*rect))
        .expect("a closed Grok tab must return to the folder label");
    assert!(
        text_rects(&closed, "Grok Build", 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "a closed Grok tab must not keep the Grok Build label"
    );
    assert!(
        !painted_images(&closed).into_iter().any(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && tab.contains_rect(image.rect)
        }),
        "a closed Grok tab must drop grok.svg"
    );
    let terminal_window = rect_shapes(&closed)
        .into_iter()
        .find(|shape| {
            (shape.rect.width() - 14.0).abs() < 0.75
                && (shape.rect.height() - 11.0).abs() < 0.75
                && shape.stroke.color == made_app::theme::TEXT_PRIMARY
                && tab.contains_rect(shape.rect)
                && shape.rect.right() <= folder.left() - 2.0
        })
        .map(|shape| shape.rect)
        .expect("terminal glyph in the 16px leading slot");
    assert!(
        (terminal_window.center().x - (tab.left() + 10.0 + 8.0)).abs() < 1.0,
        "the terminal glyph must return to the leading slot: {terminal_window:?}"
    );
    let restored = output_text(&headless.hover_until_text(workspace_tab_id(id), &path));
    assert!(
        restored.contains(&path),
        "the folder tab must restore its path tooltip: {restored}"
    );
}

#[cfg(unix)]
#[test]
fn running_grok_tui_shows_grok_build_before_a_session_and_reverts_on_exit() {
    if isolated("running_grok_tui_shows_grok_build_before_a_session_and_reverts_on_exit") {
        return;
    }
    use std::os::unix::fs::PermissionsExt;

    let bin = PathBuf::from(std::env::var_os("HOME").expect("isolated home")).join("bin");
    let executable = bin.join("grok");
    fs::write(&executable, "#!/bin/sh\nexec sleep 60\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();

    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "project-alpha", "main.rs");
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([GROK]),
    );
    let shell_id = open_workspace(&mut model, &root);
    model.begin_tui_workspace(GROK);
    model.ensure_workbench();
    let grok_id = model.active_workspace_id().expect("grok tab");
    assert_ne!(grok_id, shell_id);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    headless.pump_until(|model| {
        model
            .workspace(grok_id)
            .and_then(Workbench::terminal)
            .is_some_and(made_app::TerminalPane::is_running)
    });
    headless.render();
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(grok_id)).rect;
    let label = text_rects(&output, "Grok Build", 55.0)
        .into_iter()
        .find(|rect| tab.contains_rect(*rect))
        .expect("a running Grok TUI must be labeled Grok Build before any session");
    let folder = {
        let workbench = headless.model.workspace(grok_id).unwrap();
        workbench
            .launch_plan()
            .map(|plan| plan.workspace.display_name().to_owned())
            .unwrap_or_else(|| {
                workbench
                    .shell_cwd()
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Workspace".to_owned())
            })
    };
    let path = headless
        .model
        .workspace(grok_id)
        .unwrap()
        .shell_cwd()
        .display()
        .to_string();
    assert_ne!(folder, "Grok Build");
    assert!(
        text_rects(&output, &folder, 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "a running Grok TUI must not keep the folder label {folder}"
    );
    let grok_image = painted_images(&output)
        .into_iter()
        .find(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && tab.contains_rect(image.rect)
                && image.rect.right() <= label.left() - 1.0
        })
        .expect("a running Grok TUI must paint grok.svg before any session");
    let grok_px = (output.pixels_per_point * grok_image.rect.size()).round();
    assert_eq!(
        grok_image.texture,
        embedded_svg_texture(
            &headless.ctx,
            "bytes://grok.svg",
            include_bytes!("../assets/grok.svg"),
            grok_px.x as u32,
            grok_px.y as u32,
        ),
        "the leading slot must use grok.svg"
    );
    headless.hover(workspace_tab_id(grok_id));
    headless.advance(0.2);
    let no_session = output_text(&headless.render());
    assert!(
        !no_session.contains(&path) && !no_session.contains("Sessions:"),
        "a Grok Build tab with no sessions must show no parent tooltip: {no_session}"
    );

    let pid = headless
        .model
        .workspace(grok_id)
        .and_then(Workbench::terminal)
        .and_then(made_app::TerminalPane::pid)
        .expect("grok pid");
    let status = std::process::Command::new("kill")
        .arg(pid.to_string())
        .status()
        .expect("kill grok");
    assert!(status.success(), "the running Grok child must be stoppable");
    headless.pump_until(|model| {
        !model
            .workspace(grok_id)
            .and_then(Workbench::terminal)
            .is_some_and(made_app::TerminalPane::is_running)
    });
    let closed = headless.render();
    let tab = headless.response(workspace_tab_id(grok_id)).rect;
    let folder_label = text_rects(&closed, &folder, 55.0)
        .into_iter()
        .find(|rect| tab.contains_rect(*rect))
        .expect("an exited Grok TUI with no sessions must return to the folder label");
    assert!(
        text_rects(&closed, "Grok Build", 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "an exited Grok TUI must drop the Grok Build label"
    );
    assert!(
        !painted_images(&closed).into_iter().any(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && tab.contains_rect(image.rect)
        }),
        "an exited Grok TUI must drop grok.svg"
    );
    let terminal_window = rect_shapes(&closed)
        .into_iter()
        .find(|shape| {
            (shape.rect.width() - 14.0).abs() < 0.75
                && (shape.rect.height() - 11.0).abs() < 0.75
                && shape.stroke.color == made_app::theme::TEXT_PRIMARY
                && tab.contains_rect(shape.rect)
                && shape.rect.right() <= folder_label.left() - 2.0
        })
        .map(|shape| shape.rect)
        .expect("terminal glyph after Grok exits");
    assert!(
        (terminal_window.center().x - (tab.left() + 10.0 + 8.0)).abs() < 1.0,
        "the terminal glyph must return to the leading slot: {terminal_window:?}"
    );
    let restored = output_text(&headless.hover_until_text(workspace_tab_id(grok_id), &path));
    assert!(
        restored.contains(&path),
        "the folder tab must restore its path tooltip: {restored}"
    );
}

#[test]
fn an_opencode_tab_keeps_its_dots_and_root_when_grok_events_arrive() {
    if isolated("an_opencode_tab_keeps_its_dots_and_root_when_grok_events_arrive") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "opencode-owned", "main.rs");
    let grok_dir = temp.path().join("grok-dir");
    fs::create_dir(&grok_dir).unwrap();
    let grok_dir = grok_dir.canonicalize().unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    let options = bridge_options(headless.model.workspace(id).unwrap()).expect("bridge options");
    let mut socket = connect(&options);
    headless.pump_until(|model| model.workspace(id).unwrap().is_connected());
    send_sessions_state(
        &mut socket,
        1,
        &root,
        &[
            ("busy-session", true, false),
            ("idle-session", false, false),
        ],
    );
    headless.pump_until(|model| model.workspace(id).unwrap().session_map().rows().len() == 2);
    append_grok_hook(&json!({
        "sessionId": "grok-1",
        "cwd": grok_dir,
        "event": "UserPromptSubmit",
        "status": "attention"
    }));
    headless.render();
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(id)).rect;
    let dots = tab_session_dots(&output, tab);
    assert_eq!(dots.len(), 2, "OpenCode dots must ignore Grok events");
    assert_session_dot_color(&dots[0], made_app::theme::ACCENT, "the OpenCode busy dot");
    assert_session_dot_color(
        &dots[1],
        made_app::theme::TEXT_TERTIARY,
        "the OpenCode idle dot",
    );
    assert!(
        text_rects(&output, "OpenCode", 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "the connected tab must stay labeled OpenCode"
    );
    assert!(
        text_rects(&output, "Grok Build", 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "OpenCode must keep the tab when Grok sessions exist"
    );
    let opencode_icon = icon_named("opencode.svg");
    let icon_image = painted_images(&output)
        .into_iter()
        .find(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && tab.contains_rect(image.rect)
        })
        .expect("the connected tab must keep opencode.svg");
    assert_eq!(
        icon_image.texture,
        icon_texture(&headless.ctx, opencode_icon),
        "Grok sessions must not replace the OpenCode icon"
    );
    assert_eq!(
        headless.model.workspace(id).unwrap().explorer_root(),
        Some(root.as_path()),
        "Grok must not replace the OpenCode explorer root"
    );
}

fn menu_model(root: &Path, tuis: &[IntegrationRef]) -> (Headless, WorkspaceId) {
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis(tuis.iter().copied()),
    );
    let id = open_workspace(&mut model, root);
    let mut headless = Headless::new(model, 1100.0, 700.0);
    wait_explorer_ready(&mut headless, id);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    (headless, id)
}

#[test]
fn header_plus_without_a_detected_tui_shows_no_menu() {
    if isolated("header_plus_without_a_detected_tui_shows_no_menu") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "no-tui", "main.rs");
    let (mut headless, id) = menu_model(&root, &[]);
    headless.pump_until(|model| {
        model
            .workspace(id)
            .and_then(Workbench::terminal)
            .is_some_and(made_app::TerminalPane::is_running)
    });
    headless.click(top_action_id("open-folder"));
    assert!(headless.model.is_picker());
    assert_eq!(headless.model.workspace_count(), 1);
    assert!(headless.model.workspace(id).is_some());
    assert!(
        headless
            .model
            .workspace(id)
            .and_then(Workbench::terminal)
            .is_some_and(made_app::TerminalPane::is_running),
        "opening the picker must leave the local terminal running"
    );
    assert!(headless
        .ctx
        .read_response(new_tab_menu_item_id("terminal"))
        .is_none());
    assert!(headless
        .ctx
        .read_response(new_tab_menu_item_id("opencode"))
        .is_none());
    assert!(headless
        .ctx
        .read_response(new_tab_menu_item_id("grok"))
        .is_none());
}

#[test]
fn catalog_menu_lists_terminal_then_opencode_then_grok() {
    if isolated("catalog_menu_lists_terminal_then_opencode_then_grok") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "both", "main.rs");
    let (mut headless, _) = menu_model(&root, &[OPENCODE, GROK]);
    let hovered =
        output_text(&headless.hover_until_text(top_action_id("open-folder"), "Open folder"));
    assert!(
        hovered.contains("Open folder"),
        "the + tooltip and accessible name stay Open folder: {hovered}"
    );
    headless.click(top_action_id("open-folder"));
    let terminal = headless.response(new_tab_menu_item_id("terminal")).rect;
    let opencode = headless.response(new_tab_menu_item_id("opencode")).rect;
    let grok = headless.response(new_tab_menu_item_id("grok")).rect;
    assert!(
        terminal.top() < opencode.top() && opencode.top() < grok.top(),
        "menu order must be Terminal, OpenCode, Grok: {terminal:?} {opencode:?} {grok:?}"
    );
    assert!(!headless.model.is_picker());
    assert_eq!(headless.model.workspace_count(), 1);
}

#[test]
fn only_grok_omits_opencode_from_the_menu() {
    if isolated("only_grok_omits_opencode_from_the_menu") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "grok-only", "main.rs");
    let (mut headless, _) = menu_model(&root, &[GROK]);
    headless.click(top_action_id("open-folder"));
    let terminal = headless.response(new_tab_menu_item_id("terminal")).rect;
    let grok = headless.response(new_tab_menu_item_id("grok")).rect;
    assert!(terminal.top() < grok.top());
    assert!(headless
        .ctx
        .read_response(new_tab_menu_item_id("opencode"))
        .is_none());
    let text = output_text(&headless.render());
    assert!(text.contains("Terminal"));
    assert!(
        text.contains("Grok Build"),
        "the Grok row must show the full product label: {text}"
    );
}

#[test]
fn choosing_grok_starts_a_home_tab_without_a_picker_or_stopping_the_current_one() {
    if isolated("choosing_grok_starts_a_home_tab_without_a_picker_or_stopping_the_current_one") {
        return;
    }
    use made_platform::Platform;

    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "current", "main.rs");
    let (mut headless, id) = menu_model(&root, &[GROK]);
    let home = made_platform::host_platform()
        .home_dir()
        .expect("test home")
        .canonicalize()
        .expect("canonical test home");
    headless.click(top_action_id("open-folder"));
    headless.click(new_tab_menu_item_id("grok"));
    assert!(!headless.model.is_picker());
    assert_eq!(headless.model.active_workspace_id(), Some(id));
    assert_eq!(headless.model.workspace_count(), 1);
    assert!(headless.model.start_pending());
    assert_eq!(headless.model.terminal_target(), TerminalTarget::Tui(GROK));
    assert_eq!(
        headless.model.launch().source(),
        WorkspaceSource::DefaultHome
    );
    assert_eq!(headless.model.launch().terminal_cwd(), Some(home.as_path()));
    assert!(headless
        .model
        .workspace(id)
        .and_then(Workbench::terminal)
        .is_some());
}

#[test]
fn terminal_row_still_opens_the_folder_picker_when_grok_is_listed() {
    if isolated("terminal_row_still_opens_the_folder_picker_when_grok_is_listed") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "terminal-row", "main.rs");
    let (mut headless, id) = menu_model(&root, &[OPENCODE, GROK]);
    headless.click(top_action_id("open-folder"));
    headless.click(new_tab_menu_item_id("terminal"));
    assert!(headless.model.is_picker());
    assert_eq!(headless.model.workspace_count(), 1);
    assert!(headless
        .model
        .workspace(id)
        .and_then(Workbench::terminal)
        .is_some());
    assert_eq!(headless.model.terminal_target(), TerminalTarget::Shell);
}

#[test]
fn grok_row_paints_the_terminal_glyph_and_opencode_keeps_its_svg() {
    if isolated("grok_row_paints_the_terminal_glyph_and_opencode_keeps_its_svg") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "glyphs", "main.rs");
    let (mut headless, _) = menu_model(&root, &[OPENCODE, GROK]);
    headless.ctx.set_pixels_per_point(2.0);
    headless.click(top_action_id("open-folder"));
    headless.advance(0.3);
    let output = headless.render();
    let text = output_text(&output);
    assert!(
        text.contains("Grok Build"),
        "missing Grok Build label: {text}"
    );
    assert!(text.contains("OpenCode"), "missing OpenCode label: {text}");
    let grok = headless.response(new_tab_menu_item_id("grok")).rect;
    let opencode = headless.response(new_tab_menu_item_id("opencode")).rect;
    let grok_label = painted_menu_label(&output, "Grok Build", grok);
    let grok_image = painted_images(&output)
        .into_iter()
        .find(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && grok.contains_rect(image.rect)
                && image.rect.right() <= grok_label.line.left() - 1.0
        })
        .expect("Grok row must paint the 16px official mark");
    let grok_px = (output.pixels_per_point * grok_image.rect.size()).round();
    assert_eq!(
        grok_image.texture,
        embedded_svg_texture(
            &headless.ctx,
            "bytes://grok.svg",
            include_bytes!("../assets/grok.svg"),
            grok_px.x as u32,
            grok_px.y as u32,
        ),
        "the Grok row's 16px image must match grok.svg bytes"
    );
    assert!(
        !rect_shapes(&output).into_iter().any(|shape| {
            (shape.rect.width() - 14.0).abs() < 0.75
                && (shape.rect.height() - 11.0).abs() < 0.75
                && (shape.stroke.width - 1.5).abs() < 0.05
                && shape.corner_radius == egui::CornerRadius::same(2)
                && grok.contains_rect(shape.rect)
                && shape.rect.right() <= grok_label.line.left() - 2.0
        }),
        "the terminal-window glyph is not the Grok mark"
    );
    let opencode_icon =
        explorer_icons::icon_by_file_name("opencode.svg").expect("opencode.svg is vendored");
    let opencode_image = painted_images(&output)
        .into_iter()
        .find(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && opencode.contains_rect(image.rect)
        })
        .expect("OpenCode row must keep the 16px opencode.svg");
    let image_px = (output.pixels_per_point * opencode_image.rect.size()).round();
    assert_eq!(
        opencode_image.texture,
        icon_texture_px(
            &headless.ctx,
            opencode_icon,
            image_px.x as u32,
            image_px.y as u32,
        )
    );
}

#[cfg(unix)]
#[test]
fn detection_does_not_launch_grok_or_block_shell_startup() {
    if isolated("detection_does_not_launch_grok_or_block_shell_startup") {
        return;
    }
    use std::os::unix::fs::PermissionsExt;

    let home = PathBuf::from(std::env::var_os("HOME").expect("isolated home"));
    let session = home.join("grok-session");
    let bin = std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
        .next()
        .expect("isolated bin");
    let grok = bin.join("grok");
    std::fs::write(
        &grok,
        format!(
            "#!/bin/sh\nif [ \"$1\" = plugin ]; then exit 0; fi\nprintf launched >> '{}'\n",
            session.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&grok, std::fs::Permissions::from_mode(0o755)).unwrap();

    let model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tui_detection(),
    );
    assert!(
        model.compatible_tuis().is_empty(),
        "detection must not block construction"
    );
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "shell", "main.rs");
    let mut model = model;
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1100.0, 700.0);
    wait_explorer_ready(&mut headless, id);
    assert!(
        headless
            .model
            .workspace(id)
            .and_then(Workbench::terminal)
            .is_some(),
        "the shell must start without waiting for TUI detection"
    );
    assert_eq!(
        headless.model.workspace(id).unwrap().terminal_target(),
        TerminalTarget::Shell,
        "detection must not replace a shell tab with Grok"
    );
    headless.pump_until(|model| model.compatible_tuis() == [OPENCODE, GROK].as_slice());
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        headless.model.logic(&headless.ctx);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !session.exists(),
        "detection must not launch a grok session"
    );
    assert_eq!(headless.model.workspace_count(), 1);
}

#[cfg(unix)]
#[test]
fn explicit_grok_catalog_does_not_install_the_plugin() {
    if isolated("explicit_grok_catalog_does_not_install_the_plugin") {
        return;
    }
    use std::os::unix::fs::PermissionsExt;

    let home = PathBuf::from(std::env::var_os("HOME").expect("isolated home"));
    let sentinel = home.join("grok-install");
    let bin = std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
        .next()
        .expect("isolated bin");
    let grok = bin.join("grok");
    std::fs::write(
        &grok,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
            sentinel.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&grok, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("GROK_HOME", home.join("grok-home"));

    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "catalog", "main.rs");
    let (mut headless, _) = menu_model(&root, &[GROK]);
    headless.click(top_action_id("open-folder"));
    assert!(headless
        .ctx
        .read_response(new_tab_menu_item_id("grok"))
        .is_some());
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        headless.model.logic(&headless.ctx);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !sentinel.exists(),
        "an explicit Grok catalog must not run grok plugin install"
    );
}

const GROK_DIAGNOSTIC_SENTENCES: [&str; 7] = [
    "Checking for Grok Build.",
    "Grok Build is not on PATH.",
    "Installing the Grok Build plugin.",
    "Grok Build session follow is ready.",
    "Grok Build is not running.",
    "Grok Build session-status hooks are unavailable.",
    "Grok Build plugin was not installed for this launch.",
];

const OPENCODE_DIAGNOSTIC_SENTENCE: &str = "OpenCode is not running.";
const OPENCODE_READY_SENTENCE: &str = "OpenCode session follow is ready.";
const OPENCODE_MISSING_SENTENCE: &str = "OpenCode is not on PATH.";
const OPENCODE_WITHDRAWN_SENTENCE: &str = "Optional OpenCode integration is inactive.";

struct HitText {
    rect: egui::Rect,
    clip: egui::Rect,
    text: String,
    color: egui::Color32,
}

fn hit_texts(output: &egui::FullOutput, needle: &str) -> Vec<HitText> {
    fn visit(shape: &egui::Shape, clip: egui::Rect, needle: &str, hits: &mut Vec<HitText>) {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text.contains(needle) => {
                let color = text
                    .galley
                    .job
                    .sections
                    .first()
                    .map(|section| {
                        if section.format.color == egui::Color32::PLACEHOLDER {
                            text.fallback_color
                        } else {
                            section.format.color
                        }
                    })
                    .unwrap_or(text.fallback_color);
                hits.push(HitText {
                    rect: egui::Rect::from_min_size(text.pos, text.galley.size()),
                    clip,
                    text: text.galley.job.text.clone(),
                    color,
                });
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, clip, needle, hits);
                }
            }
            _ => {}
        }
    }
    let mut hits = Vec::new();
    for clipped in &output.shapes {
        visit(&clipped.shape, clipped.clip_rect, needle, &mut hits);
    }
    hits
}

fn assert_grok_sentences_absent(text: &str) {
    for sentence in GROK_DIAGNOSTIC_SENTENCES {
        assert!(
            !text.contains(sentence),
            "Grok diagnostics leaked {sentence:?} in {text}"
        );
    }
    assert!(
        !text.contains(OPENCODE_DIAGNOSTIC_SENTENCE),
        "OpenCode diagnostics leaked before Show diagnostics: {text}"
    );
    for pill in ["○ inactive", "● ready", "! degraded"] {
        assert!(
            !text.contains(pill),
            "status pill leaked {pill:?} in {text}"
        );
    }
}

fn pill_beside(output: &egui::FullOutput, sentence: egui::Rect, pill_text: &str) -> HitText {
    let pills: Vec<HitText> = hit_texts(output, pill_text)
        .into_iter()
        .filter(|hit| hit.text == pill_text)
        .filter(|hit| hit.rect.center().x < sentence.left())
        .filter(|hit| (hit.rect.center().y - sentence.center().y).abs() < 14.0)
        .collect();
    assert_eq!(
        pills.len(),
        1,
        "expected one {pill_text:?} beside {sentence:?}"
    );
    pills.into_iter().next().expect("pill")
}

fn assert_pill_tint(output: &egui::FullOutput, center: egui::Pos2, tint: egui::Color32) {
    let matched = rects_with_fill(output, tint)
        .into_iter()
        .any(|rect| rect.contains(center) && (rect.height() - 20.0).abs() < 1.5);
    assert!(matched, "missing pill fill {tint:?} around {center:?}");
}

fn copied_diagnostics(output: &egui::FullOutput) -> String {
    output
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .expect("copy diagnostics command")
}

/// OpenCode row that belongs to the catalog currently offered.
///
/// An explicit list that includes OpenCode keeps the inactive not-running
/// sentence. A list that omits it is the not-on-PATH row.
fn opencode_row_for_catalog(
    headless: &Headless,
) -> (&'static str, &'static str, egui::Color32, egui::Color32) {
    if headless.model.compatible_tuis().contains(&OPENCODE) {
        (
            OPENCODE_DIAGNOSTIC_SENTENCE,
            "○ inactive",
            made_app::theme::TEXT_TERTIARY,
            made_app::theme::BG_ELEVATED,
        )
    } else {
        (
            OPENCODE_MISSING_SENTENCE,
            "× unsupported",
            made_app::theme::ERROR,
            made_app::theme::ERROR_TINT,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn assert_revealed_grok_row(
    output: &egui::FullOutput,
    headless: &Headless,
    id: WorkspaceId,
    folder: &str,
    sentence: &str,
    pill_text: &str,
    tone: egui::Color32,
    tint: egui::Color32,
) {
    let text = output_text(output);
    assert_eq!(
        text.matches(sentence).count(),
        1,
        "expected one {sentence:?} in {text}"
    );
    for other in GROK_DIAGNOSTIC_SENTENCES {
        if other != sentence {
            assert!(
                !text.contains(other),
                "{sentence:?} frame also showed {other:?}: {text}"
            );
        }
    }
    let (opencode_sentence, opencode_pill, opencode_tone, opencode_tint) =
        opencode_row_for_catalog(headless);
    assert_eq!(
        text.matches(opencode_sentence).count(),
        1,
        "OpenCode sentence changed: {text}"
    );

    let pane_id = headless.model.workspace(id).unwrap().pane_id();
    let window = headless
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::unique(("made-about-window", pane_id))))
        .expect("about window");
    let mut sentences = hit_texts(output, sentence);
    sentences.retain(|hit| hit.text == sentence);
    assert_eq!(sentences.len(), 1, "expected one painted {sentence:?}");
    let hit = &sentences[0];
    assert!(
        window.expand(1.0).contains_rect(hit.rect),
        "Grok sentence is outside About: {:?} window {window:?}",
        hit.rect
    );
    // About overlays the terminal. The clip must belong to that window so the
    // header, tabs, explorer, and terminal did not paint the row.
    assert!(
        window.expand(1.0).contains_rect(hit.clip),
        "Grok sentence clip is not About: {:?} window {window:?}",
        hit.clip
    );

    let mut sections = hit_texts(output, "WORKSPACE");
    sections.retain(|hit| hit.text == "WORKSPACE");
    sections.sort_by(|left, right| left.rect.top().total_cmp(&right.rect.top()));
    let section = sections.first().expect("WORKSPACE section");
    assert!(
        hit.rect.bottom() <= section.rect.top() + 0.5,
        "Grok row must sit with the OpenCode pill, above the sectioned detail: {:?} vs {:?}",
        hit.rect,
        section.rect
    );

    let tab = headless.response(workspace_tab_id(id)).rect;
    assert!(
        !hit.rect.intersects(tab),
        "Grok sentence intersected the workspace tab"
    );
    let header_bottom =
        made_app::theme::Layout::for_window(headless.width, headless.height).content_top();
    assert!(
        hit.rect.top() >= header_bottom,
        "Grok sentence intersected the header: {:?} bottom {header_bottom}",
        hit.rect
    );
    let splitter = headless.response(explorer_splitter_id(pane_id)).rect;
    assert!(
        hit.rect.right() < splitter.left(),
        "Grok sentence intersected the explorer: {:?} splitter {:?}",
        hit.rect,
        splitter
    );
    assert!(
        hit_texts(output, folder)
            .iter()
            .any(|label| label.text == folder && tab.contains_rect(label.rect)),
        "the workspace tab label changed"
    );
    assert!(
        hit_texts(output, "Grok Build")
            .iter()
            .filter(|label| label.text == "Grok Build")
            .all(|label| !tab.intersects(label.rect)),
        "the Grok Build tab label changed"
    );

    let pill = pill_beside(output, hit.rect, pill_text);
    assert_eq!(pill.color, tone, "Grok pill {pill_text:?} tone");
    assert_pill_tint(output, pill.rect.center(), tint);
    assert!(
        window.expand(1.0).contains_rect(pill.rect),
        "Grok pill is outside About"
    );

    let opencode = hit_texts(output, opencode_sentence);
    let opencode = opencode
        .iter()
        .find(|hit| hit.text == opencode_sentence)
        .expect("OpenCode sentence");
    let opencode_pill = pill_beside(output, opencode.rect, opencode_pill);
    assert_eq!(opencode_pill.color, opencode_tone);
    assert_pill_tint(output, opencode_pill.rect.center(), opencode_tint);
    assert!(
        opencode.rect.bottom() <= section.rect.top() + 0.5,
        "OpenCode pill moved into the sectioned detail"
    );
}

fn expect_revealed_grok_row(
    headless: &mut Headless,
    id: WorkspaceId,
    tab_label: &str,
    sentence: &str,
    pill: &str,
    tone: egui::Color32,
    tint: egui::Color32,
) {
    let output = headless.render();
    assert_revealed_grok_row(&output, headless, id, tab_label, sentence, pill, tone, tint);
    assert_copied_diagnostics(headless, id, sentence);
}

fn assert_copied_diagnostics(headless: &mut Headless, id: WorkspaceId, sentence: &str) {
    let pane_id = headless.model.workspace(id).unwrap().pane_id().to_owned();
    let copied = copied_diagnostics(&headless.click(diagnostics_copy_id(&pane_id)));
    assert_eq!(
        copied.matches(sentence).count(),
        1,
        "copy missing or duplicated {sentence:?}: {copied}"
    );
    let (opencode_sentence, _, _, _) = opencode_row_for_catalog(headless);
    assert_eq!(
        copied.matches(opencode_sentence).count(),
        1,
        "OpenCode copy changed: {copied}"
    );
}

#[test]
fn grok_build_diagnostics_row_follows_detection_and_install() {
    if isolated("grok_build_diagnostics_row_follows_detection_and_install") {
        return;
    }
    // A bug that reached the real installer must not write the user's Grok home.
    std::env::remove_var("GROK_HOME");

    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "grok-diag", "main.rs");
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([OPENCODE]),
    );
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    assert!(
        headless
            .model
            .workspace(id)
            .and_then(Workbench::terminal)
            .is_some(),
        "an explicit catalog must not block the shell"
    );
    let chrome = output_text(&headless.render());
    assert_grok_sentences_absent(&chrome);
    let tab = headless.response(workspace_tab_id(id)).rect;
    assert!(
        hit_texts(&headless.render(), "grok-diag")
            .iter()
            .any(|hit| hit.text == "grok-diag" && tab.contains_rect(hit.rect)),
        "the shell tab label changed before diagnostics were revealed"
    );

    headless.click(made_app::theme::wordmark_id());
    let hidden = output_text(&headless.render());
    assert!(
        hidden.contains("Show diagnostics"),
        "About did not open: {hidden}"
    );
    assert_grok_sentences_absent(&hidden);

    let pane_id = headless.model.workspace(id).unwrap().pane_id().to_owned();
    headless.click(about_diagnostics_toggle_id(&pane_id));
    settle_about_animation(&mut headless);
    let revealed = headless.render();
    assert_revealed_grok_row(
        &revealed,
        &headless,
        id,
        "grok-diag",
        "Grok Build is not on PATH.",
        "○ inactive",
        made_app::theme::TEXT_TERTIARY,
        made_app::theme::BG_ELEVATED,
    );
    assert_copied_diagnostics(&mut headless, id, "Grok Build is not on PATH.");

    let inactive = (
        "○ inactive",
        made_app::theme::TEXT_TERTIARY,
        made_app::theme::BG_ELEVATED,
    );
    let degraded = (
        "! degraded",
        made_app::theme::WARNING,
        made_app::theme::WARNING_TINT,
    );
    let states = [
        (
            GrokDiagnosticsHold::Detecting,
            "Checking for Grok Build.",
            inactive.0,
            inactive.1,
            inactive.2,
        ),
        (
            GrokDiagnosticsHold::NotOnPath,
            "Grok Build is not on PATH.",
            inactive.0,
            inactive.1,
            inactive.2,
        ),
        (
            GrokDiagnosticsHold::Installing,
            "Installing the Grok Build plugin.",
            inactive.0,
            inactive.1,
            inactive.2,
        ),
        (
            GrokDiagnosticsHold::Installed,
            "Grok Build is not running.",
            inactive.0,
            inactive.1,
            inactive.2,
        ),
        (
            GrokDiagnosticsHold::AlreadyCurrent,
            "Grok Build is not running.",
            inactive.0,
            inactive.1,
            inactive.2,
        ),
        (
            GrokDiagnosticsHold::InstallFailed,
            "Grok Build session-status hooks are unavailable.",
            degraded.0,
            degraded.1,
            degraded.2,
        ),
    ];
    for (hold, sentence, pill, tone, tint) in states {
        grok::hold_diagnostics(&mut headless.model, hold);
        let output = headless.render();
        assert_revealed_grok_row(
            &output,
            &headless,
            id,
            "grok-diag",
            sentence,
            pill,
            tone,
            tint,
        );
        assert_copied_diagnostics(&mut headless, id, sentence);
    }

    let menu_root = fixture_workspace(temp.path(), "grok-menu", "main.rs");
    let mut menu_model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([GROK]),
    );
    let menu_id = open_workspace(&mut menu_model, &menu_root);
    let mut menu = Headless::new(menu_model, 1280.0, 800.0);
    wait_explorer_ready(&mut menu, menu_id);
    assert!(
        menu.model
            .workspace(menu_id)
            .and_then(Workbench::terminal)
            .is_some(),
        "a Grok catalog must not block the shell or install before the menu opens"
    );
    menu.click(top_action_id("open-folder"));
    let menu_output = menu.render();
    let grok_row = menu.response(new_tab_menu_item_id("grok")).rect;
    let menu_labels: Vec<String> = hit_texts(&menu_output, "Grok")
        .into_iter()
        .filter(|hit| grok_row.intersects(hit.rect))
        .map(|hit| hit.text)
        .collect();
    assert!(
        menu_labels.iter().any(|label| label == "Grok Build"),
        "the Grok Build menu row changed: {menu_labels:?}"
    );
    assert!(
        menu_labels
            .iter()
            .all(|label| !GROK_DIAGNOSTIC_SENTENCES.contains(&label.as_str())),
        "the menu row shows a diagnostics sentence: {menu_labels:?}"
    );

    menu.click(made_app::theme::wordmark_id());
    let menu_pane = menu.model.workspace(menu_id).unwrap().pane_id().to_owned();
    menu.click(about_diagnostics_toggle_id(&menu_pane));
    settle_about_animation(&mut menu);
    for _ in 0..3 {
        let output = menu.render();
        assert_revealed_grok_row(
            &output,
            &menu,
            menu_id,
            "grok-menu",
            "Grok Build plugin was not installed for this launch.",
            "○ inactive",
            made_app::theme::TEXT_TERTIARY,
            made_app::theme::BG_ELEVATED,
        );
    }
    assert_copied_diagnostics(
        &mut menu,
        menu_id,
        "Grok Build plugin was not installed for this launch.",
    );
}

#[test]
fn grok_build_diagnostics_row_is_ready_only_while_grok_is_running_in_that_workspace() {
    if isolated("grok_build_diagnostics_row_is_ready_only_while_grok_is_running_in_that_workspace")
    {
        return;
    }
    // A bug that reached the real installer must not write the user's Grok home.
    std::env::remove_var("GROK_HOME");

    let temp = tempfile::tempdir().unwrap();
    let here = fixture_workspace(temp.path(), "grok-here", "main.rs");
    let there = fixture_workspace(temp.path(), "grok-there", "main.rs");
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([OPENCODE]),
    );
    let here_id = open_workspace(&mut model, &here);
    let there_id = open_workspace(&mut model, &there);
    assert!(model.switch_workspace(here_id));
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, here_id);
    wait_explorer_ready(&mut headless, there_id);
    assert!(
        headless
            .model
            .workspace(here_id)
            .and_then(Workbench::terminal)
            .is_some_and(|terminal| terminal.is_running()),
        "the shell must be alive before a running mark is applied"
    );
    assert_grok_sentences_absent(&output_text(&headless.render()));

    open_about_diagnostics(&mut headless, here_id);

    let inactive = (
        "○ inactive",
        made_app::theme::TEXT_TERTIARY,
        made_app::theme::BG_ELEVATED,
    );
    let ready = (
        "● ready",
        made_app::theme::SUCCESS,
        made_app::theme::SUCCESS_TINT,
    );
    let degraded = (
        "! degraded",
        made_app::theme::WARNING,
        made_app::theme::WARNING_TINT,
    );

    for hold in [
        GrokDiagnosticsHold::Installed,
        GrokDiagnosticsHold::AlreadyCurrent,
    ] {
        grok::hold_diagnostics(&mut headless.model, hold);
        grok::hold_workspace_running(&mut headless.model, there_id, GrokRunningHold::LivingTarget);
        grok::hold_workspace_running(&mut headless.model, here_id, GrokRunningHold::NotRunning);
        expect_revealed_grok_row(
            &mut headless,
            here_id,
            "grok-here",
            "Grok Build is not running.",
            inactive.0,
            inactive.1,
            inactive.2,
        );

        grok::hold_workspace_running(&mut headless.model, there_id, GrokRunningHold::NotRunning);
        grok::hold_workspace_running(&mut headless.model, here_id, GrokRunningHold::LivingTarget);
        expect_revealed_grok_row(
            &mut headless,
            here_id,
            "grok-here",
            "Grok Build session follow is ready.",
            ready.0,
            ready.1,
            ready.2,
        );

        grok::hold_workspace_running(&mut headless.model, there_id, GrokRunningHold::LivingTarget);
        grok::hold_workspace_running(&mut headless.model, here_id, GrokRunningHold::OpenSession);
        expect_revealed_grok_row(
            &mut headless,
            here_id,
            "grok-here",
            "Grok Build session follow is ready.",
            ready.0,
            ready.1,
            ready.2,
        );
    }

    let unchanged = [
        (
            GrokDiagnosticsHold::Detecting,
            "Checking for Grok Build.",
            inactive.0,
            inactive.1,
            inactive.2,
        ),
        (
            GrokDiagnosticsHold::NotOnPath,
            "Grok Build is not on PATH.",
            inactive.0,
            inactive.1,
            inactive.2,
        ),
        (
            GrokDiagnosticsHold::Installing,
            "Installing the Grok Build plugin.",
            inactive.0,
            inactive.1,
            inactive.2,
        ),
        (
            GrokDiagnosticsHold::InstallFailed,
            "Grok Build session-status hooks are unavailable.",
            degraded.0,
            degraded.1,
            degraded.2,
        ),
    ];
    for running in [GrokRunningHold::LivingTarget, GrokRunningHold::NotRunning] {
        grok::hold_workspace_running(&mut headless.model, here_id, running);
        for (hold, sentence, pill, tone, tint) in unchanged {
            grok::hold_diagnostics(&mut headless.model, hold);
            expect_revealed_grok_row(
                &mut headless,
                here_id,
                "grok-here",
                sentence,
                pill,
                tone,
                tint,
            );
        }
    }

    grok::hold_diagnostics(&mut headless.model, GrokDiagnosticsHold::Installed);
    grok::hold_workspace_running(&mut headless.model, here_id, GrokRunningHold::LivingTarget);
    made_app::integrations::opencode::hold_label(&mut headless.model, here_id);
    expect_revealed_grok_row(
        &mut headless,
        here_id,
        "OpenCode",
        "Grok Build session follow is ready.",
        ready.0,
        ready.1,
        ready.2,
    );
    let labeled = headless.render();
    let tab = headless.response(workspace_tab_id(here_id)).rect;
    assert!(
        hit_texts(&labeled, "grok-here")
            .iter()
            .filter(|hit| hit.text == "grok-here")
            .all(|hit| !tab.intersects(hit.rect)),
        "OpenCode must own the tab label while Grok is running"
    );

    grok::hold_diagnostics(&mut headless.model, GrokDiagnosticsHold::AlreadyCurrent);
    grok::hold_workspace_running(&mut headless.model, here_id, GrokRunningHold::OpenSession);
    expect_revealed_grok_row(
        &mut headless,
        here_id,
        "OpenCode",
        "Grok Build session follow is ready.",
        ready.0,
        ready.1,
        ready.2,
    );

    grok::hold_workspace_running(&mut headless.model, there_id, GrokRunningHold::LivingTarget);
    grok::hold_workspace_running(&mut headless.model, here_id, GrokRunningHold::NotRunning);
    expect_revealed_grok_row(
        &mut headless,
        here_id,
        "OpenCode",
        "Grok Build is not running.",
        inactive.0,
        inactive.1,
        inactive.2,
    );

    let menu_root = fixture_workspace(temp.path(), "grok-other", "main.rs");
    let mut menu_model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([GROK]),
    );
    let menu_id = open_workspace(&mut menu_model, &menu_root);
    let mut menu = Headless::new(menu_model, 1280.0, 800.0);
    wait_explorer_ready(&mut menu, menu_id);
    grok::hold_workspace_running(&mut menu.model, menu_id, GrokRunningHold::LivingTarget);
    assert!(
        menu.model
            .workspace(menu_id)
            .and_then(Workbench::terminal)
            .is_some(),
        "a running mark must not block the shell or install"
    );
    menu.click(top_action_id("open-folder"));
    let menu_output = menu.render();
    let grok_row = menu.response(new_tab_menu_item_id("grok")).rect;
    let menu_labels: Vec<String> = hit_texts(&menu_output, "Grok")
        .into_iter()
        .filter(|hit| grok_row.intersects(hit.rect))
        .map(|hit| hit.text)
        .collect();
    assert!(
        menu_labels.iter().any(|label| label == "Grok Build"),
        "the Grok Build menu row changed: {menu_labels:?}"
    );
    assert!(
        menu_labels
            .iter()
            .all(|label| !GROK_DIAGNOSTIC_SENTENCES.contains(&label.as_str())),
        "the menu row shows a diagnostics sentence: {menu_labels:?}"
    );

    open_about_diagnostics(&mut menu, menu_id);
    for running in [GrokRunningHold::LivingTarget, GrokRunningHold::NotRunning] {
        grok::hold_workspace_running(&mut menu.model, menu_id, running);
        expect_revealed_grok_row(
            &mut menu,
            menu_id,
            "grok-other",
            "Grok Build plugin was not installed for this launch.",
            inactive.0,
            inactive.1,
            inactive.2,
        );
    }
}

const GROK_EXPLICIT_CATALOG_SENTENCE: &str = "Grok Build plugin was not installed for this launch.";

fn assert_catalog_menu_unchanged(headless: &mut Headless) {
    headless.click(top_action_id("open-folder"));
    let output = headless.render();
    let terminal = headless.response(new_tab_menu_item_id("terminal")).rect;
    let opencode = headless.response(new_tab_menu_item_id("opencode")).rect;
    let grok = headless.response(new_tab_menu_item_id("grok")).rect;
    assert!(
        terminal.top() < opencode.top() && opencode.top() < grok.top(),
        "menu order changed"
    );
    let opencode_labels: Vec<String> = hit_texts(&output, "OpenCode")
        .into_iter()
        .filter(|hit| opencode.intersects(hit.rect))
        .map(|hit| hit.text)
        .collect();
    assert!(
        opencode_labels.iter().any(|label| label == "OpenCode"),
        "the OpenCode menu row changed: {opencode_labels:?}"
    );
    for sentence in [
        OPENCODE_DIAGNOSTIC_SENTENCE,
        OPENCODE_READY_SENTENCE,
        OPENCODE_MISSING_SENTENCE,
        OPENCODE_WITHDRAWN_SENTENCE,
    ] {
        assert!(
            opencode_labels.iter().all(|label| label != sentence),
            "the OpenCode menu row shows {sentence:?}: {opencode_labels:?}"
        );
    }
    let grok_labels: Vec<String> = hit_texts(&output, "Grok")
        .into_iter()
        .filter(|hit| grok.intersects(hit.rect))
        .map(|hit| hit.text)
        .collect();
    assert!(
        grok_labels.iter().any(|label| label == "Grok Build"),
        "the Grok Build menu row changed: {grok_labels:?}"
    );
    assert!(
        grok_labels
            .iter()
            .all(|label| !GROK_DIAGNOSTIC_SENTENCES.contains(&label.as_str())),
        "the menu row shows a diagnostics sentence: {grok_labels:?}"
    );
}

#[allow(clippy::too_many_arguments)]
fn assert_opencode_diagnostics_row(
    headless: &mut Headless,
    id: WorkspaceId,
    sentence: &str,
    pill_text: &str,
    tone: egui::Color32,
    tint: egui::Color32,
    status_label: &str,
    reason_code: &str,
    grok_sentence: &str,
) {
    let snapshot = headless.model.workspace(id).unwrap().diagnostics_snapshot();
    assert_eq!(snapshot.status_label, status_label);
    assert_eq!(
        snapshot.status.label(),
        status_label,
        "machine status code changed"
    );
    assert_eq!(snapshot.reason_code, reason_code);
    assert_eq!(snapshot.reason, sentence);
    assert_eq!(snapshot.row("grok status").unwrap().sentence, grok_sentence);

    let output = headless.render();
    let text = output_text(&output);
    assert!(
        !text.contains(OPENCODE_WITHDRAWN_SENTENCE),
        "revealed row still shows the withdrawn sentence: {text}"
    );
    let pane_id = headless.model.workspace(id).unwrap().pane_id();
    let window = headless
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::unique(("made-about-window", pane_id))))
        .expect("about window");
    let mut sentences = hit_texts(&output, sentence);
    sentences.retain(|hit| hit.text == sentence);
    assert_eq!(
        sentences.len(),
        1,
        "expected one painted {sentence:?} in {text}"
    );
    let hit = &sentences[0];
    assert!(
        window.expand(1.0).contains_rect(hit.rect),
        "OpenCode sentence is outside About"
    );
    let pill = pill_beside(&output, hit.rect, pill_text);
    assert_eq!(pill.color, tone, "OpenCode pill {pill_text:?} tone");
    assert_pill_tint(&output, pill.rect.center(), tint);
    if status_label == "connected" {
        assert!(
            hit_texts(&output, "● connected")
                .iter()
                .all(|painted| painted.text != "● connected"),
            "connected OpenCode pill still says connected"
        );
    }
    let tab = headless.response(workspace_tab_id(id)).rect;
    assert!(
        hit_texts(&output, "Grok Build")
            .iter()
            .filter(|label| label.text == "Grok Build")
            .all(|label| !tab.intersects(label.rect)),
        "the Grok Build tab label changed"
    );
    for other in GROK_DIAGNOSTIC_SENTENCES {
        if other != grok_sentence {
            assert!(
                !text.contains(other),
                "Grok sentence changed to {other:?}: {text}"
            );
        }
    }

    let pane_id = headless.model.workspace(id).unwrap().pane_id().to_owned();
    let copied = copied_diagnostics(&headless.click(diagnostics_copy_id(&pane_id)));
    assert!(
        copied.contains(&format!("reason: {sentence}")),
        "copy missing {sentence:?}: {copied}"
    );
    assert!(
        copied.contains(&format!("status: {status_label}")),
        "copy status changed: {copied}"
    );
    assert!(
        copied.contains(&format!("reason code: {reason_code}")),
        "copy reason code changed: {copied}"
    );
    assert!(
        !copied.contains(OPENCODE_WITHDRAWN_SENTENCE),
        "copy still has the withdrawn sentence: {copied}"
    );
    assert!(
        copied.contains(&format!("grok: {grok_sentence}")),
        "copied Grok sentence changed: {copied}"
    );
}

/// Reveal About diagnostics without polling PATH detection.
fn reveal_diagnostics_without_logic(headless: &mut Headless, id: WorkspaceId) -> egui::FullOutput {
    fn click_ui_only(headless: &mut Headless, id: egui::Id) {
        headless.render_ui_only(Vec::new());
        headless.render_ui_only(Vec::new());
        let position = headless.response(id).rect.center();
        headless.render_ui_only(vec![egui::Event::PointerMoved(position)]);
        headless.render_ui_only(vec![pointer_button(position, true)]);
        headless.render_ui_only(vec![pointer_button(position, false)]);
    }

    click_ui_only(headless, made_app::theme::wordmark_id());
    let pane_id = headless.model.workspace(id).unwrap().pane_id().to_owned();
    click_ui_only(headless, about_diagnostics_toggle_id(&pane_id));
    let mut output = headless.render_ui_only(Vec::new());
    for _ in 0..8 {
        std::thread::sleep(Duration::from_millis(40));
        output = headless.render_ui_only(Vec::new());
    }
    output
}

#[test]
fn opencode_diagnostics_row_uses_the_grok_wording() {
    if isolated("opencode_diagnostics_row_uses_the_grok_wording") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "opencode-wording", "main.rs");
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([OPENCODE, GROK]),
    );
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    assert_catalog_menu_unchanged(&mut headless);

    open_about_diagnostics(&mut headless, id);
    let inactive = (
        "○ inactive",
        made_app::theme::TEXT_TERTIARY,
        made_app::theme::BG_ELEVATED,
    );
    let ready = (
        "● ready",
        made_app::theme::SUCCESS,
        made_app::theme::SUCCESS_TINT,
    );
    let unsupported = (
        "× unsupported",
        made_app::theme::ERROR,
        made_app::theme::ERROR_TINT,
    );
    assert_opencode_diagnostics_row(
        &mut headless,
        id,
        OPENCODE_DIAGNOSTIC_SENTENCE,
        inactive.0,
        inactive.1,
        inactive.2,
        "inactive",
        "inactive",
        GROK_EXPLICIT_CATALOG_SENTENCE,
    );

    let options = bridge_options(headless.model.workspace(id).unwrap()).expect("bridge options");
    let socket = connect(&options);
    headless.pump_until(|model| {
        let view = model.workspace(id).unwrap().diagnostics_snapshot();
        view.status_label == "connected" && view.reason_code == "connected"
    });
    assert_opencode_diagnostics_row(
        &mut headless,
        id,
        OPENCODE_READY_SENTENCE,
        ready.0,
        ready.1,
        ready.2,
        "connected",
        "connected",
        GROK_EXPLICIT_CATALOG_SENTENCE,
    );

    drop(socket);
    headless.pump_until(|model| {
        model
            .workspace(id)
            .unwrap()
            .diagnostics_snapshot()
            .reason_code
            == "bridge_closed"
    });
    let dropped = headless.model.workspace(id).unwrap().diagnostics_snapshot();
    assert!(
        dropped
            .reason
            .starts_with("The integration connection closed; waiting to reconnect while the terminal stays usable."),
        "bridge sentence changed: {}",
        dropped.reason
    );

    let socket = connect(&options);
    headless.pump_until(|model| {
        let view = model.workspace(id).unwrap().diagnostics_snapshot();
        view.status_label == "connected" && view.reason_code == "reconnected"
    });
    assert_opencode_diagnostics_row(
        &mut headless,
        id,
        OPENCODE_READY_SENTENCE,
        ready.0,
        ready.1,
        ready.2,
        "connected",
        "reconnected",
        GROK_EXPLICIT_CATALOG_SENTENCE,
    );

    drop(socket);
    headless.pump_until(|model| {
        model
            .workspace(id)
            .unwrap()
            .diagnostics_snapshot()
            .reason_code
            == "bridge_closed"
    });
    // About covers the popup's next frame. Close it before checking the menu.
    headless.click(made_app::theme::wordmark_id());
    assert_catalog_menu_unchanged(&mut headless);

    let omitted_root = fixture_workspace(temp.path(), "opencode-omitted", "main.rs");
    let omitted_later = fixture_workspace(temp.path(), "opencode-omitted-later", "main.rs");
    let mut omitted_model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([GROK]),
    );
    let omitted_id = open_workspace(&mut omitted_model, &omitted_root);
    let mut omitted = Headless::new(omitted_model, 1280.0, 800.0);
    wait_explorer_ready(&mut omitted, omitted_id);
    open_about_diagnostics(&mut omitted, omitted_id);
    assert_opencode_diagnostics_row(
        &mut omitted,
        omitted_id,
        OPENCODE_MISSING_SENTENCE,
        unsupported.0,
        unsupported.1,
        unsupported.2,
        "unsupported",
        "opencode_missing",
        GROK_EXPLICIT_CATALOG_SENTENCE,
    );
    let omitted_later_id = open_workspace(&mut omitted.model, &omitted_later);
    let omitted_still = omitted
        .model
        .workspace(omitted_id)
        .unwrap()
        .diagnostics_snapshot();
    assert_eq!(omitted_still.reason, OPENCODE_MISSING_SENTENCE);
    assert_eq!(omitted_still.status_label, "unsupported");
    wait_explorer_ready(&mut omitted, omitted_later_id);
    omitted.click(made_app::theme::wordmark_id());
    open_about_diagnostics(&mut omitted, omitted_later_id);
    assert_opencode_diagnostics_row(
        &mut omitted,
        omitted_later_id,
        OPENCODE_MISSING_SENTENCE,
        unsupported.0,
        unsupported.1,
        unsupported.2,
        "unsupported",
        "opencode_missing",
        GROK_EXPLICIT_CATALOG_SENTENCE,
    );

    let path_only = tempfile::tempdir().unwrap();
    std::env::set_var("PATH", path_only.path());
    let detect_root = fixture_workspace(temp.path(), "opencode-detect", "main.rs");
    let detect_later = fixture_workspace(temp.path(), "opencode-detect-later", "main.rs");
    let mut detect_model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tui_detection(),
    );
    assert!(
        detect_model.compatible_tuis().is_empty(),
        "detection must not block construction"
    );
    let detect_id = open_workspace(&mut detect_model, &detect_root);
    let pending = detect_model
        .workspace(detect_id)
        .unwrap()
        .diagnostics_snapshot();
    assert_eq!(pending.reason, OPENCODE_DIAGNOSTIC_SENTENCE);
    assert_eq!(pending.status_label, "inactive");
    assert_eq!(
        pending.row("grok status").unwrap().sentence,
        "Checking for Grok Build."
    );
    assert!(
        !pending.reason.contains("not on PATH"),
        "detection still running must not say OpenCode is not on PATH"
    );
    let mut detecting = Headless::new(detect_model, 1280.0, 800.0);
    let pending_output = reveal_diagnostics_without_logic(&mut detecting, detect_id);
    let pending_text = output_text(&pending_output);
    assert!(
        pending_text.contains(OPENCODE_DIAGNOSTIC_SENTENCE),
        "in-flight detection painted the wrong OpenCode sentence: {pending_text}"
    );
    assert!(
        !pending_text.contains(OPENCODE_MISSING_SENTENCE),
        "in-flight detection already says OpenCode is not on PATH: {pending_text}"
    );
    assert!(
        pending_text.contains("Checking for Grok Build."),
        "in-flight detection adopted a Grok result early: {pending_text}"
    );
    detecting.pump_until(|model| {
        let view = model.workspace(detect_id).unwrap().diagnostics_snapshot();
        view.reason == OPENCODE_MISSING_SENTENCE
            && view.row("grok status").unwrap().sentence == "Grok Build is not on PATH."
    });
    wait_explorer_ready(&mut detecting, detect_id);
    assert_opencode_diagnostics_row(
        &mut detecting,
        detect_id,
        OPENCODE_MISSING_SENTENCE,
        unsupported.0,
        unsupported.1,
        unsupported.2,
        "unsupported",
        "opencode_missing",
        "Grok Build is not on PATH.",
    );
    let detect_later_id = open_workspace(&mut detecting.model, &detect_later);
    let detect_still = detecting
        .model
        .workspace(detect_id)
        .unwrap()
        .diagnostics_snapshot();
    assert_eq!(detect_still.reason, OPENCODE_MISSING_SENTENCE);
    assert_eq!(detect_still.status_label, "unsupported");
    wait_explorer_ready(&mut detecting, detect_later_id);
    detecting.click(made_app::theme::wordmark_id());
    open_about_diagnostics(&mut detecting, detect_later_id);
    assert_opencode_diagnostics_row(
        &mut detecting,
        detect_later_id,
        OPENCODE_MISSING_SENTENCE,
        unsupported.0,
        unsupported.1,
        unsupported.2,
        "unsupported",
        "opencode_missing",
        "Grok Build is not on PATH.",
    );
}

#[test]
fn remote_grok_diagnostics_follow_the_tab_tier_and_server_state() {
    if isolated("remote_grok_diagnostics_follow_the_tab_tier_and_server_state") {
        return;
    }
    use made_app::app::RemoteGrokInstallHold;
    use made_app::remote_workspace::RemoteWorkspace;
    use made_app::ssh_servers::ServerMetadata;
    use made_remote::protocol::{
        Capabilities, GrokActivity, GrokSessionInfo, GrokSessionSnapshot, Identity, Operation,
        RemotePath, RemoteTarget, TabCapability, WireEnvelope,
    };

    let grok_capabilities = Capabilities {
        target: RemoteTarget::LinuxX64,
        tmux: false,
        opencode: false,
        shell_cwd: false,
        helper_version: "probe".to_owned(),
        grok: true,
        python3: false,
    };
    let no_grok_capabilities = Capabilities {
        grok: false,
        ..grok_capabilities.clone()
    };
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let authority = model
        .add_ssh_metadata(ServerMetadata {
            display_name: "grok-server".into(),
            host: "grok-server".into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("server metadata saves");
    let other = model
        .add_ssh_metadata(ServerMetadata {
            display_name: "plain-server".into(),
            host: "plain-server".into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("server metadata saves");
    model
        .hold_ssh_probe_result(authority, Ok(grok_capabilities))
        .expect("the probe result records");
    model
        .hold_ssh_probe_result(other, Ok(no_grok_capabilities))
        .expect("the probe result records");
    assert!(
        model.remote_grok_detected_on(authority).unwrap(),
        "the server's own probe detected grok"
    );
    assert!(!model.remote_grok_detected_on(other).unwrap());
    assert!(
        !model.remote_grok_consent_granted(authority).unwrap(),
        "no consent exists before an explicit approval"
    );
    // A server without detected grok offers no Grok tier at all.
    assert!(!model.remote_grok_detected_on(other).unwrap());

    model
        .attach_remote_grok_model(
            authority,
            RemoteWorkspace::new(
                Identity {
                    authority,
                    tab: TabCapability::random().unwrap(),
                    epoch: 1,
                    generation: 1,
                },
                RemotePath::new(b"/project".to_vec()).unwrap(),
            )
            .unwrap(),
        )
        .expect("remote model attaches");
    let remote_id = model.active_workspace_id().unwrap();
    let mut headless = Headless::new(model, 1280.0, 800.0);
    headless.render();

    let sentence = |headless: &Headless| {
        headless
            .model
            .workspace(remote_id)
            .unwrap()
            .diagnostics_snapshot()
            .row("grok status")
            .unwrap()
            .sentence
            .to_owned()
    };
    let pill = |headless: &Headless| {
        headless
            .model
            .workspace(remote_id)
            .unwrap()
            .diagnostics_snapshot()
            .row("grok status")
            .unwrap()
            .pill
    };

    // Attached, detected, nothing running: the honest row is not running.
    assert!(
        sentence(&headless).contains("Grok Build is not running."),
        "{}",
        sentence(&headless)
    );

    // An owned open session without consent: the inventory tier is degraded
    // and names the missing approval. The host sentence never applies.
    {
        let workbench = headless.model.workspace_mut(remote_id).unwrap();
        let remote = workbench.remote_workspace_mut().unwrap();
        let identity = remote.identity().clone();
        remote
            .accept_grok_push(WireEnvelope::new(
                identity,
                1,
                Operation::GrokSnapshot {
                    snapshot: GrokSessionSnapshot {
                        revision: 1,
                        running: false,
                        hooks_ready: false,
                        sessions: vec![GrokSessionInfo {
                            session_id: "a".to_owned(),
                            cwd: Some(RemotePath::new(b"/grok-a".to_vec()).unwrap()),
                            opened_at: 1,
                            activity: GrokActivity::Working,
                        }],
                    },
                },
            ))
            .expect("the current-attach Grok snapshot applies");
    }
    headless.advance(0.1);
    assert!(
        sentence(&headless).contains("hooks are unavailable"),
        "{}",
        sentence(&headless)
    );
    assert!(
        sentence(&headless).contains("not approved"),
        "{}",
        sentence(&headless)
    );
    assert_eq!(
        pill(&headless),
        made_app::diagnostics::DiagnosticsPill::Degraded
    );

    // The bounded attempt is busy while it runs.
    headless
        .model
        .hold_remote_grok_install(authority, RemoteGrokInstallHold::Running);
    assert!(
        sentence(&headless).contains("Installing the Grok Build plugin"),
        "{}",
        sentence(&headless)
    );

    // A bounded companion reason names the failed install and no interpreter.
    headless.model.hold_remote_grok_install(
        authority,
        RemoteGrokInstallHold::Failed {
            reason: Some("install-failed"),
        },
    );
    assert!(
        sentence(&headless).contains("the plugin install failed")
            && !sentence(&headless).contains("python3"),
        "{}",
        sentence(&headless)
    );

    // A generic install failure names the failed install.
    headless
        .model
        .hold_remote_grok_install(authority, RemoteGrokInstallHold::Failed { reason: None });
    assert!(
        sentence(&headless).contains("the plugin install failed"),
        "{}",
        sentence(&headless)
    );

    // After a successful install the row is ready while the session is open.
    headless
        .model
        .hold_remote_grok_install(authority, RemoteGrokInstallHold::Succeeded);
    assert_eq!(
        sentence(&headless),
        "Grok Build session follow is ready.",
        "{}",
        sentence(&headless)
    );

    // When the session closes the row says not running again.
    {
        let workbench = headless.model.workspace_mut(remote_id).unwrap();
        let remote = workbench.remote_workspace_mut().unwrap();
        let identity = remote.identity().clone();
        remote
            .accept_grok_push(WireEnvelope::new(
                identity,
                1,
                Operation::GrokSnapshot {
                    snapshot: GrokSessionSnapshot {
                        revision: 2,
                        running: false,
                        hooks_ready: false,
                        sessions: vec![],
                    },
                },
            ))
            .expect("the exit snapshot applies");
    }
    headless.advance(0.1);
    assert!(
        sentence(&headless).contains("Grok Build is not running."),
        "{}",
        sentence(&headless)
    );

    // A transport disconnect leaves no stale Grok claim at all.
    {
        let workbench = headless.model.workspace_mut(remote_id).unwrap();
        let remote = workbench.remote_workspace_mut().unwrap();
        let identity = remote.identity().clone();
        remote
            .accept_grok_push(WireEnvelope::new(
                identity,
                1,
                Operation::GrokSnapshot {
                    snapshot: GrokSessionSnapshot {
                        revision: 3,
                        running: true,
                        hooks_ready: false,
                        sessions: vec![GrokSessionInfo {
                            session_id: "b".to_owned(),
                            cwd: Some(RemotePath::new(b"/grok-b".to_vec()).unwrap()),
                            opened_at: 1,
                            activity: GrokActivity::Working,
                        }],
                    },
                },
            ))
            .expect("the reconnect snapshot applies");
        workbench.remote_workspace_mut().unwrap().disconnect();
    }
    headless.advance(0.1);
    let row_sentence = sentence(&headless);
    assert!(
        row_sentence.contains("SSH transport is disconnected"),
        "no stale Grok claim may survive the disconnect: {row_sentence}"
    );
    assert!(
        !row_sentence.contains("ready") && !row_sentence.contains("hooks are unavailable"),
        "the transport row must not carry pre-disconnect tier claims: {row_sentence}"
    );
}

#[test]
fn grok_ssh_menu_row_and_consent_card_follow_per_server_detection() {
    if isolated("grok_ssh_menu_row_and_consent_card_follow_per_server_detection") {
        return;
    }
    use made_app::ssh_servers::ServerMetadata;
    use made_remote::protocol::{Capabilities, RemoteTarget};

    let capabilities = Capabilities {
        target: RemoteTarget::LinuxX64,
        tmux: false,
        opencode: false,
        shell_cwd: false,
        helper_version: "probe".to_owned(),
        grok: true,
        python3: false,
    };
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let with_grok = model
        .add_ssh_metadata(ServerMetadata {
            display_name: "Grok server".into(),
            host: "grok-server".into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("server metadata saves");
    let _without_grok = model
        .add_ssh_metadata(ServerMetadata {
            display_name: "Plain server".into(),
            host: "plain-server".into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("server metadata saves");
    model
        .hold_ssh_probe_result(with_grok, Ok(capabilities))
        .expect("probe result records");
    // A local workspace tab hosts the header; the servers exist beside it.
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "menu-host", "main.rs");
    open_workspace(&mut model, &root);
    let menu_host = model.active_workspace_id().unwrap();
    let mut headless = Headless::new(model, 1280.0, 800.0);
    headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    wait_explorer_ready(&mut headless, menu_host);
    headless.render();

    // The `+` menu offers `Grok SSH` only for the server whose own probe
    // detected grok; the other server's choices omit it.
    headless.click(top_action_id("open-folder"));
    let menu = output_text(&headless.render());
    assert!(
        menu.contains("Grok SSH — Grok server"),
        "the detected server must offer Grok SSH: {menu}"
    );
    assert!(
        !menu.contains("Grok SSH — Plain server"),
        "an undetected server must not offer Grok SSH: {menu}"
    );
    assert!(
        menu.contains("Terminal SSH — Plain server"),
        "the undetected server keeps its terminal choice: {menu}"
    );
    headless.press_key(egui::Key::Escape);
    headless.render();

    // The consent card lives in the server window for the selected server.
    headless.click(top_action_id("ssh-servers"));
    let window = output_text(&headless.render());
    assert!(
        window.contains("Servidores SSH"),
        "the SSH window opens: {window}"
    );
    assert!(
        !window.contains("Grok Build plugin install"),
        "a server whose probe did not detect grok shows no Grok consent card: {window}"
    );

    // Select the grok-capable server: the card offers approve and refuse with
    // the honest new-sessions-only disclosure, and nothing has been installed.
    headless.model.select_ssh_server(with_grok);
    // The bounded body measures the card, then the window grows to fit it.
    headless.render();
    headless.render();
    let card = output_text(&headless.render());
    assert!(
        card.contains("Grok Build plugin install"),
        "the detected server offers the consent card: {card}"
    );
    assert!(
        card.contains("only to Grok sessions started after the install"),
        "the card discloses the new-sessions-only limit: {card}"
    );
    assert!(
        card.contains("Approve") && card.contains("Refuse"),
        "the card presents approve and refuse: {card}"
    );
    assert!(
        !headless
            .model
            .remote_grok_consent_granted(with_grok)
            .unwrap(),
        "no consent exists before the explicit approval"
    );
    assert!(
        headless
            .model
            .remote_grok_install_state(with_grok)
            .is_none(),
        "viewing the card must not start any install"
    );

    // Refusal stores nothing and starts nothing; the terminal choice stays.
    headless.click(top_action_id("ssh-grok-refuse"));
    headless.render();
    let refused = output_text(&headless.render());
    assert!(
        refused.contains("Refused. Nothing was installed or written"),
        "refusal is disclosed honestly: {refused}"
    );
    assert!(
        !headless
            .model
            .remote_grok_consent_granted(with_grok)
            .unwrap(),
        "refusal never records approval"
    );
    assert!(
        headless
            .model
            .remote_grok_install_state(with_grok)
            .is_none(),
        "refusal must not start an install attempt"
    );

    // A fresh server has no staged helper, so the approval stays gated until
    // this session verifies a provisioned or reused companion.
    assert!(!headless.model.remote_grok_install_ready(with_grok));
    headless.click(top_action_id("ssh-grok-approve"));
    assert!(
        !headless
            .model
            .remote_grok_consent_granted(with_grok)
            .unwrap(),
        "a gated approval stores nothing"
    );
    assert!(!headless.model.remote_grok_install_running(with_grok));

    headless.model.hold_remote_companion_provisioned(with_grok);
    headless.render();

    // Approval persists the per-server/version consent and starts the bounded
    // attempt; without the packaged helper it fails closed and says so.
    headless.click(top_action_id("ssh-grok-approve"));
    assert!(
        headless
            .model
            .remote_grok_consent_granted(with_grok)
            .unwrap(),
        "approval persists the consent"
    );
    headless.pump_until(|model| {
        !matches!(
            model.remote_grok_install_state(with_grok),
            Some(made_app::app::RemoteGrokInstallState::Running)
        )
    });
    assert!(matches!(
        headless.model.remote_grok_install_state(with_grok),
        Some(made_app::app::RemoteGrokInstallState::Failed { .. })
    ));
    headless.render();
    let failed = output_text(&headless.render());
    assert!(
        failed.contains("install failed"),
        "the failed attempt is disclosed honestly: {failed}"
    );

    // Withdrawal clears the consent and the card returns to its offer.
    headless.click(top_action_id("ssh-grok-withdraw"));
    assert!(
        !headless
            .model
            .remote_grok_consent_granted(with_grok)
            .unwrap(),
        "withdrawal removes the consent"
    );
    headless.render();
    let withdrawn = output_text(&headless.render());
    assert!(
        withdrawn.contains("Consent withdrawn"),
        "withdrawal is disclosed: {withdrawn}"
    );
}

// ── Update UI (add-release-auto-update task 6.1) ───────────────────────────

#[derive(Clone, Default)]
struct FakeUpdates {
    kept: Option<String>,
    offer: Option<String>,
    check_delay: Duration,
    calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl made_app::updates::UpdateCapability for FakeUpdates {
    fn kept(&mut self) -> Option<String> {
        self.kept.clone()
    }
    fn check(&mut self) -> Result<Option<String>, made_app::updates::UpdateFailure> {
        std::thread::sleep(self.check_delay);
        self.calls.lock().unwrap().push("check".into());
        Ok(self.offer.clone())
    }
    fn download(&mut self, version: &str) -> Result<String, made_app::updates::UpdateFailure> {
        Ok(version.to_owned())
    }
    fn restarter(&self) -> made_app::updates::Restarter {
        let calls = self.calls.clone();
        Box::new(move |version: &str| {
            calls.lock().unwrap().push(format!("restart {version}"));
            Ok(())
        })
    }
}

fn installed_updates(model: &mut AppModel, fake: &FakeUpdates) {
    let store = made_app::preferences::PreferenceStore::from_app_paths();
    model.set_updates(
        made_app::updates::UpdateScheduler::start(Box::new(fake.clone()), false, Instant::now()),
        store,
    );
}

fn accent_dots_in(output: &egui::FullOutput, rect: egui::Rect) -> usize {
    circle_shapes(output)
        .iter()
        .filter(|circle| {
            circle.fill == made_app::theme::ACCENT && rect.expand(1.0).contains(circle.center)
        })
        .count()
}

fn wordmark_rect(headless: &Headless) -> egui::Rect {
    headless.response(made_app::theme::wordmark_id()).rect
}

#[test]
fn update_indicator_appears_only_when_ready_on_the_active_header_wordmark() {
    if isolated("update_indicator_appears_only_when_ready_on_the_active_header_wordmark") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "update-dot", "main.rs");
    for (width, height) in [(1280.0, 800.0), (720.0, 480.0)] {
        // Non-installed and installed-without-ready builds show no indicator.
        let mut rects = Vec::new();
        for ready in [None, Some("0.3.0")] {
            let mut model =
                AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
            let fake = FakeUpdates {
                kept: ready.map(str::to_owned),
                ..FakeUpdates::default()
            };
            installed_updates(&mut model, &fake);
            let id = open_workspace(&mut model, &root);
            let mut headless = Headless::new(model, width, height);
            wait_explorer_ready(&mut headless, id);
            headless.pump_until(|model| model.update_ready() == ready.is_some());
            let output = headless.render();
            let rect = wordmark_rect(&headless);
            rects.push(rect);
            assert_eq!(
                accent_dots_in(&output, rect),
                usize::from(ready.is_some()),
                "{width}x{height} ready={ready:?}"
            );
            let name = if ready.is_some() {
                "About, update ready"
            } else {
                "About"
            };
            headless.ctx.enable_accesskit();
            let output = headless.render();
            let (role, label, _) = accesskit_row_state(&output, made_app::theme::wordmark_id());
            assert_eq!(role, egui::Role::Button);
            assert_eq!(
                label.as_deref(),
                Some(name),
                "accessibility name at {width}x{height}"
            );
        }
        assert_eq!(
            rects[0], rects[1],
            "the indicator never changes the wordmark target"
        );
    }

    // A non-installed build never shows it; the picker wordmark never does.
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    let output = headless.render();
    assert_eq!(accent_dots_in(&output, wordmark_rect(&headless)), 0);

    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    installed_updates(
        &mut model,
        &FakeUpdates {
            kept: Some("0.3.0".into()),
            ..FakeUpdates::default()
        },
    );
    let mut headless = Headless::new(model, 1280.0, 800.0);
    headless.pump_until(AppModel::update_ready);
    let picker = headless.render();
    assert!(
        circle_shapes(&picker)
            .iter()
            .all(|circle| circle.fill != made_app::theme::ACCENT
                || circle.radius != made_app::theme::UPDATE_DOT_RADIUS),
        "the picker wordmark never shows the indicator"
    );
    // Opening a workspace shows the ready update again.
    let id = open_workspace(&mut headless.model, &root);
    wait_explorer_ready(&mut headless, id);
    headless.render();
    let output = headless.render();
    assert_eq!(
        accent_dots_in(&output, wordmark_rect(&headless)),
        1,
        "dots: {:?} wordmark: {:?}",
        circle_shapes(&output)
            .iter()
            .map(|c| (c.center, c.radius))
            .collect::<Vec<_>>(),
        wordmark_rect(&headless)
    );
}

#[test]
fn about_updates_section_shows_status_and_controls_for_the_build() {
    if isolated("about_updates_section_shows_status_and_controls_for_the_build") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "update-about", "main.rs");

    // Non-installed: the status line only, no controls, no error treatment.
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = open_workspace(&mut model, &root);
    let mut headless = Headless::new(model, 1280.0, 800.0);
    wait_explorer_ready(&mut headless, id);
    headless.click(made_app::theme::wordmark_id());
    let text = output_text(&headless.render());
    assert!(
        text.contains("UPDATES") && text.contains("Updates are not available for this build."),
        "{text}"
    );
    for absent in ["Check for updates", "Restart to update"] {
        assert!(
            !text.contains(absent),
            "{absent} in a non-installed build: {text}"
        );
    }

    // Installed, nothing ready, automatic checks off.
    for (width, height) in [(1280.0, 800.0), (720.0, 480.0)] {
        let mut model =
            AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
        let fake = FakeUpdates {
            check_delay: Duration::from_millis(400),
            ..FakeUpdates::default()
        };
        installed_updates(&mut model, &fake);
        let id = open_workspace(&mut model, &root);
        let mut headless = Headless::new(model, width, height);
        wait_explorer_ready(&mut headless, id);
        headless.click(made_app::theme::wordmark_id());
        let text = output_text(&headless.render());
        for expected in [
            "UPDATES",
            "Not checked for updates yet.",
            "Check for updates",
            "Check for updates automatically",
        ] {
            assert!(
                text.contains(expected),
                "missing {expected:?} at {width}x{height}: {text}"
            );
        }
        assert!(!text.contains("Restart to update"));

        headless.click(made_app::ui::about_update_check_id());
        assert!(headless.model.updates().busy());
        assert!(output_text(&headless.render()).contains("Checking for updates"));
        headless.click(made_app::ui::about_update_check_id());
        headless.pump_until(|model| !model.updates().busy());
        assert_eq!(
            fake.calls.lock().unwrap().len(),
            1,
            "Check for updates is unavailable while a check runs"
        );
        assert!(output_text(&headless.render()).contains("Up to date. Last checked just now."));

        assert!(!headless.model.updates().automatic());
        headless.click(made_app::ui::about_update_automatic_id());
        headless.pump_until(|model| !model.updates().busy());
        assert!(
            headless.model.updates().automatic(),
            "the toggle changes the REL-10 choice"
        );
        assert!(
            made_app::preferences::PreferenceStore::from_app_paths()
                .unwrap()
                .load()
                .automatic_checks
        );
        headless.click(made_app::ui::about_update_automatic_id());
        assert!(
            !made_app::preferences::PreferenceStore::from_app_paths()
                .unwrap()
                .load()
                .automatic_checks
        );

        let window = headless
            .ctx
            .memory(|memory| {
                let pane = headless.model.workspace(id).unwrap().pane_id().to_owned();
                memory.area_rect(egui::Id::unique(("made-about-window", pane.as_str())))
            })
            .expect("About window rect");
        headless.pump_until(|model| !model.updates().busy());
        for control in [
            made_app::ui::about_update_check_id(),
            made_app::ui::about_update_automatic_id(),
        ] {
            assert!(
                window.contains_rect(headless.response(control).rect),
                "control clipped at {width}x{height}"
            );
            assert!(
                headless.response(control).rect.height() >= 24.0,
                "minimum target"
            );
            headless.focus(control);
            let focused = headless.render();
            assert!(
                !focus_ring_rects(&focused, headless.response(control).rect.expand(6.0)).is_empty(),
                "visible focus ring on {control:?} (focused: {:?})",
                headless.ctx.memory(|memory| memory.focused())
            );
        }
    }
}

#[test]
fn restart_confirmation_lives_inside_about_and_not_now_touches_nothing() {
    if isolated("restart_confirmation_lives_inside_about_and_not_now_touches_nothing") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let first = fixture_workspace(temp.path(), "update-one", "main.rs");
    let second = fixture_workspace(temp.path(), "update-two", "lib.rs");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let fake = FakeUpdates {
        kept: Some("0.3.0".into()),
        ..FakeUpdates::default()
    };
    installed_updates(&mut model, &fake);
    let one = open_workspace(&mut model, &first);
    let two = open_workspace(&mut model, &second);
    let mut headless = Headless::new(model, 720.0, 480.0);
    wait_explorer_ready(&mut headless, two);
    headless.pump_until(AppModel::update_ready);
    let confirmation = made_app::ui::restart_confirmation_text("0.3.0");

    headless.click(made_app::theme::wordmark_id());
    let text = output_text(&headless.render());
    assert!(
        text.contains("Made 0.3.0 is ready to install.") && text.contains("Restart to update"),
        "{text}"
    );
    headless.click(made_app::ui::about_update_restart_id());
    let text = output_text(&headless.render());
    for expected in [
        "0.3.0",
        "OpenCode, Grok and SSH sessions",
        "will not be restored",
        "Restart now",
        "Not now",
    ] {
        assert!(text.contains(expected), "missing {expected:?}: {text}");
    }
    let pane = headless.model.workspace(two).unwrap().pane_id().to_owned();
    let window = headless
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::unique(("made-about-window", pane.as_str()))))
        .unwrap();
    assert!(
        window.contains_rect(
            headless
                .response(made_app::ui::about_update_not_now_id())
                .rect
        ),
        "the compact card fits the confirmation"
    );

    headless.click(made_app::ui::about_update_not_now_id());
    let text = output_text(&headless.render());
    assert!(
        !text.contains("Restart now"),
        "Not now dismisses the confirmation"
    );
    assert!(fake.calls.lock().unwrap().is_empty());
    for id in [one, two] {
        assert!(
            headless.model.workspace(id).is_some(),
            "workspaces untouched"
        );
    }

    // Closing About dismisses it; reopening shows the ready state only.
    headless.click(made_app::ui::about_update_restart_id());
    assert!(output_text(&headless.render()).contains("Restart now"));
    headless.click(made_app::theme::wordmark_id());
    headless.render();
    headless.click(made_app::theme::wordmark_id());
    let text = output_text(&headless.render());
    assert!(
        text.contains("Restart to update") && !text.contains("Restart now"),
        "{text}"
    );
    assert!(!text.contains(&confirmation[..20]));

    // Restart now closes every workspace, then hands 0.3.0 to the updater.
    headless.click(made_app::ui::about_update_restart_id());
    assert!(output_text(&headless.render()).contains("Restart now"));
    headless.click(made_app::ui::about_update_restart_now_id());
    assert_eq!(
        *fake.calls.lock().unwrap(),
        vec!["restart 0.3.0".to_owned()],
        "ready={:?} restarting={}",
        headless.model.updates().ready_version(),
        headless.model.update_restarting()
    );
    assert!(headless.model.update_restarting());
    for id in [one, two] {
        let running = headless
            .model
            .workspace(id)
            .and_then(Workbench::terminal)
            .is_some_and(|terminal| terminal.is_running());
        assert!(
            !running,
            "workspace {id:?} must be closed before the update applies"
        );
    }
}

// ---- Claude Code menu row and tab presentation ------------------------------

const CLAUDE_ESC_WORKING: &[u8] = b"\x1b]7501;state=working:app=claude-code\x07";
const CLAUDE_ESC_IDLE: &[u8] = b"\x1b]7501;state=idle:app=claude-code\x07";
const CLAUDE_ESC_CLEAR: &[u8] = b"\x1b]7501;state=clear:app=claude-code\x07";

/// Assert the 16px official Claude mark is painted inside `area`, left of
/// `left_of`, with its shipped colors.
fn assert_claude_mark(
    headless: &Headless,
    output: &egui::FullOutput,
    area: egui::Rect,
    left_of: f32,
    what: &str,
) {
    let image = painted_images(output)
        .into_iter()
        .find(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && area.contains_rect(image.rect)
                && image.rect.right() <= left_of - 1.0
        })
        .unwrap_or_else(|| panic!("{what}: no 16px image in the leading slot"));
    let px = (output.pixels_per_point * image.rect.size()).round();
    assert_eq!(
        image.texture,
        embedded_svg_texture(
            &headless.ctx,
            "bytes://claude-code.svg",
            include_bytes!("../assets/claude-code.svg"),
            px.x as u32,
            px.y as u32,
        ),
        "{what}: the 16px image must match claude-code.svg"
    );
    assert_eq!(
        image.tint,
        egui::Color32::WHITE,
        "{what}: the mark keeps its colors"
    );
}

/// Whether a 16px image is painted in `area` left of `left_of`.
fn has_leading_image(output: &egui::FullOutput, area: egui::Rect, left_of: f32) -> bool {
    painted_images(output).into_iter().any(|image| {
        (image.rect.width() - 16.0).abs() < 0.75
            && (image.rect.height() - 16.0).abs() < 0.75
            && area.contains_rect(image.rect)
            && image.rect.right() <= left_of - 1.0
    })
}

#[test]
fn catalog_menu_lists_claude_code_after_grok() {
    if isolated("catalog_menu_lists_claude_code_after_grok") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "all-three", "main.rs");
    let (mut headless, _) = menu_model(&root, &[OPENCODE, GROK, CLAUDE_CODE]);
    let hovered =
        output_text(&headless.hover_until_text(top_action_id("open-folder"), "Open folder"));
    assert!(
        hovered.contains("Open folder"),
        "the + tooltip stays Open folder: {hovered}"
    );
    headless.click(top_action_id("open-folder"));
    let terminal = headless.response(new_tab_menu_item_id("terminal")).rect;
    let opencode = headless.response(new_tab_menu_item_id("opencode")).rect;
    let grok = headless.response(new_tab_menu_item_id("grok")).rect;
    let claude = headless.response(new_tab_menu_item_id("claude")).rect;
    assert!(
        terminal.top() < opencode.top() && opencode.top() < grok.top() && grok.top() < claude.top(),
        "menu order must be Terminal, OpenCode, Grok Build, Claude Code"
    );
    assert!(!headless.model.is_picker());
    assert!(headless
        .ctx
        .read_response(new_tab_menu_item_id("codex"))
        .is_none());
}

#[test]
fn only_claude_code_shows_terminal_then_claude_code_with_its_mark() {
    if isolated("only_claude_code_shows_terminal_then_claude_code_with_its_mark") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "claude-only", "main.rs");
    let (mut headless, _) = menu_model(&root, &[CLAUDE_CODE]);
    headless.ctx.set_pixels_per_point(2.0);
    headless.click(top_action_id("open-folder"));
    headless.advance(0.3);
    let output = headless.render();
    let terminal = headless.response(new_tab_menu_item_id("terminal")).rect;
    let claude = headless.response(new_tab_menu_item_id("claude")).rect;
    assert!(terminal.top() < claude.top());
    for absent in ["opencode", "grok"] {
        assert!(headless
            .ctx
            .read_response(new_tab_menu_item_id(absent))
            .is_none());
    }
    let label = painted_menu_label(&output, "Claude Code", claude);
    assert_claude_mark(
        &headless,
        &output,
        claude,
        label.line.left(),
        "the Claude Code menu row",
    );
}

#[test]
fn choosing_claude_code_starts_a_home_tab_and_terminal_still_opens_the_picker() {
    if isolated("choosing_claude_code_starts_a_home_tab_and_terminal_still_opens_the_picker") {
        return;
    }
    use made_platform::Platform;

    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "current", "main.rs");
    let (mut headless, id) = menu_model(&root, &[CLAUDE_CODE]);
    let home = made_platform::host_platform()
        .home_dir()
        .expect("test home")
        .canonicalize()
        .expect("canonical test home");
    headless.click(top_action_id("open-folder"));
    headless.click(new_tab_menu_item_id("claude"));
    assert!(!headless.model.is_picker());
    assert_eq!(headless.model.active_workspace_id(), Some(id));
    assert!(headless.model.start_pending());
    assert_eq!(
        headless.model.terminal_target(),
        TerminalTarget::Tui(CLAUDE_CODE)
    );
    assert_eq!(
        headless.model.launch().source(),
        WorkspaceSource::DefaultHome
    );
    assert_eq!(headless.model.launch().terminal_cwd(), Some(home.as_path()));
    assert!(headless
        .model
        .workspace(id)
        .and_then(Workbench::terminal)
        .is_some_and(made_app::TerminalPane::is_running));
}

/// Stage the shipped Claude Code plugin beside the test executable: its
/// directory and an executable hook binary, which together count as present.
fn stage_claude_plugin() {
    let hook = made_platform::install_root_from_current_executable(
        made_platform::PlatformFamily::current(),
    )
    .expect("install root")
    .join("plugins")
    .join("claude-code-made")
    .join("bin")
    .join(format!(
        "made-claude-status{}",
        std::env::consts::EXE_SUFFIX
    ));
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[cfg(target_os = "linux")]
struct ClaudeFixture {
    headless: Headless,
    id: WorkspaceId,
    root: PathBuf,
    emit: PathBuf,
    spool: PathBuf,
    claude_pid: u32,
}

#[cfg(target_os = "linux")]
impl ClaudeFixture {
    fn new(temp: &Path) -> Self {
        stage_claude_plugin();
        let root = fixture_workspace(temp, "project-alpha", "main.rs");
        let mut model = AppModel::from_launch(
            WorkspaceLaunch::waiting_for_picker(),
            AppOptions::new().with_compatible_tuis([CLAUDE_CODE]),
        );
        let id = open_workspace(&mut model, &root);
        let mut headless = Headless::new(model, 1280.0, 800.0);
        headless.ctx.enable_accesskit();
        wait_explorer_ready(&mut headless, id);
        headless.ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.interaction.tooltip_delay = 0.0
        });
        let emit = temp.join("emit.bin");
        std::fs::write(&emit, b"").unwrap();
        let workbench = headless.model.workspace_mut(id).unwrap();
        let spool = PathBuf::from(
            workbench
                .terminal()
                .unwrap()
                .options()
                .env
                .iter()
                .find(|(key, _)| key == "MADE_CLAUDE_SPOOL")
                .expect("the shell tab has a spool")
                .1
                .clone(),
        );
        let shell = workbench.terminal().and_then(made_app::TerminalPane::pid);
        // A foreground child of the shell stands in for Claude Code: it keeps
        // the shell off its prompt and relays whatever the test appends.
        type_terminal_command(workbench, &format!("tail -n0 -f '{}'", emit.display()));
        let shell = shell.expect("shell pid");
        let deadline = Instant::now() + Duration::from_secs(10);
        let claude_pid = loop {
            headless.render();
            let found = std::process::Command::new("pgrep")
                .args(["-P", &shell.to_string(), "tail"])
                .output()
                .ok()
                .and_then(|out| {
                    String::from_utf8_lossy(&out.stdout)
                        .trim()
                        .parse::<u32>()
                        .ok()
                });
            if let Some(pid) = found {
                break pid;
            }
            assert!(Instant::now() < deadline, "tail did not start");
            std::thread::sleep(Duration::from_millis(20));
        };
        std::thread::sleep(Duration::from_millis(200));
        Self {
            headless,
            id,
            root,
            emit,
            spool,
            claude_pid,
        }
    }

    fn emit(&self, bytes: &[u8]) {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&self.emit)
            .unwrap();
        file.write_all(bytes).unwrap();
    }

    fn hook(&self, cwd: &Path, event: &str, status: &str, title: &str) {
        use std::io::Write;
        let line = json!({
            "at": "t", "sessionId": "s1", "cwd": cwd, "event": event, "notification": "",
            "status": status, "title": title, "pids": [4_000_000u32, self.claude_pid],
        });
        let shell = self
            .headless
            .model
            .workspace(self.id)
            .and_then(Workbench::terminal)
            .and_then(made_app::TerminalPane::pid)
            .unwrap();
        let mut line = line;
        line["pids"] = json!([4_000_000u32, self.claude_pid, shell]);
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&self.spool)
            .unwrap();
        writeln!(file, "{line}").unwrap();
    }

    fn tab(&mut self) -> egui::Rect {
        self.headless.render();
        self.headless.response(workspace_tab_id(self.id)).rect
    }

    /// Render until `ready` accepts the tab's painted output.
    fn until(
        &mut self,
        mut ready: impl FnMut(&egui::FullOutput, egui::Rect) -> bool,
    ) -> egui::FullOutput {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let output = self.headless.render();
            let tab = self.headless.response(workspace_tab_id(self.id)).rect;
            if ready(&output, tab) || Instant::now() >= deadline {
                return output;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn labeled(&mut self, label: &str) -> egui::FullOutput {
        let label = label.to_owned();
        self.until(move |output, tab| {
            text_rects(output, &label, 55.0)
                .iter()
                .any(|rect| tab.contains_rect(*rect))
        })
    }

    fn hover_tab(&mut self, needle: &str) -> String {
        let id = workspace_tab_id(self.id);
        output_text(&self.headless.hover_until_text(id, needle))
    }
}

#[cfg(target_os = "linux")]
fn tab_has_label(fixture: &mut ClaudeFixture, output: &egui::FullOutput, label: &str) -> bool {
    let tab = fixture.tab();
    text_rects(output, label, 55.0)
        .iter()
        .any(|rect| tab.contains_rect(*rect))
}

#[cfg(target_os = "linux")]
#[test]
fn protocol_status_paints_one_busy_dot_with_the_window_title_tooltip() {
    if isolated("protocol_status_paints_one_busy_dot_with_the_window_title_tooltip") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = ClaudeFixture::new(temp.path());
    let output = fixture.headless.render();
    assert!(
        !tab_has_label(&mut fixture, &output, "Claude Code"),
        "a plain shell is not branded"
    );

    // Protocol only: no hook record, so no live owned session.
    fixture.emit(b"\x1b]0;Refactor parser\x07");
    fixture.emit(CLAUDE_ESC_WORKING);
    let output = fixture.labeled("Claude Code");
    let tab = fixture.tab();
    let dots = tab_session_dots(&output, tab);
    assert_eq!(
        dots.len(),
        1,
        "one protocol-only dot without a live session"
    );
    assert_session_dot_color(&dots[0], made_app::theme::ACCENT, "the busy dot");
    let label = text_rects(&output, "Claude Code", 55.0)
        .into_iter()
        .find(|rect| tab.contains_rect(*rect))
        .unwrap();
    assert_claude_mark(&fixture.headless, &output, tab, label.left(), "the tab");
    assert!(
        text_rects(&output, "project-alpha", 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "the folder label is replaced"
    );
    let hovered = fixture.hover_tab("Refactor parser");
    assert!(
        hovered.contains("Refactor parser — Sessions: 1 running"),
        "tooltip is the window title and the summary: {hovered}"
    );
    assert!(
        !hovered.contains(&fixture.root.display().to_string()),
        "the tooltip omits the workspace path: {hovered}"
    );
    let output = fixture.headless.render();
    let (role, name, _) = accesskit_row_state(&output, workspace_tab_id(fixture.id));
    assert_eq!(role, egui::Role::Tab);
    assert_eq!(name.as_deref(), Some("Claude Code — Sessions: 1 running"));
}

#[cfg(target_os = "linux")]
#[test]
fn the_window_title_never_reaches_the_copied_report_or_the_log() {
    if isolated("the_window_title_never_reaches_the_copied_report_or_the_log") {
        return;
    }
    const MARKER: &str = "TITLE-MARKER-7f3a9c";
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = ClaudeFixture::new(temp.path());
    let id = fixture.id;
    fixture.emit(format!("\u{1b}]0;{MARKER}\u{7}").as_bytes());
    fixture.emit(CLAUDE_ESC_WORKING);
    fixture.labeled("Claude Code");
    // The title is read, for the tooltip only.
    let hovered = fixture.hover_tab(MARKER);
    assert!(hovered.contains(MARKER), "{hovered}");

    // Open the diagnostics with the title live so every report path runs.
    open_about_diagnostics(&mut fixture.headless, id);
    let shown = output_text(&fixture.headless.render());
    assert!(!shown.contains(MARKER), "the About panel: {shown}");
    let workbench = fixture.headless.model.workspace(id).unwrap();
    let report = workbench.sanitized_diagnostics_text();
    assert!(report.contains("claude status:"), "{report}");
    assert!(!report.contains(MARKER), "the copied report: {report}");
    let log = std::fs::read_to_string(isolated_log_path()).unwrap_or_default();
    assert!(!log.contains(MARKER), "the log: {log}");
}

#[cfg(target_os = "linux")]
#[test]
fn hook_only_dots_go_busy_then_attention_and_ignore_the_glyph_title() {
    if isolated("hook_only_dots_go_busy_then_attention_and_ignore_the_glyph_title") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = ClaudeFixture::new(temp.path());
    let cwd = fixture.root.clone();
    // No protocol status: the glyph title must not be read.
    fixture.emit("\u{1b}]0;◐ Refactor parser\u{7}".as_bytes());
    fixture.hook(&cwd, "SessionStart", "idle", "");
    let output = fixture.labeled("Claude Code");
    let tab = fixture.tab();
    let dots = tab_session_dots(&output, tab);
    assert_eq!(dots.len(), 1);
    assert_session_dot_color(&dots[0], made_app::theme::TEXT_TERTIARY, "the idle dot");

    fixture.hook(&cwd, "UserPromptSubmit", "working", "");
    let output = fixture.until(|output, tab| {
        tab_session_dots(output, tab)
            .first()
            .is_some_and(|dot| dot.fill == made_app::theme::ACCENT)
    });
    let tab = fixture.tab();
    assert_session_dot_color(
        &tab_session_dots(&output, tab)[0],
        made_app::theme::ACCENT,
        "busy",
    );
    fixture.hook(&cwd, "PermissionRequest", "attention", "");
    let output = fixture.until(|output, tab| {
        tab_session_dots(output, tab)
            .first()
            .is_some_and(|dot| dot.fill == made_app::theme::WARNING)
    });
    let tab = fixture.tab();
    assert_session_dot_color(
        &tab_session_dots(&output, tab)[0],
        made_app::theme::WARNING,
        "attention",
    );
    let hovered = fixture.hover_tab("Claude Code");
    assert!(
        hovered.contains("Claude Code — Sessions: 1 needs attention"),
        "the window title is ignored without a protocol status: {hovered}"
    );
    assert!(!hovered.contains("Refactor parser"), "{hovered}");

    // A renamed session title arrives with the next prompt.
    fixture.hook(&cwd, "UserPromptSubmit", "working", "foo");
    let hovered = fixture.hover_tab("foo — Sessions");
    assert!(hovered.contains("foo — Sessions: 1 running"), "{hovered}");
}

#[cfg(target_os = "linux")]
#[test]
fn empty_and_hostile_window_titles_fall_back_or_are_sanitized() {
    if isolated("empty_and_hostile_window_titles_fall_back_or_are_sanitized") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = ClaudeFixture::new(temp.path());
    fixture.emit(b"\x1b]0;\x07");
    fixture.emit(CLAUDE_ESC_WORKING);
    fixture.labeled("Claude Code");
    let hovered = fixture.hover_tab("Sessions: 1 running");
    assert!(
        hovered.contains("Claude Code — Sessions: 1 running"),
        "an empty window title falls back: {hovered}"
    );

    let hostile = format!("a\u{202e}b\tc{}", "x".repeat(300));
    fixture.emit(format!("\u{1b}]0;{hostile}\u{7}").as_bytes());
    let hovered = fixture.hover_tab("abc");
    assert!(!hovered.contains('\u{202e}'), "{hovered:?}");
    assert!(
        hovered.contains("abc"),
        "bidi controls are removed: {hovered:?}"
    );
    assert!(
        !hovered.contains(&"x".repeat(130)),
        "capped at 120: {hovered:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn clearing_returns_the_folder_label_path_tooltip_and_terminal_glyph() {
    if isolated("clearing_returns_the_folder_label_path_tooltip_and_terminal_glyph") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = ClaudeFixture::new(temp.path());
    fixture.emit(CLAUDE_ESC_IDLE);
    fixture.labeled("Claude Code");
    fixture.emit(CLAUDE_ESC_CLEAR);
    let output = fixture.labeled("project-alpha");
    let tab = fixture.tab();
    assert!(
        text_rects(&output, "Claude Code", 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "the Claude Code label is gone"
    );
    assert!(tab_session_dots(&output, tab).is_empty());
    assert!(
        !has_leading_image(&output, tab, tab.right()),
        "the terminal glyph returns instead of an image"
    );
    let hovered = fixture.hover_tab(&fixture.root.display().to_string());
    assert!(
        hovered.contains(&fixture.root.display().to_string()),
        "the full-path tooltip returns: {hovered}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn claude_code_takes_over_a_grok_tab_and_hands_it_back() {
    if isolated("claude_code_takes_over_a_grok_tab_and_hands_it_back") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = ClaudeFixture::new(temp.path());
    let shell = fixture
        .headless
        .model
        .workspace(fixture.id)
        .and_then(Workbench::terminal)
        .and_then(made_app::TerminalPane::pid)
        .unwrap();
    write_grok_inventory(&[
        json!({"pid": shell, "session_id": "g1", "cwd": "/tmp/made-grok-missing", "opened_at": "2024-01-01T00:00:01Z"}),
        json!({"pid": shell, "session_id": "g2", "cwd": "/tmp/made-grok-missing", "opened_at": "2024-01-01T00:00:02Z"}),
    ]);
    let output = fixture.labeled("Grok Build");
    let tab = fixture.tab();
    assert_eq!(tab_session_dots(&output, tab).len(), 2, "two Grok dots");

    fixture.emit(CLAUDE_ESC_WORKING);
    let output = fixture.labeled("Claude Code");
    let tab = fixture.tab();
    assert_eq!(
        tab_session_dots(&output, tab).len(),
        1,
        "only Claude Code dots while it is present"
    );
    assert!(
        text_rects(&output, "Grok Build", 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "Grok Build yields to Claude Code"
    );

    fixture.emit(CLAUDE_ESC_CLEAR);
    let output = fixture.labeled("Grok Build");
    let tab = fixture.tab();
    assert_eq!(
        tab_session_dots(&output, tab).len(),
        2,
        "the Grok dots return after Claude Code ends"
    );
}

#[cfg(unix)]
#[test]
fn a_direct_claude_code_tab_is_branded_before_any_status() {
    if isolated("a_direct_claude_code_tab_is_branded_before_any_status") {
        return;
    }
    use std::os::unix::fs::PermissionsExt;

    let bin = std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
        .next()
        .expect("isolated bin");
    let claude = bin.join("claude");
    std::fs::write(&claude, "#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "current", "main.rs");
    let (mut headless, first) = menu_model(&root, &[CLAUDE_CODE]);
    headless.ctx.enable_accesskit();
    headless.click(top_action_id("open-folder"));
    headless.click(new_tab_menu_item_id("claude"));
    let mut direct = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while direct.is_none() && Instant::now() < deadline {
        headless.render();
        direct = headless.model.workspace_ids().find(|id| *id != first);
        std::thread::sleep(Duration::from_millis(20));
    }
    let direct = direct.expect("the Claude Code tab opened");
    headless.pump_until(|model| {
        model
            .workspace(direct)
            .and_then(Workbench::terminal)
            .is_some_and(made_app::TerminalPane::is_running)
    });
    assert!(
        headless
            .model
            .workspace(first)
            .and_then(Workbench::terminal)
            .is_some_and(made_app::TerminalPane::is_running),
        "the current tab keeps running"
    );
    headless.render();
    let output = headless.render();
    let tab = headless.response(workspace_tab_id(direct)).rect;
    let label = text_rects(&output, "Claude Code", 55.0)
        .into_iter()
        .find(|rect| tab.contains_rect(*rect))
        .expect("a direct launch is labeled Claude Code before any status");
    assert_claude_mark(&headless, &output, tab, label.left(), "the direct tab");
    assert!(
        tab_session_dots(&output, tab).is_empty(),
        "no dot before any status or record"
    );
    let id = workspace_tab_id(direct);
    let hovered = output_text(&headless.hover_until_text(id, "Claude Code"));
    assert!(
        !hovered.contains(&root.display().to_string()),
        "the tooltip is Claude Code without a path: {hovered}"
    );
    let output = headless.render();
    let (_, name, _) = accesskit_row_state(&output, id);
    assert_eq!(name.as_deref(), Some("Claude Code"));
}

#[test]
fn claude_code_diagnostics_row_stays_hidden_until_revealed() {
    if isolated("claude_code_diagnostics_row_stays_hidden_until_revealed") {
        return;
    }
    stage_claude_plugin();
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "hidden-row", "main.rs");
    let (mut headless, id) = menu_model(&root, &[CLAUDE_CODE]);
    headless.click(made_app::theme::wordmark_id());
    let before = output_text(&headless.render());
    assert!(
        !before.contains("Claude Code is not running.")
            && !before.contains("Claude Code is not on PATH."),
        "the row is hidden until Show diagnostics is activated: {before}"
    );
    let pane_id = headless.model.workspace(id).unwrap().pane_id().to_owned();
    headless.click(about_diagnostics_toggle_id(&pane_id));
    settle_about_animation(&mut headless);
    let after = output_text(&headless.render());
    assert!(
        after.contains("Claude Code is not running."),
        "an explicit catalog without detection uses the running state: {after}"
    );
    let grok = after.find("Grok Build is not on PATH.").expect("grok row");
    let claude = after.find("Claude Code is not running.").unwrap();
    assert!(
        grok < claude,
        "the Claude Code row follows the Grok Build row"
    );
}

#[test]
fn claude_code_row_is_not_on_path_without_a_catalog_entry() {
    if isolated("claude_code_row_is_not_on_path_without_a_catalog_entry") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "no-claude", "main.rs");
    let (mut headless, id) = menu_model(&root, &[OPENCODE]);
    open_about_diagnostics(&mut headless, id);
    let text = output_text(&headless.render());
    assert!(text.contains("Claude Code is not on PATH."), "{text}");
}

#[cfg(target_os = "linux")]
#[test]
fn claude_code_row_walks_not_running_starting_and_ready() {
    if isolated("claude_code_row_walks_not_running_starting_and_ready") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = ClaudeFixture::new(temp.path());
    let id = fixture.id;
    open_about_diagnostics(&mut fixture.headless, id);
    let text = output_text(&fixture.headless.render());
    assert!(text.contains("Claude Code is not running."), "{text}");

    // The capability probe alone is not presence.
    fixture.emit(b"\x1b]7501;?\x07");
    std::thread::sleep(Duration::from_millis(200));
    let text = output_text(&fixture.headless.render());
    assert!(text.contains("Claude Code is not running."), "{text}");

    // A status emission with no hook record: present, still starting.
    fixture.emit(CLAUDE_ESC_WORKING);
    let starting = |text: &str| text.contains("Claude Code is starting.");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut text = String::new();
    while !starting(&text) && Instant::now() < deadline {
        text = output_text(&fixture.headless.render());
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(starting(&text), "{text}");

    // A live owned session plus the observed protocol is ready, with no wait.
    let cwd = fixture.root.clone();
    fixture.hook(&cwd, "SessionStart", "idle", "");
    let ready = |text: &str| text.contains("Claude Code session follow is ready.");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready(&text) && Instant::now() < deadline {
        text = output_text(&fixture.headless.render());
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(ready(&text), "{text}");

    // The copied report carries the same sentence and no title or message.
    let report = fixture
        .headless
        .model
        .workspace(id)
        .unwrap()
        .sanitized_diagnostics_text();
    assert!(
        report.contains("claude: Claude Code session follow is ready."),
        "{report}"
    );
    assert!(report.contains("claude status: ready"), "{report}");
}

/// Assert the fixture tab is the connected OpenCode tab: label, 16px
/// `opencode.svg`, one busy and one idle dot, no Claude Code branding.
#[cfg(target_os = "linux")]
fn assert_connected_opencode_tab(fixture: &mut ClaudeFixture, phase: &str) {
    let output = fixture.labeled("OpenCode");
    let tab = fixture.tab();
    assert!(
        text_rects(&output, "OpenCode", 55.0)
            .iter()
            .any(|rect| tab.contains_rect(*rect)),
        "{phase}: the tab stays labeled OpenCode"
    );
    assert!(
        text_rects(&output, "Claude Code", 55.0)
            .iter()
            .all(|rect| !tab.contains_rect(*rect)),
        "{phase}: the tab is not labeled Claude Code"
    );
    let dots = tab_session_dots(&output, tab);
    assert_eq!(dots.len(), 2, "{phase}: only the OpenCode dots");
    assert_session_dot_color(&dots[0], made_app::theme::ACCENT, "the OpenCode busy dot");
    assert_session_dot_color(
        &dots[1],
        made_app::theme::TEXT_TERTIARY,
        "the OpenCode idle dot",
    );
    let icon = painted_images(&output)
        .into_iter()
        .find(|image| {
            (image.rect.width() - 16.0).abs() < 0.75
                && (image.rect.height() - 16.0).abs() < 0.75
                && tab.contains_rect(image.rect)
        })
        .unwrap_or_else(|| panic!("{phase}: the tab keeps a 16px icon"));
    assert_eq!(
        icon.texture,
        icon_texture(&fixture.headless.ctx, icon_named("opencode.svg")),
        "{phase}: the icon is opencode.svg, not the Claude mark"
    );
}

/// Spec `OpenCode tab unchanged` and the Claude Code precedence scenario: a
/// connected OpenCode view keeps the tab's label, icon, dots and explorer root
/// while Claude Code is present in the same shell tab.
#[cfg(target_os = "linux")]
#[test]
fn opencode_keeps_label_icon_dots_and_root_while_claude_code_is_present() {
    if isolated("opencode_keeps_label_icon_dots_and_root_while_claude_code_is_present") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = ClaudeFixture::new(temp.path());
    let id = fixture.id;
    let root = fixture.root.clone();
    let claude_dir = temp.path().join("claude-dir");
    fs::create_dir(&claude_dir).unwrap();
    let claude_dir = claude_dir.canonicalize().unwrap();

    // A shell-typed `opencode` finds the OpenCode plugin configuration in the
    // shell tab's environment.
    let options = bridge_options(fixture.headless.model.workspace(id).unwrap())
        .expect("the shell tab carries the OpenCode plugin configuration");
    let mut socket = connect(&options);
    fixture
        .headless
        .pump_until(|model| model.workspace(id).unwrap().is_connected());
    send_sessions_state(
        &mut socket,
        1,
        &root,
        &[
            ("busy-session", true, false),
            ("idle-session", false, false),
        ],
    );
    fixture
        .headless
        .pump_until(|model| model.workspace(id).unwrap().session_map().rows().len() == 2);
    service_bridge_heartbeat(socket);

    // Phase 1: no Claude Code present.
    assert!(
        !fixture
            .headless
            .model
            .workspace(id)
            .and_then(claude_code::tab)
            .is_some_and(|tab| tab.present()),
        "Claude Code is not present before any source"
    );
    assert_connected_opencode_tab(&mut fixture, "without Claude Code");
    assert_eq!(
        fixture
            .headless
            .model
            .workspace(id)
            .unwrap()
            .explorer_root(),
        Some(root.as_path())
    );

    // Phase 2: Claude Code becomes present through both channels, with a
    // working directory elsewhere. OpenCode keeps everything.
    fixture.emit(CLAUDE_ESC_WORKING);
    fixture.hook(&claude_dir, "SessionStart", "idle", "Claude title");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        fixture.headless.render();
        let present = fixture
            .headless
            .model
            .workspace(id)
            .and_then(claude_code::tab)
            .is_some_and(|tab| tab.present());
        if present {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Claude Code never became present"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    for _ in 0..30 {
        fixture.headless.render();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_connected_opencode_tab(&mut fixture, "with Claude Code present");
    let workbench = fixture.headless.model.workspace(id).unwrap();
    assert_eq!(
        workbench.explorer_root(),
        Some(root.as_path()),
        "Claude Code does not take the explorer root from OpenCode"
    );
    assert_eq!(
        workbench.session_map().rows().len(),
        2,
        "the OpenCode session map is untouched"
    );
}

/// Spec `OpenCode tab unchanged` (menu launch): the OpenCode child keeps its
/// launch injection and receives no Claude Code plugin entry or spool.
#[cfg(unix)]
#[test]
fn a_menu_launched_opencode_tab_keeps_its_injection_and_gets_no_claude_entry() {
    if isolated("a_menu_launched_opencode_tab_keeps_its_injection_and_gets_no_claude_entry") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = fixture_workspace(temp.path(), "current", "main.rs");
    let (mut headless, first) = menu_model(&root, &[OPENCODE, CLAUDE_CODE]);
    headless.model.begin_tui_workspace(OPENCODE);
    let mut opened = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while opened.is_none() && Instant::now() < deadline {
        headless.render();
        opened = headless.model.workspace_ids().find(|id| *id != first);
        std::thread::sleep(Duration::from_millis(20));
    }
    let opened = opened.expect("the OpenCode tab opened");
    let workbench = headless.model.workspace(opened).unwrap();
    let terminal = workbench.terminal().expect("the OpenCode child spawned");
    let options = terminal.options();
    assert!(
        Path::new(options.shell.as_deref().expect("explicit program")).ends_with("opencode"),
        "{:?}",
        options.shell
    );
    assert!(options.args.is_empty(), "no argument is added to opencode");
    assert!(
        bridge_options(workbench).is_some(),
        "the OpenCode plugin configuration is still injected"
    );
    for key in [
        claude_code::launch::CLAUDE_CODE_PLUGIN_DIRS_ENV,
        "MADE_CLAUDE_SPOOL",
    ] {
        assert!(
            options.env.iter().all(|(name, _)| name != key),
            "{key} must not reach the OpenCode launch"
        );
    }
    assert!(
        options
            .env_remove
            .iter()
            .any(|key| key == "MADE_CLAUDE_SPOOL"),
        "an inherited spool binding is stripped"
    );
    assert!(
        claude_code::tab(workbench).is_none(),
        "an OpenCode tab carries no Claude Code state"
    );
    let report = workbench.sanitized_diagnostics_text();
    assert!(
        !report.contains("Claude Code session"),
        "no Claude Code session wording in an OpenCode tab: {report}"
    );
}

/// Narrowed `Shell Directory Follow` regression: OSC 0/2 window-title changes
/// in a shell tab, with and without a live Claude Code protocol status, never
/// change the shell cwd or the explorer root.
#[cfg(target_os = "linux")]
#[test]
fn osc_title_changes_never_change_the_shell_cwd_or_explorer_root() {
    if isolated("osc_title_changes_never_change_the_shell_cwd_or_explorer_root") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = ClaudeFixture::new(temp.path());
    let id = fixture.id;
    let root = fixture.root.clone();
    let mut elsewhere = Vec::new();
    for name in ["title-a", "title-b", "title-c"] {
        let path = temp.path().join(name);
        fs::create_dir(&path).unwrap();
        elsewhere.push(path.canonicalize().unwrap());
    }
    let shell_cwd = fixture
        .headless
        .model
        .workspace(id)
        .unwrap()
        .shell_cwd()
        .to_path_buf();
    let settle = |fixture: &mut ClaudeFixture| {
        for _ in 0..30 {
            fixture.headless.render();
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let assert_unmoved = |fixture: &ClaudeFixture, phase: &str| {
        let workbench = fixture.headless.model.workspace(id).unwrap();
        assert_eq!(
            workbench.explorer_root(),
            Some(root.as_path()),
            "{phase}: a title never moves the explorer"
        );
        assert_eq!(
            workbench.shell_cwd(),
            shell_cwd.as_path(),
            "{phase}: a title never changes the shell cwd"
        );
        assert!(
            workbench.shell_cwd_error().is_none(),
            "{phase}: a title is not a cwd report"
        );
    };

    // Without any Claude Code status.
    for dir in &elsewhere[..2] {
        fixture.emit(format!("\x1b]0;{}\x07", dir.display()).as_bytes());
        fixture.emit(format!("\x1b]2;cd {}\x07", dir.display()).as_bytes());
    }
    settle(&mut fixture);
    assert_unmoved(&fixture, "without Claude Code status");

    // With a live Claude Code protocol status the title may be shown, but it
    // still never drives a directory.
    fixture.emit(CLAUDE_ESC_WORKING);
    fixture.labeled("Claude Code");
    fixture.emit(format!("\x1b]0;{}\x07", elsewhere[2].display()).as_bytes());
    fixture.emit(format!("\x1b]2;{}\x07", elsewhere[1].display()).as_bytes());
    settle(&mut fixture);
    assert_unmoved(&fixture, "with live Claude Code status");

    fixture.emit(CLAUDE_ESC_CLEAR);
    fixture.emit(format!("\x1b]0;{}\x07", elsewhere[0].display()).as_bytes());
    settle(&mut fixture);
    assert_unmoved(&fixture, "after Claude Code status clears");
}

/// Spec `No prompt or message content anywhere`, `Window titles and screen are
/// not used` and `Permission message never shown`, at the rendering level:
/// content markers carried by spool records never appear in the tooltip, the
/// About diagnostics, the copied report or the log, and a window-title marker
/// reaches the tooltip only while a protocol status is live.
#[cfg(target_os = "linux")]
#[test]
fn claude_content_markers_never_reach_the_tooltip_diagnostics_report_or_log() {
    if isolated("claude_content_markers_never_reach_the_tooltip_diagnostics_report_or_log") {
        return;
    }
    use std::io::Write;

    const CONTENT: [&str; 4] = [
        "MARKER-PROMPT-5e2b",
        "MARKER-TOOL-INPUT-5e2b",
        "MARKER-MESSAGE-5e2b",
        "MARKER-MSG-5e2b",
    ];
    const TITLE_MARKER: &str = "TITLE-MARKER-5e2b";
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = ClaudeFixture::new(temp.path());
    let id = fixture.id;
    let shell = fixture
        .headless
        .model
        .workspace(id)
        .and_then(Workbench::terminal)
        .and_then(made_app::TerminalPane::pid)
        .unwrap();
    let append = |fixture: &ClaudeFixture, event: &str, status: &str| {
        let line = json!({
            "at": "t", "sessionId": "s1", "cwd": fixture.root, "event": event,
            "notification": "", "status": status, "title": "",
            "pids": [4_000_000u32, fixture.claude_pid, shell],
            "prompt": CONTENT[0], "tool_input": {"command": CONTENT[1]},
            "message": CONTENT[2], "msg": CONTENT[3],
        });
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&fixture.spool)
            .unwrap();
        writeln!(file, "{line}").unwrap();
    };
    let assert_clean = |text: &str, what: &str| {
        for marker in CONTENT {
            assert!(!text.contains(marker), "{marker} leaked into {what}");
        }
    };

    // Hook-only presence with a window-title marker: no live protocol status,
    // so the title is not read and the tooltip is the plain name.
    fixture.emit(format!("\x1b]0;{TITLE_MARKER} title\x07").as_bytes());
    append(&fixture, "SessionStart", "idle");
    append(&fixture, "UserPromptSubmit", "working");
    append(&fixture, "Notification", "attention");
    fixture.labeled("Claude Code");
    let hovered = fixture.hover_tab("Claude Code");
    assert_clean(&hovered, "the hook-only tooltip");
    assert!(
        !hovered.contains(TITLE_MARKER),
        "the window title is not shown without a live protocol status: {hovered}"
    );

    // With a live protocol status the sanitized title is the tooltip, and
    // still no content marker appears.
    fixture.emit(CLAUDE_ESC_WORKING);
    fixture.emit(format!("\x1b]0;{TITLE_MARKER} title\x07").as_bytes());
    fixture.labeled("Claude Code");
    let hovered = fixture.hover_tab(TITLE_MARKER);
    assert!(hovered.contains(TITLE_MARKER), "{hovered}");
    assert_clean(&hovered, "the protocol tooltip");

    // About diagnostics, the copied report and the log.
    open_about_diagnostics(&mut fixture.headless, id);
    let about = output_text(&fixture.headless.render());
    assert_clean(&about, "the About diagnostics");
    assert!(!about.contains(TITLE_MARKER), "{about}");
    let report = fixture
        .headless
        .model
        .workspace(id)
        .unwrap()
        .sanitized_diagnostics_text();
    assert_clean(&report, "the copied report");
    assert!(!report.contains(TITLE_MARKER), "{report}");
    let log = fixture
        .headless
        .model
        .workspace(id)
        .unwrap()
        .diagnostics()
        .log()
        .entries()
        .map(|entry| entry.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert_clean(&log, "the workspace log");
    assert!(!log.contains(TITLE_MARKER));
    if let Ok(file) = fs::read_to_string(isolated_log_path()) {
        assert_clean(&file, "the integration log");
        assert!(!file.contains(TITLE_MARKER));
    }
}
