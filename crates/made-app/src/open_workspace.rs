//! Headless state of the Open workspace screen.
//!
//! The screen picks a host, a folder and a launch program for a new workspace
//! tab. This model owns the selected host, the folder navigation handle (the
//! picker tree for Local, a root path for an SSH host), the git and worktree
//! snapshot tagged with a generation, the effective launch path and the launch
//! buttons. It never spawns a process, reads git, or touches egui: the UI
//! renders it, git reads run elsewhere and report back with the generation of
//! the request they answer, and stale reports are discarded.

use std::path::{Path, PathBuf};

use made_remote::git::ChangeCounts;
use made_remote::protocol::{AuthorityId, RemotePath};

use crate::integrations::{IntegrationList, IntegrationRef};
use crate::tui::TerminalTarget;
use crate::workspace_picker::WorkspacePicker;

/// One host of the host bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostId {
    /// This computer.
    Local,
    /// A saved SSH server.
    Ssh(AuthorityId),
}

/// The screen's key for a server path: the same POSIX text as a `PathBuf`,
/// used only to compare and display. It is never opened, canonicalized or
/// resolved on this computer. `None` for a path that is not UTF-8.
pub fn remote_key(path: &RemotePath) -> Option<PathBuf> {
    path.to_utf8().ok().map(PathBuf::from)
}

/// One button of the host bar, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostButton {
    /// A selectable host.
    Host(HostId),
    /// The trailing `+ Add host` button.
    AddHost,
}

/// Host bar order: Local first, saved servers in record order, `+ Add host`.
pub fn host_buttons(saved: &[AuthorityId]) -> Vec<HostButton> {
    std::iter::once(HostButton::Host(HostId::Local))
        .chain(saved.iter().map(|id| HostButton::Host(HostId::Ssh(*id))))
        .chain(std::iter::once(HostButton::AddHost))
        .collect()
}

/// The host selected when the screen opens: the active tab's host, else
/// Local. A tab whose server record no longer exists falls back to Local.
pub fn default_host(active_tab: Option<HostId>, saved: &[AuthorityId]) -> HostId {
    match active_tab {
        Some(HostId::Ssh(id)) if saved.contains(&id) => HostId::Ssh(id),
        _ => HostId::Local,
    }
}

/// A saved SSH server as the screen needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshHost {
    /// Server record identity.
    pub id: AuthorityId,
    /// The record's initial remote directory, when set.
    pub initial_directory: Option<PathBuf>,
}

/// Connection state of an SSH host.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConnectionState {
    /// No connection attempt yet.
    #[default]
    NotConnected,
    /// Connecting or authenticating.
    Connecting,
    /// Connected; browsing and launching are possible.
    Connected,
    /// An unknown host key waits for the user's approval.
    HostKeyApprovalNeeded,
    /// The user cancelled authentication.
    AuthenticationCancelled,
    /// The connection failed for the given reason.
    Failed(String),
}

/// Change counts of one working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountsState {
    /// The status read has not finished.
    Pending,
    /// Whole-working-tree counts.
    Ready(ChangeCounts),
    /// The status read failed.
    Failed,
}

/// One item of the Worktrees list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeItem {
    /// Working-tree path on the selected host.
    pub path: PathBuf,
    /// Branch name or short detached commit id, when known.
    pub label: Option<String>,
    /// Whether the directory is missing or unreadable.
    pub missing: bool,
    /// The working tree's change counts.
    pub counts: CountsState,
}

/// Result of the git reads for one launch target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitState {
    /// Not a repository, git missing, or a read failed: nothing is shown.
    NoRepository,
    /// The launch target is inside a working tree.
    Repository {
        /// Top level of the working tree that contains the launch target.
        toplevel: PathBuf,
        /// Every working tree of the repository, main first, then repository
        /// order.
        worktrees: Vec<WorktreeItem>,
        /// Change counts of the working tree at `toplevel`.
        counts: CountsState,
    },
}

/// A git read the screen needs for its current host and launch target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRequest {
    /// Generation the reply must carry to be applied.
    pub generation: u64,
    /// Host to read on.
    pub host: HostId,
    /// Launch target to read for.
    pub target: PathBuf,
}

/// What a launch button starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchKind {
    /// A shell workspace.
    Terminal,
    /// A detected catalog TUI.
    Tui(IntegrationRef),
}

impl LaunchKind {
    /// Button label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::Tui(tui) => tui.descriptor().label,
        }
    }

    /// Terminal target of this button for Local. SSH launches are bound by
    /// the remote path from the request's host and kind.
    pub fn local_target(self) -> TerminalTarget {
        match self {
            Self::Terminal => TerminalTarget::Shell,
            Self::Tui(tui) => TerminalTarget::Tui(tui),
        }
    }
}

