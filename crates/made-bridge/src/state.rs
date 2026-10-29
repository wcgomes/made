//! Normalized session snapshot model and validation.
//!
//! A snapshot is a full replacement for the previous state: every accepted
//! `state` message carries the complete open-session list. Revisions are
//! monotonic within a plugin connection epoch. A new authenticated connection
//! (a new hello) starts a new epoch and resets revision tracking.
//!
//! The wire field names are defined by [`crate::protocol::StateMessage`] and
//! match the production OpenCode plugin. Validation is intentionally strict
//! about size and identity, and deliberately tolerant about transient
//! no-active-session states observed during tab focus transitions.

use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use crate::security::{
    MAX_PATH_CHARS, MAX_PROJECT_ID_CHARS, MAX_SESSIONS, MAX_SESSION_ID_CHARS, MAX_TITLE_CHARS,
};

/// A complete normalized view of the open root-session tabs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    /// Monotonic revision within the current connection epoch. Must be positive.
    pub revision: u64,
    /// The active session identifier, or `None` when no tab is active.
    #[serde(rename = "activeSessionID")]
    pub active_session_id: Option<String>,
    /// Whether OpenCode reports that session tabs are available.
    pub tabs_enabled: bool,
    /// Optional absolute view directory selected in OpenCode's Home/new-session
    /// view. Additive within protocol major 1.
    #[serde(default)]
    pub view_directory: Option<PathBuf>,
    /// Open root-session tabs.
    pub sessions: Vec<SessionInfo>,
}

/// One normalized OpenCode root-session tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    /// OpenCode session identifier.
    #[serde(rename = "sessionID")]
    pub session_id: String,
    /// Session title. Treated as untrusted display text.
    pub title: String,
    /// Whether this is the active root-session tab.
    pub active: bool,
    /// Whether the session is busy.
    #[serde(default)]
    pub busy: bool,
    /// Whether the session needs attention.
    #[serde(default)]
    pub attention: bool,
    /// Whether the session has unread activity.
    ///
    /// OpenCode v2.1.4 does not expose this field. It defaults to false in the
    /// production plugin and is retained for protocol forward compatibility.
    #[serde(default)]
    pub unread: bool,
    /// OpenCode prompt pulse counter.
    #[serde(default)]
    pub prompt_pulse: u32,
    /// Whether the tab is being renamed.
    #[serde(default)]
    pub renaming: bool,
    /// Absolute session working directory, which may be a worktree root.
    #[serde(default)]
    pub directory: Option<PathBuf>,
    /// Optional subpath within the project or worktree.
    #[serde(default)]
    pub subpath: Option<String>,
    /// OpenCode project identifier.
    #[serde(default, rename = "projectID")]
    pub project_id: Option<String>,
}

