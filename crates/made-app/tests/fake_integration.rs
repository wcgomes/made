//! A fake integration defined only in test code, plugged in through the same
//! interface and an injected list (AIA-03, AIA-04, AIA-09).
//!
//! `FakeAgent` appears in every seam the requirement lists without a
//! production code edit: menu rows, detection reaction, launch arguments and
//! variables, app-wide and per-tab state, polling and teardown, follow ranking,
//! tab presentation, diagnostics rows, About rows, copied report lines and
//! smoke markers. A second variant ranks above Claude Code. The list-variation
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
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use eframe::egui;
use made_app::app::{AppModel, AppOptions};
use made_app::diagnostics::{DiagnosticsPill, DiagnosticsSnapshot};
use made_app::integrations::{
    apply_contributions, validate, AppCtx, AppInit, AppState, Descriptor, DiagnosticsDecl,
    EnvContribution, EnvPolicy, FollowKind, FollowPolicy, Integration, IntegrationList,
    IntegrationRef, LaunchCtx, Mark, OnUnusable, Presentation, PresentationCtx, Rank, RowDecl,
    RowReport, TabInit, TabState, WhileOutranked, WorkspaceTabs, CLAUDE_CODE, GROK, OPENCODE,
    PRODUCT,
};
use made_app::session_follow::{RootSource, SessionFollowCoordinator};
use made_app::session_map::SessionActivity;
use made_app::ui::{
    about_integration_rows, new_tab_menu_entries, workspace_tab_summaries, TabSummary,
};
use made_app::workspace::{LaunchPlan, WorkspaceSource};
use made_app::{TerminalTarget, Workbench};
use made_terminal::PtyOptions;

// ---- The fake ----

/// What the fake recorded, in order, in this process.
static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// While set, the fake brands every local tab.
static ACTIVE: AtomicBool = AtomicBool::new(false);

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
    descriptor: &'static Descriptor,
    status_key: &'static str,
    sentence_key: &'static str,
    marker: &'static str,
}

static FAKE: FakeAgent = FakeAgent {
    descriptor: &FAKE_DESCRIPTOR,
    status_key: "fake status",
    sentence_key: "fake",
    marker: "fake_status",
};

static ABOVE: FakeAgent = FakeAgent {
    descriptor: &ABOVE_DESCRIPTOR,
    status_key: "fake status",
    sentence_key: "fake",
    marker: "fake_status",
};

static SECOND: FakeAgent = FakeAgent {
    descriptor: &SECOND_DESCRIPTOR,
    status_key: "second status",
    sentence_key: "second",
    marker: "second_status",
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
            shell: false,
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
            descriptor: self.descriptor,
            row: None,
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
    descriptor: &'static Descriptor,
    row: Option<RowReport>,
}

impl TabState for FakeTab {
    fn prepare_launch(
        &mut self,
        target: TerminalTarget,
        _ctx: &mut LaunchCtx<'_>,
    ) -> EnvContribution {
        let covered = matches!(
            target,
            TerminalTarget::Tui(entry) if entry.descriptor().key == self.descriptor.key
        );
        record(format!(
            "prepare:{}:{target:?}:covered={covered}",
            self.descriptor.key
        ));
        EnvContribution {
            set: if covered {
                vec![(FAKE_ENV.into(), "1".into())]
            } else {
                Vec::new()
            },
            strip: vec![FAKE_STRIP],
        }
    }

    fn poll(
        &mut self,
        _now: Instant,
        _io: &mut made_app::integrations::TabIo<'_>,
    ) -> made_app::integrations::TabReport {
        record(format!("poll:{}", self.descriptor.key));
        made_app::integrations::TabReport::default()
    }

    fn teardown(&mut self) {
        record(format!("teardown:{}", self.descriptor.key));
    }

    fn row(&self, _remote: bool) -> Option<RowReport> {
        self.row.clone()
    }

