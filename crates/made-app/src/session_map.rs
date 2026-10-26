//! Session map model for open OpenCode root sessions.
//!
//! The map is a headless model: the UI renders [`SessionRow`] values and calls
//! [`SessionMap::select`] when the user activates a row. Session titles and
//! paths are untrusted display text and are never interpreted as markup.
//!
//! Rows follow snapshot order, which keeps the map stable across activity
//! updates. Adding, removing, and changing the active session produce an
//! explicit [`SessionMapUpdate`] so a view can avoid a full relayout.

use std::collections::HashSet;
use std::path::PathBuf;

use made_bridge::state::{SessionInfo, SessionSnapshot};

/// Default title width used by [`SessionRow::display_title`] callers.
pub const DEFAULT_TITLE_CHARS: usize = 48;

/// Default path width used by [`SessionRow::display_path`] callers.
pub const DEFAULT_PATH_CHARS: usize = 64;

/// One open root-session row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    /// OpenCode session identifier.
    pub session_id: String,
    /// Full, untrusted session title.
    pub title: String,
    /// Working directory or worktree path reported by OpenCode.
    pub directory: Option<PathBuf>,
    /// Optional subpath within the project.
    pub subpath: Option<String>,
    /// Whether this is the active root session.
    pub active: bool,
    /// Whether the session is busy.
    pub busy: bool,
    /// Whether the session needs attention.
    pub attention: bool,
    /// Unread activity, using the plugin's derived boolean.
    pub unread: bool,
    /// OpenCode prompt pulse counter.
    pub prompt_pulse: u32,
    /// Whether the tab is being renamed.
    pub renaming: bool,
}

impl SessionRow {
    fn from_info(info: &SessionInfo, active: bool) -> Self {
        let title = if info.title.is_empty() {
            "(untitled)".to_owned()
        } else {
            info.title.clone()
        };
        Self {
            session_id: info.session_id.clone(),
            title,
            directory: info.directory.clone(),
            subpath: info.subpath.clone(),
            active,
            busy: info.busy,
            attention: info.attention,
            unread: info.unread,
            prompt_pulse: info.prompt_pulse,
            renaming: info.renaming,
        }
    }

    /// Title truncated to `max_chars` with a trailing ellipsis.
    pub fn display_title(&self, max_chars: usize) -> String {
        truncate_end(&self.title, max_chars)
    }

    /// Path truncated in the middle so the final components remain visible.
    pub fn display_path(&self, max_chars: usize) -> String {
        match &self.directory {
            Some(directory) => truncate_path(&directory.to_string_lossy(), max_chars),
            None => truncate_end("path unavailable", max_chars),
        }
    }

    fn activity_key(&self) -> (bool, bool, bool, u32, bool, bool) {
        (
            self.active,
            self.busy,
            self.attention,
            self.prompt_pulse,
            self.unread,
            self.renaming,
        )
    }

    fn content_key(&self) -> (&str, Option<&PathBuf>, Option<&str>) {
        (
            self.title.as_str(),
            self.directory.as_ref(),
            self.subpath.as_deref(),
        )
    }
}

/// One model update, suitable for driving a view diff.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionMapUpdate {
    /// Sessions that appeared.
    pub added: Vec<String>,
    /// Sessions that disappeared.
    pub removed: Vec<String>,
    /// Sessions whose title, path, subpath, or indicators changed.
    pub updated: Vec<String>,
    /// Whether the active session changed.
    pub active_changed: bool,
    /// Whether the snapshot order changed.
    pub reordered: bool,
}

impl SessionMapUpdate {
    /// Whether anything visible changed.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.updated.is_empty()
            && !self.active_changed
            && !self.reordered
    }
}

/// Headless session map.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionMap {
    rows: Vec<SessionRow>,
    active: Option<String>,
    selected: Option<String>,
}

