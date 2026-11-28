//! Read-only git metadata and worktree indicator.
//!
//! Made observes worktrees; it never creates, removes, merges, or prunes them.
//! Every git invocation goes through the closed [`GitCommand`] enum so the
//! process surface is a fixed read-only allowlist. There is no API for an
//! arbitrary git argument vector. The implementation, allowlist, and status
//! parsers are re-exported from `made_remote::git`.
//!
//! A non-repository folder or a branch/topology read failure yields unknown
//! metadata; a missing git executable yields unavailable metadata. Neither
//! blocks the explorer.
//!
//! [`GitRunner::read_metadata`] is a synchronous best-effort call. It runs a
//! fixed set of local `rev-parse`/`worktree list` commands; the UI should run
//! it on a worker when refreshing the header so a slow filesystem cannot stall
//! a frame. [`GitRunner::read_status`] is the same kind of call for
//! `status --porcelain=v1 -z`: read-only, fixed arguments, and never a reason
//! to block the explorer.
//!
//! The Open workspace screen reads the worktree list and change counts of its
//! launch target through [`WorktreeReads`]: bounded, on a worker thread, with
//! each reply tagged by the generation of the request it answers.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

pub use made_remote::git::{
    group_status, is_read_only_git_args, parse_porcelain_v1_z, parse_worktree_entries,
    ChangeCounts, ChangeKind, FolderMark, GitCommand, GitLimits, GitStatusGroup, GitStatusRead,
    GitStatusRow, GitStatusSnapshot, MetadataStatus, PillKind, PorcelainRecord, WorktreeEntry,
    WorktreeMetadata, GIT_PROGRAM_ENV, GROUP_CHANGES, GROUP_MERGE, GROUP_STAGED, GROUP_UNTRACKED,
    READ_ONLY_GIT_ARGUMENTS, WORKTREE_MANAGEMENT_AVAILABLE,
};

use crate::open_workspace::{CountsState, GitState, WorktreeItem};

/// Runs the fixed read-only git commands through the shared remote observer.
#[derive(Debug, Clone)]
pub struct GitRunner {
    inner: made_remote::git::GitRunner,
}

impl Default for GitRunner {
    fn default() -> Self {
        Self::from_env()
    }
}

impl GitRunner {
    /// Runner using [`GIT_PROGRAM_ENV`] or `git`.
    pub fn new() -> Self {
        Self::from_env()
    }

    /// Runner using [`GIT_PROGRAM_ENV`] or `git`, preserving local behavior.
    pub fn from_env() -> Self {
        Self::from_env_value(env::var_os(GIT_PROGRAM_ENV))
    }

    /// Build a runner from an explicit environment value.
    ///
    /// This is the testable form of [`GitRunner::from_env`]. An empty or
    /// missing value falls back to `git`.
    pub fn from_env_value(program: Option<OsString>) -> Self {
        Self::with_program(program.unwrap_or_default())
    }

    /// Runner with an explicit executable and no local execution limits.
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            inner: made_remote::git::GitRunner::with_limits(program, GitLimits::UNBOUNDED),
        }
    }

    /// Configured git executable.
    pub fn program(&self) -> &OsStr {
        self.inner.program()
    }

    /// Configured process execution limits.
    pub const fn limits(&self) -> GitLimits {
        self.inner.limits()
    }

    /// Read metadata for `directory`.
    ///
    /// Missing directories, non-repositories, and partial read failures return
    /// `Unknown`; a missing git executable returns `Unavailable`. The function
    /// never returns an error and never blocks longer than one git command.
    pub fn read_metadata(&self, directory: impl AsRef<Path>) -> WorktreeMetadata {
        let directory = directory.as_ref();
        if directory.as_os_str().is_empty() {
            return WorktreeMetadata::unknown("the directory path is empty");
        }
        let metadata = match fs::metadata(directory) {
            Ok(metadata) => metadata,
            Err(error) => {
                return WorktreeMetadata::unknown(format!(
                    "cannot inspect {}: {error}",
                    directory.display()
                ))
            }
        };
        if !metadata.is_dir() {
            return WorktreeMetadata::unknown(format!(
                "{} is not a directory",
                directory.display()
            ));
        }

        self.inner.read_metadata(directory)
    }

    /// Read porcelain status for `directory`.
    ///
    /// `directory` is both the process cwd and git's `-C` path. The fixed
    /// pathspec is `.`, so a subdirectory explorer root does not list the rest
    /// of the repository. `toplevel`, when known, interprets porcelain paths,
    /// which are relative to the repository root. A missing git executable is
    /// [`GitStatusRead::Unavailable`]; a failed read is [`GitStatusRead::Failed`].
    /// Neither panics.
    pub fn read_status(&self, directory: &Path, toplevel: Option<&Path>) -> GitStatusRead {
        if directory.as_os_str().is_empty() {
            return GitStatusRead::Failed {
                reason: "the directory path is empty".to_owned(),
            };
        }
        self.inner.read_status(directory, toplevel)
    }
}

