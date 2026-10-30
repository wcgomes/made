//! Read-only git metadata and worktree indicator.
//!
//! Made observes worktrees; it never creates, removes, merges, or prunes them.
//! Every git invocation goes through a closed [`GitCommand`] enum so the
//! process surface is a fixed read-only allowlist. There is no API for an
//! arbitrary git argument vector.
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

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

/// Environment variable that overrides the git executable.
///
/// Intended for packaged builds and tests. An empty value falls back to
/// `git` on `PATH`.
pub const GIT_PROGRAM_ENV: &str = "MADE_GIT_BIN";

/// Read-only git command surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GitCommand {
    /// `rev-parse --show-toplevel`
    ShowToplevel,
    /// `rev-parse --abbrev-ref HEAD`
    Branch,
    /// `rev-parse --git-dir`
    GitDir,
    /// `rev-parse --git-common-dir`
    CommonDir,
    /// `worktree list --porcelain`
    WorktreeList,
    /// `rev-parse --short=8 HEAD`
    ///
    /// Git may return more than 8 characters when a shorter id is ambiguous.
    /// The header displays the first 8 and keeps the full abbreviation for hover.
    CommitId,
    /// `log -1 --format=%s`
    CommitSubject,
    /// `status --porcelain=v1 -z --untracked-files=all -- .`
    ///
    /// `all` lists every untracked file. `normal` would collapse a new
    /// directory to one entry, so those files would never get a file dot.
    /// Ignored paths stay omitted.
    Status,
}

impl GitCommand {
    /// Every command the module can run.
    pub const ALL: [Self; 8] = [
        Self::ShowToplevel,
        Self::Branch,
        Self::GitDir,
        Self::CommonDir,
        Self::WorktreeList,
        Self::CommitId,
        Self::CommitSubject,
        Self::Status,
    ];

    /// Fixed argument vector for this command.
    pub const fn args(self) -> &'static [&'static str] {
        match self {
            Self::ShowToplevel => &["rev-parse", "--show-toplevel"],
            Self::Branch => &["rev-parse", "--abbrev-ref", "HEAD"],
            Self::GitDir => &["rev-parse", "--git-dir"],
            Self::CommonDir => &["rev-parse", "--git-common-dir"],
            Self::WorktreeList => &["worktree", "list", "--porcelain"],
            Self::CommitId => &["rev-parse", "--short=8", "HEAD"],
            Self::CommitSubject => &["log", "-1", "--format=%s"],
            Self::Status => &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--",
                ".",
            ],
        }
    }

    /// Stable name for diagnostics.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ShowToplevel => "show-toplevel",
            Self::Branch => "branch",
            Self::GitDir => "git-dir",
            Self::CommonDir => "common-dir",
            Self::WorktreeList => "worktree-list",
            Self::CommitId => "commit-id",
            Self::CommitSubject => "commit-subject",
            Self::Status => "status",
        }
    }
}

/// Complete read-only git argument allowlist.
pub const READ_ONLY_GIT_ARGUMENTS: &[&[&str]] = &[
    &["rev-parse", "--show-toplevel"],
    &["rev-parse", "--abbrev-ref", "HEAD"],
    &["rev-parse", "--git-dir"],
    &["rev-parse", "--git-common-dir"],
    &["worktree", "list", "--porcelain"],
    &["rev-parse", "--short=8", "HEAD"],
    &["log", "-1", "--format=%s"],
    &[
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--",
        ".",
    ],
];

/// Whether an argument vector is one of the read-only commands.
pub fn is_read_only_git_args(args: &[&str]) -> bool {
    READ_ONLY_GIT_ARGUMENTS.iter().any(|allowed| {
        allowed.len() == args.len()
            && allowed
                .iter()
                .zip(args.iter())
                .all(|(allowed, requested)| allowed == requested)
    })
}

/// Metadata availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataStatus {
    /// Git answered every read command used by the indicator.
    Known,
    /// The directory is not a git repository or a read failed.
    Unknown,
    /// The git executable is missing or could not be started.
    Unavailable,
}

