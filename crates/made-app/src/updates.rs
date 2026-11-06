//! Update scheduling and state (REL-06, REL-07, REL-09 to REL-11).
//!
//! The scheduler owns *when* Made checks and downloads; the capability in
//! `made-platform` owns *how*. Every capability call runs on one worker
//! thread, never on the UI thread. Automatic checks run at launch and every
//! 24 hours while the stored choice is on; a manual `Check for updates` (or the
//! command-line entry) performs one check and its download even while the
//! choice is off, without changing it. A downloaded update becomes ready only
//! after the capability verified it, is kept when the user declines, and is
//! re-offered at the next launch without a new download. Nothing here applies
//! an update: only an explicit confirmation does, through [`UpdateScheduler::restart`].
//! Failures are logged and shown only as About status lines.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

/// Interval between automatic checks while Made runs.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Why an update operation failed, as the scheduler classifies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateFailure {
    /// Offline, unreachable, rate-limited, refused location or malformed data.
    Unreachable,
    /// The download stopped or could not be stored.
    Download,
    /// The package failed size, digest or embedded-version verification.
    Verification,
    /// Another update operation is running for this install.
    Busy,
}

/// The update operations the scheduler drives. Implemented over
/// `made_platform::update::Updater` for installed builds and by fakes in tests.
pub trait UpdateCapability: Send + 'static {
    /// A verified package kept from an earlier download, if still newer.
    fn kept(&mut self) -> Option<String>;
    /// Check once; `Ok(Some(version))` when a newer stable version exists.
    fn check(&mut self) -> Result<Option<String>, UpdateFailure>;
    /// Download and verify `version`; returns the ready version.
    fn download(&mut self, version: &str) -> Result<String, UpdateFailure>;
    /// A handle that hands a ready version to the updater for apply and
    /// relaunch. It runs on the UI thread when the user confirms, so a
    /// background download never delays the restart.
    fn restarter(&self) -> Restarter;

    /// A check run before `Restart now` closes anything: `Err` names why the
    /// restart must not start (for example another Made window of the same
    /// install is open). The default never blocks.
    fn restart_guard(&self) -> RestartGuard {
        Box::new(|| Ok(()))
    }
}

/// See [`UpdateCapability::restart_guard`].
pub type RestartGuard = Box<dyn Fn() -> Result<(), String> + Send>;

/// Hands a ready version to the updater; see [`UpdateCapability::restarter`].
pub type Restarter = Box<dyn FnMut(&str) -> Result<(), String> + Send>;

/// The one-line About Updates status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// Non-installed build: updates are not available.
    NotAvailable,
    /// Installed, but updates are disabled for this run (smoke test).
    Disabled,
    /// No check has completed in this run.
    NotChecked,
    /// The last check found no newer version.
    UpToDate {
        /// When that check succeeded.
        checked_at: SystemTime,
    },
    /// A check is running.
    Checking,
    /// The named version is downloading.
    Downloading(String),
    /// The named version is verified and ready to install.
    Ready(String),
    /// The last check could not reach the update service.
    CheckFailed,
    /// The download of the named version failed.
    DownloadFailed(String),
    /// The named version failed verification and was discarded.
    VerificationFailed(String),
}

impl UpdateStatus {
    /// The status line text. `time` formats the last successful check.
    pub fn line(&self, time: impl Fn(SystemTime) -> String) -> String {
        match self {
            Self::NotAvailable => "Updates are not available for this build.".to_owned(),
            Self::Disabled => "Updates are disabled for this run.".to_owned(),
            Self::NotChecked => "Not checked for updates yet.".to_owned(),
            Self::UpToDate { checked_at } => {
                format!("Up to date. Last checked {}.", time(*checked_at))
            }
            Self::Checking => "Checking for updates…".to_owned(),
            Self::Downloading(version) => format!("Downloading Made {version}…"),
            Self::Ready(version) => format!("Made {version} is ready to install."),
            Self::CheckFailed => "The last check could not reach the update service.".to_owned(),
            Self::DownloadFailed(version) => format!("The download of Made {version} failed."),
            Self::VerificationFailed(version) => {
                format!("Made {version} failed verification and was discarded.")
            }
        }
    }
}

