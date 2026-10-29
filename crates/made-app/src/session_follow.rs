//! Active-session follow: normalized snapshot to explorer root.
//!
//! The coordinator consumes the bridge's [`SessionSnapshot`] and decides
//! whether the explorer root must change. A connected active directory takes
//! precedence over the independently retained shell cwd; releasing that session
//! restores shell follow. Same-directory updates do not invalidate the tree.
//!
//! Root switches advance a [`RootGeneration`] so the explorer can reject tree
//! loads and filesystem events started for a previous root. Stale snapshot
//! revisions are discarded by the coordinator, and a new plugin connection
//! begins a new revision epoch.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use made_bridge::server::BridgeEvent;
use made_bridge::state::{SessionSnapshot, SnapshotError, SnapshotTracker};
use made_explorer::root_switch::{GenerationGate, RootGeneration};

use crate::workspace::{LaunchPlan, WorkspaceError, WorkspaceErrorKind};

/// The active session identity after a snapshot is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowedSession {
    /// OpenCode session identifier.
    pub session_id: String,
    /// Session title, retained as untrusted display text.
    pub title: String,
    /// Directory exactly as reported by OpenCode, before canonicalization.
    pub requested_directory: Option<PathBuf>,
    /// Canonical directory when it exists and is readable.
    pub canonical_directory: Option<PathBuf>,
    /// Optional subpath. Context only; it never changes the explorer root.
    pub subpath: Option<String>,
    /// Whether the directory exists and is readable.
    pub available: bool,
}

impl FollowedSession {
    /// Best available directory for display.
    pub fn directory(&self) -> Option<&Path> {
        self.canonical_directory
            .as_deref()
            .or(self.requested_directory.as_deref())
    }
}

/// The connected OpenCode view directory after a snapshot is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowedViewDirectory {
    /// Directory exactly as reported by OpenCode, before canonicalization.
    pub requested: PathBuf,
    /// Canonical directory when it exists and is readable.
    pub canonical: Option<PathBuf>,
    /// Whether the directory exists and is readable.
    pub available: bool,
    /// Why the directory cannot be used, when unavailable.
    pub reason: Option<String>,
}

impl FollowedViewDirectory {
    /// Best available directory for display.
    pub fn directory(&self) -> &Path {
        self.canonical.as_deref().unwrap_or(&self.requested)
    }
}

/// Which owner selected the effective explorer root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootSource {
    /// The latest validated shell cwd or launch folder.
    Shell,
    /// A connected OpenCode active session or view directory.
    OpenCode,
}

impl RootSource {
    /// Stable identifier used by diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::OpenCode => "opencode",
        }
    }
}

/// What one accepted or rejected snapshot changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionFollowUpdate {
    /// The revision was at or below the last accepted revision.
    StaleRevision {
        /// Rejected revision.
        revision: u64,
        /// Last accepted revision in the epoch.
        last_accepted: u64,
    },
    /// The snapshot failed local validation.
    RejectedSnapshot {
        /// Validation detail.
        detail: String,
    },
    /// No OpenCode session is active. The current root is retained.
    NoActiveSession,
    /// The active session was applied. Check `root_switched` to know whether
    /// the explorer root changed.
    ActiveSession,
    /// No active session; the usable connected view directory was applied.
    ViewDirectory,
    /// The connected view directory is reported but cannot be used. The shell
    /// fallback is current and no unavailable-root state is presented.
    ViewDirectoryUnavailable {
        /// Requested view directory.
        directory: PathBuf,
        /// Why the directory cannot be used.
        detail: String,
    },
    /// The active session directory is missing, unreadable, relative, or was
    /// not reported. The previous root is retained.
    ActiveDirectoryUnavailable {
        /// Active session identifier.
        session_id: String,
        /// Requested directory, when one was reported.
        directory: Option<PathBuf>,
        /// Why the directory cannot be used.
        detail: String,
    },
}

impl SessionFollowUpdate {
    /// Whether this outcome represents a discarded stale revision.
    pub const fn is_stale(&self) -> bool {
        matches!(self, Self::StaleRevision { .. })
    }

    /// Whether the active directory was unavailable.
    pub const fn is_unavailable(&self) -> bool {
        matches!(self, Self::ActiveDirectoryUnavailable { .. })
    }
}

