//! A fake integration defined only in test code, plugged in through the same
//! interface and an injected list (AIA-03, AIA-04, AIA-09).
//!
//! `FakeAgent` appears in every seam the requirement lists without a
//! production code edit: menu rows, detection reaction, launch arguments and
//! variables, app-wide and per-tab state, polling and teardown, follow ranking,
//! tab presentation, diagnostics rows, About rows, copied report lines and
//! smoke markers. A second variant ranks above Claude Code. The fake holds the
//! explorer root and brands a tab from a directory its tab state reports
//! through a real tab. The list-variation
//! tests run the same seams over the product list, the product list plus the
//! fake, and the product list without Claude Code, and show that the output of
//! each seam is exactly the list's entries.
//!
//! Tests that start a workspace re-run themselves in a child process with an
//! isolated home and fake agent programs first on `PATH`, so the shared
//! recorders below never see another test.
#![cfg(unix)]

mod support;

use std::any::Any;
use std::fs;
use std::io::Write;
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use made_app::app::{AppModel, AppOptions};
use made_app::diagnostics::{DiagnosticsPill, DiagnosticsSnapshot};
use made_app::integrations::{
    apply_contributions, validate, AppCtx, AppInit, AppState, Descriptor, DiagnosticsDecl,
    EnvContribution, EnvPolicy, FollowKind, FollowPolicy, Integration, IntegrationList,
    IntegrationRef, LaunchCtx, Mark, OnUnusable, Presentation, PresentationCtx, Rank, RowDecl,
    RowReport, TabInit, TabIo, TabReport, TabState, WhileOutranked, WorkspaceTabs, CLAUDE_CODE,
    GROK, OPENCODE, PRODUCT,
};
use made_app::session_follow::{RootSource, SessionFollowCoordinator};
use made_app::session_map::SessionActivity;
use made_app::ui::{
    about_integration_rows, new_tab_menu_entries, workspace_tab_summaries, TabSummary,
};
use made_app::workspace::{LaunchPlan, WorkspaceSource};
use made_app::{TerminalTarget, Workbench};
use made_terminal::PtyOptions;
use serde_json::{json, Value};
use tungstenite::{Message, WebSocket};

// ---- The fake ----

/// What the fake recorded, in order, in this process.
static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The directory the fakes that report one hand to the follow coordinator.
/// `None` releases it.
static FAKE_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

fn set_fake_directory(directory: Option<&Path>) {
    *FAKE_DIR.lock().unwrap() = directory.map(Path::to_path_buf);
}

fn record(event: impl Into<String>) {
    EVENTS.lock().unwrap().push(event.into());
}

fn events() -> Vec<String> {
    EVENTS.lock().unwrap().clone()
}

fn events_with(prefix: &str) -> Vec<String> {
    events()
        .into_iter()
        .filter(|event| event.starts_with(prefix))
        .collect()
}

const FAKE_BYTES: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";

static FAKE_DESCRIPTOR: Descriptor = Descriptor {
    key: "fake",
    program: "made-fake-agent",
    label: "Fake Agent",
    menu_item_id: "fake",
    scope_id: "fake-agent",
    scope_name: "Fake Agent",
    root_source_id: Some("fake"),
    mark: Mark::Image {
        uri: "bytes://fake.svg",
        bytes: FAKE_BYTES,
    },
    rank: Rank(250),
};

/// The same fake, ranked above Claude Code and below OpenCode.
static ABOVE_DESCRIPTOR: Descriptor = Descriptor {
    rank: Rank(150),
    ..FAKE_DESCRIPTOR
};

static SECOND_DESCRIPTOR: Descriptor = Descriptor {
    key: "second",
    program: "made-second-agent",
    label: "Second Agent",
    menu_item_id: "second",
    scope_id: "second-agent",
    scope_name: "Second Agent",
    root_source_id: Some("second"),
    mark: Mark::Image {
        uri: "bytes://second.svg",
        bytes: FAKE_BYTES,
    },
    rank: Rank(260),
};

/// A test-only integration.
struct FakeAgent {
    /// This fake as a list entry.
    this: &'static FakeAgent,
    descriptor: &'static Descriptor,
    status_key: &'static str,
    sentence_key: &'static str,
    marker: &'static str,
    /// Whether the tab state reports [`FAKE_DIR`] to the follow coordinator.
    reports_directory: bool,
}

static FAKE: FakeAgent = FakeAgent {
    this: &FAKE,
    descriptor: &FAKE_DESCRIPTOR,
    status_key: "fake status",
    sentence_key: "fake",
    marker: "fake_status",
    reports_directory: true,
};

static ABOVE: FakeAgent = FakeAgent {
    this: &ABOVE,
    descriptor: &ABOVE_DESCRIPTOR,
    status_key: "fake status",
    sentence_key: "fake",
    marker: "fake_status",
    reports_directory: true,
};

static SECOND: FakeAgent = FakeAgent {
    this: &SECOND,
    descriptor: &SECOND_DESCRIPTOR,
    status_key: "second status",
    sentence_key: "second",
    marker: "second_status",
    reports_directory: false,
};

const FAKE_ENV: &str = "FAKE_AGENT_VAR";
const FAKE_STRIP: &str = "FAKE_AGENT_STRIP";

