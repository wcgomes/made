//! Headless egui UI-model test: picker -> selected folder -> workspace
//! surfaces, without a display, GPU, window, PTY, or bridge.

use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use made_app::app::{AppModel, AppOptions, WorkspaceId};
use made_app::cli::LaunchArgs;
use made_app::explorer_view::PreviewAction;
use made_app::ui::{
    diagnostics_copy_id, explorer_splitter_id, left_surface_action_id, picker_control_id,
    picker_current_target, picker_list_id, top_action_id, top_workspace_action_id,
    workspace_tab_close_id, workspace_tab_id,
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
        .selected_entry()
        .unwrap()
        .name()
        .to_owned();
    assert_eq!(selected_name, "alpha");
    let plan = model.launch().picker().unwrap().confirm_entry(0).unwrap();
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
        }
    }

    fn raw(&mut self, events: Vec<egui::Event>) -> egui::RawInput {
        self.frame = self.frame.saturating_add(1);
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(self.width, self.height),
            )),
            time: Some(self.started.elapsed().as_secs_f64()),
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

    fn press_key(&mut self, key: egui::Key) {
        self.render_events(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
    }

    fn settle_animation(&mut self) {
        for _ in 0..12 {
            self.render();
            std::thread::sleep(Duration::from_millis(16));
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

fn isolated(name: &str) -> bool {
    if std::env::var_os("MADE_UI_MODEL_CHILD").is_some() {
        return false;
    }
    let home = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", name, "--nocapture"])
        .env("MADE_UI_MODEL_CHILD", "1")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CACHE_HOME", home.path().join("cache"));
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
    path
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

    let active_tab = rects_with_fill(&output, made_app::theme::BG_CARD)
        .into_iter()
        .find(|rect| rect.top() < 70.0)
        .expect("active workspace tab must use a full-opacity background");
    assert!(active_tab.width() > 40.0);
    assert!(
        !output.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Circle(circle) => {
                (circle.fill == made_app::theme::ACCENT
                    || circle.fill == made_app::theme::TEXT_TERTIARY)
                    && circle.radius <= 4.0
            }
            _ => false,
        }),
        "workspace tabs must not paint dot markers"
    );
    assert!(
        rects_with_fill(&output, made_app::theme::ACCENT)
            .iter()
            .any(|rect| rect.height() <= 3.0 && rect.width() > 20.0),
        "active workspace tab must keep its underline"
    );

    let band = rects_with_fill(&output, made_app::theme::BG_CHROME_GLASS)
        .into_iter()
        .max_by(|left, right| left.area().total_cmp(&right.area()))
        .expect("glass top band must paint");
    assert!(
        (band.height() - 40.0).abs() < 2.0 && band.width() > 1200.0,
        "top glass band geometry: {band:?}"
    );
    assert!(
        !rects_with_fill(&output, made_app::theme::GLASS_HIGHLIGHT).is_empty(),
        "glass band must paint an inner top highlight"
    );
    assert!(
        !rects_with_fill(&output, made_app::theme::GLASS_SHADOW).is_empty(),
        "glass band must paint a soft shadow"
    );
    assert!(
        !rects_with_fill(&output, made_app::theme::BG_HEADER_GLASS).is_empty(),
        "explorer header must use the glass header tone"
    );
    let explorer_card =
        largest_rect(&output, made_app::theme::BG_CARD_GLASS).expect("explorer card must paint");
    assert!(
        (explorer_card.height() - 732.0).abs() < 2.0,
        "explorer must fill the main content height: {explorer_card:?}"
    );

    let wordmark = output_text(&headless.hover(made_app::theme::wordmark_id()));
    assert!(
        wordmark.contains(made_app::theme::WORDMARK_TOOLTIP),
        "wordmark tooltip missing: {wordmark}"
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
    let hovered = output_text(&headless.hover(top_action_id("open-folder")));
    assert!(
        hovered.contains("Open folder"),
        "Open folder tooltip missing: {hovered}"
    );
    let hovered = output_text(&headless.hover(top_workspace_action_id(id, "explorer")));
    assert!(
        hovered.contains("Hide explorer"),
        "Explorer tooltip missing: {hovered}"
    );
    let hovered = output_text(&headless.hover(top_action_id("diagnostics")));
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
    headless.drag(splitter, egui::vec2(-40.0, 0.0));
    let after_drag = headless.model.workspace(id).unwrap().explorer_width();
    assert!(
        after_drag > before_drag,
        "dragging the splitter left must widen the explorer"
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
    for _ in 0..40 {
        let output = headless.render();
        if let Some(response) = headless.ctx.read_response(splitter) {
            let left = response.rect.left();
            if left > full_left + 4.0 && left < 1270.0 {
                closing = Some(output);
                break;
            }
        } else {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
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
    for _ in 0..60 {
        let output = headless.render();
        if let Some(response) = headless.ctx.read_response(splitter) {
            let left = response.rect.left();
            if left > full_left + 4.0 && left < 1270.0 {
                opening = Some(output);
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
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
    let hovered = headless.render_events(vec![egui::Event::PointerMoved(position)]);
    assert!(
        has_stroke_color(&hovered, made_app::theme::BORDER_CONTROL_HOVER),
        "hovered splitter must reveal a thin line and grip"
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
    let dragged = headless.render();
    assert!(
        has_stroke_color(&dragged, made_app::theme::ACCENT),
        "dragging the splitter must reveal an accent grip"
    );
    assert!(
        headless.model.workspace(id).unwrap().explorer_width() > before_hover,
        "dragging left must widen the explorer"
    );

    headless.render_events(vec![pointer_button(end, false)]);
    let away = egui::pos2(100.0, 400.0);
    headless.render_events(vec![egui::Event::PointerMoved(away)]);
    let rest = headless.render();
    let rested_center = headless.response(splitter).rect.center();
    assert!(
        !shape_hits_point(&rest, rested_center),
        "the splitter paint must disappear when the pointer leaves"
    );
    assert!(
        !has_stroke_color(&rest, made_app::theme::BORDER_CONTROL_HOVER),
        "no permanent hover line may remain at rest"
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
        let layout = made_app::theme::Layout::for_window(width, height);
        let explorer_width = layout.explorer_width(
            headless.model.workspace(id).unwrap().explorer_width(),
            layout.content_width(),
        );
        let expected_left = layout.content_width() - layout.gap - explorer_width;
        let expected_height =
            height - (layout.gutter + layout.top_height + layout.gap) - layout.gutter;
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
    let card_top = layout.gutter + layout.top_height + layout.gap;
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
fn picker_close_without_workspace_falls_back_to_home() {
    if isolated("picker_close_without_workspace_falls_back_to_home") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("alpha")).unwrap();
    let mut closed = picker_headless(temp.path(), 900.0, 620.0);
    closed.ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.0
    });
    let picker_output = closed.render();
    let text = output_text(&picker_output);
    assert!(
        !rects_with_fill(&picker_output, made_app::theme::BG_SELECTED).is_empty(),
        "selected picker row must keep its rounded background"
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
    let hovered = output_text(&closed.hover(picker_control_id("close")));
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
    let hovered = output_text(&headless.hover(top_workspace_action_id(id, "sessions")));
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
        (explorer_card.height() - 422.0).abs() < 2.0,
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
    let alpha = temp.path().join("alpha");
    let beta = temp.path().join("beta");
    fs::create_dir(&alpha).unwrap();
    fs::create_dir(&beta).unwrap();

    let mut up = picker_headless(&alpha, 900.0, 620.0);
    up.render();
    up.focus(picker_control_id("up"));
    up.press_key(egui::Key::Enter);
    assert_eq!(
        up.model.launch().picker().unwrap().current_dir(),
        temp.path()
    );
    assert!(up.model.is_picker());

    let mut refresh = picker_headless(temp.path(), 900.0, 620.0);
    refresh.render();
    let gamma = temp.path().join("gamma");
    fs::create_dir(&gamma).unwrap();
    refresh.focus(picker_control_id("refresh"));
    refresh.press_key(egui::Key::Enter);
    let refreshed = output_text(&refresh.render());
    assert!(
        refreshed.contains("gamma"),
        "refresh did not re-list: {refreshed}"
    );

    let mut list = picker_headless(temp.path(), 900.0, 620.0);
    list.render();
    list.focus(picker_list_id());
    list.press_key(egui::Key::ArrowDown);
    assert_eq!(list.model.launch().picker().unwrap().selection(), Some(1));
    list.press_key(egui::Key::Enter);
    assert_eq!(list.model.launch().picker().unwrap().current_dir(), beta);

    let mut button_switch = picker_headless(temp.path(), 900.0, 620.0);
    button_switch.render();
    button_switch.focus(picker_control_id("up"));
    button_switch.press_key(egui::Key::ArrowDown);
    assert_eq!(
        button_switch.model.launch().picker().unwrap().selection(),
        Some(0),
        "a focused button must not let global arrows move the list"
    );

    let mut selected = picker_headless(temp.path(), 900.0, 620.0);
    selected.render();
    selected.focus(picker_control_id("selected"));
    selected.press_key(egui::Key::Enter);
    assert!(!selected.model.is_picker());
    assert_eq!(
        selected.model.launch().explorer_root(),
        Some(alpha.as_path())
    );

    let mut current = picker_headless(temp.path(), 900.0, 620.0);
    current.render();
    current.focus(picker_control_id("current"));
    current.press_key(egui::Key::Enter);
    assert!(!current.model.is_picker());
    assert_eq!(
        current.model.launch().explorer_root(),
        Some(temp.path().canonicalize().unwrap().as_path())
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
    let text = output_text(&picker.render());
    assert!(text.contains("Use current folder"));
    assert!(text.contains("Open selected folder"));
    assert_eq!(
        picker_current_target(&picker.model),
        Some(temp.path().canonicalize().unwrap().display().to_string())
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