/// The result of applying one snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFollowOutcome {
    /// What changed.
    pub update: SessionFollowUpdate,
    /// Explorer root after applying the snapshot.
    pub root: PathBuf,
    /// Root generation after applying the snapshot.
    pub generation: RootGeneration,
    /// Whether the explorer root changed in this update.
    pub root_switched: bool,
    /// Active session identity after applying the snapshot.
    pub active_session: Option<FollowedSession>,
}

impl SessionFollowOutcome {
    /// Whether the revision was discarded as stale.
    pub const fn stale_rejected(&self) -> bool {
        self.update.is_stale()
    }

    /// Active session identifier, if one is active.
    pub fn active_session_id(&self) -> Option<&str> {
        self.active_session
            .as_ref()
            .map(|session| session.session_id.as_str())
    }
}

/// Coordinator failure when no usable root can be established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowError {
    /// Candidate paths that were tried, in precedence order.
    pub candidates: Vec<PathBuf>,
    /// Last underlying workspace error.
    pub error: WorkspaceError,
}

impl fmt::Display for FollowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "cannot establish a session-follow root: {}",
            self.error
        )
    }
}

impl Error for FollowError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}

/// Default lifetime of a cached directory validation.
const DEFAULT_DIRECTORY_CACHE_TTL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
struct CachedDirectoryCheck {
    requested: PathBuf,
    result: Result<PathBuf, WorkspaceError>,
    checked_at: Instant,
}

/// An authoritative session directory that could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
struct UnavailableRoot {
    session_id: String,
    requested: PathBuf,
}

/// Follows the active OpenCode session and owns the active explorer root.
#[derive(Debug, Clone)]
pub struct SessionFollowCoordinator {
    launch_workspace: Option<PathBuf>,
    terminal_cwd: Option<PathBuf>,
    current_root: PathBuf,
    generation: GenerationGate,
    last_known_active_root: Option<PathBuf>,
    active_session: Option<FollowedSession>,
    view_directory: Option<FollowedViewDirectory>,
    unavailable: Option<UnavailableRoot>,
    connection_id: Option<u64>,
    tracker: SnapshotTracker,
    directory_check: Option<CachedDirectoryCheck>,
    view_check: Option<CachedDirectoryCheck>,
    directory_cache_ttl: Duration,
    directory_validation_count: u64,
    fallback_cache: Option<(PathBuf, Instant)>,
}

impl SessionFollowCoordinator {
    /// Create a coordinator from a launch workspace and terminal fallback.
    ///
    /// The launch workspace wins when it is usable; otherwise the terminal
    /// working directory is used. At least one must be an existing readable
    /// directory.
    pub fn new(
        launch_workspace: impl AsRef<Path>,
        terminal_cwd: impl AsRef<Path>,
    ) -> Result<Self, FollowError> {
        let launch_requested = launch_workspace.as_ref().to_path_buf();
        let terminal_requested = terminal_cwd.as_ref().to_path_buf();

        let launch = usable_directory(&launch_requested);
        let terminal = usable_directory(&terminal_requested);
        let launch_ok = launch.as_ref().ok().cloned();
        let terminal_ok = terminal.as_ref().ok().cloned();

        let current_root = match launch_ok.clone().or_else(|| terminal_ok.clone()) {
            Some(root) => root,
            None => {
                let error = match (&launch, &terminal) {
                    (Err(launch_error), Err(terminal_error)) => WorkspaceError::new(
                        terminal_error.kind(),
                        terminal_error.requested(),
                        format!(
                            "{}; launch workspace: {}",
                            terminal_error.detail(),
                            launch_error.message()
                        ),
                    ),
                    (Err(launch_error), _) => launch_error.clone(),
                    (_, Err(terminal_error)) => terminal_error.clone(),
                    _ => WorkspaceError::new(
                        WorkspaceErrorKind::InvalidPath,
                        terminal_requested.clone(),
                        "no usable workspace root",
                    ),
                };
                return Err(FollowError {
                    candidates: vec![launch_requested, terminal_requested],
                    error,
                });
            }
        };

        Ok(Self {
            launch_workspace: launch_ok,
            terminal_cwd: terminal_ok,
            current_root,
            generation: GenerationGate::new(RootGeneration::INITIAL),
            last_known_active_root: None,
            active_session: None,
            view_directory: None,
            unavailable: None,
            connection_id: None,
            tracker: SnapshotTracker::new(),
            directory_check: None,
            view_check: None,
            directory_cache_ttl: DEFAULT_DIRECTORY_CACHE_TTL,
            directory_validation_count: 0,
            fallback_cache: None,
        })
    }

