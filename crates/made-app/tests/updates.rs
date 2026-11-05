//! Update scheduler (add-release-auto-update task 5.3).

use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use made_app::updates::{
    Restarter, UpdateCapability, UpdateEvent, UpdateFailure, UpdateScheduler, UpdateStatus,
    CHECK_INTERVAL,
};

#[derive(Default)]
struct Script {
    kept: Option<String>,
    checks: Vec<Result<Option<String>, UpdateFailure>>,
    downloads: Vec<Result<String, UpdateFailure>>,
}

#[derive(Default)]
struct Calls {
    kept: usize,
    checks: usize,
    downloads: Vec<String>,
    restarts: Vec<String>,
    threads: Vec<ThreadId>,
}

#[derive(Clone, Default)]
struct Fake {
    script: Arc<Mutex<Script>>,
    calls: Arc<Mutex<Calls>>,
}

impl Fake {
    fn with(script: Script) -> Self {
        Self {
            script: Arc::new(Mutex::new(script)),
            calls: Arc::default(),
        }
    }
    fn calls(&self) -> std::sync::MutexGuard<'_, Calls> {
        self.calls.lock().unwrap()
    }
    fn push_check(&self, result: Result<Option<String>, UpdateFailure>) {
        self.script.lock().unwrap().checks.push(result);
    }
    fn push_download(&self, result: Result<String, UpdateFailure>) {
        self.script.lock().unwrap().downloads.push(result);
    }
}

impl UpdateCapability for Fake {
    fn kept(&mut self) -> Option<String> {
        let mut calls = self.calls();
        calls.kept += 1;
        calls.threads.push(std::thread::current().id());
        self.script.lock().unwrap().kept.clone()
    }
    fn check(&mut self) -> Result<Option<String>, UpdateFailure> {
        let mut calls = self.calls();
        calls.checks += 1;
        calls.threads.push(std::thread::current().id());
        let mut script = self.script.lock().unwrap();
        if script.checks.is_empty() {
            Ok(None)
        } else {
            script.checks.remove(0)
        }
    }
    fn download(&mut self, version: &str) -> Result<String, UpdateFailure> {
        let mut calls = self.calls();
        calls.downloads.push(version.to_owned());
        calls.threads.push(std::thread::current().id());
        let mut script = self.script.lock().unwrap();
        if script.downloads.is_empty() {
            Ok(version.to_owned())
        } else {
            script.downloads.remove(0)
        }
    }
    fn restarter(&self) -> Restarter {
        let calls = self.calls.clone();
        Box::new(move |version: &str| {
            calls.lock().unwrap().restarts.push(version.to_owned());
            Ok(())
        })
    }
}

/// Tick until the worker is idle.
fn settle(scheduler: &mut UpdateScheduler, now: Instant) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        scheduler.tick(now);
        if !scheduler.busy() {
            return;
        }
        assert!(Instant::now() < deadline, "scheduler never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn checks_at_launch_then_every_24_hours_off_the_ui_thread() {
    let fake = Fake::default();
    let start = Instant::now();
    let mut scheduler = UpdateScheduler::start(Box::new(fake.clone()), true, start);
    settle(&mut scheduler, start);
    assert_eq!(fake.calls().checks, 1, "one launch check");
    assert!(matches!(scheduler.status(), UpdateStatus::UpToDate { .. }));
    settle(&mut scheduler, start + Duration::from_secs(60 * 60));
    assert_eq!(fake.calls().checks, 1, "no check before 24 hours");
    settle(
        &mut scheduler,
        start + CHECK_INTERVAL + Duration::from_secs(1),
    );
    assert_eq!(fake.calls().checks, 2, "one periodic check after 24 hours");
    settle(
        &mut scheduler,
        start + CHECK_INTERVAL + Duration::from_secs(2),
    );
    assert_eq!(fake.calls().checks, 2, "only one check per interval");
    let ui = std::thread::current().id();
    assert!(
        fake.calls().threads.iter().all(|id| *id != ui),
        "capability calls run off the UI thread"
    );
}

