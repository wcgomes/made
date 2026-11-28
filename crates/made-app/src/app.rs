//! Application model and eframe composition.
//!
//! [`AppModel`] owns the launch state, the folder-picker transition, and the
//! workbench runtime. It is deliberately usable without a display for the UI
//! model test: only [`AppModel::logic`] starts the runtime, while
//! [`AppModel::ui`] renders whichever surface the launch state selects.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use made_platform::native_backdrop::NativeBackdropStatus;
use made_platform::{Platform, PlatformFamily};

use crate::cli::LaunchArgs;
use crate::integrations::{
    AppCtx, AppInit, AppState, IntegrationList, IntegrationRef, Slots, PRODUCT,
};
use crate::open_workspace::{
    ConnectionState as HostConnection, HostButton, HostId, LaunchKind, OpenWorkspace,
};
use crate::ssh_servers::{PrivateMetadataStorage, ProbeAttempt, ServerMetadata, ServerRegistry};
use crate::theme;
use crate::tui::TerminalTarget;
use crate::ui;
use crate::updates::{UpdateAction, UpdatePanel, UpdateScheduler};
use crate::workbench::{platform_family_label, Workbench};
use crate::workspace::{
    LaunchPlan, WorkspaceError, WorkspaceLaunch, WorkspaceSelection, WorkspaceSource,
};
use crate::workspace_picker::{AddressError, WorkspacePicker};
use crate::{APP_NAME, APP_VERSION};
use made_remote::protocol::{AuthorityId, RemotePath};

/// The window and Dock icon, embedded at build time.
///
/// eframe sets it at runtime, which on macOS replaces the bundle's `.icns`, so
/// macOS gets the artwork inset to Apple's icon grid; other platforms get the
/// full-bleed tile. An undecodable PNG yields eframe's "no icon" value.
pub fn app_icon() -> egui::IconData {
    const MACOS_PNG: &[u8] = include_bytes!("../assets/app-icon/made-icon-macos-512.png");
    const FULL_BLEED_PNG: &[u8] = include_bytes!("../assets/app-icon/made-icon-256.png");
    let png = match PlatformFamily::current() {
        PlatformFamily::MacOs => MACOS_PNG,
        PlatformFamily::Linux | PlatformFamily::Windows => FULL_BLEED_PNG,
    };
    eframe::icon_data::from_png_bytes(png).unwrap_or_default()
}

