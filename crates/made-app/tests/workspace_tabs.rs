//! Real-process ownership and hidden-runtime regression tests.
mod support;
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

/// Set by the isolated-child helpers so helpers that persist app data can
/// prove they resolve their app-data directory under the test temp root.
const ISOLATED_TEST_HOME_ENV: &str = "MADE_WORKSPACE_TEST_HOME";

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
    // Hand the child an explicit fixture plugin package staged inside the
    // install root, which release builds require the plugin override to
    // canonicalize into; cargo never packages a plugin beside the test
    // binary.
    let plugins = support::StagedPlugins::stage("workspace-tabs");
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

/// Child process with fake `opencode` and `grok` binaries first on `PATH`.
#[cfg(unix)]
fn isolated_menu_launch(name: &str) -> bool {
    if std::env::var_os("MADE_WORKSPACE_TEST_CHILD").is_some() {
        return false;
    }
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let opencode_sentinel = home.path().join("opencode-launch");
    let opencode_config = home.path().join("opencode-config");
    let opencode_cwd = home.path().join("opencode-cwd");
    let grok_sentinel = home.path().join("grok-launch");
    let grok_argv0 = home.path().join("grok-argv0");
    let grok_config = home.path().join("grok-config");
    let grok_cwd = home.path().join("grok-cwd");
    let grok_bin = bin.join("grok");
    let opencode_bin = bin.join("opencode");
    std::fs::write(
        &opencode_bin,
        "#!/bin/sh\nprintf 'invoked\\n' >> \"$MADE_OPENCODE_SENTINEL\"\nprintf '%s' \"$OPENCODE_CLI_CONFIG_CONTENT\" > \"$MADE_OPENCODE_CONFIG_CAPTURE\"\npwd > \"$MADE_OPENCODE_CWD_CAPTURE\"\nprintf '1.9.0\\n'\n",
    )
    .unwrap();
    std::fs::write(
        &grok_bin,
        "#!/bin/sh\nprintf '%s\\n' \"$0\" > \"$MADE_GROK_ARGV0\"\nif [ -n \"${OPENCODE_CLI_CONFIG_CONTENT+x}\" ]; then printf '%s' \"$OPENCODE_CLI_CONFIG_CONTENT\" > \"$MADE_GROK_CONFIG\"; else printf unset > \"$MADE_GROK_CONFIG\"; fi\npwd > \"$MADE_GROK_CWD\"\nprintf 'invoked\\n' >> \"$MADE_GROK_SENTINEL\"\nsleep 30\n",
    )
    .unwrap();
    for executable in [&opencode_bin, &grok_bin] {
        assert!(std::process::Command::new("chmod")
            .arg("+x")
            .arg(executable)
            .status()
            .unwrap()
            .success());
    }
    let tmp = home.path().join("tmp");
    let runtime = home.path().join("runtime");
    let roaming = home.path().join("AppData").join("Roaming");
    let local = home.path().join("AppData").join("Local");
    for directory in [&tmp, &runtime, &roaming, &local] {
        std::fs::create_dir_all(directory).unwrap();
    }
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(bin).chain(std::env::split_paths(&path));
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", name, "--nocapture"])
        .env("MADE_WORKSPACE_TEST_CHILD", "1")
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
        .env("LOCALAPPDATA", &local)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("SHELL", "/bin/bash")
        .env("MADE_OPENCODE_SENTINEL", &opencode_sentinel)
        .env("MADE_OPENCODE_CONFIG_CAPTURE", &opencode_config)
        .env("MADE_OPENCODE_CWD_CAPTURE", &opencode_cwd)
        .env("MADE_GROK_SENTINEL", &grok_sentinel)
        .env("MADE_GROK_ARGV0", &grok_argv0)
        .env("MADE_GROK_CONFIG", &grok_config)
        .env("MADE_GROK_CWD", &grok_cwd)
        .env("MADE_GROK_BIN", &grok_bin);
    command.env_remove(made_bridge::config::CONFIG_CONTENT_ENV);
    command.env_remove(made_platform::SHELL_ENV);
    // Hand the child an explicit fixture plugin package staged inside the
    // install root, which release builds require the plugin override to
    // canonicalize into; cargo never packages a plugin beside the test
    // binary.
    let plugins = support::StagedPlugins::stage("workspace-menu");
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
    let grok_launches = std::fs::read_to_string(&grok_sentinel)
        .map(|content| content.lines().filter(|line| *line == "invoked").count())
        .unwrap_or(0);
    let opencode_launches = std::fs::read_to_string(&opencode_sentinel)
        .map(|content| content.lines().filter(|line| *line == "invoked").count())
        .unwrap_or(0);
    assert_eq!(grok_launches, 1, "the menu test must launch grok once");
    assert_eq!(
        opencode_launches, 1,
        "the menu test must still launch OpenCode once"
    );
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
            made_app::IntegrationStatus::Unsupported
        );
        let diagnostics = workbench.diagnostics_snapshot();
        assert_eq!(diagnostics.status_label, "unsupported");
        assert_eq!(diagnostics.reason, "OpenCode is not on PATH.");
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
    // A loaded host can start a shell or tick a tab later than the fixed
    // window above; wait on the condition, bounded, before asserting it.
    let settled = Instant::now() + Duration::from_secs(10);
    while ids.iter().any(|id| {
        let workbench = model.workspace(*id).unwrap();
        !workbench.terminal().unwrap().is_running() || workbench.tick_count() <= 1
    }) && Instant::now() < settled
    {
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
            made_app::IntegrationStatus::Unsupported,
            "elapsed time must not replace the not-on-PATH row"
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
    let ctx = egui::Context::default();
    // Bounded wait until every shell has drawn its first output (its prompt),
    // so the commands below are typed into a started shell on slow runners.
    let ready_deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < ready_deadline
        && !ids.iter().all(|id| {
            model
                .workspace(*id)
                .and_then(|workspace| workspace.terminal())
                .is_some_and(|terminal| terminal.output_bytes() > 0)
        })
    {
        model.logic(&ctx);
        std::thread::sleep(Duration::from_millis(10));
    }
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
    // Hosted macOS arm64 runners can take longer to drain the 2000-line shell
    // command. The assertion still requires every hidden tab to produce the
    // output; only the observation window grows.
    let deadline = Instant::now() + Duration::from_secs(20);
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

#[cfg(unix)]
fn grok_state_dir() -> PathBuf {
    let dir = PathBuf::from(std::env::var_os("HOME").expect("isolated home")).join(".grok");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(unix)]
fn write_grok_inventory(rows: &[Value]) {
    std::fs::write(
        grok_state_dir().join("active_sessions.json"),
        serde_json::to_string(rows).unwrap(),
    )
    .unwrap();
}

#[cfg(unix)]
fn append_grok_hook(line: &Value) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(grok_state_dir().join("made-grok-status.jsonl"))
        .unwrap();
    writeln!(file, "{}", serde_json::to_string(line).unwrap()).unwrap();
}