impl GitRunner {
    /// The same executable with the bounded limits the Open workspace screen
    /// requires.
    fn bounded(&self) -> made_remote::git::GitRunner {
        made_remote::git::GitRunner::with_limits(self.program(), GitLimits::BOUNDED)
    }

    /// Read the Open workspace git state of `target`: the top level of the
    /// working tree that contains it and every working tree of the repository
    /// (main first, then repository order) from the read-only
    /// `worktree list --porcelain`, run from `target`. A non-repository, a
    /// missing git or a failed read is [`GitState::NoRepository`]. Counts are
    /// left pending; read them with [`Self::read_change_counts`].
    pub fn read_open_workspace_target(&self, target: &Path) -> GitState {
        self.read_target_unless(target, &|| false)
            .unwrap_or(GitState::NoRepository)
    }

    /// [`Self::read_open_workspace_target`], abandoned (`None`) when
    /// `superseded` holds between its two git commands.
    fn read_target_unless(&self, target: &Path, superseded: &dyn Fn() -> bool) -> Option<GitState> {
        let runner = self.bounded();
        let Some(toplevel) = runner
            .read_bytes(GitCommand::ShowToplevel, target)
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
            .filter(|text| !text.is_empty())
            .map(|text| canonical(Path::new(&text)))
        else {
            return Some(GitState::NoRepository);
        };
        if superseded() {
            return None;
        }
        let entries = runner
            .read_bytes(GitCommand::WorktreeList, target)
            .map(|bytes| parse_worktree_entries(&String::from_utf8_lossy(&bytes)))
            .unwrap_or_default();
        let mut worktrees: Vec<WorktreeItem> = entries
            .into_iter()
            .map(|entry| {
                let path = canonical(&entry.path);
                WorktreeItem {
                    missing: entry.missing || fs::read_dir(&path).is_err(),
                    label: entry.head_label(),
                    path,
                    counts: CountsState::Pending,
                }
            })
            .collect();
        if !worktrees.iter().any(|item| item.path == toplevel) {
            // Without a usable list the working tree itself is the only item.
            worktrees = vec![WorktreeItem {
                path: toplevel.clone(),
                label: None,
                missing: false,
                counts: CountsState::Pending,
            }];
        }
        Some(GitState::Repository {
            toplevel,
            worktrees,
            counts: CountsState::Pending,
        })
    }

    /// Whole-working-tree change counts of the working tree at `worktree`
    /// (its top level), from the bounded read-only status read.
    pub fn read_change_counts(&self, worktree: &Path) -> CountsState {
        match self.bounded().read_status(worktree, Some(worktree)) {
            GitStatusRead::Ready(snapshot) => {
                CountsState::Ready(ChangeCounts::from_snapshot(&snapshot))
            }
            GitStatusRead::Failed { .. } | GitStatusRead::Unavailable { .. } => CountsState::Failed,
        }
    }
}

fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// One reply of an Open workspace git read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeReadUpdate {
    /// The git state of the requested launch target.
    Target {
        /// Generation of the request.
        generation: u64,
        /// Repository and worktree list, counts pending.
        state: GitState,
    },
    /// Change counts of one working tree of that state.
    Counts {
        /// Generation of the request.
        generation: u64,
        /// Working-tree top level the counts belong to.
        path: PathBuf,
        /// Its counts.
        counts: CountsState,
    },
}

/// Off-thread Open workspace git reads on one worker thread, started with
/// the first request. Requests do not queue: the worker takes the newest
/// one, checks between git commands whether it was superseded and then
/// moves to the newest, so at most one git process runs at a time however
/// fast the target changes. Replies of any request but the latest are
/// discarded.
#[derive(Debug)]
pub struct WorktreeReads {
    runner: GitRunner,
    latest: Option<u64>,
    shared: Arc<WorkerSlot>,
    worker: Option<thread::JoinHandle<()>>,
    sender: Sender<WorktreeReadUpdate>,
    receiver: Receiver<WorktreeReadUpdate>,
}

/// The newest request and whether the owner is gone.
#[derive(Debug, Default)]
struct WorkerSlot {
    state: Mutex<WorkerState>,
    wake: Condvar,
}

#[derive(Debug, Default)]
struct WorkerState {
    request: Option<(u64, PathBuf)>,
    latest: u64,
    closed: bool,
}

impl WorkerSlot {
    fn lock(&self) -> std::sync::MutexGuard<'_, WorkerState> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Whether `generation` is no longer the newest request.
    fn superseded(&self, generation: u64) -> bool {
        let state = self.lock();
        state.closed || state.latest != generation
    }

