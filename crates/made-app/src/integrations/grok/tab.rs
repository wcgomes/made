//! Grok Build state of one tab: the hook and inventory channels, the sessions
//! open in the tab's terminal, and the tab's diagnostics row.

use std::any::Any;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::diagnostics::GrokDiagnosticsRow;
use crate::integrations::{
    Presentation, PresentationCtx, RowReport, TabIo, TabReport, TabState, GROK,
};
use crate::session_map::SessionActivity;
use crate::terminal_view::TerminalPane;
use crate::tui::TerminalTarget;

/// One Grok session open in this terminal, with the latest hook status.
#[derive(Debug, Clone)]
pub(crate) struct GrokLiveSession {
    pub(crate) id: String,
    pub(crate) cwd: PathBuf,
    pub(crate) opened_at: String,
    pub(crate) activity: SessionActivity,
    pub(crate) touched: Instant,
}

/// Grok Build state of one tab.
pub(crate) struct GrokTabState {
    checked_at: Instant,
    log_offset: u64,
    sessions: Vec<GrokLiveSession>,
    /// Session ids removed by a session-end hook. Inventory must not put the
    /// dot back while Grok's file still lists that row.
    closed: HashSet<String>,
    /// Hook session moved before the inventory could prove it belongs here.
    provisional_id: Option<String>,
    /// Explorer root after that provisional move. A later release restores the
    /// saved directory only while the explorer is still here.
    provisional_cwd: Option<PathBuf>,
    /// Grok directory to restore if that provisional session is not ours.
    /// `None` while a provisional move is active means the shell owned the root.
    restore_root: Option<PathBuf>,
    file_newest_id: Option<String>,
    /// Grok Build pill and sentence attached to the diagnostics snapshot.
    diagnostics: GrokDiagnosticsRow,
}

impl GrokTabState {
    pub(crate) fn new() -> Self {
        Self {
            checked_at: Instant::now(),
            log_offset: u64::MAX,
            sessions: Vec::new(),
            closed: HashSet::new(),
            provisional_id: None,
            provisional_cwd: None,
            restore_root: None,
            file_newest_id: None,
            diagnostics: GrokDiagnosticsRow::NOT_ON_PATH,
        }
    }

    /// Whether Grok is running in this tab.
    ///
    /// Running means the terminal target is Grok and that process is still
    /// alive, or the inventory has confirmed at least one open Grok session
    /// for this terminal. A hook row starts with an empty `opened_at` and does
    /// not count until that confirmation. The tab label is not part of the
    /// check: OpenCode can own the label while Grok runs.
    pub(crate) fn follow_running(&self, target: TerminalTarget, terminal_running: bool) -> bool {
        let living_target = target == TerminalTarget::Tui(GROK) && terminal_running;
        let confirmed = self
            .sessions
            .iter()
            .any(|session| !session.opened_at.is_empty());
        living_target || confirmed
    }

    /// Status dots for Grok sessions open in this terminal.
    pub(crate) fn tab_activities(&self) -> Vec<SessionActivity> {
        let mut sessions = self.sessions.clone();
        sessions.sort_by(|left, right| left.opened_at.cmp(&right.opened_at));
        sessions
            .into_iter()
            .map(|session| session.activity)
            .collect()
    }

    /// Keep the diagnostics row for the next snapshot. `remote_hooks_ready` is
    /// whether the tab's remote Grok is running with its hooks ready.
    pub(crate) fn set_diagnostics(&mut self, row: GrokDiagnosticsRow, remote_hooks_ready: bool) {
        // Consent authorizes Made's installer, not hooks a user installed.
        // Keep transport, detection, install and not-running rows unchanged.
        let hooks_ready = remote_hooks_ready
            && (row == GrokDiagnosticsRow::REMOTE_HOOKS_NOT_APPROVED
                || row == GrokDiagnosticsRow::REMOTE_INSTALL_PENDING);
        self.diagnostics = if hooks_ready {
            GrokDiagnosticsRow::READY
        } else {
            row
        };
    }

    /// The Grok Build pill and sentence currently shown in About.
    pub(crate) fn diagnostics(&self) -> GrokDiagnosticsRow {
        self.diagnostics.clone()
    }