#[test]
fn a_manual_check_runs_while_automatic_checks_are_off_without_changing_the_choice() {
    let fake = Fake::default();
    fake.push_check(Ok(Some("0.3.0".into())));
    let start = Instant::now();
    let mut scheduler = UpdateScheduler::start(Box::new(fake.clone()), false, start);
    settle(&mut scheduler, start + CHECK_INTERVAL * 3);
    assert_eq!(fake.calls().checks, 0, "no automatic check while off");
    assert_eq!(scheduler.status(), &UpdateStatus::NotChecked);

    scheduler.check_now();
    assert!(
        scheduler.busy(),
        "Check for updates is unavailable while busy"
    );
    settle(&mut scheduler, start);
    assert_eq!(fake.calls().checks, 1);
    assert_eq!(
        fake.calls().downloads,
        vec!["0.3.0".to_owned()],
        "the download it leads to runs"
    );
    assert_eq!(scheduler.ready_version(), Some("0.3.0"));
    assert!(!scheduler.automatic(), "the stored choice stays off");
    settle(&mut scheduler, start + CHECK_INTERVAL * 4);
    assert_eq!(fake.calls().checks, 1);
}

#[test]
fn turning_automatic_checks_on_checks_and_off_stops_them() {
    let fake = Fake::default();
    let start = Instant::now();
    let mut scheduler = UpdateScheduler::start(Box::new(fake.clone()), true, start);
    settle(&mut scheduler, start);
    scheduler.set_automatic(false, start);
    settle(&mut scheduler, start + CHECK_INTERVAL * 2);
    assert_eq!(fake.calls().checks, 1);
    scheduler.set_automatic(true, start + CHECK_INTERVAL * 2);
    settle(&mut scheduler, start + CHECK_INTERVAL * 2);
    assert_eq!(fake.calls().checks, 2);
}

#[test]
fn unavailable_builds_and_smoke_runs_make_no_request() {
    for installed in [false, true] {
        let mut scheduler = UpdateScheduler::unavailable(installed);
        let start = Instant::now();
        scheduler.check_now();
        scheduler.set_automatic(true, start);
        scheduler.tick(start + CHECK_INTERVAL * 2);
        assert!(!scheduler.busy());
        assert!(!scheduler.available());
        assert_eq!(scheduler.ready_version(), None);
        let expected = if installed {
            UpdateStatus::Disabled
        } else {
            UpdateStatus::NotAvailable
        };
        assert_eq!(scheduler.status(), &expected);
        assert!(scheduler.take_events().is_empty());
        assert!(scheduler.restart().is_err());
    }
    assert_eq!(
        UpdateStatus::NotAvailable.line(|_| String::new()),
        "Updates are not available for this build."
    );
}

#[test]
fn ready_only_after_verification_and_failures_show_only_in_status() {
    let fake = Fake::default();
    fake.push_check(Ok(Some("0.3.0".into())));
    fake.push_download(Err(UpdateFailure::Verification));
    let start = Instant::now();
    let mut scheduler = UpdateScheduler::start(Box::new(fake.clone()), true, start);
    scheduler.tick(start);
    assert_eq!(scheduler.ready_version(), None);
    settle(&mut scheduler, start);
    assert_eq!(
        scheduler.ready_version(),
        None,
        "an unverified package is never ready"
    );
    assert_eq!(
        scheduler.status(),
        &UpdateStatus::VerificationFailed("0.3.0".into())
    );
    assert!(scheduler
        .take_events()
        .contains(&UpdateEvent::VerificationFailed("0.3.0".into())));

    fake.push_check(Ok(Some("0.3.0".into())));
    fake.push_download(Err(UpdateFailure::Download));
    scheduler.check_now();
    settle(&mut scheduler, start);
    assert_eq!(
        scheduler.status(),
        &UpdateStatus::DownloadFailed("0.3.0".into())
    );
    assert_eq!(
        scheduler.ready_version(),
        None,
        "no indicator after a failed download"
    );

    fake.push_check(Err(UpdateFailure::Unreachable));
    scheduler.check_now();
    settle(&mut scheduler, start);
    assert_eq!(scheduler.status(), &UpdateStatus::CheckFailed);
    assert_eq!(
        scheduler.ready_version(),
        None,
        "no indicator after a failed check"
    );
    assert!(scheduler.take_events().contains(&UpdateEvent::CheckFailed));

    fake.push_check(Ok(Some("0.3.0".into())));
    scheduler.check_now();
    assert!(matches!(scheduler.status(), UpdateStatus::Checking));
    settle(&mut scheduler, start);
    assert_eq!(scheduler.ready_version(), Some("0.3.0"));
    assert_eq!(scheduler.status(), &UpdateStatus::Ready("0.3.0".into()));
}