    /// Create a coordinator from a validated launch plan.
    pub fn from_launch_plan(plan: &LaunchPlan) -> Result<Self, FollowError> {
        Self::new(plan.workspace.path(), plan.terminal_cwd())
    }

    /// Current explorer root. Always a usable directory unless it was removed
    /// after the last follow update.
    pub fn current_root(&self) -> &Path {
        &self.current_root
    }

    /// Current root generation.
    pub const fn generation(&self) -> RootGeneration {
        self.generation.current()
    }

    /// Active session identity.
    pub fn active_session(&self) -> Option<&FollowedSession> {
        self.active_session.as_ref()
    }

    /// Connected OpenCode view directory, when one is reported.
    pub fn view_directory(&self) -> Option<&FollowedViewDirectory> {
        self.view_directory.as_ref()
    }

    /// Which owner selected the effective explorer root.
    pub fn effective_root_source(&self) -> RootSource {
        if self.active_session.is_some() || self.view_is_effective() {
            RootSource::OpenCode
        } else {
            RootSource::Shell
        }
    }

    fn view_is_effective(&self) -> bool {
        self.active_session.is_none()
            && self
                .view_directory
                .as_ref()
                .is_some_and(|view| view.available)
    }

    /// Whether OpenCode currently owns the effective root.
    pub fn is_agent_owned(&self) -> bool {
        self.effective_root_source() == RootSource::OpenCode
    }

    /// Last root established by an active session.
    pub fn last_known_root(&self) -> Option<&Path> {
        self.last_known_active_root.as_deref()
    }

    /// Requested directory of the currently unavailable authoritative root.
    ///
    /// The last usable root remains current and displayed; this reports the
    /// identity that could not be followed.
    pub fn unavailable_root(&self) -> Option<&Path> {
        self.unavailable
            .as_ref()
            .map(|unavailable| unavailable.requested.as_path())
    }

    /// Whether an authoritative root is currently unavailable.
    pub fn is_root_unavailable(&self) -> bool {
        self.unavailable.is_some()
    }

    /// Last accepted revision in the current connection epoch.
    pub fn last_revision(&self) -> Option<u64> {
        self.tracker.last_accepted_revision()
    }

    /// Current connection epoch.
    pub fn epoch(&self) -> u64 {
        self.tracker.epoch()
    }

    /// Reset revision tracking for a new plugin connection epoch.
    ///
    /// The previous epoch's connected view directory is cleared before the new
    /// epoch can accept snapshots.
    pub fn begin_epoch(&mut self) -> u64 {
        self.clear_view_directory();
        self.tracker.begin_epoch()
    }

    /// Override the directory-validation cache lifetime. Intended for tests.
    pub fn set_directory_cache_ttl(&mut self, ttl: Duration) {
        self.directory_cache_ttl = ttl;
    }

    /// Instant of the last actual filesystem directory validation.
    pub fn last_directory_check(&self) -> Option<Instant> {
        self.directory_check.as_ref().map(|check| check.checked_at)
    }

    /// Number of actual filesystem directory validations performed.
    pub fn directory_validation_count(&self) -> u64 {
        self.directory_validation_count
    }

    /// Force the next active-session application to revalidate the directory.
    pub fn invalidate_directory_cache(&mut self) {
        self.directory_check = None;
    }