/// One outcome for the bounded diagnostics log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateEvent {
    /// A kept, verified update was found at launch.
    KeptFound(String),
    /// A check started (`manual` when the user or the command line asked).
    CheckStarted {
        /// Whether the check was explicitly requested.
        manual: bool,
    },
    /// The check found no newer version.
    UpToDate,
    /// The check found a newer version.
    Available(String),
    /// The check failed.
    CheckFailed,
    /// The newer version is downloaded and verified.
    Ready(String),
    /// The download failed.
    DownloadFailed(String),
    /// The downloaded package failed verification and was discarded.
    VerificationFailed(String),
    /// Another operation held the install; nothing changed.
    Busy,
    /// The user confirmed the restart into a version.
    RestartRequested(String),
    /// The restart could not start the updater.
    RestartFailed(String),
}

enum Job {
    Kept,
    Check,
    Download(String),
}

enum Command {
    Run(Job),
}

enum Outcome {
    Kept(Option<String>),
    Checked(Result<Option<String>, UpdateFailure>),
    Downloaded(String, Result<String, UpdateFailure>),
}

/// Drives the capability on a worker thread and holds the update state.
pub struct UpdateScheduler {
    worker: Option<(Sender<Command>, Receiver<Outcome>)>,
    restarter: Option<Restarter>,
    restart_guard: Option<RestartGuard>,
    status: UpdateStatus,
    ready: Option<String>,
    automatic: bool,
    next_check: Option<Instant>,
    busy: bool,
    events: Vec<UpdateEvent>,
    disabled_reason: Option<UpdateStatus>,
}

impl UpdateScheduler {
    /// A scheduler for a build without updates (non-installed or smoke test).
    /// It never makes a request.
    pub fn unavailable(installed: bool) -> Self {
        let status = if installed {
            UpdateStatus::Disabled
        } else {
            UpdateStatus::NotAvailable
        };
        Self {
            worker: None,
            restarter: None,
            restart_guard: None,
            status: status.clone(),
            ready: None,
            automatic: false,
            next_check: None,
            busy: false,
            events: Vec::new(),
            disabled_reason: Some(status),
        }
    }

    /// A scheduler for an installed build. It first looks for a kept update
    /// (no network), then checks at `now` when automatic checks are on.
    pub fn start(capability: Box<dyn UpdateCapability>, automatic: bool, now: Instant) -> Self {
        let restarter = capability.restarter();
        let restart_guard = capability.restart_guard();
        let (commands, inbox) = mpsc::channel::<Command>();
        let (outbox, outcomes) = mpsc::channel::<Outcome>();
        let spawned = thread::Builder::new()
            .name("made-updates".into())
            .spawn(move || worker(capability, inbox, outbox));
        let worker = spawned.ok().map(|_| (commands, outcomes));
        let mut scheduler = Self {
            worker,
            restarter: Some(restarter),
            restart_guard: Some(restart_guard),
            status: UpdateStatus::NotChecked,
            ready: None,
            automatic,
            next_check: automatic.then_some(now),
            busy: false,
            events: Vec::new(),
            disabled_reason: None,
        };
        scheduler.send(Job::Kept);
        scheduler
    }

    fn send(&mut self, job: Job) {
        if let Some((commands, _)) = &self.worker {
            if commands.send(Command::Run(job)).is_ok() {
                self.busy = true;
            }
        }
    }

    /// Whether this build can update at all.
    pub fn available(&self) -> bool {
        self.disabled_reason.is_none()
    }

    /// Current About status.
    pub fn status(&self) -> &UpdateStatus {
        &self.status
    }

    /// The verified version ready to install, if any.
    pub fn ready_version(&self) -> Option<&str> {
        self.ready.as_deref()
    }

    /// Whether a check or download is in progress (`Check for updates` is
    /// then unavailable).
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// The stored automatic-check choice as the scheduler applies it.
    pub fn automatic(&self) -> bool {
        self.automatic
    }

    /// Change the automatic-check choice. Turning it on schedules a check now.
    pub fn set_automatic(&mut self, enabled: bool, now: Instant) {
        if !self.available() {
            return;
        }
        self.automatic = enabled;
        self.next_check = enabled.then_some(now);
    }