impl MetadataStatus {
    /// Stable lowercase label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Known => "known",
            Self::Unknown => "unknown",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Best-effort read-only worktree metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeMetadata {
    /// Availability classification.
    pub status: MetadataStatus,
    /// Whether the path is inside a git worktree.
    pub is_worktree: bool,
    /// Canonical worktree top-level, when known.
    pub toplevel: Option<PathBuf>,
    /// Current branch, when attached.
    pub branch: Option<String>,
    /// Whether HEAD is detached.
    pub detached: bool,
    /// `rev-parse --short=8 HEAD` when detached. May be longer than 8 characters.
    pub commit_abbrev: Option<String>,
    /// Commit subject when detached. An empty string is a successful empty subject.
    pub commit_subject: Option<String>,
    /// Whether this directory is a linked worktree.
    pub is_linked_worktree: bool,
    /// Folder name of the worktree top-level. The REPOS header uses this, not the branch.
    pub directory_name: Option<String>,
    /// Branch-first label used by diagnostics. Not the REPOS worktree name.
    pub worktree_label: Option<String>,
    /// Git directory, normalized to an absolute path.
    pub git_dir: Option<PathBuf>,
    /// Common git directory shared by linked worktrees.
    pub common_dir: Option<PathBuf>,
    /// Number of entries in `git worktree list`.
    pub worktree_count: usize,
    /// Why metadata is unknown or unavailable.
    pub reason: Option<String>,
}

impl WorktreeMetadata {
    /// Metadata known to be unavailable because git cannot be run.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            status: MetadataStatus::Unavailable,
            is_worktree: false,
            toplevel: None,
            branch: None,
            detached: false,
            commit_abbrev: None,
            commit_subject: None,
            is_linked_worktree: false,
            directory_name: None,
            worktree_label: None,
            git_dir: None,
            common_dir: None,
            worktree_count: 0,
            reason: Some(reason.into()),
        }
    }

    /// Metadata that could not be established.
    pub fn unknown(reason: impl Into<String>) -> Self {
        Self {
            status: MetadataStatus::Unknown,
            is_worktree: false,
            toplevel: None,
            branch: None,
            detached: false,
            commit_abbrev: None,
            commit_subject: None,
            is_linked_worktree: false,
            directory_name: None,
            worktree_label: None,
            git_dir: None,
            common_dir: None,
            worktree_count: 0,
            reason: Some(reason.into()),
        }
    }

    /// Whether the metadata is fully known.
    pub const fn is_known(&self) -> bool {
        matches!(self.status, MetadataStatus::Known)
    }

    /// Header text for the explorer; never used as markup.
    pub fn header_label(&self) -> String {
        match self.status {
            MetadataStatus::Unavailable => "git unavailable".to_owned(),
            MetadataStatus::Unknown => "unknown git metadata".to_owned(),
            MetadataStatus::Known => {
                let kind = if self.is_linked_worktree {
                    "linked worktree"
                } else {
                    "worktree"
                };
                match &self.branch {
                    Some(branch) => format!("{kind} · {branch}"),
                    None if self.detached => format!("{kind} · detached HEAD"),
                    None => kind.to_owned(),
                }
            }
        }
    }
}

/// Runs the fixed read-only git commands.
#[derive(Debug, Clone)]
pub struct GitRunner {
    program: OsString,
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

    /// Runner using [`GIT_PROGRAM_ENV`] or `git`.
    pub fn from_env() -> Self {
        Self::from_env_value(env::var_os(GIT_PROGRAM_ENV))
    }