#[cfg(unix)]
fn tick_until_root(
    workbench: &mut made_app::Workbench,
    ctx: &egui::Context,
    expected: &std::path::Path,
    timeout: Duration,
) -> PathBuf {
    let deadline = Instant::now() + timeout;
    loop {
        workbench.tick(ctx);
        let root = workbench
            .explorer_root()
            .expect("explorer root")
            .to_path_buf();
        if root == expected || Instant::now() >= deadline {
            return root;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Like [`tick_until_root`], but also returns the instant the returning tick
/// began. `Workbench::tick` captures `now` as its first statement and passes
/// it to the Grok inventory pass, so for an adoption this is the instant the
/// poll's one-second debounce is measured from.
#[cfg(unix)]
fn tick_until_root_from(
    workbench: &mut made_app::Workbench,
    ctx: &egui::Context,
    expected: &std::path::Path,
    timeout: Duration,
) -> (PathBuf, Instant) {
    let deadline = Instant::now() + timeout;
    loop {
        let tick_began = Instant::now();
        workbench.tick(ctx);
        let root = workbench
            .explorer_root()
            .expect("explorer root")
            .to_path_buf();
        if root == expected || Instant::now() >= deadline {
            return (root, tick_began);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Inventory, hook history, prompt priority, focus, and shell restore.
///
/// The refresh decision lives in the workbench, not `session_follow`: a prompt
/// moves immediately, and a later inventory pass does not call follow again
/// while the newest attachment id is unchanged.
#[cfg(unix)]
#[test]
fn grok_follow_moves_on_attach_and_prompt_without_inventory_stealing_back() {
    if isolated("grok_follow_moves_on_attach_and_prompt_without_inventory_stealing_back") {
        return;
    }

    let workspace = tempfile::tempdir().unwrap();
    let mk = |name: &str| {
        let path = workspace.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path.canonicalize().unwrap()
    };
    let root = mk("root");
    let stale = mk("stale");
    let older = mk("older");
    let newer = mk("newer");
    let attached = mk("attached");
    let plan = LaunchPlan::open(&root, WorkspaceSource::Picker).unwrap();
    let mut workbench = made_app::Workbench::start(
        Some(plan),
        None,
        root.clone(),
        TerminalTarget::Shell,
        Instant::now(),
    );
    let pid = workbench
        .terminal()
        .and_then(made_app::TerminalPane::pid)
        .expect("shell pid");
    let ctx = egui::Context::default();

    append_grok_hook(&json!({
        "sessionId": "stale",
        "cwd": stale,
        "event": "UserPromptSubmit",
        "status": "working"
    }));
    workbench.tick(&ctx);
    assert_ne!(
        workbench.explorer_root(),
        Some(stale.as_path()),
        "a hook line already in the log when Made starts must not move the explorer"
    );

    let row = |id: &str, cwd: &std::path::Path, opened_at: &str| {
        json!({
            "pid": pid,
            "session_id": id,
            "cwd": cwd,
            "opened_at": opened_at
        })
    };
    write_grok_inventory(&[
        row("older", &older, "2024-01-01T00:00:01Z"),
        row("newer", &newer, "2024-01-01T00:00:02Z"),
    ]);
    let (newer_root, newer_adopted_tick) =
        tick_until_root_from(&mut workbench, &ctx, &newer, Duration::from_secs(3));
    assert_eq!(
        newer_root, newer,
        "the newest attachment moves the explorer"
    );

    write_grok_inventory(&[
        row("older", &older, "2024-01-01T00:00:01Z"),
        row("newer", &newer, "2024-01-01T00:00:02Z"),
        row("attached", &attached, "2024-01-01T00:00:03Z"),
    ]);
    let (attached_root, attached_adopted_tick) =
        tick_until_root_from(&mut workbench, &ctx, &attached, Duration::from_secs(3));
    assert_eq!(
        attached_root, attached,
        "a newly attached session moves the explorer on the next refresh"
    );
    // The inventory poll is debounced to once a second, measured from the
    // adopting tick's own start instant (`tick` captures `now` first and both
    // passes run inside their tick). Comparing the two adoption instants
    // asserts the debounce exactly, with no assumption about how long a tick
    // or the host may stall; counting ticks was not safe — five of them
    // straddled the one-second boundary under load.
    assert!(
        attached_adopted_tick.saturating_duration_since(newer_adopted_tick)
            >= Duration::from_secs(1),
        "inventory refresh is about once a second and must not run again immediately"
    );

    append_grok_hook(&json!({
        "sessionId": "older",
        "cwd": older,
        "event": "UserPromptSubmit",
        "status": "working"
    }));
    // A prompt moves the explorer in the very next hook read, without waiting
    // for the inventory poll; the bound only covers observation, not
    // scheduling.
    assert_eq!(
        tick_until_root(&mut workbench, &ctx, &older, Duration::from_secs(3)),
        older,
        "a prompt moves the explorer without waiting for inventory"
    );

    // Same attachment times, file order reversed: focus does not change
    // opened_at, so the refresh must not yank the explorer back to `attached`.
    write_grok_inventory(&[
        row("attached", &attached, "2024-01-01T00:00:03Z"),
        row("newer", &newer, "2024-01-01T00:00:02Z"),
        row("older", &older, "2024-01-01T00:00:01Z"),
    ]);
    std::thread::sleep(Duration::from_millis(1100));
    for _ in 0..8 {
        workbench.tick(&ctx);
    }
    assert_eq!(
        workbench.explorer_root(),
        Some(older.as_path()),
        "inventory must not steal the explorer back from the prompted session"
    );

    write_grok_inventory(&[]);
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(
        tick_until_root(&mut workbench, &ctx, &root, Duration::from_secs(3)),
        root,
        "no remaining Grok session returns the explorer to the shell"
    );
    workbench.shutdown();
}

#[cfg(unix)]
#[test]
fn grok_session_start_moves_the_explorer_immediately() {
    if isolated("grok_session_start_moves_the_explorer_immediately") {
        return;
    }
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("root");
    let stale = workspace.path().join("stale");
    let started = workspace.path().join("started");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::create_dir_all(&started).unwrap();
    let root = root.canonicalize().unwrap();
    let stale = stale.canonicalize().unwrap();
    let started = started.canonicalize().unwrap();
    let plan = LaunchPlan::open(&root, WorkspaceSource::Picker).unwrap();
    let mut workbench = made_app::Workbench::start(
        Some(plan),
        None,
        root.clone(),
        TerminalTarget::Shell,
        Instant::now(),
    );
    let ctx = egui::Context::default();
    append_grok_hook(&json!({
        "sessionId": "history",
        "cwd": stale,
        "event": "SessionStart",
        "status": "idle"
    }));
    workbench.tick(&ctx);
    assert_ne!(
        workbench.explorer_root(),
        Some(stale.as_path()),
        "a session-start line already on disk must not move the explorer"
    );
    append_grok_hook(&json!({
        "sessionId": "live",
        "cwd": started,
        "event": "SessionStart",
        "status": "idle"
    }));
    // The move happens in the tick that reads the hook line — not on the
    // one-second inventory cadence. A tick can stall past that debounce under
    // load, and the same tick's empty-inventory pass would then release the
    // unconfirmed session back to the shell; a revert is itself a poll, so
    // re-driving the hook line moves again inside a fresh debounce window.
    let mut started_immediately = false;
    for attempt in 0..4 {
        if attempt > 0 {
            append_grok_hook(&json!({
                "sessionId": "live",
                "cwd": started,
                "event": "SessionStart",
                "status": "idle"
            }));
        }
        workbench.tick(&ctx);
        if workbench.explorer_root() == Some(started.as_path()) {
            started_immediately = true;
            break;
        }
    }
    assert!(
        started_immediately,
        "session start moves the explorer immediately and shows idle"
    );
    workbench.shutdown();
}

#[cfg(unix)]
#[test]
fn a_later_owned_prompt_survives_release_of_an_earlier_provisional_move() {
    if isolated("a_later_owned_prompt_survives_release_of_an_earlier_provisional_move") {
        return;
    }
    let workspace = tempfile::tempdir().unwrap();
    let mk = |name: &str| {
        let path = workspace.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path.canonicalize().unwrap()
    };
    let root = mk("root");
    let older = mk("older");
    let newest = mk("newest");
    let foreign = mk("foreign");
    let plan = LaunchPlan::open(&root, WorkspaceSource::Picker).unwrap();
    let mut workbench = made_app::Workbench::start(
        Some(plan),
        None,
        root.clone(),
        TerminalTarget::Shell,
        Instant::now(),
    );
    let pid = workbench
        .terminal()
        .and_then(made_app::TerminalPane::pid)
        .expect("shell pid");
    let ctx = egui::Context::default();
    std::fs::create_dir_all(grok_state_dir()).unwrap();
    std::fs::write(grok_state_dir().join("made-grok-status.jsonl"), "").unwrap();
    workbench.tick(&ctx);

    let row = |id: &str, cwd: &std::path::Path, opened_at: &str| {
        json!({
            "pid": pid,
            "session_id": id,
            "cwd": cwd,
            "opened_at": opened_at
        })
    };
    write_grok_inventory(&[
        row("older", &older, "2024-01-01T00:00:01Z"),
        row("newest", &newest, "2024-01-01T00:00:02Z"),
    ]);
    assert_eq!(
        tick_until_root(&mut workbench, &ctx, &newest, Duration::from_secs(3)),
        newest,
        "the newest owned attachment is where the provisional slot will be filled"
    );

    append_grok_hook(&json!({
        "sessionId": "foreign-new",
        "cwd": foreign,
        "event": "SessionStart",
        "status": "idle"
    }));
    // The provisional move is observable only while no inventory poll reverts
    // it, and a tick can outlast the one-second debounce under load — the
    // same tick's pass then releases the unconfirmed session back to the
    // adopted root. A revert is itself a poll, so re-driving the hook line
    // moves again inside a fresh debounce window and the retry converges.
    let mut provisional = false;
    for attempt in 0..4 {
        if attempt > 0 {
            append_grok_hook(&json!({
                "sessionId": "foreign-new",
                "cwd": foreign,
                "event": "SessionStart",
                "status": "idle"
            }));
        }
        workbench.tick(&ctx);
        if workbench.explorer_root() == Some(foreign.as_path()) {
            provisional = true;
            break;
        }
    }
    assert!(
        provisional,
        "a session not yet in the inventory moves the explorer provisionally"
    );

    append_grok_hook(&json!({
        "sessionId": "older",
        "cwd": older,
        "event": "UserPromptSubmit",
        "status": "working"
    }));
    // One tick reads the appended hook line; the inventory poll cannot have
    // fired in the same tick's directory pass (its newest row is unchanged),
    // so this proves the prompt moved without waiting for inventory.
    workbench.tick(&ctx);
    assert_eq!(
        workbench.explorer_root(),
        Some(older.as_path()),
        "a prompt in the older owned session moves the explorer there immediately"
    );

    std::thread::sleep(Duration::from_millis(1100));
    for _ in 0..8 {
        workbench.tick(&ctx);
    }
    assert_ne!(
        workbench.explorer_root(),
        Some(newest.as_path()),
        "inventory must not restore the directory saved when the provisional slot was filled"
    );
    assert_ne!(
        workbench.explorer_root(),
        Some(foreign.as_path()),
        "the foreign session must not keep the explorer"
    );
    assert_eq!(
        workbench.explorer_root(),
        Some(older.as_path()),
        "the explorer stays on the prompted session"
    );
    workbench.shutdown();
}

#[cfg(unix)]
#[test]
fn a_foreign_hook_does_not_keep_or_yank_this_tabs_explorer() {
    if isolated("a_foreign_hook_does_not_keep_or_yank_this_tabs_explorer") {
        return;
    }
    let workspace = tempfile::tempdir().unwrap();
    let mk = |name: &str| {
        let path = workspace.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path.canonicalize().unwrap()
    };
    let root = mk("root");
    let own = mk("own");
    let other = mk("other");
    let foreign = mk("foreign");
    let plan = LaunchPlan::open(&root, WorkspaceSource::Picker).unwrap();
    let mut workbench = made_app::Workbench::start(
        Some(plan),
        None,
        root.clone(),
        TerminalTarget::Shell,
        Instant::now(),
    );
    let pid = workbench
        .terminal()
        .and_then(made_app::TerminalPane::pid)
        .expect("shell pid");
    let ctx = egui::Context::default();
    // An empty log is the first observation, so later lines are live.
    std::fs::create_dir_all(grok_state_dir()).unwrap();
    std::fs::write(grok_state_dir().join("made-grok-status.jsonl"), "").unwrap();
    workbench.tick(&ctx);

    let row = |id: &str, cwd: &std::path::Path, opened_at: &str, pid: u32| {
        json!({
            "pid": pid,
            "session_id": id,
            "cwd": cwd,
            "opened_at": opened_at
        })
    };
    write_grok_inventory(&[
        row("own", &own, "2024-01-01T00:00:01Z", pid),
        row("other", &other, "2024-01-01T00:00:02Z", pid),
        row("foreign-known", &foreign, "2024-01-01T00:00:03Z", 1),
    ]);
    assert_eq!(
        tick_until_root(&mut workbench, &ctx, &other, Duration::from_secs(3)),
        other,
        "the owned newest attachment still moves the explorer"
    );

    append_grok_hook(&json!({
        "sessionId": "own",
        "cwd": own,
        "event": "UserPromptSubmit",
        "status": "working"
    }));
    // One tick reads the appended hook line; the inventory poll cannot have
    // moved the root in the same tick's directory pass (its newest row is
    // unchanged), so this proves the prompt moved without waiting for it.
    workbench.tick(&ctx);
    assert_eq!(
        workbench.explorer_root(),
        Some(own.as_path()),
        "a prompt for a session this terminal owns moves immediately"
    );

    append_grok_hook(&json!({
        "sessionId": "foreign-known",
        "cwd": foreign,
        "event": "UserPromptSubmit",
        "status": "working"
    }));
    for _ in 0..5 {
        workbench.tick(&ctx);
    }
    assert_eq!(
        workbench.explorer_root(),
        Some(own.as_path()),
        "a hook for another process must not yank the explorer off this tab's prompted session"
    );

    append_grok_hook(&json!({
        "sessionId": "foreign-new",
        "cwd": foreign,
        "event": "SessionStart",
        "status": "idle"
    }));
    std::thread::sleep(Duration::from_millis(1100));
    for _ in 0..8 {
        workbench.tick(&ctx);
    }
    assert_ne!(
        workbench.explorer_root(),
        Some(foreign.as_path()),
        "a session this terminal does not own must not leave the explorer on its directory"
    );
    assert_ne!(
        workbench.explorer_root(),
        Some(other.as_path()),
        "dropping that session must not jump to a different open session"
    );
    assert_eq!(
        workbench.explorer_root(),
        Some(own.as_path()),
        "the explorer returns to the directory this tab was following"
    );
    workbench.shutdown();
}

#[cfg(unix)]
#[test]
fn menu_launched_grok_gets_no_plugin_config_and_opencode_still_does() {
    if isolated_menu_launch("menu_launched_grok_gets_no_plugin_config_and_opencode_still_does") {
        return;
    }
    use made_platform::Platform;

    let home = made_platform::host_platform()
        .home_dir()
        .expect("test home")
        .canonicalize()
        .expect("canonical test home");
    let grok_config_dir = home.join(".grok");
    let before = grok_config_dir.exists();

    let mut grok = made_app::Workbench::start(
        None,
        None,
        home.clone(),
        TerminalTarget::Tui(made_app::CompatibleTui::Grok),
        Instant::now(),
    );
    {
        let grok_terminal = grok.terminal().expect("grok child");
        assert!(
            grok_terminal
                .options()
                .env
                .iter()
                .all(|(key, _)| key != made_bridge::config::CONFIG_CONTENT_ENV),
            "a Grok child must not receive OpenCode plugin configuration"
        );
        assert_eq!(
            grok_terminal.options().args,
            vec![std::ffi::OsString::from("dashboard")],
            "a menu-launched Grok child is the absolute path plus dashboard"
        );
    }
    let argv0 = wait_for_capture(&PathBuf::from(
        std::env::var_os("MADE_GROK_ARGV0").expect("argv capture"),
    ));
    let config = wait_for_capture(&PathBuf::from(
        std::env::var_os("MADE_GROK_CONFIG").expect("config capture"),
    ));
    let cwd = wait_for_capture(&PathBuf::from(
        std::env::var_os("MADE_GROK_CWD").expect("cwd capture"),
    ));
    assert_eq!(
        argv0.trim(),
        std::env::var("MADE_GROK_BIN").unwrap(),
        "grok must be the absolute path, with no shell first"
    );
    assert_eq!(config.trim(), "unset");
    assert_eq!(
        std::fs::canonicalize(cwd.trim()).unwrap(),
        home,
        "grok must start in the home directory"
    );
    assert_eq!(
        grok_config_dir.exists(),
        before,
        "Made must not write the user's Grok config"
    );
    grok.shutdown();

    let mut opencode = made_app::Workbench::start(
        None,
        None,
        home.clone(),
        TerminalTarget::Tui(made_app::CompatibleTui::OpenCode),
        Instant::now(),
    );
    assert!(
        opencode
            .terminal()
            .expect("opencode child")
            .options()
            .args
            .is_empty(),
        "a menu-launched OpenCode child keeps an empty argument list"
    );
    let captured = wait_for_capture(&PathBuf::from(
        std::env::var_os("MADE_OPENCODE_CONFIG_CAPTURE").expect("opencode config"),
    ));
    let captured: Value = serde_json::from_str(&captured).expect("composed config");
    assert!(
        captured["plugins"]
            .as_array()
            .expect("plugins")
            .iter()
            .any(|entry| entry
                .get("options")
                .and_then(|options| options.get("made"))
                .is_some()),
        "OpenCode must still receive its composed plugin configuration"
    );
    let opencode_cwd = wait_for_capture(&PathBuf::from(
        std::env::var_os("MADE_OPENCODE_CWD_CAPTURE").expect("opencode cwd"),
    ));
    assert_eq!(std::fs::canonicalize(opencode_cwd.trim()).unwrap(), home);
    opencode.shutdown();
}

/// Child with a fake grok that blocks in `plugin install` until released, then
/// exits non-zero. `GROK_HOME` is the temporary home, never the user's `~/.grok`.
#[cfg(unix)]
fn isolated_failed_plugin_install(name: &str) -> bool {
    if std::env::var_os("MADE_WORKSPACE_TEST_CHILD").is_some() {
        return false;
    }
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join("bin");
    let grok_home = home.path().join("grok-home");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&grok_home).unwrap();
    let grok_bin = bin.join("grok");
    std::fs::write(
        &grok_bin,
        "#!/bin/sh\n\
if [ \"$1\" = plugin ] && [ \"$2\" = install ]; then\n\
  printf '%s\\n' \"$0\" > \"$MADE_GROK_INSTALL_ARGV0\"\n\
  printf '%s\\n' \"$*\" > \"$MADE_GROK_INSTALL_ARGS\"\n\
  if [ -n \"${OPENCODE_CLI_CONFIG_CONTENT+x}\" ]; then printf set > \"$MADE_GROK_OPENCODE_ENV\"; else printf unset > \"$MADE_GROK_OPENCODE_ENV\"; fi\n\
  printf start > \"$MADE_GROK_INSTALL_PHASE\"\n\
  i=0\n\
  while [ ! -f \"$MADE_GROK_INSTALL_RELEASE\" ] && [ \"$i\" -lt 200 ]; do\n\
    sleep 0.05\n\
    i=$((i + 1))\n\
  done\n\
  exit 1\n\
fi\n\
printf 'args:%s\\n' \"$*\" >> \"$MADE_GROK_LAUNCH_ARGS\"\n\
sleep 30\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&grok_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let tmp = home.path().join("tmp");
    let runtime = home.path().join("runtime");
    let roaming = home.path().join("AppData").join("Roaming");
    let local = home.path().join("AppData").join("Local");
    for directory in [&tmp, &runtime, &roaming, &local] {
        std::fs::create_dir_all(directory).unwrap();
    }
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(bin).chain(std::env::split_paths(&path));
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", name, "--nocapture"])
        .env("MADE_WORKSPACE_TEST_CHILD", "1")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("GROK_HOME", &grok_home)
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env("TMPDIR", &tmp)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("APPDATA", &roaming)
        .env("LOCALAPPDATA", &local)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("SHELL", "/bin/bash")
        .env("MADE_GROK_BIN", &grok_bin)
        .env("MADE_GROK_INSTALL_ARGV0", home.path().join("install-argv0"))
        .env("MADE_GROK_INSTALL_ARGS", home.path().join("install-args"))
        .env("MADE_GROK_INSTALL_PHASE", home.path().join("install-phase"))
        .env(
            "MADE_GROK_INSTALL_RELEASE",
            home.path().join("install-release"),
        )
        .env("MADE_GROK_OPENCODE_ENV", home.path().join("opencode-env"))
        .env("MADE_GROK_LAUNCH_ARGS", home.path().join("launch-args"));
    command.env_remove(made_bridge::config::CONFIG_CONTENT_ENV);
    command.env_remove(made_platform::SHELL_ENV);
    // Hand the child an explicit fixture plugin package staged inside the
    // install root, which release builds require the plugin override to
    // canonicalize into; cargo never packages a plugin beside the test
    // binary.
    let plugins = support::StagedPlugins::stage("workspace-install");
    command.env(
        made_platform::PLUGIN_DIR_ENV,
        plugins.package("opencode-made"),
    );
    // The shipped grok-made package is the staged copy beside the install root.
    support::stage_shipped_grok_plugin();
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

#[cfg(unix)]
#[test]
fn failed_plugin_install_still_starts_grok_and_records_hooks_unavailable() {
    if isolated_failed_plugin_install(
        "failed_plugin_install_still_starts_grok_and_records_hooks_unavailable",
    ) {
        return;
    }
    std::env::set_var(
        made_bridge::config::CONFIG_CONTENT_ENV,
        "leaked-plugin-config",
    );
    let release = PathBuf::from(std::env::var_os("MADE_GROK_INSTALL_RELEASE").unwrap());
    let config = PathBuf::from(std::env::var_os("GROK_HOME").unwrap()).join("config.toml");
    std::fs::write(&config, "made-must-not-edit = true\n").unwrap();
    let config_before = std::fs::read(&config).unwrap();
    let mut model = AppModel::from_launch(
        WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tui_detection(),
    );
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path()).unwrap();
    model.open_plan(LaunchPlan::open(root.path(), WorkspaceSource::Picker).unwrap());
    model.ensure_workbench();
    let shell_id = model.active_workspace_id().unwrap();
    assert!(
        model
            .workspace(shell_id)
            .and_then(made_app::Workbench::terminal)
            .is_some(),
        "the shell must start before plugin install is polled"
    );
    let ctx = egui::Context::default();
    let install_started = Instant::now();
    while !std::fs::read_to_string(std::env::var_os("MADE_GROK_INSTALL_PHASE").unwrap())
        .ok()
        .is_some_and(|text| text.trim() == "start")
    {
        assert!(
            install_started.elapsed() < Duration::from_secs(3),
            "detected grok did not run plugin install, or startup waited for it"
        );
        model.logic(&ctx);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        install_started.elapsed() < Duration::from_secs(3) && !release.exists(),
        "the shell must be up before plugin install finishes"
    );
    assert!(model
        .workspace(shell_id)
        .and_then(made_app::Workbench::terminal)
        .is_some());
    model.begin_tui_workspace(made_app::CompatibleTui::Grok);
    let grok_deadline = Instant::now() + Duration::from_secs(6);
    let grok_id = loop {
        model.logic(&ctx);
        if let Some(id) = model.workspace_ids().find(|id| *id != shell_id) {
            if model
                .workspace(id)
                .and_then(made_app::Workbench::terminal)
                .is_some_and(|terminal| terminal.is_running())
            {
                break id;
            }
        }
        assert!(
            Instant::now() < grok_deadline && !release.exists(),
            "menu-launched Grok waited on the failed plugin install"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let terminal = model
        .workspace(grok_id)
        .unwrap()
        .terminal()
        .expect("grok child");
    assert_eq!(
        terminal.options().args,
        vec![std::ffi::OsString::from("dashboard")],
        "a menu-launched Grok child is the absolute path plus dashboard"
    );
    assert!(
        terminal
            .options()
            .env
            .iter()
            .all(|(key, _)| key != made_bridge::config::CONFIG_CONTENT_ENV),
        "Grok must not receive OpenCode's plugin environment"
    );
    assert!(
        terminal
            .options()
            .shell
            .as_ref()
            .is_some_and(|program| PathBuf::from(program).is_absolute()),
        "menu-launched Grok must be the absolute detected path"
    );
    assert!(
        !hooks_unavailable(&model),
        "hooks stay available until the install command fails"
    );
    type_command(model.workspace_mut(shell_id).unwrap(), "grok");
    let launch_deadline = Instant::now() + Duration::from_secs(5);
    let launch_path = PathBuf::from(std::env::var_os("MADE_GROK_LAUNCH_ARGS").unwrap());
    loop {
        model.logic(&ctx);
        let text = std::fs::read_to_string(&launch_path).unwrap_or_default();
        if text.lines().any(|line| line == "args:") {
            break;
        }
        assert!(
            Instant::now() < launch_deadline,
            "a shell-typed grok did not start: {text}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let launches = std::fs::read_to_string(&launch_path).unwrap();
    assert!(
        launches.lines().any(|line| line == "args:dashboard"),
        "menu grok must receive dashboard: {launches}"
    );
    assert!(
        launches.lines().any(|line| line == "args:"),
        "a shell-typed grok must not be given dashboard: {launches}"
    );
    std::fs::write(&release, b"release").unwrap();
    let failed = Instant::now() + Duration::from_secs(6);
    while !hooks_unavailable(&model) {
        assert!(
            Instant::now() < failed,
            "a failed plugin install must record that session-status hooks are unavailable"
        );
        model.logic(&ctx);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(std::env::var_os("MADE_GROK_OPENCODE_ENV").unwrap())
            .expect("opencode env capture")
            .trim(),
        "unset"
    );
    assert_eq!(
        std::fs::read(&config).unwrap(),
        config_before,
        "Made must not hand-edit config.toml"
    );
    model.shutdown();
}

#[cfg(unix)]
fn hooks_unavailable(model: &AppModel) -> bool {
    model.workspace_ids().any(|id| {
        model
            .workspace(id)
            .unwrap()
            .diagnostics()
            .log()
            .entries()
            .any(|entry| {
                entry
                    .message
                    .contains("session-status hooks are unavailable")
            })
    })
}

#[test]
fn grok_ssh_requires_detection_and_launches_remote_grok_directly() {
    if isolated("grok_ssh_requires_detection_and_launches_remote_grok_directly") {
        return;
    }
    use made_app::remote_workspace::RemoteWorkspace;
    use made_app::ssh_servers::ServerMetadata;
    use made_remote::protocol::{Capabilities, Identity, RemotePath, RemoteTarget, TabCapability};

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
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
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
    let without_grok = model
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
        .hold_ssh_probe_result(with_grok, Ok(grok_capabilities))
        .expect("probe result records");
    model
        .hold_ssh_probe_result(without_grok, Ok(no_grok_capabilities))
        .expect("probe result records");

    // The row exists only where that server's own probe detected grok.
    assert!(model.remote_grok_detected_on(with_grok).unwrap());
    assert!(!model.remote_grok_detected_on(without_grok).unwrap());

    // The current tab keeps running: open a local shell tab first.
    model.open_plan(LaunchPlan::open(&root, WorkspaceSource::Picker).unwrap());
    model.ensure_workbench();
    let local_id = model.active_workspace_id().expect("local tab");

    // The launch is immediate: no folder picker step, no SSH panel draft.
    model
        .begin_remote_grok_launch(with_grok)
        .expect("grok ssh starts");
    let grok_id = model.active_workspace_id().expect("remote grok tab");
    assert_ne!(grok_id, local_id, "the launch opens a new tab");
    assert!(
        !model.remote_launch_pending(),
        "a direct Grok SSH launch must not open a folder picker draft"
    );
    assert_eq!(
        model.workspace(grok_id).unwrap().terminal_target(),
        TerminalTarget::RemoteGrok {
            authority: with_grok
        },
        "the new tab owns the remote Grok target"
    );
    assert!(
        model
            .workspace(local_id)
            .and_then(made_app::Workbench::terminal)
            .is_some_and(made_app::TerminalPane::is_running),
        "the current tab keeps running across the Grok SSH launch"
    );

    // An undetected server never starts a launch, locally or remotely.
    let before = model.workspace_count();
    assert!(
        model.begin_remote_grok_launch(without_grok).is_err(),
        "Grok SSH must refuse a server whose probe did not detect grok"
    );
    assert_eq!(
        model.workspace_count(),
        before,
        "a refused launch must not create a tab"
    );
    assert!(
        model.workspace(grok_id).unwrap().terminal_target()
            == TerminalTarget::RemoteGrok {
                authority: with_grok
            },
        "the refused attempt must not retarget an existing tab"
    );

    // The remote model binding path mirrors the launch target for tests.
    let identity = Identity {
        authority: with_grok,
        tab: TabCapability::random().unwrap(),
        epoch: 1,
        generation: 1,
    };
    let workspace =
        RemoteWorkspace::new(identity, RemotePath::new(b"/project".to_vec()).unwrap()).unwrap();
    model
        .attach_remote_grok_model(with_grok, workspace)
        .expect("remote model attaches");
    assert_eq!(
        model.active_workspace_id(),
        Some(grok_id.max(model.active_workspace_id().unwrap())),
        "attaching keeps a live tab selected"
    );
}

#[test]
fn grok_install_consent_is_explicit_bounded_and_withdrawable() {
    if isolated("grok_install_consent_is_explicit_bounded_and_withdrawable") {
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
    let id = model
        .add_ssh_metadata(ServerMetadata {
            display_name: "Grok server".into(),
            host: "grok-server".into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("server metadata saves");
    model
        .hold_ssh_probe_result(id, Ok(capabilities))
        .expect("probe result records");

    let ctx = egui::Context::default();
    let poll_until = |model: &mut AppModel, ready: &dyn Fn(&AppModel) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            model.logic(&ctx);
            if ready(model) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the model did not reach the expected state");
    };

    // Before any approval nothing is installed and no attempt is running.
    assert!(!model.remote_grok_consent_granted(id).unwrap());
    assert!(model.remote_grok_install_state(id).is_none());
    assert!(!model.remote_grok_install_running(id));

    // Refusal stores nothing and starts nothing.
    model.refuse_remote_grok_install(id);
    assert!(model.remote_grok_install_refused(id));
    assert!(!model.remote_grok_consent_granted(id).unwrap());
    assert!(model.remote_grok_install_state(id).is_none());

    // A fresh server has no staged helper, so approval stays gated until this
    // session verifies a provisioned or reused companion.
    assert!(!model.remote_grok_install_ready(id));
    assert!(model.approve_remote_grok_install(id).is_err());
    assert!(!model.remote_grok_install_running(id));
    assert!(!model.remote_grok_consent_granted(id).unwrap());
    model.hold_remote_companion_provisioned(id);
    assert!(model.remote_grok_install_ready(id));

    // Approval persists the per-server/version consent and starts the bounded
    // attempt. In this environment the attempt fails closed (no packaged
    // askpass); the failure is honest and consent stays recorded.
    model.approve_remote_grok_install(id).expect("approve");
    assert!(
        model.remote_grok_consent_granted(id).unwrap(),
        "approval persists the consent"
    );
    assert!(
        model.remote_grok_install_running(id),
        "the bounded attempt starts only after approval"
    );
    poll_until(&mut model, &|model: &AppModel| {
        !matches!(
            model.remote_grok_install_state(id),
            Some(made_app::app::RemoteGrokInstallState::Running)
        )
    });
    assert!(
        matches!(
            model.remote_grok_install_state(id),
            Some(made_app::app::RemoteGrokInstallState::Failed { .. })
        ),
        "the attempt fails closed without the packaged helper: {:?}",
        model.remote_grok_install_state(id)
    );
    assert!(
        model.remote_grok_consent_granted(id).unwrap(),
        "a failed install never revokes the recorded approval"
    );
    let notice = model.ssh_notice().unwrap_or_default();
    assert!(
        notice.contains("install failed"),
        "the failure is disclosed honestly: {notice}"
    );

    // Withdrawal removes the consent; no further install may start.
    model.withdraw_remote_grok_install(id).expect("withdraw");
    assert!(!model.remote_grok_consent_granted(id).unwrap());
    assert!(!model.remote_grok_install_refused(id));

    // A server without detected grok refuses approval outright.
    let other = model
        .add_ssh_metadata(ServerMetadata {
            display_name: "Plain server".into(),
            host: "plain-server".into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("server metadata saves");
    assert!(model.approve_remote_grok_install(other).is_err());
    assert!(!model.remote_grok_consent_granted(other).unwrap());
    assert!(model.remote_grok_install_state(other).is_none());
}

#[test]
fn grok_install_approval_is_gated_until_the_companion_is_provisioned() {
    if isolated("grok_install_approval_is_gated_until_the_companion_is_provisioned") {
        return;
    }
    use made_app::ssh_servers::ServerMetadata;
    use made_remote::protocol::{Capabilities, RemoteTarget};

    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = model
        .add_ssh_metadata(ServerMetadata {
            display_name: "Grok server".into(),
            host: "grok-server".into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("server metadata saves");
    model
        .hold_ssh_probe_result(
            id,
            Ok(Capabilities {
                target: RemoteTarget::LinuxX64,
                tmux: false,
                opencode: false,
                shell_cwd: false,
                helper_version: "probe".to_owned(),
                grok: true,
                python3: false,
            }),
        )
        .expect("probe result records");

    // A fresh server has no helper for `GrokPluginInstall` to exec.
    assert!(!model.remote_grok_install_ready(id));
    let error = model
        .approve_remote_grok_install(id)
        .expect_err("a missing companion must gate approval");
    assert!(
        error.contains("private companion"),
        "the gate names the missing requirement: {error}"
    );
    assert!(!model.remote_grok_consent_granted(id).unwrap());
    assert!(!model.remote_grok_install_running(id));
    assert!(model.remote_grok_install_state(id).is_none());

    // A provisioned or reused companion makes this authority independently
    // approvable and starts the bounded fixed install.
    model.hold_remote_companion_provisioned(id);
    assert!(model.remote_grok_install_ready(id));
    model
        .approve_remote_grok_install(id)
        .expect("a provisioned companion allows approval");
    assert!(model.remote_grok_consent_granted(id).unwrap());
    assert!(model.remote_grok_install_running(id));
    assert!(matches!(
        model.remote_grok_install_state(id),
        Some(made_app::app::RemoteGrokInstallState::Running)
    ));
}

#[test]
fn grok_install_attempts_stay_independent_across_servers() {
    if isolated("grok_install_attempts_stay_independent_across_servers") {
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
    let server_a = model
        .add_ssh_metadata(ServerMetadata {
            display_name: "Grok A".into(),
            host: "grok-a".into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("server metadata saves");
    model
        .hold_ssh_probe_result(server_a, Ok(capabilities.clone()))
        .expect("probe result records");
    let server_b = model
        .add_ssh_metadata(ServerMetadata {
            display_name: "Grok B".into(),
            host: "grok-b".into(),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("server metadata saves");
    model
        .hold_ssh_probe_result(server_b, Ok(capabilities))
        .expect("probe result records");

    let ctx = egui::Context::default();
    let poll_until = |model: &mut AppModel, ready: &dyn Fn(&AppModel) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            model.logic(&ctx);
            if ready(model) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the model did not reach the expected state");
    };

    // Approving A starts A's own bounded attempt once its companion is
    // provisioned or verified in this session.
    model.hold_remote_companion_provisioned(server_a);
    model
        .approve_remote_grok_install(server_a)
        .expect("approve A");
    assert!(model.remote_grok_install_running(server_a));

    // Approving B must not cancel or overwrite A's in-flight attempt: SSH
    // tabs are independent servers.
    model.hold_remote_companion_provisioned(server_b);
    model
        .approve_remote_grok_install(server_b)
        .expect("approve B");
    assert!(
        model.remote_grok_install_running(server_a),
        "approving B must not cancel A's in-flight attempt"
    );
    assert!(model.remote_grok_install_running(server_b));
    assert!(
        matches!(
            model.remote_grok_install_state(server_a),
            Some(made_app::app::RemoteGrokInstallState::Running)
        ),
        "A's record stays honest while B is approved: {:?}",
        model.remote_grok_install_state(server_a)
    );
    assert!(model.remote_grok_consent_granted(server_a).unwrap());
    assert!(model.remote_grok_consent_granted(server_b).unwrap());

    // Both attempts finish on their own, each result recorded against its
    // own authority. In this environment both fail closed (no packaged
    // askpass); the failure is honest and both consents stay recorded.
    poll_until(&mut model, &|model: &AppModel| {
        !matches!(
            model.remote_grok_install_state(server_a),
            Some(made_app::app::RemoteGrokInstallState::Running)
        ) && !matches!(
            model.remote_grok_install_state(server_b),
            Some(made_app::app::RemoteGrokInstallState::Running)
        ) && !model.remote_grok_install_running(server_a)
            && !model.remote_grok_install_running(server_b)
    });
    for (server, name) in [(server_a, "A"), (server_b, "B")] {
        assert!(
            matches!(
                model.remote_grok_install_state(server),
                Some(made_app::app::RemoteGrokInstallState::Failed { .. })
            ),
            "{name}'s attempt fails closed without the packaged helper: {:?}",
            model.remote_grok_install_state(server)
        );
        assert!(
            model.remote_grok_consent_granted(server).unwrap(),
            "{name}'s failed install never revokes its recorded approval"
        );
    }

    // Withdrawing A touches neither B's consent nor B's recorded outcome.
    model
        .withdraw_remote_grok_install(server_a)
        .expect("withdraw A");
    assert!(!model.remote_grok_consent_granted(server_a).unwrap());
    assert!(
        model.remote_grok_consent_granted(server_b).unwrap(),
        "withdrawing A must not withdraw B"
    );
    assert!(matches!(
        model.remote_grok_install_state(server_b),
        Some(made_app::app::RemoteGrokInstallState::Failed { .. })
    ));

    // Re-approving B replaces only B's own attempt; A's outcome stays.
    model
        .approve_remote_grok_install(server_b)
        .expect("re-approve B");
    assert!(model.remote_grok_install_running(server_b));
    assert!(
        !model.remote_grok_install_running(server_a),
        "A has no attempt after it finished"
    );
    assert!(matches!(
        model.remote_grok_install_state(server_a),
        Some(made_app::app::RemoteGrokInstallState::Failed { .. })
    ));
    poll_until(&mut model, &|model: &AppModel| {
        !model.remote_grok_install_running(server_b)
    });
    assert!(matches!(
        model.remote_grok_install_state(server_b),
        Some(made_app::app::RemoteGrokInstallState::Failed { .. })
    ));
}

fn remote_update_model(
    name: &str,
) -> (
    AppModel,
    made_remote::protocol::AuthorityId,
    made_app::app::WorkspaceId,
) {
    remote_update_model_for(name, false)
}

fn remote_update_model_for(
    name: &str,
    claude: bool,
) -> (
    AppModel,
    made_remote::protocol::AuthorityId,
    made_app::app::WorkspaceId,
) {
    use made_app::remote_workspace::RemoteWorkspace;
    use made_app::ssh_servers::ServerMetadata;
    use made_platform::Platform;
    use made_remote::protocol::{Identity, RemotePath, TabCapability};

    // `add_ssh_metadata` persists immediately. Refuse to build this model
    // unless the resolved app-data directory is provably under the temp root
    // supplied by an isolated-child helper, so a future direct caller cannot
    // write the user's real ssh-servers.json.
    let test_root = std::env::var_os(ISOLATED_TEST_HOME_ENV)
        .expect("remote_update_model requires an isolated temporary HOME");
    let app_data = made_platform::host_platform()
        .app_paths()
        .expect("test app paths")
        .data_dir;
    assert!(
        app_data.starts_with(&test_root),
        "remote_update_model would write the host app-data registry at {}",
        app_data.display()
    );

    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let id = model
        .add_ssh_metadata(ServerMetadata {
            display_name: name.into(),
            host: format!("{name}.example"),
            user: None,
            port: None,
            key_file: None,
            initial_directory: None,
        })
        .expect("server metadata saves");
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
    if claude {
        model
            .hold_ssh_probe_result(
                id,
                Ok(made_remote::protocol::Capabilities {
                    target: made_remote::protocol::RemoteTarget::LinuxX64,
                    tmux: false,
                    opencode: false,
                    shell_cwd: false,
                    helper_version: "probe".to_owned(),
                    grok: false,
                    python3: false,
                }),
            )
            .expect("probe result records");
        model
            .hold_ssh_claude_detection(id, true)
            .expect("detection records");
        model
            .attach_remote_claude_model(id, workspace)
            .expect("remote claude model");
    } else {
        model
            .attach_remote_model(id, workspace, false)
            .expect("remote model");
    }
    let remote_id = model.active_workspace_id().expect("remote tab");
    model
        .workspace_mut(remote_id)
        .unwrap()
        .hold_remote_target("x86_64-unknown-linux-gnu");
    (model, id, remote_id)
}

fn update_required_error() -> made_app::ssh_servers::AttemptError {
    made_app::ssh_servers::AttemptError {
        class: made_app::remote_session::SshFailureClass::UpdateRequired,
        detail: "the installed companion must be updated with explicit approval".to_owned(),
        fingerprint: None,
    }
}

#[test]
fn same_version_update_waits_for_separate_approval_and_reports_the_real_reason() {
    if isolated("same_version_update_waits_for_separate_approval_and_reports_the_real_reason") {
        return;
    }
    let (mut model, authority, remote_id) = remote_update_model("update-boundary");
    // An earlier successful stage is no longer a usable install path once this
    // attempt reports the same-version mismatch.
    model.hold_remote_companion_provisioned(authority);
    assert!(model.remote_grok_install_ready(authority));
    model
        .workspace_mut(remote_id)
        .unwrap()
        .deliver_stage_result(Err(update_required_error()));
    let ctx = egui::Context::default();
    model.logic(&ctx);

    let workbench = model.workspace(remote_id).unwrap();
    assert!(workbench.remote_needs_update());
    assert!(
        !workbench.companion_spawn_requested(),
        "a same-version mismatch must not spawn the companion before approval"
    );
    assert!(!workbench.remote_update_approved());
    assert_eq!(
        workbench.remote_update_target(),
        Some("x86_64-unknown-linux-gnu")
    );
    assert!(
        workbench
            .remote_detail()
            .unwrap()
            .contains("installed companion must be updated"),
        "the actual companion update reason must survive: {:?}",
        workbench.remote_detail()
    );
    assert!(!model.remote_grok_install_ready(authority));

    // A pre-attach stage failure has no tree to show and must not spin.
    let projection = workbench.remote_workspace().unwrap().projection();
    assert!(matches!(
        projection.root_state,
        made_app::remote_workspace::Freshness::Unavailable(_)
    ));
    assert!(projection.directories.is_empty());

    // The initial provisioning consent is not granted or implied.
    assert!(!model
        .remote_provision_consent_granted(authority)
        .expect("consent state"));

    // The copied report keeps the actual reason, version and honest failure
    // classification, with no path/hash/secret.
    let report = model
        .workspace(remote_id)
        .unwrap()
        .sanitized_diagnostics_text();
    assert!(
        report.contains("installed companion must be updated"),
        "the copied report must preserve the companion update reason: {report}"
    );
    assert!(
        report.contains(&format!("remote version: {}", made_app::APP_VERSION)),
        "{report}"
    );
    assert!(!report.contains("ssh connection failed"), "{report}");
    assert!(
        !report
            .split_whitespace()
            .any(|word| word.len() >= 32 && word.chars().all(|ch| ch.is_ascii_hexdigit())),
        "the copied report must not add a hash: {report}"
    );

    // Approval starts only this attempt's update stage; the companion still
    // waits for a successful stage result.
    model
        .approve_remote_companion_update()
        .expect("update approval");
    let workbench = model.workspace(remote_id).unwrap();
    assert!(workbench.remote_update_approved());
    assert!(workbench.remote_update_in_progress());
    assert!(!workbench.companion_spawn_requested());

    // Approved update success continues the normal companion launch.
    model
        .workspace_mut(remote_id)
        .unwrap()
        .deliver_stage_result(Ok(()));
    let workbench = model.workspace(remote_id).unwrap();
    assert!(workbench.companion_spawn_requested());
    assert!(!workbench.remote_needs_update());
    assert!(!workbench.remote_update_approved());
    // The stage success also starts an asynchronous companion attach. Its
    // failure would clear install readiness, so publish the diagnostics
    // snapshot without ticking the remote tab: that reads this tab's
    // post-stage provisioning while the attach result is still queued.
    model.hold_grok_diagnostics(made_app::app::GrokDiagnosticsHold::NotOnPath);
    assert!(
        model.remote_grok_install_ready(authority),
        "a successful matching update stage makes the current path install-ready"
    );
    // Release the queued attach result now that the stage-success transition
    // has been observed; the frame loop consumes it and the thread exits.
    model.logic(&ctx);

    // A later exact staging reuse succeeds without another update prompt.
    model
        .workspace_mut(remote_id)
        .unwrap()
        .deliver_stage_result(Ok(()));
    let workbench = model.workspace(remote_id).unwrap();
    assert!(!workbench.remote_needs_update());
    assert!(!workbench.remote_update_approved());
    assert!(workbench.companion_spawn_requested());
    drop(model);
}

#[test]
fn update_refusal_writes_nothing_and_failed_update_keeps_retry_actionable() {
    if isolated("update_refusal_writes_nothing_and_failed_update_keeps_retry_actionable") {
        return;
    }
    let (mut model, authority, remote_id) = remote_update_model("update-refusal");
    model
        .workspace_mut(remote_id)
        .unwrap()
        .deliver_stage_result(Err(update_required_error()));
    model.refuse_remote_companion_update();

    let workbench = model.workspace(remote_id).unwrap();
    assert!(!workbench.remote_needs_update());
    assert!(!workbench.remote_update_approved());
    assert!(!workbench.companion_spawn_requested());
    assert!(!model.remote_grok_install_ready(authority));
    assert!(
        workbench
            .remote_detail()
            .unwrap()
            .contains("Nothing was written or deleted"),
        "{:?}",
        workbench.remote_detail()
    );

    let (mut model, _authority, remote_id) = remote_update_model("update-retry");
    model
        .workspace_mut(remote_id)
        .unwrap()
        .deliver_stage_result(Err(update_required_error()));
    model
        .approve_remote_companion_update()
        .expect("update approval");
    model
        .workspace_mut(remote_id)
        .unwrap()
        .deliver_stage_result(Err(made_app::ssh_servers::AttemptError {
            class: made_app::remote_session::SshFailureClass::Timeout,
            detail: "ssh connection timed out".to_owned(),
            fingerprint: None,
        }));
    let workbench = model.workspace(remote_id).unwrap();
    assert!(workbench.remote_needs_update());
    assert!(workbench.remote_update_failed());
    assert!(
        !workbench.companion_spawn_requested(),
        "a failed update stage must not fall through to the companion"
    );
    assert!(workbench
        .remote_detail()
        .unwrap()
        .contains("ssh connection timed out"));
    model.retry_remote_companion_update().expect("update retry");
    let workbench = model.workspace(remote_id).unwrap();
    assert!(workbench.remote_update_approved());
    assert!(workbench.remote_update_in_progress());
    assert!(
        !workbench.remote_update_failed(),
        "a new retry attempt clears the previous update failure"
    );
    assert!(!workbench.companion_spawn_requested());
}

#[test]
fn terminal_update_failures_clear_consent_before_a_later_normal_stage() {
    if isolated("terminal_update_failures_clear_consent_before_a_later_normal_stage") {
        return;
    }
    use made_app::remote_session::SshFailureClass;
    for class in [
        SshFailureClass::AuthFailed,
        SshFailureClass::Cancelled,
        SshFailureClass::MissingCapability,
        SshFailureClass::ChangedHostKey,
    ] {
        let (mut model, _authority, remote_id) =
            remote_update_model(&format!("terminal-update-{class:?}"));
        model
            .workspace_mut(remote_id)
            .unwrap()
            .deliver_stage_result(Err(update_required_error()));
        model
            .approve_remote_companion_update()
            .expect("update approval");
        assert!(model.workspace(remote_id).unwrap().remote_update_approved());

        model
            .workspace_mut(remote_id)
            .unwrap()
            .deliver_stage_result(Err(made_app::ssh_servers::AttemptError {
                class,
                detail: format!("terminal {class:?}"),
                fingerprint: None,
            }));
        let workbench = model.workspace(remote_id).unwrap();
        assert!(
            !workbench.remote_update_approved(),
            "{class:?} must clear this attempt's update consent"
        );
        assert!(
            !workbench.remote_update_failed(),
            "{class:?} must not leave an update retry card"
        );
        assert!(!workbench.remote_needs_update(), "{class:?}");
        assert!(
            workbench.remote_update_target().is_none(),
            "{class:?} must clear the update target"
        );

        // A later ordinary stage failure must stay an ordinary open failure;
        // it must not inherit update consent or an update retry state.
        model
            .workspace_mut(remote_id)
            .unwrap()
            .deliver_stage_result(Err(made_app::ssh_servers::AttemptError {
                class: SshFailureClass::Timeout,
                detail: "ssh connection timed out".to_owned(),
                fingerprint: None,
            }));
        let workbench = model.workspace(remote_id).unwrap();
        assert!(
            !workbench.remote_needs_update(),
            "{class:?} let a later stage inherit update consent"
        );
        assert!(!workbench.remote_update_failed(), "{class:?}");
        assert!(!workbench.remote_update_approved(), "{class:?}");
    }
}

fn registry_file_exists(root: &std::path::Path) -> bool {
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

/// Run the update tests with an outer temporary HOME/XDG. Each test must
/// isolate itself in a nested temp HOME, so the outer root never receives the
/// private SSH registry that `add_ssh_metadata` persists immediately.
#[test]
fn update_tests_never_write_the_host_app_data_registry() {
    if std::env::var_os("MADE_WORKSPACE_TEST_CHILD").is_some() {
        return;
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
    for name in [
        "same_version_update_waits_for_separate_approval_and_reports_the_real_reason",
        "update_refusal_writes_nothing_and_failed_update_keeps_retry_actionable",
        "terminal_update_failures_clear_consent_before_a_later_normal_stage",
        "claude_ssh_against_an_out_of_date_companion_requires_the_ssh_06_update",
    ] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", name, "--nocapture"])
            .env_remove("MADE_WORKSPACE_TEST_CHILD")
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
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{name} failed in the canary HOME:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !registry_file_exists(home.path()),
            "{name} wrote {} under the host app-data root",
            made_app::ssh_servers::REGISTRY_FILE
        );
    }
}

#[test]
fn claude_ssh_requires_detection_and_launches_remote_claude_directly() {
    if isolated("claude_ssh_requires_detection_and_launches_remote_claude_directly") {
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
        grok: false,
        python3: false,
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let mut model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let mut add = |name: &str| {
        model
            .add_ssh_metadata(ServerMetadata {
                display_name: name.into(),
                host: format!("{name}.example"),
                user: None,
                port: None,
                key_file: None,
                initial_directory: None,
            })
            .expect("server metadata saves")
    };
    let with_claude = add("claude-server");
    let without_claude = add("plain-server");
    for id in [with_claude, without_claude] {
        model
            .hold_ssh_probe_result(id, Ok(capabilities.clone()))
            .expect("probe result records");
    }
    model
        .hold_ssh_claude_detection(with_claude, true)
        .expect("detection records");

    // The row exists only where that server's own probe detected claude.
    assert!(model.remote_claude_detected_on(with_claude).unwrap());
    assert!(!model.remote_claude_detected_on(without_claude).unwrap());

    // The current tab keeps running: open a local shell tab first.
    model.open_plan(LaunchPlan::open(&root, WorkspaceSource::Picker).unwrap());
    model.ensure_workbench();
    let local_id = model.active_workspace_id().expect("local tab");

    model
        .begin_remote_claude_launch(with_claude)
        .expect("claude ssh starts");
    let claude_id = model.active_workspace_id().expect("remote claude tab");
    assert_ne!(claude_id, local_id, "the launch opens a new tab");
    assert!(
        !model.remote_launch_pending(),
        "a direct Claude Code SSH launch must not open a folder picker draft"
    );
    assert_eq!(
        model.workspace(claude_id).unwrap().terminal_target(),
        TerminalTarget::RemoteClaude {
            authority: with_claude
        },
    );
    assert!(
        model
            .workspace(local_id)
            .and_then(made_app::Workbench::terminal)
            .is_some_and(made_app::TerminalPane::is_running),
        "the current tab keeps running across the Claude Code SSH launch"
    );

    // An undetected server never starts a launch, locally or remotely.
    let before = model.workspace_count();
    assert!(model.begin_remote_claude_launch(without_claude).is_err());
    assert_eq!(model.workspace_count(), before);
}

#[test]
fn claude_ssh_against_an_out_of_date_companion_requires_the_ssh_06_update() {
    if isolated("claude_ssh_against_an_out_of_date_companion_requires_the_ssh_06_update") {
        return;
    }
    let (mut model, authority, remote_id) = remote_update_model_for("claude-update", true);
    assert_eq!(
        model.workspace(remote_id).unwrap().terminal_target(),
        TerminalTarget::RemoteClaude { authority },
    );
    // The staged companion predates the Claude target: the fixed stage
    // reports the same-version mismatch before any companion is spawned.
    model
        .workspace_mut(remote_id)
        .unwrap()
        .deliver_stage_result(Err(update_required_error()));
    let ctx = egui::Context::default();
    model.logic(&ctx);

    let workbench = model.workspace(remote_id).unwrap();
    assert!(workbench.remote_needs_update(), "SSH-06 update-required");
    assert!(
        !workbench.companion_spawn_requested(),
        "no companion, so no remote shell or other program, starts"
    );
    assert!(!workbench.remote_update_approved());
    assert_eq!(
        workbench.terminal_target(),
        TerminalTarget::RemoteClaude { authority },
        "the tab does not fall back to a shell"
    );
    assert!(
        workbench.terminal().is_none(),
        "no local shell or process backs the tab"
    );
    assert!(workbench
        .remote_detail()
        .unwrap()
        .contains("installed companion must be updated"));

    // Refusing leaves everything untouched: nothing written, nothing started.
    model.refuse_remote_companion_update();
    let workbench = model.workspace(remote_id).unwrap();
    assert!(!workbench.remote_needs_update());
    assert!(!workbench.companion_spawn_requested());
    assert!(workbench
        .remote_detail()
        .unwrap()
        .contains("Nothing was written or deleted"));
    assert!(workbench.terminal().is_none());
}