impl SessionMap {
    /// Empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the map contents from a snapshot.
    pub fn apply_snapshot(&mut self, snapshot: &SessionSnapshot) -> SessionMapUpdate {
        let previous_rows = std::mem::take(&mut self.rows);
        let previous_ids: Vec<String> = previous_rows
            .iter()
            .map(|row| row.session_id.clone())
            .collect();
        let previous_active = self.active.clone();

        let active = resolve_active(snapshot);
        let mut rows = Vec::with_capacity(snapshot.sessions.len());
        for info in &snapshot.sessions {
            let is_active = active.as_deref() == Some(info.session_id.as_str());
            rows.push(SessionRow::from_info(info, is_active));
        }

        let new_ids: Vec<String> = rows.iter().map(|row| row.session_id.clone()).collect();
        let old_ids: HashSet<&str> = previous_ids.iter().map(String::as_str).collect();
        let new_id_set: HashSet<&str> = new_ids.iter().map(String::as_str).collect();

        let added = new_ids
            .iter()
            .filter(|id| !old_ids.contains(id.as_str()))
            .cloned()
            .collect();
        let removed = previous_ids
            .iter()
            .filter(|id| !new_id_set.contains(id.as_str()))
            .cloned()
            .collect();

        let mut updated = Vec::new();
        for row in &rows {
            let Some(old) = previous_rows
                .iter()
                .find(|old| old.session_id == row.session_id)
            else {
                continue;
            };
            if old.activity_key() != row.activity_key() || old.content_key() != row.content_key() {
                updated.push(row.session_id.clone());
            }
        }

        let reordered = previous_ids != new_ids;
        let active_changed = previous_active != active;

        self.rows = rows;
        self.active = active;
        if let Some(selected) = &self.selected {
            if !self.rows.iter().any(|row| &row.session_id == selected) {
                self.selected = None;
            }
        }

        SessionMapUpdate {
            added,
            removed,
            updated,
            active_changed,
            reordered,
        }
    }

    /// Remove every row.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.active = None;
        self.selected = None;
    }

    /// Rows in stable snapshot order.
    pub fn rows(&self) -> &[SessionRow] {
        &self.rows
    }

    /// Number of open root sessions.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the map has no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Active session identifier.
    pub fn active_session_id(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Row for a session.
    pub fn row(&self, session_id: &str) -> Option<&SessionRow> {
        self.rows.iter().find(|row| row.session_id == session_id)
    }

    /// Selected session identifier.
    pub fn selected_session_id(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Selected row.
    pub fn selected(&self) -> Option<&SessionRow> {
        self.selected.as_deref().and_then(|id| self.row(id))
    }

    /// Select a session if it exists.
    pub fn select(&mut self, session_id: &str) -> bool {
        if self.row(session_id).is_some() {
            self.selected = Some(session_id.to_owned());
            true
        } else {
            false
        }
    }

    /// Select by row index.
    pub fn select_index(&mut self, index: usize) -> bool {
        let Some(row) = self.rows.get(index) else {
            return false;
        };
        self.selected = Some(row.session_id.clone());
        true
    }

    /// Clear the selection.
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// Move the selection by `delta`, wrapping at the ends.
    pub fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            self.selected = None;
            return;
        }
        let len = self.rows.len() as isize;
        let current = self
            .selected
            .as_deref()
            .and_then(|id| self.rows.iter().position(|row| row.session_id == id));
        let next = match current {
            Some(current) => (current as isize).saturating_add(delta).rem_euclid(len),
            None if delta < 0 => len - 1,
            None => 0,
        };
        self.selected = Some(self.rows[next as usize].session_id.clone());
    }

    /// Select the next row.
    pub fn select_next(&mut self) {
        self.move_selection(1);
    }

    /// Select the previous row.
    pub fn select_previous(&mut self) {
        self.move_selection(-1);
    }
}

/// Prefer the explicit active identifier, then a single active flag.
fn resolve_active(snapshot: &SessionSnapshot) -> Option<String> {
    if let Some(active_id) = &snapshot.active_session_id {
        if snapshot
            .sessions
            .iter()
            .any(|session| &session.session_id == active_id)
        {
            return Some(active_id.clone());
        }
    }
    snapshot
        .sessions
        .iter()
        .find(|session| session.active)
        .map(|session| session.session_id.clone())
}

/// Truncate at the end with an ellipsis, counting characters.
pub fn truncate_end(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    if max_chars == 1 {
        return "…".to_owned();
    }
    let mut truncated: String = text.chars().take(max_chars - 1).collect();
    truncated.push('…');
    truncated
}

/// Truncate in the middle with an ellipsis, preserving the final path
/// components.
pub fn truncate_path(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    if max_chars == 1 {
        return "…".to_owned();
    }
    if max_chars == 2 {
        let tail: String = text.chars().skip(count - 1).collect();
        return format!("…{tail}");
    }

    // Ellipsis plus a separator is two characters; keep the remaining tail.
    let tail_len = max_chars - 2;
    let tail: String = text.chars().skip(count - tail_len).collect();
    let tail = tail.trim_start_matches(['/', '\\']);
    format!("…/{tail}")
}