    /// Build a runner from an explicit environment value.
    ///
    /// This is the testable form of [`GitRunner::from_env`].
    pub fn from_env_value(program: Option<OsString>) -> Self {
        Self::with_program(
            program
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| OsString::from("git")),
        )
    }

    /// Runner with an explicit executable.
    pub fn with_program(program: impl Into<OsString>) -> Self {
        let program = program.into();
        Self {
            program: if program.is_empty() {
                OsString::from("git")
            } else {
                program
            },
        }
    }

    /// Configured git executable.
    pub fn program(&self) -> &OsStr {
        &self.program
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

        let toplevel = match self.run(GitCommand::ShowToplevel, directory) {
            Ok(output) if !output.is_empty() => {
                let path = PathBuf::from(output);
                Some(fs::canonicalize(&path).unwrap_or(path))
            }
            Ok(_) => {
                return WorktreeMetadata::unknown(format!(
                    "git did not report a worktree root for {}",
                    directory.display()
                ))
            }
            Err(GitRunError::Spawn(error)) => {
                return WorktreeMetadata::unavailable(format!(
                    "could not run git ({}): {error}",
                    self.program.to_string_lossy()
                ))
            }
            Err(GitRunError::Failed { stderr, .. }) => {
                return WorktreeMetadata::unknown(if stderr.is_empty() {
                    "not a git repository".to_owned()
                } else {
                    format!("not a git repository or git failed: {stderr}")
                })
            }
            Err(GitRunError::Rejected) => {
                return WorktreeMetadata::unknown("git arguments are not read-only")
            }
        };

        let mut unknown_reason: Option<String> = None;
        let branch_output = self.run(GitCommand::Branch, directory);
        let (branch, detached) = match branch_output {
            Ok(value) if value == "HEAD" => (None, true),
            Ok(value) if !value.is_empty() => (Some(value), false),
            Ok(_) => {
                unknown_reason.get_or_insert_with(|| "git did not report a branch".to_owned());
                (None, false)
            }
            Err(error) => {
                unknown_reason.get_or_insert_with(|| format!("branch read failed: {error}"));
                (None, false)
            }
        };
        let (commit_abbrev, commit_subject) = if detached {
            let abbrev = self
                .run(GitCommand::CommitId, directory)
                .ok()
                .filter(|value| !value.is_empty());
            let subject = self.run(GitCommand::CommitSubject, directory).ok();
            (abbrev, subject)
        } else {
            (None, None)
        };

        let git_dir = self
            .run(GitCommand::GitDir, directory)
            .ok()
            .and_then(|value| normalize_git_path(directory, &value));
        let common_dir = self
            .run(GitCommand::CommonDir, directory)
            .ok()
            .and_then(|value| normalize_git_path(directory, &value));
        if git_dir.is_none() {
            unknown_reason.get_or_insert_with(|| "git directory was not reported".to_owned());
        }
        if common_dir.is_none() {
            unknown_reason
                .get_or_insert_with(|| "git common directory was not reported".to_owned());
        }

        let is_linked_worktree =
            matches!((&git_dir, &common_dir), (Some(git), Some(common)) if git != common);

        let worktrees = self
            .run(GitCommand::WorktreeList, directory)
            .map(|output| parse_worktree_list(&output))
            .unwrap_or_default();
        let worktree_count = worktrees.len();

        let matching = toplevel.as_ref().and_then(|toplevel| {
            worktrees
                .iter()
                .find(|entry| entry.path.as_ref() == Some(toplevel))
        });
        let directory_name = toplevel.as_ref().and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
        });
        let worktree_label = matching
            .and_then(|entry| entry.branch.clone())
            .or_else(|| branch.clone())
            .or_else(|| directory_name.clone());

        let status = if unknown_reason.is_some() {
            MetadataStatus::Unknown
        } else {
            MetadataStatus::Known
        };

        WorktreeMetadata {
            status,
            is_worktree: true,
            toplevel,
            branch,
            detached,
            commit_abbrev,
            commit_subject,
            is_linked_worktree,
            directory_name,
            worktree_label,
            git_dir,
            common_dir,
            worktree_count,
            reason: unknown_reason,
        }
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
        // Porcelain paths are relative to the repository root. Canonicalize
        // both sides so a symlink prefix (`/var` vs `/private/var`) cannot
        // drop every entry.
        let directory = fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf());
        let toplevel =
            toplevel.map(|path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()));
        match self.run_status(&directory) {
            Ok(output) => {
                let records = parse_porcelain_v1_z(&output);
                GitStatusRead::Ready(group_status(&records, &directory, toplevel.as_deref()))
            }
            Err(GitRunError::Spawn(error)) => GitStatusRead::Unavailable {
                reason: format!(
                    "could not run git ({}): {error}",
                    self.program.to_string_lossy()
                ),
            },
            Err(error) => GitStatusRead::Failed {
                reason: error.to_string(),
            },
        }
    }

    fn run(&self, command: GitCommand, directory: &Path) -> Result<String, GitRunError> {
        self.spawn(command, directory, false)
            .map(|output| output.trim().to_owned())
    }

    /// Status output is NUL-separated and must not be trimmed.
    fn run_status(&self, directory: &Path) -> Result<String, GitRunError> {
        self.spawn(GitCommand::Status, directory, true)
    }

    fn spawn(
        &self,
        command: GitCommand,
        directory: &Path,
        status_cwd: bool,
    ) -> Result<String, GitRunError> {
        debug_assert!(is_read_only_git_args(command.args()));
        if !is_read_only_git_args(command.args()) {
            return Err(GitRunError::Rejected);
        }

        let mut process = Command::new(&self.program);
        process
            .arg("-C")
            .arg(directory)
            .args(command.args())
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat")
            .env("LC_ALL", "C");
        if status_cwd {
            process.current_dir(directory);
        }
        let output = process.output().map_err(GitRunError::Spawn)?;

        if !output.status.success() {
            return Err(GitRunError::Failed {
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// Read metadata with the environment-configured runner.
pub fn read_metadata(directory: impl AsRef<Path>) -> WorktreeMetadata {
    GitRunner::from_env().read_metadata(directory)
}

/// Whether Made exposes worktree lifecycle management. Always false in v1.
pub const WORKTREE_MANAGEMENT_AVAILABLE: bool = false;

#[derive(Debug)]
enum GitRunError {
    Spawn(io::Error),
    Failed { code: Option<i32>, stderr: String },
    Rejected,
}

impl fmt::Display for GitRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(formatter, "could not start git: {error}"),
            Self::Failed {
                code: Some(code),
                stderr,
            } => write!(formatter, "git exited with {code}: {stderr}"),
            Self::Failed { code: None, stderr } => {
                write!(formatter, "git was terminated: {stderr}")
            }
            Self::Rejected => formatter.write_str("git arguments are not read-only"),
        }
    }
}

fn normalize_git_path(directory: &Path, raw: &str) -> Option<PathBuf> {
    if raw.is_empty() {
        return None;
    }
    let path = PathBuf::from(raw);
    let absolute = if path.is_absolute() {
        path
    } else {
        directory.join(path)
    };
    Some(fs::canonicalize(&absolute).unwrap_or(absolute))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PorcelainWorktree {
    path: Option<PathBuf>,
    branch: Option<String>,
}

fn parse_worktree_list(text: &str) -> Vec<PorcelainWorktree> {
    let mut entries = Vec::new();
    let mut current: Option<PorcelainWorktree> = None;

    for line in text.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            let path = PathBuf::from(path);
            current = Some(PorcelainWorktree {
                path: Some(fs::canonicalize(&path).unwrap_or(path)),
                branch: None,
            });
        } else if let Some(branch) = line.strip_prefix("branch ") {
            if let Some(entry) = current.as_mut() {
                entry.branch = Some(
                    branch
                        .strip_prefix("refs/heads/")
                        .unwrap_or(branch)
                        .to_owned(),
                );
            }
        }
        // `detached`, `bare`, `HEAD`, and `prunable` lines do not change the
        // branch or path label used by the indicator.
    }
    if let Some(entry) = current {
        entries.push(entry);
    }
    entries
}

