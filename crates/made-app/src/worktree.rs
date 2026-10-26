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
//! a frame.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
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
}

impl GitCommand {
    /// Every command the module can run.
    pub const ALL: [Self; 5] = [
        Self::ShowToplevel,
        Self::Branch,
        Self::GitDir,
        Self::CommonDir,
        Self::WorktreeList,
    ];

    /// Fixed argument vector for this command.
    pub const fn args(self) -> &'static [&'static str] {
        match self {
            Self::ShowToplevel => &["rev-parse", "--show-toplevel"],
            Self::Branch => &["rev-parse", "--abbrev-ref", "HEAD"],
            Self::GitDir => &["rev-parse", "--git-dir"],
            Self::CommonDir => &["rev-parse", "--git-common-dir"],
            Self::WorktreeList => &["worktree", "list", "--porcelain"],
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
    /// Whether this directory is a linked worktree.
    pub is_linked_worktree: bool,
    /// Branch or worktree label used by the header.
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
            is_linked_worktree: false,
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
            is_linked_worktree: false,
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
        let worktree_label = matching
            .and_then(|entry| entry.branch.clone())
            .or_else(|| branch.clone())
            .or_else(|| {
                toplevel
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
            });

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
            is_linked_worktree,
            worktree_label,
            git_dir,
            common_dir,
            worktree_count,
            reason: unknown_reason,
        }
    }

    fn run(&self, command: GitCommand, directory: &Path) -> Result<String, GitRunError> {
        debug_assert!(is_read_only_git_args(command.args()));
        if !is_read_only_git_args(command.args()) {
            return Err(GitRunError::Rejected);
        }

        let output = Command::new(&self.program)
            .arg("-C")
            .arg(directory)
            .args(command.args())
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat")
            .env("LC_ALL", "C")
            .output()
            .map_err(GitRunError::Spawn)?;

        if !output.status.success() {
            return Err(GitRunError::Failed {
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
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
