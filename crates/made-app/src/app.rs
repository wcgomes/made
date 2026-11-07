//! Application model and eframe composition.
//!
//! [`AppModel`] owns the launch state, the folder-picker transition, and the
//! workbench runtime. It is deliberately usable without a display for the UI
//! model test: only [`AppModel::logic`] starts the runtime, while
//! [`AppModel::ui`] renders whichever surface the launch state selects.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use made_platform::native_backdrop::NativeBackdropStatus;
use made_platform::Platform;

use crate::cli::LaunchArgs;

use crate::integrations::{
    AppCtx, AppInit, AppState, IntegrationList, IntegrationRef, Slots, OPENCODE, PRODUCT,
};
use crate::session_follow::validate_remote_directory;
use crate::ssh_servers::{PrivateMetadataStorage, ProbeAttempt, ServerMetadata, ServerRegistry};
use crate::theme;
use crate::tui::TerminalTarget;
use crate::ui;
use crate::updates::{UpdateAction, UpdatePanel, UpdateScheduler};
use crate::workbench::{platform_family_label, Workbench};
use crate::workspace::{
    LaunchPlan, WorkspaceError, WorkspaceLaunch, WorkspaceSelection, WorkspaceSource,
};
use crate::{APP_NAME, APP_VERSION};

/// Native window options shared by the binary and tests.
///
/// `clipboard_shortcuts` must stay disabled: egui-winit otherwise consumes
/// Ctrl/Cmd+C/X/V before the terminal input encoder sees them.
pub fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Made")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 480.0])
            .with_transparent(theme::TRANSLUCENT_WINDOW),
        clipboard_shortcuts: false,
        wgpu_options: wgpu_options(),
        ..Default::default()
    }
}

/// GPU surface options.
///
/// Where the platform needs a composition swapchain for per-pixel alpha
/// ([`made_platform::native_backdrop::requires_composition_swapchain`], Windows)
/// the DX12 backend with a DirectComposition swapchain is requested; Vulkan is
/// not guaranteed to expose a transparent surface there. Explicit `WGPU_BACKEND`
/// and `WGPU_DX12_PRESENTATION_SYSTEM` overrides still win.
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
    use eframe::wgpu::{Backends, Dx12SwapchainKind};

    if !made_platform::native_backdrop::requires_composition_swapchain() {
        return WgpuConfiguration::default();
    }
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    if Backends::from_env().is_none() {
        // Vulkan is intentionally dropped here: DX12 first, GL as the fallback.
        setup.instance_descriptor.backends = Backends::DX12 | Backends::GL;
    }
    setup
        .instance_descriptor
        .backend_options
        .dx12
        .presentation_system =
        Dx12SwapchainKind::from_env().unwrap_or(Dx12SwapchainKind::DxgiFromVisual);
    WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..Default::default()
    }
}

/// Runtime options that are not part of the workspace selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppOptions {
    /// Headless CI smoke mode: create the real window, pump frames, then exit.
    pub smoke_test: Option<Duration>,
    /// Legacy process-start instant retained for callers that construct
    /// [`AppOptions`] directly. Per-workspace startup is measured from the
    /// moment that workspace's preparation is scheduled, so a tab opened much
    /// later never inherits the application's launch age.
    pub started_at: Instant,
    /// Compatible TUIs offered by the `+` new-tab menu.
    ///
    /// The default is deliberately empty so tests and direct callers cannot
    /// launch a real TUI by accident. The binary enables a background `PATH`
    /// lookup with [`AppOptions::with_compatible_tui_detection`]. Tests that
    /// opt in must isolate `PATH`.
    pub compatible_tuis: Vec<IntegrationRef>,
    /// Whether to detect compatible TUIs on a background thread at startup.
    ///
    /// Detection reads executable metadata only. The window, shell, and
    /// explorer never wait for it.
    pub detect_compatible_tuis: bool,
}

impl AppOptions {
    /// Default options with no smoke run and no detected TUIs.
    pub fn new() -> Self {
        Self {
            smoke_test: None,
            started_at: Instant::now(),
            compatible_tuis: Vec::new(),
            detect_compatible_tuis: false,
        }
    }

    /// Enable the smoke run.
    pub fn with_smoke(mut self, duration: Duration) -> Self {
        self.smoke_test = Some(duration);
        self
    }

    /// Set the compatible TUIs offered by the `+` menu.
    ///
    /// An explicit list disables background detection.
    pub fn with_compatible_tuis(mut self, tuis: impl IntoIterator<Item = IntegrationRef>) -> Self {
        self.compatible_tuis = tuis.into_iter().collect();
        self.detect_compatible_tuis = false;
        self
    }

    /// Detect compatible TUIs on a background thread started at launch.
    ///
    /// This clears any explicit list; the last builder call wins. The lookup
    /// stays metadata-only. When it reports grok, a separate worker may install
    /// the shipped plugin. An explicit catalog never does.
    pub fn with_compatible_tui_detection(mut self) -> Self {
        self.compatible_tuis.clear();
        self.detect_compatible_tuis = true;
        self
    }
}

impl Default for AppOptions {
    fn default() -> Self {
        Self::new()
    }
}

// BEGIN remote-path (AIA-02)
/// The owning remote tab's Grok Build row from resolved per-server state plus
/// the tab's own remote state. The host detection phase never applies.
fn remote_grok_row_for(
    consented: bool,
    install: Option<crate::diagnostics::RemoteGrokInstallOutcome>,
    workbench: &Workbench,
) -> crate::integrations::grok::diagnostics::GrokDiagnosticsRow {
    let report = crate::diagnostics::remote_grok_row(
        workbench.remote_transport_failed(),
        workbench.remote_attached(),
        workbench.remote_failure_detail(),
        workbench.remote_grok_detected(),
        install,
        consented,
        workbench.remote_grok_running(),
    );
    crate::integrations::grok::diagnostics::GrokDiagnosticsRow {
        pill: report.pill,
        sentence: report.sentence.into(),
    }
}
// END remote-path

/// Recorded outcome of one server's consented remote Grok plugin install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteGrokInstallState {
    /// The bounded install attempt is still running.
    Running,
    /// The official installer reported installed or already-current.
    Succeeded,
    /// The attempt failed with the companion's bounded machine reason.
    Failed { reason: Option<&'static str> },
}

/// Test-only hold for one server's remote Grok install outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteGrokInstallHold {
    /// The bounded attempt is still running.
    Running,
    /// The official installer reported installed or already-current.
    Succeeded,
    /// The attempt failed; `reason` is the companion's bounded machine reason.
    Failed { reason: Option<&'static str> },
}

/// Map the companion's bounded install reasons to the ones diagnostics name.
/// Unknown or absent reasons degrade to the generic failed row.
fn known_install_reason(reason: &str) -> Option<&'static str> {
    match reason {
        "grok-missing" => Some("grok-missing"),
        "staged-plugin-missing" => Some("staged-plugin-missing"),
        "install-failed" => Some("install-failed"),
        "timeout" => Some("timeout"),
        "lock-unavailable" => Some("lock-unavailable"),
        "lock-timeout" => Some("lock-timeout"),
        _ => None,
    }
}

impl RemoteGrokInstallState {
    /// Diagnostics outcome for the owning tab's Grok Build row.
    pub fn outcome(self) -> crate::diagnostics::RemoteGrokInstallOutcome {
        match self {
            RemoteGrokInstallState::Running => {
                crate::diagnostics::RemoteGrokInstallOutcome::Running
            }
            RemoteGrokInstallState::Succeeded => {
                crate::diagnostics::RemoteGrokInstallOutcome::Succeeded
            }
            RemoteGrokInstallState::Failed { reason } => {
                crate::diagnostics::RemoteGrokInstallOutcome::Failed { reason }
            }
        }
    }
}

struct SmokeState {
    started: Option<Instant>,
    duration: Duration,
    frames: u64,
    reported: bool,
}

/// Stable process-local workspace identity; never reused after close.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceId(u64);