    fn presentation(&self, ctx: &PresentationCtx<'_>) -> Option<Presentation> {
        (ACTIVE.load(Ordering::SeqCst) && !ctx.remote).then(|| Presentation {
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
    for program in ["opencode", "grok", "claude"] {
        executable(&bin.join(program), "#!/bin/sh\nsleep 30\n");
    }
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
    panic!("the condition did not hold in time; events: {:?}", events());
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
    .with_integrations(TWO_FAKES);
    drive(&mut model, |model| !model.compatible_tuis().is_empty());
    // Another frame must not repeat the reaction.
    drive(&mut model, |_| true);
    // `made-fake-agent` is on PATH, `made-second-agent` is not.
    assert_eq!(
        events_with("detected:"),
        ["detected:fake:true", "detected:second:false"]
    );
    assert_eq!(model.compatible_tuis(), [FAKE_ENTRY]);
}

// ---- Launch ----

#[test]
fn launch_arguments_come_from_the_list_entry() {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let temp = tempfile::tempdir().unwrap();
    executable(&temp.path().join("made-fake-agent"), "#!/bin/sh\n");
    let bin = std::env::join_paths([temp.path()]).unwrap();
    let _ = path;
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
fn launch_variables_apply_to_the_fakes_own_target_and_not_to_the_shell() {
    if isolated("launch_variables_apply_to_the_fakes_own_target_and_not_to_the_shell") {
        return;
    }
    let model = model_with_tab(WITH_FAKE, Some(FAKE_ENTRY));
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let env = loop {
        if let Ok(text) = fs::read_to_string(home.join("fake-env.txt")) {
            if !text.is_empty() {
                break text;
            }
        }
        assert!(Instant::now() < deadline, "the fake program never ran");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(env.lines().any(|line| line == "FAKE_AGENT_VAR=1"));
    assert!(
        env.lines()
            .all(|line| !line.starts_with("FAKE_AGENT_STRIP=")),
        "the inherited variable the fake strips reached the child"
    );
    assert_eq!(
        fs::read_to_string(home.join("fake-args.txt"))
            .unwrap()
            .trim(),
        "--fake"
    );
    assert!(events().contains(&"prepare:fake:Tui(Integration(\"fake\")):covered=true".to_owned()));
    drop(model);

    let _shell = model_with_tab(WITH_FAKE, None);
    assert!(events().contains(&"prepare:fake:Shell:covered=false".to_owned()));
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
fn a_fake_at_rank_250_brands_a_tab_over_grok_and_yields_to_opencode_and_claude() {
    if isolated("a_fake_at_rank_250_brands_a_tab_over_grok_and_yields_to_opencode_and_claude") {
        return;
    }
    // Grok Build runs in the tab; the fake is inactive: Grok Build brands it.
    ACTIVE.store(false, Ordering::SeqCst);
    let mut model = model_with_tab(WITH_FAKE, Some(GROK));
    assert_eq!(tab(&model).label, "Grok Build");

    // The fake becomes active: rank 250 outranks Grok Build at 300.
    ACTIVE.store(true, Ordering::SeqCst);
    let summary = tab(&model);
    assert_eq!(summary.label, "Fake Agent");
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

    // Claude Code live outranks the fake at 250.
    let model = model_with_tab(WITH_FAKE, Some(CLAUDE_CODE));
    assert_eq!(tab(&model).label, "Claude Code");
    // Claude Code exits: the fake brands the tab again.
    drop(model);
    ACTIVE.store(false, Ordering::SeqCst);
}

#[test]
fn a_fake_at_rank_150_brands_a_tab_over_claude_code() {
    if isolated("a_fake_at_rank_150_brands_a_tab_over_claude_code") {
        return;
    }
    ACTIVE.store(true, Ordering::SeqCst);
    let mut model = model_with_tab(WITH_ABOVE, Some(CLAUDE_CODE));
    assert_eq!(tab(&model).label, "Fake Agent");
    assert_eq!(tab(&model).mark, Some(ABOVE_DESCRIPTOR.mark));
    let id = model.active_workspace_id().unwrap();
    model.hold_workspace_opencode_label(id);
    assert_eq!(tab(&model).label, "OpenCode", "OpenCode keeps rank 100");
    ACTIVE.store(false, Ordering::SeqCst);
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
    assert_eq!(
        events_with("prepare:"),
        ["prepare:fake:Shell:covered=false"]
    );
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
    ACTIVE.store(true, Ordering::SeqCst);
    // The fake is not in the product list: an active fake brands nothing.
    let product = model_with_tab(PRODUCT, Some(GROK));
    assert_eq!(tab(&product).label, "Grok Build");
    drop(product);
    // A list without Claude Code: a Claude Code tab is not branded as it.
    let without = model_with_tab(WITHOUT_CLAUDE, None);
    assert_ne!(tab(&without).label, "Claude Code");
    ACTIVE.store(false, Ordering::SeqCst);
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