#[test]
fn every_status_line_is_distinct_and_names_the_version() {
    let lines: Vec<String> = [
        UpdateStatus::NotAvailable,
        UpdateStatus::Disabled,
        UpdateStatus::NotChecked,
        UpdateStatus::UpToDate {
            checked_at: std::time::SystemTime::now(),
        },
        UpdateStatus::Checking,
        UpdateStatus::Downloading("0.3.0".into()),
        UpdateStatus::Ready("0.3.0".into()),
        UpdateStatus::CheckFailed,
        UpdateStatus::DownloadFailed("0.3.0".into()),
        UpdateStatus::VerificationFailed("0.3.0".into()),
    ]
    .iter()
    .map(|status| status.line(|_| "just now".to_owned()))
    .collect();
    let unique: std::collections::BTreeSet<_> = lines.iter().collect();
    assert_eq!(unique.len(), lines.len(), "{lines:?}");
    assert!(lines[3].contains("Up to date") && lines[3].contains("just now"));
    assert!(lines[5].contains("Downloading Made 0.3.0"));
    assert!(lines[6].contains("0.3.0 is ready to install"));
    assert!(lines[7].contains("could not reach the update service"));
    assert!(lines[8].contains("download of Made 0.3.0 failed"));
    assert!(lines[9].contains("0.3.0 failed verification and was discarded"));
}

#[test]
fn declining_keeps_the_update_and_a_relaunch_reoffers_it_without_download() {
    let fake = Fake::with(Script {
        kept: Some("0.3.0".into()),
        ..Script::default()
    });
    let start = Instant::now();
    // Relaunch with no network and automatic checks off.
    let mut scheduler = UpdateScheduler::start(Box::new(fake.clone()), false, start);
    settle(&mut scheduler, start);
    assert_eq!(scheduler.ready_version(), Some("0.3.0"));
    assert!(fake.calls().downloads.is_empty(), "no new download");
    assert_eq!(fake.calls().checks, 0, "no request");
    assert!(fake.calls().restarts.is_empty(), "never applied at launch");
    // Declining is simply not confirming; dropping the scheduler is a quit.
    drop(scheduler);
    assert!(fake.calls().restarts.is_empty(), "never applied at quit");

    // A check that finds the same kept version does not download again.
    let fake = Fake::with(Script {
        kept: Some("0.3.0".into()),
        checks: vec![Ok(Some("0.3.0".into()))],
        ..Script::default()
    });
    let mut scheduler = UpdateScheduler::start(Box::new(fake.clone()), true, start);
    settle(&mut scheduler, start);
    assert_eq!(scheduler.ready_version(), Some("0.3.0"));
    assert!(fake.calls().downloads.is_empty());
}

#[test]
fn a_newer_version_supersedes_a_ready_one_only_once_verified() {
    let fake = Fake::with(Script {
        kept: Some("0.3.0".into()),
        ..Script::default()
    });
    fake.push_check(Ok(Some("0.4.0".into())));
    fake.push_download(Err(UpdateFailure::Verification));
    let start = Instant::now();
    let mut scheduler = UpdateScheduler::start(Box::new(fake.clone()), true, start);
    settle(&mut scheduler, start);
    assert_eq!(
        scheduler.ready_version(),
        Some("0.3.0"),
        "the older ready version stays offered"
    );
    assert_eq!(scheduler.status(), &UpdateStatus::Ready("0.3.0".into()));

    fake.push_check(Ok(Some("0.4.0".into())));
    scheduler.check_now();
    settle(&mut scheduler, start);
    assert_eq!(
        scheduler.ready_version(),
        Some("0.4.0"),
        "only the newest verified version is offered"
    );
}

#[test]
fn restart_applies_only_the_ready_version_on_confirmation() {
    let fake = Fake::with(Script {
        kept: Some("0.3.0".into()),
        ..Script::default()
    });
    let start = Instant::now();
    let mut scheduler = UpdateScheduler::start(Box::new(fake.clone()), false, start);
    settle(&mut scheduler, start);
    assert!(fake.calls().restarts.is_empty());
    assert_eq!(scheduler.restart().unwrap(), "0.3.0");
    assert_eq!(fake.calls().restarts, vec!["0.3.0".to_owned()]);
    assert!(scheduler
        .take_events()
        .contains(&UpdateEvent::RestartRequested("0.3.0".into())));
}