/// Section title for unmerged paths. Empty sections are omitted.
pub const GROUP_MERGE: &str = "Merge Changes";
/// Section title for index changes.
pub const GROUP_STAGED: &str = "Staged Changes";
/// Section title for worktree changes that are not staged.
pub const GROUP_CHANGES: &str = "Changes";
/// Section title for untracked paths.
pub const GROUP_UNTRACKED: &str = "Untracked";

/// Kind of change used by folder dots and row color.
///
/// Declaration order is the tooltip order for a mixed folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangeKind {
    /// Untracked path or staged add.
    New,
    /// Content or type change.
    Modified,
    /// Deletion.
    Deleted,
    /// Rename, or a git copy (no separate letter).
    Renamed,
    /// Unmerged path.
    Conflict,
}

impl ChangeKind {
    /// Tooltip name. Never relies on color alone.
    pub const fn label(self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Modified => "Modified",
            Self::Deleted => "Deleted",
            Self::Renamed => "Renamed",
            Self::Conflict => "Conflict",
        }
    }
}

/// One porcelain v1 `-z` record, still relative to the repository root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorcelainRecord {
    /// Index status (`X`).
    pub index: char,
    /// Worktree status (`Y`).
    pub worktree: char,
    /// Path without a trailing slash.
    pub path: String,
    /// Original path for a rename or copy.
    pub original: Option<String>,
    /// `true` when porcelain reported a directory (`path/`).
    pub is_directory: bool,
}