/// One launch button of the launch bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchButton {
    /// What the button starts.
    pub kind: LaunchKind,
    /// Whether the button can be activated.
    pub enabled: bool,
    /// Tooltip reason when disabled.
    pub disabled_reason: Option<&'static str>,
}

/// A launch the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    /// Host to launch on.
    pub host: HostId,
    /// What to start.
    pub kind: LaunchKind,
    /// Launch folder and explorer root of the new tab.
    pub path: PathBuf,
}

impl LaunchRequest {
    /// Terminal target of a Local launch, `None` for an SSH host.
    pub fn local_target(&self) -> Option<TerminalTarget> {
        (self.host == HostId::Local).then(|| self.kind.local_target())
    }
}

/// Folder navigation of the selected host.
#[derive(Debug, Clone)]
enum Navigation {
    /// The realized picker tree on this computer.
    Local(Box<WorkspacePicker>),
    /// The local start folder could not be read.
    LocalUnavailable,
    /// An SSH host's root, `None` until the server reports its home.
    Remote {
        root: Option<PathBuf>,
        selected: Option<PathBuf>,
    },
}

/// Navigation identity whose change resets the worktree selection.
type NavigationKey = (HostId, Option<PathBuf>, Option<PathBuf>);

/// Headless Open workspace screen state.
#[derive(Debug)]
pub struct OpenWorkspace {
    catalog: IntegrationList,
    ssh: Vec<SshHost>,
    selected: HostId,
    local_start: PathBuf,
    navigation: Navigation,
    detected: Vec<(HostId, Vec<IntegrationRef>)>,
    connections: Vec<(AuthorityId, ConnectionState)>,
    generation: u64,
    git_key: Option<NavigationKey>,
    git: Option<GitState>,
    worktree_selection: Option<usize>,
}

impl OpenWorkspace {
    /// Open the screen. `active_tab` is the active workspace tab's host, if
    /// any; `local_start` is the folder picker's start directory.
    pub fn new(
        catalog: IntegrationList,
        ssh: Vec<SshHost>,
        active_tab: Option<HostId>,
        local_start: impl Into<PathBuf>,
    ) -> Self {
        let saved: Vec<AuthorityId> = ssh.iter().map(|host| host.id).collect();
        let selected = default_host(active_tab, &saved);
        let mut screen = Self {
            catalog,
            ssh,
            selected,
            local_start: local_start.into(),
            navigation: Navigation::LocalUnavailable,
            detected: Vec::new(),
            connections: Vec::new(),
            generation: 0,
            git_key: None,
            git: None,
            worktree_selection: None,
        };
        screen.navigation = screen.start_navigation(selected);
        screen
    }

    /// Replace the saved servers after an add, edit or remove. A selected
    /// server that no longer exists falls back to Local.
    pub fn set_hosts(&mut self, ssh: Vec<SshHost>) {
        self.ssh = ssh;
        if let HostId::Ssh(id) = self.selected {
            if !self.ssh.iter().any(|server| server.id == id) {
                self.select_host(HostId::Local);
            }
        }
    }

    /// Host bar buttons in display order.
    pub fn host_buttons(&self) -> Vec<HostButton> {
        let saved: Vec<AuthorityId> = self.ssh.iter().map(|host| host.id).collect();
        host_buttons(&saved)
    }

    /// The selected host.
    pub fn selected_host(&self) -> HostId {
        self.selected
    }

    /// Select `host`, resetting navigation to its start folder and clearing
    /// the worktree selection. Unknown servers are ignored.
    pub fn select_host(&mut self, host: HostId) -> bool {
        if let HostId::Ssh(id) = host {
            if !self.ssh.iter().any(|server| server.id == id) {
                return false;
            }
        }
        self.selected = host;
        self.navigation = self.start_navigation(host);
        self.reset_git();
        true
    }

    /// The local picker tree, while Local is selected and readable.
    pub fn local_picker(&self) -> Option<&WorkspacePicker> {
        match &self.navigation {
            Navigation::Local(picker) => Some(picker),
            _ => None,
        }
    }

    /// Mutable local picker tree, while Local is selected and readable.
    ///
    /// Navigation through it changes the launch target; call [`Self::sync`]
    /// afterwards (the UI does every frame) to start the matching git read.
    pub fn local_picker_mut(&mut self) -> Option<&mut WorkspacePicker> {
        match &mut self.navigation {
            Navigation::Local(picker) => Some(picker),
            _ => None,
        }
    }

    /// The SSH host's current root, while an SSH host is selected.
    pub fn remote_root(&self) -> Option<&Path> {
        match &self.navigation {
            Navigation::Remote { root, .. } => root.as_deref(),
            _ => None,
        }
    }