    /// Apply a bridge event, returning a follow outcome for state snapshots.
    ///
    /// A connection starts a new epoch; lifecycle release events restore the
    /// latest shell cwd. Replaced sockets and events from foreign or rejected
    /// connections never release the current authenticated session.
    pub fn apply_bridge_event(&mut self, event: &BridgeEvent) -> Option<SessionFollowOutcome> {
        match event {
            BridgeEvent::Connected(info) => {
                self.connection_id = Some(info.connection_id);
                self.begin_epoch();
                Some(self.release_active_session())
            }
            BridgeEvent::State(snapshot) => Some(self.apply_snapshot(snapshot.clone())),
            BridgeEvent::Disconnected(info)
                if !info.replaced && self.owns_connection(Some(info.connection_id)) =>
            {
                self.connection_id = None;
                Some(self.release_active_session())
            }
            BridgeEvent::Goodbye(goodbye) if self.owns_connection(Some(goodbye.connection_id)) => {
                self.connection_id = None;
                Some(self.release_active_session())
            }
            BridgeEvent::HeartbeatTimeout(timeout)
                if self.owns_connection(Some(timeout.connection_id)) =>
            {
                self.connection_id = None;
                Some(self.release_active_session())
            }
            BridgeEvent::ProtocolError(error)
                if error.fatal && self.owns_connection(error.connection_id) =>
            {
                if error.connection_id.is_some() {
                    self.connection_id = None;
                }
                Some(self.release_active_session())
            }
            _ => None,
        }
    }

    fn owns_connection(&self, connection_id: Option<u64>) -> bool {
        match connection_id {
            Some(connection_id) => self
                .connection_id
                .is_none_or(|current| current == connection_id),
            None => self.connection_id.is_none(),
        }
    }

    /// Apply a normalized session snapshot.
    ///
    /// Stale or invalid snapshots are discarded without changing the root or
    /// the active identity.
    pub fn apply_snapshot(&mut self, snapshot: SessionSnapshot) -> SessionFollowOutcome {
        match self.tracker.accept(snapshot) {
            Err(SnapshotError::StaleRevision {
                revision,
                last_accepted,
            }) => self.outcome(
                SessionFollowUpdate::StaleRevision {
                    revision,
                    last_accepted,
                },
                false,
            ),
            Err(error) => self.outcome(
                SessionFollowUpdate::RejectedSnapshot {
                    detail: error.to_string(),
                },
                false,
            ),
            Ok(snapshot) => self.apply_accepted(snapshot),
        }
    }

    /// Update the terminal working directory used by fallback resolution.
    ///
    /// Invalid paths are ignored so a stale OSC 7 report cannot degrade the
    /// fallback chain.
    pub fn set_terminal_cwd(&mut self, cwd: impl AsRef<Path>) {
        if let Ok(canonical) = usable_directory(cwd.as_ref()) {
            self.fallback_cache = None;
            self.terminal_cwd = Some(canonical);
        }
    }

    /// Validate a changed shell report once, retaining the usable tree on error.
    /// A connected active session or usable view directory keeps precedence
    /// until explicitly released.
    pub fn follow_shell_cwd(&mut self, cwd: &Path) -> Result<SessionFollowOutcome, WorkspaceError> {
        let canonical = usable_directory(cwd)?;
        self.fallback_cache = None;
        self.terminal_cwd = Some(canonical);
        if self.active_session.is_some() {
            return Ok(self.outcome(SessionFollowUpdate::ActiveSession, false));
        }
        if self.view_is_effective() {
            return Ok(self.outcome(SessionFollowUpdate::ViewDirectory, false));
        }
        Ok(self.release_to_fallback(SessionFollowUpdate::NoActiveSession))
    }

    /// Release agent precedence on no-active, tabs-disabled or disconnection.
    ///
    /// The connected view directory belongs to the connection epoch, so it is
    /// cleared here before the shell fallback is applied.
    pub fn release_active_session(&mut self) -> SessionFollowOutcome {
        self.clear_view_directory();
        self.release_to_fallback(SessionFollowUpdate::NoActiveSession)
    }

    fn release_to_fallback(&mut self, update: SessionFollowUpdate) -> SessionFollowOutcome {
        self.active_session = None;
        self.unavailable = None;
        self.directory_check = None;
        let now = Instant::now();
        let root = match &self.fallback_cache {
            Some((root, checked))
                if now.saturating_duration_since(*checked) < self.directory_cache_ttl =>
            {
                root.clone()
            }
            _ => {
                let root = self.fallback_root();
                self.fallback_cache = Some((root.clone(), now));
                root
            }
        };
        let switched = root != self.current_root;
        if switched {
            self.generation.advance();
            self.current_root = root;
        }
        self.outcome(update, switched)
    }

    /// Terminal working directory known to the coordinator.
    pub fn terminal_cwd(&self) -> Option<&Path> {
        self.terminal_cwd.as_deref()
    }