/// One row in a source-control group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusRow {
    /// `U`, `A`, `M`, `D`, `R`, or `C`.
    pub letter: char,
    /// Semantic kind for color. The letter is the non-color cue.
    pub kind: ChangeKind,
    /// Absolute path under the explorer root.
    pub path: PathBuf,
    /// Path relative to the explorer root, for display.
    pub relative: PathBuf,
    /// Directory entries select only; they do not open a preview.
    pub is_directory: bool,
    /// Original path relative to the explorer root, when it is inside the root.
    pub original_relative: Option<PathBuf>,
}

impl GitStatusRow {
    /// Whether activating the row should open the read-only preview.
    pub const fn opens_preview(&self) -> bool {
        !self.is_directory
    }
}

/// One non-empty source-control section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusGroup {
    /// Exact section title.
    pub title: &'static str,
    /// Rows in path order. A partially staged file may appear in two groups.
    pub rows: Vec<GitStatusRow>,
}

/// Kinds present in a folder, from the full status list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderMark {
    /// Unique kinds in tooltip order.
    pub kinds: Vec<ChangeKind>,
}

impl FolderMark {
    /// Two or more kinds share the neutral accent instead of one semantic color.
    pub fn is_mixed(&self) -> bool {
        self.kinds.len() > 1
    }

    /// The only kind, when the folder is not mixed.
    pub fn single(&self) -> Option<ChangeKind> {
        if self.kinds.len() == 1 {
            self.kinds.first().copied()
        } else {
            None
        }
    }

