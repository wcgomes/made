//! Headless egui UI-model test: picker -> selected folder -> workspace
//! surfaces, without a display, GPU, window, PTY, or bridge.

use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use made_app::app::{AppModel, AppOptions, WorkspaceId};
use made_app::cli::LaunchArgs;
use made_app::explorer_view::{ExplorerPane, PreviewAction};
use made_app::ui::{
    diagnostics_copy_id, diagnostics_previous_log_copy_id, diagnostics_previous_log_path_id,
    explorer_splitter_id, left_surface_action_id, picker_control_id, picker_current_target,
    picker_list_id, picker_path_id, top_action_id, top_workspace_action_id, workspace_tab_close_id,
    workspace_tab_id,
};
use made_app::workbench::LeftSurface;
use made_app::workspace::{
    LaunchPlan, Surfaces, WorkspaceLaunch, WorkspaceSelection, WorkspaceSource,
};
use made_app::workspace_picker::WorkspacePicker;
use made_app::Workbench;
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
        let deadline = Instant::now() + Duration::from_secs(6);
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

fn path_center(path: &egui::epaint::PathShape) -> egui::Pos2 {
    egui::Rect::from_points(&path.points).center()
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

fn isolated(name: &str) -> bool {
    if std::env::var_os("MADE_UI_MODEL_CHILD").is_some() {
        return false;
    }
    let home = tempfile::tempdir().unwrap();
    let tmp = home.path().join("tmp");
    let runtime = home.path().join("runtime");
    let roaming = home.path().join("AppData").join("Roaming");
    let local = home.path().join("AppData").join("Local");
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
        .args(["--exact", name, "--nocapture"])
        .env("MADE_UI_MODEL_CHILD", "1")
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
    if std::path::Path::new("/bin/bash").exists() {
        command.env("SHELL", "/bin/bash");
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
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

fn fixture_workspace(temp: &Path, name: &str, file: &str) -> PathBuf {
    let path = temp.join(name);
    fs::create_dir(&path).unwrap();
    fs::write(path.join(file), "fixture contents\n").unwrap();
    path.canonicalize().unwrap()
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
        top_action_id("diagnostics"),
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
                && shape.stroke == egui::Stroke::NONE),
        "active workspace tab must use the opaque selected fill with no border"
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

    let wordmark = output_text(&headless.hover_until_text(
        made_app::theme::wordmark_id(),
        made_app::theme::WORDMARK_TOOLTIP,
    ));
    assert!(
        wordmark.contains(made_app::theme::WORDMARK_TOOLTIP),
        "wordmark tooltip missing: {wordmark}"
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
        egui::Stroke::NONE,
        "selected workspace tab must not paint a border"
    );
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
        !rect_shapes(&output)
            .iter()
            .any(|shape| { shape.rect == active_tab && shape.stroke != egui::Stroke::NONE }),
        "selected workspace tab must not paint any stroke"
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
        "inactive workspace tab must have no rest border"
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
        "hovered workspace tab must not paint a border"
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
                && (shape.stroke == egui::Stroke::new(1.0, made_app::theme::ACCENT)
                    || shape.stroke == egui::Stroke::new(1.0, made_app::theme::BORDER))
        }),
        "hovered workspace tab must not paint an accent or neutral border"
    );

    let selected_hover = headless.hover(workspace_tab_id(beta_id));
    let selected_hover_shape = rect_shapes(&selected_hover)
        .into_iter()
        .find(|shape| shape.rect == active_tab && shape.fill == made_app::theme::BG_TAB_SELECTED)
        .expect("hovered selected workspace tab must keep the selected fill");
    assert_eq!(
        selected_hover_shape.stroke,
        egui::Stroke::NONE,
        "hovered selected workspace tab must not paint a border"
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
    let output = headless.hover_until_text(workspace_tab_close_id(alpha_id), "Close workspace");
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
        "close hover must not paint a parent tab border"
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
    let active_output =
        headless.hover_until_text(workspace_tab_close_id(beta_id), "Close workspace");
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
        egui::Stroke::NONE,
        "selected-tab close hover must not paint a border"
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
        (folder_text.left() - (explorer_left + 32.0)).abs() < 1.0,
        "depth-0 text must start 24px from the row edge: {folder_text:?}"
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
        "folder disclosure must be an app-drawn shape, not a font glyph"
    );
    assert!(
        !output_text(&output).contains('·'),
        "file bullet must be an app-drawn shape, not a font glyph"
    );
    let folder_marker = path_shapes(&output)
        .into_iter()
        .filter(|path| path.points.len() == 3 && path.fill == made_app::theme::TEXT_PRIMARY)
        .find(|path| {
            let center = path_center(path);
            folder_rect.contains(center) && (center.x - (folder_rect.left() + 12.0)).abs() < 1.5
        })
        .expect("expanded folder app-drawn disclosure triangle");
    let folder_marker_rect = egui::Rect::from_points(&folder_marker.points);
    assert!(
        (folder_marker_rect.center().y - folder_text.center().y).abs() < 0.75,
        "disclosure must share the label's optical center: {folder_marker_rect:?} vs {folder_text:?}"
    );
    let file_marker = circle_shapes(&output)
        .into_iter()
        .filter(|circle| circle.radius <= 3.0 && file_rect.contains(circle.center))
        .find(|circle| (circle.center.x - (folder_marker_rect.center().x + 14.0)).abs() < 1.5)
        .expect("file app-drawn bullet");
    assert!(
        (file_marker.center.y - file_text.center().y).abs() < 0.75,
        "file bullet must share the label's optical center: {file_marker:?} vs {file_text:?}"
    );
    assert!(
        (file_marker.center.x - (folder_marker_rect.center().x + 14.0)).abs() < 1.5,
        "depth must shift the icon slot by 14px: {file_marker:?} vs {folder_marker_rect:?}"
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
        top_action_id("diagnostics"),
    ] {
        let response = headless.response(action);
        assert!(
            response.rect.width() >= 24.0 && response.rect.height() >= 24.0,
            "icon target below 24px: {action:?} {:?}",
            response.rect
        );
    }
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
    let hovered =
        output_text(&headless.hover_until_text(top_action_id("diagnostics"), "Diagnostics"));
    assert!(
        hovered.contains("Diagnostics"),
        "Diagnostics tooltip missing: {hovered}"
    );
    headless.focus(top_action_id("open-folder"));
    let focused = headless.render();
    assert!(
        has_stroke_color(&focused, made_app::theme::FOCUS),
        "focused header icon must paint a visible focus ring"
    );
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
        let diagnostics = headless.response(top_action_id("diagnostics")).rect;
        let explorer = headless
            .response(top_workspace_action_id(beta_id, "explorer"))
            .rect;
        let content_left = layout.gutter;
        let content_right = width - layout.gutter;

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
        assert!(
            (diagnostics.left() - wordmark.right() - gap).abs() < 0.75,
            "{width}px: wordmark-to-icon gap must be {gap}px: wordmark {wordmark:?}, diagnostics {diagnostics:?}"
        );
        assert!(
            explorer.left() > diagnostics.left() && explorer.right() > diagnostics.right(),
            "{width}px: Explorer toggle must be rightmost: diagnostics {diagnostics:?}, explorer {explorer:?}"
        );
        assert!(
            plus.width() >= 24.0 && plus.height() >= 24.0,
            "{width}px: Open folder target below 24px: {plus:?}"
        );
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
        for (name, rect) in [
            ("wordmark", wordmark),
            ("plus", plus),
            ("diagnostics", diagnostics),
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
        let icon_cluster_left = diagnostics.left();
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

        let wordmark_hover = headless.hover_until_text(
            made_app::theme::wordmark_id(),
            made_app::theme::WORDMARK_TOOLTIP,
        );
        assert!(
            output_text(&wordmark_hover).contains(made_app::theme::WORDMARK_TOOLTIP),
            "{width}px: wordmark tooltip missing"
        );
        assert!(
            !rects_with_fill(&wordmark_hover, made_app::theme::BG_TAB_HOVER)
                .iter()
                .any(|rect| rect.intersects(wordmark)),
            "{width}px: hovering the wordmark must not paint a tab fill"
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

        headless.focus(top_action_id("open-folder"));
        headless.press_key(egui::Key::Enter);
        headless.render();
        assert!(
            headless.model.is_picker(),
            "{width}px: Enter on the focused Open folder plus must activate the picker"
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
    for (name, rect) in [("plus", plus), ("overflow", overflow)] {
        assert!(
            rect.width() >= 24.0 && rect.height() >= 24.0,
            "{name} target below 24px or clipped: {rect:?}"
        );
    }
    for (name, rect) in [
        ("plus", plus),
        ("overflow", overflow),
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

    let hovered_tab = headless.hover(alpha_tab_id);
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

    let hovered_close = headless.hover(alpha_close_id);
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
        "i=0; while [ $i -lt 250 ]; do echo tick; i=$((i+1)); sleep 0.01; done",
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
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        headless.model.logic(&headless.ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        headless
            .model
            .workspace(id)
            .unwrap()
            .terminal()
            .unwrap()
            .output_bytes()
            > before_output,
        "hidden terminal must keep consuming output"
    );

    headless.render_events(typed_events(&format!(
        "printf leaked > '{}'\n",
        marker.display()
    )));
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        headless.model.logic(&headless.ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
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
        largest_rect(&output, made_app::theme::BG_TERMINAL).expect("terminal card painted");
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
    let hidden_terminal = largest_rect(&hidden, made_app::theme::BG_TERMINAL)
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
    headless.render_events(vec![egui::Event::PointerMoved(away)]);
    let leaving_center = headless.response(splitter).rect.center();
    let leaving = headless.render();
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
            headless.response(top_action_id("diagnostics")).rect,
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
    headless.click(top_action_id("diagnostics"));
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
    headless.click(top_action_id("diagnostics"));
    let text = output_text(&headless.render());
    for expected in ["Previous log", "Copy previous log path", "not yet rotated"] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
    let pane_id = headless.model.workspace(id).unwrap().pane_id();
    let previous_copy_id = diagnostics_previous_log_copy_id(pane_id);
    let response = headless.response(previous_copy_id);
    assert!(
        !response.enabled(),
        "copy action must be disabled without a previous log"
    );
    assert!(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(720.0, 480.0))
            .contains_rect(response.rect),
        "previous-log action clipped at 720x480: {:?}",
        response.rect
    );

    std::fs::write(&previous, b"previous record\n").unwrap();
    // The `.1` existence probe is bucketed to at most once per second.
    headless.render_events_at(Vec::new(), headless.last_time + 1.1);
    let text = output_text(&headless.render());
    assert!(!text.contains("not yet rotated"), "{text}");
    assert!(headless.response(previous_copy_id).enabled());

    let pane_id = headless.model.workspace(id).unwrap().pane_id();
    let path_id = diagnostics_previous_log_path_id(pane_id);
    let window_id = egui::Id::unique(("made-diagnostics-window", pane_id));
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
    headless.click(top_action_id("diagnostics"));
    let output = headless.render();
    let text = output_text(&output);
    for expected in ["degraded", "Previous log", "not yet rotated"] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
    let pane_id = headless.model.workspace(id).unwrap().pane_id();
    let copy_id = diagnostics_previous_log_copy_id(pane_id);
    let window = headless
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::unique(("made-diagnostics-window", pane_id))))
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
    headless.click(top_action_id("diagnostics"));
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
    headless.click(top_action_id("diagnostics"));
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
    let window_id = egui::Id::unique(("made-diagnostics-window", pane_id));
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
    let diagnostics = headless.response(top_action_id("diagnostics")).rect;
    let explorer = headless
        .response(top_workspace_action_id(id, "explorer"))
        .rect;
    assert!(
        sessions.right() < diagnostics.left()
            && diagnostics.right() < explorer.left()
            && explorer.left() > sessions.left(),
        "right icon order must be Sessions, Diagnostics, Explorer (Explorer rightmost): sessions {sessions:?}, diagnostics {diagnostics:?}, explorer {explorer:?}"
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
    for action in ["open-folder", "diagnostics"] {
        let response = headless.response(top_action_id(action));
        assert!(
            screen.contains_rect(response.rect),
            "{action} control is clipped: {:?}",
            response.rect
        );
        assert!(response.rect.width() >= 24.0 && response.rect.height() >= 24.0);
    }
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