    /// Launch workspace known to the coordinator.
    pub fn launch_workspace(&self) -> Option<&Path> {
        self.launch_workspace.as_deref()
    }

    /// Latest usable shell cwd, then launch folder, never a stale agent root.
    ///
    /// A candidate that no longer exists or is unreadable is skipped. The
    /// current root is the last resort so the UI always has a path to show.
    pub fn fallback_root(&self) -> PathBuf {
        for path in [&self.terminal_cwd, &self.launch_workspace]
            .into_iter()
            .flatten()
        {
            if is_usable_directory(path) {
                return path.clone();
            }
        }
        self.current_root.clone()
    }

    fn apply_accepted(&mut self, snapshot: SessionSnapshot) -> SessionFollowOutcome {
        if !snapshot.tabs_enabled {
            // The view directory is scoped to tabs-enabled sessions. Clear it
            // before the degraded shell-cwd fallback can be observed.
            return self.release_active_session();
        }

        if let Some(view) = snapshot.view_directory.clone() {
            self.record_view_directory(&view);
        } else {
            self.clear_view_directory();
        }

        let Some(active_id) = snapshot.active_session_id.clone() else {
            return self.apply_view_or_fallback();
        };

        let Some(session) = snapshot
            .sessions
            .iter()
            .find(|session| session.session_id == active_id)
            .cloned()
        else {
            // `SessionSnapshot::validate` rejects this, but keep the coordinator
            // defensive if a future protocol accepts it.
            return self.outcome(
                SessionFollowUpdate::RejectedSnapshot {
                    detail: format!("active session {active_id:?} is not in the snapshot"),
                },
                false,
            );
        };

        let requested = session.directory.clone();
        let mut followed = FollowedSession {
            session_id: session.session_id.clone(),
            title: session.title.clone(),
            requested_directory: requested.clone(),
            canonical_directory: None,
            subpath: session.subpath.clone(),
            available: false,
        };

        let Some(requested) = requested else {
            return self.release_active_session();
        };

        if !requested.is_absolute() {
            self.active_session = Some(followed);
            self.mark_unavailable(&active_id, &requested);
            return self.outcome(
                SessionFollowUpdate::ActiveDirectoryUnavailable {
                    session_id: active_id,
                    directory: Some(requested),
                    detail: "the active session directory is not absolute".to_owned(),
                },
                false,
            );
        }

        match self.validate_directory(&requested) {
            Ok(canonical) => {
                self.unavailable = None;
                followed.canonical_directory = Some(canonical.clone());
                followed.available = true;
                let switched = canonical != self.current_root;
                if switched {
                    self.generation.advance();
                    self.current_root = canonical.clone();
                }
                self.last_known_active_root = Some(canonical);
                self.active_session = Some(followed);
                self.outcome(SessionFollowUpdate::ActiveSession, switched)
            }
            Err(error) => {
                self.active_session = Some(followed);
                self.mark_unavailable(&active_id, &requested);
                self.outcome(
                    SessionFollowUpdate::ActiveDirectoryUnavailable {
                        session_id: active_id,
                        directory: Some(requested),
                        detail: error.message(),
                    },
                    false,
                )
            }
        }
    }

    fn apply_view_or_fallback(&mut self) -> SessionFollowOutcome {
        let Some(view) = self.view_directory.as_ref() else {
            return self.release_active_session();
        };
        let available = view.available;
        let requested = view.requested.clone();
        let canonical = view.canonical.clone();
        let reason = view.reason.clone();

        if !available {
            return self.release_to_fallback(SessionFollowUpdate::ViewDirectoryUnavailable {
                directory: requested,
                detail: reason.unwrap_or_else(|| "the view directory is unavailable".to_owned()),
            });
        }
        let Some(canonical) = canonical else {
            return self.release_active_session();
        };

        self.active_session = None;
        self.unavailable = None;
        self.directory_check = None;
        let switched = canonical != self.current_root;
        if switched {
            self.generation.advance();
            self.current_root = canonical;
        }
        self.outcome(SessionFollowUpdate::ViewDirectory, switched)
    }

