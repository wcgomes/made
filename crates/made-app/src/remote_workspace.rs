//! Transport-independent, authority-bound request tracking for a remote tab.
//!
//! This is a model seam, not an SSH client. The caller sends returned envelopes
//! through the authenticated transport and supplies replies to `accept`.
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Instant,
};

use made_remote::protocol::{
    DirectoryEntry, ErrorCode, GitRead, Identity, Operation, PreviewKind, ProtocolError,
    RemotePath, ReplyProgress, WireEnvelope, CLOSE_TIMEOUT, MAX_IN_FLIGHT, MAX_QUEUED_CHUNKS,
    MAX_TREE_DEPTH, MAX_TREE_ROWS, RPC_TIMEOUT,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootSource {
    Launch,
    Shell,
    View,
    Session,
    Selection,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplorerMode {
    Files,
    Git,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseState {
    Open,
    Waiting,
    Confirmed,
    Unconfirmed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Stale,
    Loading,
    Current,
    Unavailable(ErrorCode),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryProjection {
    pub path: RemotePath,
    pub entries: Vec<DirectoryEntry>,
    pub truncated: bool,
    pub freshness: Freshness,
}
#[derive(Clone, PartialEq, Eq)]
pub struct PreviewProjection {
    pub path: RemotePath,
    pub kind: Option<PreviewKind>,
    pub data: Vec<u8>,
    pub freshness: Freshness,
}
#[derive(Clone, PartialEq, Eq)]
pub struct GitProjection {
    pub command: GitRead,
    pub data: Vec<u8>,
    pub freshness: Freshness,
}
/// UI-facing Git rows retain POSIX bytes even on Windows clients. The shared
/// native Git parser is intentionally not used to interpret remote paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteGitRow {
    pub path: RemotePath,
    pub original: Option<RemotePath>,
    pub letter: char,
    pub kind: made_remote::git::ChangeKind,
    pub directory: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteGitGroup {
    pub title: &'static str,
    pub rows: Vec<RemoteGitRow>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteGitView {
    pub branch: Option<String>,
    pub detached: bool,
    pub commit: Option<String>,
    pub subject: Option<String>,
    pub linked_worktree: bool,
    pub worktree_name: Option<String>,
    pub groups: Vec<RemoteGitGroup>,
    pub freshness: Freshness,
    pub folder_marks: HashMap<RemotePath, Vec<made_remote::git::ChangeKind>>,
    /// A complete status read has succeeded at least once for this root.
    pub has_status: bool,
}
/// UI reads this projection rather than treating previous-epoch data as live.
#[derive(Debug, Clone)]
pub struct WorkspaceProjection {
    pub source: RootSource,
    pub requested_root: Option<RemotePath>,
    pub root_state: Freshness,
    pub directories: HashMap<RemotePath, DirectoryProjection>,
    pub expanded: HashSet<RemotePath>,
    pub selected: Option<RemotePath>,
    /// Source-control row selection is independent of the file-tree selection.
    pub git_selected: Option<RemotePath>,
    /// A repository toplevel was observed, before the full Git read finishes.
    pub git_root_known: bool,
    pub preview: Option<PreviewProjection>,
    /// Bumped on every preview state change so hidden refreshes invalidate UI caches.
    pub preview_revision: u64,
    /// Bumped on projection changes so the UI can cache flattened tree rows.
    pub revision: u64,
    pub git: Vec<GitProjection>,
    pub git_view: Option<RemoteGitView>,
    pub sessions: Option<made_bridge::state::SessionSnapshot>,
    pub session_state: Freshness,
    pub integration_error: Option<ErrorCode>,
    pub focus: Option<(String, bool)>,
    pub mode: ExplorerMode,
    pub cleanup_unconfirmed: bool,
}
#[derive(Clone, Copy)]
enum Purpose {
    User,
    Follow(RootSource),
    Shell,
    Resnapshot,
}

/// Connection health is independent of optional OpenCode integration health.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationState {
    Unknown,
    Connected,
    Unavailable,
}

/// A complete result; partial streams are never published as current state.
#[derive(Clone, PartialEq, Eq)]
pub enum RemoteResult {
    Root(RemotePath),
    Listing {
        path: RemotePath,
        entries: Vec<DirectoryEntry>,
        truncated: bool,
    },
    Preview {
        path: RemotePath,
        kind: PreviewKind,
        data: Vec<u8>,
    },
    Git {
        command: GitRead,
        data: Vec<u8>,
    },
    Session(made_bridge::state::SessionSnapshot),
    Focus {
        session_id: String,
        accepted: bool,
    },
    Changed(RemotePath),
    Error(ErrorCode),
    Closed {
        confirmed: bool,
    },
}
impl std::fmt::Debug for PreviewProjection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewProjection")
            .field("path", &self.path)
            .field("kind", &self.kind)
            .field("freshness", &self.freshness)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for GitProjection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitProjection")
            .field("command", &self.command)
            .field("freshness", &self.freshness)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for RemoteResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RemoteResult(<redacted>)")
    }
}

struct Pending {
    request: WireEnvelope,
    bytes: Vec<u8>,
    entries: Vec<DirectoryEntry>,
    progress: ReplyProgress,
    purpose: Purpose,
    started: Instant,
}

/// Owns only one tab's pending work and never interprets paths using host I/O.
pub struct RemoteWorkspace {
    identity: Identity,
    root: RemotePath,
    connection: ConnectionState,
    integration: IntegrationState,
    next_request: u64,
    pending: HashMap<u64, Pending>,
    outbox: VecDeque<WireEnvelope>,
    projection: WorkspaceProjection,
    launch: RemotePath,
    selection: Option<RemotePath>,
    shell: Option<RemotePath>,
    shell_report: Option<RemotePath>,
    follow_request: Option<u64>,
    session_revision: u64,
    resnapshot: bool,
    close_state: CloseState,
    watch: Option<(Identity, RemotePath)>,
    integration_epoch: u64,
}

impl RemoteWorkspace {
    pub fn new(identity: Identity, root: RemotePath) -> Result<Self, ProtocolError> {
        identity.validate()?;
        Ok(Self {
            identity,
            launch: root.clone(),
            root,
            connection: ConnectionState::Connected,
            integration: IntegrationState::Unknown,
            next_request: 1,
            pending: HashMap::new(),
            outbox: VecDeque::new(),
            projection: WorkspaceProjection {
                source: RootSource::Launch,
                requested_root: None,
                root_state: Freshness::Stale,
                directories: HashMap::new(),
                expanded: HashSet::new(),
                selected: None,
                git_selected: None,
                git_root_known: false,
                preview: None,
                preview_revision: 0,
                revision: 0,
                git: Vec::new(),
                git_view: None,
                sessions: None,
                session_state: Freshness::Stale,
                integration_error: None,
                focus: None,
                mode: ExplorerMode::Files,
                cleanup_unconfirmed: false,
            },
            shell: None,
            selection: None,
            shell_report: None,
            follow_request: None,
            session_revision: 0,
            resnapshot: false,
            close_state: CloseState::Open,
            watch: None,
            integration_epoch: 0,
        })
    }
    pub fn identity(&self) -> &Identity {
        &self.identity
    }
    pub fn root(&self) -> &RemotePath {
        &self.root
    }
    pub fn launch_directory(&self) -> &RemotePath {
        &self.launch
    }
    pub fn shell_cwd(&self) -> Option<&RemotePath> {
        self.shell.as_ref()
    }
    pub fn connection(&self) -> ConnectionState {
        self.connection
    }
    pub fn integration(&self) -> IntegrationState {
        self.integration
    }
    fn set_integration(&mut self, state: IntegrationState) {
        self.integration = state;
        if state != IntegrationState::Connected {
            self.projection.sessions = None;
            self.projection.session_state = Freshness::Unavailable(ErrorCode::Unavailable);
            self.projection.focus = None;
            self.session_revision = 0;
        }
    }
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Enqueue only model operations. An outside-root root selection or
    /// validation requires an exact grant supplied by the owning UI action.
    pub fn request(
        &mut self,
        operation: Operation,
        grant: Option<&RemotePath>,
    ) -> Result<WireEnvelope, ProtocolError> {
        if self.connection != ConnectionState::Connected {
            return Err(ProtocolError::Identity);
        }
        if self.close_state != CloseState::Open && !matches!(operation, Operation::Close) {
            return Err(ProtocolError::Identity);
        }
        if let Operation::Focus { session_id } = &operation {
            if self.integration != IntegrationState::Connected
                || !self.projection.sessions.as_ref().is_some_and(|s| {
                    s.tabs_enabled
                        && s.sessions
                            .iter()
                            .any(|session| &session.session_id == session_id)
                })
            {
                return Err(ProtocolError::Identity);
            }
        }
        if self.pending.len() >= MAX_IN_FLIGHT {
            return Err(ProtocolError::Limit);
        }
        if !matches!(
            operation,
            Operation::RootValidate { .. }
                | Operation::RootSelect { .. }
                | Operation::List { .. }
                | Operation::Preview { .. }
                | Operation::Refresh { .. }
                | Operation::Watch { .. }
                | Operation::Rescan { .. }
                | Operation::GitRead { .. }
                | Operation::SessionState
                | Operation::Focus { .. }
                | Operation::Close
        ) {
            return Err(ProtocolError::UnexpectedResponse);
        }
        let id = self.next_request;
        self.next_request = id.checked_add(1).ok_or(ProtocolError::Limit)?;
        let request = WireEnvelope::new(self.identity.clone(), id, operation);
        request.validate_request(&self.root, grant)?;
        match &request.operation {
            Operation::List { path }
            | Operation::Preview { path }
            | Operation::Refresh { path }
            | Operation::Watch { path }
            | Operation::Rescan { path } => self.check_tree_path(path)?,
            _ => {}
        }
        if matches!(
            request.operation,
            Operation::List { .. }
                | Operation::Preview { .. }
                | Operation::Refresh { .. }
                | Operation::GitRead { .. }
        ) && matches!(self.projection.root_state, Freshness::Unavailable(_))
        {
            return Err(ProtocolError::Identity);
        }
        let progress = ReplyProgress::new(request.clone())?;
        self.pending.insert(
            id,
            Pending {
                request: request.clone(),
                bytes: Vec::new(),
                entries: Vec::new(),
                progress,
                purpose: Purpose::User,
                started: Instant::now(),
            },
        );
        Ok(request)
    }

    pub fn cancel(&mut self, request_id: u64) -> bool {
        self.outbox
            .retain(|request| request.request_id != request_id);
        self.pending.remove(&request_id).is_some()
    }

    /// Feed a root-port resolution back into the workspace projection. The
    /// client keeps root and session replies on dedicated ports; the desktop
    /// pump is responsible for draining them in the same order as the request
    /// ids it maps.
    pub fn accept_root(
        &mut self,
        resolution: made_remote::protocol::RootResolution,
    ) -> Result<Option<RemoteResult>, ProtocolError> {
        let requested = match self
            .pending
            .get(&resolution.request_id)
            .map(|pending| &pending.request.operation)
        {
            Some(Operation::RootSelect { path }) | Some(Operation::RootValidate { path }) => {
                path.clone()
            }
            _ => return Err(ProtocolError::Identity),
        };
        let operation = match resolution.result {
            Ok(path) => Operation::RootResult {
                requested,
                path,
                generation: resolution.generation,
            },
            Err(code) => Operation::Error { code },
        };
        // Root resolutions are delivered on a dedicated port and may advance
        // the generation. The generic reply guard compares the envelope
        // identity to the workspace's current generation, so re-stamp the
        // reply with the current identity while keeping the server's new
        // generation in the operation itself.
        if resolution.identity.authority != self.identity.authority
            || !resolution.identity.tab.authenticates(&self.identity.tab)
            || resolution.identity.epoch != self.identity.epoch
            || resolution.generation < self.identity.generation
        {
            return Err(ProtocolError::Identity);
        }
        self.accept(WireEnvelope::new(
            self.identity.clone(),
            resolution.request_id,
            operation,
        ))
    }

    /// Reject stale, unsolicited, mismatched or malformed replies before applying.
    pub fn accept(&mut self, reply: WireEnvelope) -> Result<Option<RemoteResult>, ProtocolError> {
        if self.connection != ConnectionState::Connected || !reply.identity.matches(&self.identity)
        {
            return Err(ProtocolError::Identity);
        }
        let pending = self
            .pending
            .get_mut(&reply.request_id)
            .ok_or(ProtocolError::Identity)?;
        if let Err(error) = pending.progress.accept(&reply) {
            // Poisoned streams are discarded, including any accumulated content.
            self.pending.remove(&reply.request_id);
            return Err(error);
        }
        let purpose = pending.purpose;
        let original = pending.request.operation.clone();
        let result = match reply.operation {
            Operation::RootResult {
                requested,
                path,
                generation,
            } => {
                // A server cannot use an authorized transition to redirect the client.
                if path != requested {
                    return Err(ProtocolError::Path);
                }
                if matches!(pending.request.operation, Operation::RootSelect { .. }) {
                    self.root = path.clone();
                    self.identity.generation = generation;
                }
                self.projection.requested_root = None;
                Some(RemoteResult::Root(path))
            }
            Operation::Listed {
                path,
                entries,
                truncated,
                done,
            } => {
                if entries
                    .iter()
                    .any(|entry| remote_depth(&entry.path, &self.root) > MAX_TREE_DEPTH)
                {
                    return Err(ProtocolError::Limit);
                }
                pending.entries.extend(entries);
                done.then(|| RemoteResult::Listing {
                    path,
                    entries: std::mem::take(&mut pending.entries),
                    truncated,
                })
            }
            Operation::PreviewResult {
                path,
                kind,
                offset: _,
                data,
                done,
            } => {
                pending.bytes.extend_from_slice(data.as_bytes());
                done.then(|| RemoteResult::Preview {
                    path,
                    kind,
                    data: std::mem::take(&mut pending.bytes),
                })
            }
            Operation::GitResult {
                command,
                offset: _,
                data,
                done,
            } => {
                pending.bytes.extend_from_slice(data.as_bytes());
                done.then(|| RemoteResult::Git {
                    command,
                    data: std::mem::take(&mut pending.bytes),
                })
            }
            Operation::SessionSnapshot { snapshot } => Some(RemoteResult::Session(snapshot)),
            Operation::FocusResult {
                session_id,
                accepted,
            } => Some(RemoteResult::Focus {
                session_id,
                accepted,
            }),
            Operation::Changed { path } => Some(RemoteResult::Changed(path)),
            Operation::Error { code } => Some(RemoteResult::Error(code)),
            Operation::Closed { confirmed } => Some(RemoteResult::Closed { confirmed }),
            _ => return Err(ProtocolError::UnexpectedResponse),
        };
        if result.is_some() {
            self.pending.remove(&reply.request_id);
            // A root change invalidates every old-generation request.
            self.pending
                .retain(|_, p| p.request.identity.matches(&self.identity));
            self.outbox
                .retain(|request| request.identity.matches(&self.identity));
        }
        if let Some(result) = &result {
            self.apply(result, &original, purpose)?;
        }
        Ok(result)
    }

    /// Discard all pending requests; no input, focus or upload is replayed.
    pub fn disconnect(&mut self) {
        self.connection = ConnectionState::Disconnected;
        self.integration = IntegrationState::Unknown;
        self.pending.clear();
        self.outbox.clear();
        self.follow_request = None;
        self.projection.root_state = Freshness::Stale;
        for directory in self.projection.directories.values_mut() {
            directory.freshness = Freshness::Stale;
        }
        for git in &mut self.projection.git {
            git.freshness = Freshness::Stale;
        }
        if let Some(view) = &mut self.projection.git_view {
            view.freshness = Freshness::Stale;
        }
        if let Some(preview) = &mut self.projection.preview {
            preview.freshness = Freshness::Stale;
            self.projection.preview_revision = self.projection.preview_revision.wrapping_add(1);
        }
        self.projection.requested_root = None;
        self.projection.sessions = None;
        self.projection.session_state = Freshness::Stale;
        self.projection.focus = None;
        self.session_revision = 0;
        self.shell = None;
        self.shell_report = None;
        self.watch = None;
        self.integration_epoch = 0;
    }

    /// Caller must authenticate and revalidate the authority before invoking.
    /// Previously returned results remain stale until the caller resnapshots.
    /// Mark the initial Open attached without bumping the epoch or enqueueing
    /// RootValidate. The caller must already have received Attached.
    pub fn mark_attached(&mut self) -> Result<(), ProtocolError> {
        if self.close_state != CloseState::Open {
            return Err(ProtocolError::Identity);
        }
        self.connection = ConnectionState::Connected;
        self.projection.root_state = Freshness::Stale;
        Ok(())
    }

    pub fn reattach(&mut self, epoch: u64) -> Result<(), ProtocolError> {
        if self.close_state != CloseState::Open
            || self.connection != ConnectionState::Disconnected
            || epoch <= self.identity.epoch
        {
            return Err(ProtocolError::Identity);
        }
        self.identity.epoch = epoch;
        self.connection = ConnectionState::Connected;
        self.integration = IntegrationState::Unknown;
        self.begin_resnapshot()
    }

    pub fn projection(&self) -> &WorkspaceProjection {
        &self.projection
    }
    pub fn set_mode(&mut self, mode: ExplorerMode) {
        self.projection.mode = mode;
    }
    /// One envelope at a time; callers must never replay a drained envelope.
    pub fn next_command(&mut self) -> Option<WireEnvelope> {
        self.outbox.pop_front()
    }

    fn enqueue(
        &mut self,
        operation: Operation,
        grant: Option<&RemotePath>,
        purpose: Purpose,
    ) -> Result<u64, ProtocolError> {
        if self.outbox.len() >= MAX_QUEUED_CHUNKS {
            return Err(ProtocolError::Limit);
        }
        let request = self.request(operation, grant)?;
        let id = request.request_id;
        if let Some(pending) = self.pending.get_mut(&id) {
            pending.purpose = purpose;
        }
        self.outbox.push_back(request);
        Ok(id)
    }

    /// Explicit cancellation releases local state immediately. Sending Cancel
    /// is not confirmation that a remote subscription has terminated.
    pub fn cancel_request(&mut self, request_id: u64) -> Result<bool, ProtocolError> {
        let Some(pending) = self.pending.remove(&request_id) else {
            return Ok(false);
        };
        self.outbox.retain(|r| r.request_id != request_id);
        if self.follow_request == Some(request_id) {
            self.follow_request = None;
        }
        match &pending.request.operation {
            Operation::List { path } => {
                if let Some(directory) = self.projection.directories.get_mut(path) {
                    directory.freshness = Freshness::Stale;
                }
            }
            Operation::Preview { .. } | Operation::Refresh { .. } => {
                if let Some(preview) = &mut self.projection.preview {
                    preview.freshness = Freshness::Stale;
                    self.projection.preview_revision =
                        self.projection.preview_revision.wrapping_add(1);
                }
            }
            Operation::GitRead { command } => {
                if let Some(git) = self
                    .projection
                    .git
                    .iter_mut()
                    .find(|g| &g.command == command)
                {
                    git.freshness = Freshness::Stale;
                }
            }
            Operation::Focus { .. } => {
                self.projection.focus = None;
            }
            _ => {}
        }
        if self.connection == ConnectionState::Connected {
            if self.outbox.len() >= MAX_QUEUED_CHUNKS {
                return Err(ProtocolError::Limit);
            }
            let id = self.next_request;
            self.next_request = id.checked_add(1).ok_or(ProtocolError::Limit)?;
            self.outbox.push_back(WireEnvelope::new(
                pending.request.identity,
                id,
                Operation::Cancel { request_id },
            ));
        }
        Ok(true)
    }

    fn cancel_where(&mut self, predicate: impl Fn(&Pending) -> bool) -> Result<(), ProtocolError> {
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| predicate(p))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.cancel_request(id)?;
        }
        Ok(())
    }

    /// Drop every old-root request without emitting one Cancel per request.
    /// A transition owns the next generation; the client is reset separately
    /// before the RootSelect is sent, so old-root Cance frames would only fill
    /// its bounded in-flight queue and starve the transition itself.
    fn abandon_all_pending(&mut self) {
        self.pending.clear();
        self.outbox.clear();
        self.follow_request = None;
    }

    /// Ten-second RPC deadlines are driven by the UI's nonblocking timer.
    pub fn expire(&mut self, now: Instant) -> Result<Vec<u64>, ProtocolError> {
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| {
                now.saturating_duration_since(p.started)
                    >= if matches!(p.request.operation, Operation::Close) {
                        CLOSE_TIMEOUT
                    } else {
                        RPC_TIMEOUT
                    }
            })
            .map(|(id, _)| *id)
            .collect();
        for id in &ids {
            if let Some(pending) = self.pending.get(id) {
                let operation = pending.request.operation.clone();
                let purpose = pending.purpose;
                self.cancel_request(*id)?;
                self.apply(
                    &RemoteResult::Error(ErrorCode::Timeout),
                    &operation,
                    purpose,
                )?;
            }
        }
        Ok(ids)
    }

    pub fn load_directory(&mut self, path: RemotePath) -> Result<u64, ProtocolError> {
        self.check_tree_path(&path)?;
        if !self.projection.directories.contains_key(&path)
            && self.projection.directories.len() >= MAX_TREE_ROWS
        {
            return Err(ProtocolError::Limit);
        }
        self.cancel_where(
            |p| matches!(&p.request.operation, Operation::List { path: old } if old == &path),
        )?;
        let id = self.enqueue(Operation::List { path: path.clone() }, None, Purpose::User)?;
        let projection = self
            .projection
            .directories
            .entry(path.clone())
            .or_insert_with(|| DirectoryProjection {
                path,
                entries: Vec::new(),
                truncated: false,
                freshness: Freshness::Stale,
            });
        projection.freshness = Freshness::Loading;
        Ok(id)
    }
    pub fn expand(&mut self, path: RemotePath) -> Result<u64, ProtocolError> {
        let id = self.load_directory(path.clone())?;
        self.projection.expanded.insert(path);
        self.projection.revision = self.projection.revision.wrapping_add(1);
        Ok(id)
    }
    pub fn collapse(&mut self, path: &RemotePath) {
        self.projection.expanded.remove(path);
        self.projection.revision = self.projection.revision.wrapping_add(1);
    }
    pub fn select(&mut self, path: RemotePath) -> Result<(), ProtocolError> {
        self.check_tree_path(&path)?;
        self.projection.selected = Some(path);
        self.projection.revision = self.projection.revision.wrapping_add(1);
        Ok(())
    }
    /// Select a source-control row without changing the file-tree selection.
    pub fn select_git(&mut self, path: RemotePath) -> Result<(), ProtocolError> {
        self.check_tree_path(&path)?;
        self.projection.git_selected = Some(path);
        Ok(())
    }
    pub fn open_preview(&mut self, path: RemotePath) -> Result<u64, ProtocolError> {
        self.check_tree_path(&path)?;
        self.cancel_where(|p| {
            matches!(
                p.request.operation,
                Operation::Preview { .. } | Operation::Refresh { .. }
            )
        })?;
        let id = self.enqueue(
            Operation::Preview { path: path.clone() },
            None,
            Purpose::User,
        )?;
        self.projection.preview = Some(PreviewProjection {
            path,
            kind: None,
            data: Vec::new(),
            freshness: Freshness::Loading,
        });
        self.projection.preview_revision = self.projection.preview_revision.wrapping_add(1);
        Ok(id)
    }
    pub fn refresh_preview(&mut self) -> Result<Option<u64>, ProtocolError> {
        let Some(path) = self.projection.preview.as_ref().map(|p| p.path.clone()) else {
            return Ok(None);
        };
        self.cancel_where(|p| {
            matches!(
                p.request.operation,
                Operation::Preview { .. } | Operation::Refresh { .. }
            )
        })?;
        let id = self.enqueue(Operation::Refresh { path }, None, Purpose::User)?;
        if let Some(preview) = &mut self.projection.preview {
            preview.freshness = Freshness::Loading;
            self.projection.preview_revision = self.projection.preview_revision.wrapping_add(1);
        }
        Ok(Some(id))
    }
    pub fn close_preview(&mut self) -> Result<(), ProtocolError> {
        self.cancel_where(|p| {
            matches!(
                p.request.operation,
                Operation::Preview { .. } | Operation::Refresh { .. }
            )
        })?;
        if self.projection.preview.take().is_some() {
            self.projection.preview_revision = self.projection.preview_revision.wrapping_add(1);
        }
        Ok(())
    }
    fn check_tree_path(&self, path: &RemotePath) -> Result<(), ProtocolError> {
        if !path.is_within(&self.root) {
            return Err(ProtocolError::Path);
        }
        let depth = remote_depth(path, &self.root);
        if depth > MAX_TREE_DEPTH {
            return Err(ProtocolError::Limit);
        }
        Ok(())
    }

    /// Refresh Git on mode entry, unless the same refresh is already in flight.
    pub fn refresh_git_if_idle(&mut self) -> Result<(), ProtocolError> {
        let refreshing = self
            .projection
            .git
            .iter()
            .any(|git| git.freshness == Freshness::Loading)
            || self
                .projection
                .git_view
                .as_ref()
                .is_some_and(|git| git.freshness == Freshness::Loading);
        if refreshing {
            return Ok(());
        }
        self.refresh_git()
    }

    pub fn refresh_git(&mut self) -> Result<(), ProtocolError> {
        self.cancel_where(|p| matches!(p.request.operation, Operation::GitRead { .. }))?;
        const COMMANDS: [GitRead; 8] = [
            GitRead::ShowToplevel,
            GitRead::Branch,
            GitRead::GitDir,
            GitRead::CommonDir,
            GitRead::WorktreeList,
            GitRead::CommitId,
            GitRead::CommitSubject,
            GitRead::Status,
        ];
        if self.pending.len() + COMMANDS.len() > MAX_IN_FLIGHT
            || self.outbox.len() + COMMANDS.len() > MAX_QUEUED_CHUNKS
        {
            return Err(ProtocolError::Limit);
        }
        let previous_view = self.projection.git_view.take();
        self.projection.git.clear();
        for command in COMMANDS {
            self.enqueue(Operation::GitRead { command }, None, Purpose::User)?;
            self.projection.git.push(GitProjection {
                command,
                data: Vec::new(),
                freshness: Freshness::Loading,
            });
        }
        // Keep the last complete view visible while a refresh is in flight,
        // matching the local Explorer's snapshot-while-loading behavior.
        if let Some(mut view) = previous_view {
            view.freshness = Freshness::Loading;
            self.projection.git_view = Some(view);
        }
        Ok(())
    }
    pub fn refresh_sessions(&mut self) -> Result<u64, ProtocolError> {
        self.cancel_where(|p| matches!(p.request.operation, Operation::SessionState))?;
        let id = self.enqueue(Operation::SessionState, None, Purpose::User)?;
        self.projection.session_state = Freshness::Loading;
        Ok(id)
    }
    pub fn focus_session(&mut self, session_id: &str) -> Result<u64, ProtocolError> {
        if self.integration != IntegrationState::Connected
            || !self.projection.sessions.as_ref().is_some_and(|s| {
                s.tabs_enabled
                    && s.sessions
                        .iter()
                        .any(|session| session.session_id == session_id)
            })
        {
            return Err(ProtocolError::Identity);
        }
        self.cancel_where(|p| matches!(p.request.operation, Operation::Focus { .. }))?;
        self.projection.focus = None;
        self.enqueue(
            Operation::Focus {
                session_id: session_id.to_owned(),
            },
            None,
            Purpose::User,
        )
    }

    /// Authenticated cwd reports use a dedicated event boundary, not unsolicited
    /// RPC replies. Lexical checks never consult host fs; server validates next.
    pub fn report_shell_cwd(
        &mut self,
        identity: &Identity,
        path: RemotePath,
    ) -> Result<u64, ProtocolError> {
        if self.connection != ConnectionState::Connected || !identity.matches(&self.identity) {
            return Err(ProtocolError::Identity);
        }
        self.cancel_where(|p| matches!(p.purpose, Purpose::Shell))?;
        let id = self.enqueue(
            Operation::RootValidate { path: path.clone() },
            Some(&path),
            Purpose::Shell,
        )?;
        self.shell_report = Some(path);
        Ok(id)
    }

    pub fn integration_changed(
        &mut self,
        identity: &Identity,
        state: IntegrationState,
    ) -> Result<(), ProtocolError> {
        if self.connection != ConnectionState::Connected || !identity.matches(&self.identity) {
            return Err(ProtocolError::Identity);
        }
        self.set_integration(state);
        if state != IntegrationState::Connected {
            self.cancel_where(|p| {
                matches!(
                    p.request.operation,
                    Operation::SessionState | Operation::Focus { .. }
                )
            })?;
            self.follow()?;
        }
        Ok(())
    }

    /// Optional plugin reconnect has its own monotonic connection epoch; a low
    /// snapshot revision after a new plugin hello must not reuse old view data.
    pub fn integration_reconnected(
        &mut self,
        identity: &Identity,
        epoch: u64,
    ) -> Result<(), ProtocolError> {
        if !identity.matches(&self.identity)
            || self.connection != ConnectionState::Connected
            || epoch <= self.integration_epoch
        {
            return Err(ProtocolError::Identity);
        }
        self.integration_epoch = epoch;
        self.session_revision = 0;
        self.projection.sessions = None;
        self.projection.focus = None;
        self.projection.integration_error = None;
        self.integration = IntegrationState::Connected;
        self.cancel_where(|p| {
            matches!(
                p.request.operation,
                Operation::SessionState | Operation::Focus { .. }
            )
        })?;
        self.follow()?;
        // Root transitions trigger a full state request after validation.
        if self.follow_request.is_none() {
            self.refresh_sessions()?;
        }
        Ok(())
    }

    /// Explicit remote picker grant. Only the exact chosen POSIX path is granted.
    pub fn select_root(&mut self, path: RemotePath) -> Result<u64, ProtocolError> {
        self.transition(path, RootSource::Selection)
    }
    fn candidate(&self) -> Result<(RemotePath, RootSource), ProtocolError> {
        if self.integration == IntegrationState::Connected {
            if let Some(snapshot) = &self.projection.sessions {
                if snapshot.tabs_enabled {
                    if let Some(active) = &snapshot.active_session_id {
                        let directory = snapshot
                            .sessions
                            .iter()
                            .find(|s| &s.session_id == active)
                            .and_then(|s| s.directory.as_ref())
                            .and_then(|p| p.to_str())
                            .ok_or(ProtocolError::Path)?;
                        return Ok((
                            RemotePath::new(directory.as_bytes().to_vec())?,
                            RootSource::Session,
                        ));
                    }
                    if let Some(directory) =
                        snapshot.view_directory.as_ref().and_then(|p| p.to_str())
                    {
                        return Ok((
                            RemotePath::new(directory.as_bytes().to_vec())?,
                            RootSource::View,
                        ));
                    }
                }
            }
        }
        Ok((
            self.shell
                .clone()
                .or_else(|| self.selection.clone())
                .unwrap_or_else(|| self.launch.clone()),
            if self.shell.is_some() {
                RootSource::Shell
            } else {
                if self.selection.is_some() {
                    RootSource::Selection
                } else {
                    RootSource::Launch
                }
            },
        ))
    }
    pub fn follow(&mut self) -> Result<(), ProtocolError> {
        match self.candidate() {
            Ok((path, source)) => {
                if path != self.root
                    || self.projection.root_state != Freshness::Current
                    || self.follow_request.is_some()
                {
                    self.transition(path, source)?;
                } else {
                    self.projection.source = source;
                }
            }
            Err(_) => {
                self.abandon_all_pending();
                self.clear_root_projection();
                self.projection.root_state = Freshness::Unavailable(ErrorCode::Unavailable);
                self.projection.source = RootSource::Session;
            }
        }
        Ok(())
    }
    fn transition(&mut self, path: RemotePath, source: RootSource) -> Result<u64, ProtocolError> {
        // A single root transition owns the next generation. Drop old work
        // silently: the client is reset before RootSelect, and per-request
        // Cance frames would exhaust its bounded in-flight queue.
        self.abandon_all_pending();
        self.clear_root_projection();
        self.projection.source = source;
        self.projection.requested_root = Some(path.clone());
        self.projection.root_state = Freshness::Loading;
        let id = self.enqueue(
            Operation::RootSelect { path: path.clone() },
            Some(&path),
            Purpose::Follow(source),
        )?;
        self.follow_request = Some(id);
        Ok(id)
    }
    fn clear_root_projection(&mut self) {
        self.watch = None;
        self.projection.directories.clear();
        self.projection.expanded.clear();
        self.projection.selected = None;
        self.projection.git_selected = None;
        self.projection.git_root_known = false;
        self.projection.preview = None;
        self.projection.preview_revision = self.projection.preview_revision.wrapping_add(1);
        self.projection.revision = self.projection.revision.wrapping_add(1);
        self.projection.git.clear();
        self.projection.git_view = None;
        self.projection.focus = None;
    }
    pub fn begin_resnapshot(&mut self) -> Result<(), ProtocolError> {
        self.abandon_all_pending();
        self.clear_root_projection();
        self.projection.sessions = None;
        self.projection.session_state = Freshness::Stale;
        self.session_revision = 0;
        self.resnapshot = true;
        self.projection.root_state = Freshness::Loading;
        let path = self.root.clone();
        self.enqueue(Operation::RootValidate { path }, None, Purpose::Resnapshot)?;
        Ok(())
    }
    fn snapshot_root(&mut self) -> Result<(), ProtocolError> {
        self.load_directory(self.root.clone())?;
        self.refresh_git()?;
        self.refresh_sessions()?;
        self.enqueue(
            Operation::Watch {
                path: self.root.clone(),
            },
            None,
            Purpose::User,
        )?;
        Ok(())
    }
    pub fn resnapshot_pending(&self) -> bool {
        self.resnapshot
    }
    pub fn close_state(&self) -> CloseState {
        self.close_state
    }

    /// Start the bounded online teardown. Caller keeps polling/draining while
    /// waiting, then detaches local resources regardless of confirmation.
    pub fn begin_close(&mut self) -> Result<(), ProtocolError> {
        if self.close_state != CloseState::Open {
            return Err(ProtocolError::Identity);
        }
        self.projection.cleanup_unconfirmed = true;
        if self.connection == ConnectionState::Disconnected {
            self.close_state = CloseState::Unconfirmed;
            self.disconnect();
        } else {
            self.cancel_where(|_| true)?;
            self.enqueue(Operation::Close, None, Purpose::User)?;
            self.close_state = CloseState::Waiting;
        }
        Ok(())
    }
    /// Detach is local attachment release, never proof of remote teardown.
    pub fn release_terminal(
        &mut self,
        transport: &mut dyn made_terminal::transport::TerminalTransport,
    ) -> Result<(), made_terminal::PtyError> {
        self.disconnect();
        if self.close_state != CloseState::Confirmed {
            self.close_state = CloseState::Unconfirmed;
            self.projection.cleanup_unconfirmed = true;
        }
        transport.detach()
    }

    /// Only a previously acknowledged watch grants notification authority.
    /// This is separate from RPC replies; no unsolicited reply is accepted.
    pub fn filesystem_changed(
        &mut self,
        identity: &Identity,
        path: &RemotePath,
    ) -> Result<(), ProtocolError> {
        if self.connection != ConnectionState::Connected
            || !identity.matches(&self.identity)
            || !self
                .watch
                .as_ref()
                .is_some_and(|(owner, root)| owner.matches(identity) && path.is_within(root))
        {
            return Err(ProtocolError::Identity);
        }
        // Watch events are coalesced: one dirty mark, no unbounded event queue.
        self.mark_changed(path)
    }

    fn mark_changed(&mut self, path: &RemotePath) -> Result<(), ProtocolError> {
        let affected: Vec<_> = self
            .projection
            .directories
            .keys()
            .filter(|directory| directory.is_within(path) || path.is_within(directory))
            .cloned()
            .collect();
        for directory in affected {
            if let Some(projection) = self.projection.directories.get_mut(&directory) {
                projection.freshness = Freshness::Stale;
            }
        }
        if let Some(preview) = &mut self.projection.preview {
            preview.freshness = Freshness::Stale;
            self.projection.preview_revision = self.projection.preview_revision.wrapping_add(1);
        }
        for git in &mut self.projection.git {
            git.freshness = Freshness::Stale;
        }
        if let Some(view) = &mut self.projection.git_view {
            view.freshness = Freshness::Stale;
        }
        self.projection.revision = self.projection.revision.wrapping_add(1);
        Ok(())
    }

    /// Call on the refresh timer: prioritize visible directories and coalesce
    /// mutations while stalled. Bound work to available request capacity.
    pub fn reconcile(&mut self) -> Result<(), ProtocolError> {
        if self.connection != ConnectionState::Connected || self.close_state != CloseState::Open {
            return Ok(());
        }
        let dirty: Vec<_> = self
            .projection
            .directories
            .values()
            .filter(|d| d.freshness == Freshness::Stale)
            .map(|d| d.path.clone())
            .collect();
        for directory in dirty {
            if self.pending.len() >= MAX_IN_FLIGHT {
                break;
            }
            self.load_directory(directory)?;
        }
        if self.pending.len() < MAX_IN_FLIGHT
            && self
                .projection
                .preview
                .as_ref()
                .is_some_and(|p| p.freshness == Freshness::Stale)
        {
            self.refresh_preview()?;
        }
        if self.pending.len() + 8 <= MAX_IN_FLIGHT
            && self
                .projection
                .git
                .iter()
                .any(|g| g.freshness == Freshness::Stale)
        {
            self.refresh_git()?;
        }
        Ok(())
    }

    fn apply(
        &mut self,
        result: &RemoteResult,
        original: &Operation,
        purpose: Purpose,
    ) -> Result<(), ProtocolError> {
        match result {
            RemoteResult::Root(path) => match purpose {
                Purpose::Shell => {
                    if self.shell_report.as_ref() == Some(path) {
                        self.shell = Some(path.clone());
                        self.follow()?;
                    }
                }
                Purpose::Follow(source) => {
                    self.follow_request = None;
                    self.projection.source = source;
                    if source == RootSource::Selection {
                        self.selection = Some(path.clone());
                    }
                    self.projection.root_state = Freshness::Current;
                    self.snapshot_root()?;
                }
                Purpose::Resnapshot => {
                    self.projection.root_state = Freshness::Current;
                    self.snapshot_root()?;
                }
                Purpose::User => {
                    if matches!(original, Operation::RootSelect { .. }) {
                        self.clear_root_projection();
                        self.selection = Some(path.clone());
                        self.projection.source = RootSource::Selection;
                        self.projection.root_state = Freshness::Current;
                        self.snapshot_root()?;
                    }
                }
            },
            RemoteResult::Listing {
                path,
                entries,
                truncated,
            } => {
                let old_count = self
                    .projection
                    .directories
                    .get(path)
                    .map_or(0, |d| d.entries.len());
                let total: usize = self
                    .projection
                    .directories
                    .values()
                    .map(|d| d.entries.len())
                    .sum();
                if total - old_count
                    + entries.len()
                    + self.projection.directories.len()
                    + usize::from(!self.projection.directories.contains_key(path))
                    > MAX_TREE_ROWS
                {
                    return Err(ProtocolError::Limit);
                }
                self.projection.directories.insert(
                    path.clone(),
                    DirectoryProjection {
                        path: path.clone(),
                        entries: entries.clone(),
                        truncated: *truncated,
                        freshness: Freshness::Current,
                    },
                );
                // Deleted nodes lose their selection, expansion and preview.
                let children: HashSet<_> = entries.iter().map(|entry| entry.path.clone()).collect();
                let removed: Vec<_> = self
                    .projection
                    .expanded
                    .iter()
                    .filter(|p| p.is_direct_child_of(path) && !children.contains(*p))
                    .cloned()
                    .collect();
                for removed in removed {
                    self.projection.expanded.retain(|p| !p.is_within(&removed));
                    self.projection
                        .directories
                        .retain(|p, _| !p.is_within(&removed));
                }
                if self
                    .projection
                    .selected
                    .as_ref()
                    .is_some_and(|p| p.is_direct_child_of(path) && !children.contains(p))
                {
                    self.projection.selected = None;
                }
                if self
                    .projection
                    .preview
                    .as_ref()
                    .is_some_and(|p| p.path.is_direct_child_of(path) && !children.contains(&p.path))
                {
                    self.close_preview()?;
                }
            }
            RemoteResult::Preview { path, kind, data } => {
                if self
                    .projection
                    .preview
                    .as_ref()
                    .is_some_and(|p| &p.path == path)
                {
                    self.projection.preview = Some(PreviewProjection {
                        path: path.clone(),
                        kind: Some(*kind),
                        data: data.clone(),
                        freshness: Freshness::Current,
                    });
                    self.projection.preview_revision =
                        self.projection.preview_revision.wrapping_add(1);
                }
            }
            RemoteResult::Git { command, data } => {
                if let Some(git) = self
                    .projection
                    .git
                    .iter_mut()
                    .find(|g| &g.command == command)
                {
                    git.data.clone_from(data);
                    git.freshness = Freshness::Current;
                }
                if *command == GitRead::ShowToplevel && !trim_git_line(data).is_empty() {
                    self.projection.git_root_known = true;
                }
                self.update_git_view()?;
            }
            RemoteResult::Session(snapshot) => {
                if snapshot.revision < self.session_revision {
                    return Err(ProtocolError::Identity);
                }
                if snapshot.revision == self.session_revision {
                    self.projection.session_state = Freshness::Current;
                    self.projection.integration_error = None;
                } else {
                    self.session_revision = snapshot.revision;
                    self.integration = IntegrationState::Connected;
                    self.projection.sessions = Some(snapshot.clone());
                    self.projection.session_state = Freshness::Current;
                    self.projection.integration_error = None;
                    self.follow()?;
                }
            }
            RemoteResult::Focus {
                session_id,
                accepted,
            } => {
                self.projection.focus = Some((session_id.clone(), *accepted));
            }
            RemoteResult::Changed(path) => {
                if matches!(original, Operation::Watch { .. }) {
                    self.watch = Some((self.identity.clone(), path.clone()));
                } else {
                    self.mark_changed(path)?;
                }
            }
            RemoteResult::Closed { confirmed } => {
                self.close_state = if *confirmed {
                    CloseState::Confirmed
                } else {
                    CloseState::Unconfirmed
                };
                self.projection.cleanup_unconfirmed = !confirmed;
                self.disconnect();
            }
            RemoteResult::Error(code) => match original {
                Operation::RootSelect { .. } | Operation::RootValidate { .. } => match purpose {
                    Purpose::Follow(RootSource::View) => {
                        if let Some(snapshot) = &mut self.projection.sessions {
                            snapshot.view_directory = None;
                        }
                        self.follow_request = None;
                        self.follow()?;
                    }
                    Purpose::Shell => {
                        self.shell_report = None;
                    }
                    _ => {
                        self.follow_request = None;
                        self.projection.root_state = Freshness::Unavailable(*code);
                    }
                },
                Operation::List { path } => {
                    if let Some(directory) = self.projection.directories.get_mut(path) {
                        directory.freshness = Freshness::Unavailable(*code);
                    }
                    if path == &self.root {
                        self.close_preview()?;
                    }
                }
                Operation::Preview { .. } | Operation::Refresh { .. } => {
                    if let Some(preview) = &mut self.projection.preview {
                        preview.freshness = Freshness::Unavailable(*code);
                    }
                }
                Operation::GitRead { command } => {
                    if let Some(git) = self
                        .projection
                        .git
                        .iter_mut()
                        .find(|g| &g.command == command)
                    {
                        git.freshness = Freshness::Unavailable(*code);
                    }
                    self.update_git_view()?;
                }
                Operation::SessionState => {
                    self.set_integration(IntegrationState::Unavailable);
                    self.projection.integration_error = Some(*code);
                    self.follow()?;
                }
                Operation::Focus { .. } => {
                    self.projection.focus = None;
                }
                Operation::Close => {
                    self.close_state = CloseState::Unconfirmed;
                    self.projection.cleanup_unconfirmed = true;
                    self.disconnect();
                }
                _ => {}
            },
        }
        if self.resnapshot
            && self.projection.root_state == Freshness::Current
            && self
                .projection
                .directories
                .get(&self.root)
                .is_some_and(|d| d.freshness == Freshness::Current)
            && self
                .projection
                .git
                .iter()
                .all(|g| !matches!(g.freshness, Freshness::Loading | Freshness::Stale))
            && !matches!(
                self.projection.session_state,
                Freshness::Loading | Freshness::Stale
            )
        {
            self.resnapshot = false;
        }
        self.projection.revision = self.projection.revision.wrapping_add(1);
        Ok(())
    }

    fn update_git_view(&mut self) -> Result<(), ProtocolError> {
        if self.projection.git.len() != 8
            || self
                .projection
                .git
                .iter()
                .any(|g| matches!(g.freshness, Freshness::Loading | Freshness::Stale))
        {
            return Ok(());
        }
        let previous = self.projection.git_view.take();
        let get = |command| self.projection.git.iter().find(|g| g.command == command);
        let text = |command| {
            get(command)
                .filter(|g| g.freshness == Freshness::Current)
                .and_then(|g| std::str::from_utf8(&g.data).ok())
                .map(|s| s.trim_end_matches(['\r', '\n']).to_owned())
        };
        let branch = text(GitRead::Branch);
        let detached = branch.as_deref() == Some("HEAD");
        let top = get(GitRead::ShowToplevel)
            .filter(|g| g.freshness == Freshness::Current)
            .and_then(|g| RemotePath::new(trim_git_line(&g.data).to_vec()).ok());
        let status = get(GitRead::Status);
        let status_current = status.is_some_and(|status| status.freshness == Freshness::Current);
        let status_failure = status
            .filter(|status| matches!(status.freshness, Freshness::Unavailable(_)))
            .map(|status| status.freshness);
        let freshness = if top.is_some() && status_current {
            Freshness::Current
        } else if let Some(failure) = status_failure {
            failure
        } else {
            Freshness::Unavailable(ErrorCode::Unavailable)
        };
        // A failed read keeps the last complete groups and header. Only a root
        // that has never produced a complete status falls back to the failed
        // empty state.
        if top.is_none() || !status_current {
            if let Some(view) = previous.as_ref().filter(|view| view.has_status) {
                let mut view = view.clone();
                view.freshness = freshness;
                self.projection.git_view = Some(view);
                return Ok(());
            }
        }
        let groups = match (
            &top,
            status.filter(|status| status.freshness == Freshness::Current),
        ) {
            (Some(top), Some(status)) => match remote_git_groups(&status.data, top, &self.root) {
                Ok(groups) => groups,
                Err(error) => {
                    if let Some(view) = previous.filter(|view| view.has_status) {
                        let mut view = view;
                        view.freshness = Freshness::Unavailable(ErrorCode::Unavailable);
                        self.projection.git_view = Some(view);
                        return Ok(());
                    }
                    return Err(error);
                }
            },
            _ => Vec::new(),
        };
        let linked_worktree = match (text(GitRead::GitDir), text(GitRead::CommonDir)) {
            (Some(a), Some(b)) => a != b,
            _ => false,
        };
        let has_status = top.is_some() && status_current;
        self.projection.git_view = Some(RemoteGitView {
            branch: branch.filter(|s| s != "HEAD"),
            detached,
            commit: text(GitRead::CommitId),
            subject: text(GitRead::CommitSubject),
            linked_worktree,
            worktree_name: top
                .as_ref()
                .and_then(|p| p.as_bytes().rsplit(|b| *b == b'/').next())
                .map(|s| String::from_utf8_lossy(s).into_owned()),
            folder_marks: remote_folder_marks(&groups, &self.root)?,
            groups,
            freshness,
            has_status,
        });
        Ok(())
    }
}