    /// Start one explicitly requested check and the download it leads to.
    /// Ignored while busy; never changes the stored choice.
    pub fn check_now(&mut self) {
        if !self.available() || self.busy {
            return;
        }
        self.start_check(true);
    }

    fn start_check(&mut self, manual: bool) {
        self.events.push(UpdateEvent::CheckStarted { manual });
        if self.ready.is_none() {
            self.status = UpdateStatus::Checking;
        }
        self.send(Job::Check);
    }

    /// Drain finished work and start a scheduled automatic check when due.
    pub fn tick(&mut self, now: Instant) {
        while let Some((_, outcomes)) = &self.worker {
            let outcome = match outcomes.try_recv() {
                Ok(outcome) => outcome,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.busy = false;
                    break;
                }
            };
            self.busy = false;
            self.apply(outcome, now);
        }
        if self.busy || !self.automatic || !self.available() {
            return;
        }
        if self.next_check.is_some_and(|due| now >= due) {
            self.next_check = Some(now + CHECK_INTERVAL);
            self.start_check(false);
        }
    }

    fn apply(&mut self, outcome: Outcome, now: Instant) {
        match outcome {
            Outcome::Kept(Some(version)) => {
                self.events.push(UpdateEvent::KeptFound(version.clone()));
                self.ready = Some(version.clone());
                self.status = UpdateStatus::Ready(version);
            }
            Outcome::Kept(None) => {}
            Outcome::Checked(result) => {
                if self.automatic {
                    self.next_check = Some(now + CHECK_INTERVAL);
                }
                match result {
                    Ok(Some(version)) if self.ready.as_deref() == Some(version.as_str()) => {
                        self.status = UpdateStatus::Ready(version);
                    }
                    Ok(Some(version)) => {
                        self.events.push(UpdateEvent::Available(version.clone()));
                        if self.ready.is_none() {
                            self.status = UpdateStatus::Downloading(version.clone());
                        }
                        self.send(Job::Download(version));
                    }
                    Ok(None) => {
                        self.events.push(UpdateEvent::UpToDate);
                        self.status = match &self.ready {
                            Some(version) => UpdateStatus::Ready(version.clone()),
                            None => UpdateStatus::UpToDate {
                                checked_at: SystemTime::now(),
                            },
                        };
                    }
                    Err(UpdateFailure::Busy) => {
                        self.events.push(UpdateEvent::Busy);
                        self.restore_ready_or(UpdateStatus::NotChecked);
                    }
                    Err(_) => {
                        self.events.push(UpdateEvent::CheckFailed);
                        self.restore_ready_or(UpdateStatus::CheckFailed);
                    }
                }
            }
            Outcome::Downloaded(version, result) => match result {
                Ok(ready) => {
                    self.events.push(UpdateEvent::Ready(ready.clone()));
                    self.ready = Some(ready.clone());
                    self.status = UpdateStatus::Ready(ready);
                }
                Err(UpdateFailure::Verification) => {
                    self.events
                        .push(UpdateEvent::VerificationFailed(version.clone()));
                    self.restore_ready_or(UpdateStatus::VerificationFailed(version));
                }
                Err(UpdateFailure::Busy) => {
                    self.events.push(UpdateEvent::Busy);
                    self.restore_ready_or(UpdateStatus::NotChecked);
                }
                Err(_) => {
                    self.events
                        .push(UpdateEvent::DownloadFailed(version.clone()));
                    self.restore_ready_or(UpdateStatus::DownloadFailed(version));
                }
            },
        }
    }

    /// A failure never hides an older verified update: it stays offered.
    fn restore_ready_or(&mut self, status: UpdateStatus) {
        self.status = match &self.ready {
            Some(version) => UpdateStatus::Ready(version.clone()),
            None => status,
        };
    }

    /// Drain outcomes for the diagnostics log.
    pub fn take_events(&mut self) -> Vec<UpdateEvent> {
        std::mem::take(&mut self.events)
    }

    /// Wait until the worker is idle, for the command-line entry. Returns the
    /// final status.
    pub fn run_until_idle(&mut self, now: impl Fn() -> Instant) -> &UpdateStatus {
        loop {
            self.tick(now());
            if !self.busy {
                return &self.status;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Whether `Restart now` may start: `Err` says why not, before anything
    /// is closed.
    pub fn restart_check(&self) -> Result<(), String> {
        match &self.restart_guard {
            Some(guard) => guard(),
            None => Err("updates are unavailable".to_owned()),
        }
    }

    /// The confirmed restart: hand the ready version to the updater. The
    /// caller has already closed every workspace and exits on `Ok`.
    pub fn restart(&mut self) -> Result<String, String> {
        let version = self
            .ready
            .clone()
            .ok_or_else(|| "no update is ready".to_owned())?;
        let restarter = self
            .restarter
            .as_mut()
            .ok_or_else(|| "updates are unavailable".to_owned())?;
        self.events
            .push(UpdateEvent::RestartRequested(version.clone()));
        match restarter(&version) {
            Ok(()) => Ok(version),
            Err(error) => {
                self.events.push(UpdateEvent::RestartFailed(version));
                Err(error)
            }
        }
    }
}

fn worker(
    mut capability: Box<dyn UpdateCapability>,
    inbox: Receiver<Command>,
    outbox: Sender<Outcome>,
) {
    for command in inbox {
        let outcome = match command {
            Command::Run(Job::Kept) => Outcome::Kept(capability.kept()),
            Command::Run(Job::Check) => Outcome::Checked(capability.check()),
            Command::Run(Job::Download(version)) => {
                let result = capability.download(&version);
                Outcome::Downloaded(version, result)
            }
        };
        if outbox.send(outcome).is_err() {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// About panel model
// ---------------------------------------------------------------------------

/// What the About Updates section shows, rebuilt every frame from the
/// scheduler, plus the actions the user took in that frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdatePanel {
    /// Installed build with updates enabled for this run: controls shown.
    pub controls: bool,
    /// The one-line status.
    pub status_line: String,
    /// A check or download is running.
    pub busy: bool,
    /// The REL-10 choice.
    pub automatic: bool,
    /// Verified version ready to install.
    pub ready: Option<String>,
    /// The inline restart confirmation is shown.
    pub confirming: bool,
    /// Why the last `Restart now` did not start, shown instead of the status.
    pub notice: Option<String>,
    /// Actions taken this frame, handled by the app after drawing.
    pub actions: Vec<UpdateAction>,
}

/// One About Updates action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    /// `Check for updates`.
    CheckNow,
    /// `Check for updates automatically` changed to this value.
    SetAutomatic(bool),
    /// `Restart to update`: show the inline confirmation.
    RequestRestart,
    /// `Not now` or closing About: dismiss the confirmation.
    Dismiss,
    /// `Restart now`.
    RestartNow,
}

/// How long ago the last successful check ran, for the up-to-date line.
pub fn format_check_time(time: SystemTime) -> String {
    let minutes = SystemTime::now()
        .duration_since(time)
        .map(|age| age.as_secs() / 60)
        .unwrap_or(0);
    match minutes {
        0 => "just now".to_owned(),
        1 => "1 minute ago".to_owned(),
        2..=59 => format!("{minutes} minutes ago"),
        60..=119 => "1 hour ago".to_owned(),
        _ => format!("{} hours ago", minutes / 60),
    }
}

// ---------------------------------------------------------------------------
// Installed-build capability
// ---------------------------------------------------------------------------

/// [`UpdateCapability`] over the platform updater of an installed build.
pub struct PlatformCapability {
    updater: std::sync::Arc<made_platform::update::Updater>,
    candidate: Option<made_platform::update::Candidate>,
    ready: std::sync::Arc<std::sync::Mutex<Option<made_platform::update::Ready>>>,
}

impl PlatformCapability {
    /// Wrap the installed build's updater.
    pub fn new(updater: std::sync::Arc<made_platform::update::Updater>) -> Self {
        Self {
            updater,
            candidate: None,
            ready: std::sync::Arc::default(),
        }
    }
}

fn classify(error: &made_platform::update::UpdateError) -> UpdateFailure {
    use made_platform::update::UpdateError;
    match error {
        UpdateError::Unreachable(_) | UpdateError::Refused(_) | UpdateError::MalformedFeed(_) => {
            UpdateFailure::Unreachable
        }
        UpdateError::Verification(_) => UpdateFailure::Verification,
        UpdateError::Busy | UpdateError::InstanceRunning(_) => UpdateFailure::Busy,
        UpdateError::Download(_) | UpdateError::Local(_) => UpdateFailure::Download,
    }
}

impl UpdateCapability for PlatformCapability {
    fn kept(&mut self) -> Option<String> {
        let ready = self.updater.kept()?;
        let version = ready.version().to_owned();
        *self.ready.lock().ok()? = Some(ready);
        Some(version)
    }

    fn check(&mut self) -> Result<Option<String>, UpdateFailure> {
        let candidate = self.updater.check().map_err(|error| classify(&error))?;
        let version = candidate.as_ref().map(|c| c.version().to_owned());
        self.candidate = candidate;
        Ok(version)
    }

    fn download(&mut self, version: &str) -> Result<String, UpdateFailure> {
        let candidate = self
            .candidate
            .clone()
            .filter(|candidate| candidate.version() == version)
            .ok_or(UpdateFailure::Download)?;
        let ready = self
            .updater
            .download(&candidate)
            .map_err(|error| classify(&error))?;
        let version = ready.version().to_owned();
        if let Ok(mut slot) = self.ready.lock() {
            *slot = Some(ready);
        }
        Ok(version)
    }

    fn restart_guard(&self) -> RestartGuard {
        let updater = self.updater.clone();
        Box::new(move || {
            let others = updater.other_instances();
            if others.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "Close the other Made window of this install before restarting to update (process {}).",
                    others.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
                ))
            }
        })
    }

    fn restarter(&self) -> Restarter {
        let updater = self.updater.clone();
        let ready = self.ready.clone();
        Box::new(move |version: &str| {
            let ready = ready
                .lock()
                .map_err(|_| "the update state is unavailable".to_owned())?
                .clone()
                .filter(|ready| ready.version() == version)
                .ok_or_else(|| "the ready update is no longer available".to_owned())?;
            updater
                .restart_into(&ready)
                .map_err(|error| error.to_string())
        })
    }
}