    /// The sessions open in the tab's terminal.
    #[cfg(test)]
    pub(crate) fn sessions_mut(&mut self) -> &mut Vec<GrokLiveSession> {
        &mut self.sessions
    }

    /// Apply hook lines as soon as they are appended.
    fn poll_hooks(&mut self, now: Instant, io: &mut TabIo<'_>) {
        let (offset, events) = super::follow::read_hook_events(self.log_offset);
        self.log_offset = offset;
        for event in events {
            let moves_explorer = matches!(
                event.event.to_ascii_lowercase().as_str(),
                "session_start" | "sessionstart" | "user_prompt_submit" | "userpromptsubmit"
            );
            if event.event.eq_ignore_ascii_case("session_end")
                || event.event.eq_ignore_ascii_case("sessionend")
            {
                let ended = event.session_id.clone();
                let was_provisional = self.provisional_id.as_deref() == Some(ended.as_str());
                self.closed.insert(ended.clone());
                self.sessions.retain(|session| session.id != ended);
                if was_provisional {
                    self.release_provisional_move(io);
                } else if self.sessions.is_empty() {
                    self.file_newest_id = None;
                    io.follow.follow_directory(GROK, None);
                }
                continue;
            }
            // The log is global. Re-read inventory before moving so a session
            // this tab owns is not delayed, and a foreign pid is not followed.
            let ownership = Self::hook_session_ownership(io, &event.session_id);
            if ownership == super::follow::SessionOwnership::Foreign {
                let id = event.session_id.clone();
                let was_provisional = self.provisional_id.as_deref() == Some(id.as_str());
                self.sessions.retain(|session| session.id != id);
                if was_provisional {
                    self.release_provisional_move(io);
                }
                continue;
            }
            self.closed.remove(&event.session_id);
            let activity = match event.status.as_str() {
                "working" => SessionActivity::Busy,
                "attention" | "error" => SessionActivity::Attention,
                _ => SessionActivity::Idle,
            };
            let cwd = PathBuf::from(&event.cwd);
            if let Some(session) = self
                .sessions
                .iter_mut()
                .find(|session| session.id == event.session_id)
            {
                session.activity = activity;
                session.touched = now;
                if cwd.is_absolute() {
                    session.cwd = cwd.clone();
                }
            } else if !event.session_id.is_empty() {
                self.sessions.push(GrokLiveSession {
                    id: event.session_id.clone(),
                    cwd: cwd.clone(),
                    opened_at: String::new(),
                    activity,
                    touched: now,
                });
            }
            if moves_explorer && cwd.is_absolute() {
                if ownership == super::follow::SessionOwnership::Owned {
                    // A prompt in a session this tab owns replaces any earlier
                    // provisional move, including one for a different session.
                    self.clear_provisional_slot();
                    io.follow.follow_directory(GROK, Some(cwd.as_path()));
                } else {
                    self.begin_provisional_move(io, &event.session_id, cwd.as_path());
                }
            }
            io.follow.note_activity();
        }
    }

    fn hook_session_ownership(io: &TabIo<'_>, session_id: &str) -> super::follow::SessionOwnership {
        let Some(pid) = io.terminal.as_deref().and_then(TerminalPane::pid) else {
            return super::follow::SessionOwnership::Unknown;
        };
        super::follow::session_ownership(pid, session_id)
    }

    /// Remember the directory this tab was following, then follow `cwd` until
    /// inventory confirms the session.
    fn begin_provisional_move(&mut self, io: &mut TabIo<'_>, session_id: &str, cwd: &Path) {
        if self.provisional_id.is_none() {
            self.restore_root = io.follow.root_held_by(GROK);
        }
        self.provisional_id = Some(session_id.to_owned());
        io.follow.follow_directory(GROK, Some(cwd));
        self.provisional_cwd = io.follow.explorer_root().map(Path::to_path_buf);
    }

    fn clear_provisional_slot(&mut self) {
        self.provisional_id = None;
        self.provisional_cwd = None;
        self.restore_root = None;
    }