fn trim_git_line(bytes: &[u8]) -> &[u8] {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    bytes.strip_suffix(b"\r").unwrap_or(bytes)
}

fn remote_depth(path: &RemotePath, root: &RemotePath) -> usize {
    path.as_bytes()[root.as_bytes().len()..]
        .split(|b| *b == b'/')
        .filter(|part| !part.is_empty())
        .count()
}

fn remote_folder_marks(
    groups: &[RemoteGitGroup],
    root: &RemotePath,
) -> Result<HashMap<RemotePath, Vec<made_remote::git::ChangeKind>>, ProtocolError> {
    fn parent(path: &RemotePath) -> Option<RemotePath> {
        let last = path.as_bytes().iter().rposition(|b| *b == b'/')?;
        if path.as_bytes() == b"/" {
            return None;
        }
        RemotePath::new(path.as_bytes()[..last.max(1)].to_vec()).ok()
    }
    let mut marks: HashMap<RemotePath, Vec<made_remote::git::ChangeKind>> = HashMap::new();
    for row in groups.iter().flat_map(|group| &group.rows) {
        for path in std::iter::once(&row.path).chain(row.original.iter()) {
            let mut directory = if row.directory && path == &row.path {
                Some(path.clone())
            } else {
                parent(path)
            };
            while let Some(path) = directory {
                if path == *root || !path.is_within(root) {
                    break;
                }
                if !marks.contains_key(&path) && marks.len() >= MAX_TREE_ROWS {
                    return Err(ProtocolError::Limit);
                }
                let kinds = marks.entry(path.clone()).or_default();
                if !kinds.contains(&row.kind) {
                    kinds.push(row.kind);
                    kinds.sort();
                }
                directory = parent(&path);
            }
        }
    }
    Ok(marks)
}