/// Reasons a snapshot cannot be accepted.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SnapshotError {
    /// Revisions start at one; zero is not a valid accepted revision.
    #[error("state revision must be positive")]
    ZeroRevision,
    /// A snapshot carried more than [`MAX_SESSIONS`] sessions.
    #[error("state has {count} sessions; the maximum is {max}")]
    TooManySessions {
        /// Observed session count.
        count: usize,
        /// Maximum permitted session count.
        max: usize,
    },
    /// A session identifier was empty.
    #[error("session at index {index} has an empty session id")]
    EmptySessionId {
        /// Index in the session array.
        index: usize,
    },
    /// A session identifier was longer than [`MAX_SESSION_ID_CHARS`].
    #[error("session at index {index} has a {length}-character id; the maximum is {max}")]
    SessionIdTooLong {
        /// Index in the session array.
        index: usize,
        /// Observed length in characters.
        length: usize,
        /// Maximum permitted length.
        max: usize,
    },
    /// Two sessions used the same identifier.
    #[error("session at index {index} duplicates an earlier session id")]
    DuplicateSessionId {
        /// Index in the session array.
        index: usize,
    },
    /// A title was longer than [`MAX_TITLE_CHARS`].
    #[error("session at index {index} has a {length}-character title; the maximum is {max}")]
    TitleTooLong {
        /// Index in the session array.
        index: usize,
        /// Observed length in characters.
        length: usize,
        /// Maximum permitted length.
        max: usize,
    },
    /// A directory or subpath was longer than [`MAX_PATH_CHARS`].
    #[error("session at index {index} {field} is {length} characters; the maximum is {max}")]
    PathTooLong {
        /// Index in the session array.
        index: usize,
        /// Field name (`directory` or `subpath`).
        field: &'static str,
        /// Observed length in characters.
        length: usize,
        /// Maximum permitted length.
        max: usize,
    },
    /// A session-derived string contained a NUL character.
    #[error("session at index {index} {field} contains a NUL character")]
    NulInField {
        /// Index in the session array.
        index: usize,
        /// Field name.
        field: &'static str,
    },
    /// A directory was present but empty.
    #[error("session at index {index} has an empty directory")]
    EmptyDirectory {
        /// Index in the session array.
        index: usize,
    },
    /// A directory was not absolute.
    #[error("session at index {index} directory is not absolute")]
    DirectoryNotAbsolute {
        /// Index in the session array.
        index: usize,
    },
    /// A path contained a `.` or `..` segment.
    #[error("session at index {index} {field} contains a traversal segment")]
    PathTraversal {
        /// Index in the session array.
        index: usize,
        /// Field name (`directory` or `subpath`).
        field: &'static str,
    },
    /// A subpath was absolute or rooted instead of relative.
    #[error("session at index {index} subpath must be relative")]
    SubpathNotRelative {
        /// Index in the session array.
        index: usize,
    },
    /// `viewDirectory` was present but empty.
    #[error("viewDirectory is empty")]
    EmptyViewDirectory,
    /// `viewDirectory` was not absolute.
    #[error("viewDirectory is not absolute")]
    ViewDirectoryNotAbsolute,
    /// `viewDirectory` contained a NUL character.
    #[error("viewDirectory contains a NUL character")]
    NulInViewDirectory,
    /// `viewDirectory` contained a `.` or `..` segment.
    #[error("viewDirectory contains a traversal segment")]
    ViewDirectoryTraversal,
    /// `viewDirectory` was longer than [`MAX_PATH_CHARS`].
    #[error("viewDirectory is {length} characters; the maximum is {max}")]
    ViewDirectoryTooLong {
        /// Observed length in characters.
        length: usize,
        /// Maximum permitted length.
        max: usize,
    },
    /// A project identifier was longer than [`MAX_PROJECT_ID_CHARS`].
    #[error("session at index {index} projectID is {length} characters; the maximum is {max}")]
    ProjectIdTooLong {
        /// Index in the session array.
        index: usize,
        /// Observed length in characters.
        length: usize,
        /// Maximum permitted length.
        max: usize,
    },
    /// More than one session was marked active.
    #[error("state has {count} active sessions; at most one is allowed")]
    MultipleActiveSessions {
        /// Number of sessions marked active.
        count: usize,
    },
    /// The active session id did not appear in the session list.
    #[error("active session id {0:?} is not present in sessions")]
    ActiveSessionMissing(String),
    /// `activeSessionID` named an existing session that was not marked active.
    #[error("active session id {0:?} is not marked active")]
    ActiveSessionNotActive(String),
    /// The single active flag and `activeSessionID` disagreed.
    #[error(
        "activeSessionID {active_session_id:?} does not match the session flagged active {flagged:?}"
    )]
    ActiveSessionIdMismatch {
        /// `activeSessionID`, when present.
        active_session_id: Option<String>,
        /// The session marked active.
        flagged: String,
    },
    /// The revision was not newer than the last accepted revision.
    #[error(
        "state revision {revision} is not newer than the last accepted revision {last_accepted}"
    )]
    StaleRevision {
        /// Revision that was rejected.
        revision: u64,
        /// Last accepted revision in this epoch.
        last_accepted: u64,
    },
}

