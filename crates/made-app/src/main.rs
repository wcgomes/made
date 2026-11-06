//! Made desktop application entry point.
//!
//! `made [workspace]` opens one native window with a real PTY terminal on the
//! left and the read-only explorer/preview on the right. No workspace argument
//! opens the in-app folder picker. `--smoke-test` creates the real window,
//! pumps frames, prints a smoke marker, and exits. `--update [--apply]` is the
//! non-interactive update entry of an installed build.

// GUI subsystem on Windows: launching Made must not open a console window.
// `windows::attach_parent_console` restores CLI output when started from a
// terminal.
#![cfg_attr(windows, windows_subsystem = "windows")]

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
    made_platform::attach_parent_console();
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
        CliAction::ApplyHelper {
            wait_pid,
            root,
            lock_handle,
        } => match platform_update::run_windows_helper(wait_pid, &root, lock_handle) {
            Ok(version) => {
                println!("Made {version} is installed.");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("made: {error}");
                ExitCode::FAILURE
            }
        },
        CliAction::Run(args) => run(args),
    }
}

/// REL-12 command-line entry: the same capability, allowlist and
/// verification as About. Never reads or changes the stored automatic-check
/// choice and never starts the GUI.
fn update_entry(apply: bool) -> ExitCode {
    /// Exit status while another process applies an update (EX_TEMPFAIL).
    const APPLY_IN_PROGRESS: u8 = 75;
    let updater = Updater::detect().map(Arc::new);
    if let Some(updater) = &updater {
        if updater.apply_in_progress() {
            println!("An update is being applied to this install (apply in progress); run `made --update` again when it finishes.");
            return ExitCode::from(APPLY_IN_PROGRESS);
        }
        if let Some(outcome) = updater.last_cli_apply() {
            println!("Last command-line apply: {}.", outcome.describe());
        }
    }
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
                "Handed off to helper process {pid}: Made {version} is being installed and the apply is not complete yet. Made will not start until it finishes; run `made --update` to see the result."
            )),
        }
    };
    let apply_hook: Option<made_app::updates::CliApply<'_>> =
        if apply { Some(&mut apply_ready) } else { None };
    let code = command_line_update(&mut scheduler, apply_hook, &mut std::io::stdout());
    ExitCode::from(code as u8)
}

/// A small native window that says why Made did not start, for launches with
/// no terminal attached. Closes on its button, Enter or Escape.
fn show_startup_refusal(message: String) {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Made")
            .with_inner_size([460.0, 150.0])
            .with_resizable(false),
        ..Default::default()
    };
    let _ = eframe::run_ui_native("Made", options, move |ui, _frame| {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Made cannot start right now").strong());
            ui.add_space(6.0);
            ui.label(message.as_str());
            ui.add_space(14.0);
            let close = ui.button("Close").clicked()
                || ui.input(|input| {
                    input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Escape)
                });
            if close {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    });
}

fn run(args: cli::LaunchArgs) -> ExitCode {
    let smoke = args.smoke_test;
    let started_at = Instant::now();
    let updater = Updater::detect().map(Arc::new);
    // Announce this GUI instance; refuse only while a command-line apply holds
    // the install lock (REL-12). A window that cannot register still starts,
    // with updates off for this run.
    let (_instance, unregistered) = match &updater {
        Some(updater) => match platform_update::register_instance(updater.installation()) {
            Ok(platform_update::Registration::Registered(guard)) => (Some(guard), None),
            Ok(platform_update::Registration::UpdatesUnavailable(detail)) => (None, Some(detail)),
            Err(refused) => {
                eprintln!("made: {refused}");
                if smoke.is_none() {
                    show_startup_refusal(refused.to_string());
                }
                return ExitCode::FAILURE;
            }
        },
        None => (None, None),
    };
    let options = AppOptions {
        smoke_test: smoke,
        started_at,
        ..AppOptions::new().with_compatible_tui_detection()
    };
    let mut model = AppModel::new(args, options);
    if let Some(updater) = &updater {
        model.set_update_apply_outcome(updater.last_cli_apply().map(|outcome| outcome.describe()));
    }
    if let Some(detail) = &unregistered {
        let note = format!("Updates are off for this run: {detail}.");
        eprintln!("made: {note}");
        model.record_update_note(made_app::diagnostics::LogLevel::Warn, note);
    }
    // Smoke runs and non-installed builds make no update request (REL-11).
    match (&updater, smoke) {
        (Some(_), None) if unregistered.is_some() => {
            model.set_updates(UpdateScheduler::unavailable(true), None);
        }
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