    /// Block until a request is waiting; `None` once the owner is gone.
    fn next(&self) -> Option<(u64, PathBuf)> {
        let mut state = self.lock();
        loop {
            if state.closed {
                return None;
            }
            if let Some(request) = state.request.take() {
                return Some(request);
            }
            state = self
                .wake
                .wait(state)
                .unwrap_or_else(|poison| poison.into_inner());
        }
    }
}

impl WorktreeReads {
    /// Reads with `runner`'s executable and the bounded limits.
    pub fn new(runner: GitRunner) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            runner,
            latest: None,
            shared: Arc::new(WorkerSlot::default()),
            worker: None,
            sender,
            receiver,
        }
    }

    /// Read `target` for request `generation`, superseding earlier requests.
    /// The worker reads the worktree list, then the counts of every listed
    /// worktree when `target` is a top level with linked worktrees, else of
    /// the working tree that contains `target`. A worker that cannot start
    /// leaves the screen without git details.
    pub fn request(&mut self, generation: u64, target: PathBuf) {
        self.latest = Some(generation);
        {
            let mut state = self.shared.lock();
            state.latest = generation;
            state.request = Some((generation, target));
        }
        self.shared.wake.notify_one();
        if self.worker.is_none() {
            let runner = self.runner.clone();
            let shared = Arc::clone(&self.shared);
            let sender = self.sender.clone();
            self.worker = thread::Builder::new()
                .name("made-open-workspace-git".to_owned())
                .spawn(move || {
                    while let Some((generation, target)) = shared.next() {
                        read_target(&runner, &shared, &sender, generation, &target);
                    }
                })
                .ok();
        }
    }

    /// Replies of the latest request received so far.
    pub fn poll(&mut self) -> Vec<WorktreeReadUpdate> {
        let latest = self.latest;
        self.receiver
            .try_iter()
            .filter(|update| {
                let generation = match update {
                    WorktreeReadUpdate::Target { generation, .. }
                    | WorktreeReadUpdate::Counts { generation, .. } => *generation,
                };
                Some(generation) == latest
            })
            .collect()
    }
}

impl Drop for WorktreeReads {
    /// Stop the worker after its current git command; never waits for it.
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.wake.notify_one();
    }
}

/// One request on the worker, abandoned between git commands as soon as a
/// newer request arrives.
fn read_target(
    runner: &GitRunner,
    shared: &WorkerSlot,
    sender: &Sender<WorktreeReadUpdate>,
    generation: u64,
    target: &Path,
) {
    let Some(state) = runner.read_target_unless(target, &|| shared.superseded(generation)) else {
        return;
    };
    if shared.superseded(generation) {
        return;
    }
    let counted: Vec<PathBuf> = match &state {
        GitState::NoRepository => Vec::new(),
        GitState::Repository {
            toplevel,
            worktrees,
            ..
        } if toplevel == target && worktrees.len() > 1 => worktrees
            .iter()
            .filter(|item| !item.missing)
            .map(|item| item.path.clone())
            .collect(),
        GitState::Repository { toplevel, .. } => vec![toplevel.clone()],
    };
    if sender
        .send(WorktreeReadUpdate::Target { generation, state })
        .is_err()
    {
        return;
    }
    for path in counted {
        if shared.superseded(generation) {
            return;
        }
        let counts = runner.read_change_counts(&path);
        let update = WorktreeReadUpdate::Counts {
            generation,
            path,
            counts,
        };
        if sender.send(update).is_err() {
            return;
        }
    }
}