    /// Dot tooltip: one kind's name, or the kinds listed in order.
    pub fn tooltip(&self) -> String {
        self.kinds
            .iter()
            .map(|kind| kind.label())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Grouped status plus folder-dot marks keyed by absolute directory path.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitStatusSnapshot {
    /// Non-empty groups, in Merge / Staged / Changes / Untracked order.
    pub groups: Vec<GitStatusGroup>,
    /// Ancestor folders that contain a change, including collapsed ones.
    pub folders: BTreeMap<PathBuf, FolderMark>,
}

/// Outcome of one read-only status command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitStatusRead {
    /// Git answered. An empty snapshot is a clean tree, not a failure.
    Ready(GitStatusSnapshot),
    /// Git ran and failed, or the path could not be read.
    Failed { reason: String },
    /// The git executable is missing or could not be started.
    Unavailable { reason: String },
}

/// Parse `git status --porcelain=v1 -z` output.
///
/// Rename and copy records are `XY new\0old\0`. Ignored (`!!`) records are
/// dropped. A trailing slash marks a directory entry.
pub fn parse_porcelain_v1_z(text: &str) -> Vec<PorcelainRecord> {
    let mut records = Vec::new();
    let mut parts = text.split('\0');
    while let Some(record) = parts.next() {
        if record.is_empty() {
            continue;
        }
        let bytes = record.as_bytes();
        if bytes.len() < 4 || bytes[2] != b' ' {
            continue;
        }
        let index = record.chars().next().unwrap_or(' ');
        let worktree = record.chars().nth(1).unwrap_or(' ');
        if index == '!' || worktree == '!' {
            continue;
        }
        let mut path = record[3..].to_owned();
        let mut original = None;
        if is_rename_or_copy(index) || is_rename_or_copy(worktree) {
            if let Some(next) = parts.next() {
                if !next.is_empty() {
                    original = Some(next.to_owned());
                }
            }
        }
        let is_directory = path.ends_with('/');
        if is_directory {
            path.pop();
        }
        if path.is_empty() || path_escapes(&path) {
            continue;
        }
        records.push(PorcelainRecord {
            index,
            worktree,
            path,
            original,
            is_directory,
        });
    }
    records
}

/// Group porcelain records into source-control sections and folder dots.
///
/// Paths outside `explorer_root` are dropped. Porcelain paths are relative to
/// `toplevel` when it is known; otherwise they are treated as relative to the
/// explorer root.
pub fn group_status(
    records: &[PorcelainRecord],
    explorer_root: &Path,
    toplevel: Option<&Path>,
) -> GitStatusSnapshot {
    let mut merge = Vec::new();
    let mut staged = Vec::new();
    let mut changes = Vec::new();
    let mut untracked = Vec::new();
    let mut folder_kinds: BTreeMap<PathBuf, BTreeSet<ChangeKind>> = BTreeMap::new();

    for record in records {
        let Some(resolved) = resolve_status_path(explorer_root, toplevel, &record.path) else {
            continue;
        };
        let original_relative = record.original.as_deref().and_then(|original| {
            resolve_status_path(explorer_root, toplevel, original).map(|path| path.relative)
        });
        if is_unmerged(record.index, record.worktree) {
            push_row(
                &mut merge,
                'C',
                ChangeKind::Conflict,
                &resolved,
                record.is_directory,
                original_relative.clone(),
            );
            mark_folders(
                &mut folder_kinds,
                explorer_root,
                &resolved.absolute,
                record.is_directory,
                ChangeKind::Conflict,
            );
            continue;
        }
        if record.index == '?' && record.worktree == '?' {
            push_row(
                &mut untracked,
                'U',
                ChangeKind::New,
                &resolved,
                record.is_directory,
                None,
            );
            mark_folders(
                &mut folder_kinds,
                explorer_root,
                &resolved.absolute,
                record.is_directory,
                ChangeKind::New,
            );
            continue;
        }
        if let Some((letter, kind)) = side_letter(record.index) {
            push_row(
                &mut staged,
                letter,
                kind,
                &resolved,
                record.is_directory,
                original_relative.clone(),
            );
            mark_folders(
                &mut folder_kinds,
                explorer_root,
                &resolved.absolute,
                record.is_directory,
                kind,
            );
            if kind == ChangeKind::Renamed {
                if let Some(original) = record.original.as_deref() {
                    if let Some(old) = resolve_status_path(explorer_root, toplevel, original) {
                        mark_folders(
                            &mut folder_kinds,
                            explorer_root,
                            &old.absolute,
                            false,
                            ChangeKind::Renamed,
                        );
                    }
                }
            }
        }
        if let Some((letter, kind)) = side_letter(record.worktree) {
            push_row(
                &mut changes,
                letter,
                kind,
                &resolved,
                record.is_directory,
                original_relative,
            );
            mark_folders(
                &mut folder_kinds,
                explorer_root,
                &resolved.absolute,
                record.is_directory,
                kind,
            );
        }
    }

    sort_rows(&mut merge);
    sort_rows(&mut staged);
    sort_rows(&mut changes);
    sort_rows(&mut untracked);

    let mut groups = Vec::new();
    push_group(&mut groups, GROUP_MERGE, merge);
    push_group(&mut groups, GROUP_STAGED, staged);
    push_group(&mut groups, GROUP_CHANGES, changes);
    push_group(&mut groups, GROUP_UNTRACKED, untracked);

    let folders = folder_kinds
        .into_iter()
        .map(|(path, kinds)| {
            (
                path,
                FolderMark {
                    kinds: kinds.into_iter().collect(),
                },
            )
        })
        .collect();
    GitStatusSnapshot { groups, folders }
}

struct ResolvedPath {
    absolute: PathBuf,
    relative: PathBuf,
}

fn resolve_status_path(
    explorer_root: &Path,
    toplevel: Option<&Path>,
    porcelain: &str,
) -> Option<ResolvedPath> {
    if porcelain.is_empty() || path_escapes(porcelain) {
        return None;
    }
    let rel = Path::new(porcelain);
    let absolute = if let Some(top) = toplevel {
        let from_top = top.join(rel);
        let stripped = from_top.strip_prefix(explorer_root).ok()?;
        if stripped
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return None;
        }
        explorer_root.join(stripped)
    } else {
        explorer_root.join(rel)
    };
    let relative = absolute.strip_prefix(explorer_root).ok()?.to_path_buf();
    if relative.as_os_str().is_empty() {
        return None;
    }
    Some(ResolvedPath { absolute, relative })
}

