//! Restart sequence, smoke markers and the command-line update entry
//! (add-release-auto-update task 5.7). The `Restart now` / `Not now` UI
//! sequence with live workspaces is exercised in `ui_model.rs`, which owns the
//! isolated headless harness; the install lock, running-instance refusal and
//! GUI startup refusal are exercised against the real platform locks in
//! `made-platform/tests/update.rs`.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use made_app::app::{AppModel, AppOptions};
use made_app::cli::{parse_args, CliAction};
use made_app::preferences::PreferenceStore;
use made_app::updates::{
    command_line_update, Restarter, UpdateCapability, UpdateFailure, UpdateScheduler,
};
use made_app::workspace::WorkspaceLaunch;

#[derive(Clone, Default)]
struct Fake {
    offer: Option<String>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl UpdateCapability for Fake {
    fn kept(&mut self) -> Option<String> {
        self.calls.lock().unwrap().push("kept".into());
        None
    }
    fn check(&mut self) -> Result<Option<String>, UpdateFailure> {
        self.calls.lock().unwrap().push("check".into());
        Ok(self.offer.clone())
    }
    fn download(&mut self, version: &str) -> Result<String, UpdateFailure> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("download {version}"));
        Ok(version.to_owned())
    }
    fn restarter(&self) -> Restarter {
        let calls = self.calls.clone();
        Box::new(move |version: &str| {
            calls.lock().unwrap().push(format!("restart {version}"));
            Ok(())
        })
    }
}

fn run_entry(fake: &Fake, apply: Option<made_app::updates::CliApply<'_>>) -> (i32, String) {
    let mut scheduler = UpdateScheduler::start(Box::new(fake.clone()), false, Instant::now());
    let mut out = Vec::new();
    let code = command_line_update(&mut scheduler, apply, &mut out);
    (code, String::from_utf8(out).unwrap())
}

#[test]
fn the_update_flags_parse_and_apply_requires_update() {
    let parse = |values: &[&str]| parse_args(values.iter().map(std::ffi::OsString::from));
    assert_eq!(
        parse(&["--update"]).unwrap(),
        CliAction::Update { apply: false }
    );
    assert_eq!(
        parse(&["--update", "--apply"]).unwrap(),
        CliAction::Update { apply: true }
    );
    assert!(parse(&["--apply"]).is_err());
    assert!(parse(&["--update", "/tmp"]).is_err());
    assert!(parse(&["--update", "--smoke-test"]).is_err());
}

#[test]
fn by_default_the_entry_only_checks_and_downloads() {
    let fake = Fake {
        offer: Some("0.3.0".into()),
        ..Fake::default()
    };
    let (code, out) = run_entry(&fake, None);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Made 0.3.0 is ready to install."), "{out}");
    let calls = fake.calls.lock().unwrap().clone();
    assert_eq!(calls, vec!["kept", "check", "download 0.3.0"]);
}

#[test]
fn the_apply_flag_applies_through_the_same_capability_and_does_not_restart() {
    let fake = Fake {
        offer: Some("0.3.0".into()),
        ..Fake::default()
    };
    let applied = Arc::new(Mutex::new(Vec::<String>::new()));
    let record = applied.clone();
    let mut apply = move |version: &str| {
        record.lock().unwrap().push(version.to_owned());
        Ok(format!("Made {version} is installed."))
    };
    let (code, out) = run_entry(&fake, Some(&mut apply));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Made 0.3.0 is installed."), "{out}");
    assert_eq!(*applied.lock().unwrap(), vec!["0.3.0".to_owned()]);
    let calls = fake.calls.lock().unwrap().clone();
    assert!(
        !calls.iter().any(|call| call.starts_with("restart")),
        "the CLI apply never relaunches Made: {calls:?}"
    );
}

#[test]
fn apply_refused_while_made_runs_exits_non_zero_with_the_message() {
    let fake = Fake {
        offer: Some("0.3.0".into()),
        ..Fake::default()
    };
    let mut apply = |_: &str| {
        Err(
            "Made is running from this install (process 4242); quit it before applying the update"
                .to_owned(),
        )
    };
    let (code, out) = run_entry(&fake, Some(&mut apply));
    assert_ne!(code, 0);
    assert!(out.contains("process 4242"), "{out}");
}

#[test]
fn a_non_installed_build_reports_not_applicable_without_any_request() {
    let mut scheduler = UpdateScheduler::unavailable(false);
    let mut out = Vec::new();
    let called = Arc::new(Mutex::new(false));
    let flag = called.clone();
    let mut apply = move |_: &str| {
        *flag.lock().unwrap() = true;
        Ok(String::new())
    };
    let code = command_line_update(&mut scheduler, Some(&mut apply), &mut out);
    assert_eq!(code, 0);
    assert_eq!(
        String::from_utf8(out).unwrap().trim(),
        "Updates are not available for this build."
    );
    assert!(!*called.lock().unwrap());
}

#[test]
fn the_entry_works_with_automatic_checks_off_and_leaves_the_choice_off() {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("data"));
    store.set_automatic_checks(false).unwrap();
    let fake = Fake {
        offer: Some("0.3.0".into()),
        ..Fake::default()
    };
    let (code, _) = run_entry(&fake, None);
    assert_eq!(code, 0);
    assert!(fake.calls.lock().unwrap().contains(&"check".to_owned()));
    assert!(
        !store.load().automatic_checks,
        "the stored choice stays off"
    );
}

#[test]
fn the_smoke_line_reports_layout_and_disabled_updates_for_the_production_repository() {
    let model = AppModel::from_launch(WorkspaceLaunch::waiting_for_picker(), AppOptions::new());
    let markers = model.smoke_layout_markers();
    for expected in [
        "opencode_plugin=",
        "grok_status=",
        "companions=",
        "askpass=",
        "updates=disabled",
        "update_repo=wcgomes/made",
        &format!("version={}", made_app::APP_VERSION),
    ] {
        assert!(
            markers.contains(expected),
            "missing {expected:?} in {markers}"
        );
    }
    assert!(markers.contains(&format!(
        "update_seam={}",
        u8::from(made_platform::update::TEST_SEAM_ENABLED)
    )));
}
