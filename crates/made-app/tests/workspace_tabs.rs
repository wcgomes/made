//! Real-process ownership and hidden-runtime regression tests.
use std::time::{Duration, Instant};

use eframe::egui;
use made_app::app::{AppModel, AppOptions};
use made_app::workbench::LeftSurface;
use made_app::workspace::{LaunchPlan, WorkspaceLaunch, WorkspaceSource};
use made_app::TerminalTarget;
use serde_json::{json, Value};
use std::net::TcpStream;
use std::path::PathBuf;
use tungstenite::{Message, WebSocket};

fn isolated(name: &str) -> bool {
    isolated_with_opencode_launches(name, 0)
}

/// Run `name` in an isolated child process with a fake `opencode` first on
/// `PATH`, then assert the fake ran exactly `expected_opencode_launches` times.
fn isolated_with_opencode_launches(name: &str, expected_opencode_launches: usize) -> bool {
    if std::env::var_os("MADE_WORKSPACE_TEST_CHILD").is_some() {
        return false;
    }
    let home = tempfile::tempdir().unwrap();
    let sentinel = home.path().join("opencode-launch");
    let config_capture = home.path().join("opencode-config");
    let cwd_capture = home.path().join("opencode-cwd");
    let bin = home.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
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
        .env("MADE_WORKSPACE_TEST_CHILD", "1")
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
    if made_platform::PlatformFamily::current() != made_platform::PlatformFamily::Windows {
        let executable = bin.join("opencode");
        std::fs::write(
            &executable,
            "#!/bin/sh\nprintf 'invoked\\n' >> \"$MADE_OPENCODE_SENTINEL\"\nprintf '%s' \"$OPENCODE_CLI_CONFIG_CONTENT\" > \"$MADE_OPENCODE_CONFIG_CAPTURE\"\npwd > \"$MADE_OPENCODE_CWD_CAPTURE\"\nprintf '1.9.0\\n'\n",
        )
        .unwrap();
        assert!(std::process::Command::new("chmod")
            .arg("+x")
            .arg(&executable)
            .status()
            .unwrap()
            .success());
        let path = std::env::var_os("PATH").unwrap_or_default();
        let paths = std::iter::once(bin).chain(std::env::split_paths(&path));
        command.env("PATH", std::env::join_paths(paths).unwrap());
        command.env("MADE_OPENCODE_SENTINEL", &sentinel);
        command.env("MADE_OPENCODE_CONFIG_CAPTURE", &config_capture);
        command.env("MADE_OPENCODE_CWD_CAPTURE", &cwd_capture);
        command.env("SHELL", "/bin/bash");
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let launches = std::fs::read_to_string(&sentinel)
        .map(|content| content.lines().filter(|line| *line == "invoked").count())
        .unwrap_or(0);
    if expected_opencode_launches == 0 {
        assert_eq!(
            launches, 0,
            "normal workspace use must never invoke OpenCode, even for --version"
        );
    } else {
        assert_eq!(
            launches, expected_opencode_launches,
            "expected exactly {expected_opencode_launches} OpenCode launch(es)"
        );
    }
    true
}

#[cfg(unix)]
fn wait_for_capture(path: &std::path::Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match std::fs::read_to_string(path) {
            Ok(content) if !content.is_empty() => return content,
            _ if Instant::now() >= deadline => {
                return std::fs::read_to_string(path).unwrap_or_default();
            }
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

fn type_command(workbench: &mut made_app::Workbench, command: &str) {
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

fn isolated_opencode_config_path() -> PathBuf {
    use made_platform::Platform;
    made_platform::host_platform()
        .app_paths()
        .expect("isolated app paths")
        .opencode_config_path
}

fn credentials(workbench: &made_app::Workbench) -> Value {
    let content = workbench
        .terminal()
        .unwrap()
        .options()
        .env
        .iter()
        .find(|(key, _)| key == made_bridge::config::CONFIG_CONTENT_ENV)
        .unwrap()
        .1
        .to_str()
        .unwrap();
    let config: Value = serde_json::from_str(content).unwrap();
    config["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|entry| entry.get("options")?.get("made").cloned())
        .unwrap()
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
    let token = options["token"].as_str().unwrap();
    let pane_id = options["paneId"].as_str().unwrap();
    let mut socket = try_connect(options, token, pane_id);
    assert_eq!(read(&mut socket)["type"], "welcome");
    socket
}

fn try_connect(options: &Value, token: &str, pane_id: &str) -> WebSocket<TcpStream> {
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
        json!({"type":"hello", "protocolVersion":1, "token":token, "paneId":pane_id, "pluginVersion":"test", "opencodeVersion":"2.1.4", "pid":123, "cwd":"/"}),
    );
    socket
}

fn state(socket: &mut WebSocket<TcpStream>, revision: u64, directory: &std::path::Path) {
    send(
        socket,
        json!({"type":"state", "revision":revision, "activeSessionID":"shared-id", "tabsEnabled":true,
        "sessions":[{"sessionID":"shared-id", "title":"Owned session", "active":true, "busy":false, "attention":false, "unread":false,
        "promptPulse":0, "renaming":false, "directory":directory, "subpath":null, "projectID":null}]}),
    );
}

fn state_with_view(
    socket: &mut WebSocket<TcpStream>,
    revision: u64,
    view: Option<&std::path::Path>,
) {
    send(
        socket,
        json!({"type":"state", "revision":revision, "activeSessionID":null, "tabsEnabled":true,
        "sessions":[],
        "viewDirectory": view.map(|path| path.to_string_lossy().into_owned())}),
    );
}

fn pump_until(model: &mut AppModel, ctx: &egui::Context, predicate: impl Fn(&AppModel) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate(model) && Instant::now() < deadline {
        model.logic(ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(predicate(model), "runtime did not reach expected state");
}

#[test]
fn real_bridges_keep_state_focus_epochs_and_late_deliveries_tab_local() {
    if isolated("real_bridges_keep_state_focus_epochs_and_late_deliveries_tab_local") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut options = AppOptions::new();
    options.started_at = Instant::now() - Duration::from_secs(30);
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), options);
    let ctx = egui::Context::default();
    for _ in 0..3 {
        open(&mut model, &root);
    }
    let ids: Vec<_> = model.workspace_ids().collect();
    let configs: Vec<_> = ids
        .iter()
        .map(|id| credentials(model.workspace(*id).unwrap()))
        .collect();
    assert_ne!(configs[0]["token"], configs[1]["token"]);
    assert_ne!(configs[0]["bridgeUrl"], configs[1]["bridgeUrl"]);
    let mut sockets: Vec<_> = configs.iter().map(connect).collect();
    let dirs: Vec<_> = ["agent-a", "agent-b", "agent-c"]
        .iter()
        .map(|name| {
            let path = root.join(name);
            std::fs::create_dir(&path).unwrap();
            path
        })
        .collect();
    for (socket, dir) in sockets.iter_mut().zip(&dirs) {
        state(socket, 99, dir);
    }
    pump_until(&mut model, &ctx, |model| {
        ids.iter()
            .zip(&dirs)
            .all(|(id, dir)| model.workspace(*id).unwrap().explorer_root() == Some(dir.as_path()))
    });
    for id in &ids {
        let owner = model.workspace(*id).unwrap();
        assert_eq!(
            owner.integration().report().connection_id,
            Some(1),
            "connection numbers are bridge-local"
        );
        assert_eq!(owner.session_map().active_session_id(), Some("shared-id"));
    }
    model.workspace_mut(ids[0]).unwrap().focus_session("next-a");
    model.workspace_mut(ids[1]).unwrap().focus_session("next-b");
    let a = read(&mut sockets[0]);
    let b = read(&mut sockets[1]);
    assert_eq!(a["type"], "focus");
    assert_eq!(b["type"], "focus");
    assert_eq!(
        a["requestID"], b["requestID"],
        "request ids are intentionally local, not global"
    );
    send(
        &mut sockets[0],
        json!({"type":"focus_result", "requestID":a["requestID"], "ok":true}),
    );
    pump_until(&mut model, &ctx, |model| {
        !model
            .workspace(ids[0])
            .unwrap()
            .focus_controller()
            .is_in_flight()
    });
    assert!(model
        .workspace(ids[1])
        .unwrap()
        .focus_controller()
        .is_in_flight());
    let mut replacement = connect(&configs[2]);
    state(&mut replacement, 1, &dirs[0]);
    pump_until(&mut model, &ctx, |model| {
        model.workspace(ids[2]).unwrap().explorer_root() == Some(dirs[0].as_path())
    });
    assert_eq!(
        model
            .workspace(ids[2])
            .unwrap()
            .integration()
            .last_revision(),
        Some(1)
    );
    assert_eq!(
        model.workspace(ids[1]).unwrap().explorer_root(),
        Some(dirs[1].as_path())
    );
    let _ = sockets[2].send(Message::Text(json!({"type":"state", "revision":1000, "activeSessionID":null, "tabsEnabled":true, "sessions":[]}).to_string().into()));
    model.logic(&ctx);
    assert_eq!(
        model.workspace(ids[2]).unwrap().explorer_root(),
        Some(dirs[0].as_path())
    );
    assert!(model.close_workspace(ids[0]));
    let _ = sockets[0].send(Message::Text(
        json!({"type":"focus_result", "requestID":a["requestID"], "ok":false, "error":"late"})
            .to_string()
            .into(),
    ));
    open(&mut model, &root);
    assert!(model.active_workspace_id().unwrap() > ids[2]);
    model.logic(&ctx);
    assert!(model
        .workspace(ids[1])
        .unwrap()
        .focus_controller()
        .is_in_flight());
    if made_platform::PlatformFamily::current() != made_platform::PlatformFamily::Windows {
        type_command(
            model.workspace_mut(ids[1]).unwrap(),
            &format!("cd '{}'", dirs[2].display()),
        );
        pump_until(&mut model, &ctx, |model| {
            model.workspace(ids[1]).unwrap().shell_cwd() == dirs[2]
        });
        assert_eq!(
            model.workspace(ids[1]).unwrap().explorer_root(),
            Some(dirs[1].as_path()),
            "connected agent keeps precedence over real shell cd"
        );
    }
    send(
        &mut sockets[1],
        json!({"type":"goodbye", "reason":"normal exit"}),
    );
    let fallback =
        if made_platform::PlatformFamily::current() != made_platform::PlatformFamily::Windows {
            dirs[2].as_path()
        } else {
            root.as_path()
        };
    pump_until(&mut model, &ctx, |model| {
        model.workspace(ids[1]).unwrap().explorer_root() == Some(fallback)
    });
    assert_eq!(
        model.workspace(ids[2]).unwrap().explorer_root(),
        Some(dirs[0].as_path())
    );
    assert!(model
        .workspace(ids[2])
        .unwrap()
        .terminal()
        .unwrap()
        .is_running());
}

fn open(model: &mut AppModel, path: &std::path::Path) {
    model.open_plan(LaunchPlan::open(path, WorkspaceSource::Picker).unwrap());
    model.ensure_workbench();
}

#[test]
fn view_directory_follow_switches_root_invalidates_preview_and_reports_source() {
    if isolated("view_directory_follow_switches_root_invalidates_preview_and_reports_source") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    let view = root.join("view");
    let missing = root.join("missing-view");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&view).unwrap();
    let preview_file = workspace.join("preview.txt");
    std::fs::write(&preview_file, "previewed content\n").unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let ctx = egui::Context::default();
    open(&mut model, &workspace);

    let config = credentials(model.workbench().unwrap());
    let mut socket = connect(&config);
    state(&mut socket, 1, &workspace);
    pump_until(&mut model, &ctx, |model| {
        model.workbench().unwrap().root_source() == "opencode"
    });
    assert_eq!(
        model.workbench().unwrap().explorer_root(),
        Some(workspace.as_path())
    );

    model
        .workbench_mut()
        .unwrap()
        .explorer_mut()
        .unwrap()
        .open_preview(preview_file);
    pump_until(&mut model, &ctx, |model| {
        model
            .workbench()
            .unwrap()
            .explorer()
            .unwrap()
            .preview_document()
            .is_some()
    });

    // A connected view directory replaces the shell root once the session is
    // gone and invalidates preview work for the previous root.
    state_with_view(&mut socket, 2, Some(&view));
    pump_until(&mut model, &ctx, |model| {
        model.workbench().unwrap().explorer_root() == Some(view.as_path())
    });
    {
        let workbench = model.workbench().unwrap();
        assert_eq!(workbench.root_source(), "opencode");
        assert!(
            workbench.explorer().unwrap().preview_document().is_none(),
            "the preview for the previous root is invalidated"
        );
        let diagnostics = workbench.diagnostics_snapshot();
        assert_eq!(diagnostics.status, made_app::IntegrationStatus::Connected);
        assert_eq!(diagnostics.status_label, "connected");
        assert_eq!(diagnostics.active_session_id, None);
        assert_eq!(diagnostics.active_session_title, None);
        assert_eq!(diagnostics.active_root.as_deref(), Some(view.as_path()));
        assert_eq!(diagnostics.root_source.as_deref(), Some("opencode"));
        assert_eq!(diagnostics.view_directory.as_deref(), Some(view.as_path()));
        assert_eq!(diagnostics.view_directory_available, Some(true));
        assert_eq!(diagnostics.view_directory_reason, None);
        assert!(diagnostics.tabs_enabled);
        assert_eq!(diagnostics.plugin_version.as_deref(), Some("test"));
        assert_eq!(diagnostics.opencode_version.as_deref(), Some("2.1.4"));
        assert_eq!(diagnostics.protocol_version, Some(1));
        let text = diagnostics.to_text();
        for expected in [
            "status: connected",
            "active session: none",
            "root source: opencode",
            "view directory available: yes",
            "plugin version: test",
            "opencode version: 2.1.4",
            "protocol version: 1",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in {text}");
        }
    }

    // An unavailable view directory falls back to the shell root without
    // presenting an unavailable-root tree, and keeps the reason.
    let shell_fallback = model.workbench().unwrap().shell_cwd().to_path_buf();
    state_with_view(&mut socket, 3, Some(&missing));
    pump_until(&mut model, &ctx, |model| {
        model.workbench().unwrap().explorer_root() == Some(shell_fallback.as_path())
    });
    {
        let workbench = model.workbench().unwrap();
        assert_eq!(workbench.root_source(), "shell");
        assert_eq!(workbench.explorer().unwrap().root_unavailable(), None);
        let diagnostics = workbench.workspace_diagnostics_context();
        assert_eq!(
            diagnostics.view_directory.as_deref(),
            Some(missing.as_path())
        );
        assert_eq!(diagnostics.view_directory_available, Some(false));
        assert!(diagnostics
            .view_directory_reason
            .as_deref()
            .is_some_and(|reason| !reason.is_empty()));
    }

    // Recovery switches to the view root again.
    state_with_view(&mut socket, 4, Some(&view));
    pump_until(&mut model, &ctx, |model| {
        model.workbench().unwrap().explorer_root() == Some(view.as_path())
    });

    send(
        &mut socket,
        json!({"type":"goodbye", "reason":"normal exit"}),
    );
    pump_until(&mut model, &ctx, |model| {
        model.workbench().unwrap().explorer_root() == Some(shell_fallback.as_path())
    });
    let workbench = model.workbench().unwrap();
    assert_eq!(workbench.root_source(), "shell");
    assert_eq!(
        workbench.workspace_diagnostics_context().view_directory,
        None
    );
}

#[test]
fn view_directories_stay_isolated_across_live_workspaces() {
    if isolated("view_directories_stay_isolated_across_live_workspaces") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let first = root.join("first");
    let second = root.join("second");
    let first_view = root.join("first-view");
    let second_view = root.join("second-view");
    for path in [&first, &second, &first_view, &second_view] {
        std::fs::create_dir(path).unwrap();
    }
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let ctx = egui::Context::default();
    open(&mut model, &first);
    let first_id = model.active_workspace_id().unwrap();
    open(&mut model, &second);
    let second_id = model.active_workspace_id().unwrap();
    assert_ne!(first_id, second_id);

    let first_config = credentials(model.workspace(first_id).unwrap());
    let second_config = credentials(model.workspace(second_id).unwrap());
    let mut first_socket = connect(&first_config);
    let mut second_socket = connect(&second_config);
    state_with_view(&mut first_socket, 1, Some(&first_view));
    state_with_view(&mut second_socket, 1, Some(&second_view));
    pump_until(&mut model, &ctx, |model| {
        model.workspace(first_id).unwrap().explorer_root() == Some(first_view.as_path())
            && model.workspace(second_id).unwrap().explorer_root() == Some(second_view.as_path())
    });

    for (id, view) in [
        (first_id, first_view.as_path()),
        (second_id, second_view.as_path()),
    ] {
        let workbench = model.workspace(id).unwrap();
        assert_eq!(workbench.root_source(), "opencode");
        let diagnostics = workbench.diagnostics_snapshot();
        assert_eq!(diagnostics.status_label, "connected");
        assert_eq!(diagnostics.active_session_id, None);
        assert_eq!(diagnostics.active_root.as_deref(), Some(view));
        assert_eq!(diagnostics.root_source.as_deref(), Some("opencode"));
        assert_eq!(diagnostics.view_directory.as_deref(), Some(view));
        assert_eq!(diagnostics.view_directory_available, Some(true));
    }

    // Closing one workbench must not clear or change the other's view state.
    assert!(model.close_workspace(first_id));
    assert!(model.workspace(first_id).is_none());
    {
        let workbench = model.workspace(second_id).unwrap();
        assert_eq!(
            workbench.explorer_root(),
            Some(second_view.as_path()),
            "the surviving workspace keeps its own view root"
        );
        assert_eq!(workbench.root_source(), "opencode");
        let diagnostics = workbench.diagnostics_snapshot();
        assert_eq!(
            diagnostics.view_directory.as_deref(),
            Some(second_view.as_path())
        );
        assert_eq!(diagnostics.view_directory_available, Some(true));
        assert_eq!(diagnostics.root_source.as_deref(), Some("opencode"));
    }

    // The surviving bridge connection still accepts its own view state.
    state_with_view(&mut second_socket, 2, Some(&second_view));
    pump_until(&mut model, &ctx, |model| {
        model.workspace(second_id).unwrap().explorer_root() == Some(second_view.as_path())
    });
    assert_eq!(
        model
            .workspace(second_id)
            .unwrap()
            .diagnostics_snapshot()
            .view_directory
            .as_deref(),
        Some(second_view.as_path())
    );
}

#[test]
fn three_live_tabs_keep_independent_resources_and_close_only_the_owner() {
    if isolated("three_live_tabs_keep_independent_resources_and_close_only_the_owner") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut options = AppOptions::new();
    options.started_at = Instant::now() - Duration::from_secs(30);
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), options);
    let ctx = egui::Context::default();
    let mut ids = Vec::new();
    let mut pids = Vec::new();
    let mut panes = Vec::new();
    let mut bootstraps = Vec::new();
    let mut configs = Vec::new();
    for name in ["one", "two", "three"] {
        let path = root.join(name);
        std::fs::create_dir(&path).unwrap();
        open(&mut model, &path);
        ids.push(model.active_workspace_id().unwrap());
        let workbench = model.workbench().unwrap();
        pids.push(workbench.terminal().unwrap().pid().unwrap());
        panes.push(workbench.pane_id().to_owned());
        bootstraps.push(
            workbench
                .terminal()
                .unwrap()
                .bootstrap_directory()
                .map(std::path::Path::to_path_buf),
        );
        configs.push(workbench.terminal().unwrap().options().env.clone());
        assert_eq!(workbench.explorer_root(), Some(path.as_path()));
        assert_eq!(workbench.left_surface(), LeftSurface::Terminal);
        assert!(!workbench.sessions_visible());
        assert_eq!(
            workbench.integration().status(),
            made_app::IntegrationStatus::Inactive
        );
        let diagnostics = workbench.diagnostics_snapshot();
        assert_eq!(diagnostics.status_label, "inactive");
        assert_eq!(
            diagnostics.workspace_identity.as_deref(),
            Some(workbench.pane_id()),
            "diagnostics identify the selected workspace, not a global process"
        );
        assert_eq!(
            diagnostics.active_root.as_deref(),
            Some(path.as_path()),
            "diagnostics expose the effective root"
        );
        assert_eq!(diagnostics.root_source.as_deref(), Some("shell"));
        assert!(
            diagnostics.cwd_supported.is_some(),
            "diagnostics expose shell cwd capability"
        );
        assert!(
            diagnostics
                .cwd_reason
                .as_deref()
                .is_some_and(|reason| !reason.is_empty()),
            "diagnostics expose the cwd capability or fallback reason"
        );
    }
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(pids.windows(2).all(|pair| pair[0] != pair[1]));
    assert!(panes.windows(2).all(|pair| pair[0] != pair[1]));
    assert!(
        configs.windows(2).all(|pair| pair[0] != pair[1]),
        "each launch must inject distinct bridge credentials/endpoints"
    );
    assert!(model.switch_workspace(ids[0]));
    model.workbench_mut().unwrap().set_explorer_width(333.0);
    model.workbench_mut().unwrap().set_explorer_visible(false);
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        model.logic(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    for id in &ids {
        let workbench = model.workspace(*id).unwrap();
        assert!(workbench.terminal().unwrap().is_running());
        assert!(workbench.tick_count() > 1);
        assert_eq!(workbench.paint_count(), 0);
        assert_eq!(
            workbench.integration().status(),
            made_app::IntegrationStatus::Inactive,
            "no elapsed-time warning may activate absent optional integration"
        );
    }
    assert!(model.close_workspace(ids[1]));
    if let Some(path) = &bootstraps[1] {
        assert!(!path.exists());
    }
    assert!(model.workspace(ids[1]).is_none());
    assert!(!model.switch_workspace(ids[1]));
    assert_eq!(model.workbench().unwrap().explorer_width(), 333.0);
    assert!(!model.workbench().unwrap().explorer_visible());
    assert!(model
        .workspace(ids[2])
        .unwrap()
        .terminal()
        .unwrap()
        .is_running());
    open(&mut model, &root);
    let newest = model.active_workspace_id().unwrap();
    assert!(newest > ids[2]);
    model.begin_new_workspace();
    assert!(model.is_picker());
    assert!(model.cancel_picker());
    assert_eq!(model.active_workspace_id(), Some(newest));
    for id in [ids[0], ids[2], newest] {
        assert!(model.close_workspace(id));
    }
    assert!(model.is_picker());
    assert!(model.workbench().is_none());
    assert_eq!(model.workspace_count(), 0);
    for path in bootstraps.into_iter().flatten() {
        assert!(!path.exists());
    }
}

#[test]
fn isolated_real_bash_cd_follows_hidden_tabs_without_painting() {
    if made_platform::PlatformFamily::current() == made_platform::PlatformFamily::Windows {
        return;
    }
    if isolated("isolated_real_bash_cd_follows_hidden_tabs_without_painting") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let target = root.join("space café");
    std::fs::create_dir(&target).unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    for _ in 0..3 {
        open(&mut model, &root);
    }
    let ids: Vec<_> = model.workspace_ids().collect();
    for id in &ids {
        type_command(
            model.workspace_mut(*id).unwrap(),
            "for i in {1..2000}; do printf 'hidden output\\n'; done",
        );
    }
    type_command(
        model.workspace_mut(ids[0]).unwrap(),
        &format!("cd '{}'", target.display()),
    );
    let ctx = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        model.logic(&ctx);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 620.0),
                )),
                ..Default::default()
            },
            |ui| model.ui(ui),
        );
        output.textures_delta.clear();
        if model.workspace(ids[0]).unwrap().explorer_root() == Some(target.as_path())
            && ids.iter().all(|id| {
                model
                    .workspace(*id)
                    .unwrap()
                    .terminal()
                    .unwrap()
                    .output_bytes()
                    > 2000
            })
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        model.workspace(ids[0]).unwrap().explorer_root(),
        Some(target.as_path())
    );
    for id in &ids[..2] {
        let terminal = model.workspace(*id).unwrap().terminal().unwrap();
        assert!(terminal.output_bytes() > 2000);
        assert_eq!(terminal.grid_paint_count(), 0);
        assert!(!terminal.is_focused());
    }
    assert!(
        model
            .workbench()
            .unwrap()
            .terminal()
            .unwrap()
            .grid_paint_count()
            > 0
    );
    assert_eq!(
        model.workspace(ids[1]).unwrap().explorer_root(),
        Some(root.as_path())
    );
    let old_pid = model.workspace(ids[0]).unwrap().terminal().unwrap().pid();
    model.switch_workspace(ids[0]);
    model.logic(&ctx);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| model.ui(ui));
    output.textures_delta.clear();
    assert_eq!(
        model.workbench().unwrap().terminal().unwrap().pid(),
        old_pid
    );
    model
        .workbench_mut()
        .unwrap()
        .set_left_surface(LeftSurface::Preview);
    let paints = model
        .workbench()
        .unwrap()
        .terminal()
        .unwrap()
        .grid_paint_count();
    model.logic(&ctx);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| model.ui(ui));
    output.textures_delta.clear();
    assert_eq!(
        model
            .workbench()
            .unwrap()
            .terminal()
            .unwrap()
            .grid_paint_count(),
        paints
    );
    assert!(!model.workbench().unwrap().terminal().unwrap().is_focused());
    model.workbench_mut().unwrap().close_preview();
    model.close_workspace(ids[2]);
    let marker = root.join("sibling-alive");
    type_command(
        model.workbench_mut().unwrap(),
        &format!("printf alive > '{}'", marker.display()),
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while !marker.exists() && Instant::now() < deadline {
        model.logic(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "alive");
}

#[test]
fn rejected_bridge_clients_cannot_reset_a_healthy_workspace() {
    if isolated("rejected_bridge_clients_cannot_reset_a_healthy_workspace") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let agent = root.join("agent-root");
    std::fs::create_dir(&agent).unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let ctx = egui::Context::default();
    open(&mut model, &workspace);

    let config = credentials(model.workbench().unwrap());
    let mut healthy = connect(&config);
    state(&mut healthy, 1, &agent);
    pump_until(&mut model, &ctx, |model| {
        model.workbench().unwrap().explorer_root() == Some(agent.as_path())
    });
    model.workbench_mut().unwrap().focus_session("next-session");
    assert!(model.workbench().unwrap().focus_controller().is_in_flight());
    let focus = read(&mut healthy);
    assert_eq!(focus["type"], "focus");

    let wrong_token = "0".repeat(64);
    let pane = config["paneId"].as_str().unwrap();
    for _ in 0..4 {
        let mut rejected = try_connect(&config, &wrong_token, pane);
        let _ = rejected.close(None);
    }

    // Drain the rejection events (and let the bridge settle) before checking.
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        model.logic(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }

    let owner = model.workbench().unwrap();
    assert_eq!(
        owner.integration().status(),
        made_app::IntegrationStatus::Connected,
        "rejected clients must not degrade a healthy authenticated workspace"
    );
    assert_eq!(owner.integration().reason().code.code(), "connected");
    assert_eq!(owner.explorer_root(), Some(agent.as_path()));
    assert_eq!(owner.session_map().active_session_id(), Some("shared-id"));
    assert!(
        owner.focus_controller().is_in_flight(),
        "a foreign rejection must not cancel the current workspace's focus request"
    );

    send(
        &mut healthy,
        json!({"type":"focus_result", "requestID":focus["requestID"], "ok":true}),
    );
    pump_until(&mut model, &ctx, |model| {
        !model.workbench().unwrap().focus_controller().is_in_flight()
    });
    assert_eq!(
        model.workbench().unwrap().integration().status(),
        made_app::IntegrationStatus::Connected
    );
}

#[test]
fn unavailable_authoritative_root_invalidates_preview_and_recovers() {
    if isolated("unavailable_authoritative_root_invalidates_preview_and_recovers") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let preview_file = workspace.join("preview.txt");
    std::fs::write(&preview_file, "previewed content\n").unwrap();
    let missing = root.join("removed-agent-root");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let ctx = egui::Context::default();
    open(&mut model, &workspace);

    let config = credentials(model.workbench().unwrap());
    let mut healthy = connect(&config);
    state(&mut healthy, 1, &workspace);
    pump_until(&mut model, &ctx, |model| {
        let workbench = model.workbench().unwrap();
        workbench
            .explorer()
            .is_some_and(|explorer| explorer.is_root_ready())
    });
    let generation = model.workbench().unwrap().explorer().unwrap().generation();
    model
        .workbench_mut()
        .unwrap()
        .explorer_mut()
        .unwrap()
        .open_preview(preview_file);
    pump_until(&mut model, &ctx, |model| {
        model
            .workbench()
            .unwrap()
            .explorer()
            .unwrap()
            .preview_document()
            .is_some()
    });

    state(&mut healthy, 2, &missing);
    pump_until(&mut model, &ctx, |model| {
        model
            .workbench()
            .unwrap()
            .explorer()
            .unwrap()
            .root_unavailable()
            .is_some()
    });
    {
        let workbench = model.workbench().unwrap();
        let explorer = workbench.explorer().unwrap();
        assert_eq!(
            workbench.explorer_root(),
            Some(workspace.as_path()),
            "the last usable root stays displayed"
        );
        assert!(
            explorer.generation() > generation,
            "the unavailable effective root advances the generation"
        );
        assert!(
            explorer.preview_document().is_none(),
            "the preview and pending reads for the previous root are invalidated"
        );
        assert_eq!(
            workbench.left_surface(),
            LeftSurface::Terminal,
            "invalidating the root returns to the terminal"
        );
    }

    // The same unavailable identity must not advance the generation again.
    let unavailable_generation = model.workbench().unwrap().explorer().unwrap().generation();
    state(&mut healthy, 3, &missing);
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        model.logic(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        model.workbench().unwrap().explorer().unwrap().generation(),
        unavailable_generation
    );

    // A -> unavailable B -> disconnect -> A clears the banner even though the
    // pathname stayed the same.
    send(
        &mut healthy,
        json!({"type":"goodbye", "reason":"normal exit"}),
    );
    pump_until(&mut model, &ctx, |model| {
        model
            .workbench()
            .unwrap()
            .explorer()
            .unwrap()
            .root_unavailable()
            .is_none()
    });
    assert_eq!(
        model.workbench().unwrap().explorer_root(),
        Some(workspace.as_path())
    );
    assert_eq!(
        model.workbench().unwrap().explorer().unwrap().generation(),
        unavailable_generation,
        "falling back to the same path clears the marker without a new root identity"
    );

    // Recovery to the now-available authoritative directory switches normally.
    std::fs::create_dir(&missing).unwrap();
    let mut recovered = connect(&config);
    state(&mut recovered, 1, &missing);
    pump_until(&mut model, &ctx, |model| {
        model.workbench().unwrap().explorer_root() == Some(missing.as_path())
    });
    let workbench = model.workbench().unwrap();
    assert_eq!(workbench.explorer().unwrap().root_unavailable(), None);
    assert!(workbench.explorer().unwrap().generation() > unavailable_generation);
}

#[test]
fn deleting_the_last_usable_agent_root_clears_an_open_preview() {
    if isolated("deleting_the_last_usable_agent_root_clears_an_open_preview") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let agent = root.join("last-usable-agent-root");
    std::fs::create_dir(&agent).unwrap();
    let preview_file = agent.join("preview.txt");
    std::fs::write(&preview_file, "agent preview\n").unwrap();
    let missing = root.join("removed-agent-root");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let ctx = egui::Context::default();
    open(&mut model, &workspace);

    let config = credentials(model.workbench().unwrap());
    let mut healthy = connect(&config);
    state(&mut healthy, 1, &agent);
    pump_until(&mut model, &ctx, |model| {
        let workbench = model.workbench().unwrap();
        workbench.explorer_root() == Some(agent.as_path())
            && workbench
                .explorer()
                .is_some_and(|explorer| explorer.is_root_ready())
    });
    model
        .workbench_mut()
        .unwrap()
        .explorer_mut()
        .unwrap()
        .open_preview(preview_file);
    let preview_deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < preview_deadline {
        model.logic(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| model.ui(ui));
        output.textures_delta.clear();
        model.logic(&ctx);
        if model.workbench().unwrap().left_surface() == LeftSurface::Preview
            && model
                .workbench()
                .unwrap()
                .explorer()
                .unwrap()
                .preview_document()
                .is_some()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        model.workbench().unwrap().left_surface(),
        LeftSurface::Preview
    );

    // The loaded preview's root is the last usable tree. Remove it before the
    // active session reports a different unavailable directory.
    #[cfg(windows)]
    {
        // Windows can refuse recursive deletion while a handle under the
        // watched root is still open, even though notify opens the directory
        // watch with delete sharing. Renaming removes the root from its watched
        // path without losing the unavailable-root scenario; the tempdir owns
        // cleanup.
        std::fs::rename(&agent, root.join("deleted-agent-root")).unwrap();
    }
    #[cfg(not(windows))]
    std::fs::remove_dir_all(&agent).unwrap();
    state(&mut healthy, 2, &missing);
    pump_until(&mut model, &ctx, |model| {
        model
            .workbench()
            .unwrap()
            .explorer()
            .unwrap()
            .root_unavailable()
            .is_some()
    });
    {
        let workbench = model.workbench().unwrap();
        let explorer = workbench.explorer().unwrap();
        assert_eq!(
            workbench.explorer_root(),
            Some(agent.as_path()),
            "the deleted last usable root remains the displayed identity"
        );
        assert!(
            explorer.preview_document().is_none(),
            "keep-cached-tree unavailability must clear the preview document"
        );
        assert!(
            explorer.preview_path().is_none(),
            "pending preview reads for the deleted tree are invalidated"
        );
        assert_eq!(workbench.left_surface(), LeftSurface::Terminal);
        assert!(workbench.terminal().unwrap().is_running());
    }

    // Root recovery switches normally and never restores the stale preview.
    std::fs::create_dir(&missing).unwrap();
    state(&mut healthy, 3, &missing);
    pump_until(&mut model, &ctx, |model| {
        model.workbench().unwrap().explorer_root() == Some(missing.as_path())
    });
    let workbench = model.workbench().unwrap();
    assert!(workbench.explorer().unwrap().root_unavailable().is_none());
    assert!(workbench.explorer().unwrap().preview_document().is_none());
    assert_eq!(workbench.left_surface(), LeftSurface::Terminal);
}

#[test]
fn releasing_an_agent_root_with_no_usable_fallback_clears_the_preview() {
    if made_platform::PlatformFamily::current() == made_platform::PlatformFamily::Windows {
        return;
    }
    if isolated("releasing_an_agent_root_with_no_usable_fallback_clears_the_preview") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let agent = root.join("agent-root");
    std::fs::create_dir(&agent).unwrap();
    let preview_file = agent.join("preview.txt");
    std::fs::write(&preview_file, "preview\n").unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let ctx = egui::Context::default();
    open(&mut model, &workspace);
    let id = model.active_workspace_id().unwrap();
    model
        .workspace_mut(id)
        .unwrap()
        .set_shell_root_revalidation_interval(Duration::from_millis(25));

    let config = credentials(model.workbench().unwrap());
    let mut healthy = connect(&config);
    state(&mut healthy, 1, &agent);
    pump_until(&mut model, &ctx, |model| {
        model.workbench().unwrap().explorer_root() == Some(agent.as_path())
    });
    model
        .workbench_mut()
        .unwrap()
        .explorer_mut()
        .unwrap()
        .open_preview(preview_file);
    let preview_deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < preview_deadline {
        model.logic(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| model.ui(ui));
        output.textures_delta.clear();
        model.logic(&ctx);
        if model.workbench().unwrap().left_surface() == LeftSurface::Preview
            && model
                .workbench()
                .unwrap()
                .explorer()
                .unwrap()
                .preview_document()
                .is_some()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        model.workbench().unwrap().left_surface(),
        LeftSurface::Preview
    );

    // Remove both the agent root and the fallback workspace before release.
    std::fs::remove_dir_all(&agent).unwrap();
    std::fs::remove_dir_all(&workspace).unwrap();
    send(
        &mut healthy,
        json!({"type":"goodbye", "reason":"normal exit"}),
    );
    pump_until(&mut model, &ctx, |model| {
        model
            .workbench()
            .unwrap()
            .explorer()
            .unwrap()
            .root_unavailable()
            .is_some()
    });
    let displayed_root = {
        let workbench = model.workbench().unwrap();
        assert_eq!(workbench.left_surface(), LeftSurface::Terminal);
        assert!(workbench.explorer().unwrap().preview_document().is_none());
        assert!(workbench.explorer().unwrap().preview_path().is_none());
        assert!(workbench.terminal().unwrap().is_running());
        workbench.explorer_root().unwrap().to_path_buf()
    };

    // Recovering the displayed root clears the marker without reviving the
    // preview or the hidden terminal input path.
    std::fs::create_dir(&displayed_root).unwrap();
    pump_until(&mut model, &ctx, |model| {
        model
            .workbench()
            .unwrap()
            .explorer()
            .unwrap()
            .root_unavailable()
            .is_none()
    });
    let workbench = model.workbench().unwrap();
    assert_eq!(workbench.left_surface(), LeftSurface::Terminal);
    assert!(workbench.explorer().unwrap().preview_document().is_none());
}

#[test]
fn bad_shell_cwd_does_not_mark_an_authoritative_agent_root_unavailable() {
    if made_platform::PlatformFamily::current() == made_platform::PlatformFamily::Windows {
        return;
    }
    if isolated("bad_shell_cwd_does_not_mark_an_authoritative_agent_root_unavailable") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let agent = root.join("agent-root");
    std::fs::create_dir(&agent).unwrap();
    let bad_shell = root.join("missing-shell-cwd");
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let ctx = egui::Context::default();
    open(&mut model, &workspace);

    let config = credentials(model.workbench().unwrap());
    let mut healthy = connect(&config);
    state(&mut healthy, 1, &agent);
    pump_until(&mut model, &ctx, |model| {
        model.workbench().unwrap().explorer_root() == Some(agent.as_path())
    });
    type_command(
        model.workbench_mut().unwrap(),
        &format!(
            "printf '\\033]7;file://localhost{}\\007'; exec sleep 2",
            bad_shell.display()
        ),
    );

    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && model.workbench().unwrap().shell_cwd_error().is_none() {
        model.logic(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    let workbench = model.workbench().unwrap();
    assert!(
        workbench.shell_cwd_error().is_some(),
        "the bad shell report should be tracked separately"
    );
    assert_eq!(workbench.explorer_root(), Some(agent.as_path()));
    assert_eq!(
        workbench.explorer().unwrap().root_unavailable(),
        None,
        "a bad shell cwd must not mark the authoritative agent root unavailable"
    );
    assert_eq!(
        workbench.integration().status(),
        made_app::IntegrationStatus::Connected
    );
}

#[test]
fn shutdown_diagnostics_preserve_workspace_capability_versions_and_metrics() {
    if isolated("shutdown_diagnostics_preserve_workspace_capability_versions_and_metrics") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("diagnostics-workspace");
    std::fs::create_dir(&workspace).unwrap();
    let plan = LaunchPlan::open(&workspace, WorkspaceSource::Picker).unwrap();
    let cwd = plan.terminal_cwd().to_path_buf();
    let mut workbench =
        made_app::Workbench::start(Some(plan), None, cwd, TerminalTarget::Shell, Instant::now());

    let identity = workbench.pane_id().to_owned();
    let shell_cwd = workbench.shell_cwd().to_path_buf();
    let capability = workbench
        .shell_cwd_capability()
        .expect("the isolated bash launch exposes a cwd capability");
    let supported = capability.is_supported();
    let reason = capability.reason.clone();
    let token = credentials(&workbench)["token"]
        .as_str()
        .expect("bridge token")
        .to_owned();
    let log_path = workbench.diagnostics().log_location().to_path_buf();
    assert!(!log_path.as_os_str().is_empty(), "diagnostics log path");

    workbench.shutdown();

    let view = workbench.diagnostics_snapshot();
    assert_eq!(view.workspace_identity.as_deref(), Some(identity.as_str()));
    assert_eq!(view.shell_cwd.as_ref(), Some(&shell_cwd));
    assert_eq!(view.cwd_supported, Some(supported));
    assert_eq!(view.cwd_reason.as_deref(), Some(reason.as_str()));
    assert_eq!(view.root_source.as_deref(), Some("shell"));
    assert!(view.active_root.is_some());
    assert!(view.watcher.is_some());
    assert!(!view.app_version.is_empty());
    assert_eq!(
        workbench
            .shell_cwd_capability()
            .map(made_platform::ShellCwdCapability::is_supported),
        Some(supported),
        "the capability survives terminal shutdown for both diagnostics paths"
    );

    let text = std::fs::read_to_string(&log_path).unwrap();
    for expected in [
        format!("workspace identity: {identity}"),
        format!("shell cwd: {}", shell_cwd.display()),
        format!(
            "cwd reporting: {}",
            if supported {
                "supported"
            } else {
                "unsupported"
            }
        ),
        format!("cwd reason: {reason}"),
        "root source: shell".to_owned(),
        "effective root:".to_owned(),
        "watcher:".to_owned(),
        "status: inactive".to_owned(),
        "app version:".to_owned(),
        "frame_time:".to_owned(),
        "cwd_to_root:".to_owned(),
        "hidden_output_bytes=".to_owned(),
        "bridge_dropped_events=".to_owned(),
    ] {
        assert!(text.contains(&expected), "missing {expected:?} in {text}");
    }
    assert!(
        !text.contains(&token),
        "shutdown log leaked the bridge token"
    );
    let copied = workbench.sanitized_diagnostics_text();
    for expected in [
        format!("workspace identity: {identity}"),
        "cwd reporting:".to_owned(),
        "cwd reason:".to_owned(),
        "root source: shell".to_owned(),
        "effective root:".to_owned(),
        "performance:".to_owned(),
        "frame_time:".to_owned(),
        "counters: hidden_output_bytes=".to_owned(),
    ] {
        assert!(
            copied.contains(&expected),
            "copied diagnostics missing {expected:?} in {copied}"
        );
    }
    assert!(
        !copied.contains(token.as_str()),
        "copied diagnostics after shutdown leaked the bridge token"
    );
}

#[test]
fn deleted_in_place_shell_root_is_marked_unavailable_once_and_recovers() {
    if made_platform::PlatformFamily::current() == made_platform::PlatformFamily::Windows {
        return;
    }
    if isolated("deleted_in_place_shell_root_is_marked_unavailable_once_and_recovers") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    let target = workspace.join("deleted-in-place");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("kept.txt"), "tree row\n").unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let ctx = egui::Context::default();
    open(&mut model, &workspace);
    let id = model.active_workspace_id().unwrap();
    model
        .workspace_mut(id)
        .unwrap()
        .set_shell_root_revalidation_interval(Duration::from_millis(25));
    type_command(
        model.workspace_mut(id).unwrap(),
        &format!("cd '{}'", target.display()),
    );
    pump_until(&mut model, &ctx, |model| {
        model.workspace(id).unwrap().explorer_root() == Some(target.as_path())
    });
    pump_until(&mut model, &ctx, |model| {
        model
            .workspace(id)
            .unwrap()
            .explorer()
            .is_some_and(|explorer| explorer.is_root_ready())
    });
    assert!(
        model
            .workspace(id)
            .unwrap()
            .metrics()
            .cwd_to_root()
            .is_none(),
        "cwd-to-root finishes at the first explorer paint, not during logic"
    );
    model.logic(&ctx);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| model.ui(ui));
    output.textures_delta.clear();
    model.logic(&ctx);
    assert!(
        model
            .workspace(id)
            .unwrap()
            .metrics()
            .cwd_to_root()
            .is_some_and(|latency| latency > Duration::ZERO),
        "shell cwd report to the first painted explorer state is measured"
    );
    let rows_before = model
        .workspace_mut(id)
        .unwrap()
        .explorer_mut()
        .unwrap()
        .visible_row_count();
    assert!(rows_before > 0, "the target tree has a visible row");

    model
        .workspace_mut(id)
        .unwrap()
        .explorer_mut()
        .unwrap()
        .open_preview(target.join("kept.txt"));
    let preview_deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < preview_deadline {
        model.logic(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| model.ui(ui));
        output.textures_delta.clear();
        model.logic(&ctx);
        if model.workspace(id).unwrap().left_surface() == LeftSurface::Preview
            && model
                .workspace(id)
                .unwrap()
                .explorer()
                .unwrap()
                .preview_document()
                .is_some()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        model.workspace(id).unwrap().left_surface(),
        LeftSurface::Preview,
        "preview is open before the shell root is deleted"
    );

    std::fs::remove_dir_all(&target).unwrap();
    pump_until(&mut model, &ctx, |model| {
        model
            .workspace(id)
            .unwrap()
            .explorer()
            .unwrap()
            .root_unavailable()
            .is_some()
    });

    let workbench = model.workspace(id).unwrap();
    assert_eq!(
        workbench.explorer_root(),
        Some(target.as_path()),
        "the deleted path is still the honest effective root"
    );
    assert_eq!(
        workbench.shell_root_unavailable(),
        workbench.explorer().unwrap().root_unavailable(),
        "the coordinator and explorer agree on the unavailability detail"
    );
    assert!(
        workbench.explorer().unwrap().preview_document().is_none(),
        "the deleted root's preview document is cleared"
    );
    assert!(
        workbench.explorer().unwrap().preview_path().is_none(),
        "pending preview reads for the deleted root are invalidated"
    );
    assert_eq!(
        workbench.left_surface(),
        LeftSurface::Terminal,
        "the stale preview returns the left surface to the terminal"
    );
    assert!(
        workbench.terminal().unwrap().is_running(),
        "the shell remains usable in its deleted cwd"
    );
    let unavailable_logs = |workbench: &made_app::Workbench| {
        workbench
            .diagnostics()
            .log()
            .entries()
            .filter(|entry| entry.message.contains("shell directory unavailable at"))
            .count()
    };
    assert_eq!(unavailable_logs(workbench), 1);
    let rows_after = model
        .workspace_mut(id)
        .unwrap()
        .explorer_mut()
        .unwrap()
        .visible_row_count();
    assert_eq!(
        rows_after, rows_before,
        "the last usable tree is preserved while the root is unavailable"
    );

    // Several revalidation intervals must not flap or spam the log.
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        model.logic(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        unavailable_logs(model.workspace(id).unwrap()),
        1,
        "repeated checks of the same unavailable path log once"
    );
    assert!(model
        .workspace(id)
        .unwrap()
        .explorer()
        .unwrap()
        .root_unavailable()
        .is_some());

    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("recovered.txt"), "back\n").unwrap();
    pump_until(&mut model, &ctx, |model| {
        model
            .workspace(id)
            .unwrap()
            .explorer()
            .unwrap()
            .root_unavailable()
            .is_none()
    });
    let workbench = model.workspace(id).unwrap();
    assert_eq!(workbench.explorer_root(), Some(target.as_path()));
    assert!(workbench.shell_root_unavailable().is_none());
    assert!(workbench.explorer().unwrap().preview_document().is_none());
    assert_eq!(workbench.left_surface(), LeftSurface::Terminal);
    assert_eq!(
        workbench
            .diagnostics()
            .log()
            .entries()
            .filter(|entry| entry.message.contains("shell directory available again at"))
            .count(),
        1,
        "recovery is reported once"
    );

    assert!(model.close_workspace(id));
    assert!(model.workbench().is_none());
}