fn path_escapes(path: &str) -> bool {
    Path::new(path)
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
}

fn is_rename_or_copy(code: char) -> bool {
    code == 'R' || code == 'C'
}

fn is_unmerged(index: char, worktree: char) -> bool {
    index == 'U'
        || worktree == 'U'
        || (index == 'A' && worktree == 'A')
        || (index == 'D' && worktree == 'D')
}

fn side_letter(code: char) -> Option<(char, ChangeKind)> {
    match code {
        'A' => Some(('A', ChangeKind::New)),
        'M' | 'T' => Some(('M', ChangeKind::Modified)),
        'D' => Some(('D', ChangeKind::Deleted)),
        // Git's `C` is copied. The display letter `C` is reserved for conflicts.
        'R' | 'C' => Some(('R', ChangeKind::Renamed)),
        _ => None,
    }
}

fn push_row(
    rows: &mut Vec<GitStatusRow>,
    letter: char,
    kind: ChangeKind,
    resolved: &ResolvedPath,
    is_directory: bool,
    original_relative: Option<PathBuf>,
) {
    rows.push(GitStatusRow {
        letter,
        kind,
        path: resolved.absolute.clone(),
        relative: resolved.relative.clone(),
        is_directory,
        original_relative,
    });
}

fn push_group(groups: &mut Vec<GitStatusGroup>, title: &'static str, rows: Vec<GitStatusRow>) {
    if rows.is_empty() {
        return;
    }
    groups.push(GitStatusGroup { title, rows });
}

fn sort_rows(rows: &mut [GitStatusRow]) {
    rows.sort_by(|left, right| left.relative.cmp(&right.relative));
}

fn mark_folders(
    folders: &mut BTreeMap<PathBuf, BTreeSet<ChangeKind>>,
    explorer_root: &Path,
    path: &Path,
    is_directory: bool,
    kind: ChangeKind,
) {
    let mut current = if is_directory {
        Some(path)
    } else {
        path.parent()
    };
    while let Some(dir) = current {
        if dir.as_os_str().is_empty() || dir == explorer_root || !dir.starts_with(explorer_root) {
            break;
        }
        folders.entry(dir.to_path_buf()).or_default().insert(kind);
        current = dir.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