/// Read metadata with the environment-configured runner.
pub fn read_metadata(directory: impl AsRef<Path>) -> WorktreeMetadata {
    GitRunner::from_env().read_metadata(directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn grouped(porcelain: &str) -> GitStatusSnapshot {
        let records = parse_porcelain_v1_z(porcelain);
        group_status(&records, Path::new("/repo"), Some(Path::new("/repo")))
    }

    fn letters(snapshot: &GitStatusSnapshot, title: &str) -> Vec<(char, String)> {
        snapshot
            .groups
            .iter()
            .find(|group| group.title == title)
            .map(|group| {
                group
                    .rows
                    .iter()
                    .map(|row| (row.letter, row.relative.display().to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn porcelain_groups_partial_stage_conflict_and_omits_empty_sections() {
        let snapshot = grouped(
            "MM src/both.rs\0AM src/added.rs\0UU src/clash.rs\0?? src/new.txt\0D  src/gone.rs\0 M src/dirty.rs\0R  src/next.rs\0src/prev.rs\0!! src/skip.log\0",
        );
        let titles: Vec<_> = snapshot.groups.iter().map(|group| group.title).collect();
        assert_eq!(
            titles,
            vec![GROUP_MERGE, GROUP_STAGED, GROUP_CHANGES, GROUP_UNTRACKED]
        );
        assert_eq!(
            letters(&snapshot, GROUP_MERGE),
            vec![('C', "src/clash.rs".to_owned())]
        );
        assert_eq!(
            letters(&snapshot, GROUP_STAGED),
            vec![
                ('A', "src/added.rs".to_owned()),
                ('M', "src/both.rs".to_owned()),
                ('D', "src/gone.rs".to_owned()),
                ('R', "src/next.rs".to_owned()),
            ]
        );
        assert_eq!(
            letters(&snapshot, GROUP_CHANGES),
            vec![
                ('M', "src/added.rs".to_owned()),
                ('M', "src/both.rs".to_owned()),
                ('M', "src/dirty.rs".to_owned()),
            ]
        );
        assert_eq!(
            letters(&snapshot, GROUP_UNTRACKED),
            vec![('U', "src/new.txt".to_owned())]
        );
        let renamed = snapshot
            .groups
            .iter()
            .find(|group| group.title == GROUP_STAGED)
            .and_then(|group| {
                group
                    .rows
                    .iter()
                    .find(|row| row.relative.ends_with("next.rs"))
            })
            .unwrap();
        assert_eq!(
            renamed.original_relative.as_deref(),
            Some(Path::new("src/prev.rs"))
        );
        assert!(renamed.opens_preview());
    }

    #[test]
    fn directory_entries_do_not_open_a_preview() {
        let snapshot = grouped("?? vendor/\0");
        let row = &snapshot.groups[0].rows[0];
        assert!(row.is_directory);
        assert!(!row.opens_preview());
        assert_eq!(row.letter, 'U');
    }

    #[test]
    fn folder_dots_aggregate_single_and_mixed_kinds_including_collapsed_parents() {
        let snapshot = grouped(" M src/nested/dirty.rs\0?? src/nested/new.txt\0D  other/gone.rs\0");
        let nested = snapshot
            .folders
            .get(Path::new("/repo/src/nested"))
            .expect("collapsed nested folder");
        let parent = snapshot
            .folders
            .get(Path::new("/repo/src"))
            .expect("collapsed parent");
        let other = snapshot
            .folders
            .get(Path::new("/repo/other"))
            .expect("deleted ancestor");
        assert!(nested.is_mixed());
        assert!(parent.is_mixed());
        assert_eq!(nested.tooltip(), "New, Modified");
        assert_eq!(parent.tooltip(), "New, Modified");
        assert!(!other.is_mixed());
        assert_eq!(other.single(), Some(ChangeKind::Deleted));
        assert_eq!(other.tooltip(), "Deleted");
        assert!(!snapshot.folders.contains_key(Path::new("/repo")));
        assert!(!snapshot
            .folders
            .contains_key(Path::new("/repo/src/nested/dirty.rs")));
    }

    #[test]
    fn subdirectory_root_drops_paths_outside_the_explorer_root() {
        let records =
            parse_porcelain_v1_z(" M README.md\0 M sub/nested/keep.txt\0?? sub/nested/\0");
        let snapshot = group_status(&records, Path::new("/repo/sub"), Some(Path::new("/repo")));
        let paths: Vec<_> = snapshot
            .groups
            .iter()
            .flat_map(|group| group.rows.iter().map(|row| row.relative.clone()))
            .collect();
        assert_eq!(
            paths,
            vec![PathBuf::from("nested/keep.txt"), PathBuf::from("nested")]
        );
        let nested = snapshot
            .folders
            .get(Path::new("/repo/sub/nested"))
            .expect("folder inside the subdirectory root");
        assert!(nested.is_mixed());
        assert!(!snapshot
            .folders
            .keys()
            .any(|path| path.ends_with("README.md")));
        assert!(!snapshot.groups.iter().any(|group| group
            .rows
            .iter()
            .any(|row| row.relative.ends_with("README.md"))));
    }

    #[test]
    fn status_command_stays_on_the_closed_allowlist() {
        assert_eq!(GitCommand::ALL.len(), READ_ONLY_GIT_ARGUMENTS.len());
        for command in GitCommand::ALL {
            assert!(is_read_only_git_args(command.args()));
            assert!(READ_ONLY_GIT_ARGUMENTS.contains(&command.args()));
        }
        assert_eq!(
            GitCommand::CommitId.args(),
            &["rev-parse", "--short=8", "HEAD"]
        );
        assert_eq!(
            GitCommand::CommitSubject.args(),
            &["log", "-1", "--format=%s"]
        );
        assert_eq!(
            GitCommand::Status.args(),
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--ignore-submodules=dirty",
                "--",
                "."
            ]
        );
        assert!(!is_read_only_git_args(&["add", "."]));
        assert!(!is_read_only_git_args(&["commit", "-m", "x"]));
        assert!(!is_read_only_git_args(&["checkout", "main"]));
        assert!(!is_read_only_git_args(&["status", "--porcelain=v1"]));
    }
}
