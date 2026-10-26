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
use crate::theme;
use crate::ui;
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
}

impl AppOptions {
    /// Default options with no smoke run.
    pub fn new() -> Self {
        Self {
            smoke_test: None,
            started_at: Instant::now(),
        }
    }

    /// Enable the smoke run.
    pub fn with_smoke(mut self, duration: Duration) -> Self {
        self.smoke_test = Some(duration);
        self
    }
}

impl Default for AppOptions {
    fn default() -> Self {
        Self::new()
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
    show_diagnostics: bool,
    shutdown_done: bool,
    frame_count: u64,
    ui_frames: u64,
    theme_ready: bool,
    native_backdrop: NativeBackdropStatus,
    smoke: Option<SmokeState>,
}

impl std::fmt::Debug for AppModel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppModel")
            .field("launch", &self.launch)
            .field("start_pending", &self.start_pending)
            .field("sessions_open", &self.show_sessions)
            .field("diagnostics_open", &self.show_diagnostics)
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
            show_diagnostics: false,
            shutdown_done: false,
            frame_count: 0,
            ui_frames: 0,
            theme_ready: false,
            native_backdrop: NativeBackdropStatus::default(),
            smoke,
        }
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
        self.launch = WorkspaceLaunch::waiting_for_picker();
        self.start_pending = false;
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
        let id = WorkspaceId(self.next_workspace);
        self.next_workspace = self
            .next_workspace
            .checked_add(1)
            .expect("workspace identity exhausted");
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
        self.start_pending = true;
        self.show_sessions = false;
        if let Some(workbench) = self.workbench_mut() {
            workbench.set_visible(true);
        }
    }

    /// Navigate the picker to an entry.
    pub fn picker_enter(
        &mut self,
        index: usize,
    ) -> Result<(), crate::workspace_picker::PickerError> {
        self.launch.picker_enter(index)
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
                let workbench = Workbench::start(Some(plan.clone()), None, cwd, prepared_at);
                self.insert_workbench(workbench, source);
            }
            WorkspaceLaunch::TerminalFallback { error, cwd, .. } => {
                let workbench =
                    Workbench::start(None, Some(error.clone()), cwd.clone(), prepared_at);
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
        let prepared_at = Instant::now();
        let (sender, receiver) = mpsc::sync_channel(1);
        let spawn = thread::Builder::new()
            .name("made-workspace-prepare".to_owned())
            .spawn(move || {
                if worker_cancel.load(Ordering::Acquire) {
                    return;
                }
                let mut workbench = Workbench::start(plan, launch_error, cwd, prepared_at);
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

    /// Poll runtime work and drive the smoke run.
    pub fn logic(&mut self, ctx: &egui::Context) {
        self.apply_focus_reset(ctx);
        self.poll_pending_start();
        self.begin_pending_start();
        for (_, workbench) in &mut self.workspaces {
            workbench.tick(ctx);
        }
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
            "made smoke: window_ready=1 ui_frames={} frames={frames} elapsed_ms={} platform={} {status}",
            self.ui_frames,
            elapsed.as_millis(),
            platform_family_label()
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
            self.show_diagnostics = !self.show_diagnostics;
        }
        if self.is_picker() {
            return;
        }
        let active = self.active;
        if let Some((id, workbench)) = self
            .workspaces
            .iter_mut()
            .find(|(id, _)| Some(*id) == active)
        {
            ui.push_id(*id, |ui| {
                workbench.ui(ui, &mut self.show_sessions, &mut self.show_diagnostics)
            });
        } else {
            ui::starting_ui(self, ui);
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
            workbench.shutdown();
        }
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
            kind: NativeBackdropKind::WindowsMica,
            applied: true,
            reason: "test status".to_owned(),
        });
        assert_eq!(
            model.native_backdrop().kind(),
            NativeBackdropKind::WindowsMica
        );
        assert!(model.native_backdrop().is_applied());
        assert_eq!(model.native_backdrop().reason(), "test status");

        model.shutdown();
    }
}