    fn record_view_directory(&mut self, requested: &Path) {
        match self.validate_view_directory(requested) {
            Ok(canonical) => {
                self.view_directory = Some(FollowedViewDirectory {
                    requested: requested.to_path_buf(),
                    canonical: Some(canonical),
                    available: true,
                    reason: None,
                });
            }
            Err(error) => {
                self.view_directory = Some(FollowedViewDirectory {
                    requested: requested.to_path_buf(),
                    canonical: None,
                    available: false,
                    reason: Some(error.message()),
                });
            }
        }
    }

    fn clear_view_directory(&mut self) {
        self.view_directory = None;
        self.view_check = None;
    }

    fn mark_unavailable(&mut self, session_id: &str, requested: &Path) {
        let unchanged = self.unavailable.as_ref().is_some_and(|unavailable| {
            unavailable.session_id == session_id && unavailable.requested == requested
        });
        if unchanged {
            return;
        }
        self.unavailable = Some(UnavailableRoot {
            session_id: session_id.to_owned(),
            requested: requested.to_path_buf(),
        });
        // The effective root identity changed to an unavailable directory.
        // Advance the generation even though the last usable tree stays
        // displayed, so work started for the previous identity is rejected.
        self.generation.advance();
    }

    fn validate_directory(&mut self, requested: &Path) -> Result<PathBuf, WorkspaceError> {
        let ttl = self.directory_cache_ttl;
        Self::validate_cached_directory(
            &mut self.directory_check,
            &mut self.directory_validation_count,
            ttl,
            requested,
        )
    }

    fn validate_view_directory(&mut self, requested: &Path) -> Result<PathBuf, WorkspaceError> {
        let ttl = self.directory_cache_ttl;
        Self::validate_cached_directory(
            &mut self.view_check,
            &mut self.directory_validation_count,
            ttl,
            requested,
        )
    }

    fn validate_cached_directory(
        cache: &mut Option<CachedDirectoryCheck>,
        validation_count: &mut u64,
        ttl: Duration,
        requested: &Path,
    ) -> Result<PathBuf, WorkspaceError> {
        let now = Instant::now();
        if let Some(cached) = cache {
            if cached.requested == requested
                && now.saturating_duration_since(cached.checked_at) < ttl
            {
                // A cached failure is only reused while the path still looks
                // absent. A directory that reappears is revalidated at the next
                // snapshot instead of waiting for the cache TTL.
                let may_have_recovered =
                    cached.result.is_err() && std::fs::metadata(requested).is_ok();
                if !may_have_recovered {
                    return cached.result.clone();
                }
            }
        }

        let result = usable_directory(requested);
        *validation_count = (*validation_count).saturating_add(1);
        *cache = Some(CachedDirectoryCheck {
            requested: requested.to_path_buf(),
            result: result.clone(),
            checked_at: now,
        });
        result
    }

    fn outcome(&self, update: SessionFollowUpdate, root_switched: bool) -> SessionFollowOutcome {
        SessionFollowOutcome {
            update,
            root: self.current_root.clone(),
            generation: self.generation.current(),
            root_switched,
            active_session: self.active_session.clone(),
        }
    }
}

fn usable_directory(path: &Path) -> Result<PathBuf, WorkspaceError> {
    if path.as_os_str().is_empty() {
        return Err(WorkspaceError::new(
            WorkspaceErrorKind::InvalidPath,
            path,
            "directory path is empty",
        ));
    }
    if !path.is_absolute() {
        return Err(WorkspaceError::new(
            WorkspaceErrorKind::InvalidPath,
            path,
            "directory path is not absolute",
        ));
    }

    let canonical =
        fs::canonicalize(path).map_err(|error| WorkspaceError::from_io(path, &error))?;
    let metadata =
        fs::metadata(&canonical).map_err(|error| WorkspaceError::from_io(path, &error))?;
    if !metadata.is_dir() {
        return Err(WorkspaceError::new(
            WorkspaceErrorKind::NotADirectory,
            path,
            "path is not a directory",
        ));
    }
    let mut entries =
        fs::read_dir(&canonical).map_err(|error| WorkspaceError::from_io(path, &error))?;
    if let Some(Err(error)) = entries.next() {
        return Err(WorkspaceError::from_io(path, &error));
    }
    Ok(canonical)
}

fn is_usable_directory(path: &Path) -> bool {
    usable_directory(path).is_ok()
}
