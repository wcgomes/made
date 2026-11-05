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

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::Path;

pub use made_remote::git::{
    group_status, is_read_only_git_args, parse_porcelain_v1_z, ChangeKind, FolderMark, GitCommand,
    GitLimits, GitStatusGroup, GitStatusRead, GitStatusRow, GitStatusSnapshot, MetadataStatus,
    PorcelainRecord, WorktreeMetadata, GIT_PROGRAM_ENV, GROUP_CHANGES, GROUP_MERGE, GROUP_STAGED,
    GROUP_UNTRACKED, READ_ONLY_GIT_ARGUMENTS, WORKTREE_MANAGEMENT_AVAILABLE,
};

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