    fn release_provisional_move(&mut self, io: &mut TabIo<'_>) {
        if self.provisional_id.is_none() {
            return;
        }
        let restore = self.restore_root.take();
        let provisional_cwd = self.provisional_cwd.take();
        self.provisional_id = None;
        // An owned prompt after this move already left the provisional
        // directory. Restoring would undo that prompt.
        let still_there = provisional_cwd
            .as_ref()
            .is_some_and(|cwd| io.follow.explorer_root() == Some(cwd.as_path()));
        if !still_there {
            return;
        }
        match restore {
            Some(path) => io.follow.follow_directory(GROK, Some(&path)),
            None => io.follow.follow_directory(GROK, None),
        }
    }

    /// Refresh the open-session inventory and follow a newly attached session.
    fn poll_directory(&mut self, now: Instant, io: &mut TabIo<'_>) {
        if now.saturating_duration_since(self.checked_at) < Duration::from_secs(1) {
            return;
        }
        self.checked_at = now;
        let Some(pid) = io.terminal.as_deref().and_then(TerminalPane::pid) else {
            return;
        };
        let open = super::follow::open_sessions(pid);
        let open_ids: Vec<&str> = open.iter().map(|row| row.session_id.as_str()).collect();
        self.closed.retain(|id| open_ids.contains(&id.as_str()));
        for row in &open {
            if self.closed.contains(&row.session_id) {
                continue;
            }
            let cwd = PathBuf::from(&row.cwd);
            if let Some(session) = self
                .sessions
                .iter_mut()
                .find(|session| session.id == row.session_id)
            {
                session.opened_at = row.opened_at.clone();
                if cwd.is_absolute() {
                    session.cwd = cwd;
                }
            } else {
                self.sessions.push(GrokLiveSession {
                    id: row.session_id.clone(),
                    cwd,
                    opened_at: row.opened_at.clone(),
                    activity: SessionActivity::Idle,
                    touched: now,
                });
            }
        }
        let mut reverted_provisional = false;
        if let Some(id) = self.provisional_id.clone() {
            let confirmed = open_ids.contains(&id.as_str());
            if confirmed {
                self.clear_provisional_slot();
            } else {
                // Pid is missing, or the row belongs to another process and was
                // filtered out. Drop the dot and undo only that session's move.
                self.sessions.retain(|session| session.id != id);
                self.release_provisional_move(io);
                reverted_provisional = true;
            }
        }
        self.sessions.retain(|session| {
            !self.closed.contains(&session.id) && open_ids.contains(&session.id.as_str())
        });
        if reverted_provisional {
            return;
        }
        let newest = open
            .iter()
            .rev()
            .find(|row| !self.closed.contains(&row.session_id));
        match newest {
            Some(row) if self.file_newest_id.as_deref() != Some(row.session_id.as_str()) => {
                self.file_newest_id = Some(row.session_id.clone());
                let path = PathBuf::from(&row.cwd);
                if path.is_absolute() {
                    io.follow.follow_directory(GROK, Some(path.as_path()));
                }
            }
            None if open.is_empty() && self.file_newest_id.is_some() => {
                self.file_newest_id = None;
                io.follow.follow_directory(GROK, None);
            }
            _ => {}
        }
    }
}

impl TabState for GrokTabState {
    fn row(&self, _remote: bool) -> Option<RowReport> {
        Some(self.diagnostics().into())
    }

    /// Grok Build brands a local tab while its TUI runs or while the terminal
    /// has open Grok sessions. A remote tab's branding comes from its own
    /// server state.
    fn presentation(&self, ctx: &PresentationCtx<'_>) -> Option<Presentation> {
        if ctx.remote {
            return None;
        }
        let dots = self.tab_activities();
        super::workspace_grok_branded(false, ctx.target, ctx.terminal_running, dots.len())
            .then_some(Presentation { dots, title: None })
    }

    /// Apply the hook lines, then refresh the inventory. The directories they
    /// follow reach the coordinator through the poll's follow port.
    fn poll(&mut self, now: Instant, io: &mut TabIo<'_>) -> TabReport {
        self.poll_hooks(now, io);
        self.poll_directory(now, io);
        TabReport::default()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