impl SessionSnapshot {
    /// Validate the snapshot against the protocol limits and identity rules.
    ///
    /// The rules are: positive revision, at most [`MAX_SESSIONS`] sessions,
    /// unique non-empty session ids at most [`MAX_SESSION_ID_CHARS`]
    /// characters, titles at most [`MAX_TITLE_CHARS`] characters, directories
    /// non-empty and absolute, subpaths relative, and session-derived strings
    /// free of NUL characters. Directory and subpath are at most
    /// [`MAX_PATH_CHARS`] characters; `projectID` is at most
    /// [`MAX_PROJECT_ID_CHARS`]. Paths containing `.` or `..` segments are
    /// rejected. An optional `viewDirectory` is validated like a session
    /// directory as defense-in-depth against malformed state.
    ///
    /// At most one session may be marked active. When `activeSessionID` is
    /// present it must name that session, and when a session is marked active
    /// `activeSessionID` must name it; a snapshot with neither is valid (the
    /// transient no-active-tab state).
    pub fn validate(&self) -> Result<(), SnapshotError> {
        if self.revision == 0 {
            return Err(SnapshotError::ZeroRevision);
        }
        if self.sessions.len() > MAX_SESSIONS {
            return Err(SnapshotError::TooManySessions {
                count: self.sessions.len(),
                max: MAX_SESSIONS,
            });
        }
        if let Some(view_directory) = &self.view_directory {
            validate_view_directory(view_directory)?;
        }

        let mut seen = HashSet::with_capacity(self.sessions.len());
        let mut active_count = 0_usize;
        let mut flagged_active: Option<&str> = None;

        for (index, session) in self.sessions.iter().enumerate() {
            if session.session_id.is_empty() {
                return Err(SnapshotError::EmptySessionId { index });
            }
            let id_length = session.session_id.chars().count();
            if id_length > MAX_SESSION_ID_CHARS {
                return Err(SnapshotError::SessionIdTooLong {
                    index,
                    length: id_length,
                    max: MAX_SESSION_ID_CHARS,
                });
            }
            if session.session_id.contains('\0') {
                return Err(SnapshotError::NulInField {
                    index,
                    field: "sessionID",
                });
            }
            if !seen.insert(session.session_id.as_str()) {
                return Err(SnapshotError::DuplicateSessionId { index });
            }

            let title_length = session.title.chars().count();
            if title_length > MAX_TITLE_CHARS {
                return Err(SnapshotError::TitleTooLong {
                    index,
                    length: title_length,
                    max: MAX_TITLE_CHARS,
                });
            }

            if let Some(directory) = &session.directory {
                validate_directory(index, directory)?;
            }
            if let Some(subpath) = &session.subpath {
                validate_subpath(index, subpath)?;
            }
            if let Some(project_id) = &session.project_id {
                validate_project_id(index, project_id)?;
            }

            if session.active {
                active_count += 1;
                flagged_active = Some(session.session_id.as_str());
            }
        }

        if active_count > 1 {
            return Err(SnapshotError::MultipleActiveSessions {
                count: active_count,
            });
        }

        if let Some(active_id) = &self.active_session_id {
            if !seen.contains(active_id.as_str()) {
                return Err(SnapshotError::ActiveSessionMissing(active_id.clone()));
            }
        }

        match (self.active_session_id.as_deref(), flagged_active) {
            (Some(active), Some(flagged)) if active != flagged => {
                return Err(SnapshotError::ActiveSessionIdMismatch {
                    active_session_id: Some(active.to_owned()),
                    flagged: flagged.to_owned(),
                });
            }
            (None, Some(flagged)) => {
                return Err(SnapshotError::ActiveSessionIdMismatch {
                    active_session_id: None,
                    flagged: flagged.to_owned(),
                });
            }
            (Some(active), None) => {
                return Err(SnapshotError::ActiveSessionNotActive(active.to_owned()));
            }
            _ => {}
        }

        Ok(())
    }
}

fn validate_directory(index: usize, path: &Path) -> Result<(), SnapshotError> {
    let raw = path.to_string_lossy();
    let length = raw.chars().count();
    if length > MAX_PATH_CHARS {
        return Err(SnapshotError::PathTooLong {
            index,
            field: "directory",
            length,
            max: MAX_PATH_CHARS,
        });
    }
    if path.as_os_str().is_empty() {
        return Err(SnapshotError::EmptyDirectory { index });
    }
    if raw.contains('\0') {
        return Err(SnapshotError::NulInField {
            index,
            field: "directory",
        });
    }
    if !path.is_absolute() {
        return Err(SnapshotError::DirectoryNotAbsolute { index });
    }
    if has_dot_segment(&raw)
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(SnapshotError::PathTraversal {
            index,
            field: "directory",
        });
    }
    Ok(())
}