impl Integration for FakeAgent {
    fn descriptor(&self) -> &'static Descriptor {
        self.descriptor
    }

    fn launch_args(&self) -> &'static [&'static str] {
        &["--fake"]
    }

    fn follow(&self) -> FollowKind {
        FollowKind::Directory(FollowPolicy {
            on_unusable: OnUnusable::Ignore,
            under_session_channel: WhileOutranked::Retain,
            under_higher_directory: WhileOutranked::Retain,
        })
    }

    fn env_policy(&self) -> EnvPolicy {
        EnvPolicy {
            shell: true,
            own_direct: true,
            strip: &[FAKE_STRIP],
        }
    }

    fn new_app_state(&self, _init: &AppInit<'_>) -> Option<Box<dyn AppState>> {
        record(format!("app-state:{}", self.descriptor.key));
        Some(Box::new(FakeApp {
            descriptor: self.descriptor,
            marker: self.marker,
        }))
    }

    fn new_tab_state(
        &self,
        target: TerminalTarget,
        init: &TabInit<'_>,
    ) -> Option<Box<dyn TabState>> {
        if init.remote {
            return None;
        }
        record(format!("tab-state:{}:{target:?}", self.descriptor.key));
        Some(Box::new(FakeTab {
            entry: self.this,
            reports_directory: self.reports_directory,
            row: None,
            held: false,
            reported: None,
            seen_root: None,
        }))
    }

    fn diagnostics(&self) -> DiagnosticsDecl {
        DiagnosticsDecl::Row(RowDecl {
            status_key: self.status_key,
            sentence_key: self.sentence_key,
            initial: RowReport::new(DiagnosticsPill::Inactive, "Fake Agent is not on PATH."),
        })
    }
}

struct FakeApp {
    descriptor: &'static Descriptor,
    marker: &'static str,
}

impl AppState for FakeApp {
    fn on_detection(&mut self, detected: bool, _ctx: &mut AppCtx) {
        record(format!("detected:{}:{detected}", self.descriptor.key));
    }

    fn publish(&self, workspace: &mut WorkspaceTabs<'_>) {
        let Some(entry) = workspace
            .list
            .iter()
            .copied()
            .find(|entry| entry.descriptor().key == self.descriptor.key)
        else {
            return;
        };
        if let Some(tab) = made_app::integrations::tab_state_mut::<FakeTab>(
            workspace.list,
            workspace.tab_states,
            entry,
        ) {
            tab.row = Some(RowReport::new(
                DiagnosticsPill::Ready,
                "Fake Agent session follow is ready.",
            ));
        }
    }

    fn smoke_marker(
        &self,
        _paths: Option<&made_platform::AppPaths>,
    ) -> Option<(&'static str, String)> {
        Some((self.marker, "ok".to_owned()))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

struct FakeTab {
    entry: IntegrationRef,
    reports_directory: bool,
    row: Option<RowReport>,
    /// The fake holds the explorer root, as the last poll saw it.
    held: bool,
    /// The directory the last poll reported.
    reported: Option<PathBuf>,
    /// The explorer root the last poll saw.
    seen_root: Option<PathBuf>,
}

impl FakeTab {
    fn key(&self) -> &'static str {
        self.entry.descriptor().key
    }
}

impl TabState for FakeTab {
    /// The core decides coverage from the declared policy and passes it in; the
    /// fake only supplies the variable of a covered launch.
    fn prepare_launch(
        &mut self,
        target: TerminalTarget,
        covered: bool,
        _ctx: &mut LaunchCtx<'_>,
    ) -> EnvContribution {
        record(format!(
            "prepare:{}:{target:?}:covered={covered}",
            self.key()
        ));
        EnvContribution {
            set: if covered {
                vec![(FAKE_ENV.into(), "1".into())]
            } else {
                Vec::new()
            },
            strip: Vec::new(),
        }
    }

    fn poll(&mut self, _now: Instant, io: &mut TabIo<'_>) -> TabReport {
        record(format!("poll:{}", self.key()));
        let root = io.follow.explorer_root().map(Path::to_path_buf);
        if root != self.seen_root {
            let name = root
                .as_deref()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            record(format!("saw:{}:{name}", self.key()));
            self.seen_root = root;
        }
        self.held = io.follow.root_held_by(self.entry).is_some();
        if !self.reports_directory {
            return TabReport::default();
        }
        let wanted = FAKE_DIR.lock().unwrap().clone();
        if wanted == self.reported {
            return TabReport::default();
        }
        self.reported.clone_from(&wanted);
        TabReport {
            directory: Some(wanted),
        }
    }

    fn teardown(&mut self) {
        record(format!("teardown:{}", self.key()));
    }

    fn row(&self, _remote: bool) -> Option<RowReport> {
        self.row.clone()
    }