#[test]
fn runtime_instrumentation_records_activation_layout_preview_and_hidden_work() {
    if made_platform::PlatformFamily::current() == made_platform::PlatformFamily::Windows {
        return;
    }
    if isolated("runtime_instrumentation_records_activation_layout_preview_and_hidden_work") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("metrics-workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("preview.txt"), "preview\n").unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let ctx = egui::Context::default();
    open(&mut model, &workspace);
    let id = model.active_workspace_id().unwrap();
    let paint = |model: &mut AppModel, ctx: &egui::Context| {
        model.logic(ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| model.ui(ui));
        output.textures_delta.clear();
        // Paint metrics are finalized from the end-of-pass timestamp on the
        // next tick; this is the same one-frame accounting real frames use.
        model.logic(ctx);
    };
    paint(&mut model, &ctx);
    assert!(
        model
            .workspace(id)
            .unwrap()
            .metrics()
            .activation()
            .is_some(),
        "activation is measured through the first visible paint"
    );
    for _ in 0..3 {
        paint(&mut model, &ctx);
    }
    assert_eq!(
        model
            .workspace(id)
            .unwrap()
            .metrics()
            .counters()
            .hidden_grid_paints,
        0,
        "a single visible terminal must not report a transition frame as hidden work"
    );

    model.workspace_mut(id).unwrap().set_explorer_width(380.0);
    paint(&mut model, &ctx);
    assert!(model
        .workspace(id)
        .unwrap()
        .metrics()
        .explorer_layout()
        .is_some());
    model.workspace_mut(id).unwrap().set_explorer_visible(false);
    paint(&mut model, &ctx);
    assert!(
        model
            .workspace(id)
            .unwrap()
            .metrics()
            .counters()
            .explorer_layout_changes
            >= 2
    );

    model
        .workspace_mut(id)
        .unwrap()
        .explorer_mut()
        .unwrap()
        .open_preview(workspace.join("preview.txt"));
    let preview_deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < preview_deadline {
        paint(&mut model, &ctx);
        if model.workspace(id).unwrap().left_surface() == LeftSurface::Preview
            && model
                .workspace(id)
                .unwrap()
                .explorer()
                .and_then(|explorer| explorer.preview_document().map(|_| ()))
                .is_some()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        model.workspace(id).unwrap().left_surface(),
        LeftSurface::Preview
    );
    model.workspace_mut(id).unwrap().close_preview();
    paint(&mut model, &ctx);
    assert!(model
        .workspace(id)
        .unwrap()
        .metrics()
        .preview_roundtrip()
        .is_some());
    assert_eq!(
        model
            .workspace(id)
            .unwrap()
            .metrics()
            .counters()
            .preview_roundtrips,
        1
    );

    // A hidden workspace keeps consuming output without painting a grid.
    let second = temp.path().join("second");
    std::fs::create_dir(&second).unwrap();
    open(&mut model, &second);
    let second_id = model.active_workspace_id().unwrap();
    assert_ne!(second_id, id);
    let paints_before_hidden = model
        .workspace(id)
        .unwrap()
        .terminal()
        .unwrap()
        .grid_paint_count();
    type_command(
        model.workspace_mut(id).unwrap(),
        "for i in {1..2000}; do printf 'hidden metrics\\n'; done",
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        model.logic(&ctx);
        let counters = *model.workspace(id).unwrap().metrics().counters();
        if counters.hidden_output_bytes > 2_000 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let metrics = model.workspace(id).unwrap().metrics();
    assert!(
        metrics.counters().hidden_output_bytes > 2_000,
        "hidden output is counted: {:?}",
        metrics.counters()
    );
    assert_eq!(metrics.counters().hidden_grid_paints, 0);
    assert_eq!(
        model
            .workspace(id)
            .unwrap()
            .terminal()
            .unwrap()
            .grid_paint_count(),
        paints_before_hidden,
        "hidden work must never paint the grid"
    );
}

#[test]
fn deleted_shell_root_does_not_mark_an_agent_authoritative_root_unavailable() {
    if made_platform::PlatformFamily::current() == made_platform::PlatformFamily::Windows {
        return;
    }
    if isolated("deleted_shell_root_does_not_mark_an_agent_authoritative_root_unavailable") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let agent = root.join("agent-root");
    std::fs::create_dir(&agent).unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let ctx = egui::Context::default();
    open(&mut model, &workspace);
    let id = model.active_workspace_id().unwrap();
    model
        .workspace_mut(id)
        .unwrap()
        .set_shell_root_revalidation_interval(Duration::from_millis(20));

    let config = credentials(model.workbench().unwrap());
    let mut healthy = connect(&config);
    state(&mut healthy, 1, &agent);
    pump_until(&mut model, &ctx, |model| {
        model.workspace(id).unwrap().explorer_root() == Some(agent.as_path())
    });

    std::fs::remove_dir_all(&workspace).unwrap();
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        model.logic(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    let workbench = model.workspace(id).unwrap();
    assert_eq!(workbench.explorer_root(), Some(agent.as_path()));
    assert_eq!(workbench.explorer().unwrap().root_unavailable(), None);
    assert!(workbench.shell_root_unavailable().is_none());
    assert!(
        !workbench
            .diagnostics()
            .log()
            .entries()
            .any(|entry| entry.message.contains("shell directory unavailable at")),
        "the shell root is not authoritative while an agent session owns the root"
    );
    assert!(workbench.terminal().unwrap().is_running());
}

#[test]
fn workspace_preparation_is_backgrounded_and_cancellable() {
    if isolated("workspace_preparation_is_backgrounded_and_cancellable") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let ctx = egui::Context::default();
    open(&mut model, &first);
    let first_id = model.active_workspace_id().unwrap();
    let first_ticks = model.workspace(first_id).unwrap().tick_count();

    model.open_plan(LaunchPlan::open(&second, WorkspaceSource::Picker).unwrap());
    let frame_started = Instant::now();
    model.logic(&ctx);
    assert!(
        frame_started.elapsed() < Duration::from_secs(2),
        "starting a workspace must not run shell/config preparation on the frame"
    );
    assert!(
        model.preparation_in_flight(),
        "the second workspace should be prepared in the background"
    );
    assert!(
        model.workspace(first_id).unwrap().tick_count() > first_ticks,
        "an existing tab must keep ticking while another workspace prepares"
    );

    // Cancel before the next poll can insert the late preparation.
    model.begin_new_workspace();
    assert!(!model.preparation_in_flight());
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        model.logic(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        model.workspace_count(),
        1,
        "a cancelled preparation must never be inserted"
    );
    assert!(model.workspace(first_id).is_some());
    assert!(model.cancel_picker());
    assert_eq!(model.active_workspace_id(), Some(first_id));
}

#[test]
fn later_workspace_startup_uses_its_own_preparation_timestamp() {
    if isolated("later_workspace_startup_uses_its_own_preparation_timestamp") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let mut options = AppOptions::new();
    options.started_at = Instant::now() - Duration::from_secs(30);
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), options);
    let ctx = egui::Context::default();

    open(&mut model, temp.path());
    let first_id = model.active_workspace_id().unwrap();
    pump_until(&mut model, &ctx, |model| {
        model
            .workspace(first_id)
            .unwrap()
            .metrics()
            .startup()
            .is_some()
    });
    let first_startup = model
        .workspace(first_id)
        .unwrap()
        .metrics()
        .startup()
        .unwrap();
    assert!(
        first_startup < Duration::from_secs(5),
        "first workspace startup must not use the 30s-old process launch: {first_startup:?}"
    );

    let second = temp.path().join("second");
    std::fs::create_dir(&second).unwrap();
    open(&mut model, &second);
    let second_id = model.active_workspace_id().unwrap();
    assert_ne!(second_id, first_id);
    pump_until(&mut model, &ctx, |model| {
        model
            .workspace(second_id)
            .unwrap()
            .metrics()
            .startup()
            .is_some()
    });
    let second_startup = model
        .workspace(second_id)
        .unwrap()
        .metrics()
        .startup()
        .unwrap();
    assert!(
        second_startup < Duration::from_secs(5),
        "a later tab must report its own preparation time, not app launch: {second_startup:?}"
    );
}

#[test]
fn oversized_user_config_is_skipped_without_blocking_workspace_start() {
    if isolated("oversized_user_config_is_skipped_without_blocking_workspace_start") {
        return;
    }
    let config = isolated_opencode_config_path();
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(
        &config,
        vec![b'{'; made_app::workbench::MAX_USER_CONFIG_BYTES as usize + 1],
    )
    .unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let plan = LaunchPlan::open(workspace.path(), WorkspaceSource::Picker).unwrap();
    let started = Instant::now();
    let mut workbench = made_app::Workbench::start(
        Some(plan),
        None,
        workspace.path().to_path_buf(),
        TerminalTarget::Shell,
        Instant::now(),
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "an oversized config must not be read on the preparation path"
    );
    let failure = workbench.config_failure().unwrap_or_default();
    assert!(failure.contains("above the"), "{failure}");
    assert!(failure.contains("skipped without reading"), "{failure}");
    assert!(workbench.terminal().is_some(), "the shell stays usable");
    workbench.shutdown();
}

#[test]
fn special_user_config_is_rejected_without_blocking_workspace_start() {
    if made_platform::PlatformFamily::current() == made_platform::PlatformFamily::Windows {
        return;
    }
    if isolated("special_user_config_is_rejected_without_blocking_workspace_start") {
        return;
    }
    let config = isolated_opencode_config_path();
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    let created = std::process::Command::new("mkfifo")
        .arg(&config)
        .status()
        .is_ok_and(|status| status.success());
    if !created {
        eprintln!("skipping special-file assertion: mkfifo is unavailable");
        return;
    }
    let workspace = tempfile::tempdir().unwrap();
    let plan = LaunchPlan::open(workspace.path(), WorkspaceSource::Picker).unwrap();
    let cwd = workspace.path().to_path_buf();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let workbench = made_app::Workbench::start(
            Some(plan),
            None,
            cwd,
            TerminalTarget::Shell,
            Instant::now(),
        );
        let _ = sender.send(workbench);
    });
    let mut workbench = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("workspace start blocked on the special config file");
    let failure = workbench.config_failure().unwrap_or_default();
    assert!(failure.contains("not a regular file"), "{failure}");
    assert!(failure.contains("skipped without reading"), "{failure}");
    assert!(workbench.terminal().is_some(), "the shell stays usable");
    workbench.shutdown();
}

