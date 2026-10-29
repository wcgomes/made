//! Made desktop application entry point.
//!
//! `made [workspace]` opens one native window with a real PTY terminal on the
//! left and the read-only explorer/preview on the right. No workspace argument
//! opens the in-app folder picker. `--smoke-test` creates the real window,
//! pumps frames, prints a smoke marker, and exits.

use std::process::ExitCode;
use std::time::Instant;

use eframe::egui;

use made_app::app::{AppModel, AppOptions};
use made_app::cli::{self, CliAction};

fn main() -> ExitCode {
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
        CliAction::Run(args) => run(args),
    }
}

fn run(args: cli::LaunchArgs) -> ExitCode {
    let smoke = args.smoke_test;
    let started_at = Instant::now();
    let options = AppOptions {
        smoke_test: smoke,
        started_at,
        ..AppOptions::new().with_compatible_tui_detection()
    };
    let model = AppModel::new(args, options);

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