// ---------------------------------------------------------------------------
// Command-line entry (REL-12)
// ---------------------------------------------------------------------------

/// The confirmed apply of the command-line entry: takes the ready version,
/// returns the line to print or the refusal.
pub type CliApply<'a> = &'a mut dyn FnMut(&str) -> Result<String, String>;

/// Run the non-interactive update entry: one check and its download through
/// the same scheduler and capability as About, then, only with `apply`, the
/// confirmed apply. Returns the exit code; lines go to `out`. The stored
/// automatic-check choice is never read or changed.
pub fn command_line_update(
    scheduler: &mut UpdateScheduler,
    apply: Option<CliApply<'_>>,
    out: &mut dyn std::io::Write,
) -> i32 {
    if !scheduler.available() {
        let _ = writeln!(
            out,
            "{}",
            UpdateStatus::NotAvailable.line(format_check_time)
        );
        return 0;
    }
    // Let the launch-time kept-update lookup finish first.
    scheduler.run_until_idle(Instant::now);
    scheduler.check_now();
    let status = scheduler.run_until_idle(Instant::now).clone();
    let busy = scheduler.take_events().contains(&UpdateEvent::Busy);
    if busy {
        let _ = writeln!(
            out,
            "Another update operation is in progress for this install."
        );
        return 1;
    }
    let _ = writeln!(out, "{}", status.line(format_check_time));
    let code = match status {
        UpdateStatus::CheckFailed
        | UpdateStatus::DownloadFailed(_)
        | UpdateStatus::VerificationFailed(_) => 1,
        _ => 0,
    };
    let Some(apply) = apply else {
        return code;
    };
    let Some(version) = scheduler.ready_version().map(str::to_owned) else {
        let _ = writeln!(out, "No verified update to apply.");
        return 1;
    };
    match apply(&version) {
        Ok(message) => {
            let _ = writeln!(out, "{message}");
            0
        }
        Err(message) => {
            let _ = writeln!(out, "made: {message}");
            1
        }
    }
}