/// Git porcelain paths are POSIX bytes relative to repository top, not host
/// paths. Only rows inside the selected remote subroot reach the presentation.
fn remote_git_groups(
    bytes: &[u8],
    top: &RemotePath,
    root: &RemotePath,
) -> Result<Vec<RemoteGitGroup>, ProtocolError> {
    use made_remote::git::ChangeKind;
    fn resolve(top: &RemotePath, root: &RemotePath, raw: &[u8]) -> Option<RemotePath> {
        if raw.is_empty() || raw.starts_with(b"/") {
            return None;
        }
        let mut bytes = top.as_bytes().to_vec();
        if bytes != b"/" {
            bytes.push(b'/');
        }
        bytes.extend_from_slice(raw);
        RemotePath::new(bytes)
            .ok()
            .filter(|path| path != root && path.is_within(root))
    }
    fn side(code: u8) -> Option<(char, ChangeKind)> {
        match code {
            b'A' => Some(('A', ChangeKind::New)),
            b'M' | b'T' => Some(('M', ChangeKind::Modified)),
            b'D' => Some(('D', ChangeKind::Deleted)),
            b'R' | b'C' => Some(('R', ChangeKind::Renamed)),
            _ => None,
        }
    }
    if !bytes.is_empty() && !bytes.ends_with(b"\0") {
        return Err(ProtocolError::Malformed);
    }
    let mut groups: [Vec<RemoteGitRow>; 4] = std::array::from_fn(|_| Vec::new());
    let mut records = bytes.split(|b| *b == 0);
    let mut count = 0usize;
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if record.len() < 4 || record[2] != b' ' || !record[..2].is_ascii() {
            return Err(ProtocolError::Malformed);
        }
        let index = record[0];
        let worktree = record[1];
        let original = if matches!(index, b'R' | b'C') || matches!(worktree, b'R' | b'C') {
            Some(records.next().ok_or(ProtocolError::Malformed)?)
        } else {
            None
        };
        if index == b'!' || worktree == b'!' {
            continue;
        }
        let raw = &record[3..];
        let directory = raw.ends_with(b"/");
        let raw = raw.strip_suffix(b"/").unwrap_or(raw);
        let Some(path) = resolve(top, root, raw) else {
            continue;
        };
        let original = original.and_then(|raw| resolve(top, root, raw));
        let conflict = index == b'U'
            || worktree == b'U'
            || (index == b'A' && worktree == b'A')
            || (index == b'D' && worktree == b'D');
        let mut placements = Vec::with_capacity(2);
        if conflict {
            placements.push((0, 'C', ChangeKind::Conflict));
        } else if index == b'?' && worktree == b'?' {
            placements.push((3, 'U', ChangeKind::New));
        } else {
            if let Some((letter, kind)) = side(index) {
                placements.push((1, letter, kind));
            }
            if let Some((letter, kind)) = side(worktree) {
                placements.push((2, letter, kind));
            }
        }
        for (group, letter, kind) in placements {
            count += 1;
            if count > MAX_TREE_ROWS {
                return Err(ProtocolError::Limit);
            }
            groups[group].push(RemoteGitRow {
                path: path.clone(),
                original: original.clone(),
                letter,
                kind,
                directory,
            });
        }
    }
    let titles = [
        crate::worktree::GROUP_MERGE,
        crate::worktree::GROUP_STAGED,
        crate::worktree::GROUP_CHANGES,
        crate::worktree::GROUP_UNTRACKED,
    ];
    Ok(groups
        .into_iter()
        .enumerate()
        .filter_map(|(index, mut rows)| {
            rows.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
            (!rows.is_empty()).then_some(RemoteGitGroup {
                title: titles[index],
                rows,
            })
        })
        .collect())
}
