//! Made desktop application entry point.
//!
//! `made [workspace]` opens one native window with a real PTY terminal on the
//! left and the read-only explorer/preview on the right. No workspace argument
//! opens the in-app folder picker. `--smoke-test` creates the real window,
//! pumps frames, prints a smoke marker, and exits. `--update [--apply]` is the
//! non-interactive update entry of an installed build.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use eframe::egui;

use made_app::app::{AppModel, AppOptions};
use made_app::cli::{self, CliAction};
use made_app::preferences::PreferenceStore;
use made_app::updates::{command_line_update, PlatformCapability, UpdateScheduler};
use made_platform::update::{self as platform_update, CliApplyOutcome, Updater};

fn main() -> ExitCode {
    // Velopack's install/update hooks first. Auto-apply on startup is off:
    // a downloaded update is applied only after confirmation (REL-07).
    platform_update::run_startup_hook();
    let action = match cli::parse_process_args() {
        Ok(action) => action,
        Err(error) => {
            eprintln!("made: {error}");
            eprintln!("{}", cli::usage());
            return ExitCode::from(2);
        }
    };

    match action {
        CliAction::Help => {
            println!("{}", cli::usage());
            ExitCode::SUCCESS
        }
        CliAction::Version => {
            println!("{}", AppModel::version_banner());
            ExitCode::SUCCESS
        }
        CliAction::Update { apply } => update_entry(apply),
        CliAction::ApplyHelper { wait_pid, root } => {
            match platform_update::run_windows_helper(wait_pid, &root) {
                Ok(version) => {
                    println!("Made {version} is installed.");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("made: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        CliAction::Run(args) => run(args),
    }
}

/// REL-12 command-line entry: the same capability, allowlist and
/// verification as About. Never reads or changes the stored automatic-check
/// choice and never starts the GUI.
fn update_entry(apply: bool) -> ExitCode {
    let updater = Updater::detect().map(Arc::new);
    let mut scheduler = match &updater {
        Some(updater) => UpdateScheduler::start(
            Box::new(PlatformCapability::new(updater.clone())),
            false,
            Instant::now(),
        ),
        None => UpdateScheduler::unavailable(false),
    };
    let mut apply_ready = |version: &str| -> Result<String, String> {
        let updater = updater
            .as_ref()
            .ok_or("updates are not available for this build")?;
        let ready = updater
            .kept()
            .filter(|ready| ready.version() == version)
            .ok_or("the verified update is no longer available")?;
        match updater.apply_from_cli(&ready).map_err(|error| error.to_string())? {
            CliApplyOutcome::Applied(version) => Ok(format!("Made {version} is installed.")),
            CliApplyOutcome::HandedOff { pid, version } => Ok(format!(
                "Installing Made {version} in helper process {pid}; Made will not start until it finishes."
            )),
        }
    };
    let apply_hook: Option<made_app::updates::CliApply<'_>> =
        if apply { Some(&mut apply_ready) } else { None };
    let code = command_line_update(&mut scheduler, apply_hook, &mut std::io::stdout());
    ExitCode::from(code as u8)
}

fn run(args: cli::LaunchArgs) -> ExitCode {
    let smoke = args.smoke_test;
    let started_at = Instant::now();
    let updater = Updater::detect().map(Arc::new);
    // Announce this GUI instance; refuse while a command-line apply holds the
    // install lock (REL-12).
    let _instance = match &updater {
        Some(updater) => match platform_update::register_instance(updater.installation()) {
            Ok(guard) => Some(guard),
            Err(refused) => {
                eprintln!("made: {refused}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };
    let options = AppOptions {
        smoke_test: smoke,
        started_at,
        ..AppOptions::new().with_compatible_tui_detection()
    };
    let mut model = AppModel::new(args, options);
    // Smoke runs and non-installed builds make no update request (REL-11).
    match (&updater, smoke) {
        (Some(updater), None) => {
            let store = PreferenceStore::from_app_paths();
            let preference = store.as_ref().map(PreferenceStore::load).unwrap_or(
                made_app::preferences::UpdatePreference {
                    automatic_checks: true,
                    diagnostic: None,
                },
            );
            if let Some(diagnostic) = preference.diagnostic.clone() {
                model.record_update_note(made_app::diagnostics::LogLevel::Warn, diagnostic);
            }
            model.set_updates(
                UpdateScheduler::start(
                    Box::new(PlatformCapability::new(updater.clone())),
                    preference.automatic_checks,
                    Instant::now(),
                ),
                store,
            );
        }
        (Some(_), Some(_)) => model.set_updates(UpdateScheduler::unavailable(true), None),
        (None, _) => {}
    }

    let result = eframe::run_native(
        "Made",
        made_app::app::native_options(),
        Box::new(move |_cc| {
            Ok(Box::new(NativeApp {
                model,
                native_backdrop_checked: false,
            }))
        }),
    );

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if smoke.is_some() {
                eprintln!("made smoke: FAILED to create or run the native window: {error}");
            } else {
                eprintln!("made: failed to create or run the native window: {error}");
            }
            ExitCode::FAILURE
        }
    }
}

struct NativeApp {
    model: AppModel,
    native_backdrop_checked: bool,
}

impl eframe::App for NativeApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if !self.native_backdrop_checked {
            self.native_backdrop_checked = true;
            let status = made_platform::native_backdrop::apply(&*frame);
            eprintln!(
                "made: native backdrop: {} applied={} reason={}",
                status.label(),
                status.is_applied(),
                status.reason()
            );
            self.model.set_native_backdrop(status);
        }
        self.model.logic(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.model.ui(ui);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        made_app::theme::clear_color_with_native(self.model.native_backdrop().is_applied())
    }

    fn on_exit(&mut self) {
        self.model.shutdown();
    }
}