#[cfg(unix)]
#[test]
fn tui_terminal_target_spawns_opencode_with_composed_config_in_the_isolated_home() {
    if isolated_with_opencode_launches(
        "tui_terminal_target_spawns_opencode_with_composed_config_in_the_isolated_home",
        1,
    ) {
        return;
    }
    use made_platform::Platform;

    let home = made_platform::host_platform()
        .home_dir()
        .expect("test home")
        .canonicalize()
        .expect("canonical test home");
    let config_capture = PathBuf::from(
        std::env::var_os("MADE_OPENCODE_CONFIG_CAPTURE").expect("config capture path"),
    );
    let cwd_capture =
        PathBuf::from(std::env::var_os("MADE_OPENCODE_CWD_CAPTURE").expect("cwd capture path"));

    let mut workbench = made_app::Workbench::start(
        None,
        None,
        home.clone(),
        TerminalTarget::Tui(made_app::CompatibleTui::OpenCode),
        Instant::now(),
    );
    assert_eq!(
        workbench.terminal_target(),
        TerminalTarget::Tui(made_app::CompatibleTui::OpenCode)
    );

    let config = wait_for_capture(&config_capture);
    let config: Value = serde_json::from_str(&config).expect("captured config must be JSON");
    let options = config["plugins"]
        .as_array()
        .expect("composed config must contain a plugins array")
        .iter()
        .find_map(|entry| entry.get("options")?.get("made").cloned())
        .expect("composed config must contain Made plugin options");
    let options = options
        .as_object()
        .expect("Made plugin options must be an object");
    assert!(
        !options.is_empty(),
        "Made plugin options must be a non-empty object"
    );
    assert!(
        options.get("bridgeUrl").is_some_and(Value::is_string),
        "missing bridgeUrl"
    );
    assert!(
        options.get("token").is_some_and(Value::is_string),
        "missing token"
    );
    assert!(
        options.get("paneId").is_some_and(Value::is_string),
        "missing paneId"
    );

    let cwd = wait_for_capture(&cwd_capture);
    assert_eq!(
        std::fs::canonicalize(cwd.trim()).expect("captured cwd must exist"),
        home,
        "the TUI child must start in the isolated home"
    );

    workbench.shutdown();
}