/// Native window options shared by the binary and tests.
///
/// `clipboard_shortcuts` must stay disabled: egui-winit otherwise consumes
/// Ctrl/Cmd+C/X/V before the terminal input encoder sees them.
pub fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Made")
            .with_icon(app_icon())
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
// END remote-path

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
        crate::credential_cache::AttemptSecret,
    )>,
    /// Authorities whose private companion was staged or verified in this
    /// session. The Grok plugin installer needs that helper on the server, so
    /// approval stays unavailable until this set contains the authority.
    remote_grok_provisioned: Vec<made_remote::protocol::AuthorityId>,
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
    ssh_selected: Option<made_remote::protocol::AuthorityId>,
    ssh_notice: Option<String>,
    /// The last success or neutral SSH notice, never shown on the screen.
    ssh_info: Option<String>,
    ssh_host_key_changed: bool,
    probe: Option<ProbeAttempt>,
    /// Password handling of the running connection test.
    probe_secret: crate::credential_cache::AttemptSecret,
    /// The shown SSH card of a connection test or an install, with its owner.
    ssh_card: Option<PromptCard>,
    /// Owner and id of the prompt the draft was typed for.
    draft_for: Option<PromptIdentity>,
    prompt_draft: String,
    remote_closes: Vec<Receiver<crate::remote_session::CloseReport>>,
    remote_close_notice: Option<String>,
    /// The Open workspace screen while it is shown.
    screen: Option<Screen>,
    /// Servers whose companion the screen staged or verified this session.
    screen_staged: Vec<AuthorityId>,
    /// Connection epoch of the screen's browse grants.
    screen_epoch: u64,
    /// Unit tests: the scripted ends of the SSH attempts started instead of
    /// OpenSSH.
    #[cfg(test)]
    seams: Seams,
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
            remote_grok_install_refusals: Vec::new(),
            // END remote-path
            terminal_target: TerminalTarget::Shell,
            smoke,
            ssh: load_ssh_registry(),
            ssh_selected: None,
            ssh_notice: None,
            ssh_info: None,
            ssh_host_key_changed: false,
            probe: None,
            probe_secret: crate::credential_cache::AttemptSecret::default(),
            ssh_card: None,
            draft_for: None,
            prompt_draft: String::new(),
            remote_closes: Vec::new(),
            remote_close_notice: None,
            screen: None,
            screen_staged: Vec::new(),
            screen_epoch: 0,
            #[cfg(test)]
            seams: Seams::default(),
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
            .any(|(authority, ..)| *authority == id)
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
        if let Some((_, previous, _)) = self
            .remote_grok_install_attempts
            .iter_mut()
            .find(|(key, ..)| *key == id)
        {
            previous.cancel();
        }
        self.remote_grok_install_attempts
            .retain(|(key, ..)| *key != id);
        self.drop_card(|card| card.install() == Some(id));
        self.upsert_remote_grok_install(id, RemoteGrokInstallState::Running);
        let secret = crate::credential_cache::AttemptSecret::new(
            crate::credential_cache::CacheKey::new(id, &metadata),
        );
        self.remote_grok_install_attempts.push((
            id,
            crate::ssh_servers::spawn_grok_install(
                metadata,
                crate::APP_VERSION.to_owned(),
                target.to_owned(),
            ),
            secret,
        ));
        self.push_integration_diagnostics();
        self.ssh_info(
            "Approved for this server and version. Installing the official Grok Build plugin.",
        );
        Ok(())
    }

    /// Refuse the Grok plugin install for this session: no consent is stored,
    /// nothing is staged or invoked, and the inventory tier keeps working.
    pub fn refuse_remote_grok_install(&mut self, id: made_remote::protocol::AuthorityId) {
        if !self.remote_grok_install_refusals.contains(&id) {
            self.remote_grok_install_refusals.push(id);
        }
        self.ssh_info(
            "Refused. Nothing was installed or written; session follow uses the remote inventory.",
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
        self.ssh_info("Consent withdrawn. Nothing was uninstalled; no further install can start.");
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
        if self.ssh_card.is_none() {
            // One auth card slot per frame; other attempts keep their prompts
            // queued until the slot frees.
            for (authority, attempt, _) in &self.remote_grok_install_attempts {
                if let Some(prompt) = attempt.poll_prompt() {
                    self.ssh_card = Some(PromptCard::Install(*authority, prompt));
                    break;
                }
            }
        }
        let mut finished = Vec::new();
        self.remote_grok_install_attempts
            .retain_mut(|(authority, attempt, secret)| match attempt.poll() {
                Some(result) => {
                    // A typed password is cached only after authentication.
                    match &result {
                        Ok(_) => secret.authenticated(),
                        Err(error) => secret.failed(Some(error.class)),
                    }
                    finished.push((*authority, result));
                    false
                }
                None => true,
            });
        let any_finished = !finished.is_empty();
        for (authority, result) in finished {
            self.drop_card(|card| card.install() == Some(authority));
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
                    let info = !matches!(state, RemoteGrokInstallState::Failed { .. });
                    self.upsert_remote_grok_install(authority, state);
                    if info {
                        self.ssh_info(notice)
                    } else {
                        self.ssh_notice = Some(notice)
                    }
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
        // BEGIN remote-path (AIA-02)
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
        // END remote-path
    }

    /// PTY child selected for the next workspace preparation.
    pub fn terminal_target(&self) -> TerminalTarget {
        self.terminal_target
    }

    pub(crate) fn set_ssh_notice(&mut self, notice: impl Into<String>) {
        self.ssh_notice = Some(notice.into());
    }

    /// Record a success or neutral SSH notice. The Open workspace screen
    /// shows only failures, so this text never reaches its notice line.
    fn ssh_info(&mut self, notice: impl Into<String>) {
        let notice = notice.into();
        self.ssh_info = Some(notice.clone());
        self.ssh_notice = Some(notice);
    }

    pub(crate) fn ssh_registry(&self) -> Result<&ServerRegistry<PrivateMetadataStorage>, &str> {
        self.ssh.as_ref().map_err(String::as_str)
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
        self.ssh_info(
            "Provisioning approved for this server and version. No password was written to disk.",
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
            self.ssh_info(
                "Companion update approved for this attempt only. Running sessions are not signalled.",
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
        if registry.snapshot(id).is_err() {
            return;
        }
        self.ssh_selected = Some(id);
        self.ssh_host_key_changed = false;
    }

    pub(crate) fn prompt_draft(&mut self) -> &mut String {
        &mut self.prompt_draft
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

    // BEGIN remote-path (AIA-02)
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
        self.drop_card(PromptCard::is_probe);
        self.probe_secret = crate::credential_cache::AttemptSecret::new(
            crate::credential_cache::CacheKey::new(id, request.server.server().metadata()),
        );
        self.probe = Some(self.spawn_probe(request));
        self.ssh_host_key_changed = false;
        self.ssh_notice = None; // a running test shows only the Connecting state
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
        self.ssh_info(
            "Old host key forgotten. Test again to check and approve the new fingerprint.",
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
        self.drop_card(PromptCard::is_probe);
        self.ssh_info("Connection test cancelled.");
    }

    pub(crate) fn submit_ssh_prompt(&mut self, accept: bool) {
        // Answer, decline and cancel go only to the attempt that emitted it.
        let Some(card) = self.ssh_card.take() else {
            return;
        };
        let text = std::mem::take(&mut self.prompt_draft);
        let reply = CardReply::new(card.prompt(), accept, &text);
        match card {
            PromptCard::Install(owner, prompt) => {
                if let Some((_, attempt, attempt_secret)) = self
                    .remote_grok_install_attempts
                    .iter_mut()
                    .find(|(key, ..)| *key == owner)
                {
                    // Held only once the install took it; cached on success.
                    if reply.live && attempt.answer(prompt.id, reply.answer) && reply.typed {
                        attempt_secret.typed(&prompt, &text);
                    }
                    if reply.stop {
                        attempt.cancel();
                    }
                }
            }
            PromptCard::Probe(ticket, prompt) => {
                self.answer_probe_card(
                    ticket,
                    &prompt,
                    reply.answer,
                    reply.typed,
                    reply.stop,
                    &text,
                );
            }
        }
        crate::credential_cache::wipe(text);
        self.note_card_reply(reply.class, accept);
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

    fn poll_ssh_probe(&mut self) {
        if self.ssh_card.is_none() {
            if let Some(probe) = self.probe.as_ref() {
                if let Some(prompt) = probe.poll_prompt() {
                    self.ssh_card = Some(PromptCard::Probe(probe.ticket(), prompt));
                    self.try_cached_ssh_password();
                }
            }
        }
        let finished = self.probe.as_ref().and_then(ProbeAttempt::poll);
        if let Some(result) = finished {
            let ticket = self.probe.as_ref().map(ProbeAttempt::ticket);
            self.probe = None;
            self.drop_card(PromptCard::is_probe);
            match &result {
                Ok(_) => self.probe_secret.authenticated(),
                Err(error) => self.probe_secret.failed(Some(error.class)),
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
            // Success shows only as the host's Connected state and marks.
            if let (Some(ticket), Some(screen)) = (ticket, self.screen.as_mut()) {
                if result.is_ok() || screen.passed == Some(ticket.authority) {
                    screen.passed = result.is_ok().then_some(ticket.authority);
                }
            }
            self.ssh_notice = result.err().map(|error| match error.fingerprint {
                Some(fingerprint) => format!("{}\n{fingerprint}", error.detail),
                None => error.detail,
            });
        }
    }

    /// Answer a password prompt from the process-lifetime cache. The card is
    /// suppressed only when a password is already available for this server
    /// and was not already sent, and rejected, in this attempt.
    fn try_cached_ssh_password(&mut self) {
        let (Some(PromptCard::Probe(ticket, prompt)), Some(probe)) =
            (self.ssh_card.as_ref(), self.probe.as_ref())
        else {
            return;
        };
        if *ticket != probe.ticket() {
            return;
        }
        let Some(answer) = self.probe_secret.on_prompt(prompt) else {
            return;
        };
        let _ = probe.answer(prompt.id, Some(answer));
        self.ssh_card = None;
    }
    // END remote-path

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

    /// Start a new tab whose PTY child is `tui` in `path`, the Open workspace
    /// screen's effective launch path, which is also its explorer root.
    ///
    /// The current tab keeps running and the new tab is selected only when its
    /// runtime is ready. A path that fails validation changes nothing, so the
    /// screen stays open with the error.
    pub fn begin_tui_workspace_in(
        &mut self,
        tui: IntegrationRef,
        path: &Path,
    ) -> Result<(), WorkspaceError> {
        let plan = LaunchPlan::open(path, WorkspaceSource::Picker)?;
        self.cancel_pending_start();
        self.reset_ui_focus = true;
        self.terminal_target = TerminalTarget::Tui(tui);
        self.launch = WorkspaceLaunch::from_plan(plan);
        self.start_pending = true;
        self.show_sessions = false;
        Ok(())
    }

    /// Open a shell workspace in `path`, the Open workspace screen's effective
    /// launch path, with the folder picker's confirmation semantics. A path
    /// that fails validation changes nothing, so the screen stays open with
    /// the error.
    pub fn begin_terminal_workspace_in(&mut self, path: &Path) -> Result<(), WorkspaceError> {
        let plan = LaunchPlan::open(path, WorkspaceSource::Picker)?;
        self.open_plan(plan);
        Ok(())
    }

    /// Start an SSH launch of the Open workspace screen: a new tab bound to
    /// the target's server and to `directory`, the effective launch path the
    /// server just revalidated. Nothing runs on this computer, no other server
    /// is substituted, and a refusal leaves the screen open.
    pub fn begin_remote_workspace_in(
        &mut self,
        target: TerminalTarget,
        directory: made_remote::protocol::RemotePath,
    ) -> Result<(), String> {
        let Some(authority) = target.authority() else {
            return Err("Choose a saved server for an SSH launch.".to_owned());
        };
        let registry = self.ssh.as_ref().map_err(|error| error.clone())?;
        let snapshot = registry
            .snapshot(authority)
            .map_err(|_| "That server is no longer saved. Nothing was started.".to_owned())?;
        if !registry.can_launch_target(target) {
            let label = self
                .integrations
                .iter()
                .find(|tui| {
                    crate::remote_workspace::remote_launch_target(LaunchKind::Tui(**tui), authority)
                        == target
                })
                .map_or(target.program_label(), |tui| tui.descriptor().label);
            return Err(format!(
                "{label} was not detected on that server. Nothing was started locally."
            ));
        }
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
        self.start_pending = false;
        Ok(())
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
        let result = self.launch.picker_toggle(index);
        self.mirror_local_picker();
        result
    }

    /// Toggle expansion of the selected picker row.
    pub fn picker_toggle_selected(&mut self) -> Result<(), crate::workspace_picker::PickerError> {
        let result = self.launch.picker_toggle_selected();
        self.mirror_local_picker();
        result
    }

    /// Expand the selected picker row, or select its first child.
    pub fn picker_expand_selected(&mut self) -> Result<(), crate::workspace_picker::PickerError> {
        let result = self.launch.picker_expand_selected();
        self.mirror_local_picker();
        result
    }

    /// Collapse the selected picker row, or select its parent row.
    pub fn picker_collapse_selected(&mut self) -> Result<(), crate::workspace_picker::PickerError> {
        let result = self.launch.picker_collapse_selected();
        self.mirror_local_picker();
        result
    }

    /// Navigate the picker to the parent folder.
    pub fn picker_up(&mut self) -> Result<(), crate::workspace_picker::PickerError> {
        let result = self.launch.picker_up();
        self.mirror_local_picker();
        result
    }

    /// Move the picker selection.
    pub fn picker_move(&mut self, delta: isize) {
        self.launch.picker_move(delta);
        self.mirror_local_picker();
    }

    /// Select a picker row.
    pub fn picker_select(
        &mut self,
        index: usize,
    ) -> Result<(), crate::workspace_picker::PickerError> {
        let result = self.launch.picker_select(index);
        self.mirror_local_picker();
        result
    }

    /// Refresh the picker listing.
    pub fn picker_refresh(&mut self) -> Result<(), WorkspaceError> {
        let result = self.launch.picker_refresh();
        self.mirror_local_picker();
        result
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
        // BEGIN remote-path (AIA-02)
        self.poll_remote_grok_install();
        // END remote-path
        self.poll_remote_closes();
        self.persist_remote_consent();
        self.sync_screen(ctx);
        self.apply_focus_reset(ctx);
        self.poll_pending_start();
        self.begin_pending_start();
        for (_, workbench) in &mut self.workspaces {
            workbench.tick(ctx);
        }
        let exited: Vec<WorkspaceId> = self
            .workspaces
            .iter()
            .filter(|(_, workbench)| workbench.shell_exited_cleanly())
            .map(|(id, _)| *id)
            .collect();
        for id in exited {
            self.close_workspace(id);
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
        theme::sync_text_color_transfer(ui.ctx());
        ui::install_ssh_field_focus(ui.ctx());
        self.sync_screen(ui.ctx());
        if self.is_picker() {
            ui::picker_ui(self, ui);
            return;
        }
        if ui::workspace_strip(self, ui) {
            self.show_about = !self.show_about;
        }
        self.sync_screen(ui.ctx());
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
        for state in self.app_states.iter_mut() {
            state.shutdown();
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

// ── Open workspace screen ───────────────────────────────────────────────────

/// One folder row of the Open workspace tree, Local or SSH.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenRow {
    /// Sanitized folder name.
    pub name: String,
    /// Full path, for the tooltip.
    pub path: String,
    /// Depth below the root.
    pub depth: usize,
    /// Whether the folder is expanded.
    pub expanded: bool,
}

/// One selectable host of the host bar as the screen renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenHost {
    /// The host.
    pub id: HostId,
    /// `Local` or the saved display name.
    pub name: String,
    /// This computer's host name, or `[user@]host[:port]`.
    pub address: String,
    /// Catalog TUIs detected there, `None` while detection is pending.
    pub detected: Option<Vec<IntegrationRef>>,
    /// Connection state of an SSH host; `None` for Local.
    pub connection: Option<HostConnection>,
}

/// The add/edit host form of the host bar.
#[derive(Debug, Clone)]
pub struct ScreenForm {
    /// `None` adds a host; `Some` edits that record.
    pub editing: Option<AuthorityId>,
    /// The draft. It has no credential field.
    pub form: crate::ssh_servers::HostForm,
    /// One error per invalid field after Save.
    pub errors: Vec<(crate::ssh_servers::HostField, &'static str)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptOwner {
    Stage,
    Browse,
}

/// One folder row of an SSH host's tree.
#[derive(Debug)]
struct RemoteRow {
    path: RemotePath,
    /// Sanitized display name.
    name: String,
    /// Depth below the root.
    depth: usize,
    /// Its children are shown.
    expanded: bool,
    /// Its listing was asked for to expand it; a listing that arrives
    /// without this (or `expanded`) is late and dropped.
    expanding: bool,
}

impl RemoteRow {
    fn new(folder: crate::remote_workspace::RemoteFolder, depth: usize) -> Self {
        Self {
            path: folder.path,
            name: folder.name,
            depth,
            expanded: false,
            expanding: false,
        }
    }
}

/// The folder tree of an SSH host, filled from bounded server listings.
#[derive(Debug, Default)]
struct RemoteTree {
    rows: Vec<RemoteRow>,
    selection: Option<usize>,
    truncated: bool,
    error: Option<String>,
    revision: u64,
}

impl RemoteTree {
    fn reset(&mut self) {
        let revision = self.revision.wrapping_add(1);
        *self = Self::default();
        self.revision = revision;
    }

    fn subtree_end(&self, index: usize) -> usize {
        let depth = self.rows[index].depth;
        self.rows[index + 1..]
            .iter()
            .position(|row| row.depth <= depth)
            .map_or(self.rows.len(), |offset| index + 1 + offset)
    }

    fn collapse(&mut self, index: usize) {
        let end = self.subtree_end(index);
        self.rows.drain(index + 1..end);
        self.rows[index].expanded = false;
        self.rows[index].expanding = false;
        self.revision = self.revision.wrapping_add(1);
        if self
            .selection
            .is_some_and(|selected| selected > index && selected < end)
        {
            self.selection = Some(index);
        } else if let Some(selected) = self.selection.filter(|selected| *selected >= end) {
            self.selection = Some(selected - (end - index - 1));
        }
    }
}

/// Where the screen's SSH browse grant for the selected host stands.
enum RemoteLink {
    /// The connection test (with its host-key and auth prompts) is running.
    Probing,
    /// Waiting for the SSH-06 staging consent.
    Consent { target: String },
    /// Staging or verifying the companion.
    Staging {
        target: String,
        attempt: crate::ssh_servers::StageAttempt,
    },
    /// The open browse grant.
    Browsing(Box<crate::remote_workspace::BrowseConnection<crate::ssh_servers::BrowseStream>>),
    /// Nothing more happens for this selection.
    Stopped(HostConnection),
}

struct RemoteSession {
    id: AuthorityId,
    link: RemoteLink,
    /// Password handling of the current staging or browse attempt.
    secret: crate::credential_cache::AttemptSecret,
}

/// A contextual notice kind of the Open workspace screen's notice line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenNotice {
    /// The address bar's navigation error.
    Address,
    /// Consent refusal, missing home or an ended browse connection.
    Notice,
    /// A listing failure.
    Tree,
    /// A launch that failed validation.
    Launch,
}

impl ScreenNotice {
    /// Every kind, in notice-line order.
    pub const ALL: [Self; 4] = [Self::Address, Self::Notice, Self::Tree, Self::Launch];
}

/// Open workspace screen state, alive while the screen is shown.
struct Screen {
    model: OpenWorkspace,
    git: crate::worktree::WorktreeReads,
    address_error: Option<AddressError>,
    /// The address error is that the SSH host is not connected.
    address_offline: bool,
    /// The SSH address the server is checking; a reply for any other path,
    /// or after the user navigated elsewhere, is stale.
    address_check: Option<RemotePath>,
    remote: Option<RemoteSession>,
    remote_tree: RemoteTree,
    prompt: Option<(made_platform::ssh_askpass::PromptRequest, PromptOwner)>,
    form: Option<ScreenForm>,
    removal: Option<crate::ssh_servers::RemovalPrompt>,
    notice: Option<String>,
    launch_error: Option<String>,
    tested: Option<AuthorityId>,
    /// The host whose explicit connection test last succeeded: its button
    /// shows Connected, the only success feedback of a test.
    passed: Option<AuthorityId>,
    /// The selected host's connection failure was dismissed; it shows again
    /// after the host leaves the failed state.
    failure_dismissed: bool,
    /// The dismissed text of each contextual notice kind; a notice shows
    /// again once its text changes or clears.
    dismissed: [Option<String>; 4],
    rows: (RowsKey, Arc<Vec<ScreenRow>>),
    picker_key: Option<(u64, u64, Option<usize>, PathBuf)>,
}

/// Identity of the rendered rows: host, picker instance, revision and root,
/// or the remote tree revision.
type RowsKey = Option<(HostId, u64, u64, PathBuf)>;

fn picker_key(picker: &WorkspacePicker) -> (u64, u64, Option<usize>, PathBuf) {
    (
        picker.instance_id(),
        picker.revision(),
        picker.selection(),
        picker.current_dir().to_path_buf(),
    )
}

fn remote_path(path: &Path) -> Option<RemotePath> {
    RemotePath::new(path.to_str()?.as_bytes().to_vec()).ok()
}

fn remote_parent(path: &RemotePath) -> Option<RemotePath> {
    let bytes = path.as_bytes();
    if bytes == b"/" {
        return None;
    }
    let index = bytes.iter().rposition(|byte| *byte == b'/')?;
    RemotePath::new(if index == 0 {
        b"/".to_vec()
    } else {
        bytes[..index].to_vec()
    })
    .ok()
}

fn remote_target_triple(target: &made_remote::protocol::RemoteTarget) -> &'static str {
    match target {
        made_remote::protocol::RemoteTarget::LinuxX64 => "x86_64-unknown-linux-gnu",
        made_remote::protocol::RemoteTarget::LinuxArm64 => "aarch64-unknown-linux-gnu",
        made_remote::protocol::RemoteTarget::MacosX64 => "x86_64-apple-darwin",
        made_remote::protocol::RemoteTarget::MacosArm64 => "aarch64-apple-darwin",
    }
}

/// This computer's host name, read once off the UI thread. `None` until the
/// read finishes or when it fails; the screen then shows `This computer`.
pub fn local_host_name() -> Option<String> {
    static NAME: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new();
    let cell = NAME.get_or_init(|| {
        let _ = thread::Builder::new()
            .name("made-host-name".to_owned())
            .spawn(|| {
                let name = std::env::var("COMPUTERNAME")
                    .ok()
                    .or_else(|| {
                        std::process::Command::new("hostname")
                            .stdin(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .output()
                            .ok()
                            .filter(|output| output.status.success())
                            .and_then(|output| String::from_utf8(output.stdout).ok())
                    })
                    .map(|name| name.trim().to_owned())
                    .filter(|name| !name.is_empty());
                if let Some(cell) = NAME.get() {
                    if let Ok(mut slot) = cell.lock() {
                        *slot = name;
                    }
                }
            });
        Mutex::new(None)
    });
    cell.lock().ok().and_then(|name| name.clone())
}

impl AppModel {
    /// The Open workspace screen model while the screen is shown.
    pub fn open_workspace(&self) -> Option<&OpenWorkspace> {
        self.screen.as_ref().map(|screen| &screen.model)
    }

    /// Build the screen when the picker surface shows, drop it (ending any
    /// browse grant) when it closes, and bring its model up to date. Never
    /// blocks: git and SSH work runs elsewhere and is only polled here.
    pub(crate) fn sync_screen(&mut self, ctx: &egui::Context) {
        self.sync_draft();
        if !self.is_picker() {
            if let Some(mut screen) = self.screen.take() {
                end_remote(&mut screen);
            }
            return;
        }
        if self.screen.is_none() {
            let Some(picker) = self.launch.picker() else {
                return;
            };
            let start = picker.current_dir().to_path_buf();
            let hosts = self
                .ssh
                .as_ref()
                .map(|registry| registry.screen_hosts())
                .unwrap_or_default();
            let active = self
                .workbench()
                .map(|workbench| match workbench.remote_authority() {
                    Some(id) => HostId::Ssh(id),
                    None => HostId::Local,
                });
            let model = OpenWorkspace::new(self.integrations, hosts, active, start);
            self.screen = Some(Screen {
                model,
                git: crate::worktree::WorktreeReads::new(crate::worktree::GitRunner::from_env()),
                address_error: None,
                address_offline: false,
                address_check: None,
                remote: None,
                remote_tree: RemoteTree::default(),
                prompt: None,
                form: None,
                removal: None,
                notice: None,
                launch_error: None,
                tested: None,
                passed: None,
                failure_dismissed: false,
                dismissed: Default::default(),
                rows: (None, Arc::new(Vec::new())),
                picker_key: None,
            });
            let selected = self
                .screen
                .as_ref()
                .map(|screen| screen.model.selected_host());
            if let Some(selected @ HostId::Ssh(_)) = selected {
                self.start_remote(selected);
            }
        }
        self.mirror_local_picker();
        self.refresh_host_states();
        self.pump_remote();
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        if let Some(request) = screen.model.sync() {
            match request.host {
                HostId::Local => screen.git.request(request.generation, request.target),
                HostId::Ssh(_) => {
                    if let (
                        Some(RemoteSession {
                            link: RemoteLink::Browsing(connection),
                            ..
                        }),
                        Some(target),
                    ) = (screen.remote.as_mut(), remote_path(&request.target))
                    {
                        let _ = connection.browse_mut().read_git(request.generation, target);
                    }
                }
            }
        }
        for update in screen.git.poll() {
            match update {
                crate::worktree::WorktreeReadUpdate::Target { generation, state } => {
                    screen.model.apply_git(generation, state);
                }
                crate::worktree::WorktreeReadUpdate::Counts {
                    generation,
                    path,
                    counts,
                } => {
                    screen.model.apply_counts(generation, &path, counts);
                }
            }
        }
        let busy = screen.remote.is_some()
            || screen.model.git_state().is_none() && screen.model.launch_target().is_some();
        if busy {
            ctx.request_repaint_after(crate::metrics::ACTIVE_REPAINT_INTERVAL);
        }
        // A prompt that arrived while pumping gets its own draft.
        self.sync_draft();
    }

    /// Keep the screen's Local tree equal to the launch picker, which the
    /// tree's keyboard and pointer actions drive.
    fn mirror_local_picker(&mut self) {
        let (Some(screen), Some(picker)) = (self.screen.as_mut(), self.launch.picker()) else {
            return;
        };
        if screen.model.selected_host() != HostId::Local {
            return;
        }
        let key = picker_key(picker);
        if screen.picker_key.as_ref() == Some(&key) {
            return;
        }
        if let Some(local) = screen.model.local_picker_mut() {
            *local = picker.clone();
        }
        screen.picker_key = Some(key);
    }

    fn refresh_host_states(&mut self) {
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        if !matches!(
            screen.remote.as_ref().map(|session| &session.link),
            Some(RemoteLink::Stopped(HostConnection::Failed(_)))
        ) {
            screen.failure_dismissed = false;
        }
        let current = ScreenNotice::ALL.map(|kind| self.screen_notice_text(kind));
        if let Some(screen) = self.screen.as_mut() {
            for (dismissed, current) in screen.dismissed.iter_mut().zip(current) {
                if current.is_none() {
                    *dismissed = None;
                }
            }
        }
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        screen
            .model
            .set_detected(HostId::Local, self.compatible_tuis.clone());
        let Ok(registry) = self.ssh.as_ref() else {
            return;
        };
        for server in registry.servers() {
            let id = server.id();
            if let crate::ssh_servers::ProbeState::Complete(_) = registry.probe_state(id) {
                let found = self
                    .integrations
                    .iter()
                    .copied()
                    .filter(|tui| {
                        registry.can_launch_target(crate::remote_workspace::remote_launch_target(
                            LaunchKind::Tui(*tui),
                            id,
                        ))
                    })
                    .collect();
                screen.model.set_detected(HostId::Ssh(id), found);
            }
            let state = match (&screen.remote, registry.probe_state(id)) {
                (Some(session), _) if session.id == id => match &session.link {
                    RemoteLink::Probing => {
                        if self
                            .ssh_card
                            .as_ref()
                            .filter(|card| card.is_probe())
                            .is_some_and(|card| {
                                matches!(
                                    crate::remote_session::classify_prompt(&card.prompt().text),
                                    crate::remote_session::SshPromptClass::UnknownHost { .. }
                                )
                            })
                        {
                            HostConnection::HostKeyApprovalNeeded
                        } else {
                            HostConnection::Connecting
                        }
                    }
                    RemoteLink::Consent { .. } | RemoteLink::Staging { .. } => {
                        HostConnection::Connecting
                    }
                    RemoteLink::Browsing(connection) => {
                        if connection.browse().phase() == crate::remote_workspace::BrowsePhase::Open
                        {
                            HostConnection::Connected
                        } else {
                            HostConnection::Connecting
                        }
                    }
                    RemoteLink::Stopped(state) => state.clone(),
                },
                (_, crate::ssh_servers::ProbeState::Running(_)) => HostConnection::Connecting,
                (_, crate::ssh_servers::ProbeState::Failed(code)) => HostConnection::Failed(
                    crate::ssh_servers::probe_failure_detail(code).to_owned(),
                ),
                (_, crate::ssh_servers::ProbeState::Cancelled) => {
                    HostConnection::AuthenticationCancelled
                }
                (_, crate::ssh_servers::ProbeState::Complete(_)) if screen.passed == Some(id) => {
                    HostConnection::Connected
                }
                _ => HostConnection::NotConnected,
            };
            screen.model.set_connection(id, state);
        }
    }

    /// Host bar entries in display order, without `+ Add host`.
    pub fn screen_hosts(&self) -> Vec<ScreenHost> {
        let Some(screen) = self.screen.as_ref() else {
            return Vec::new();
        };
        let entries = self
            .ssh
            .as_ref()
            .map(|registry| registry.host_entries())
            .unwrap_or_default();
        screen
            .model
            .host_buttons()
            .into_iter()
            .filter_map(|button| match button {
                HostButton::Host(HostId::Local) => Some(ScreenHost {
                    id: HostId::Local,
                    name: "Local".to_owned(),
                    address: local_host_name().unwrap_or_else(|| "This computer".to_owned()),
                    detected: (self.compatible_tui_receiver.is_none())
                        .then(|| self.compatible_tuis.clone()),
                    connection: None,
                }),
                HostButton::Host(HostId::Ssh(id)) => {
                    let entry = entries.iter().find(|entry| entry.id == id)?;
                    let detected = self.ssh.as_ref().ok().and_then(|registry| {
                        matches!(
                            registry.probe_state(id),
                            crate::ssh_servers::ProbeState::Complete(_)
                        )
                        .then(|| {
                            self.integrations
                                .iter()
                                .copied()
                                .filter(|tui| {
                                    registry.can_launch_target(
                                        crate::remote_workspace::remote_launch_target(
                                            LaunchKind::Tui(*tui),
                                            id,
                                        ),
                                    )
                                })
                                .collect()
                        })
                    });
                    Some(ScreenHost {
                        id: HostId::Ssh(id),
                        name: entry.name.clone(),
                        address: entry.address.clone(),
                        detected,
                        connection: Some(screen.model.connection(id)),
                    })
                }
                HostButton::AddHost => None,
            })
            .collect()
    }

    /// Select a host: navigation resets to its start folder and an SSH host
    /// starts connecting for browsing only.
    pub fn screen_select_host(&mut self, host: HostId) {
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        if screen.model.selected_host() == host {
            return;
        }
        if !screen.model.select_host(host) {
            return;
        }
        end_remote(screen);
        screen.remote_tree.reset();
        screen.address_error = None;
        screen.notice = None;
        screen.failure_dismissed = false;
        // A host change clears the previous host's test report.
        self.ssh_notice = None;
        screen.launch_error = None;
        screen.picker_key = None;
        match host {
            HostId::Local => {
                if let (Some(local), Some(picker)) =
                    (screen.model.local_picker(), self.launch.picker_mut())
                {
                    *picker = local.clone();
                }
            }
            HostId::Ssh(_) => self.start_remote(host),
        }
    }

    fn start_remote(&mut self, host: HostId) {
        let HostId::Ssh(id) = host else {
            return;
        };
        self.ssh_selected = Some(id);
        let complete = self
            .ssh
            .as_ref()
            .ok()
            .and_then(|registry| match registry.probe_state(id) {
                crate::ssh_servers::ProbeState::Complete(capabilities) => {
                    Some(remote_target_triple(&capabilities.target).to_owned())
                }
                _ => None,
            });
        let link = match complete {
            Some(target) => self.consent_link(id, target),
            None => match self.test_selected_ssh_server() {
                Ok(()) => RemoteLink::Probing,
                Err(error) => RemoteLink::Stopped(HostConnection::Failed(error)),
            },
        };
        let secret = self.attempt_secret(id);
        if let Some(screen) = self.screen.as_mut() {
            screen.remote = Some(RemoteSession { id, link, secret });
        }
    }

    /// Password handling for a new attempt to the saved record `id` as it is
    /// now.
    fn attempt_secret(&self, id: AuthorityId) -> crate::credential_cache::AttemptSecret {
        self.ssh
            .as_ref()
            .ok()
            .and_then(|registry| registry.snapshot(id).ok())
            .map(|snapshot| {
                crate::credential_cache::AttemptSecret::new(crate::credential_cache::CacheKey::new(
                    id,
                    snapshot.server().metadata(),
                ))
            })
            .unwrap_or_default()
    }

    fn consent_link(&mut self, id: AuthorityId, target: String) -> RemoteLink {
        let consented = self.ssh.as_ref().ok().is_some_and(|registry| {
            registry
                .snapshot(id)
                .is_ok_and(|snapshot| snapshot.server().allows_provision(APP_VERSION))
        });
        match crate::remote_workspace::browse_consent(consented, None) {
            crate::remote_workspace::BrowseConsent::Proceed => self.stage_link(id, target),
            crate::remote_workspace::BrowseConsent::Ask => RemoteLink::Consent { target },
            crate::remote_workspace::BrowseConsent::Refused(reason) => {
                if let Some(screen) = self.screen.as_mut() {
                    screen.notice = Some(reason.to_owned());
                }
                RemoteLink::Stopped(HostConnection::NotConnected)
            }
        }
    }

    fn stage_link(&mut self, id: AuthorityId, target: String) -> RemoteLink {
        let Ok(snapshot) = self
            .ssh
            .as_ref()
            .map_err(|_| ())
            .and_then(|registry| registry.snapshot(id).map_err(|_| ()))
        else {
            return RemoteLink::Stopped(HostConnection::NotConnected);
        };
        let metadata = snapshot.server().metadata().clone();
        if self.screen_staged.contains(&id) {
            return self.browse_link(id, &metadata, &target);
        }
        RemoteLink::Staging {
            attempt: self.spawn_stage(metadata, target.clone()),
            target,
        }
    }

    fn browse_link(
        &mut self,
        id: AuthorityId,
        metadata: &ServerMetadata,
        target: &str,
    ) -> RemoteLink {
        self.screen_epoch = self.screen_epoch.wrapping_add(1);
        let browse = match crate::remote_workspace::RemoteBrowse::open(id, self.screen_epoch) {
            Ok(browse) => browse,
            Err(_) => {
                return RemoteLink::Stopped(HostConnection::Failed(
                    "Could not start browsing.".to_owned(),
                ))
            }
        };
        match self.connect_browse(metadata, target, browse) {
            Ok(connection) => RemoteLink::Browsing(Box::new(connection)),
            Err(error) => RemoteLink::Stopped(HostConnection::Failed(error.detail)),
        }
    }

    /// Whether the selected SSH host waits for the staging consent.
    pub fn screen_consent_pending(&self) -> bool {
        self.screen.as_ref().is_some_and(|screen| {
            matches!(
                screen.remote.as_ref().map(|session| &session.link),
                Some(RemoteLink::Consent { .. })
            )
        })
    }

    /// Answer the staging consent. Refusal writes nothing and shows the
    /// reason in the folder area; the host stays listed and selectable.
    pub fn screen_answer_consent(&mut self, approve: bool) {
        let Some((id, target)) = self.screen.as_ref().and_then(|screen| {
            let session = screen.remote.as_ref()?;
            match &session.link {
                RemoteLink::Consent { target } => Some((session.id, target.clone())),
                _ => None,
            }
        }) else {
            return;
        };
        let link = match crate::remote_workspace::browse_consent(false, Some(approve)) {
            crate::remote_workspace::BrowseConsent::Proceed => {
                let saved = self
                    .ssh
                    .as_mut()
                    .map_err(|error| error.clone())
                    .and_then(|registry| {
                        registry
                            .set_consent(id, Some(APP_VERSION.to_owned()))
                            .map_err(|error| error.to_string())
                    });
                match saved {
                    Ok(()) => self.stage_link(id, target),
                    Err(error) => RemoteLink::Stopped(HostConnection::Failed(error)),
                }
            }
            crate::remote_workspace::BrowseConsent::Refused(reason) => {
                if let Some(screen) = self.screen.as_mut() {
                    screen.notice = Some(reason.to_owned());
                }
                RemoteLink::Stopped(HostConnection::NotConnected)
            }
            crate::remote_workspace::BrowseConsent::Ask => RemoteLink::Consent { target },
        };
        if let Some(session) = self
            .screen
            .as_mut()
            .and_then(|screen| screen.remote.as_mut())
        {
            session.link = link;
        }
    }

    fn pump_remote(&mut self) {
        let Some((id, phase)) = self.screen.as_ref().and_then(|screen| {
            let session = screen.remote.as_ref()?;
            Some((
                session.id,
                match &session.link {
                    RemoteLink::Probing => 0,
                    RemoteLink::Staging { .. } => 1,
                    RemoteLink::Browsing(_) => 2,
                    RemoteLink::Consent { .. } | RemoteLink::Stopped(_) => 3,
                },
            ))
        }) else {
            return;
        };
        match phase {
            0 => {
                let state = self
                    .ssh
                    .as_ref()
                    .ok()
                    .map(|registry| registry.probe_state(id));
                let next = match state {
                    Some(crate::ssh_servers::ProbeState::Complete(capabilities)) => {
                        Some(self.consent_link(
                            id,
                            remote_target_triple(&capabilities.target).to_owned(),
                        ))
                    }
                    Some(crate::ssh_servers::ProbeState::Failed(code)) => {
                        Some(RemoteLink::Stopped(HostConnection::Failed(
                            self.ssh_notice.clone().unwrap_or_else(|| {
                                crate::ssh_servers::probe_failure_detail(code).to_owned()
                            }),
                        )))
                    }
                    Some(crate::ssh_servers::ProbeState::Cancelled) | None => {
                        Some(RemoteLink::Stopped(HostConnection::AuthenticationCancelled))
                    }
                    _ => None,
                };
                if let Some(next) = next {
                    if let Some(session) = self
                        .screen
                        .as_mut()
                        .and_then(|screen| screen.remote.as_mut())
                    {
                        session.link = next;
                    }
                }
            }
            1 => self.pump_stage(id),
            2 => self.pump_browse(),
            _ => {}
        }
    }

    fn pump_stage(&mut self, id: AuthorityId) {
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        let Some(RemoteSession {
            link: RemoteLink::Staging { attempt, target },
            secret,
            ..
        }) = screen.remote.as_mut()
        else {
            return;
        };
        if screen.prompt.is_none() {
            if let Some(prompt) = attempt.poll_prompt() {
                if let Some(answer) = secret.on_prompt(&prompt) {
                    attempt.answer(prompt.id, Some(answer));
                } else {
                    screen.prompt = Some((prompt, PromptOwner::Stage));
                }
            }
        }
        let Some(result) = attempt.poll() else {
            return;
        };
        let target = target.clone();
        screen.prompt = None;
        match result {
            Ok(()) => {
                secret.authenticated();
                self.screen_staged.push(id);
                let metadata = self
                    .ssh
                    .as_ref()
                    .ok()
                    .and_then(|registry| registry.snapshot(id).ok())
                    .map(|snapshot| snapshot.server().metadata().clone());
                let link = match metadata {
                    Some(metadata) => self.browse_link(id, &metadata, &target),
                    None => RemoteLink::Stopped(HostConnection::NotConnected),
                };
                let secret = self.attempt_secret(id);
                if let Some(session) = self
                    .screen
                    .as_mut()
                    .and_then(|screen| screen.remote.as_mut())
                {
                    session.link = link;
                    session.secret = secret;
                }
            }
            Err(error) => {
                secret.failed(Some(error.class));
                let state = match error.class {
                    crate::remote_session::SshFailureClass::Cancelled => {
                        HostConnection::AuthenticationCancelled
                    }
                    crate::remote_session::SshFailureClass::ChangedHostKey => {
                        self.ssh_host_key_changed = true;
                        self.ssh_notice = Some(error.detail.clone());
                        HostConnection::Failed(error.detail)
                    }
                    _ => HostConnection::Failed(error.detail),
                };
                if let Some(session) = screen.remote.as_mut() {
                    session.link = RemoteLink::Stopped(state);
                }
            }
        }
    }

    fn pump_browse(&mut self) {
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        let Some(RemoteSession {
            id,
            link: RemoteLink::Browsing(connection),
            secret,
        }) = screen.remote.as_mut()
        else {
            return;
        };
        let id = *id;
        if screen.prompt.is_none() {
            if let Some(prompt) = connection.poll_prompt() {
                if let Some(answer) = secret.on_prompt(&prompt) {
                    let _ = connection.answer_prompt(prompt.id, Some(answer));
                } else {
                    screen.prompt = Some((prompt, PromptOwner::Browse));
                }
            }
        }
        let events = connection.pump(Instant::now());
        let ended = connection.browse().phase() == crate::remote_workspace::BrowsePhase::Ended;
        let end_failure = connection.end_failure();
        let mut new_root: Option<RemotePath> = None;
        let mut launch: Option<(LaunchKind, RemotePath)> = None;
        let mut authenticated = false;
        for event in events {
            use crate::remote_workspace::BrowseEvent;
            match event {
                BrowseEvent::Opened { home } => {
                    authenticated = true;
                    let home = home.as_ref().and_then(crate::open_workspace::remote_key);
                    if let Err(reason) = screen.model.apply_remote_home(home) {
                        screen.notice = Some(reason.to_owned());
                    }
                    new_root = screen.model.remote_root().and_then(remote_path);
                    // A git read asked for before the grant opened was dropped.
                    screen.model.invalidate_git();
                }
                BrowseEvent::Listed {
                    path,
                    folders,
                    truncated,
                } => apply_listing(screen, path, folders, truncated),
                BrowseEvent::ListFailed { path } => {
                    if let Some(row) = screen
                        .remote_tree
                        .rows
                        .iter_mut()
                        .find(|row| row.path == path)
                    {
                        row.expanding = false;
                    }
                    let root = screen.model.remote_root().and_then(remote_path);
                    screen.remote_tree.error = Some(if root.as_ref() == Some(&path) {
                        "This folder could not be listed on the server.".to_owned()
                    } else {
                        "A folder could not be read; it stays collapsed.".to_owned()
                    });
                }
                BrowseEvent::AddressChecked { requested, result } => {
                    // Only the latest address the user is still waiting on applies.
                    if screen.address_check.as_ref() != Some(&requested) {
                        continue;
                    }
                    screen.address_check = None;
                    match result {
                        Ok(path) => {
                            if let Some(key) = crate::open_workspace::remote_key(&path) {
                                screen.model.set_remote_root(key);
                                screen.remote_tree.reset();
                                screen.address_error = None;
                                new_root = Some(path);
                            }
                        }
                        Err(error) => screen.address_error = Some(error),
                    }
                }
                BrowseEvent::LaunchChecked {
                    kind,
                    requested,
                    result,
                } => match revalidated_launch(&screen.model, kind, requested, result) {
                    LaunchCheck::Stale => {}
                    LaunchCheck::Launch(path) => launch = Some((kind, path)),
                    LaunchCheck::Failed(error) => screen.launch_error = Some(error),
                },
                BrowseEvent::Git { generation, state } => {
                    screen.model.apply_git(generation, state);
                }
                BrowseEvent::Counts {
                    generation,
                    path,
                    counts,
                } => {
                    screen.model.apply_counts(generation, &path, counts);
                }
                BrowseEvent::Ended => {}
            }
        }
        if let Some(session) = screen.remote.as_mut() {
            if authenticated {
                session.secret.authenticated();
            }
            if ended {
                session.secret.failed(end_failure);
                if screen
                    .prompt
                    .as_ref()
                    .is_some_and(|(_, owner)| *owner == PromptOwner::Browse)
                {
                    screen.prompt = None;
                }
                let state = match end_failure {
                    Some(class @ crate::remote_session::SshFailureClass::ChangedHostKey) => {
                        self.ssh_host_key_changed = true;
                        self.ssh_selected = Some(id);
                        self.ssh_notice = Some(class.detail().to_owned());
                        HostConnection::Failed(class.detail().to_owned())
                    }
                    _ => HostConnection::Failed(
                        "The browse connection to this server ended. Select the host again to reconnect."
                            .to_owned(),
                    ),
                };
                session.link = RemoteLink::Stopped(state);
            } else if let (Some(root), RemoteLink::Browsing(connection)) =
                (new_root, &mut session.link)
            {
                connection.browse_mut().set_root(root.clone());
                let _ = connection.browse_mut().list(root);
            }
        }
        if let Some((kind, path)) = launch {
            let target = crate::remote_workspace::remote_launch_target(kind, id);
            if let Err(error) = self.begin_remote_workspace_in(target, path) {
                if let Some(screen) = self.screen.as_mut() {
                    screen.launch_error = Some(error);
                }
            }
        }
    }

    /// The interactive SSH prompt the screen shows: the connection test's, or
    /// the screen's own staging or browse attempt's.
    pub fn screen_prompt(&self) -> Option<&made_platform::ssh_askpass::PromptRequest> {
        self.ssh_card.as_ref().map(PromptCard::prompt).or_else(|| {
            self.screen
                .as_ref()
                .and_then(|screen| screen.prompt.as_ref().map(|(prompt, _)| prompt))
        })
    }

    /// Answer the shown prompt. Cancelling stops only that attempt.
    pub fn screen_answer_prompt(&mut self, accept: bool) {
        if let Some(card) = self.ssh_card.as_ref() {
            let changed = matches!(
                crate::remote_session::classify_prompt(&card.prompt().text),
                crate::remote_session::SshPromptClass::ChangedHostKey { .. }
            );
            // Accepting, or declining another attempt's card, goes to that
            // card's owner only.
            if accept && !changed || !card.is_probe() {
                self.submit_ssh_prompt(accept);
                return;
            }
            // Refusing or cancelling stops only this connection attempt.
            let ticket = self.probe.as_ref().map(ProbeAttempt::ticket);
            if changed {
                self.submit_ssh_prompt(false);
            } else {
                self.cancel_ssh_probe();
            }
            if let (Some(ticket), Ok(registry)) = (ticket, self.ssh.as_mut()) {
                registry.cancel_probe(ticket);
            }
            let state = if changed {
                HostConnection::Failed(
                    crate::remote_session::SshFailureClass::ChangedHostKey
                        .detail()
                        .to_owned(),
                )
            } else {
                HostConnection::AuthenticationCancelled
            };
            if let Some(session) = self
                .screen
                .as_mut()
                .and_then(|screen| screen.remote.as_mut())
            {
                if matches!(session.link, RemoteLink::Probing) {
                    session.link = RemoteLink::Stopped(state);
                }
            }
            return;
        }
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        let Some((prompt, owner)) = screen.prompt.take() else {
            return;
        };
        let class = crate::remote_session::classify_prompt(&prompt.text);
        let text = std::mem::take(&mut self.prompt_draft);
        // An expired prompt is never answered: OpenSSH stopped waiting.
        if crate::credential_cache::expired(&prompt) {
            crate::credential_cache::wipe(text);
            return;
        }
        let answer = match class {
            crate::remote_session::SshPromptClass::ChangedHostKey { .. } => None,
            crate::remote_session::SshPromptClass::UnknownHost { .. } if accept => {
                made_platform::ssh_askpass::PromptAnswer::new(b"yes".to_vec()).ok()
            }
            crate::remote_session::SshPromptClass::Secret { .. } if accept => {
                made_platform::ssh_askpass::PromptAnswer::new(text.as_bytes().to_vec()).ok()
            }
            _ => None,
        };
        let Some(session) = screen.remote.as_mut() else {
            crate::credential_cache::wipe(text);
            return;
        };
        let cancel = answer.is_none();
        let taken = match (owner, &mut session.link) {
            (PromptOwner::Stage, RemoteLink::Staging { attempt, .. }) => {
                let taken = attempt.answer(prompt.id, answer);
                if cancel {
                    attempt.cancel();
                }
                taken
            }
            (PromptOwner::Browse, RemoteLink::Browsing(connection)) => {
                connection.answer_prompt(prompt.id, answer).is_ok()
            }
            _ => false,
        };
        // Held only once the attempt took the answer, and cached only once it
        // authenticates.
        if taken && !cancel {
            session.secret.typed(&prompt, &text);
        }
        crate::credential_cache::wipe(text);
    }

    /// Visible rows of the folder tree of the selected host, sanitized once
    /// per tree change.
    pub fn screen_rows(&mut self) -> Arc<Vec<ScreenRow>> {
        let Some(screen) = self.screen.as_mut() else {
            return Arc::new(Vec::new());
        };
        let host = screen.model.selected_host();
        let key = match host {
            HostId::Local => self.launch.picker().map(|picker| {
                (
                    host,
                    picker.instance_id(),
                    picker.revision(),
                    picker.current_dir().to_path_buf(),
                )
            }),
            HostId::Ssh(_) => Some((host, 0, screen.remote_tree.revision, PathBuf::new())),
        };
        if screen.rows.0 == key && key.is_some() {
            return screen.rows.1.clone();
        }
        let rows: Vec<ScreenRow> = match host {
            HostId::Local => self
                .launch
                .picker()
                .map(|picker| {
                    picker
                        .rows()
                        .iter()
                        .map(|row| ScreenRow {
                            name: crate::sanitize::sanitize_display(row.name(), 200),
                            path: row.path().display().to_string(),
                            depth: row.depth(),
                            expanded: row.expanded(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            HostId::Ssh(_) => screen
                .remote_tree
                .rows
                .iter()
                .map(|row| ScreenRow {
                    name: row.name.clone(),
                    path: crate::sanitize::sanitize_display(
                        &String::from_utf8_lossy(row.path.as_bytes()),
                        900,
                    ),
                    depth: row.depth,
                    expanded: row.expanded,
                })
                .collect(),
        };
        screen.rows = (key, Arc::new(rows));
        screen.rows.1.clone()
    }

    /// Selected tree row of the selected host.
    pub fn screen_selection(&self) -> Option<usize> {
        let screen = self.screen.as_ref()?;
        match screen.model.selected_host() {
            HostId::Local => self.launch.picker()?.selection(),
            HostId::Ssh(_) => screen.remote_tree.selection,
        }
    }

    /// Entries left out of the root listing by the 1000-entry cap (Local) or
    /// whether a server listing was capped (SSH).
    pub fn screen_truncated(&self) -> usize {
        let Some(screen) = self.screen.as_ref() else {
            return 0;
        };
        match screen.model.selected_host() {
            HostId::Local => self
                .launch
                .picker()
                .map_or(0, WorkspacePicker::truncated_entries),
            HostId::Ssh(_) => usize::from(screen.remote_tree.truncated),
        }
    }

    /// A non-blocking tree error of the selected host.
    pub fn screen_tree_error(&self) -> Option<String> {
        let screen = self.screen.as_ref()?;
        match screen.model.selected_host() {
            HostId::Local => self
                .launch
                .picker()
                .and_then(|picker| picker.error().map(ToString::to_string)),
            HostId::Ssh(_) => screen.remote_tree.error.clone(),
        }
    }

    /// Click or Enter on a row: select it and toggle its expansion.
    pub fn screen_toggle(&mut self, index: usize) {
        if self.screen_host() == Some(HostId::Local) {
            let _ = self.picker_toggle(index);
            return;
        }
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        if index >= screen.remote_tree.rows.len() {
            return;
        }
        screen.remote_tree.selection = Some(index);
        remote_toggle(screen, index);
        sync_remote_selection(screen);
    }

    /// Up/Down over the visible rows.
    pub fn screen_move(&mut self, delta: isize) {
        if self.screen_host() == Some(HostId::Local) {
            self.picker_move(delta);
            return;
        }
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        let count = screen.remote_tree.rows.len();
        if count == 0 {
            return;
        }
        let next = match screen.remote_tree.selection {
            None if delta >= 0 => 0,
            None => count - 1,
            Some(current) => (current as isize + delta).clamp(0, count as isize - 1) as usize,
        };
        screen.remote_tree.selection = Some(next);
        sync_remote_selection(screen);
    }

    /// Right: expand the selected folder or select its first child.
    pub fn screen_expand(&mut self) {
        if self.screen_host() == Some(HostId::Local) {
            let _ = self.picker_expand_selected();
            return;
        }
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        let Some(index) = screen.remote_tree.selection else {
            return;
        };
        if screen.remote_tree.rows[index].expanded {
            if screen
                .remote_tree
                .rows
                .get(index + 1)
                .is_some_and(|row| row.depth > screen.remote_tree.rows[index].depth)
            {
                screen.remote_tree.selection = Some(index + 1);
            }
        } else if !screen.remote_tree.rows[index].expanding {
            remote_toggle(screen, index);
        }
        sync_remote_selection(screen);
    }

    /// Left: collapse the selected folder or select its parent row.
    pub fn screen_collapse(&mut self) {
        if self.screen_host() == Some(HostId::Local) {
            let _ = self.picker_collapse_selected();
            return;
        }
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        let Some(index) = screen.remote_tree.selection else {
            return;
        };
        if screen.remote_tree.rows[index].expanded {
            screen.remote_tree.collapse(index);
        } else if screen.remote_tree.rows[index].expanding {
            // Collapsed before its children arrived.
            screen.remote_tree.rows[index].expanding = false;
        } else {
            let depth = screen.remote_tree.rows[index].depth;
            if let Some(parent) = screen.remote_tree.rows[..index]
                .iter()
                .rposition(|row| row.depth < depth)
            {
                screen.remote_tree.selection = Some(parent);
            }
        }
        sync_remote_selection(screen);
    }

    /// Enter: toggle the selected folder without launching.
    pub fn screen_toggle_selected(&mut self) {
        if let Some(index) = self.screen_selection() {
            self.screen_toggle(index);
        }
    }

    /// Backspace or Up: re-root at the parent folder.
    pub fn screen_up(&mut self) {
        if self.screen_host() == Some(HostId::Local) {
            let _ = self.picker_up();
            return;
        }
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        let Some(parent) = screen
            .model
            .remote_root()
            .and_then(remote_path)
            .and_then(|root| remote_parent(&root))
        else {
            return;
        };
        if let Some(RemoteSession {
            link: RemoteLink::Browsing(connection),
            ..
        }) = screen.remote.as_mut()
        {
            if let Some(key) = crate::open_workspace::remote_key(&parent) {
                screen.model.set_remote_root(key);
                screen.remote_tree.reset();
                screen.address_check = None;
                connection.browse_mut().set_root(parent.clone());
                let _ = connection.browse_mut().list(parent);
            }
        }
    }

    /// Re-read the root and every expanded folder.
    pub fn screen_refresh(&mut self) {
        if self.screen_host() == Some(HostId::Local) {
            let _ = self.picker_refresh();
            if let Some(screen) = self.screen.as_mut() {
                // A refresh re-reads git too.
                screen.picker_key = None;
                screen.model.invalidate_git();
            }
            return;
        }
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        let Some(root) = screen.model.remote_root().and_then(remote_path) else {
            return;
        };
        if let Some(RemoteSession {
            link: RemoteLink::Browsing(connection),
            ..
        }) = screen.remote.as_mut()
        {
            let expanded: Vec<RemotePath> = screen
                .remote_tree
                .rows
                .iter()
                .filter(|row| row.expanded)
                .map(|row| row.path.clone())
                .collect();
            let _ = connection.browse_mut().list(root);
            for path in expanded {
                let _ = connection.browse_mut().list(path);
            }
        }
        screen.model.invalidate_git();
    }

    /// Navigate to an address-bar path. Local paths are checked at once; an
    /// SSH path is checked on the server and applied when it answers.
    pub fn screen_navigate(&mut self, text: &str) -> Result<(), AddressError> {
        let host = self.screen_host();
        let Some(screen) = self.screen.as_mut() else {
            return Ok(());
        };
        screen.address_error = None;
        let mut offline = false;
        let result = match host {
            Some(HostId::Local) => match self.launch.picker_mut() {
                Some(picker) => picker.navigate_to(text),
                None => Ok(()),
            },
            Some(HostId::Ssh(_)) => match crate::remote_workspace::parse_remote_address(text) {
                Ok(path) => match screen.remote.as_mut() {
                    Some(RemoteSession {
                        link: RemoteLink::Browsing(connection),
                        ..
                    }) => match connection.browse_mut().check_address(path.clone()) {
                        Ok(_) => {
                            screen.address_check = Some(path);
                            Ok(())
                        }
                        Err(_) => Err(AddressError::NotReadable),
                    },
                    _ => {
                        offline = true;
                        Err(AddressError::NotReadable)
                    }
                },
                Err(error) => Err(error),
            },
            None => Ok(()),
        };
        screen.address_offline = offline;
        if let Err(error) = result {
            screen.address_error = Some(error);
        }
        self.mirror_local_picker();
        result
    }

    /// The inline address-bar reason, if the last navigation failed.
    pub fn screen_address_message(&self) -> Option<&'static str> {
        let screen = self.screen.as_ref()?;
        let error = screen.address_error?;
        Some(if screen.address_offline {
            "This server is not connected. Select the host again to reconnect"
        } else {
            error.message()
        })
    }

    /// The inline address-bar error, if the last navigation failed.
    pub fn screen_address_error(&self) -> Option<AddressError> {
        self.screen.as_ref()?.address_error
    }

    /// Drop the address-bar error (Escape restores the launch target).
    pub fn screen_clear_address_error(&mut self) {
        if let Some(screen) = self.screen.as_mut() {
            screen.address_error = None;
        }
    }

    /// Select a Worktrees item.
    pub fn screen_select_worktree(&mut self, index: usize) -> bool {
        self.screen
            .as_mut()
            .is_some_and(|screen| screen.model.select_worktree(index))
    }

    /// Activate a launch button. Local launches start at once; an SSH launch
    /// first revalidates the path on the server and never runs locally. A
    /// failure keeps the screen open with a contextual error.
    pub fn screen_launch(&mut self, kind: LaunchKind) {
        let Some(request) = self
            .screen
            .as_ref()
            .and_then(|screen| screen.model.launch_request(kind))
        else {
            return;
        };
        if let Some(screen) = self.screen.as_mut() {
            screen.launch_error = None;
        }
        let error = match request.local_target() {
            Some(TerminalTarget::Tui(tui)) => self
                .begin_tui_workspace_in(tui, &request.path)
                .err()
                .map(|error| error.message()),
            Some(_) => self
                .begin_terminal_workspace_in(&request.path)
                .err()
                .map(|error| error.message()),
            None => {
                let screen = self.screen.as_mut().expect("screen is shown");
                match (screen.remote.as_mut(), remote_path(&request.path)) {
                    (
                        Some(RemoteSession {
                            link: RemoteLink::Browsing(connection),
                            ..
                        }),
                        Some(path),
                    ) => connection
                        .browse_mut()
                        .check_launch(kind, path)
                        .err()
                        .map(|_| {
                            "The server could not check that folder. Nothing was started."
                                .to_owned()
                        }),
                    _ => Some("The server is not connected. Nothing was started.".to_owned()),
                }
            }
        };
        if let (Some(error), Some(screen)) = (error, self.screen.as_mut()) {
            screen.launch_error = Some(error);
        }
    }

    /// The contextual launch or folder-area notice, if any.
    pub fn screen_launch_error(&self) -> Option<&str> {
        self.screen.as_ref()?.launch_error.as_deref()
    }

    /// The folder-area notice of the selected SSH host (consent refusal,
    /// missing home, ended connection).
    pub fn screen_notice(&self) -> Option<&str> {
        self.screen.as_ref()?.notice.as_deref()
    }

    fn screen_host(&self) -> Option<HostId> {
        Some(self.screen.as_ref()?.model.selected_host())
    }

    /// Open the inline add-host form.
    pub fn screen_add_host(&mut self) {
        if let Some(screen) = self.screen.as_mut() {
            screen.form = Some(ScreenForm {
                editing: None,
                form: crate::ssh_servers::HostForm::default(),
                errors: Vec::new(),
            });
        }
    }

    /// Open the form prefilled with a saved record.
    pub fn screen_edit_host(&mut self, id: AuthorityId) {
        let Some(form) = self
            .ssh
            .as_ref()
            .ok()
            .and_then(|registry| registry.edit_form(id))
        else {
            return;
        };
        if let Some(screen) = self.screen.as_mut() {
            screen.form = Some(ScreenForm {
                editing: Some(id),
                form,
                errors: Vec::new(),
            });
        }
    }

    /// The open host form.
    pub fn screen_form(&self) -> Option<&ScreenForm> {
        self.screen.as_ref()?.form.as_ref()
    }

    /// The open host form's draft.
    pub fn screen_form_mut(&mut self) -> Option<&mut crate::ssh_servers::HostForm> {
        Some(&mut self.screen.as_mut()?.form.as_mut()?.form)
    }

    /// Cancel discards the draft.
    pub fn screen_cancel_form(&mut self) {
        if let Some(screen) = self.screen.as_mut() {
            screen.form = None;
        }
    }

    /// Validate and save the form. Add appends the record and selects it;
    /// Edit keeps live tabs on their original record.
    pub fn screen_save_form(&mut self) -> bool {
        let Some(draft) = self.screen.as_ref().and_then(|screen| screen.form.clone()) else {
            return false;
        };
        if let Err(errors) = draft.form.validate() {
            if let Some(form) = self.screen.as_mut().and_then(|screen| screen.form.as_mut()) {
                form.errors = errors;
            }
            return false;
        }
        let Ok(registry) = self.ssh.as_mut() else {
            return false;
        };
        let saved = match draft.editing {
            Some(id) => registry.edit_host(id, &draft.form).map(|_| {
                crate::credential_cache::forget_server(id);
                id
            }),
            None => registry.add_host(&draft.form),
        };
        let Ok(id) = saved else {
            if let Some(form) = self.screen.as_mut().and_then(|screen| screen.form.as_mut()) {
                form.errors = vec![(
                    crate::ssh_servers::HostField::Name,
                    "The host could not be saved. Check the details and the saved host limit.",
                )];
            }
            return false;
        };
        let hosts = registry.screen_hosts();
        // An edited record is another server for the screen: its staging no
        // longer counts and its live grant ends.
        if draft.editing.is_some() {
            self.screen_staged.retain(|staged| *staged != id);
        }
        let reconnect = draft.editing.is_some() && self.screen_host() == Some(HostId::Ssh(id));
        if let Some(screen) = self.screen.as_mut() {
            screen.form = None;
            screen.model.set_hosts(hosts);
            if reconnect {
                end_remote(screen);
                screen.remote_tree.reset();
                screen.address_error = None;
                screen.notice = None;
                screen.launch_error = None;
                screen.model.select_host(HostId::Ssh(id));
            }
        }
        if reconnect {
            self.start_remote(HostId::Ssh(id));
        }
        if draft.editing.is_none() {
            self.screen_select_host(HostId::Ssh(id));
        }
        true
    }

    /// Ask to remove a saved host; nothing is removed until confirmed.
    pub fn screen_request_remove(&mut self, id: AuthorityId) {
        let prompt = self
            .ssh
            .as_ref()
            .ok()
            .and_then(|registry| registry.removal_prompt(id));
        if let Some(screen) = self.screen.as_mut() {
            screen.removal = prompt;
        }
    }

    /// The pending removal confirmation text.
    pub fn screen_removal(&self) -> Option<&str> {
        Some(self.screen.as_ref()?.removal.as_ref()?.message())
    }

    /// Confirm or cancel the pending removal. Open tabs on that server stay
    /// open with their own record snapshot.
    pub fn screen_answer_removal(&mut self, confirm: bool) {
        let Some(prompt) = self
            .screen
            .as_mut()
            .and_then(|screen| screen.removal.take())
        else {
            return;
        };
        if !confirm {
            return;
        }
        let id = prompt.id();
        let Ok(registry) = self.ssh.as_mut() else {
            return;
        };
        if registry.remove_confirmed(prompt).is_err() {
            return;
        }
        crate::credential_cache::forget_server(id);
        let hosts = registry.screen_hosts();
        if self.ssh_selected == Some(id) {
            self.ssh_selected = None;
        }
        self.screen_staged.retain(|staged| *staged != id);
        if let Some(screen) = self.screen.as_mut() {
            if screen
                .remote
                .as_ref()
                .is_some_and(|session| session.id == id)
            {
                end_remote(screen);
                screen.remote_tree.reset();
            }
            screen.model.set_hosts(hosts);
        }
        self.mirror_after_host_change();
    }

    fn mirror_after_host_change(&mut self) {
        if let Some(screen) = self.screen.as_mut() {
            if screen.model.selected_host() == HostId::Local {
                if let (Some(local), Some(picker)) =
                    (screen.model.local_picker(), self.launch.picker_mut())
                {
                    if picker.instance_id() != local.instance_id() {
                        *picker = local.clone();
                    }
                }
            }
        }
    }

    /// Run the explicit connection test and remote detection for a host. The
    /// result is reported on that host button.
    pub fn screen_test_host(&mut self, id: AuthorityId) {
        if let Some(screen) = self.screen.as_mut() {
            screen.tested = Some(id);
        }
        self.ssh_selected = Some(id);
        if let Err(error) = self.test_selected_ssh_server() {
            self.ssh_notice = Some(error);
        }
    }

    /// The latest SSH report (connection test, consent, install) and the host
    /// it belongs to, while that host is selected or was just tested.
    pub fn screen_test_report(&self) -> Option<(AuthorityId, &str)> {
        let screen = self.screen.as_ref()?;
        let host = self.ssh_selected?;
        (screen.tested == Some(host) || screen.model.selected_host() == HostId::Ssh(host))
            .then_some(())?;
        let report = self.ssh_notice.as_deref()?;
        // Successes add no text; the selected host's failure is written once,
        // as its error line.
        (self.ssh_info.as_deref() != Some(report) && self.selected_failure() != Some(report))
            .then_some((host, report))
    }

    fn selected_failure(&self) -> Option<&str> {
        match &self.screen.as_ref()?.remote.as_ref()?.link {
            RemoteLink::Stopped(HostConnection::Failed(text)) => Some(text),
            _ => None,
        }
    }

    /// The selected SSH host's connection, consent, staging or browse
    /// failure, unless dismissed.
    pub fn screen_connection_failure(&self) -> Option<&str> {
        let text = self.selected_failure()?;
        (!self.screen.as_ref()?.failure_dismissed && !text.trim().is_empty()).then_some(text)
    }

    /// The tree area's text when it has no rows: the empty-folder text only
    /// for a listing Made actually has; `Connecting…` while an SSH host is
    /// connecting; nothing while it failed or waits for the user.
    pub fn screen_empty_text(&self) -> Option<&'static str> {
        const EMPTY: &str = "No child folders here.";
        let screen = self.screen.as_ref()?;
        if screen.model.selected_host() == HostId::Local {
            return Some(EMPTY);
        }
        match &screen.remote.as_ref()?.link {
            RemoteLink::Probing | RemoteLink::Staging { .. } => Some("Connecting…"),
            RemoteLink::Browsing(connection) => {
                let open =
                    connection.browse().phase() == crate::remote_workspace::BrowsePhase::Open;
                match (open, screen.model.remote_root()) {
                    (true, Some(_)) => Some(EMPTY),
                    (true, None) => None,
                    (false, _) => Some("Connecting…"),
                }
            }
            RemoteLink::Consent { .. } | RemoteLink::Stopped(_) => None,
        }
    }

    /// The raw text of one contextual notice kind, dismissed or not.
    pub fn screen_notice_text(&self, kind: ScreenNotice) -> Option<String> {
        match kind {
            ScreenNotice::Address => self.screen_address_message().map(str::to_owned),
            ScreenNotice::Notice => self.screen_notice().map(str::to_owned),
            ScreenNotice::Tree => self.screen_tree_error(),
            ScreenNotice::Launch => self.screen_launch_error().map(str::to_owned),
        }
    }

    /// The contextual notices to show, in order, without dismissed ones.
    pub fn screen_visible_notices(&self) -> Vec<(ScreenNotice, String)> {
        let Some(screen) = self.screen.as_ref() else {
            return Vec::new();
        };
        ScreenNotice::ALL
            .into_iter()
            .filter_map(|kind| {
                let text = self.screen_notice_text(kind)?;
                (screen.dismissed[kind as usize].as_deref() != Some(text.as_str()))
                    .then_some((kind, text))
            })
            .collect()
    }

    /// Dismiss one contextual notice until its text changes or clears.
    pub fn screen_dismiss_notice(&mut self, kind: ScreenNotice) {
        let text = self.screen_notice_text(kind);
        if let Some(screen) = self.screen.as_mut() {
            screen.dismissed[kind as usize] = text;
        }
    }

    /// Dismiss the selected host's connection failure; its red status stays.
    pub fn screen_dismiss_connection_failure(&mut self) {
        if let Some(screen) = self.screen.as_mut() {
            screen.failure_dismissed = true;
        }
    }

    /// Dismiss the shown SSH report; the screen stays open.
    pub fn screen_dismiss_test_report(&mut self) {
        if self.screen_test_report().is_some() {
            self.ssh_notice = None;
        }
    }
}

impl AppModel {
    /// A draft belongs to the prompt it was typed for; another prompt, or
    /// none, starts from an empty one.
    fn sync_draft(&mut self) {
        let shown = self.prompt_identity();
        if shown != self.draft_for {
            crate::credential_cache::wipe_in_place(&mut self.prompt_draft);
            self.draft_for = shown;
        }
    }

    /// Owner and id of the prompt shown now, the card's before the screen's.
    fn prompt_identity(&self) -> Option<PromptIdentity> {
        if let Some(card) = self.ssh_card.as_ref() {
            return Some(match card {
                PromptCard::Probe(ticket, prompt) => {
                    (0, ticket.authority, ticket.attempt, prompt.id)
                }
                PromptCard::Install(authority, prompt) => (1, *authority, 0, prompt.id),
            });
        }
        let screen = self.screen.as_ref()?;
        let (prompt, owner) = screen.prompt.as_ref()?;
        let session = screen.remote.as_ref()?;
        Some((
            match owner {
                PromptOwner::Stage => 2,
                PromptOwner::Browse => 3,
            },
            session.id,
            0,
            prompt.id,
        ))
    }

    /// Drop the shown card when `owned` says its attempt ended, was replaced
    /// or cancelled, and wipe its draft in the same place, so a card is never
    /// answered after its owner is gone.
    fn drop_card(&mut self, owned: impl Fn(&PromptCard) -> bool) {
        if self.ssh_card.as_ref().is_some_and(owned) {
            self.ssh_card = None;
            crate::credential_cache::wipe_in_place(&mut self.prompt_draft);
        }
    }

    /// Deliver a card answer to the connection test that emitted it. A card
    /// of an earlier test reaches nothing.
    fn answer_probe_card(
        &mut self,
        ticket: crate::ssh_servers::ProbeTicket,
        prompt: &made_platform::ssh_askpass::PromptRequest,
        answer: Option<made_platform::ssh_askpass::PromptAnswer>,
        typed: bool,
        stop: bool,
        text: &str,
    ) {
        let Some(probe) = self.probe.as_ref().filter(|probe| probe.ticket() == ticket) else {
            return;
        };
        // An expired prompt is never answered: OpenSSH stopped waiting.
        if crate::credential_cache::expired(prompt) {
            return;
        }
        // Held only once the test took the answer; cached on success.
        if probe.answer(prompt.id, answer) && typed {
            self.probe_secret.typed(prompt, text);
        }
        if stop {
            probe.cancel();
            self.probe = None;
        }
    }

    fn note_card_reply(&mut self, class: crate::remote_session::SshPromptClass, accept: bool) {
        match class {
            crate::remote_session::SshPromptClass::ChangedHostKey { .. } => {
                self.ssh_host_key_changed = true;
                self.ssh_notice = Some(
                    crate::remote_session::SshFailureClass::ChangedHostKey
                        .detail()
                        .to_owned(),
                );
            }
            crate::remote_session::SshPromptClass::UnknownHost { .. } if !accept => {
                self.ssh_notice = Some(
                    crate::remote_session::SshFailureClass::Cancelled
                        .detail()
                        .to_owned(),
                );
            }
            _ => {}
        }
    }

    fn spawn_probe(&mut self, request: crate::ssh_servers::ProbeRequest) -> ProbeAttempt {
        #[cfg(test)]
        {
            let (attempt, script) = crate::ssh_servers::scripted_probe(request.ticket);
            self.seams.probes.push(script);
            attempt
        }
        #[cfg(not(test))]
        crate::ssh_servers::spawn_probe(request)
    }

    fn spawn_stage(
        &mut self,
        metadata: ServerMetadata,
        target: String,
    ) -> crate::ssh_servers::StageAttempt {
        #[cfg(test)]
        {
            let _ = (metadata, target);
            let (attempt, script) = crate::ssh_servers::scripted_stage();
            self.seams.stages.push(script);
            attempt
        }
        #[cfg(not(test))]
        crate::ssh_servers::spawn_consented_stage(metadata, APP_VERSION.to_owned(), target)
    }

    fn connect_browse(
        &mut self,
        metadata: &ServerMetadata,
        target: &str,
        browse: crate::remote_workspace::RemoteBrowse,
    ) -> Result<
        crate::remote_workspace::BrowseConnection<crate::ssh_servers::BrowseStream>,
        crate::ssh_servers::AttemptError,
    > {
        #[cfg(test)]
        {
            let _ = (metadata, target);
            let server = crate::ssh_servers::ScriptedServer::default();
            let stream = server.stream();
            self.seams.servers.push(server);
            Ok(crate::remote_workspace::BrowseConnection::new(
                stream, browse,
            ))
        }
        #[cfg(not(test))]
        crate::ssh_servers::connect_browse(metadata, APP_VERSION, target, browse)
    }
}

/// The SSH card shown for a connection test or an install, always with the
/// attempt that emitted it: no card exists without its owner.
enum PromptCard {
    /// Emitted by the connection test with this ticket.
    Probe(
        crate::ssh_servers::ProbeTicket,
        made_platform::ssh_askpass::PromptRequest,
    ),
    /// Emitted by this server's install attempt.
    Install(AuthorityId, made_platform::ssh_askpass::PromptRequest),
}

impl PromptCard {
    fn prompt(&self) -> &made_platform::ssh_askpass::PromptRequest {
        match self {
            Self::Probe(_, prompt) | Self::Install(_, prompt) => prompt,
        }
    }

    fn is_probe(&self) -> bool {
        matches!(self, Self::Probe(..))
    }

    fn install(&self) -> Option<AuthorityId> {
        match self {
            Self::Install(authority, _) => Some(*authority),
            Self::Probe(..) => None,
        }
    }
}

/// Owner kind, server, attempt and prompt id of a shown prompt.
type PromptIdentity = (u8, AuthorityId, u64, u64);

/// How one answer to a card is delivered.
struct CardReply {
    class: crate::remote_session::SshPromptClass,
    /// The prompt is still within its deadline; an expired one is never
    /// answered.
    live: bool,
    answer: Option<made_platform::ssh_askpass::PromptAnswer>,
    /// A typed password the owner may cache once it authenticates.
    typed: bool,
    /// The owner's attempt stops: a changed key always, other cards on decline.
    stop: bool,
}

impl CardReply {
    fn new(prompt: &made_platform::ssh_askpass::PromptRequest, accept: bool, text: &str) -> Self {
        use crate::remote_session::SshPromptClass;
        let class = crate::remote_session::classify_prompt(&prompt.text);
        let answer = match (&class, accept) {
            (SshPromptClass::UnknownHost { .. }, true) => b"yes".to_vec(),
            (SshPromptClass::Secret { .. }, true) => text.as_bytes().to_vec(),
            _ => Vec::new(),
        };
        let given = accept && !matches!(class, SshPromptClass::ChangedHostKey { .. });
        let secret = matches!(class, SshPromptClass::Secret { .. });
        let live = !crate::credential_cache::expired(prompt);
        Self {
            live,
            answer: given
                .then(|| made_platform::ssh_askpass::PromptAnswer::new(answer).ok())
                .flatten(),
            typed: secret && accept,
            stop: live
                && (matches!(class, SshPromptClass::ChangedHostKey { .. }) || (!accept && !secret)),
            class,
        }
    }
}

/// The scripted far ends of the SSH attempts unit tests start: no OpenSSH
/// runs under test.
#[cfg(test)]
#[derive(Default)]
struct Seams {
    probes: Vec<crate::ssh_servers::ScriptedAttempt<crate::ssh_servers::ProbeReport>>,
    stages: Vec<crate::ssh_servers::ScriptedAttempt<()>>,
    servers: Vec<crate::ssh_servers::ScriptedServer>,
}

/// Close the screen's browse grant, if any. No tab is affected.
fn end_remote(screen: &mut Screen) {
    if let Some(mut session) = screen.remote.take() {
        match &mut session.link {
            RemoteLink::Browsing(connection) => {
                connection.browse_mut().close();
                let _ = connection.pump(Instant::now());
                connection.browse_mut().end();
            }
            RemoteLink::Staging { attempt, .. } => attempt.cancel(),
            _ => {}
        }
    }
    screen.prompt = None;
    screen.address_check = None;
}

/// What a launch revalidation reply starts.
#[derive(Debug, PartialEq, Eq)]
enum LaunchCheck {
    /// The host, button or effective launch path changed since the check was
    /// sent: nothing starts and nothing is shown.
    Stale,
    /// Start in this path: the effective launch path the user chose, which the
    /// server just confirmed. A different canonical path it reports (a
    /// symlink resolved there) is never substituted.
    Launch(RemotePath),
    /// The folder vanished or became unreadable; nothing starts.
    Failed(String),
}

fn revalidated_launch(
    model: &OpenWorkspace,
    kind: LaunchKind,
    requested: RemotePath,
    result: Result<RemotePath, AddressError>,
) -> LaunchCheck {
    let current = model
        .launch_request(kind)
        .and_then(|request| remote_path(&request.path));
    if current.as_ref() != Some(&requested) {
        return LaunchCheck::Stale;
    }
    match result {
        Ok(_) => LaunchCheck::Launch(requested),
        Err(error) => LaunchCheck::Failed(format!(
            "{}: {}. Nothing was started.",
            crate::sanitize::sanitize_display(&String::from_utf8_lossy(requested.as_bytes()), 900),
            error.message()
        )),
    }
}

fn apply_listing(
    screen: &mut Screen,
    path: RemotePath,
    folders: Vec<crate::remote_workspace::RemoteFolder>,
    truncated: bool,
) {
    let tree = &mut screen.remote_tree;
    let root = screen.model.remote_root().and_then(remote_path);
    let selected = tree
        .selection
        .and_then(|index| tree.rows.get(index))
        .map(|row| row.path.clone());
    if root.as_ref() == Some(&path) {
        // A folder still listed keeps its expanded subtree; a folder no longer
        // listed leaves with its whole subtree.
        let mut previous = std::mem::take(&mut tree.rows);
        for folder in folders {
            match previous
                .iter()
                .position(|row| row.depth == 0 && row.path == folder.path)
            {
                Some(start) => {
                    let end = previous[start + 1..]
                        .iter()
                        .position(|row| row.depth == 0)
                        .map_or(previous.len(), |offset| start + 1 + offset);
                    tree.rows.extend(previous.drain(start..end));
                }
                None => tree.rows.push(RemoteRow::new(folder, 0)),
            }
        }
        tree.truncated = truncated;
        tree.error = None;
    } else {
        let Some(index) = tree.rows.iter().position(|row| row.path == path) else {
            return;
        };
        let row = &mut tree.rows[index];
        if !row.expanded && !row.expanding {
            // Collapsed since it was asked for: a late listing changes nothing.
            return;
        }
        row.expanded = true;
        row.expanding = false;
        let depth = row.depth;
        let end = tree.subtree_end(index);
        // As at the root: a child still listed keeps its expanded subtree.
        let mut previous: Vec<RemoteRow> = tree.rows.drain(index + 1..end).collect();
        let mut children = Vec::with_capacity(folders.len());
        for folder in folders {
            match previous
                .iter()
                .position(|row| row.depth == depth + 1 && row.path == folder.path)
            {
                Some(start) => {
                    let end = previous[start + 1..]
                        .iter()
                        .position(|row| row.depth <= depth + 1)
                        .map_or(previous.len(), |offset| start + 1 + offset);
                    children.extend(previous.drain(start..end));
                }
                None => children.push(RemoteRow::new(folder, depth + 1)),
            }
        }
        tree.rows.splice(index + 1..index + 1, children);
    }
    tree.revision = tree.revision.wrapping_add(1);
    tree.selection = selected.and_then(|path| tree.rows.iter().position(|row| row.path == path));
    sync_remote_selection(screen);
}

fn remote_toggle(screen: &mut Screen, index: usize) {
    let row = &mut screen.remote_tree.rows[index];
    if row.expanded {
        screen.remote_tree.collapse(index);
        return;
    }
    if row.expanding {
        // A second toggle before the listing arrives cancels the expansion.
        row.expanding = false;
        return;
    }
    let path = row.path.clone();
    if let Some(RemoteSession {
        link: RemoteLink::Browsing(connection),
        ..
    }) = screen.remote.as_mut()
    {
        if connection.browse_mut().list(path).is_ok() {
            screen.remote_tree.rows[index].expanding = true;
        }
    }
}

fn sync_remote_selection(screen: &mut Screen) {
    let path = screen
        .remote_tree
        .selection
        .and_then(|index| screen.remote_tree.rows.get(index))
        .and_then(|row| crate::open_workspace::remote_key(&row.path));
    screen.model.set_remote_selection(path);
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

    // ── Open workspace SSH glue ─────────────────────────────────────────────

    use crate::remote_session::SshFailureClass;
    use crate::ssh_servers::{AttemptError, ProbeReport, ScriptedServer};
    use made_platform::ssh::SshEvent;
    use made_platform::ssh_askpass::PromptRequest;
    use made_remote::protocol::{
        BrowseEnvelope, BrowseOperation, Capabilities, DirectoryCheck, DirectoryEntry, EntryKind,
        RemoteTarget,
    };

    const PASSWORD: &str = "me@lab.invalid's password: ";

    fn path(text: &str) -> RemotePath {
        RemotePath::new(text.as_bytes().to_vec()).unwrap()
    }

    fn capabilities() -> Capabilities {
        Capabilities {
            target: RemoteTarget::LinuxX64,
            tmux: false,
            opencode: false,
            shell_cwd: false,
            helper_version: "probe".to_owned(),
            grok: false,
            python3: false,
        }
    }

    fn prompt(id: u64, text: &str) -> PromptRequest {
        PromptRequest {
            id,
            text: text.to_owned(),
            expires_at: Instant::now() + Duration::from_secs(300),
        }
    }

    fn failure(class: SshFailureClass) -> AttemptError {
        AttemptError {
            class,
            detail: class.detail().to_owned(),
            fingerprint: None,
        }
    }

    /// The Open workspace screen over a temp folder with one saved SSH host
    /// in a private temp registry. Every SSH attempt is scripted.
    struct Fixture {
        model: AppModel,
        id: AuthorityId,
        ctx: egui::Context,
        server: Option<ScriptedServer>,
        pending: Vec<BrowseEnvelope>,
        _temp: tempfile::TempDir,
    }

    impl Fixture {
        /// The host is saved and shown; `detected` finishes its detection.
        fn new(initial: Option<&str>, detected: bool) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let start = temp.path().join("start");
            std::fs::create_dir(&start).unwrap();
            let picker = crate::WorkspacePicker::new(&start).unwrap();
            let mut model = AppModel::from_launch(
                crate::workspace::WorkspaceLaunch::Picker {
                    picker,
                    error: None,
                },
                AppOptions::new(),
            );
            model.ssh = Ok(ServerRegistry::load(PrivateMetadataStorage::new(
                temp.path().join("registry"),
            ))
            .unwrap());
            let id = model
                .add_ssh_metadata(ServerMetadata {
                    display_name: "lab".into(),
                    host: "lab.invalid".into(),
                    user: None,
                    port: None,
                    key_file: None,
                    initial_directory: initial.map(path),
                })
                .unwrap();
            if detected {
                model.hold_ssh_probe_result(id, Ok(capabilities())).unwrap();
            }
            let ctx = egui::Context::default();
            model.sync_screen(&ctx);
            Self {
                model,
                id,
                ctx,
                server: None,
                pending: Vec::new(),
                _temp: temp,
            }
        }

        /// A detected host, selected: it waits for the staging consent.
        fn consent(initial: Option<&str>) -> Self {
            let mut fixture = Self::new(initial, true);
            fixture.model.screen_select_host(HostId::Ssh(fixture.id));
            assert!(fixture.model.screen_consent_pending());
            fixture
        }

        /// Approve the consent: staging runs (scripted).
        fn staging(initial: Option<&str>) -> Self {
            let mut fixture = Self::consent(initial);
            fixture.model.screen_answer_consent(true);
            fixture.sync();
            assert!(fixture.link_is(|link| matches!(link, RemoteLink::Staging { .. })));
            fixture
        }

        /// Staging succeeded: the browse grant is opening on a scripted server.
        fn browsing(initial: Option<&str>) -> Self {
            let mut fixture = Self::staging(initial);
            fixture.stage().done.send(Ok(())).unwrap();
            fixture.sync();
            fixture.server = Some(fixture.model.seams.servers.last().unwrap().clone());
            fixture
        }

        /// The server answered the open with `home`.
        fn opened(initial: Option<&str>, home: Option<&str>) -> Self {
            let mut fixture = Self::browsing(initial);
            let open = fixture
                .request(|operation| *operation == BrowseOperation::BrowseOpen)
                .expect("the open request");
            fixture.server().reply(
                &open,
                BrowseOperation::BrowseOpened {
                    home: home.map(path),
                },
            );
            fixture.sync();
            assert_eq!(fixture.connection(), HostConnection::Connected);
            fixture
        }

        fn sync(&mut self) {
            self.model.sync_screen(&self.ctx);
        }

        /// The cache key of the saved record as it is now.
        fn key(&self) -> crate::credential_cache::CacheKey {
            let registry = self.model.ssh.as_ref().unwrap();
            let snapshot = registry.snapshot(self.id).unwrap();
            crate::credential_cache::CacheKey::new(self.id, snapshot.server().metadata())
        }

        fn probe_ticket(&self) -> crate::ssh_servers::ProbeTicket {
            self.model.probe.as_ref().expect("a probe").ticket()
        }

        fn probe(&self) -> &crate::ssh_servers::ScriptedAttempt<ProbeReport> {
            self.model.seams.probes.last().expect("a probe attempt")
        }

        fn stage(&self) -> &crate::ssh_servers::ScriptedAttempt<()> {
            self.model.seams.stages.last().expect("a stage attempt")
        }

        fn server(&self) -> ScriptedServer {
            self.server.clone().expect("a browse grant")
        }

        fn screen(&self) -> &Screen {
            self.model.screen.as_ref().expect("the screen")
        }

        fn link_is(&self, check: impl Fn(&RemoteLink) -> bool) -> bool {
            self.screen()
                .remote
                .as_ref()
                .is_some_and(|session| check(&session.link))
        }

        fn connection(&mut self) -> HostConnection {
            self.model.refresh_host_states();
            self.model.open_workspace().unwrap().connection(self.id)
        }

        /// The first unanswered request matching `wanted`, after a pump.
        fn request(&mut self, wanted: impl Fn(&BrowseOperation) -> bool) -> Option<BrowseEnvelope> {
            self.sync();
            self.pending.extend(self.server().requests());
            let index = self
                .pending
                .iter()
                .position(|request| wanted(&request.operation))?;
            Some(self.pending.remove(index))
        }

        /// Answer the listing of `dir` with `folders`.
        fn list(&mut self, dir: &str, folders: &[&str]) {
            let request = self
                .request(|operation| {
                    matches!(operation, BrowseOperation::BrowseList { path: at } if *at == path(dir))
                })
                .unwrap_or_else(|| panic!("a listing request for {dir}"));
            self.server().reply(
                &request,
                BrowseOperation::BrowseListed {
                    path: path(dir),
                    entries: folders
                        .iter()
                        .map(|folder| DirectoryEntry {
                            path: path(folder),
                            kind: EntryKind::Directory,
                        })
                        .collect(),
                    truncated: false,
                    done: true,
                },
            );
            self.sync();
        }

        /// Answer the check of `requested` as the directory `canonical`.
        fn check(&mut self, requested: &str, canonical: &str) {
            let request = self
                .request(|operation| {
                    matches!(operation, BrowseOperation::BrowseCheck { path: at } if *at == path(requested))
                })
                .unwrap_or_else(|| panic!("a check request for {requested}"));
            self.server().reply(
                &request,
                BrowseOperation::BrowseChecked {
                    requested: path(requested),
                    result: DirectoryCheck::Directory {
                        path: path(canonical),
                    },
                },
            );
            self.sync();
        }

        fn rows(&mut self) -> Vec<(String, usize, bool)> {
            self.model
                .screen_rows()
                .iter()
                .map(|row| (row.name.clone(), row.depth, row.expanded))
                .collect()
        }

        fn root(&self) -> Option<PathBuf> {
            self.model
                .open_workspace()
                .unwrap()
                .remote_root()
                .map(Path::to_path_buf)
        }

        fn answer(&mut self, text: &str, accept: bool) {
            *self.model.prompt_draft() = text.to_owned();
            self.model.screen_answer_prompt(accept);
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            crate::credential_cache::forget_server(self.id);
        }
    }

    /// Render one full UI frame of the screen with `events`.
    fn frame(fixture: &mut Fixture, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let ctx = fixture.ctx.clone();
        let mut output = ctx.run_ui(raw, |ui| fixture.model.ui(ui));
        output.textures_delta.clear();
    }

    /// Render one frame and return the painted text.
    fn frame_text(fixture: &mut Fixture, events: Vec<egui::Event>) -> String {
        fn visit(shape: &egui::Shape, text: &mut String) {
            match shape {
                egui::Shape::Text(shape) => {
                    text.push_str(&shape.galley.job.text);
                    text.push('\n');
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| visit(shape, text)),
                _ => {}
            }
        }
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let ctx = fixture.ctx.clone();
        let mut output = ctx.run_ui(raw, |ui| fixture.model.ui(ui));
        output.textures_delta.clear();
        let mut text = String::new();
        for clipped in &output.shapes {
            visit(&clipped.shape, &mut text);
        }
        text
    }

    fn finish_test(fixture: &mut Fixture, result: Result<ProbeReport, AttemptError>) {
        fixture.probe().done.send(result).unwrap();
        fixture.model.poll_ssh_probe();
        fixture.sync();
    }

    fn opencode_report() -> ProbeReport {
        ProbeReport {
            capabilities: Capabilities {
                opencode: true,
                ..capabilities()
            },
            claude: false,
        }
    }

    /// Painted texts (text, color, rect) and filled rects (rect, fill).
    type Painted = (
        Vec<(String, egui::Color32, egui::Rect)>,
        Vec<(egui::Rect, egui::Color32)>,
    );

    /// Painted text (with its color) and filled rects of one frame.
    fn frame_paint(fixture: &mut Fixture, events: Vec<egui::Event>) -> Painted {
        fn visit(
            shape: &egui::Shape,
            texts: &mut Vec<(String, egui::Color32, egui::Rect)>,
            rects: &mut Vec<(egui::Rect, egui::Color32)>,
        ) {
            match shape {
                egui::Shape::Text(text) => {
                    let color = text
                        .galley
                        .job
                        .sections
                        .first()
                        .map_or(text.fallback_color, |section| section.format.color);
                    texts.push((
                        text.galley.job.text.clone(),
                        color,
                        egui::Rect::from_min_size(text.pos, text.galley.size()),
                    ));
                }
                egui::Shape::Rect(rect) => rects.push((rect.rect, rect.fill)),
                egui::Shape::Vec(shapes) => {
                    shapes.iter().for_each(|shape| visit(shape, texts, rects));
                }
                _ => {}
            }
        }
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let ctx = fixture.ctx.clone();
        let mut output = ctx.run_ui(raw, |ui| fixture.model.ui(ui));
        output.textures_delta.clear();
        let (mut texts, mut rects) = (Vec::new(), Vec::new());
        for clipped in &output.shapes {
            visit(&clipped.shape, &mut texts, &mut rects);
        }
        (texts, rects)
    }

    const MISSING: &str = "A packaged companion file is missing. Made did not start a local shell.";

    fn fail_stage(fixture: &mut Fixture) {
        fixture
            .stage()
            .done
            .send(Err(AttemptError {
                class: SshFailureClass::Other,
                detail: MISSING.to_owned(),
                fingerprint: None,
            }))
            .unwrap();
        fixture.sync();
        for _ in 0..2 {
            frame(fixture, Vec::new());
        }
    }

    fn click(fixture: &mut Fixture, id: egui::Id) {
        let at = fixture
            .ctx
            .read_response(id)
            .expect("rendered")
            .rect
            .center();
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(fixture, vec![egui::Event::PointerMoved(at)]);
        frame(fixture, vec![button(true)]);
        frame(fixture, vec![button(false)]);
        frame(
            fixture,
            vec![egui::Event::PointerMoved(egui::pos2(2.0, 2.0))],
        );
    }

    fn press(fixture: &mut Fixture, key: egui::Key) {
        for pressed in [true, false] {
            frame(
                fixture,
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
    }

    #[test]
    fn a_staging_failure_is_an_error_line_with_text_and_never_a_blank_panel() {
        let mut fixture = Fixture::staging(None);
        fail_stage(&mut fixture);
        assert_eq!(fixture.model.screen_connection_failure(), Some(MISSING));
        let (texts, rects) = frame_paint(&mut fixture, Vec::new());
        let line = texts
            .iter()
            .find(|(text, ..)| text.contains("A packaged companion file is missing"))
            .unwrap_or_else(|| panic!("the failure is written on screen: {texts:?}"));
        assert_eq!(line.1, crate::theme::ERROR, "ERROR color");
        assert!(
            texts.iter().any(|(text, color, rect)| text == "!"
                && *color == crate::theme::ERROR
                && rect.right() <= line.2.left()),
            "an error glyph leads the line"
        );
        // No panel row is painted without content.
        let toolbar = fixture
            .ctx
            .read_response(crate::ui::picker_control_id("up"))
            .unwrap()
            .rect;
        for (rect, fill) in &rects {
            if *fill == crate::theme::BG_HEADER && rect.top() < toolbar.top() {
                assert!(
                    texts
                        .iter()
                        .any(|(text, _, at)| !text.trim().is_empty() && rect.contains_rect(*at)),
                    "an empty panel row at {rect:?}"
                );
            }
        }
        let dismiss = crate::ui::picker_control_id("dismiss-failure");
        let control = fixture.ctx.read_response(dismiss).expect("dismiss").rect;
        assert!(control.width() >= 24.0 && control.height() >= 24.0);
        assert!(
            line.2.right() <= control.left(),
            "text and control never overlap"
        );
        // Pointer dismissal keeps the red status and the screen.
        click(&mut fixture, dismiss);
        assert_eq!(fixture.model.screen_connection_failure(), None);
        assert_eq!(
            fixture.connection(),
            HostConnection::Failed(MISSING.to_owned())
        );
        assert!(fixture.model.is_picker());
        let (texts, _) = frame_paint(&mut fixture, Vec::new());
        assert!(!texts
            .iter()
            .any(|(text, ..)| text.contains("companion file")));
    }

    #[test]
    fn the_empty_folder_text_shows_only_for_a_known_listing() {
        const EMPTY: &str = "No child folders here.";
        let painted = |fixture: &mut Fixture| {
            frame(fixture, Vec::new());
            let (texts, _) = frame_paint(fixture, Vec::new());
            texts.into_iter().map(|(text, ..)| text).collect::<Vec<_>>()
        };
        // Waiting for consent: no root yet, nothing in the tree area.
        let mut fixture = Fixture::consent(None);
        assert_eq!(fixture.model.screen_empty_text(), None);
        // Staging is connecting.
        fixture.model.screen_answer_consent(true);
        fixture.sync();
        assert_eq!(fixture.model.screen_empty_text(), Some("Connecting…"));
        // A failure, dismissed or not, leaves the tree area blank.
        fail_stage(&mut fixture);
        assert_eq!(fixture.model.screen_empty_text(), None);
        assert!(!painted(&mut fixture).iter().any(|text| text == EMPTY));
        fixture.model.screen_dismiss_connection_failure();
        assert_eq!(fixture.model.screen_empty_text(), None);
        assert!(!painted(&mut fixture).iter().any(|text| text == EMPTY));
        // Local keeps the empty text.
        fixture.model.screen_select_host(HostId::Local);
        fixture.sync();
        assert_eq!(fixture.model.screen_empty_text(), Some(EMPTY));
        assert!(painted(&mut fixture).iter().any(|text| text == EMPTY));
        // A connected remote's empty folder keeps it too.
        let mut fixture = Fixture::opened(None, Some("/home/me"));
        fixture.list("/home/me", &[]);
        assert_eq!(fixture.model.screen_empty_text(), Some(EMPTY));
        assert!(painted(&mut fixture).iter().any(|text| text == EMPTY));
    }

    #[test]
    fn a_connection_failure_dismisses_with_escape_and_clears_on_host_change_or_success() {
        let mut fixture = Fixture::staging(None);
        fail_stage(&mut fixture);
        let dismiss = crate::ui::picker_control_id("dismiss-failure");
        fixture
            .ctx
            .memory_mut(|memory| memory.request_focus(dismiss));
        frame(&mut fixture, Vec::new());
        frame(&mut fixture, Vec::new());
        press(&mut fixture, egui::Key::Escape);
        assert_eq!(fixture.model.screen_connection_failure(), None);
        assert!(fixture.model.is_picker(), "Escape closes only the notice");
        // A host change clears it; reselecting stages again and a new
        // failure shows again.
        fixture.model.screen_select_host(HostId::Local);
        fixture.sync();
        assert_eq!(fixture.model.screen_connection_failure(), None);
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        fixture.sync();
        assert!(fixture.link_is(|link| matches!(link, RemoteLink::Staging { .. })));
        fail_stage(&mut fixture);
        assert_eq!(fixture.model.screen_connection_failure(), Some(MISSING));
        // A later success clears it.
        fixture.model.screen_select_host(HostId::Local);
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        fixture.sync();
        fixture.stage().done.send(Ok(())).unwrap();
        fixture.sync();
        assert_eq!(fixture.model.screen_connection_failure(), None);
    }

    #[test]
    fn a_probe_failure_of_the_selected_host_is_written_once() {
        let mut fixture = Fixture::new(None, false);
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        finish_test(&mut fixture, Err(failure(SshFailureClass::AuthFailed)));
        fixture.sync();
        let detail = SshFailureClass::AuthFailed.detail();
        assert!(matches!(fixture.connection(), HostConnection::Failed(_)));
        frame(&mut fixture, Vec::new());
        let (texts, _) = frame_paint(&mut fixture, Vec::new());
        let lines: Vec<_> = texts
            .iter()
            .filter(|(text, ..)| text.contains(detail))
            .collect();
        assert_eq!(lines.len(), 1, "the failure is written once: {texts:?}");
        assert_eq!(lines[0].1, crate::theme::ERROR, "as the error line");
        assert_eq!(fixture.model.screen_connection_failure(), Some(detail));
    }

    #[test]
    fn a_remote_launch_of_an_undetected_program_names_its_catalog_label() {
        let mut fixture = Fixture::new(None, true);
        for (tui, label) in [
            (crate::integrations::OPENCODE, "OpenCode"),
            (crate::integrations::GROK, "Grok Build"),
            (crate::integrations::CLAUDE_CODE, "Claude Code"),
        ] {
            let target =
                crate::remote_workspace::remote_launch_target(LaunchKind::Tui(tui), fixture.id);
            assert_eq!(
                fixture.model.begin_remote_workspace_in(target, path("/")),
                Err(format!(
                    "{label} was not detected on that server. Nothing was started locally."
                ))
            );
        }
    }

    #[test]
    fn a_successful_test_shows_no_report_only_the_connected_host_and_its_marks() {
        let mut fixture = Fixture::new(None, false);
        fixture.model.screen_test_host(fixture.id);
        // While it runs: only the Connecting state, no text.
        assert_eq!(fixture.model.screen_test_report(), None);
        assert_eq!(fixture.connection(), HostConnection::Connecting);
        finish_test(&mut fixture, Ok(opencode_report()));
        assert_eq!(fixture.model.screen_test_report(), None);
        assert_eq!(fixture.model.ssh_notice(), None);
        assert_eq!(fixture.connection(), HostConnection::Connected);
        let host = fixture
            .model
            .screen_hosts()
            .into_iter()
            .find(|host| host.id == HostId::Ssh(fixture.id))
            .unwrap();
        assert_eq!(host.connection, Some(HostConnection::Connected));
        assert!(
            host.detected
                .as_ref()
                .is_some_and(|found| found.iter().any(|tui| tui.descriptor().label == "OpenCode")),
            "the detected TUI marks: {host:?}"
        );
        let label = crate::ui::host_accessible_label(&host);
        assert!(
            label.contains("Connected") && label.contains("detected: OpenCode"),
            "{label}"
        );
        let mut text = String::new();
        for _ in 0..2 {
            text = frame_text(&mut fixture, Vec::new());
        }
        for wording in ["Connection succeeded", "was detected", "was not started"] {
            assert!(!text.contains(wording), "no success notice: {text}");
        }
        assert!(fixture
            .ctx
            .read_response(crate::ui::picker_control_id("dismiss-report"))
            .is_none());
    }

    #[test]
    fn success_and_neutral_ssh_notices_add_no_text_on_the_screen() {
        let mut fixture = Fixture::new(None, false);
        fixture.model.screen_test_host(fixture.id);
        fixture.model.cancel_ssh_probe();
        assert_eq!(fixture.model.screen_test_report(), None);
        fixture.model.refuse_remote_grok_install(fixture.id);
        assert_eq!(fixture.model.screen_test_report(), None);
        let text = frame_text(&mut fixture, Vec::new());
        for wording in ["cancelled", "Refused", "approved", "forgotten"] {
            assert!(!text.contains(wording), "{wording}: {text}");
        }
        // A failure afterwards is still shown.
        fixture.model.screen_test_host(fixture.id);
        finish_test(&mut fixture, Err(failure(SshFailureClass::AuthFailed)));
        assert!(fixture.model.screen_test_report().is_some());
    }

    #[test]
    fn every_notice_kind_is_dismissible_and_returns_when_it_changes() {
        use crate::app::ScreenNotice;
        let mut fixture = Fixture::consent(None);
        // Consent refusal: a screen notice, dismissed with Escape on its control.
        fixture.model.screen_answer_consent(false);
        fixture.sync();
        let kinds = |fixture: &Fixture| -> Vec<ScreenNotice> {
            fixture
                .model
                .screen_visible_notices()
                .into_iter()
                .map(|(kind, _)| kind)
                .collect()
        };
        assert_eq!(kinds(&fixture), [ScreenNotice::Notice]);
        let dismiss = crate::ui::picker_control_id("dismiss-notice");
        for _ in 0..2 {
            frame(&mut fixture, Vec::new());
        }
        let (texts, _) = frame_paint(&mut fixture, Vec::new());
        let notice = fixture.model.screen_notice().unwrap().to_owned();
        assert!(texts
            .iter()
            .any(|(text, color, _)| text.contains(&notice) && *color == crate::theme::ERROR));
        assert!(fixture.ctx.read_response(dismiss).is_some());
        fixture
            .ctx
            .memory_mut(|memory| memory.request_focus(dismiss));
        frame(&mut fixture, Vec::new());
        frame(&mut fixture, Vec::new());
        press(&mut fixture, egui::Key::Escape);
        assert!(kinds(&fixture).is_empty());
        assert!(fixture.model.is_picker(), "Escape closes only the notice");
        // Tree and launch errors: dismissed by pointer; a new text returns.
        for (kind, id) in [
            (ScreenNotice::Tree, "dismiss-tree"),
            (ScreenNotice::Launch, "dismiss-launch"),
        ] {
            let set = |fixture: &mut Fixture, text: &str| {
                let screen = fixture.model.screen.as_mut().unwrap();
                match kind {
                    ScreenNotice::Tree => screen.remote_tree.error = Some(text.to_owned()),
                    _ => screen.launch_error = Some(text.to_owned()),
                }
            };
            set(&mut fixture, "first failure");
            fixture.sync();
            for _ in 0..2 {
                frame(&mut fixture, Vec::new());
            }
            assert!(kinds(&fixture).contains(&kind));
            click(&mut fixture, crate::ui::picker_control_id(id));
            assert!(!kinds(&fixture).contains(&kind), "{kind:?} dismissed");
            set(&mut fixture, "another failure");
            assert!(kinds(&fixture).contains(&kind), "{kind:?} changed");
            fixture.model.screen_dismiss_notice(kind);
            assert!(!kinds(&fixture).contains(&kind));
        }
        // A cleared error resets the dismissal: the same text shows again.
        fixture.model.screen.as_mut().unwrap().launch_error = None;
        fixture.sync();
        fixture.model.screen.as_mut().unwrap().launch_error = Some("another failure".to_owned());
        assert!(kinds(&fixture).contains(&ScreenNotice::Launch));
    }

    #[test]
    fn an_address_error_is_dismissible() {
        use crate::app::ScreenNotice;
        let mut fixture = Fixture::new(None, false);
        let missing = fixture._temp.path().join("missing-folder");
        assert!(fixture
            .model
            .screen_navigate(&missing.display().to_string())
            .is_err());
        fixture.sync();
        assert!(fixture
            .model
            .screen_visible_notices()
            .iter()
            .any(|(kind, _)| *kind == ScreenNotice::Address));
        for _ in 0..2 {
            frame(&mut fixture, Vec::new());
        }
        click(
            &mut fixture,
            crate::ui::picker_control_id("dismiss-address"),
        );
        assert!(!fixture
            .model
            .screen_visible_notices()
            .iter()
            .any(|(kind, _)| *kind == ScreenNotice::Address));
        assert!(
            fixture.model.screen_address_error().is_some(),
            "the error stays"
        );
    }

    #[test]
    fn a_failed_test_report_clears_on_a_later_success_or_a_host_change() {
        let mut fixture = Fixture::new(None, false);
        let detail = SshFailureClass::AuthFailed.detail().to_owned();
        fixture.model.screen_test_host(fixture.id);
        finish_test(&mut fixture, Err(failure(SshFailureClass::AuthFailed)));
        assert_eq!(
            fixture.model.screen_test_report(),
            Some((fixture.id, detail.as_str()))
        );
        assert_ne!(fixture.connection(), HostConnection::Connected);
        // A later successful test clears it.
        fixture.model.screen_test_host(fixture.id);
        finish_test(&mut fixture, Ok(opencode_report()));
        assert_eq!(fixture.model.screen_test_report(), None);
        assert_eq!(fixture.connection(), HostConnection::Connected);
        // A failure again, then a host change clears it.
        fixture.model.screen_test_host(fixture.id);
        finish_test(&mut fixture, Err(failure(SshFailureClass::AuthFailed)));
        assert!(fixture.model.screen_test_report().is_some());
        assert_ne!(fixture.connection(), HostConnection::Connected);
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        fixture.model.screen_select_host(HostId::Local);
        assert_eq!(fixture.model.ssh_notice(), None);
        assert_eq!(fixture.model.screen_test_report(), None);
    }

    #[test]
    fn a_failed_test_report_is_dismissed_by_pointer_and_keyboard_without_closing() {
        let dismiss = crate::ui::picker_control_id("dismiss-report");
        let fail = |fixture: &mut Fixture| {
            fixture.model.screen_test_host(fixture.id);
            finish_test(fixture, Err(failure(SshFailureClass::AuthFailed)));
            for _ in 0..2 {
                frame(fixture, Vec::new());
            }
        };
        let mut fixture = Fixture::new(None, false);
        fail(&mut fixture);
        let detail = SshFailureClass::AuthFailed.detail();
        // On the notice line, in ERROR with the error glyph.
        let (texts, _) = frame_paint(&mut fixture, Vec::new());
        let line = texts
            .iter()
            .find(|(text, ..)| text.contains(detail))
            .unwrap_or_else(|| panic!("the failure is shown: {texts:?}"));
        assert_eq!(line.1, crate::theme::ERROR);
        assert!(texts.iter().any(|(text, color, rect)| text == "!"
            && *color == crate::theme::ERROR
            && rect.right() <= line.2.left()));
        let rect = fixture.ctx.read_response(dismiss).expect("dismiss").rect;
        assert!(rect.width() >= 24.0 && rect.height() >= 24.0, "{rect:?}");
        // Pointer.
        let at = rect.center();
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut fixture, vec![egui::Event::PointerMoved(at)]);
        frame(&mut fixture, vec![button(true)]);
        frame(&mut fixture, vec![button(false)]);
        assert_eq!(fixture.model.screen_test_report(), None);
        assert!(fixture.model.is_picker(), "the screen stays open");
        frame(
            &mut fixture,
            vec![egui::Event::PointerMoved(egui::pos2(2.0, 2.0))],
        );
        // Keyboard: Escape while the dismiss control is focused.
        fail(&mut fixture);
        fixture
            .ctx
            .memory_mut(|memory| memory.request_focus(dismiss));
        frame(&mut fixture, Vec::new());
        frame(&mut fixture, Vec::new());
        let key = |pressed| egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut fixture, vec![key(true)]);
        frame(&mut fixture, vec![key(false)]);
        assert_eq!(fixture.model.screen_test_report(), None);
        assert!(fixture.model.is_picker(), "Escape closes only the report");
        // Keyboard: Enter on the focused dismiss control.
        fail(&mut fixture);
        fixture
            .ctx
            .memory_mut(|memory| memory.request_focus(dismiss));
        frame(&mut fixture, Vec::new());
        frame(&mut fixture, Vec::new());
        let enter = |pressed| egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut fixture, vec![enter(true)]);
        frame(&mut fixture, vec![enter(false)]);
        assert_eq!(fixture.model.screen_test_report(), None);
        assert!(fixture.model.is_picker());
    }

    #[test]
    fn a_repainted_prompt_keeps_another_live_field_focused() {
        let mut fixture = Fixture::staging(None);
        fixture.stage().prompts.send(prompt(1, PASSWORD)).unwrap();
        for _ in 0..2 {
            frame(&mut fixture, Vec::new());
        }
        let prompt_field = crate::ui::ssh_field_id("screen-prompt");
        let focused = |fixture: &Fixture| fixture.ctx.memory(|memory| memory.focused());
        fixture
            .ctx
            .memory_mut(|memory| memory.request_focus(prompt_field));
        frame(&mut fixture, Vec::new());
        assert_eq!(focused(&fixture), Some(prompt_field));
        fixture.model.screen_add_host();
        frame(&mut fixture, Vec::new());
        for field in [
            crate::ui::host_form_field_id(crate::ssh_servers::HostField::Name),
            crate::ui::picker_path_id(),
        ] {
            fixture.ctx.memory_mut(|memory| memory.request_focus(field));
            for _ in 0..3 {
                frame(&mut fixture, Vec::new());
                assert_eq!(
                    focused(&fixture),
                    Some(field),
                    "a repainted prompt must not steal focus from another live field"
                );
            }
            assert!(
                fixture.ctx.read_response(prompt_field).is_some(),
                "the live prompt keeps repainting"
            );
        }
    }

    #[test]
    fn remote_git_is_read_on_first_connect_with_an_initial_directory() {
        let mut fixture = Fixture::opened(Some("/srv/app"), Some("/home/me"));
        assert_eq!(fixture.root(), Some(PathBuf::from("/srv/app")));
        assert!(
            fixture
                .request(|operation| matches!(
                    operation,
                    BrowseOperation::BrowseGitRead { path: at, .. } if *at == path("/srv/app")
                ))
                .is_some(),
            "the git read dropped before the grant opened is sent once it is open"
        );
    }

    #[test]
    fn tree_expands_collapses_and_moves_with_the_keyboard() {
        let mut fixture = Fixture::opened(Some("/srv"), None);
        fixture.list("/srv", &["/srv/a", "/srv/b"]);
        assert_eq!(
            fixture.rows(),
            vec![("a".into(), 0, false), ("b".into(), 0, false)]
        );
        fixture.model.screen_move(1);
        assert_eq!(fixture.model.screen_selection(), Some(0));
        // Right asks for the children; the listing expands the row.
        fixture.model.screen_expand();
        fixture.list("/srv/a", &["/srv/a/x"]);
        assert_eq!(
            fixture.rows(),
            vec![
                ("a".into(), 0, true),
                ("x".into(), 1, false),
                ("b".into(), 0, false)
            ]
        );
        // Right on an expanded row selects its first child; Left goes back up.
        fixture.model.screen_expand();
        assert_eq!(fixture.model.screen_selection(), Some(1));
        assert_eq!(
            fixture.model.open_workspace().unwrap().launch_target(),
            Some(PathBuf::from("/srv/a/x"))
        );
        fixture.model.screen_collapse();
        assert_eq!(fixture.model.screen_selection(), Some(0));
        fixture.model.screen_collapse();
        assert_eq!(
            fixture.rows(),
            vec![("a".into(), 0, false), ("b".into(), 0, false)]
        );
        // Up re-roots at the parent and lists it.
        fixture.model.screen_up();
        assert_eq!(fixture.root(), Some(PathBuf::from("/")));
        fixture.list("/", &["/srv"]);
        assert_eq!(fixture.rows(), vec![("srv".into(), 0, false)]);
    }

    #[test]
    fn refresh_relists_the_root_and_every_expanded_folder() {
        let mut fixture = Fixture::opened(Some("/srv"), None);
        fixture.list("/srv", &["/srv/a", "/srv/b"]);
        fixture.model.screen_toggle(0);
        fixture.list("/srv/a", &["/srv/a/x"]);
        fixture.model.screen_refresh();
        fixture.list("/srv", &["/srv/a", "/srv/b", "/srv/c"]);
        fixture.list("/srv/a", &["/srv/a/x", "/srv/a/y"]);
        assert_eq!(
            fixture.rows(),
            vec![
                ("a".into(), 0, true),
                ("x".into(), 1, false),
                ("y".into(), 1, false),
                ("b".into(), 0, false),
                ("c".into(), 0, false)
            ]
        );
    }

    #[test]
    fn refresh_keeps_nested_expansion_and_a_deep_selection() {
        let mut fixture = Fixture::opened(Some("/srv"), None);
        fixture.list("/srv", &["/srv/a", "/srv/z"]);
        fixture.model.screen_toggle(0);
        fixture.list("/srv/a", &["/srv/a/b", "/srv/a/gone"]);
        fixture.model.screen_toggle(1);
        fixture.list("/srv/a/b", &["/srv/a/b/c"]);
        fixture.model.screen_toggle(2);
        fixture.list("/srv/a/b/c", &["/srv/a/b/c/d"]);
        // Select the great-grandchild d.
        fixture.model.screen_move(1);
        assert_eq!(fixture.model.screen_selection(), Some(3));
        fixture.model.screen_refresh();
        fixture.list("/srv", &["/srv/a", "/srv/z"]);
        fixture.list("/srv/a", &["/srv/a/b", "/srv/a/new"]);
        fixture.list("/srv/a/b", &["/srv/a/b/c"]);
        fixture.list("/srv/a/b/c", &["/srv/a/b/c/d"]);
        assert_eq!(
            fixture.rows(),
            vec![
                ("a".into(), 0, true),
                ("b".into(), 1, true),
                ("c".into(), 2, true),
                ("d".into(), 3, false),
                ("new".into(), 1, false),
                ("z".into(), 0, false)
            ]
        );
        assert_eq!(fixture.model.screen_selection(), Some(3));
        assert_eq!(
            fixture.model.open_workspace().unwrap().launch_target(),
            Some(PathBuf::from("/srv/a/b/c/d"))
        );
    }

    #[test]
    fn a_root_listing_prunes_the_subtree_of_a_removed_folder() {
        let mut fixture = Fixture::opened(Some("/srv"), None);
        fixture.list("/srv", &["/srv/a", "/srv/b"]);
        fixture.model.screen_toggle(0);
        fixture.list("/srv/a", &["/srv/a/x"]);
        fixture.model.screen_move(1);
        assert_eq!(fixture.model.screen_selection(), Some(1));
        fixture.model.screen_refresh();
        fixture.list("/srv", &["/srv/b"]);
        assert_eq!(fixture.rows(), vec![("b".into(), 0, false)]);
        assert_eq!(
            fixture.model.screen_selection(),
            None,
            "the selection on a pruned row is dropped"
        );
        assert_eq!(
            fixture.model.open_workspace().unwrap().launch_target(),
            Some(PathBuf::from("/srv"))
        );
    }

    #[test]
    fn a_late_listing_never_reexpands_a_collapsed_folder() {
        let mut fixture = Fixture::opened(Some("/srv"), None);
        fixture.list("/srv", &["/srv/a", "/srv/b"]);
        // Expand, then collapse again before the children arrive.
        fixture.model.screen_toggle(0);
        fixture.model.screen_toggle(0);
        fixture.list("/srv/a", &["/srv/a/x"]);
        assert_eq!(
            fixture.rows(),
            vec![("a".into(), 0, false), ("b".into(), 0, false)]
        );
        // The same from the keyboard: Right twice still expands once, Left
        // cancels.
        fixture.model.screen_expand();
        fixture.model.screen_expand();
        fixture.model.screen_collapse();
        assert_eq!(fixture.model.screen_selection(), Some(0));
        fixture.list("/srv/a", &["/srv/a/x"]);
        assert_eq!(fixture.rows()[0], ("a".into(), 0, false));
        assert!(
            fixture
                .request(|operation| matches!(operation, BrowseOperation::BrowseList { .. }))
                .is_none(),
            "Right asked for the children once"
        );
        // An expanded folder collapsed while its refresh listing is in flight.
        while fixture
            .request(|operation| matches!(operation, BrowseOperation::BrowseList { .. }))
            .is_some()
        {}
        fixture.model.screen_toggle(0);
        fixture.list("/srv/a", &["/srv/a/x"]);
        assert_eq!(fixture.rows()[0], ("a".into(), 0, true));
        fixture.model.screen_refresh();
        fixture.model.screen_toggle(0);
        fixture.list("/srv/a", &["/srv/a/x", "/srv/a/y"]);
        assert_eq!(
            fixture.rows(),
            vec![("a".into(), 0, false), ("b".into(), 0, false)]
        );
    }

    #[test]
    fn a_rejected_stage_password_is_never_cached_or_resubmitted() {
        let mut fixture = Fixture::staging(None);
        fixture.stage().prompts.send(prompt(1, PASSWORD)).unwrap();
        fixture.sync();
        assert!(fixture.model.screen_prompt().is_some());
        fixture.answer("wrong", true);
        assert_eq!(fixture.stage().answer(), Some((1, true)));
        assert!(
            !crate::credential_cache::cached(fixture.id),
            "a password joins the cache only after authentication"
        );
        // OpenSSH asks again: the rejected value is not sent again.
        fixture.stage().prompts.send(prompt(2, PASSWORD)).unwrap();
        fixture.sync();
        assert!(fixture.model.screen_prompt().is_some());
        assert_eq!(fixture.stage().answer(), None);
        fixture.answer("still wrong", true);
        fixture
            .stage()
            .done
            .send(Err(failure(SshFailureClass::AuthFailed)))
            .unwrap();
        fixture.sync();
        assert!(!crate::credential_cache::cached(fixture.id));
        assert!(matches!(fixture.connection(), HostConnection::Failed(_)));
    }

    #[test]
    fn a_cached_password_the_stage_rejects_is_forgotten_and_not_resent() {
        let mut fixture = Fixture::staging(None);
        crate::credential_cache::remember(&fixture.key(), "stale");
        fixture.stage().prompts.send(prompt(1, PASSWORD)).unwrap();
        fixture.sync();
        assert!(fixture.model.screen_prompt().is_none());
        assert_eq!(
            fixture.stage().answer(),
            Some((1, true)),
            "cache answers once"
        );
        fixture.stage().prompts.send(prompt(2, PASSWORD)).unwrap();
        fixture.sync();
        assert_eq!(fixture.stage().answer(), None, "never twice in one attempt");
        assert!(fixture.model.screen_prompt().is_some());
        assert!(!crate::credential_cache::cached(fixture.id));
    }

    #[test]
    fn a_stage_password_joins_the_cache_after_staging_succeeds() {
        let mut fixture = Fixture::staging(None);
        fixture.stage().prompts.send(prompt(1, PASSWORD)).unwrap();
        fixture.sync();
        fixture.answer("right", true);
        assert!(!crate::credential_cache::cached(fixture.id));
        fixture.stage().done.send(Ok(())).unwrap();
        fixture.sync();
        assert!(crate::credential_cache::cached(fixture.id));
    }

    #[test]
    fn a_cancelled_stage_forgets_the_cached_password_it_sent() {
        let mut fixture = Fixture::staging(None);
        crate::credential_cache::remember(&fixture.key(), "stale");
        fixture.stage().prompts.send(prompt(1, PASSWORD)).unwrap();
        fixture.sync();
        assert_eq!(fixture.stage().answer(), Some((1, true)));
        fixture
            .stage()
            .done
            .send(Err(failure(SshFailureClass::Cancelled)))
            .unwrap();
        fixture.sync();
        assert!(!crate::credential_cache::cached(fixture.id));
        assert_eq!(
            fixture.connection(),
            HostConnection::AuthenticationCancelled
        );
    }

    #[test]
    fn a_cached_password_the_browse_rejects_is_forgotten_and_not_resent() {
        let mut fixture = Fixture::browsing(None);
        crate::credential_cache::remember(&fixture.key(), "stale");
        fixture.server().prompt(1, PASSWORD);
        fixture.sync();
        assert_eq!(fixture.server().answers(), vec![(1, true)]);
        assert!(fixture.model.screen_prompt().is_none());
        fixture.server().prompt(2, PASSWORD);
        fixture.sync();
        assert!(fixture.server().answers().is_empty());
        assert!(fixture.model.screen_prompt().is_some());
        assert!(!crate::credential_cache::cached(fixture.id));
    }

    #[test]
    fn a_browse_password_joins_the_cache_once_the_grant_opens() {
        let mut fixture = Fixture::browsing(None);
        fixture.server().prompt(1, PASSWORD);
        fixture.sync();
        fixture.answer("right", true);
        assert_eq!(fixture.server().answers(), vec![(1, true)]);
        assert!(!crate::credential_cache::cached(fixture.id));
        let open = fixture
            .request(|operation| *operation == BrowseOperation::BrowseOpen)
            .unwrap();
        fixture.server().reply(
            &open,
            BrowseOperation::BrowseOpened {
                home: Some(path("/home/me")),
            },
        );
        fixture.sync();
        assert!(crate::credential_cache::cached(fixture.id));
    }

    #[test]
    fn a_browse_that_fails_authentication_forgets_the_cached_password_it_sent() {
        let mut fixture = Fixture::browsing(None);
        crate::credential_cache::remember(&fixture.key(), "stale");
        fixture.server().prompt(1, PASSWORD);
        fixture.sync();
        assert_eq!(fixture.server().answers(), vec![(1, true)]);
        fixture
            .server()
            .event(SshEvent::Failed(made_platform::ssh::classify_ssh_stderr(
                b"Permission denied (publickey,password).",
            )));
        fixture.sync();
        assert!(!crate::credential_cache::cached(fixture.id));
        assert!(fixture.link_is(|link| matches!(link, RemoteLink::Stopped(_))));
    }

    #[test]
    fn the_open_timeout_waits_while_a_browse_prompt_awaits_the_user() {
        let mut fixture = Fixture::browsing(None);
        fixture.server().prompt(1, PASSWORD);
        fixture.sync();
        assert!(fixture.model.screen_prompt().is_some());
        let session = fixture
            .model
            .screen
            .as_mut()
            .unwrap()
            .remote
            .as_mut()
            .unwrap();
        let RemoteLink::Browsing(connection) = &mut session.link else {
            panic!("browsing");
        };
        let later = Instant::now() + crate::remote_workspace::BROWSE_TIMEOUT * 3;
        assert!(connection.browse_mut().expire(later).is_empty());
        assert_eq!(
            connection.browse().phase(),
            crate::remote_workspace::BrowsePhase::Opening
        );
        fixture.answer("pw", true);
        assert_eq!(fixture.server().answers(), vec![(1, true)]);
    }

    #[test]
    fn a_changed_host_key_ending_the_browse_offers_to_forget_the_key() {
        let mut fixture = Fixture::browsing(None);
        fixture
            .server()
            .event(SshEvent::Failed(made_platform::ssh::classify_ssh_stderr(
                b"@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @",
            )));
        fixture.sync();
        let detail = SshFailureClass::ChangedHostKey.detail();
        assert_eq!(
            fixture.connection(),
            HostConnection::Failed(detail.to_owned())
        );
        assert!(fixture.model.ssh_host_key_changed());
        assert_eq!(fixture.model.ssh_selected(), Some(fixture.id));
        assert_eq!(fixture.model.ssh_notice(), Some(detail));
    }

    #[test]
    fn a_stale_address_check_never_reroots() {
        let mut fixture = Fixture::opened(Some("/srv"), None);
        fixture.model.screen_navigate("/opt").unwrap();
        fixture.model.screen_up();
        assert_eq!(fixture.root(), Some(PathBuf::from("/")));
        fixture.check("/opt", "/opt");
        assert_eq!(fixture.root(), Some(PathBuf::from("/")), "navigated away");
        // Only the latest address applies.
        fixture.model.screen_navigate("/a").unwrap();
        fixture.model.screen_navigate("/b").unwrap();
        fixture.check("/a", "/a");
        assert_eq!(fixture.root(), Some(PathBuf::from("/")));
        fixture.check("/b", "/b");
        assert_eq!(fixture.root(), Some(PathBuf::from("/b")));
    }

    #[test]
    fn an_address_typed_while_the_host_is_not_connected_says_so() {
        let mut fixture = Fixture::consent(None);
        assert!(fixture.model.screen_navigate("/srv").is_err());
        let message = fixture.model.screen_address_message().expect("a message");
        assert!(message.contains("not connected"), "{message}");
        // A connected host keeps the folder reasons.
        let mut fixture = Fixture::opened(Some("/srv"), None);
        assert_eq!(
            fixture.model.screen_navigate("relative"),
            Err(AddressError::NotAbsolute)
        );
        assert_eq!(
            fixture.model.screen_address_message(),
            Some(AddressError::NotAbsolute.message())
        );
    }

    #[test]
    fn a_launch_check_applies_only_to_the_current_launch_path() {
        let mut fixture = Fixture::opened(Some("/srv"), None);
        fixture.list("/srv", &["/srv/a"]);
        let open = fixture.model.open_workspace().unwrap();
        let terminal = LaunchKind::Terminal;
        // The server resolved a symlink: the chosen path still launches.
        assert_eq!(
            revalidated_launch(open, terminal, path("/srv"), Ok(path("/data/srv"))),
            LaunchCheck::Launch(path("/srv"))
        );
        assert_eq!(
            revalidated_launch(open, terminal, path("/srv/a"), Ok(path("/srv/a"))),
            LaunchCheck::Stale
        );
        assert!(matches!(
            revalidated_launch(open, terminal, path("/srv"), Err(AddressError::NotFound)),
            LaunchCheck::Failed(text) if text.contains("Folder not found") && text.contains("Nothing was started")
        ));
        // A kind that is not offered on this host is stale too.
        assert_eq!(
            revalidated_launch(
                open,
                LaunchKind::Tui(crate::integrations::OPENCODE),
                path("/srv"),
                Ok(path("/srv"))
            ),
            LaunchCheck::Stale
        );
    }

    #[test]
    fn a_launch_check_for_a_moved_target_starts_nothing() {
        let mut fixture = Fixture::opened(Some("/srv"), None);
        fixture.list("/srv", &["/srv/a"]);
        fixture.model.screen_launch(LaunchKind::Terminal);
        assert_eq!(fixture.model.screen_launch_error(), None);
        // The user moves on before the server answers.
        fixture.model.screen_toggle(0);
        // Any launch attempt would now fail visibly.
        fixture
            .model
            .ssh
            .as_mut()
            .unwrap()
            .remove(fixture.id)
            .unwrap();
        fixture.check("/srv", "/srv");
        assert_eq!(fixture.model.screen_launch_error(), None);
        assert_eq!(fixture.model.workspace_count(), 0);
        assert!(fixture.model.is_picker());
    }

    #[test]
    fn editing_the_selected_host_ends_the_grant_and_reconnects() {
        let mut fixture = Fixture::opened(Some("/srv"), None);
        assert!(fixture.model.screen_staged.contains(&fixture.id));
        let server = fixture.server();
        fixture.model.screen_edit_host(fixture.id);
        fixture.model.screen_form_mut().unwrap().host = "lab2.invalid".into();
        assert!(fixture.model.screen_save_form());
        assert!(
            server
                .requests()
                .iter()
                .any(|request| request.operation == BrowseOperation::BrowseClose),
            "the old grant ended"
        );
        assert!(!fixture.model.screen_staged.contains(&fixture.id));
        assert!(
            fixture.link_is(|link| matches!(link, RemoteLink::Probing)),
            "the edited record is detected again"
        );
        assert_eq!(fixture.model.seams.probes.len(), 1);
        assert_eq!(fixture.connection(), HostConnection::Connecting);
    }

    #[test]
    fn removing_the_selected_host_ends_the_grant_and_forgets_its_staging() {
        let mut fixture = Fixture::opened(Some("/srv"), None);
        let server = fixture.server();
        fixture.model.screen_request_remove(fixture.id);
        fixture.model.screen_answer_removal(true);
        assert!(server
            .requests()
            .iter()
            .any(|request| request.operation == BrowseOperation::BrowseClose));
        assert!(!fixture.model.screen_staged.contains(&fixture.id));
        assert!(fixture.screen().remote.is_none());
    }

    #[test]
    fn consent_refusal_writes_nothing_and_stops() {
        let mut fixture = Fixture::consent(None);
        fixture.model.screen_answer_consent(false);
        assert!(!fixture.model.screen_consent_pending());
        assert!(fixture.model.seams.stages.is_empty());
        assert!(!fixture
            .model
            .remote_provision_consent_granted(fixture.id)
            .unwrap());
        assert!(fixture.model.screen_notice().is_some());
        assert_eq!(fixture.connection(), HostConnection::NotConnected);
    }

    #[test]
    fn consent_approval_is_saved_and_stages() {
        let fixture = Fixture::staging(None);
        assert!(fixture
            .model
            .remote_provision_consent_granted(fixture.id)
            .unwrap());
        assert_eq!(fixture.model.seams.stages.len(), 1);
    }

    #[test]
    fn an_undetected_host_probes_then_asks_for_consent() {
        let mut fixture = Fixture::new(None, false);
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        assert!(fixture.link_is(|link| matches!(link, RemoteLink::Probing)));
        assert_eq!(fixture.connection(), HostConnection::Connecting);
        fixture.model.seams.probes[0]
            .done
            .send(Ok(ProbeReport {
                capabilities: capabilities(),
                claude: false,
            }))
            .unwrap();
        fixture.model.poll_ssh_probe();
        fixture.sync();
        assert!(fixture.model.screen_consent_pending());
    }

    #[test]
    fn an_unknown_host_key_needs_approval_and_cancel_stops_the_attempt() {
        let mut fixture = Fixture::new(None, false);
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        fixture.model.seams.probes[0]
            .prompts
            .send(prompt(
                1,
                "The authenticity of host 'lab.invalid' can't be established.\nED25519 key fingerprint is SHA256:abc.\nAre you sure you want to continue connecting (yes/no/[fingerprint])?",
            ))
            .unwrap();
        fixture.model.poll_ssh_probe();
        fixture.sync();
        assert_eq!(fixture.connection(), HostConnection::HostKeyApprovalNeeded);
        fixture.model.screen_answer_prompt(false);
        fixture.sync();
        assert_eq!(
            fixture.connection(),
            HostConnection::AuthenticationCancelled
        );
        assert!(fixture.model.seams.probes[0]
            .cancelled
            .load(Ordering::Acquire));
    }

    #[test]
    fn a_changed_host_key_prompt_is_refused_with_the_forget_key_feedback() {
        let mut fixture = Fixture::new(None, false);
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        fixture.model.seams.probes[0]
            .prompts
            .send(prompt(
                1,
                "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @",
            ))
            .unwrap();
        fixture.model.poll_ssh_probe();
        fixture.sync();
        fixture.model.screen_answer_prompt(true);
        fixture.sync();
        assert_eq!(
            fixture.connection(),
            HostConnection::Failed(SshFailureClass::ChangedHostKey.detail().to_owned())
        );
        assert!(fixture.model.ssh_host_key_changed());
    }

    #[test]
    fn an_install_prompt_is_answered_by_its_install_never_the_pending_probe() {
        let mut fixture = Fixture::new(None, false);
        // A connection test for X is pending.
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        assert!(fixture.model.probe.is_some());
        // An install on another server Y holds the shown password prompt.
        let other = AuthorityId([0x59; 16]);
        fixture.model.ssh_card = Some(PromptCard::Install(other, prompt(7, PASSWORD)));
        *fixture.model.prompt_draft() = "secret of y".to_owned();
        fixture.model.submit_ssh_prompt(true);
        assert_eq!(
            fixture.probe().answer(),
            None,
            "Y's password never reaches X's probe"
        );
        fixture
            .probe()
            .done
            .send(Ok(ProbeReport {
                capabilities: capabilities(),
                claude: false,
            }))
            .unwrap();
        fixture.model.poll_ssh_probe();
        assert!(
            !crate::credential_cache::cached(fixture.id),
            "Y's password is never cached under X"
        );
        // Host-key answers route by owner too: refusing Y's key leaves X's
        // probe alone.
        let mut fixture = Fixture::new(None, false);
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        fixture.model.ssh_card = Some(PromptCard::Install(
            other,
            prompt(
                8,
                "The authenticity of host 'y.invalid' can't be established.",
            ),
        ));
        fixture.model.submit_ssh_prompt(false);
        assert!(fixture.model.probe.is_some(), "X's probe keeps running");
        assert!(!fixture.probe().cancelled.load(Ordering::Acquire));
        assert_eq!(fixture.probe().answer(), None);
        fixture.model.ssh_card = Some(PromptCard::Install(
            other,
            prompt(
                9,
                "@@@@@@@@@@@@\n@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @",
            ),
        ));
        fixture.model.submit_ssh_prompt(false);
        assert!(fixture.model.probe.is_some(), "X's probe keeps running");
        assert!(!fixture.probe().cancelled.load(Ordering::Acquire));
    }

    #[test]
    fn a_stale_card_of_a_replaced_probe_is_never_answered_or_cached() {
        let mut fixture = Fixture::new(None, false);
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        fixture
            .probe()
            .prompts
            .send(prompt(1, "me@a.invalid's password: "))
            .unwrap();
        fixture.model.poll_ssh_probe();
        assert!(fixture.model.screen_prompt().is_some());
        *fixture.model.prompt_draft() = "half typed".to_owned();
        // A new connection test replaces the first one.
        fixture.model.test_selected_ssh_server().unwrap();
        assert_eq!(fixture.model.seams.probes.len(), 2);
        assert!(
            fixture.model.screen_prompt().is_none(),
            "the replaced attempt's card is gone"
        );
        assert!(fixture.model.prompt_draft().is_empty());
        // Even a late submit of that card reaches nothing and caches nothing.
        *fixture.model.prompt_draft() = "password of a".to_owned();
        fixture.model.submit_ssh_prompt(true);
        assert_eq!(fixture.probe().answer(), None);
        fixture
            .probe()
            .done
            .send(Ok(ProbeReport {
                capabilities: capabilities(),
                claude: false,
            }))
            .unwrap();
        fixture.model.poll_ssh_probe();
        assert!(!crate::credential_cache::cached(fixture.id));
        // Cancelling clears the card the same way.
        fixture.model.test_selected_ssh_server().unwrap();
        fixture.probe().prompts.send(prompt(2, PASSWORD)).unwrap();
        fixture.model.poll_ssh_probe();
        assert!(fixture.model.screen_prompt().is_some());
        fixture.model.cancel_ssh_probe();
        assert!(fixture.model.screen_prompt().is_none());
        assert!(fixture.model.ssh_card.is_none());
    }

    #[test]
    fn an_install_card_left_by_an_ended_install_never_reaches_the_probe() {
        let mut fixture = Fixture::new(None, false);
        // A connection test for X is pending.
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        let other = AuthorityId([0x59; 16]);
        // Y's install shows a card, then finishes or is re-approved: the
        // install path drops its card and draft in the same place.
        fixture.model.ssh_card = Some(PromptCard::Install(other, prompt(7, PASSWORD)));
        *fixture.model.prompt_draft() = "half of y".to_owned();
        fixture
            .model
            .drop_card(|card| card.install() == Some(other));
        assert!(fixture.model.screen_prompt().is_none(), "the card is gone");
        assert!(fixture.model.prompt_draft().is_empty());
        // Even had the card stayed, its answer has only its install to go to.
        fixture.model.ssh_card = Some(PromptCard::Install(other, prompt(8, PASSWORD)));
        *fixture.model.prompt_draft() = "secret of y".to_owned();
        fixture.model.screen_answer_prompt(true);
        assert_eq!(fixture.probe().answer(), None, "X's probe got nothing");
        fixture
            .probe()
            .done
            .send(Ok(ProbeReport {
                capabilities: capabilities(),
                claude: false,
            }))
            .unwrap();
        fixture.model.poll_ssh_probe();
        assert!(!crate::credential_cache::cached(fixture.id));
    }

    #[test]
    fn declining_an_install_card_leaves_the_connection_test_running() {
        let mut fixture = Fixture::new(None, false);
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        let other = AuthorityId([0x5a; 16]);
        fixture.model.ssh_card = Some(PromptCard::Install(other, prompt(3, PASSWORD)));
        fixture.model.screen_answer_prompt(false);
        assert!(fixture.model.probe.is_some(), "X's test keeps running");
        assert!(!fixture.probe().cancelled.load(Ordering::Acquire));
        fixture.sync();
        assert_eq!(fixture.connection(), HostConnection::Connecting);
        assert!(fixture.model.ssh_card.is_none());
    }

    #[test]
    fn a_probe_card_answers_only_the_test_that_emitted_it() {
        let mut fixture = Fixture::new(None, false);
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        let stale = crate::ssh_servers::ProbeTicket {
            authority: fixture.id,
            attempt: u64::MAX,
        };
        fixture.model.ssh_card = Some(PromptCard::Probe(stale, prompt(4, PASSWORD)));
        *fixture.model.prompt_draft() = "for an older test".to_owned();
        fixture.model.submit_ssh_prompt(true);
        assert_eq!(fixture.probe().answer(), None);
        fixture
            .probe()
            .done
            .send(Ok(ProbeReport {
                capabilities: capabilities(),
                claude: false,
            }))
            .unwrap();
        fixture.model.poll_ssh_probe();
        assert!(!crate::credential_cache::cached(fixture.id));
    }

    fn expired(id: u64) -> PromptRequest {
        PromptRequest {
            id,
            text: PASSWORD.to_owned(),
            expires_at: Instant::now() - Duration::from_secs(1),
        }
    }

    #[test]
    fn an_expired_card_is_neither_answered_nor_held() {
        // Connection test card.
        let mut fixture = Fixture::new(None, false);
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        let ticket = fixture.probe_ticket();
        fixture.model.ssh_card = Some(PromptCard::Probe(ticket, expired(1)));
        *fixture.model.prompt_draft() = "too late".to_owned();
        fixture.model.screen_answer_prompt(true);
        assert_eq!(fixture.probe().answer(), None);
        fixture
            .probe()
            .done
            .send(Ok(ProbeReport {
                capabilities: capabilities(),
                claude: false,
            }))
            .unwrap();
        fixture.model.poll_ssh_probe();
        assert!(!crate::credential_cache::cached(fixture.id));
        assert!(fixture.model.prompt_draft().is_empty());
        // Screen staging slot.
        let mut fixture = Fixture::staging(None);
        fixture.model.screen.as_mut().unwrap().prompt = Some((expired(2), PromptOwner::Stage));
        *fixture.model.prompt_draft() = "too late".to_owned();
        fixture.model.screen_answer_prompt(true);
        assert_eq!(fixture.stage().answer(), None);
        fixture.stage().done.send(Ok(())).unwrap();
        fixture.sync();
        assert!(!crate::credential_cache::cached(fixture.id));
        // A cached password is never spent on an expired prompt.
        let mut fixture = Fixture::staging(None);
        crate::credential_cache::remember(&fixture.key(), "cached");
        fixture.stage().prompts.send(expired(3)).unwrap();
        fixture.sync();
        assert_eq!(fixture.stage().answer(), None);
    }

    #[test]
    fn a_draft_never_moves_to_another_prompt() {
        let mut fixture = Fixture::staging(None);
        fixture.stage().prompts.send(prompt(1, PASSWORD)).unwrap();
        fixture.sync();
        *fixture.model.prompt_draft() = "for one".to_owned();
        fixture.sync();
        assert_eq!(
            fixture.model.prompt_draft(),
            "for one",
            "same prompt keeps it"
        );
        // Another prompt takes the slot.
        fixture.model.screen.as_mut().unwrap().prompt =
            Some((prompt(2, PASSWORD), PromptOwner::Stage));
        fixture.sync();
        assert!(fixture.model.prompt_draft().is_empty());
        // A card of another owner replaces it.
        *fixture.model.prompt_draft() = "for two".to_owned();
        fixture.sync();
        fixture.model.screen.as_mut().unwrap().prompt = None;
        fixture.model.ssh_card = Some(PromptCard::Install(
            AuthorityId([0x61; 16]),
            prompt(2, PASSWORD),
        ));
        fixture.sync();
        assert!(fixture.model.prompt_draft().is_empty());
    }

    #[test]
    fn an_install_prompt_password_is_not_cached_before_authentication() {
        let mut fixture = Fixture::new(None, true);
        fixture.model.ssh_selected = Some(fixture.id);
        fixture.model.ssh_card = Some(PromptCard::Install(fixture.id, prompt(1, PASSWORD)));
        *fixture.model.prompt_draft() = "typed".to_owned();
        fixture.model.submit_ssh_prompt(true);
        assert!(
            !crate::credential_cache::cached(fixture.id),
            "the install attempt has not authenticated"
        );
    }

    #[test]
    fn a_rejected_probe_password_is_never_resubmitted() {
        let mut fixture = Fixture::new(None, false);
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        fixture.probe().prompts.send(prompt(1, PASSWORD)).unwrap();
        fixture.model.poll_ssh_probe();
        fixture.answer("wrong", true);
        assert_eq!(fixture.probe().answer(), Some((1, true)));
        assert!(!crate::credential_cache::cached(fixture.id));
        fixture.probe().prompts.send(prompt(2, PASSWORD)).unwrap();
        fixture.model.poll_ssh_probe();
        assert!(fixture.model.screen_prompt().is_some(), "asked again");
        assert_eq!(fixture.probe().answer(), None);
        fixture.answer("right", true);
        fixture
            .probe()
            .done
            .send(Ok(ProbeReport {
                capabilities: capabilities(),
                claude: false,
            }))
            .unwrap();
        fixture.model.poll_ssh_probe();
        assert!(
            crate::credential_cache::cached(fixture.id),
            "the accepted password is cached"
        );
    }

    #[test]
    fn a_cached_password_the_probe_rejects_is_forgotten_and_not_resent() {
        let mut fixture = Fixture::new(None, false);
        crate::credential_cache::remember(&fixture.key(), "stale");
        fixture.model.screen_select_host(HostId::Ssh(fixture.id));
        fixture.probe().prompts.send(prompt(1, PASSWORD)).unwrap();
        fixture.model.poll_ssh_probe();
        assert_eq!(fixture.probe().answer(), Some((1, true)));
        assert!(fixture.model.screen_prompt().is_none());
        fixture.probe().prompts.send(prompt(2, PASSWORD)).unwrap();
        fixture.model.poll_ssh_probe();
        assert_eq!(fixture.probe().answer(), None);
        assert!(fixture.model.screen_prompt().is_some());
        assert!(!crate::credential_cache::cached(fixture.id));
        fixture
            .probe()
            .done
            .send(Err(failure(SshFailureClass::AuthFailed)))
            .unwrap();
        fixture.model.poll_ssh_probe();
        assert!(!crate::credential_cache::cached(fixture.id));
    }
}