fn validate_view_directory(path: &Path) -> Result<(), SnapshotError> {
    let raw = path.to_string_lossy();
    let length = raw.chars().count();
    if length > MAX_PATH_CHARS {
        return Err(SnapshotError::ViewDirectoryTooLong {
            length,
            max: MAX_PATH_CHARS,
        });
    }
    if path.as_os_str().is_empty() {
        return Err(SnapshotError::EmptyViewDirectory);
    }
    if raw.contains('\0') {
        return Err(SnapshotError::NulInViewDirectory);
    }
    if !path.is_absolute() {
        return Err(SnapshotError::ViewDirectoryNotAbsolute);
    }
    if has_dot_segment(&raw)
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(SnapshotError::ViewDirectoryTraversal);
    }
    Ok(())
}

fn validate_subpath(index: usize, subpath: &str) -> Result<(), SnapshotError> {
    let length = subpath.chars().count();
    if length > MAX_PATH_CHARS {
        return Err(SnapshotError::PathTooLong {
            index,
            field: "subpath",
            length,
            max: MAX_PATH_CHARS,
        });
    }
    if subpath.contains('\0') {
        return Err(SnapshotError::NulInField {
            index,
            field: "subpath",
        });
    }

    let path = Path::new(subpath);
    if path.is_absolute()
        || path.has_root()
        || path
            .components()
            .any(|component| matches!(component, Component::RootDir | Component::Prefix(_)))
    {
        return Err(SnapshotError::SubpathNotRelative { index });
    }
    if has_parent_segment(subpath)
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(SnapshotError::PathTraversal {
            index,
            field: "subpath",
        });
    }
    Ok(())
}

fn validate_project_id(index: usize, project_id: &str) -> Result<(), SnapshotError> {
    if project_id.contains('\0') {
        return Err(SnapshotError::NulInField {
            index,
            field: "projectID",
        });
    }
    let length = project_id.chars().count();
    if length > MAX_PROJECT_ID_CHARS {
        return Err(SnapshotError::ProjectIdTooLong {
            index,
            length,
            max: MAX_PROJECT_ID_CHARS,
        });
    }
    Ok(())
}

fn has_dot_segment(raw: &str) -> bool {
    raw.split(std::path::is_separator)
        .any(|segment| segment == "." || segment == "..")
}

fn has_parent_segment(raw: &str) -> bool {
    raw.split(std::path::is_separator)
        .any(|segment| segment == "..")
}

/// Revision and epoch tracking for received snapshots.
///
/// A tracker rejects any revision at or below the last accepted revision. Call
/// [`SnapshotTracker::begin_epoch`] when a new authenticated connection starts;
/// this resets revision tracking and increments the epoch counter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotTracker {
    last_revision: u64,
    epoch: u64,
}

impl SnapshotTracker {
    /// Create a tracker in epoch zero with no accepted revisions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate and accept `snapshot` if its revision is newer.
    ///
    /// On success the snapshot is returned unchanged and becomes the new
    /// high-water mark. Stale snapshots are rejected with
    /// [`SnapshotError::StaleRevision`] and leave the tracker unchanged.
    pub fn accept(&mut self, snapshot: SessionSnapshot) -> Result<SessionSnapshot, SnapshotError> {
        snapshot.validate()?;
        if snapshot.revision <= self.last_revision {
            return Err(SnapshotError::StaleRevision {
                revision: snapshot.revision,
                last_accepted: self.last_revision,
            });
        }
        self.last_revision = snapshot.revision;
        Ok(snapshot)
    }

    /// Reset revision tracking for a new plugin connection epoch.
    ///
    /// Returns the new epoch number. A state snapshot from the new connection
    /// may legitimately restart at revision one.
    pub fn begin_epoch(&mut self) -> u64 {
        self.last_revision = 0;
        self.epoch = self.epoch.saturating_add(1);
        self.epoch
    }

    /// The current epoch number. The initial tracker is epoch zero.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The last accepted revision in the current epoch, if any.
    pub fn last_accepted_revision(&self) -> Option<u64> {
        if self.last_revision == 0 {
            None
        } else {
            Some(self.last_revision)
        }
    }
}