    /// The fake brands a local tab while it holds the explorer root.
    fn presentation(&self, ctx: &PresentationCtx<'_>) -> Option<Presentation> {
        (self.held && !ctx.remote).then(|| Presentation {
            dots: vec![SessionActivity::Busy],
            title: Some("Fake session".to_owned()),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The product plus the fake at rank 250, after Claude Code.
const WITH_FAKE: IntegrationList = &[OPENCODE, GROK, CLAUDE_CODE, &FAKE];
/// The product plus the fake at rank 150, above Claude Code and Grok Build.
const WITH_ABOVE: IntegrationList = &[OPENCODE, GROK, CLAUDE_CODE, &ABOVE];
/// The product without Claude Code.
const WITHOUT_CLAUDE: IntegrationList = &[OPENCODE, GROK];
/// The product plus two fakes, to show polling order.
const PRODUCT_AND_TWO: IntegrationList = &[OPENCODE, GROK, CLAUDE_CODE, &FAKE, &SECOND];
/// A fake before Claude Code and one after it, to show polling order.
const FAKES_AROUND_CLAUDE: IntegrationList = &[OPENCODE, GROK, &FAKE, CLAUDE_CODE, &SECOND];
/// Two fakes, to show list order.
const TWO_FAKES: IntegrationList = &[&FAKE, &SECOND];

const FAKE_ENTRY: IntegrationRef = &FAKE;
const ABOVE_ENTRY: IntegrationRef = &ABOVE;

// ---- Isolation ----

const CHILD_ENV: &str = "MADE_FAKE_INTEGRATION_CHILD";

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Re-run `name` in a child process with an isolated home and fake programs
/// first on `PATH`. Returns `true` in the parent after the child passed.
fn isolated(name: &str) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        return false;
    }
    let home = tempfile::tempdir().unwrap();
    let tmp = home.path().join("tmp");
    let runtime = home.path().join("runtime");
    let bin = home.path().join("bin");
    for directory in [&tmp, &runtime, &bin] {
        fs::create_dir_all(directory).unwrap();
    }
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    // Programs that stay alive. The fake records its environment and arguments.
    for program in ["opencode", "grok"] {
        executable(&bin.join(program), "#!/bin/sh\nsleep 30\n");
    }
    // Claude Code records its process id and environment, then stays alive
    // until the test asks it to exit.
    executable(
        &bin.join("claude"),
        "#!/bin/sh\necho $$ > \"$HOME/claude-pid\"\nenv > \"$HOME/claude-env.txt\"\nwhile [ ! -e \"$HOME/claude-exit\" ]; do sleep 0.1; done\n",
    );
    // The shipped Claude Code hook, so a Claude Code tab binds a spool.
    let shipped = made_platform::install_root_from_current_executable(
        made_platform::PlatformFamily::current(),
    )
    .expect("test executable install root")
    .join("plugins")
    .join("claude-code-made")
    .join("bin");
    fs::create_dir_all(&shipped).unwrap();
    executable(&shipped.join("made-claude-status"), "#!/bin/sh\nexit 0\n");
    executable(
        &bin.join("made-fake-agent"),
        "#!/bin/sh\nenv > \"$HOME/fake-env.txt\"\necho \"$@\" > \"$HOME/fake-args.txt\"\nsleep 30\n",
    );
    let path = std::env::var_os("PATH").unwrap_or_default();
    let host = std::env::split_paths(&path).filter(|dir| {
        !dir.join("claude").exists() && !dir.join("grok").exists() && !dir.join("opencode").exists()
    });
    let paths = std::iter::once(bin).chain(host);
    let plugins = support::StagedPlugins::stage("fake-integration");
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env(CHILD_ENV, "1")
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env("TMPDIR", &tmp)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("SHELL", "/bin/sh")
        .env(FAKE_STRIP, "inherited")
        .env(
            made_platform::PLUGIN_DIR_ENV,
            plugins.package("opencode-made"),
        )
        .env_remove(made_bridge::config::CONFIG_CONTENT_ENV)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

fn context() -> egui::Context {
    egui::Context::default()
}

/// Run the model's frame logic until `done` holds.
fn drive(model: &mut AppModel, mut done: impl FnMut(&mut AppModel) -> bool) {
    let ctx = context();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        model.logic(&ctx);
        if done(model) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let milestones: Vec<String> = events()
        .into_iter()
        .filter(|event| !event.starts_with("poll:"))
        .collect();
    panic!("the condition did not hold in time; events: {milestones:?}");
}

fn empty_model(list: IntegrationList) -> AppModel {
    AppModel::from_launch(
        made_app::workspace::WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tuis([]),
    )
    .with_integrations(list)
}

/// A model over `list` with one running workspace whose terminal target is
/// `entry`, or a shell tab.
fn model_with_tab(list: IntegrationList, entry: Option<IntegrationRef>) -> AppModel {
    let mut model = empty_model(list);
    match entry {
        Some(entry) => model.begin_tui_workspace(entry),
        None => {
            let root = std::env::temp_dir();
            open_shell(&mut model, &root);
        }
    }
    drive(&mut model, |model| {
        model
            .workbench()
            .and_then(Workbench::terminal)
            .is_some_and(made_app::TerminalPane::is_running)
    });
    model
}

fn open_shell(model: &mut AppModel, root: &Path) -> made_app::WorkspaceId {
    model.open_plan(LaunchPlan::open(root, WorkspaceSource::Picker).unwrap());
    model.ensure_workbench();
    model.active_workspace_id().expect("active workspace")
}

/// Run the model's frame logic until the active tab's label is `label`.
fn drive_to_label(model: &mut AppModel, label: &str) {
    drive(model, |model| tab(model).label == label);
}

/// Run the model's frame logic until the active workbench's explorer root
/// is held by `source` at `root`.
fn drive_to_root(model: &mut AppModel, source: &str, root: &Path) {
    drive(model, |model| {
        let workbench = model.workbench().expect("a workbench");
        workbench.root_source() == source && workbench.explorer_root() == Some(root)
    });
}

/// A directory that stays until the test ends, canonical so it compares equal
/// to the root the coordinator reports.
fn directory(parent: &Path, name: &str) -> PathBuf {
    let path = parent.join(name);
    fs::create_dir_all(&path).unwrap();
    path.canonicalize().unwrap()
}

/// The text of a file the fake programs write under the home, once written.
fn home_file(name: &str) -> String {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(text) = fs::read_to_string(home.join(name)) {
            if !text.trim().is_empty() {
                return text;
            }
        }
        assert!(Instant::now() < deadline, "{name} was never written");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The terminal options of the active workbench.
fn options_of(model: &AppModel) -> &PtyOptions {
    model
        .workbench()
        .and_then(Workbench::terminal)
        .expect("a terminal")
        .options()
}

fn sets(options: &PtyOptions, name: &str) -> bool {
    options.env.iter().any(|(key, _)| key == name)
}

fn removes(options: &PtyOptions, name: &str) -> bool {
    options.env_remove.iter().any(|key| key == name)
}

/// A hook record of one Claude Code session in a direct Claude Code tab.
fn claude_record(session: &str, directory: &Path, pid: &str) -> String {
    format!(
        "{{\"at\":\"2026-01-01T00:00:00Z\",\"sessionId\":\"{session}\",\"cwd\":\"{}\",\"event\":\"SessionStart\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[{pid}]}}\n",
        directory.display()
    )
}

/// Make the Claude Code program of the isolated home report a session whose
/// directory is `directory`, and return once it did.
fn report_claude_session(directory: &Path) {
    let pid = home_file("claude-pid").trim().to_owned();
    let env = home_file("claude-env.txt");
    let spool = env
        .lines()
        .find_map(|line| line.strip_prefix("MADE_CLAUDE_SPOOL="))
        .expect("the Claude Code tab bound a spool")
        .to_owned();
    let mut file = fs::OpenOptions::new().append(true).open(spool).unwrap();
    file.write_all(claude_record("fake-test", directory, &pid).as_bytes())
        .unwrap();
}

/// Ask the Claude Code program of the isolated home to exit.
fn exit_claude() {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    fs::write(home.join("claude-exit"), "").unwrap();
}

/// A fake plugin connected to the bridge of the active OpenCode tab.
struct Plugin {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Plugin {
    /// Connect with the options the tab composed into its launch, report one
    /// active session in `directory` and keep answering pings.
    fn connect(model: &AppModel, directory: &Path) -> Self {
        let content = options_of(model)
            .env
            .iter()
            .find(|(key, _)| key == made_bridge::config::CONFIG_CONTENT_ENV)
            .and_then(|(_, value)| value.to_str().map(str::to_owned))
            .expect("the OpenCode tab composed its configuration");
        let config: Value = serde_json::from_str(&content).unwrap();
        let options = config["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|entry| entry.get("options")?.get("made").cloned())
            .expect("the composed configuration names the Made plugin");
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
        let send = |socket: &mut WebSocket<TcpStream>, value: Value| {
            socket
                .send(Message::Text(value.to_string().into()))
                .unwrap();
        };
        send(
            &mut socket,
            json!({"type":"hello", "protocolVersion":1, "token":options["token"], "paneId":options["paneId"], "pluginVersion":"test", "opencodeVersion":"2.1.4", "pid":123, "cwd":"/"}),
        );
        loop {
            if let Message::Text(text) = socket.read().unwrap() {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "welcome" {
                    break;
                }
            }
        }
        send(
            &mut socket,
            json!({"type":"state", "revision":1, "activeSessionID":"fake-test", "tabsEnabled":true,
            "sessions":[{"sessionID":"fake-test", "title":"Owned session", "active":true, "busy":false, "attention":false, "unread":false,
            "promptPulse":0, "renaming":false, "directory":directory, "subpath":null, "projectID":null}]}),
        );
        socket
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let mut socket = socket;
            while !flag.load(Ordering::SeqCst) {
                match socket.read() {
                    Ok(Message::Text(text)) => {
                        let Ok(value) = serde_json::from_str::<Value>(&text) else {
                            continue;
                        };
                        if value["type"] == "ping" {
                            let reply = json!({"type":"pong", "nonce":value["nonce"]});
                            if socket
                                .send(Message::Text(reply.to_string().into()))
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(error))
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(_) => return,
                }
            }
            let _ = socket.close(None);
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }

    /// Close the connection.
    fn disconnect(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn tab(model: &AppModel) -> TabSummary {
    let mut tabs = workspace_tab_summaries(model);
    assert_eq!(tabs.len(), 1);
    tabs.remove(0)
}

fn keys(list: IntegrationList) -> Vec<&'static str> {
    list.iter().map(|entry| entry.descriptor().key).collect()
}

/// The snapshot of a workbench no app model has published to yet.
fn unpublished_snapshot(list: IntegrationList) -> DiagnosticsSnapshot {
    let workbench = Workbench::start_in(
        list,
        None,
        None,
        std::env::temp_dir(),
        TerminalTarget::Shell,
        Instant::now(),
    );
    workbench.diagnostics_snapshot()
}

/// The snapshot of a workspace an app model published its rows to.
fn snapshot_over(list: IntegrationList) -> DiagnosticsSnapshot {
    let mut model = empty_model(list);
    let temp = tempfile::tempdir().unwrap();
    let id = open_shell(&mut model, temp.path());
    model.workspace(id).unwrap().diagnostics_snapshot()
}

// ---- Lists ----

#[test]
fn the_fake_lists_are_valid_and_the_product_list_has_no_fake() {
    for list in [WITH_FAKE, WITH_ABOVE, WITHOUT_CLAUDE, TWO_FAKES, PRODUCT] {
        assert_eq!(validate(list), Ok(()));
    }
    assert_eq!(keys(PRODUCT), ["opencode", "grok", "claude-code"]);
    assert!(PRODUCT.iter().all(|entry| *entry != FAKE_ENTRY));
}

#[test]
fn no_production_source_names_the_fake() {
    fn walk(dir: &Path, hits: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, hits);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = fs::read_to_string(&path).unwrap();
                if text.contains("made-fake-agent") || text.contains("fake_status") {
                    hits.push(path);
                }
            }
        }
    }
    let mut hits = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut hits,
    );
    assert!(
        hits.is_empty(),
        "production sources name the fake: {hits:?}"
    );
}

// ---- Menu ----

#[test]
fn menu_rows_follow_the_list_with_and_without_detection() {
    let model = |list, found: &[IntegrationRef]| {
        AppModel::from_launch(
            made_app::workspace::WorkspaceLaunch::waiting_for_picker(),
            AppOptions::new().with_compatible_tuis(found.iter().copied()),
        )
        .with_integrations(list)
    };
    // Detected: the fake's row appears, in list order, whatever order the
    // detection result had.
    let rows = new_tab_menu_entries(&model(WITH_FAKE, &[FAKE_ENTRY, CLAUDE_CODE, OPENCODE]));
    assert_eq!(rows, [OPENCODE, CLAUDE_CODE, FAKE_ENTRY]);
    // Not detected: no row.
    let rows = new_tab_menu_entries(&model(WITH_FAKE, &[OPENCODE, GROK, CLAUDE_CODE]));
    assert_eq!(rows, [OPENCODE, GROK, CLAUDE_CODE]);
    // Detected but not in the list: no row.
    let rows = new_tab_menu_entries(&model(PRODUCT, &[OPENCODE, FAKE_ENTRY]));
    assert_eq!(rows, [OPENCODE]);
    let descriptor = FAKE_ENTRY.descriptor();
    assert_eq!(
        (descriptor.label, descriptor.menu_item_id),
        ("Fake Agent", "fake")
    );
}

// ---- Detection ----

#[test]
fn detection_reaction_is_recorded_once_in_list_order() {
    if isolated("detection_reaction_is_recorded_once_in_list_order") {
        return;
    }
    let mut model = AppModel::from_launch(
        made_app::workspace::WorkspaceLaunch::waiting_for_picker(),
        AppOptions::new().with_compatible_tui_detection(),
    )
    .with_integrations(PRODUCT_AND_TWO);
    drive(&mut model, |model| !model.compatible_tuis().is_empty());
    // Another frame must not repeat the reaction.
    drive(&mut model, |_| true);
    // `made-fake-agent` is on PATH, `made-second-agent` is not. The product
    // programs are on PATH, so the product entries react in the same pass and
    // the fakes' reactions come in list order after them.
    assert_eq!(
        events_with("detected:"),
        ["detected:fake:true", "detected:second:false"]
    );
    assert_eq!(
        model.compatible_tuis(),
        [OPENCODE, GROK, CLAUDE_CODE, FAKE_ENTRY]
    );
    assert_eq!(new_tab_menu_entries(&model), model.compatible_tuis());
}

// ---- Launch ----

#[test]
fn launch_arguments_come_from_the_list_entry() {
    let temp = tempfile::tempdir().unwrap();
    executable(&temp.path().join("made-fake-agent"), "#!/bin/sh\n");
    let bin = std::env::join_paths([temp.path()]).unwrap();
    let options = TerminalTarget::Tui(FAKE_ENTRY)
        .pty_options(temp.path(), &bin)
        .expect("the program is on PATH");
    assert_eq!(options.args, ["--fake"]);
    assert!(TerminalTarget::Shell
        .pty_options(temp.path(), &bin)
        .unwrap()
        .args
        .is_empty());
}

#[test]
fn launch_variables_follow_the_declared_policy_on_every_local_target() {
    if isolated("launch_variables_follow_the_declared_policy_on_every_local_target") {
        return;
    }
    // The fake's own tab: the program receives the variable and the argument.
    let fake = model_with_tab(WITH_FAKE, Some(FAKE_ENTRY));
    assert!(home_file("fake-env.txt")
        .lines()
        .any(|line| line == "FAKE_AGENT_VAR=1"));
    assert_eq!(home_file("fake-args.txt").trim(), "--fake");
    assert!(sets(options_of(&fake), FAKE_ENV));
    drop(fake);

    // A shell tab: the fake declares `shell: true`.
    let shell = model_with_tab(WITH_FAKE, None);
    assert!(sets(options_of(&shell), FAKE_ENV));
    drop(shell);

    // The other integrations' tabs are not covered: no variable, and the core
    // removes the variable the policy strips.
    for entry in [OPENCODE, GROK] {
        let other = model_with_tab(WITH_FAKE, Some(entry));
        let options = options_of(&other);
        assert!(!sets(options, FAKE_ENV), "{}", entry.descriptor().key);
        assert!(removes(options, FAKE_STRIP), "{}", entry.descriptor().key);
        drop(other);
    }

    let prepared = events_with("prepare:");
    for expected in [
        "prepare:fake:Tui(Integration(\"fake\")):covered=true",
        "prepare:fake:Shell:covered=true",
        "prepare:fake:Tui(Integration(\"opencode\")):covered=false",
        "prepare:fake:Tui(Integration(\"grok\")):covered=false",
    ] {
        assert!(
            prepared.contains(&expected.to_owned()),
            "{expected}: {prepared:?}"
        );
    }
}

#[test]
fn contributions_apply_last_entry_first_so_the_earliest_entry_wins() {
    let contribution = |value: &str| EnvContribution {
        set: vec![("SHARED".into(), value.into())],
        strip: Vec::new(),
    };
    let options = apply_contributions(
        &[contribution("first"), contribution("second")],
        PtyOptions::new("/"),
    );
    let last = options
        .env
        .iter()
        .rev()
        .find(|(key, _)| key == "SHARED")
        .map(|(_, value)| value.to_string_lossy().into_owned());
    assert_eq!(last.as_deref(), Some("first"));
}

// ---- State, polling and teardown ----

#[test]
fn tab_state_is_created_polled_in_list_order_and_torn_down_once() {
    if isolated("tab_state_is_created_polled_in_list_order_and_torn_down_once") {
        return;
    }
    let mut model = empty_model(PRODUCT_AND_TWO);
    let temp = tempfile::tempdir().unwrap();
    let id = open_shell(&mut model, temp.path());
    assert_eq!(
        events_with("tab-state:"),
        ["tab-state:fake:Shell", "tab-state:second:Shell"]
    );
    drive(&mut model, |_| events_with("poll:").len() >= 4);
    let polls = events_with("poll:");
    assert!(
        polls
            .chunks(2)
            .all(|pair| pair == ["poll:fake", "poll:second"]),
        "{polls:?}"
    );
    assert!(events_with("teardown:").is_empty());
    assert!(model.close_workspace(id));
    assert_eq!(
        events_with("teardown:"),
        ["teardown:fake", "teardown:second"]
    );
}

#[test]
fn a_fake_polls_after_the_product_entries_that_precede_it_in_the_list() {
    if isolated("a_fake_polls_after_the_product_entries_that_precede_it_in_the_list") {
        return;
    }
    // [OpenCode, Grok Build, fake, Claude Code, second]: both fakes watch the
    // explorer root. The directory Claude Code reports is applied before the
    // entry after it polls, so the second fake sees it in the same pass and
    // the fake before Claude Code only in the next.
    let temp = tempfile::tempdir().unwrap();
    let claude_dir = directory(temp.path(), "claude-dir");
    let mut model = model_with_tab(FAKES_AROUND_CLAUDE, Some(CLAUDE_CODE));
    report_claude_session(&claude_dir);
    drive_to_root(&mut model, "claude-code", &claude_dir);
    // One more pass lets the fake before Claude Code see the new root too.
    drive(&mut model, |_| {
        events_with("saw:")
            .iter()
            .filter(|event| event.ends_with(":claude-dir"))
            .count()
            == 2
    });
    let saw: Vec<String> = events_with("saw:")
        .into_iter()
        .filter(|event| event.ends_with(":claude-dir"))
        .collect();
    assert_eq!(saw, ["saw:second:claude-dir", "saw:fake:claude-dir"]);
}

// ---- Follow ----

fn follow_sources(list: IntegrationList, order: &[(IntegrationRef, bool)]) -> Vec<String> {
    let temp = tempfile::tempdir().unwrap();
    let mut coordinator = SessionFollowCoordinator::new(temp.path(), temp.path())
        .unwrap()
        .with_integrations(list);
    let mut seen = Vec::new();
    for (entry, hold) in order {
        let directory = temp.path().join(entry.descriptor().key);
        fs::create_dir_all(&directory).unwrap();
        coordinator.follow_directory(*entry, hold.then_some(directory.as_path()));
        seen.push(source_name(coordinator.effective_root_source()));
    }
    seen
}

fn source_name(source: RootSource) -> String {
    match source {
        RootSource::Shell => "shell".to_owned(),
        RootSource::Agent(entry) => entry.descriptor().root_source_id.unwrap().to_owned(),
    }
}

#[test]
fn a_fake_at_rank_250_yields_to_claude_code_and_outranks_grok() {
    let seen = follow_sources(
        WITH_FAKE,
        &[
            (GROK, true),
            (FAKE_ENTRY, true),
            (CLAUDE_CODE, true),
            (CLAUDE_CODE, false),
            (FAKE_ENTRY, false),
            (GROK, false),
        ],
    );
    assert_eq!(
        seen,
        ["grok", "fake", "claude-code", "fake", "grok", "shell"]
    );
}

#[test]
fn a_fake_at_rank_150_owns_the_root_over_claude_code_and_grok() {
    let seen = follow_sources(
        WITH_ABOVE,
        &[
            (GROK, true),
            (CLAUDE_CODE, true),
            (ABOVE_ENTRY, true),
            (ABOVE_ENTRY, false),
            (CLAUDE_CODE, false),
            (GROK, false),
        ],
    );
    // Holders release in rank order; the product's relative order is
    // unchanged: Claude Code over Grok Build.
    assert_eq!(
        seen,
        [
            "grok",
            "claude-code",
            "fake",
            "claude-code",
            "grok",
            "shell"
        ]
    );
}

// ---- Presentation ----

#[test]
fn the_fake_holds_the_root_through_a_real_tab_until_opencode_connects() {
    if isolated("the_fake_holds_the_root_through_a_real_tab_until_opencode_connects") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let fake_dir = directory(temp.path(), "fake-dir");
    let opencode_dir = directory(temp.path(), "opencode-dir");
    set_fake_directory(Some(&fake_dir));
    let mut model = model_with_tab(WITH_FAKE, Some(OPENCODE));
    // The fake's tab state reported a directory: the workbench's coordinator
    // was built over the injected list, so the fake holds the root.
    drive_to_root(&mut model, "fake", &fake_dir);

    // OpenCode connects with an active session: the session channel outranks
    // the fake.
    let plugin = Plugin::connect(&model, &opencode_dir);
    drive_to_root(&mut model, "opencode", &opencode_dir);

    // It disconnects: the fake keeps the directory it reported.
    plugin.disconnect();
    drive_to_root(&mut model, "fake", &fake_dir);
}

#[test]
fn a_fake_at_rank_250_brands_a_tab_over_grok_and_yields_to_opencode_and_claude() {
    if isolated("a_fake_at_rank_250_brands_a_tab_over_grok_and_yields_to_opencode_and_claude") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let fake_dir = directory(temp.path(), "fake-dir");
    let claude_dir = directory(temp.path(), "claude-dir");

    // Grok Build runs in the tab; the fake reports nothing: Grok Build brands it.
    set_fake_directory(None);
    let mut model = model_with_tab(WITH_FAKE, Some(GROK));
    assert_eq!(tab(&model).label, "Grok Build");

    // The fake reports a directory and holds the root: rank 250 outranks
    // Grok Build at 300.
    set_fake_directory(Some(&fake_dir));
    drive_to_label(&mut model, "Fake Agent");
    let summary = tab(&model);
    assert_eq!(summary.mark, Some(FAKE_DESCRIPTOR.mark));
    assert_eq!(summary.dots, [SessionActivity::Busy]);
    assert_eq!(
        summary.tooltip.as_deref(),
        Some("Fake session — Sessions: 1 running")
    );
    assert!(summary.accessible_name.contains("Fake Agent"));

    // OpenCode connected (its label held here) outranks the fake.
    let id = model.active_workspace_id().unwrap();
    model.hold_workspace_opencode_label(id);
    assert_eq!(tab(&model).label, "OpenCode");
    drop(model);

    // Claude Code live outranks the fake at 250 and takes the root.
    let mut model = model_with_tab(WITH_FAKE, Some(CLAUDE_CODE));
    drive_to_root(&mut model, "fake", &fake_dir);
    assert_eq!(
        tab(&model).label,
        "Claude Code",
        "present Claude Code brands"
    );
    report_claude_session(&claude_dir);
    drive_to_root(&mut model, "claude-code", &claude_dir);
    assert_eq!(tab(&model).label, "Claude Code");

    // Claude Code exits: the root returns to the fake and it brands the tab
    // again.
    exit_claude();
    drive_to_root(&mut model, "fake", &fake_dir);
    drive_to_label(&mut model, "Fake Agent");
    assert_eq!(tab(&model).mark, Some(FAKE_DESCRIPTOR.mark));
    set_fake_directory(None);
}

#[test]
fn a_fake_at_rank_150_brands_a_tab_over_claude_code() {
    if isolated("a_fake_at_rank_150_brands_a_tab_over_claude_code") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let fake_dir = directory(temp.path(), "fake-dir");
    set_fake_directory(Some(&fake_dir));
    let mut model = model_with_tab(WITH_ABOVE, Some(CLAUDE_CODE));
    drive_to_label(&mut model, "Fake Agent");
    assert_eq!(tab(&model).mark, Some(ABOVE_DESCRIPTOR.mark));
    let id = model.active_workspace_id().unwrap();
    model.hold_workspace_opencode_label(id);
    assert_eq!(tab(&model).label, "OpenCode", "OpenCode keeps rank 100");
    drop(model);
    set_fake_directory(None);
    let model = model_with_tab(WITH_ABOVE, Some(CLAUDE_CODE));
    assert_eq!(tab(&model).label, "Claude Code");
}

// ---- Diagnostics, About, report and smoke markers ----

#[test]
fn rows_about_report_and_markers_follow_the_list() {
    if isolated("rows_about_report_and_markers_follow_the_list") {
        return;
    }
    // Rows before detection: the fake's declared initial row, after claude.
    let snapshot = unpublished_snapshot(WITH_FAKE);
    let status_keys: Vec<_> = snapshot.rows.iter().map(|row| row.status_key).collect();
    assert_eq!(status_keys, ["grok status", "claude status", "fake status"]);
    let fake = snapshot.row("fake status").unwrap();
    assert_eq!(
        (fake.pill, fake.sentence.as_str()),
        (DiagnosticsPill::Inactive, "Fake Agent is not on PATH.")
    );
    let about: Vec<_> = about_integration_rows(&snapshot)
        .into_iter()
        .map(|(_, sentence)| sentence.to_owned())
        .collect();
    assert_eq!(about.len(), 3);
    assert_eq!(about[2], "Fake Agent is not on PATH.");
    let report = snapshot.to_text();
    let position = |needle: &str| report.find(needle).unwrap_or_else(|| panic!("{needle}"));
    assert!(position("grok status:") < position("claude status:"));
    assert!(
        position("claude status:")
            < position("fake status: inactive\nfake: Fake Agent is not on PATH.\n")
    );

    // After the app-wide state published its row.
    let mut model = empty_model(WITH_FAKE);
    let temp = tempfile::tempdir().unwrap();
    let id = open_shell(&mut model, temp.path());
    drive(&mut model, |model| {
        model
            .workspace(id)
            .unwrap()
            .diagnostics_snapshot()
            .row("fake status")
            .is_some_and(|row| row.pill == DiagnosticsPill::Ready)
    });

    // Smoke markers: the fake's marker follows `grok_status=`.
    let line = model.smoke_layout_markers();
    let at = |needle: &str| {
        line.find(needle)
            .unwrap_or_else(|| panic!("{needle} in {line}"))
    };
    assert!(at("opencode_plugin=") < at("grok_status="));
    assert!(at("grok_status=") < at("fake_status=ok "));
    assert!(at("fake_status=ok ") < at("companions="));
}

// ---- List variation ----

#[test]
fn menu_and_launch_output_is_exactly_the_lists_entries() {
    let temp = tempfile::tempdir().unwrap();
    for program in ["opencode", "grok", "claude", "made-fake-agent"] {
        executable(&temp.path().join(program), "#!/bin/sh\n");
    }
    let bin = std::env::join_paths([temp.path()]).unwrap();
    for list in [PRODUCT, WITH_FAKE, WITH_ABOVE, WITHOUT_CLAUDE] {
        let model = AppModel::from_launch(
            made_app::workspace::WorkspaceLaunch::waiting_for_picker(),
            AppOptions::new().with_compatible_tuis(PRODUCT.iter().copied().chain([FAKE_ENTRY])),
        )
        .with_integrations(list);
        assert_eq!(new_tab_menu_entries(&model), list, "menu");
        let launched: Vec<_> = list
            .iter()
            .map(|entry| {
                let options = TerminalTarget::Tui(*entry)
                    .pty_options(temp.path(), &bin)
                    .unwrap();
                (entry.descriptor().key, options.args.clone())
            })
            .collect();
        let expected: Vec<_> = list
            .iter()
            .map(|entry| {
                (
                    entry.descriptor().key,
                    entry
                        .launch_args()
                        .iter()
                        .map(|arg| (*arg).into())
                        .collect::<Vec<std::ffi::OsString>>(),
                )
            })
            .collect();
        assert_eq!(launched, expected, "launch");
    }
}

#[test]
fn launch_state_exists_only_for_the_lists_entries() {
    if isolated("launch_state_exists_only_for_the_lists_entries") {
        return;
    }
    let _product = model_with_tab(PRODUCT, None);
    assert!(events_with("tab-state:").is_empty());
    let _with_fake = model_with_tab(WITH_FAKE, None);
    assert_eq!(events_with("tab-state:"), ["tab-state:fake:Shell"]);
    assert_eq!(events_with("prepare:"), ["prepare:fake:Shell:covered=true"]);
}

#[test]
fn follow_and_presentation_output_is_exactly_the_lists_entries() {
    // Follow: Claude Code is not a holder of a list without it, so its report
    // changes nothing.
    let seen = follow_sources(WITHOUT_CLAUDE, &[(GROK, true), (CLAUDE_CODE, true)]);
    assert_eq!(seen, ["grok", "grok"]);
    let seen = follow_sources(PRODUCT, &[(GROK, true), (CLAUDE_CODE, true)]);
    assert_eq!(seen, ["grok", "claude-code"]);
    let seen = follow_sources(PRODUCT, &[(FAKE_ENTRY, true)]);
    assert_eq!(seen, ["shell"], "the product list has no fake holder");
}

#[test]
fn presentation_is_exactly_the_lists_entries() {
    if isolated("presentation_is_exactly_the_lists_entries") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    set_fake_directory(Some(&directory(temp.path(), "fake-dir")));
    // The fake is not in the product list: a fake with a directory brands
    // nothing.
    let product = model_with_tab(PRODUCT, Some(GROK));
    assert_eq!(tab(&product).label, "Grok Build");
    drop(product);
    // A list without Claude Code: a Claude Code tab is not branded as it.
    let without = model_with_tab(WITHOUT_CLAUDE, None);
    assert_ne!(tab(&without).label, "Claude Code");
    set_fake_directory(None);
}

#[test]
fn rows_about_report_and_markers_are_exactly_the_lists_entries() {
    if isolated("rows_about_report_and_markers_are_exactly_the_lists_entries") {
        return;
    }
    let declared = |list: IntegrationList| -> Vec<&'static str> {
        list.iter()
            .filter_map(|entry| match entry.diagnostics() {
                DiagnosticsDecl::Row(decl) => Some(decl.status_key),
                _ => None,
            })
            .collect()
    };
    for list in [PRODUCT, WITH_FAKE, WITH_ABOVE, WITHOUT_CLAUDE] {
        let snapshot = snapshot_over(list);
        let rows: Vec<_> = snapshot.rows.iter().map(|row| row.status_key).collect();
        assert_eq!(rows, declared(list));
        assert_eq!(about_integration_rows(&snapshot).len(), rows.len());
        let text = snapshot.to_text();
        for key in ["grok status", "claude status", "fake status"] {
            assert_eq!(
                text.contains(&format!("{key}:")),
                rows.contains(&key),
                "{key}"
            );
        }
        let model = empty_model(list);
        let line = model.smoke_layout_markers();
        assert_eq!(line.contains("opencode_plugin="), list.contains(&OPENCODE));
        assert_eq!(line.contains("grok_status="), list.contains(&GROK));
        assert!(
            !line.contains("claude"),
            "Claude Code has no marker: {line}"
        );
        assert_eq!(
            line.contains("fake_status="),
            list.iter().any(|entry| entry.descriptor().key == "fake")
        );
    }
    // The product line keeps its fixed template.
    let line = empty_model(PRODUCT).smoke_layout_markers();
    let version = line.split(' ').next().unwrap();
    assert!(version.starts_with("version="));
    let fields: Vec<_> = line
        .split(' ')
        .map(|field| field.split('=').next().unwrap())
        .collect();
    assert_eq!(
        fields,
        [
            "version",
            "opencode_plugin",
            "grok_status",
            "companions",
            "askpass",
            "updates",
            "update_repo",
            "update_seam"
        ]
    );
}