    /// The selected folder row, else the current root.
    pub fn launch_target(&self) -> Option<PathBuf> {
        match &self.navigation {
            Navigation::Local(picker) => Some(picker.path_target().to_path_buf()),
            Navigation::LocalUnavailable => None,
            Navigation::Remote { root, selected } => selected.clone().or_else(|| root.clone()),
        }
    }

    /// Re-root the SSH host's tree at `root`, a path the server reported or
    /// checked, with no selection. `false` while Local is selected.
    pub fn set_remote_root(&mut self, root: PathBuf) -> bool {
        match &mut self.navigation {
            Navigation::Remote {
                root: current,
                selected,
            } => {
                *current = Some(root);
                *selected = None;
                true
            }
            _ => false,
        }
    }

    /// Apply the home directory the server reported when the browse grant
    /// opened. It becomes the root only when the record names no initial
    /// directory and no root is known yet. Without either, the root stays
    /// unknown and the returned reason is shown in the folder area.
    pub fn apply_remote_home(&mut self, home: Option<PathBuf>) -> Result<(), &'static str> {
        match &mut self.navigation {
            Navigation::Remote {
                root: root @ None,
                selected,
            } => match home {
                Some(home) => {
                    *root = Some(home);
                    *selected = None;
                    Ok(())
                }
                None => Err(
                    "This server did not report a home folder. Type an absolute path in the address bar.",
                ),
            },
            _ => Ok(()),
        }
    }

    /// Select a folder of the SSH host's tree, or clear the selection. A path
    /// outside the current root is refused.
    pub fn set_remote_selection(&mut self, path: Option<PathBuf>) -> bool {
        match &mut self.navigation {
            Navigation::Remote {
                root: Some(root),
                selected,
            } if path.as_ref().is_none_or(|path| path.starts_with(&*root)) => {
                *selected = path;
                true
            }
            _ => false,
        }
    }

    /// Record which catalog programs detection found on `host`.
    pub fn set_detected(&mut self, host: HostId, found: Vec<IntegrationRef>) {
        match self.detected.iter_mut().find(|(key, _)| *key == host) {
            Some((_, entry)) => *entry = found,
            None => self.detected.push((host, found)),
        }
    }

    /// Record an SSH host's connection state.
    pub fn set_connection(&mut self, id: AuthorityId, state: ConnectionState) {
        match self.connections.iter_mut().find(|(key, _)| *key == id) {
            Some((_, entry)) => *entry = state,
            None => self.connections.push((id, state)),
        }
    }

    /// An SSH host's connection state.
    pub fn connection(&self, id: AuthorityId) -> ConnectionState {
        self.connections
            .iter()
            .find(|(key, _)| *key == id)
            .map(|(_, state)| state.clone())
            .unwrap_or_default()
    }

    /// Start a new git read when the host, root or launch target changed
    /// since the last request. Any snapshot and worktree selection of the
    /// previous target are dropped, so a late reply cannot apply.
    pub fn sync(&mut self) -> Option<GitRequest> {
        let key = self.navigation_key();
        if self.git_key.as_ref() == Some(&key) {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        self.git = None;
        self.worktree_selection = None;
        let target = key.2.clone();
        self.git_key = Some(key);
        target.map(|target| GitRequest {
            generation: self.generation,
            host: self.selected,
            target,
        })
    }

    /// Apply the git snapshot of request `generation`. Returns `false` and
    /// changes nothing for a stale generation or a moved target. The
    /// Worktrees item equal to the launch target is preselected.
    pub fn apply_git(&mut self, generation: u64, state: GitState) -> bool {
        if !self.current(generation) {
            return false;
        }
        self.git = Some(state);
        self.worktree_selection = None;
        if self.worktrees_shown() {
            let target = self.launch_target();
            self.worktree_selection = self
                .worktrees()
                .iter()
                .position(|item| Some(&item.path) == target.as_ref() && !item.missing);
        }
        true
    }

    /// Apply the change counts of the working tree at `path` for request
    /// `generation`. Returns whether a shown working tree took them.
    pub fn apply_counts(&mut self, generation: u64, path: &Path, counts: CountsState) -> bool {
        if !self.current(generation) {
            return false;
        }
        let Some(GitState::Repository {
            toplevel,
            worktrees,
            counts: tree_counts,
        }) = &mut self.git
        else {
            return false;
        };
        let mut applied = false;
        if toplevel == path {
            *tree_counts = counts;
            applied = true;
        }
        for item in worktrees.iter_mut().filter(|item| item.path == path) {
            item.counts = counts;
            applied = true;
        }
        applied
    }

    /// Drop the git snapshot so the next [`Self::sync`] reads it again, for
    /// Refresh.
    pub fn invalidate_git(&mut self) {
        self.reset_git();
    }

    /// Whether the launch target is inside a working tree (git indicator).
    pub fn git_indicator(&self) -> bool {
        matches!(self.git_state(), Some(GitState::Repository { .. }))
    }

    /// The current git snapshot, `None` while pending or stale.
    pub fn git_state(&self) -> Option<&GitState> {
        if self.git_key.as_ref() == Some(&self.navigation_key()) {
            self.git.as_ref()
        } else {
            None
        }
    }

    /// Whether the Worktrees list is shown: the launch target is the top level
    /// of a working tree of a repository with at least one linked worktree.
    pub fn worktrees_shown(&self) -> bool {
        match (self.git_state(), self.launch_target()) {
            (
                Some(GitState::Repository {
                    toplevel,
                    worktrees,
                    ..
                }),
                Some(target),
            ) => *toplevel == target && worktrees.len() > 1,
            _ => false,
        }
    }

    /// Worktrees list items, empty when the list is not shown.
    pub fn worktrees(&self) -> &[WorktreeItem] {
        match self.git_state() {
            Some(GitState::Repository { worktrees, .. }) if self.worktrees_shown() => worktrees,
            _ => &[],
        }
    }

    /// Selected Worktrees item index.
    pub fn worktree_selection(&self) -> Option<usize> {
        if self.worktrees_shown() {
            self.worktree_selection
        } else {
            None
        }
    }

    /// Select a Worktrees item. Missing worktrees are not selectable.
    pub fn select_worktree(&mut self, index: usize) -> bool {
        match self.worktrees().get(index) {
            Some(item) if !item.missing => {
                self.worktree_selection = Some(index);
                true
            }
            _ => false,
        }
    }

    /// The selected worktree when the list is shown, else the launch target.
    pub fn effective_launch_path(&self) -> Option<PathBuf> {
        self.worktree_selection()
            .and_then(|index| self.worktrees().get(index))
            .map(|item| item.path.clone())
            .or_else(|| self.launch_target())
    }

    /// Launch buttons for the selected host: Terminal first, then each
    /// catalog TUI detected on that host in catalog order.
    pub fn launch_buttons(&self) -> Vec<LaunchButton> {
        let disabled_reason = if self.effective_launch_path().is_none() {
            Some("No folder to launch in")
        } else {
            match self.selected {
                HostId::Local => None,
                HostId::Ssh(id) => (self.connection(id) != ConnectionState::Connected)
                    .then_some("Server is not connected"),
            }
        };
        let detected: &[IntegrationRef] = self
            .detected
            .iter()
            .find(|(host, _)| *host == self.selected)
            .map_or(&[], |(_, found)| found.as_slice());
        std::iter::once(LaunchKind::Terminal)
            .chain(
                self.catalog
                    .iter()
                    .copied()
                    .filter(|tui| detected.contains(tui))
                    .map(LaunchKind::Tui),
            )
            .map(|kind| LaunchButton {
                kind,
                enabled: disabled_reason.is_none(),
                disabled_reason,
            })
            .collect()
    }

    /// The launch an enabled button starts, `None` when the button is not
    /// offered or disabled.
    pub fn launch_request(&self, kind: LaunchKind) -> Option<LaunchRequest> {
        self.launch_buttons()
            .into_iter()
            .find(|button| button.kind == kind && button.enabled)?;
        Some(LaunchRequest {
            host: self.selected,
            kind,
            path: self.effective_launch_path()?,
        })
    }

    fn start_navigation(&self, host: HostId) -> Navigation {
        match host {
            HostId::Local => WorkspacePicker::new(&self.local_start)
                .map_or(Navigation::LocalUnavailable, |picker| {
                    Navigation::Local(Box::new(picker))
                }),
            HostId::Ssh(id) => Navigation::Remote {
                selected: None,
                root: self
                    .ssh
                    .iter()
                    .find(|server| server.id == id)
                    .and_then(|server| server.initial_directory.clone()),
            },
        }
    }

    fn navigation_key(&self) -> NavigationKey {
        let root = match &self.navigation {
            Navigation::Local(picker) => Some(picker.current_dir().to_path_buf()),
            Navigation::LocalUnavailable => None,
            Navigation::Remote { root, .. } => root.clone(),
        };
        (self.selected, root, self.launch_target())
    }

    fn current(&self, generation: u64) -> bool {
        generation == self.generation && self.git_key.as_ref() == Some(&self.navigation_key())
    }

    fn reset_git(&mut self) {
        self.git_key = None;
        self.git = None;
        self.worktree_selection = None;
    }
}