impl WorkspaceId {
    /// Numeric identity for diagnostics and widget scopes.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One background workspace preparation owned by the app.
struct PendingStart {
    generation: u64,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Workbench>,
    source: WorkspaceSource,
}

/// Look up which integrations of `list` have their program on `PATH`, in list
/// order. Metadata only; nothing is spawned.
fn detect_integrations(list: IntegrationList) -> Vec<IntegrationRef> {
    let found = made_platform::detect_programs(list.iter().map(|tui| tui.descriptor().program));
    list.iter()
        .copied()
        .filter(|tui| found.contains(&tui.descriptor().program))
        .collect()
}

/// Start the optional metadata-only TUI lookup off the UI thread.
///
/// The receiver is polled by [`AppModel::logic`]; a failure to spawn a thread
/// leaves the menu in its no-TUI state rather than blocking startup.
fn spawn_compatible_tui_detection(list: IntegrationList) -> Option<Receiver<Vec<IntegrationRef>>> {
    let (sender, receiver) = mpsc::channel();
    match thread::Builder::new()
        .name("made-tui-detection".to_owned())
        .spawn(move || {
            let _ = sender.send(detect_integrations(list));
        }) {
        Ok(_) => Some(receiver),
        Err(error) => {
            eprintln!("made: compatible TUI detection thread could not start ({error})");
            None
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SshDraft {
    pub name: String,
    pub host: String,
    pub user: String,
    pub port: String,
    pub key_file: String,
    pub directory: String,
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteLaunchDraft {
    pub authority: made_remote::protocol::AuthorityId,
    pub opencode: bool,
    pub directory: String,
    pub label: String,
}

fn load_ssh_registry() -> Result<ServerRegistry<PrivateMetadataStorage>, String> {
    let storage = PrivateMetadataStorage::from_app_paths()
        .map_err(|_| "SSH server list is unavailable. Local workspaces still open.".to_owned())?;
    ServerRegistry::load(storage).map_err(|error| match error {
        crate::ssh_servers::RegistryError::Corrupt => {
            "The SSH server list is unreadable. The original file was kept. Local workspaces still open."
                .to_owned()
        }
        crate::ssh_servers::RegistryError::Version(version) => {
            format!("The SSH server list uses schema {version}, which this version does not rewrite.")
        }
        other => format!("SSH server list: {other}"),
    })
}

/// The app model for one process.
pub struct AppModel {
    launch: WorkspaceLaunch,
    workspaces: Vec<(WorkspaceId, Workbench)>,
    workspace_sources: Vec<(WorkspaceId, WorkspaceSource)>,
    active: Option<WorkspaceId>,
    next_workspace: u64,
    start_pending: bool,
    pending_start: Option<PendingStart>,
    start_generation: u64,
    reset_ui_focus: bool,
    show_sessions: bool,
    show_about: bool,
    shutdown_done: bool,
    frame_count: u64,
    ui_frames: u64,
    theme_ready: bool,
    native_backdrop: NativeBackdropStatus,
    compatible_tuis: Vec<IntegrationRef>,
    compatible_tui_receiver: Option<Receiver<Vec<IntegrationRef>>>,
    // BEGIN remote-path (AIA-02)
    /// Per-server remote Grok plugin install outcome, from the bounded
    /// consented attempt or a test hold.
    remote_grok_installs: Vec<(made_remote::protocol::AuthorityId, RemoteGrokInstallState)>,
    /// In-flight remote Grok install SSH attempts, at most one per authority.
    /// Attempts are independent: approving one server never cancels or
    /// replaces another server's attempt.
    remote_grok_install_attempts: Vec<(
        made_remote::protocol::AuthorityId,
        crate::ssh_servers::GrokInstallAttempt,
    )>,
    /// Authorities whose private companion was staged or verified in this
    /// session. The Grok plugin installer needs that helper on the server, so
    /// approval stays unavailable until this set contains the authority.
    remote_grok_provisioned: Vec<made_remote::protocol::AuthorityId>,
    /// Authority whose install attempt owns the displayed `ssh_prompt`.
    remote_grok_prompt_authority: Option<made_remote::protocol::AuthorityId>,
    /// Servers whose install was refused this session. Refusal is the absence
    /// of consent and is never persisted as approval.
    remote_grok_install_refusals: Vec<made_remote::protocol::AuthorityId>,
    // END remote-path
    /// The integrations this model was built with, in catalog order.
    integrations: IntegrationList,
    /// App-wide integration state, parallel to `integrations`.
    app_states: Slots<Box<dyn AppState>>,
    terminal_target: TerminalTarget,
    smoke: Option<SmokeState>,
    ssh: Result<ServerRegistry<PrivateMetadataStorage>, String>,
    show_ssh: bool,
    ssh_selected: Option<made_remote::protocol::AuthorityId>,
    ssh_draft: SshDraft,
    ssh_notice: Option<String>,
    ssh_host_key_changed: bool,
    probe: Option<ProbeAttempt>,
    ssh_prompt: Option<made_platform::ssh_askpass::PromptRequest>,
    prompt_draft: String,
    remote_launch: Option<RemoteLaunchDraft>,
    remote_closes: Vec<Receiver<crate::remote_session::CloseReport>>,
    remote_close_notice: Option<String>,
    /// Update scheduling and state. Non-installed and smoke builds hold an
    /// unavailable scheduler that never makes a request.
    updates: UpdateScheduler,
    /// Where the REL-10 choice is stored; `None` keeps it in memory only.
    update_store: Option<crate::preferences::PreferenceStore>,
    /// The inline `Restart to update` confirmation is shown.
    update_confirming: bool,
    /// Set once `Restart now` handed the update to the updater; the window
    /// then closes so the updater can apply and relaunch.
    update_restarting: bool,
    /// Recent update log lines, replayed into workspaces opened later.
    update_history: Vec<(crate::diagnostics::LogLevel, String)>,
    /// Why the last `Restart now` did not start.
    update_notice: Option<String>,
    /// The last command-line apply outcome, shown in Diagnostics.
    update_apply_outcome: Option<String>,
}

impl std::fmt::Debug for AppModel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppModel")
            .field("launch", &self.launch)
            .field("start_pending", &self.start_pending)
            .field("sessions_open", &self.show_sessions)
            .field("about_open", &self.show_about)
            .field("workspaces", &self.workspaces)
            .finish_non_exhaustive()
    }
}

impl AppModel {
    /// Build from parsed launch arguments.
    ///
    /// The hidden smoke harness without a folder keeps its pre-home picker
    /// behavior: automated checks must not start a real shell in the user's
    /// home directory. Product startup (no smoke) opens the platform home.
    pub fn new(args: LaunchArgs, options: AppOptions) -> Self {
        let selection =
            if args.smoke_test.is_some() && matches!(args.workspace, WorkspaceSelection::None) {
                WorkspaceSelection::Picker
            } else {
                args.workspace
            };
        let launch = WorkspaceLaunch::from_selection(selection);
        Self::from_launch(launch, options)
    }

    /// Build from an already constructed launch state. Used by the headless UI
    /// model test and by the binary.
    pub fn from_launch(launch: WorkspaceLaunch, options: AppOptions) -> Self {
        let smoke = options.smoke_test.map(|duration| SmokeState {
            started: None,
            duration,
            frames: 0,
            reported: false,
        });
        let start_pending = launch.surfaces().has_terminal();
        let compatible_tui_receiver = if options.detect_compatible_tuis {
            spawn_compatible_tui_detection(PRODUCT)
        } else {
            None
        };
        let app_states = Slots::from_list(PRODUCT, |entry| {
            entry.new_app_state(&AppInit {
                detecting: options.detect_compatible_tuis,
                detection_started: compatible_tui_receiver.is_some(),
                catalog: &options.compatible_tuis,
            })
        });
        Self {
            launch,
            workspaces: Vec::new(),
            workspace_sources: Vec::new(),
            active: None,
            next_workspace: 1,
            start_pending,
            pending_start: None,
            start_generation: 0,
            reset_ui_focus: false,
            show_sessions: false,
            show_about: false,
            shutdown_done: false,
            frame_count: 0,
            ui_frames: 0,
            theme_ready: false,
            native_backdrop: NativeBackdropStatus::default(),
            compatible_tuis: options.compatible_tuis,
            compatible_tui_receiver,
            integrations: PRODUCT,
            app_states,
            // BEGIN remote-path (AIA-02)
            remote_grok_installs: Vec::new(),
            remote_grok_install_attempts: Vec::new(),
            remote_grok_provisioned: Vec::new(),
            remote_grok_prompt_authority: None,
            remote_grok_install_refusals: Vec::new(),
            // END remote-path
            terminal_target: TerminalTarget::Shell,
            smoke,
            ssh: load_ssh_registry(),
            show_ssh: false,
            ssh_selected: None,
            ssh_draft: SshDraft::default(),
            ssh_notice: None,
            ssh_host_key_changed: false,
            probe: None,
            ssh_prompt: None,
            prompt_draft: String::new(),
            remote_launch: None,
            remote_closes: Vec::new(),
            remote_close_notice: None,
            updates: UpdateScheduler::unavailable(false),
            update_store: None,
            update_confirming: false,
            update_restarting: false,
            update_history: Vec::new(),
            update_notice: None,
            update_apply_outcome: None,
        }
    }

    /// Build over a given integration list instead of the product's. Call it
    /// before the model runs: it replaces the app-wide integration state and
    /// every workspace started afterwards is built over `list`.
    #[doc(hidden)]
    pub fn with_integrations(mut self, list: IntegrationList) -> Self {
        debug_assert_eq!(crate::integrations::validate(list), Ok(()));
        // A live worker has not answered yet; a model without one is not
        // detecting.
        let detecting = self.compatible_tui_receiver.is_some();
        // The lookup is over the list's programs, so a pending one restarts.
        if detecting {
            self.compatible_tui_receiver = spawn_compatible_tui_detection(list);
        }
        let detecting = self.compatible_tui_receiver.is_some();
        self.app_states = Slots::from_list(list, |entry| {
            entry.new_app_state(&AppInit {
                detecting,
                detection_started: detecting,
                catalog: &self.compatible_tuis,
            })
        });
        self.integrations = list;
        self
    }

    /// The integrations this model was built over, in catalog order.
    #[doc(hidden)]
    pub fn integrations(&self) -> IntegrationList {
        self.integrations
    }

    /// Launch state.
    pub fn launch(&self) -> &WorkspaceLaunch {
        &self.launch
    }

    /// Mutable launch state.
    pub fn launch_mut(&mut self) -> &mut WorkspaceLaunch {
        &mut self.launch
    }

    /// Which surfaces the current launch state selects.
    pub fn surfaces(&self) -> crate::workspace::Surfaces {
        self.launch.surfaces()
    }

    /// Whether the picker is the only visible surface.
    pub fn is_picker(&self) -> bool {
        self.launch.surfaces().has_picker()
    }

    /// The workbench runtime, once started.
    pub fn workbench(&self) -> Option<&Workbench> {
        self.active.and_then(|id| self.workspace(id))
    }

    /// Mutable workbench runtime.
    pub fn workbench_mut(&mut self) -> Option<&mut Workbench> {
        let id = self.active?;
        self.workspace_mut(id)
    }

    /// All live workspace identities, in tab order.
    pub fn workspace_ids(&self) -> impl Iterator<Item = WorkspaceId> + '_ {
        self.workspaces.iter().map(|(id, _)| *id)
    }

    /// Number of live workspace owners.
    pub fn workspace_count(&self) -> usize {
        self.workspaces.len()
    }

    /// Selected workspace, including while the add-folder picker is open.
    pub fn active_workspace_id(&self) -> Option<WorkspaceId> {
        self.active
    }

    /// Compatible TUIs offered by the `+` new-tab menu.
    pub fn compatible_tuis(&self) -> &[IntegrationRef] {
        &self.compatible_tuis
    }

    // BEGIN remote-path (AIA-02)
    /// Record a finished probe outcome for one server without an SSH attempt.
    ///
    /// Test-only seam: the product flow arrives through [`Self::poll_ssh_probe`]
    /// with a live attempt.
    pub fn hold_ssh_probe_result(
        &mut self,
        id: made_remote::protocol::AuthorityId,
        result: Result<made_remote::protocol::Capabilities, made_remote::protocol::ErrorCode>,
    ) -> Result<(), String> {
        let registry = self.ssh.as_mut().map_err(|error| error.clone())?;
        registry
            .record_probe_result(id, result)
            .map_err(|error| error.to_string())
    }

    /// Record whether one server's finished probe detected claude, without an
    /// SSH attempt.
    ///
    /// Test-only seam: the product flow records it from the live probe.
    pub fn hold_ssh_claude_detection(
        &mut self,
        id: made_remote::protocol::AuthorityId,
        detected: bool,
    ) -> Result<(), String> {
        let registry = self.ssh.as_mut().map_err(|error| error.clone())?;
        registry.record_claude_detection(id, detected);
        Ok(())
    }

    /// Whether this server's own probe detected remote claude. Absent or
    /// failed probes never count as detection.
    pub fn remote_claude_detected_on(
        &self,
        id: made_remote::protocol::AuthorityId,
    ) -> Result<bool, String> {
        let registry = self.ssh.as_ref().map_err(|error| error.clone())?;
        Ok(registry.can_launch_claude(id))
    }

    /// Grant or withdraw the per-server/version Grok plugin install consent.
    ///
    /// Test-only seam: the product flow arrives through the consent card
    /// ([`Self::approve_remote_grok_install`] / [`Self::withdraw_remote_grok_install`]).
    pub fn hold_grok_install_consent(
        &mut self,
        id: made_remote::protocol::AuthorityId,
        granted: bool,
    ) -> Result<(), String> {
        let registry = self.ssh.as_mut().map_err(|error| error.clone())?;
        let version = granted.then(|| crate::APP_VERSION.to_owned());
        registry
            .set_grok_consent(id, version)
            .map_err(|error| error.to_string())
    }

    /// Hold the remote Grok plugin install outcome for one server.
    ///
    /// Test-only seam: the product flow arrives through
    /// [`Self::poll_remote_grok_install`]. Holds do not spawn SSH, stage
    /// files, or invoke any installer.
    pub fn hold_remote_grok_install(
        &mut self,
        id: made_remote::protocol::AuthorityId,
        hold: RemoteGrokInstallHold,
    ) {
        let state = match hold {
            RemoteGrokInstallHold::Running => RemoteGrokInstallState::Running,
            RemoteGrokInstallHold::Succeeded => RemoteGrokInstallState::Succeeded,
            RemoteGrokInstallHold::Failed { reason } => RemoteGrokInstallState::Failed { reason },
        };
        if let Some((_, slot)) = self
            .remote_grok_installs
            .iter_mut()
            .find(|(key, _)| *key == id)
        {
            *slot = state;
        } else {
            self.remote_grok_installs.push((id, state));
        }
        self.push_integration_diagnostics();
    }

    /// Whether this server's own probe detected remote grok. Absent or failed
    /// probes never count as detection.
    pub fn remote_grok_detected_on(
        &self,
        id: made_remote::protocol::AuthorityId,
    ) -> Result<bool, String> {
        let registry = self.ssh.as_ref().map_err(|error| error.clone())?;
        Ok(registry.can_launch_grok(id))
    }

    /// Whether the initial per-server/version provisioning consent is granted.
    pub fn remote_provision_consent_granted(
        &self,
        id: made_remote::protocol::AuthorityId,
    ) -> Result<bool, String> {
        let registry = self.ssh.as_ref().map_err(|error| error.clone())?;
        let snapshot = registry.snapshot(id).map_err(|error| error.to_string())?;
        Ok(snapshot.server().allows_provision(crate::APP_VERSION))
    }

    /// Whether the per-server/version Grok install consent is granted.
    pub fn remote_grok_consent_granted(
        &self,
        id: made_remote::protocol::AuthorityId,
    ) -> Result<bool, String> {
        let registry = self.ssh.as_ref().map_err(|error| error.clone())?;
        let snapshot = registry.snapshot(id).map_err(|error| error.to_string())?;
        Ok(snapshot.server().allows_grok_install(crate::APP_VERSION))
    }

    /// Recorded remote Grok install outcome for one server, if any.
    pub fn remote_grok_install_state(
        &self,
        id: made_remote::protocol::AuthorityId,
    ) -> Option<RemoteGrokInstallState> {
        self.remote_grok_installs
            .iter()
            .find(|(key, _)| *key == id)
            .map(|(_, state)| *state)
    }

    /// Whether this server's own Grok install attempt is still bounded-busy.
    pub fn remote_grok_install_running(&self, id: made_remote::protocol::AuthorityId) -> bool {
        self.remote_grok_install_attempts
            .iter()
            .any(|(authority, _)| *authority == id)
    }

    /// Whether this session observed a successfully staged or reused private
    /// companion for this server. The official installer execs that helper, so
    /// approval must not start it before the helper exists.
    pub fn remote_grok_install_ready(&self, id: made_remote::protocol::AuthorityId) -> bool {
        self.remote_grok_provisioned.contains(&id)
    }

    /// Test-only seam: record a successfully provisioned/reused companion.
    /// Production records this from the owning remote tab's completed stage.
    pub fn hold_remote_companion_provisioned(&mut self, id: made_remote::protocol::AuthorityId) {
        if !self.remote_grok_provisioned.contains(&id) {
            self.remote_grok_provisioned.push(id);
        }
    }

    /// Whether this server's install was refused in this session. Refusal is
    /// the absence of consent and is never persisted as approval.
    pub fn remote_grok_install_refused(&self, id: made_remote::protocol::AuthorityId) -> bool {
        self.remote_grok_install_refusals.contains(&id)
    }

    /// Approve the per-server/version Grok plugin install: persist the consent
    /// and start the bounded official-installer attempt against the server's
    /// already provisioned or reused private companion. Nothing is staged,
    /// invoked, or written under this approval, and a server whose companion
    /// has not been provisioned in this session cannot be approved.
    pub fn approve_remote_grok_install(
        &mut self,
        id: made_remote::protocol::AuthorityId,
    ) -> Result<(), String> {
        let registry = self.ssh.as_mut().map_err(|error| error.clone())?;
        let snapshot = registry.snapshot(id).map_err(|error| error.to_string())?;
        let target = match registry.probe_state(id) {
            crate::ssh_servers::ProbeState::Complete(capabilities) if capabilities.grok => {
                match capabilities.target {
                    made_remote::protocol::RemoteTarget::LinuxX64 => "x86_64-unknown-linux-gnu",
                    made_remote::protocol::RemoteTarget::LinuxArm64 => "aarch64-unknown-linux-gnu",
                    made_remote::protocol::RemoteTarget::MacosX64 => "x86_64-apple-darwin",
                    made_remote::protocol::RemoteTarget::MacosArm64 => "aarch64-apple-darwin",
                }
            }
            _ => {
                return Err(
                    "Grok SSH stays hidden until this server's own detection succeeds.".to_owned(),
                )
            }
        };
        if !self.remote_grok_provisioned.contains(&id) {
            return Err(
                "Open this server's remote workspace first so Made can provision or verify its private companion; the Grok plugin install needs that staged helper."
                    .to_owned(),
            );
        }
        registry
            .set_grok_consent(id, Some(crate::APP_VERSION.to_owned()))
            .map_err(|error| error.to_string())?;
        self.remote_grok_install_refusals.retain(|key| *key != id);
        let metadata = snapshot.server().metadata().clone();
        // Re-approving this server replaces only its own attempt. Another
        // server's in-flight attempt is never cancelled or overwritten.
        if let Some((_, previous)) = self
            .remote_grok_install_attempts
            .iter_mut()
            .find(|(key, _)| *key == id)
        {
            previous.cancel();
        }
        self.remote_grok_install_attempts
            .retain(|(key, _)| *key != id);
        if self
            .remote_grok_prompt_authority
            .is_some_and(|owner| owner == id)
        {
            self.remote_grok_prompt_authority = None;
        }
        self.upsert_remote_grok_install(id, RemoteGrokInstallState::Running);
        self.remote_grok_install_attempts.push((
            id,
            crate::ssh_servers::spawn_grok_install(
                metadata,
                crate::APP_VERSION.to_owned(),
                target.to_owned(),
            ),
        ));
        self.push_integration_diagnostics();
        self.ssh_notice = Some(
            "Approved for this server and version. Installing the official Grok Build plugin."
                .to_owned(),
        );
        Ok(())
    }

    /// Refuse the Grok plugin install for this session: no consent is stored,
    /// nothing is staged or invoked, and the inventory tier keeps working.
    pub fn refuse_remote_grok_install(&mut self, id: made_remote::protocol::AuthorityId) {
        if !self.remote_grok_install_refusals.contains(&id) {
            self.remote_grok_install_refusals.push(id);
        }
        self.ssh_notice = Some(
            "Refused. Nothing was installed or written; session follow uses the remote inventory."
                .to_owned(),
        );
    }

    /// Withdraw a previously granted per-server/version consent. No further
    /// install may start; an already-installed plugin is never uninstalled.
    pub fn withdraw_remote_grok_install(
        &mut self,
        id: made_remote::protocol::AuthorityId,
    ) -> Result<(), String> {
        let registry = self.ssh.as_mut().map_err(|error| error.clone())?;
        registry
            .set_grok_consent(id, None)
            .map_err(|error| error.to_string())?;
        self.ssh_notice = Some(
            "Consent withdrawn. Nothing was uninstalled; no further install can start.".to_owned(),
        );
        Ok(())
    }

    fn upsert_remote_grok_install(
        &mut self,
        id: made_remote::protocol::AuthorityId,
        state: RemoteGrokInstallState,
    ) {
        if let Some((_, slot)) = self
            .remote_grok_installs
            .iter_mut()
            .find(|(key, _)| *key == id)
        {
            *slot = state;
        } else {
            self.remote_grok_installs.push((id, state));
        }
    }

    /// Poll every bounded remote Grok install attempt without waiting for any.
    /// Attempts are per-authority and independent: one finishing, prompting,
    /// or failing never touches another server's attempt or recorded outcome.
    fn poll_remote_grok_install(&mut self) {
        if self.ssh_prompt.is_none() {
            // One auth card slot per frame; other attempts keep their prompts
            // queued until the slot frees.
            for (authority, attempt) in &self.remote_grok_install_attempts {
                if let Some(prompt) = attempt.poll_prompt() {
                    self.ssh_prompt = Some(prompt);
                    self.remote_grok_prompt_authority = Some(*authority);
                    self.show_ssh = true;
                    break;
                }
            }
        }
        let mut finished = Vec::new();
        self.remote_grok_install_attempts
            .retain(|(authority, attempt)| match attempt.poll() {
                Some(result) => {
                    finished.push((*authority, result));
                    false
                }
                None => true,
            });
        let any_finished = !finished.is_empty();
        for (authority, result) in finished {
            if self
                .remote_grok_prompt_authority
                .is_some_and(|owner| owner == authority)
            {
                self.remote_grok_prompt_authority = None;
            }
            match result {
                Ok(report) => {
                    let state = match report.outcome {
                        crate::ssh_servers::GrokInstallOutcome::Installed
                        | crate::ssh_servers::GrokInstallOutcome::Skipped => {
                            RemoteGrokInstallState::Succeeded
                        }
                        crate::ssh_servers::GrokInstallOutcome::Failed => {
                            RemoteGrokInstallState::Failed {
                                reason: report.reason.as_deref().and_then(known_install_reason),
                            }
                        }
                    };
                    let reason_text = match (&report.reason, report.outcome) {
                        (Some(reason), _) => format!(" The server reported: {reason}."),
                        (None, crate::ssh_servers::GrokInstallOutcome::Failed) => String::new(),
                        _ => String::new(),
                    };
                    let notice = match state {
                    RemoteGrokInstallState::Succeeded => format!(
                        "The Grok Build plugin is installed. Plugin hooks apply only to Grok sessions started after the install.{reason_text}"
                    ),
                    RemoteGrokInstallState::Failed { .. } => format!(
                        "The Grok Build plugin install failed. Session follow keeps the remote inventory tier.{reason_text}"
                    ),
                    RemoteGrokInstallState::Running => String::new(),
                };
                    self.upsert_remote_grok_install(authority, state);
                    self.ssh_notice = Some(notice);
                }
                Err(error) => {
                    self.upsert_remote_grok_install(
                        authority,
                        RemoteGrokInstallState::Failed { reason: None },
                    );
                    self.ssh_notice = Some(format!(
                    "The Grok Build plugin install failed. Session follow keeps the remote inventory tier. {}",
                    error.detail
                ));
                }
            }
        }
        if any_finished {
            self.push_integration_diagnostics();
        }
    }
    // END remote-path

    /// Give this workspace the OpenCode tab label without a bridge.
    ///
    /// The existing terminal process stays the child. This does not spawn
    /// OpenCode or grok, and the label is not the Grok running signal.
    pub fn hold_workspace_opencode_label(&mut self, id: WorkspaceId) {
        if let Some(workbench) = self.workspace_mut(id) {
            workbench.terminal_target = TerminalTarget::Tui(OPENCODE);
        }
        self.push_integration_diagnostics();
    }

    /// The app-wide integration states, parallel to [`Self::integrations`].
    pub(crate) fn app_states_mut(&mut self) -> &mut Slots<Box<dyn AppState>> {
        &mut self.app_states
    }

    /// Hand every workspace its rows from the app-wide integration states.
    pub(crate) fn push_integration_diagnostics(&mut self) {
        let mut provisioned = Vec::new();
        let mut unprovisioned = Vec::new();
        for (id, workbench) in &mut self.workspaces {
            if workbench.is_remote() {
                // BEGIN remote-path (AIA-02)
                if let Some(authority) = workbench.remote_authority() {
                    if workbench.remote_companion_provisioned() {
                        provisioned.push(authority);
                    } else {
                        unprovisioned.push(authority);
                    }
                }
                // A remote tab's Grok Build row is driven by its own server
                // state: its launch detection, this tab's transport and the
                // per-server install outcome and consent. The host phase never
                // applies to it.
                let consented = workbench
                    .remote_authority()
                    .and_then(|authority| {
                        let snapshot = self.ssh.as_ref().ok()?.snapshot(authority).ok()?;
                        Some(snapshot.server().allows_grok_install(crate::APP_VERSION))
                    })
                    .unwrap_or(false);
                let install = workbench
                    .remote_authority()
                    .and_then(|authority| {
                        self.remote_grok_installs
                            .iter()
                            .find(|(key, _)| *key == authority)
                            .map(|(_, state)| *state)
                    })
                    .map(RemoteGrokInstallState::outcome);
                workbench.set_remote_grok_row(remote_grok_row_for(consented, install, workbench));
                // END remote-path
                continue;
            }
            for state in self.app_states.iter() {
                workbench.publish_app_state(*id, state.as_ref());
            }
        }
        // A failed or update-required transition is not a provisioned path for
        // the Grok installer, even if an earlier attempt succeeded.
        self.remote_grok_provisioned.retain(|authority| {
            !unprovisioned.contains(authority) || provisioned.contains(authority)
        });
        for authority in provisioned {
            if !self.remote_grok_provisioned.contains(&authority) {
                self.remote_grok_provisioned.push(authority);
            }
        }
    }

    /// PTY child selected for the next workspace preparation.
    pub fn terminal_target(&self) -> TerminalTarget {
        self.terminal_target
    }

    pub(crate) fn ssh_open(&self) -> bool {
        self.show_ssh
    }

    pub(crate) fn toggle_ssh(&mut self) {
        self.show_ssh = !self.show_ssh;
    }

    pub(crate) fn set_ssh_open(&mut self, open: bool) {
        self.show_ssh = open;
    }

    pub(crate) fn set_ssh_notice(&mut self, notice: impl Into<String>) {
        self.ssh_notice = Some(notice.into());
    }

    pub(crate) fn ssh_registry(&self) -> Result<&ServerRegistry<PrivateMetadataStorage>, &str> {
        self.ssh.as_ref().map_err(String::as_str)
    }

    pub(crate) fn ssh_draft(&mut self) -> &mut SshDraft {
        &mut self.ssh_draft
    }

    pub fn ssh_notice(&self) -> Option<&str> {
        self.ssh_notice.as_deref()
    }

    pub fn remote_close_notice(&self) -> Option<&str> {
        self.remote_close_notice.as_deref()
    }

    pub fn approve_remote_provision(&mut self) -> Result<(), String> {
        let id = self
            .workbench()
            .and_then(Workbench::remote_authority)
            .ok_or_else(|| "No remote workspace is waiting for consent.".to_owned())?;
        self.ssh
            .as_mut()
            .map_err(|error| error.clone())?
            .set_consent(id, Some(APP_VERSION.to_owned()))
            .map_err(|error| error.to_string())?;
        if let Some(workbench) = self.workbench_mut() {
            workbench.grant_remote_consent();
        }
        self.ssh_notice = Some(
            "Provisioning approved for this server and version. No password was written to disk."
                .to_owned(),
        );
        Ok(())
    }

    pub fn deny_remote_provision(&mut self) {
        if let Some(workbench) = self.workbench_mut() {
            workbench.deny_remote_consent();
        }
    }

    /// Approve this attempt's same-version companion update. The baseline
    /// provisioning consent is not touched or granted by this action.
    pub fn approve_remote_companion_update(&mut self) -> Result<(), String> {
        let approved = self
            .workbench_mut()
            .is_some_and(Workbench::approve_remote_update);
        if approved {
            self.ssh_notice = Some(
                "Companion update approved for this attempt only. Running sessions are not signalled."
                    .to_owned(),
            );
            Ok(())
        } else {
            Err("No remote workspace is waiting for a companion update.".to_owned())
        }
    }

    pub fn refuse_remote_companion_update(&mut self) {
        if let Some(workbench) = self.workbench_mut() {
            workbench.refuse_remote_update();
        }
    }

    pub fn retry_remote_companion_update(&mut self) -> Result<(), String> {
        let retried = self
            .workbench_mut()
            .is_some_and(Workbench::retry_remote_update);
        if retried {
            Ok(())
        } else {
            Err("No failed update is available to retry.".to_owned())
        }
    }

    pub fn submit_remote_auth(&mut self, accept: bool) {
        if let Some(workbench) = self.workbench_mut() {
            workbench.submit_remote_auth(accept);
        }
    }

    pub fn reconnect_remote(&mut self) -> bool {
        self.workbench_mut()
            .is_some_and(Workbench::reconnect_remote)
    }

    fn persist_remote_consent(&mut self) {
        let pending: Vec<made_remote::protocol::AuthorityId> = self
            .workspaces
            .iter_mut()
            .filter_map(|(_, workbench)| workbench.take_consent_persist())
            .collect();
        for id in pending {
            if let Ok(registry) = self.ssh.as_mut() {
                let _ = registry.set_consent(id, Some(APP_VERSION.to_owned()));
            }
        }
    }

    fn poll_remote_closes(&mut self) {
        let mut notices = Vec::new();
        let had_reports = !self.remote_closes.is_empty();
        self.remote_closes.retain(|watch| match watch.try_recv() {
            Ok(crate::remote_session::CloseReport::Confirmed) => false,
            Ok(report) => {
                notices.push(report.label().to_owned());
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                notices.push(
                    crate::remote_session::CloseReport::Unconfirmed
                        .label()
                        .to_owned(),
                );
                false
            }
        });
        if let Some(notice) = notices.last() {
            self.remote_close_notice = Some(notice.clone());
            self.ssh_notice = Some(notice.clone());
        } else if had_reports && self.remote_closes.is_empty() {
            self.remote_close_notice = None;
        }
    }

    pub(crate) fn ssh_selected(&self) -> Option<made_remote::protocol::AuthorityId> {
        self.ssh_selected
    }

    pub fn select_ssh_server(&mut self, id: made_remote::protocol::AuthorityId) {
        let Ok(registry) = self.ssh.as_ref() else {
            return;
        };
        let Ok(snapshot) = registry.snapshot(id) else {
            return;
        };
        let metadata = snapshot.server().metadata();
        self.ssh_selected = Some(id);
        self.ssh_host_key_changed = false;
        self.ssh_draft = SshDraft {
            name: metadata.display_name.clone(),
            host: metadata.host.clone(),
            user: metadata.user.clone().unwrap_or_default(),
            port: metadata
                .port
                .map(|port| port.to_string())
                .unwrap_or_default(),
            key_file: metadata.key_file.clone().unwrap_or_default(),
            directory: metadata
                .initial_directory
                .as_ref()
                .and_then(|path| path.to_utf8().ok())
                .unwrap_or("")
                .to_owned(),
        };
    }

    pub(crate) fn ssh_prompt(&self) -> Option<&made_platform::ssh_askpass::PromptRequest> {
        self.ssh_prompt.as_ref()
    }

    pub(crate) fn prompt_draft(&mut self) -> &mut String {
        &mut self.prompt_draft
    }

    pub(crate) fn remote_launch(&self) -> Option<&RemoteLaunchDraft> {
        self.remote_launch.as_ref()
    }

    pub(crate) fn remote_launch_directory(&mut self) -> Option<&mut String> {
        self.remote_launch
            .as_mut()
            .map(|draft| &mut draft.directory)
    }

    /// Save non-secret server metadata. Passwords are not accepted by the type.
    pub fn add_ssh_metadata(
        &mut self,
        metadata: ServerMetadata,
    ) -> Result<made_remote::protocol::AuthorityId, String> {
        let registry = self.ssh.as_mut().map_err(|error| error.clone())?;
        let id = registry.add(metadata).map_err(|error| error.to_string())?;
        self.ssh_selected = Some(id);
        self.ssh_host_key_changed = false;
        Ok(id)
    }

    /// Attach a remote model without starting a host shell. A missing live SSH
    /// process is not reported as a successful connection.
    pub fn attach_remote_model(
        &mut self,
        id: made_remote::protocol::AuthorityId,
        workspace: crate::remote_workspace::RemoteWorkspace,
        opencode: bool,
    ) -> Result<(), String> {
        let snapshot = self
            .ssh
            .as_ref()
            .map_err(|error| error.clone())?
            .snapshot(id)
            .map_err(|error| error.to_string())?;
        let target = if opencode {
            TerminalTarget::RemoteOpenCode { authority: id }
        } else {
            TerminalTarget::RemoteShell { authority: id }
        };
        let mut workbench =
            Workbench::bind_remote_model(snapshot, workspace, target, "Remote model attached");
        // A bound test model inherits the server's own probe detection; the
        // real launch path re-probes and records its own result.
        if let Ok(registry) = self.ssh.as_ref() {
            if let crate::ssh_servers::ProbeState::Complete(capabilities) = registry.probe_state(id)
            {
                workbench.set_remote_grok_detection(capabilities.grok);
                workbench.set_remote_claude_detection(Some(registry.can_launch_claude(id)));
            }
        }
        let source = self.launch.source();
        self.leave_picker_for_remote();
        self.insert_workbench(workbench, source);
        self.start_pending = false;
        Ok(())
    }

    /// Attach a remote model as a directly launched `Claude Code SSH` tab
    /// without starting SSH, for tests of the pre-attach states.
    pub fn attach_remote_claude_model(
        &mut self,
        id: made_remote::protocol::AuthorityId,
        workspace: crate::remote_workspace::RemoteWorkspace,
    ) -> Result<(), String> {
        let snapshot = self
            .ssh
            .as_ref()
            .map_err(|error| error.clone())?
            .snapshot(id)
            .map_err(|error| error.to_string())?;
        let mut workbench = Workbench::bind_remote_model(
            snapshot,
            workspace,
            TerminalTarget::RemoteClaude { authority: id },
            "Remote model attached",
        );
        if let Ok(registry) = self.ssh.as_ref() {
            workbench.set_remote_claude_detection(Some(registry.can_launch_claude(id)));
        }
        let source = self.launch.source();
        self.leave_picker_for_remote();
        self.insert_workbench(workbench, source);
        self.start_pending = false;
        Ok(())
    }

    /// Attach a remote model as a directly launched `Grok SSH` tab: the target
    /// is the server's grok process, never a host binary. A missing live SSH
    /// process is not reported as a successful connection.
    pub fn attach_remote_grok_model(
        &mut self,
        id: made_remote::protocol::AuthorityId,
        workspace: crate::remote_workspace::RemoteWorkspace,
    ) -> Result<(), String> {
        let snapshot = self
            .ssh
            .as_ref()
            .map_err(|error| error.clone())?
            .snapshot(id)
            .map_err(|error| error.to_string())?;
        let mut workbench = Workbench::bind_remote_model(
            snapshot,
            workspace,
            TerminalTarget::RemoteGrok { authority: id },
            "Remote model attached",
        );
        if let Ok(registry) = self.ssh.as_ref() {
            if let crate::ssh_servers::ProbeState::Complete(capabilities) = registry.probe_state(id)
            {
                workbench.set_remote_grok_detection(capabilities.grok);
                workbench.set_remote_claude_detection(Some(registry.can_launch_claude(id)));
            }
        }
        let source = self.launch.source();
        self.leave_picker_for_remote();
        self.insert_workbench(workbench, source);
        self.start_pending = false;
        Ok(())
    }

    pub fn begin_remote_choice(
        &mut self,
        id: made_remote::protocol::AuthorityId,
        opencode: bool,
    ) -> Result<(), String> {
        let registry = self.ssh.as_ref().map_err(|error| error.clone())?;
        if opencode && !registry.can_launch_opencode(id) {
            return Err(
                "OpenCode SSH stays hidden until this server's own detection succeeds.".to_owned(),
            );
        }
        let snapshot = registry.snapshot(id).map_err(|error| error.to_string())?;
        let directory = snapshot
            .server()
            .metadata()
            .initial_directory
            .as_ref()
            .and_then(|path| path.to_utf8().ok())
            .unwrap_or("/")
            .to_owned();
        self.remote_launch = Some(RemoteLaunchDraft {
            authority: id,
            opencode,
            directory,
            label: snapshot.server().metadata().display_name.clone(),
        });
        self.show_ssh = true;
        self.ssh_notice = None;
        Ok(())
    }

    pub fn confirm_remote_launch(&mut self) -> Result<(), String> {
        let draft = self
            .remote_launch
            .clone()
            .ok_or_else(|| "Choose Terminal SSH or OpenCode SSH first.".to_owned())?;
        let registry = self.ssh.as_ref().map_err(|error| error.clone())?;
        if draft.opencode && !registry.can_launch_opencode(draft.authority) {
            return Err(
                "OpenCode is not detected on that server. The launch was not started locally."
                    .to_owned(),
            );
        }
        let snapshot = registry
            .snapshot(draft.authority)
            .map_err(|error| error.to_string())?;
        let directory = validate_remote_directory(draft.directory.as_bytes())
            .map_err(|error| error.to_owned())?;
        let target = if draft.opencode {
            TerminalTarget::RemoteOpenCode {
                authority: draft.authority,
            }
        } else {
            TerminalTarget::RemoteShell {
                authority: draft.authority,
            }
        };
        let workbench = Workbench::start_remote_in(
            self.integrations,
            snapshot,
            directory,
            target,
            Instant::now(),
        )?;
        let source = self.launch.source();
        self.leave_picker_for_remote();
        self.insert_workbench(workbench, source);
        self.remote_launch = None;
        self.show_ssh = false;
        self.start_pending = false;
        Ok(())
    }

    pub(crate) fn cancel_remote_launch(&mut self) {
        self.remote_launch = None;
    }

    /// Whether a remote launch choice is waiting for folder confirmation.
    /// A direct `Grok SSH` launch never sets this: it opens its tab at once.
    pub fn remote_launch_pending(&self) -> bool {
        self.remote_launch.is_some()
    }

    /// Start the selected server's detected grok executable with the single
    /// argument `dashboard` as a new tab, bound to that server and its selected
    /// remote directory. There is no shell first and no separate folder picker,
    /// the current tab keeps running, and no plugin configuration is injected.
    /// Servers whose own probe did not detect grok refuse here.
    pub fn begin_remote_grok_launch(
        &mut self,
        id: made_remote::protocol::AuthorityId,
    ) -> Result<(), String> {
        let registry = self.ssh.as_ref().map_err(|error| error.clone())?;
        if !registry.can_launch_grok(id) {
            return Err(
                "Grok SSH stays hidden until this server's own detection succeeds.".to_owned(),
            );
        }
        let snapshot = registry.snapshot(id).map_err(|error| error.to_string())?;
        let directory = match snapshot.server().metadata().initial_directory.clone() {
            Some(directory) => directory,
            None => made_remote::protocol::RemotePath::new(b"/".to_vec())
                .map_err(|_| "the default remote directory was rejected".to_owned())?,
        };
        let workbench = Workbench::start_remote_in(
            self.integrations,
            snapshot,
            directory,
            TerminalTarget::RemoteGrok { authority: id },
            Instant::now(),
        )?;
        let source = self.launch.source();
        self.leave_picker_for_remote();
        self.insert_workbench(workbench, source);
        self.start_pending = false;
        Ok(())
    }

    /// Start the selected server's detected claude executable with no
    /// arguments as a new tab, bound to that server and its selected remote
    /// directory. There is no shell first and no separate folder picker, the
    /// current tab keeps running, and no plugin configuration is injected.
    /// Servers whose own probe did not detect claude refuse here.
    pub fn begin_remote_claude_launch(
        &mut self,
        id: made_remote::protocol::AuthorityId,
    ) -> Result<(), String> {
        let registry = self.ssh.as_ref().map_err(|error| error.clone())?;
        if !registry.can_launch_claude(id) {
            return Err(
                "Claude Code SSH stays hidden until this server's own detection succeeds."
                    .to_owned(),
            );
        }
        let snapshot = registry.snapshot(id).map_err(|error| error.to_string())?;
        let directory = match snapshot.server().metadata().initial_directory.clone() {
            Some(directory) => directory,
            None => made_remote::protocol::RemotePath::new(b"/".to_vec())
                .map_err(|_| "the default remote directory was rejected".to_owned())?,
        };
        let workbench = Workbench::start_remote_in(
            self.integrations,
            snapshot,
            directory,
            TerminalTarget::RemoteClaude { authority: id },
            Instant::now(),
        )?;
        let source = self.launch.source();
        self.leave_picker_for_remote();
        self.insert_workbench(workbench, source);
        self.start_pending = false;
        Ok(())
    }

    pub(crate) fn save_ssh_server(&mut self) -> Result<(), String> {
        let metadata = self.draft_metadata()?;
        let registry = self.ssh.as_mut().map_err(|error| error.clone())?;
        if let Some(id) = self.ssh_selected {
            registry
                .edit(id, metadata)
                .map_err(|error| error.to_string())?;
            // Host, user, or port may have changed; never reuse the old secret.
            crate::credential_cache::forget(id);
            self.ssh_notice = Some("Saved. Open workspaces keep their original server.".to_owned());
        } else {
            let id = registry.add(metadata).map_err(|error| error.to_string())?;
            self.ssh_selected = Some(id);
            self.ssh_notice = Some(
                "Server saved. Passwords stay in memory until Made closes; never on disk."
                    .to_owned(),
            );
        }
        Ok(())
    }

    pub(crate) fn remove_selected_ssh_server(&mut self) -> Result<(), String> {
        let id = self
            .ssh_selected
            .ok_or_else(|| "Select a server to remove.".to_owned())?;
        let registry = self.ssh.as_mut().map_err(|error| error.clone())?;
        registry.remove(id).map_err(|error| error.to_string())?;
        crate::credential_cache::forget(id);
        self.ssh_selected = None;
        self.ssh_host_key_changed = false;
        self.ssh_draft = SshDraft::default();
        self.ssh_notice = Some("Server removed. Open workspaces were not closed.".to_owned());
        Ok(())
    }

    pub(crate) fn test_selected_ssh_server(&mut self) -> Result<(), String> {
        let id = self
            .ssh_selected
            .ok_or_else(|| "Save the server before testing it.".to_owned())?;
        let registry = self.ssh.as_mut().map_err(|error| error.clone())?;
        let (request, _) = registry
            .begin_probe(id)
            .map_err(|error| error.to_string())?;
        if let Some(previous) = self.probe.take() {
            previous.cancel();
        }
        self.probe = Some(crate::ssh_servers::spawn_probe(request));
        self.ssh_host_key_changed = false;
        self.ssh_notice = Some("Testing the connection. OpenCode will not be started.".to_owned());
        Ok(())
    }

    pub(crate) fn ssh_host_key_changed(&self) -> bool {
        self.ssh_host_key_changed
    }

    /// Forget the app-owned host key for the selected server. The next Test is
    /// an unknown-host approval, so the new fingerprint is shown explicitly.
    pub(crate) fn forget_selected_ssh_host_key(&mut self) -> Result<(), String> {
        let id = self
            .ssh_selected
            .ok_or_else(|| "Select a server first.".to_owned())?;
        let metadata = self
            .ssh
            .as_ref()
            .map_err(|error| error.clone())?
            .snapshot(id)
            .map_err(|error| error.to_string())?
            .server()
            .metadata()
            .clone();
        crate::ssh_servers::forget_saved_host_key(&metadata).map_err(|error| error.detail)?;
        self.ssh_host_key_changed = false;
        self.ssh_notice = Some(
            "Old host key forgotten. Test again to check and approve the new fingerprint."
                .to_owned(),
        );
        Ok(())
    }

    pub(crate) fn cancel_ssh_probe(&mut self) {
        if let Some(probe) = self.probe.as_ref() {
            let ticket = probe.ticket();
            probe.cancel();
            if let Ok(registry) = self.ssh.as_mut() {
                registry.cancel_probe(ticket);
            }
        }
        self.probe = None;
        self.ssh_prompt = None;
        self.remote_grok_prompt_authority = None;
        self.prompt_draft.clear();
        self.ssh_notice = Some("Connection test cancelled.".to_owned());
    }

    pub(crate) fn submit_ssh_prompt(&mut self, accept: bool) {
        let Some(prompt) = self.ssh_prompt.take() else {
            return;
        };
        // A displayed install prompt belongs to one authority's attempt and is
        // answered there; a probe prompt routes to the probe.
        let owner = self.remote_grok_prompt_authority.take();
        match crate::remote_session::classify_prompt(&prompt.text) {
            crate::remote_session::SshPromptClass::ChangedHostKey { .. } => {
                self.prompt_draft.clear();
                if let Some(probe) = self.probe.as_ref() {
                    let _ = probe.answer(prompt.id, None);
                    probe.cancel();
                }
                self.probe = None;
                self.ssh_prompt = None;
                self.ssh_host_key_changed = true;
                self.ssh_notice = Some(
                    crate::remote_session::SshFailureClass::ChangedHostKey
                        .detail()
                        .to_owned(),
                );
            }
            crate::remote_session::SshPromptClass::UnknownHost { .. } => {
                self.prompt_draft.clear();
                let answer = if accept {
                    made_platform::ssh_askpass::PromptAnswer::new(b"yes".to_vec()).ok()
                } else {
                    None
                };
                if let Some(owner) = owner {
                    if let Some((_, attempt)) = self
                        .remote_grok_install_attempts
                        .iter_mut()
                        .find(|(key, _)| *key == owner)
                    {
                        let _ = attempt.answer(prompt.id, answer);
                        if !accept {
                            attempt.cancel();
                        }
                    }
                } else if let Some(probe) = self.probe.as_ref() {
                    let _ = probe.answer(prompt.id, answer);
                    if !accept {
                        probe.cancel();
                    }
                }
                if !accept {
                    self.probe = None;
                    self.ssh_notice = Some(
                        crate::remote_session::SshFailureClass::Cancelled
                            .detail()
                            .to_owned(),
                    );
                }
            }
            crate::remote_session::SshPromptClass::Secret { .. } => {
                let remember = accept && crate::credential_cache::is_password_prompt(&prompt.text);
                let text = std::mem::take(&mut self.prompt_draft);
                if remember {
                    if let Some(authority) = self.ssh_selected {
                        crate::credential_cache::remember(authority, &text);
                    }
                }
                let answer = if accept {
                    made_platform::ssh_askpass::PromptAnswer::new(text.into_bytes()).ok()
                } else {
                    None
                };
                if let Some(probe) = self.probe.as_ref() {
                    let _ = probe.answer(prompt.id, answer);
                } else if let Some(owner) = owner {
                    if let Some((_, attempt)) = self
                        .remote_grok_install_attempts
                        .iter_mut()
                        .find(|(key, _)| *key == owner)
                    {
                        let _ = attempt.answer(prompt.id, answer);
                    }
                }
            }
        }
    }

    fn leave_picker_for_remote(&mut self) {
        if !self.is_picker() {
            return;
        }
        self.launch = crate::workspace::WorkspaceLaunch::TerminalFallback {
            error: crate::workspace::WorkspaceError::new(
                crate::workspace::WorkspaceErrorKind::InvalidPath,
                std::path::PathBuf::new(),
                "remote workspace",
            ),
            cwd: std::path::PathBuf::new(),
            source: crate::workspace::WorkspaceSource::Picker,
        };
    }

    fn draft_metadata(&self) -> Result<ServerMetadata, String> {
        let port = if self.ssh_draft.port.trim().is_empty() {
            None
        } else {
            Some(
                self.ssh_draft
                    .port
                    .trim()
                    .parse::<u16>()
                    .map_err(|_| "Port must be a number.".to_owned())?,
            )
        };
        let directory = if self.ssh_draft.directory.trim().is_empty() {
            None
        } else {
            Some(
                validate_remote_directory(self.ssh_draft.directory.trim().as_bytes())
                    .map_err(|error| error.to_owned())?,
            )
        };
        let metadata = ServerMetadata {
            display_name: self.ssh_draft.name.trim().to_owned(),
            host: self.ssh_draft.host.trim().to_owned(),
            user: {
                let user = self.ssh_draft.user.trim();
                (!user.is_empty()).then(|| user.to_owned())
            },
            port,
            key_file: {
                let key = self.ssh_draft.key_file.trim();
                (!key.is_empty()).then(|| key.to_owned())
            },
            initial_directory: directory,
        };
        metadata.validate().map_err(|_| {
            "Check the name, host, user, port, and key file. Passwords are never written to disk.".to_owned()
        })?;
        Ok(metadata)
    }

    fn poll_ssh_probe(&mut self) {
        if self.probe.is_some() && self.ssh_prompt.is_none() {
            self.ssh_prompt = self.probe.as_ref().and_then(ProbeAttempt::poll_prompt);
            if self.ssh_prompt.is_some() {
                self.show_ssh = true;
                self.try_cached_ssh_password();
            }
        }
        let finished = self.probe.as_ref().and_then(ProbeAttempt::poll);
        if let Some(result) = finished {
            let ticket = self.probe.as_ref().map(ProbeAttempt::ticket);
            self.probe = None;
            self.ssh_prompt = None;
            self.remote_grok_prompt_authority = None;
            self.prompt_draft.clear();
            if matches!(
                &result,
                Err(error) if error.class == crate::remote_session::SshFailureClass::AuthFailed
            ) {
                if let Some(authority) = self.ssh_selected {
                    crate::credential_cache::forget(authority);
                }
            }
            if let (Some(ticket), Ok(registry)) = (ticket, self.ssh.as_mut()) {
                let stored = match &result {
                    Ok(report) => Ok(report.capabilities.clone()),
                    Err(error) => Err(error.protocol_code()),
                };
                let finished = registry.finish_probe(ticket, stored).is_ok();
                if let (true, Ok(report)) = (finished, &result) {
                    registry.record_claude_detection(ticket.authority, report.claude);
                }
            }
            self.ssh_host_key_changed = matches!(
                &result,
                Err(error)
                    if error.class == crate::remote_session::SshFailureClass::ChangedHostKey
            );
            self.ssh_notice = Some(match result {
                Ok(report) if report.capabilities.opencode => {
                    "Connection succeeded. OpenCode was detected. The TUI was not started."
                        .to_owned()
                }
                Ok(_) => {
                    "Connection succeeded. OpenCode was not detected on this server.".to_owned()
                }
                Err(error) => match error.fingerprint {
                    Some(fingerprint) => format!("{}\n{fingerprint}", error.detail),
                    None => error.detail,
                },
            });
        }
    }

    /// Answer a password prompt from the process-lifetime cache. The card is
    /// suppressed only when a password is already available for this server.
    fn try_cached_ssh_password(&mut self) {
        let Some(prompt) = self.ssh_prompt.as_ref() else {
            return;
        };
        if !crate::credential_cache::is_password_prompt(&prompt.text)
            || !matches!(
                crate::remote_session::classify_prompt(&prompt.text),
                crate::remote_session::SshPromptClass::Secret { .. }
            )
        {
            return;
        }
        let Some(authority) = self.ssh_selected else {
            return;
        };
        let Some(answer) = crate::credential_cache::answer(authority) else {
            return;
        };
        let Some(probe) = self.probe.as_ref() else {
            return;
        };
        let _ = probe.answer(prompt.id, Some(answer));
        self.ssh_prompt = None;
        self.remote_grok_prompt_authority = None;
    }

    /// Look up a live owner; closed identities never resolve again.
    pub fn workspace(&self, id: WorkspaceId) -> Option<&Workbench> {
        self.workspaces
            .iter()
            .find(|(key, _)| *key == id)
            .map(|(_, value)| value)
    }

    /// Mutate a live owner without changing selection.
    pub fn workspace_mut(&mut self, id: WorkspaceId) -> Option<&mut Workbench> {
        self.workspaces
            .iter_mut()
            .find(|(key, _)| *key == id)
            .map(|(_, value)| value)
    }

    /// Where a live workspace tab was launched from.
    ///
    /// Retained separately from [`Workbench`] so a terminal fallback without a
    /// validated root still reports `default-home`, `launch-argument`, or
    /// `picker` honestly.
    pub fn workspace_source(&self, id: WorkspaceId) -> Option<WorkspaceSource> {
        self.workspace_sources
            .iter()
            .find(|(key, _)| *key == id)
            .map(|(_, source)| *source)
    }

    /// Open the existing picker without stopping any workspace.
    pub fn begin_new_workspace(&mut self) {
        self.cancel_pending_start();
        self.reset_ui_focus = true;
        if let Some(workbench) = self.workbench_mut() {
            workbench.set_visible(false);
        }
        self.terminal_target = TerminalTarget::Shell;
        self.launch = WorkspaceLaunch::waiting_for_picker();
        self.start_pending = false;
    }

    /// Start a new home-directory tab whose PTY child is `tui`.
    ///
    /// The current tab keeps running. Preparation uses the same background
    /// path as any other workspace, and the new tab is selected only when its
    /// runtime is ready.
    pub fn begin_tui_workspace(&mut self, tui: IntegrationRef) {
        self.cancel_pending_start();
        self.reset_ui_focus = true;
        self.terminal_target = TerminalTarget::Tui(tui);
        self.launch = WorkspaceLaunch::from_selection_with_home(
            WorkspaceSelection::None,
            made_platform::host_platform().home_dir(),
        );
        self.start_pending = true;
        self.show_sessions = false;
    }

    /// Return from the picker to the selected running workspace.
    pub fn cancel_picker(&mut self) -> bool {
        self.active.is_some_and(|id| self.switch_workspace(id))
    }

    /// Dismiss the picker. With a live workspace, return to it; without one,
    /// open the default home workspace instead of trapping the user in the
    /// picker. Home resolution keeps the same terminal-only fallback behavior
    /// as normal no-argument startup when the home is unavailable.
    pub fn close_picker(&mut self) -> bool {
        if self.cancel_picker() {
            return true;
        }
        self.cancel_pending_start();
        self.reset_ui_focus = true;
        self.terminal_target = TerminalTarget::Shell;
        self.launch = WorkspaceLaunch::from_selection_with_home(
            WorkspaceSelection::None,
            made_platform::host_platform().home_dir(),
        );
        self.start_pending = self.launch.surfaces().has_terminal();
        self.show_sessions = false;
        true
    }

    /// Activate a tab and explicitly reset terminal focus/composition state.
    pub fn switch_workspace(&mut self, id: WorkspaceId) -> bool {
        if self.workspace(id).is_none() {
            return false;
        }
        if self.workspace(id).is_some_and(Workbench::is_remote) {
            if let Some(workbench) = self.workbench_mut() {
                workbench.set_visible(false);
            }
            self.active = Some(id);
            self.reset_ui_focus = true;
            if let Some(workbench) = self.workbench_mut() {
                workbench.set_visible(true);
            }
            self.start_pending = false;
            return true;
        }
        let Some(source) = self.workspace_source(id) else {
            return false;
        };
        let (plan, error, cwd) = {
            let workbench = self.workspace(id).expect("selected owner exists");
            (
                workbench.launch_plan().cloned(),
                workbench.workspace_error().cloned(),
                workbench.shell_cwd().to_path_buf(),
            )
        };
        if let Some(workbench) = self.workbench_mut() {
            workbench.set_visible(false);
        }
        self.active = Some(id);
        self.reset_ui_focus = true;
        let workbench = self.workbench_mut().expect("selected owner exists");
        workbench.set_visible(true);
        self.launch = if let Some(plan) = plan {
            WorkspaceLaunch::Ready(plan)
        } else {
            WorkspaceLaunch::TerminalFallback {
                error: error.expect("fallback owner retains launch error"),
                cwd,
                source,
            }
        };
        self.start_pending = false;
        true
    }

    /// Shut down only this tab. Closing the final tab returns to the picker.
    pub fn close_workspace(&mut self, id: WorkspaceId) -> bool {
        let Some(index) = self.workspaces.iter().position(|(key, _)| *key == id) else {
            return false;
        };
        let (_, mut workbench) = self.workspaces.remove(index);
        self.workspace_sources.retain(|(key, _)| *key != id);
        if let Some(watch) = workbench.take_remote_close() {
            self.remote_closes.push(watch);
        }
        for state in self.app_states.iter_mut() {
            state.on_workspace_closed(id);
        }
        workbench.shutdown();
        if self.active == Some(id) {
            self.reset_ui_focus = true;
            self.active = None;
            if let Some((next, _)) = self
                .workspaces
                .get(index.min(self.workspaces.len().saturating_sub(1)))
            {
                self.switch_workspace(*next);
            } else {
                self.begin_new_workspace();
            }
        }
        true
    }

    fn insert_workbench(&mut self, mut workbench: Workbench, source: WorkspaceSource) {
        if let Some(current) = self.workbench_mut() {
            current.set_visible(false);
        }
        for (level, message) in &self.update_history {
            workbench.diag.log_mut().record(*level, message.clone());
        }
        workbench
            .diag
            .set_update_apply_outcome(self.update_apply_outcome.clone());
        let id = WorkspaceId(self.next_workspace);
        self.next_workspace = self
            .next_workspace
            .checked_add(1)
            .expect("workspace identity exhausted");
        if workbench.is_remote() {
            // BEGIN remote-path (AIA-02)
            let consented = workbench
                .remote_authority()
                .and_then(|authority| {
                    let snapshot = self.ssh.as_ref().ok()?.snapshot(authority).ok()?;
                    Some(snapshot.server().allows_grok_install(crate::APP_VERSION))
                })
                .unwrap_or(false);
            let install = workbench
                .remote_authority()
                .and_then(|authority| {
                    self.remote_grok_installs
                        .iter()
                        .find(|(key, _)| *key == authority)
                        .map(|(_, state)| *state)
                })
                .map(RemoteGrokInstallState::outcome);
            workbench.set_remote_grok_row(remote_grok_row_for(consented, install, &workbench));
            // END remote-path
        } else {
            for state in self.app_states.iter() {
                workbench.publish_app_state(id, state.as_ref());
            }
        }
        for state in self.app_states.iter() {
            workbench.notify_from(state.as_ref());
        }
        workbench.set_benchmark_frames(self.smoke.is_some());
        workbench.set_visible(false);
        workbench.set_visible(true);
        self.workspaces.push((id, workbench));
        self.workspace_sources.push((id, source));
        self.active = Some(id);
        self.reset_ui_focus = true;
        self.start_pending = false;
    }

    /// Whether the runtime still needs to start.
    pub fn start_pending(&self) -> bool {
        self.start_pending
    }

    /// Native backdrop status recorded from the real window.
    ///
    /// Until the shell's first frame this is the in-app fallback with a
    /// "not checked" reason. The status is owned by the app rather than
    /// re-queried from the window every frame.
    pub fn native_backdrop(&self) -> &NativeBackdropStatus {
        &self.native_backdrop
    }

    /// Store the one-shot native backdrop result supplied by the window shell.
    pub fn set_native_backdrop(&mut self, status: NativeBackdropStatus) {
        self.native_backdrop = status;
    }

    /// Confirm the picker's current folder and select the workspace surfaces.
    ///
    /// The runtime starts on the next [`AppModel::update`] call. This method is
    /// display-free and is exercised by the headless UI model test.
    pub fn confirm_picker(&mut self) -> Result<(), WorkspaceError> {
        self.cancel_pending_start();
        self.launch.confirm_picker()?;
        self.terminal_target = TerminalTarget::Shell;
        self.start_pending = true;
        self.show_sessions = false;
        if let Some(workbench) = self.workbench_mut() {
            workbench.set_visible(true);
        }
        Ok(())
    }

    /// Open an already validated plan, for example a selected picker entry.
    pub fn open_plan(&mut self, plan: LaunchPlan) {
        self.cancel_pending_start();
        self.launch.open_plan(plan);
        self.terminal_target = TerminalTarget::Shell;
        self.start_pending = true;
        self.show_sessions = false;
        if let Some(workbench) = self.workbench_mut() {
            workbench.set_visible(true);
        }
    }

    /// Toggle expansion of a picker row.
    pub fn picker_toggle(
        &mut self,
        index: usize,
    ) -> Result<(), crate::workspace_picker::PickerError> {
        self.launch.picker_toggle(index)
    }

    /// Toggle expansion of the selected picker row.
    pub fn picker_toggle_selected(&mut self) -> Result<(), crate::workspace_picker::PickerError> {
        self.launch.picker_toggle_selected()
    }

    /// Expand the selected picker row, or select its first child.
    pub fn picker_expand_selected(&mut self) -> Result<(), crate::workspace_picker::PickerError> {
        self.launch.picker_expand_selected()
    }

    /// Collapse the selected picker row, or select its parent row.
    pub fn picker_collapse_selected(&mut self) -> Result<(), crate::workspace_picker::PickerError> {
        self.launch.picker_collapse_selected()
    }

    /// Navigate the picker to the parent folder.
    pub fn picker_up(&mut self) -> Result<(), crate::workspace_picker::PickerError> {
        self.launch.picker_up()
    }

    /// Move the picker selection.
    pub fn picker_move(&mut self, delta: isize) {
        self.launch.picker_move(delta);
    }

    /// Select a picker row.
    pub fn picker_select(
        &mut self,
        index: usize,
    ) -> Result<(), crate::workspace_picker::PickerError> {
        self.launch.picker_select(index)
    }

    /// Refresh the picker listing.
    pub fn picker_refresh(&mut self) -> Result<(), WorkspaceError> {
        self.launch.picker_refresh()
    }

    /// Start the runtime synchronously.
    ///
    /// UI callers should let [`AppModel::logic`] prepare in the background; this
    /// entry point remains for headless tests and callers that explicitly want
    /// the runtime before returning.
    pub fn ensure_workbench(&mut self) {
        self.cancel_pending_start();
        self.start_workbench_now();
    }

    /// Whether a workspace runtime is currently being prepared off the UI
    /// thread.
    pub fn preparation_in_flight(&self) -> bool {
        self.pending_start.is_some()
    }

    fn start_workbench_now(&mut self) {
        if !self.start_pending || self.shutdown_done {
            return;
        }
        let prepared_at = Instant::now();
        let source = self.launch.source();
        match &self.launch {
            WorkspaceLaunch::Ready(plan) => {
                let cwd = plan.terminal_cwd().to_path_buf();
                let workbench = Workbench::start_in(
                    self.integrations,
                    Some(plan.clone()),
                    None,
                    cwd,
                    self.terminal_target,
                    prepared_at,
                );
                self.insert_workbench(workbench, source);
            }
            WorkspaceLaunch::TerminalFallback { error, cwd, .. } => {
                let workbench = Workbench::start_in(
                    self.integrations,
                    None,
                    Some(error.clone()),
                    cwd.clone(),
                    self.terminal_target,
                    prepared_at,
                );
                self.insert_workbench(workbench, source);
            }
            WorkspaceLaunch::Picker { .. } => {}
        }
    }

    /// Prepare the next workspace off the UI thread.
    ///
    /// The worker owns bridge creation, config composition, shell startup, and
    /// explorer initialization. Only the finished owner is inserted on the UI
    /// thread; a superseded or cancelled preparation is shut down off-thread.
    fn begin_pending_start(&mut self) {
        if !self.start_pending || self.shutdown_done || self.pending_start.is_some() {
            return;
        }
        let source = self.launch.source();
        let (plan, launch_error, cwd) = match &self.launch {
            WorkspaceLaunch::Ready(plan) => {
                (Some(plan.clone()), None, plan.terminal_cwd().to_path_buf())
            }
            WorkspaceLaunch::TerminalFallback { error, cwd, .. } => {
                (None, Some(error.clone()), cwd.clone())
            }
            WorkspaceLaunch::Picker { .. } => {
                self.start_pending = false;
                return;
            }
        };

        self.start_generation = self.start_generation.wrapping_add(1);
        let generation = self.start_generation;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let terminal_target = self.terminal_target;
        let integrations = self.integrations;
        let prepared_at = Instant::now();
        let (sender, receiver) = mpsc::sync_channel(1);
        let spawn = thread::Builder::new()
            .name("made-workspace-prepare".to_owned())
            .spawn(move || {
                if worker_cancel.load(Ordering::Acquire) {
                    return;
                }
                let mut workbench = Workbench::start_in(
                    integrations,
                    plan,
                    launch_error,
                    cwd,
                    terminal_target,
                    prepared_at,
                );
                if worker_cancel.load(Ordering::Acquire) {
                    workbench.shutdown();
                    return;
                }
                if let Err(mut error) = sender.send(workbench) {
                    error.0.shutdown();
                }
            });
        match spawn {
            Ok(_) => {
                self.pending_start = Some(PendingStart {
                    generation,
                    cancel,
                    receiver,
                    source,
                });
            }
            Err(error) => {
                // Thread creation failure is exceptional. Keep the app usable
                // by falling back to synchronous preparation and report it.
                eprintln!(
                    "made: workspace preparation thread could not start ({error}); preparing synchronously"
                );
                self.start_workbench_now();
            }
        }
    }

    fn poll_pending_start(&mut self) {
        let Some((generation, source, received)) = self.pending_start.as_ref().map(|pending| {
            (
                pending.generation,
                pending.source,
                pending.receiver.try_recv(),
            )
        }) else {
            return;
        };
        match received {
            Ok(workbench) => {
                self.pending_start = None;
                if self.start_pending && !self.shutdown_done && generation == self.start_generation
                {
                    self.insert_workbench(workbench, source);
                } else {
                    retire_workbench(workbench);
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.pending_start = None;
                if generation == self.start_generation {
                    self.start_pending = false;
                    eprintln!("made: workspace preparation ended without a runtime");
                }
            }
        }
    }

    /// Cancel an in-flight preparation. The returned owner, if the worker
    /// already finished, is shut down on a helper thread rather than the UI
    /// thread.
    fn cancel_pending_start(&mut self) {
        let Some(pending) = self.pending_start.take() else {
            return;
        };
        pending.cancel.store(true, Ordering::Release);
        self.start_generation = self.start_generation.wrapping_add(1);
        retire_preparation(pending.receiver);
    }

    /// Adopt the background TUI detection result once it is ready.
    ///
    /// While the worker is pending, schedule another frame so an otherwise idle
    /// window still updates its `+` menu. OpenCode stays on the inactive
    /// not-running sentence until that result arrives. A disconnected worker
    /// leaves the current (empty) list in place and records both TUIs as not
    /// on PATH.
    fn poll_compatible_tui_detection(&mut self, ctx: &egui::Context) {
        if self.app_states.iter().any(|state| state.holds_detection()) {
            return;
        }
        let Some(receiver) = self.compatible_tui_receiver.as_ref() else {
            return;
        };
        match receiver.try_recv() {
            Ok(tuis) => {
                self.notify_detection(&tuis);
                self.compatible_tuis = tuis;
                self.compatible_tui_receiver = None;
                self.announce_notices();
                self.push_integration_diagnostics();
            }
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(crate::metrics::ACTIVE_REPAINT_INTERVAL);
            }
            Err(TryRecvError::Disconnected) => {
                self.compatible_tui_receiver = None;
                if self.notify_detection_failed().diagnostics_changed() {
                    self.push_integration_diagnostics();
                }
                self.announce_notices();
            }
        }
    }

    /// Tell every open workspace what the app-wide states that have a pending
    /// notice know, once.
    fn announce_notices(&mut self) {
        for state in self.app_states.iter_mut() {
            if state.take_pending_notice() {
                for (_, workbench) in &mut self.workspaces {
                    workbench.notify_from(state.as_ref());
                }
            }
        }
    }

    /// Tell every integration's app-wide state what PATH detection found.
    fn notify_detection(&mut self, found: &[IntegrationRef]) -> AppCtx {
        let mut ctx = AppCtx::default();
        for (index, entry) in self.integrations.iter().enumerate() {
            if let Some(state) = self.app_states.get_mut(index) {
                state.on_detection(found.contains(entry), &mut ctx);
            }
        }
        ctx
    }

    /// Tell every integration's app-wide state that PATH detection ended
    /// without a result.
    fn notify_detection_failed(&mut self) -> AppCtx {
        let mut ctx = AppCtx::default();
        for state in self.app_states.iter_mut() {
            state.on_detection_failed(&mut ctx);
        }
        ctx
    }

    /// Apply every integration's app-wide background work.
    fn poll_app_states(&mut self) -> AppCtx {
        let mut ctx = AppCtx::default();
        for state in self.app_states.iter_mut() {
            state.poll(&mut ctx);
        }
        ctx
    }

    /// Poll runtime work and drive the smoke run.
    pub fn logic(&mut self, ctx: &egui::Context) {
        self.poll_compatible_tui_detection(ctx);
        let polled = self.poll_app_states();
        if let Some(delay) = polled.repaint_after() {
            ctx.request_repaint_after(delay);
        }
        self.announce_notices();
        if polled.diagnostics_changed() {
            self.push_integration_diagnostics();
        }
        self.poll_ssh_probe();
        self.poll_remote_grok_install();
        self.poll_remote_closes();
        self.persist_remote_consent();
        self.apply_focus_reset(ctx);
        self.poll_pending_start();
        self.begin_pending_start();
        for (_, workbench) in &mut self.workspaces {
            workbench.tick(ctx);
        }
        self.push_integration_diagnostics();
        self.poll_updates(ctx);
        if ctx.input(|input| input.viewport().close_requested()) {
            self.shutdown();
        }
        self.frame_count = self.frame_count.saturating_add(1);
        self.pump_smoke(ctx);
    }

    fn pump_smoke(&mut self, ctx: &egui::Context) {
        let (frames, elapsed) = {
            let Some(smoke) = self.smoke.as_mut() else {
                return;
            };
            smoke.frames = smoke.frames.saturating_add(1);
            let started = *smoke.started.get_or_insert_with(Instant::now);
            if smoke.reported
                || smoke.frames < 2
                || self.ui_frames == 0
                || started.elapsed() < smoke.duration
            {
                ctx.request_repaint_after(crate::metrics::ACTIVE_REPAINT_INTERVAL);
                return;
            }
            smoke.reported = true;
            (smoke.frames, started.elapsed())
        };
        let status = self.smoke_summary();
        println!(
            "made smoke: window_ready=1 ui_frames={} frames={frames} elapsed_ms={} platform={} {status} {}",
            self.ui_frames,
            elapsed.as_millis(),
            platform_family_label(),
            self.smoke_layout_markers()
        );
        println!("made smoke: OK");
        self.shutdown();
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn smoke_summary(&self) -> String {
        match self.workbench() {
            Some(workbench) => workbench.smoke_summary(),
            None if self.is_picker() => {
                "surface=picker status=degraded reason=no_workbench terminal=[absent] explorer=[absent]"
                    .to_owned()
            }
            None => {
                "surface=starting status=degraded reason=no_workbench terminal=[absent] explorer=[absent]"
                    .to_owned()
            }
        }
    }

    /// Render the current surface.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        // eframe calls `ui` in the same frame that `logic` handles the window
        // close request. Shutdown has already dropped the terminal, so
        // rendering now would paint the terminal-unavailable card into the
        // closing frame.
        if self.shutdown_done {
            return;
        }
        self.apply_focus_reset(ui.ctx());
        self.ui_frames = self.ui_frames.saturating_add(1);
        if !self.theme_ready {
            theme::install(ui.ctx());
            self.theme_ready = true;
        }
        if self.is_picker() {
            ui::picker_ui(self, ui);
            return;
        }
        if ui::workspace_strip(self, ui) {
            self.show_about = !self.show_about;
        }
        ui::ssh_servers_window(self, ui.ctx());
        if self.is_picker() {
            return;
        }
        let active = self.active;
        let native_backdrop_applied = self.native_backdrop.is_applied();
        let mut panel = self.update_panel();
        if let Some((id, workbench)) = self
            .workspaces
            .iter_mut()
            .find(|(id, _)| Some(*id) == active)
        {
            ui.push_id(*id, |ui| {
                workbench.ui(
                    ui,
                    &mut self.show_sessions,
                    &mut self.show_about,
                    &mut panel,
                    native_backdrop_applied,
                )
            });
        } else {
            ui::starting_ui(self, ui);
        }
        if !self.show_about {
            self.update_confirming = false;
        }
        let actions = std::mem::take(&mut panel.actions);
        if !actions.is_empty() {
            let ctx = ui.ctx().clone();
            self.handle_update_actions(&ctx, &actions);
        }
    }

    fn apply_focus_reset(&mut self, ctx: &egui::Context) {
        if !std::mem::take(&mut self.reset_ui_focus) {
            return;
        }
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
        ctx.send_viewport_cmd(egui::ViewportCommand::IMEAllowed(false));
    }

    /// Terminate children, stop the bridge, verify config, and log. Idempotent.
    pub fn shutdown(&mut self) {
        if self.shutdown_done {
            return;
        }
        self.shutdown_done = true;
        self.cancel_pending_start();
        for (_, workbench) in &mut self.workspaces {
            if let Some(watch) = workbench.take_remote_close() {
                self.remote_closes.push(watch);
            }
            workbench.shutdown();
        }
        self.poll_remote_closes();
        let _ = made_platform::host_platform().cleanup();
    }

    /// Number of frames processed. Used by the smoke marker and performance
    /// evidence.
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Number of rendered egui frames.
    pub fn ui_frame_count(&self) -> u64 {
        self.ui_frames
    }

    /// Smoke mode enabled.
    pub fn smoke_mode(&self) -> bool {
        self.smoke.is_some()
    }

    /// Install the update scheduler and the REL-10 preference store. The
    /// default is an unavailable scheduler (non-installed build).
    pub fn set_updates(
        &mut self,
        scheduler: UpdateScheduler,
        store: Option<crate::preferences::PreferenceStore>,
    ) {
        self.updates = scheduler;
        self.update_store = store;
    }

    /// The last command-line apply outcome (fixed wording), shown in the
    /// Diagnostics VERSIONS section and copied report.
    pub fn set_update_apply_outcome(&mut self, outcome: Option<String>) {
        self.update_apply_outcome = outcome;
        for (_, workbench) in &mut self.workspaces {
            workbench
                .diag
                .set_update_apply_outcome(self.update_apply_outcome.clone());
        }
    }

    /// Record an update-related line in every workspace's diagnostics log,
    /// now and for workspaces opened later (bounded).
    pub fn record_update_note(&mut self, level: crate::diagnostics::LogLevel, message: String) {
        let message = crate::sanitize::sanitize_log_text(&message);
        for (_, workbench) in &mut self.workspaces {
            workbench.diag.log_mut().record(level, message.clone());
        }
        self.update_history.push((level, message));
        if self.update_history.len() > 32 {
            self.update_history.remove(0);
        }
    }

    /// The update scheduler.
    pub fn updates(&self) -> &UpdateScheduler {
        &self.updates
    }

    /// Whether a verified update is ready (the wordmark indicator).
    pub fn update_ready(&self) -> bool {
        self.updates.ready_version().is_some()
    }

    /// Whether `Restart now` handed the update to the updater.
    pub fn update_restarting(&self) -> bool {
        self.update_restarting
    }

    fn update_panel(&self) -> UpdatePanel {
        UpdatePanel {
            controls: self.updates.available(),
            status_line: self
                .updates
                .status()
                .line(crate::updates::format_check_time),
            busy: self.updates.busy(),
            automatic: self.updates.automatic(),
            ready: self.updates.ready_version().map(str::to_owned),
            confirming: self.update_confirming && self.updates.ready_version().is_some(),
            notice: self.update_notice.clone(),
            actions: Vec::new(),
        }
    }

    fn poll_updates(&mut self, ctx: &egui::Context) {
        self.updates.tick(Instant::now());
        let events = self.updates.take_events();
        if !events.is_empty() {
            for event in &events {
                let (level, message) = crate::diagnostics::update_log_message(event);
                self.record_update_note(level, message);
            }
            ctx.request_repaint();
        }
        if self.updates.busy() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    /// Apply About Updates actions taken in this frame.
    pub fn handle_update_actions(&mut self, ctx: &egui::Context, actions: &[UpdateAction]) {
        for action in actions {
            match action {
                UpdateAction::CheckNow => self.updates.check_now(),
                UpdateAction::SetAutomatic(enabled) => {
                    self.updates.set_automatic(*enabled, Instant::now());
                    let saved = self
                        .update_store
                        .as_ref()
                        .map(|store| store.set_automatic_checks(*enabled));
                    if let Some(Err(error)) = saved {
                        self.record_update_note(crate::diagnostics::LogLevel::Warn, error);
                    }
                }
                UpdateAction::RequestRestart => {
                    self.update_notice = None;
                    self.update_confirming = self.updates.ready_version().is_some();
                }
                UpdateAction::Dismiss => self.update_confirming = false,
                UpdateAction::RestartNow => self.restart_to_update(ctx),
            }
        }
    }

    /// `Restart now`: close every workspace, then hand the verified update to
    /// the updater and close the window so it can apply and relaunch.
    fn restart_to_update(&mut self, ctx: &egui::Context) {
        self.update_confirming = false;
        if self.updates.ready_version().is_none() {
            return;
        }
        // Refuse before anything closes, for example while another window
        // of this install is open.
        if let Err(reason) = self.updates.restart_check() {
            self.update_notice = Some(reason);
            return;
        }
        self.update_notice = None;
        self.show_about = false;
        self.shutdown();
        match self.updates.restart() {
            Ok(_) => {
                self.update_restarting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(error) => {
                eprintln!("made: could not start the update: {error}");
            }
        }
    }

    /// Smoke markers for the installed-layout check (REL-03, REL-11): the
    /// marker of each integration in list order (the OpenCode plugin source
    /// and `made-grok-status` for the product), companion and askpass
    /// resolution, the update state, the production update repository and
    /// whether the test-only update seam was compiled in.
    pub fn smoke_layout_markers(&self) -> String {
        const COMPANION_TARGETS: [&str; 4] = [
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
        ];
        let paths = made_platform::host_platform().app_paths().ok();
        let markers: String = self
            .app_states
            .iter()
            .filter_map(|state| state.smoke_marker(paths.as_ref()))
            .map(|(name, value)| format!("{name}={value} "))
            .collect();
        let companions = COMPANION_TARGETS
            .iter()
            .filter(|target| crate::ssh_servers::companion_artifact(target).is_some())
            .count();
        let askpass = std::env::current_exe()
            .ok()
            .and_then(|exe| {
                exe.parent()
                    .map(|dir| dir.join(made_platform::askpass_file_name()))
            })
            .is_some_and(|path| path.is_file());
        format!(
            "version={APP_VERSION} {markers}companions={companions}/4 askpass={} updates={} update_repo={} update_seam={}",
            if askpass { "found" } else { "missing" },
            if self.updates.available() { "enabled" } else { "disabled" },
            made_platform::update::UPDATE_REPOSITORY,
            u8::from(made_platform::update::TEST_SEAM_ENABLED),
        )
    }

    /// Version banner.
    pub fn version_banner() -> String {
        format!("{APP_NAME} {APP_VERSION}")
    }
}

fn retire_preparation(receiver: Receiver<Workbench>) {
    let spawn = thread::Builder::new()
        .name("made-workspace-prepare-reaper".to_owned())
        .spawn(move || {
            if let Ok(mut workbench) = receiver.recv() {
                workbench.shutdown();
            }
        });
    if let Err(error) = spawn {
        eprintln!("made: could not start workspace preparation reaper: {error}");
    }
}

fn retire_workbench(mut workbench: Workbench) {
    let spawn = thread::Builder::new()
        .name("made-workspace-reaper".to_owned())
        .spawn(move || workbench.shutdown());
    if let Err(error) = spawn {
        // The closure (and its owner) was dropped when spawning failed; the
        // Workbench Drop implementation still shuts it down on this thread.
        eprintln!("made: could not start workspace reaper: {error}");
    }
}

impl Drop for AppModel {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use made_platform::native_backdrop::NativeBackdropKind;

    #[test]
    fn native_backdrop_status_is_stored_and_exposed() {
        let temp = tempfile::tempdir().unwrap();
        let mut model = AppModel::new(
            LaunchArgs {
                workspace: crate::workspace::WorkspaceSelection::Explicit(
                    temp.path().to_path_buf(),
                ),
                smoke_test: None,
            },
            AppOptions::new(),
        );

        assert_eq!(
            model.native_backdrop().kind(),
            NativeBackdropKind::InAppGlass
        );
        assert!(!model.native_backdrop().is_applied());
        assert!(model
            .native_backdrop()
            .reason()
            .contains("not been checked"));

        model.set_native_backdrop(NativeBackdropStatus {
            kind: NativeBackdropKind::WindowsAcrylic,
            applied: true,
            reason: "test status".to_owned(),
        });
        assert_eq!(
            model.native_backdrop().kind(),
            NativeBackdropKind::WindowsAcrylic
        );
        assert!(model.native_backdrop().is_applied());
        assert_eq!(model.native_backdrop().reason(), "test status");

        model.shutdown();
    }

    #[test]
    fn confirmed_remote_close_shows_no_notice_and_unconfirmed_does() {
        let temp = tempfile::tempdir().unwrap();
        let picker = crate::WorkspacePicker::new(temp.path()).unwrap();
        let mut model = AppModel::from_launch(
            crate::workspace::WorkspaceLaunch::Picker {
                picker,
                error: None,
            },
            AppOptions::new(),
        );

        let (sender, watch) = mpsc::channel();
        model.remote_closes.push(watch);
        sender
            .send(crate::remote_session::CloseReport::Confirmed)
            .unwrap();
        model.poll_remote_closes();
        assert_eq!(model.remote_close_notice(), None);
        assert_eq!(model.ssh_notice(), None);

        let (sender, watch) = mpsc::channel();
        model.remote_closes.push(watch);
        sender
            .send(crate::remote_session::CloseReport::Unconfirmed)
            .unwrap();
        model.poll_remote_closes();
        assert_eq!(
            model.remote_close_notice(),
            Some("remote cleanup unconfirmed")
        );
        assert_eq!(model.ssh_notice(), Some("remote cleanup unconfirmed"));

        model.poll_remote_closes();
        assert_eq!(
            model.remote_close_notice(),
            Some("remote cleanup unconfirmed")
        );

        let (sender, watch) = mpsc::channel();
        model.remote_closes.push(watch);
        sender
            .send(crate::remote_session::CloseReport::Confirmed)
            .unwrap();
        model.poll_remote_closes();
        assert_eq!(model.remote_close_notice(), None);

        model.shutdown();
    }
}
