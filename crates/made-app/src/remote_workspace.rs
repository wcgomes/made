//! Transport-independent, authority-bound request tracking for a remote tab.
//!
//! This is a model seam, not an SSH client. The caller sends returned envelopes
//! through the authenticated transport and supplies replies to `accept`.
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Instant,
};

use made_remote::protocol::{
    ClaudeListingResult, ClaudeSessionSnapshot, DirectoryEntry, ErrorCode, GitRead, GrokEventKind,
    GrokSessionEvent, GrokSessionInfo, GrokSessionSnapshot, Identity, Operation, PreviewKind,
    ProtocolError, RemotePath, ReplyProgress, WireEnvelope, CLOSE_TIMEOUT, MAX_IN_FLIGHT,
    MAX_QUEUED_CHUNKS, MAX_TREE_DEPTH, MAX_TREE_ROWS, RPC_TIMEOUT,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootSource {
    Launch,
    Shell,
    View,
    Session,
    Claude,
    Grok,
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
    /// Owned remote Grok sessions, independent of the OpenCode integration.
    pub grok: Option<GrokSessionSnapshot>,
    /// Owned remote Claude Code sessions, relayed by the companion.
    pub claude: Option<ClaudeSessionSnapshot>,
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
    Grok(GrokSessionSnapshot),
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
    /// Monotonic Grok inventory revision within the transport epoch.
    grok_revision: u64,
    /// Hook tier from the latest snapshot, or from a hook event that proves it.
    grok_hooks_ready: Option<bool>,
    /// Last inventory-observed newest owned session; guards the no-steal-back rule.
    grok_newest: Option<String>,
    /// Remote cwd the Grok domain currently establishes for root follow.
    grok_root: Option<RemotePath>,
    /// Monotonic Claude Code snapshot revision within the transport epoch.
    claude_revision: u64,
    /// The listings' own monotonic revision within the transport epoch, so a
    /// listing drained after a newer snapshot reply still applies.
    claude_listing_revision: u64,
    /// The latest fresh agent-view listing, until the tab takes it.
    claude_listing: Option<ClaudeListingResult>,
    /// Remote cwd the tab's Claude Code state establishes for root follow.
    claude_root: Option<RemotePath>,
    /// A Claude Code directory the server rejected; not retried until the
    /// reported directory changes.
    claude_rejected: Option<RemotePath>,
    /// A Claude Code directory whose check failed transiently, retried once
    /// its backoff has passed.
    claude_retry: Option<(RemotePath, Instant)>,
}

/// How long a transiently failed Claude Code directory waits before it is
/// checked again, like the local liveness retry.
const CLAUDE_RETRY: std::time::Duration = std::time::Duration::from_secs(1);

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
                grok: None,
                claude: None,
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
            grok_revision: 0,
            grok_hooks_ready: None,
            grok_newest: None,
            grok_root: None,
            claude_revision: 0,
            claude_listing_revision: 0,
            claude_listing: None,
            claude_root: None,
            claude_rejected: None,
            claude_retry: None,
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
                | Operation::GrokState
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
            Operation::GrokSnapshot { snapshot } => Some(RemoteResult::Grok(snapshot)),
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

    /// Apply an unsolicited session snapshot that was not matched to a pending
    /// workspace request. It does not consume or complete pending work by
    /// request id; accepting a newer revision may still trigger the normal
    /// root-follow transition, which intentionally abandons old-root pending
    /// work.
    pub fn accept_session_event(&mut self, event: WireEnvelope) -> Result<(), ProtocolError> {
        if self.connection != ConnectionState::Connected || !event.identity.matches(&self.identity)
        {
            return Err(ProtocolError::Identity);
        }
        event.validate()?;
        let Operation::SessionSnapshot { snapshot } = event.operation else {
            return Err(ProtocolError::UnexpectedResponse);
        };
        self.apply(
            &RemoteResult::Session(snapshot),
            &Operation::SessionState,
            Purpose::User,
        )
    }

    /// Apply an unsolicited Grok hook event pushed under the current attach
    /// id. It never consumes pending workspace work, even by numeric request
    /// id equality; correlated GrokState replies flow through `accept`.
    pub fn accept_grok_event(&mut self, event: WireEnvelope) -> Result<(), ProtocolError> {
        if self.connection != ConnectionState::Connected || !event.identity.matches(&self.identity)
        {
            return Err(ProtocolError::Identity);
        }
        event.validate()?;
        let Operation::GrokEvent { event } = event.operation else {
            return Err(ProtocolError::UnexpectedResponse);
        };
        self.apply_grok_event(event)
    }

    /// Apply an unsolicited Grok push delivered under the current attach id:
    /// either a revisioned snapshot or one companion-filtered hook event.
    /// Neither consumes pending workspace work, even by numeric request id
    /// equality; a correlated `GrokState` reply flows through `accept` instead.
    pub fn accept_grok_push(&mut self, push: WireEnvelope) -> Result<(), ProtocolError> {
        if self.connection != ConnectionState::Connected || !push.identity.matches(&self.identity) {
            return Err(ProtocolError::Identity);
        }
        push.validate()?;
        match push.operation {
            Operation::GrokSnapshot { snapshot } => self
                .apply(
                    &RemoteResult::Grok(snapshot),
                    &Operation::GrokState,
                    Purpose::User,
                )
                .map(|_| ()),
            Operation::GrokEvent { event } => self.apply_grok_event(event),
            _ => Err(ProtocolError::UnexpectedResponse),
        }
    }

    /// Apply relayed Claude Code state: the deferred `ClaudeSnapshot` reply to
    /// `ClaudeState`, or a `ClaudeEvent`/`ClaudeListing` push under the attach
    /// id. None consumes pending workspace work. The state is not
    /// filesystem-generation scoped, so only the tab, authority and epoch must
    /// match; snapshots (and events) and listings each have their own revision
    /// sequence, an equal revision is a duplicate that changes nothing and an
    /// older one is rejected.
    pub fn accept_claude(&mut self, frame: WireEnvelope) -> Result<(), ProtocolError> {
        if self.connection != ConnectionState::Connected
            || frame.identity.authority != self.identity.authority
            || !frame.identity.tab.authenticates(&self.identity.tab)
            || frame.identity.epoch != self.identity.epoch
        {
            return Err(ProtocolError::Identity);
        }
        frame.validate()?;
        let (last, revision) = match &frame.operation {
            Operation::ClaudeSnapshot { snapshot } | Operation::ClaudeEvent { snapshot } => {
                (&mut self.claude_revision, snapshot.revision)
            }
            Operation::ClaudeListing { listing } => {
                (&mut self.claude_listing_revision, listing.revision)
            }
            _ => return Err(ProtocolError::UnexpectedResponse),
        };
        if revision < *last {
            return Err(ProtocolError::Identity);
        }
        if revision == *last {
            return Ok(());
        }
        *last = revision;
        match frame.operation {
            Operation::ClaudeSnapshot { snapshot } | Operation::ClaudeEvent { snapshot } => {
                self.projection.claude = Some(snapshot);
            }
            Operation::ClaudeListing { listing } => self.claude_listing = Some(listing),
            _ => {}
        }
        self.projection.revision = self.projection.revision.wrapping_add(1);
        Ok(())
    }

    /// The latest fresh agent-view listing, handed out once.
    pub fn take_claude_listing(&mut self) -> Option<ClaudeListingResult> {
        self.claude_listing.take()
    }

    /// Whether the staged Claude Code package is bound for this tab; unknown
    /// until the current attachment's first snapshot.
    pub fn claude_hooks_available(&self) -> Option<bool> {
        self.projection
            .claude
            .as_ref()
            .map(|snapshot| snapshot.hooks_available)
    }

    /// The directory the tab's Claude Code state selects (agent view, then the
    /// owned session), or `None`. It enters the root precedence only through
    /// server validation; a directory the server rejected is not retried.
    pub fn set_claude_root(&mut self, directory: Option<RemotePath>) -> Result<(), ProtocolError> {
        // A directory that ends or changes cancels a pending retry, so an
        // ended session's directory never comes back.
        let retrying = self
            .claude_retry
            .as_ref()
            .is_some_and(|(retry, _)| directory.as_ref() == Some(retry));
        if !retrying {
            self.claude_retry = None;
        }
        if directory.is_none() {
            self.claude_rejected = None;
        }
        if directory == self.claude_root
            || retrying
            || (directory.is_some() && directory == self.claude_rejected)
        {
            return Ok(());
        }
        self.claude_rejected = None;
        self.claude_retry = None;
        self.claude_root = directory;
        if self.connection == ConnectionState::Connected && self.close_state == CloseState::Open {
            self.follow()?;
        }
        Ok(())
    }

    /// Previous-epoch Claude Code state is never reused; a fresh snapshot
    /// re-establishes it.
    fn clear_claude(&mut self) {
        self.projection.claude = None;
        self.claude_revision = 0;
        self.claude_listing_revision = 0;
        self.claude_listing = None;
        self.claude_root = None;
        self.claude_rejected = None;
        self.claude_retry = None;
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
        self.clear_grok();
        self.clear_claude();
    }

    /// Previous-epoch Grok state is never reused; a fresh snapshot re-establishes it.
    fn clear_grok(&mut self) {
        self.projection.grok = None;
        self.grok_revision = 0;
        self.grok_hooks_ready = None;
        self.grok_newest = None;
        self.grok_root = None;
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
        // Previous-epoch Grok and Claude Code state is never reused; the
        // post-attach refresh re-establishes it from a fresh snapshot.
        self.clear_grok();
        self.clear_claude();
        self.begin_resnapshot()
    }

    /// A pre-attach failure cannot supply a tree. Clear any partial projection
    /// and publish a bounded unavailable state so the Explorer never spins.
    /// Post-attach transport loss uses `disconnect` and keeps prior rows.
    pub fn project_unavailable(&mut self, code: made_remote::protocol::ErrorCode) {
        self.connection = ConnectionState::Disconnected;
        self.integration = IntegrationState::Unknown;
        self.projection.root_state = Freshness::Unavailable(code);
        self.projection.requested_root = None;
        self.projection.directories.clear();
        self.projection.expanded.clear();
        self.projection.selected = None;
        self.projection.git_selected = None;
        self.projection.git_root_known = false;
        self.projection.git.clear();
        self.projection.git_view = None;
        self.projection.preview = None;
        self.projection.preview_revision = self.projection.preview_revision.wrapping_add(1);
        self.projection.sessions = None;
        self.projection.session_state = Freshness::Stale;
        self.projection.focus = None;
        self.session_revision = 0;
        self.projection.revision = self.projection.revision.wrapping_add(1);
    }

    pub fn projection(&self) -> &WorkspaceProjection {
        &self.projection
    }
    /// Unknown until a snapshot or hook event establishes the companion tier.
    pub fn grok_hooks_ready(&self) -> Option<bool> {
        self.grok_hooks_ready
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

    /// Ten-second RPC deadlines are driven by the UI's nonblocking timer, as
    /// is the retry of a transiently failed Claude Code directory.
    pub fn expire(&mut self, now: Instant) -> Result<Vec<u64>, ProtocolError> {
        if self
            .claude_retry
            .as_ref()
            .is_some_and(|(_, due)| now >= *due)
        {
            self.claude_root = self.claude_retry.take().map(|(directory, _)| directory);
            if self.connection == ConnectionState::Connected && self.close_state == CloseState::Open
            {
                self.follow()?;
            }
        }
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

    /// Refresh the revisioned owned-Grok inventory. The companion answers on
    /// the same request id with a GrokSnapshot; events are separate.
    pub fn refresh_grok(&mut self) -> Result<u64, ProtocolError> {
        self.cancel_where(|p| matches!(p.request.operation, Operation::GrokState))?;
        self.enqueue(Operation::GrokState, None, Purpose::User)
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
        // The tab's Claude Code directory outranks Grok and the shell cwd but
        // never a connected OpenCode session or view.
        if let Some(directory) = self.claude_root.as_ref() {
            return Ok((directory.clone(), RootSource::Claude));
        }
        // Owned Grok sessions outrank the validated shell cwd but never a
        // connected OpenCode session or view.
        if let Some(directory) = self.grok_root.as_ref() {
            return Ok((directory.clone(), RootSource::Grok));
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
        self.clear_grok();
        self.resnapshot = true;
        self.projection.root_state = Freshness::Loading;
        let path = self.root.clone();
        self.enqueue(Operation::RootValidate { path }, None, Purpose::Resnapshot)?;
        Ok(())
    }
    fn snapshot_root(&mut self, purpose: Purpose) -> Result<(), ProtocolError> {
        // The owned Grok inventory is independent of the OpenCode integration.
        // A fresh attachment discards previous-epoch state, so the resnapshot
        // asks the companion for a fresh revisioned snapshot first: answering
        // it before the shell-fallback follow keeps the owned Grok root from
        // being abandoned mid-batch. Event- and shell-driven root changes keep
        // the already-current projection; a transient GrokState error must
        // never wipe it on every root change.
        if matches!(purpose, Purpose::Resnapshot) {
            self.refresh_grok()?;
        }
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
                    self.snapshot_root(purpose)?;
                }
                Purpose::Resnapshot => {
                    self.projection.root_state = Freshness::Current;
                    self.snapshot_root(purpose)?;
                }
                Purpose::User => {
                    if matches!(original, Operation::RootSelect { .. }) {
                        self.clear_root_projection();
                        self.selection = Some(path.clone());
                        self.projection.source = RootSource::Selection;
                        self.projection.root_state = Freshness::Current;
                        self.snapshot_root(purpose)?;
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
            RemoteResult::Grok(snapshot) => {
                if snapshot.revision < self.grok_revision {
                    return Err(ProtocolError::Identity);
                }
                if snapshot.revision != self.grok_revision {
                    self.grok_revision = snapshot.revision;
                    self.grok_hooks_ready = Some(snapshot.hooks_ready);
                    self.projection.grok = Some(snapshot.clone());
                    match grok_newest_session(snapshot) {
                        Some(session)
                            if self.grok_newest.as_deref() != Some(session.session_id.as_str()) =>
                        {
                            // A newly attached session moves immediately; an
                            // unchanged newest attachment never steals back.
                            self.grok_newest = Some(session.session_id.clone());
                            if let Some(directory) = &session.cwd {
                                self.grok_root = Some(directory.clone());
                                self.follow()?;
                            }
                        }
                        None if snapshot.sessions.is_empty() && self.grok_newest.is_some() => {
                            // No owned session remains; the shell fallback returns.
                            self.grok_newest = None;
                            self.grok_root = None;
                            self.follow()?;
                        }
                        _ => {}
                    }
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
                    // An unusable Claude Code directory never moves the
                    // explorer: the next holder keeps the root. A missing,
                    // denied or invalid one is rejected; a transient failure
                    // is checked again after a backoff.
                    Purpose::Follow(RootSource::Claude) => {
                        let directory = self.claude_root.take();
                        if matches!(
                            code,
                            ErrorCode::Unavailable
                                | ErrorCode::Unauthorized
                                | ErrorCode::InvalidRequest
                                | ErrorCode::Unsupported
                        ) {
                            self.claude_rejected = directory;
                        } else {
                            self.claude_retry = directory
                                .map(|directory| (directory, Instant::now() + CLAUDE_RETRY));
                        }
                        self.follow_request = None;
                        self.follow()?;
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
                Operation::GrokState => {
                    // A failed Grok read is honest: the projection goes away
                    // without touching the independent OpenCode integration.
                    self.clear_grok();
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

    /// Apply one companion-filtered hook event to the owned Grok projection.
    /// `SessionStart`/`Prompt` move the root immediately; `SessionEnd` removes
    /// the row and releases the root only when no owned session remains.
    fn apply_grok_event(&mut self, event: GrokSessionEvent) -> Result<(), ProtocolError> {
        let mut moved = false;
        match event.kind {
            GrokEventKind::SessionEnd => {
                if let Some(snapshot) = &mut self.projection.grok {
                    snapshot
                        .sessions
                        .retain(|session| session.session_id != event.session_id);
                }
                if self
                    .projection
                    .grok
                    .as_ref()
                    .is_none_or(|snapshot| snapshot.sessions.is_empty())
                {
                    self.grok_newest = None;
                    moved = self.grok_root.take().is_some();
                }
            }
            kind => {
                if let Some(snapshot) = &mut self.projection.grok {
                    match snapshot
                        .sessions
                        .iter_mut()
                        .find(|session| session.session_id == event.session_id)
                    {
                        Some(session) => {
                            session.activity = event.activity;
                            if let Some(cwd) = &event.cwd {
                                session.cwd = Some(cwd.clone());
                            }
                        }
                        None => snapshot.sessions.push(GrokSessionInfo {
                            session_id: event.session_id.clone(),
                            cwd: event.cwd.clone(),
                            // Provisional until the next inventory snapshot
                            // confirms the attachment order.
                            opened_at: 0,
                            activity: event.activity,
                        }),
                    }
                }
                if matches!(kind, GrokEventKind::SessionStart | GrokEventKind::Prompt) {
                    if let Some(cwd) = event.cwd {
                        if self.grok_root.as_ref() != Some(&cwd) {
                            self.grok_root = Some(cwd);
                            moved = true;
                        }
                    }
                }
            }
        }
        if moved {
            self.follow()?;
        }
        // Companions emit owned hook events only after arming the hook tier.
        self.grok_hooks_ready = Some(true);
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

/// Newest owned attachment; equal keys keep the later snapshot row.
fn grok_newest_session(snapshot: &GrokSessionSnapshot) -> Option<&GrokSessionInfo> {
    snapshot
        .sessions
        .iter()
        .max_by_key(|session| session.opened_at)
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

// ---- Open workspace browse grant, client side (SSH-12) ----------------------
//
// A screen-scoped, read-only grant on one server: folder listings, directory
// checks for the address bar and for launch, and the fixed read-only git reads
// of the launch target and the worktrees the repository reports. Like
// `RemoteWorkspace` this is a transport-independent model; `BrowseConnection`
// pumps it over one companion stdio stream without ever blocking.

use crate::open_workspace::{CountsState, GitState, LaunchKind, WorktreeItem};
use crate::remote_session::SshFailureClass;
use crate::tui::TerminalTarget;
use crate::workspace_picker::AddressError;
use made_platform::ssh_askpass::{PromptAnswer, PromptRequest};
use made_remote::protocol::{
    AuthorityId, BrowseEnvelope, BrowseFrameDecoder, BrowseGrant, BrowseIdentity, BrowseOperation,
    BrowseReplyProgress, DirectoryCheck, EntryKind,
};
use std::path::PathBuf;

/// How long one browse request may stay unanswered once it is on the wire.
/// The open request's clock is paused while an SSH prompt awaits the user.
pub const BROWSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Most working trees one repository read shows. Later and duplicate entries
/// of the server's list are dropped; [`RemoteBrowse::git_truncated`] reports
/// the cut.
pub const MAX_WORKTREES: usize = 256;

/// Most requests waiting behind the [`MAX_IN_FLIGHT`] requests on the wire.
const MAX_QUEUED_BROWSE: usize = 1024;

/// Most frames, and bytes, one [`BrowseConnection::pump`] reads before it
/// yields to the UI; the rest waits for the next pump.
const PUMP_FRAME_BUDGET: usize = 256;
const PUMP_BYTE_BUDGET: usize = 256 * 1024;

/// The terminal target an Open workspace launch button starts on a server.
/// Never resolves a host executable.
pub fn remote_launch_target(kind: LaunchKind, authority: AuthorityId) -> TerminalTarget {
    match kind {
        LaunchKind::Tui(tui) if tui == crate::integrations::OPENCODE => {
            TerminalTarget::RemoteOpenCode { authority }
        }
        LaunchKind::Tui(tui) if tui == crate::integrations::GROK => {
            TerminalTarget::RemoteGrok { authority }
        }
        LaunchKind::Tui(tui) if tui == crate::integrations::CLAUDE_CODE => {
            TerminalTarget::RemoteClaude { authority }
        }
        LaunchKind::Terminal | LaunchKind::Tui(_) => TerminalTarget::RemoteShell { authority },
    }
}

/// Whether browsing may start on a server under the SSH-06 staging consent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseConsent {
    /// Consent for this version exists or was just given.
    Proceed,
    /// Ask with the existing consent flow before anything is written.
    Ask,
    /// Refused: nothing is staged, the reason is shown in the folder area.
    Refused(&'static str),
}

/// Decide browsing from the saved consent and the user's answer, if any.
pub fn browse_consent(consented: bool, answer: Option<bool>) -> BrowseConsent {
    match (consented, answer) {
        (true, _) | (false, Some(true)) => BrowseConsent::Proceed,
        (false, None) => BrowseConsent::Ask,
        (false, Some(false)) => BrowseConsent::Refused(
            "Browsing this server needs Made's helper there. Select the host again to approve it. Nothing was written.",
        ),
    }
}

/// Parse an address-bar path for a server: surrounding whitespace and
/// trailing separators are ignored, and it must be absolute. Existence is
/// checked on the server, never on this computer.
pub fn parse_remote_address(text: &str) -> Result<RemotePath, AddressError> {
    let text = text.trim();
    if !text.starts_with('/') {
        return Err(AddressError::NotAbsolute);
    }
    let trimmed = text.trim_end_matches('/');
    let text = if trimmed.is_empty() { "/" } else { trimmed };
    RemotePath::new(text.as_bytes().to_vec()).map_err(|_| AddressError::NotFound)
}

fn check_result(check: &DirectoryCheck) -> Result<RemotePath, AddressError> {
    match check {
        DirectoryCheck::Directory { path } => Ok(path.clone()),
        DirectoryCheck::Missing => Err(AddressError::NotFound),
        DirectoryCheck::Unreadable => Err(AddressError::NotReadable),
        DirectoryCheck::NotDirectory => Err(AddressError::NotAFolder),
    }
}

/// One folder of a remote listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFolder {
    /// Server path; never resolved on this computer.
    pub path: RemotePath,
    /// Sanitized display name.
    pub name: String,
}

/// A finished browse result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseEvent {
    /// The server accepted the grant and reported its user's home: a
    /// server-side absolute readable directory, or `None` when unknown.
    Opened { home: Option<RemotePath> },
    /// The folders of `path`, capped at 1000 entries.
    Listed {
        path: RemotePath,
        folders: Vec<RemoteFolder>,
        truncated: bool,
    },
    /// The listing of `path` failed; the row stays collapsed.
    ListFailed { path: RemotePath },
    /// The address-bar path check.
    AddressChecked {
        requested: RemotePath,
        result: Result<RemotePath, AddressError>,
    },
    /// The launch-time revalidation of the effective launch path.
    LaunchChecked {
        kind: LaunchKind,
        requested: RemotePath,
        result: Result<RemotePath, AddressError>,
    },
    /// The git state of the launch target for `generation`.
    Git { generation: u64, state: GitState },
    /// Change counts of one working tree for `generation`.
    Counts {
        generation: u64,
        path: PathBuf,
        counts: CountsState,
    },
    /// The grant ended; no tab was affected.
    Ended,
}

/// Lifecycle of the grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowsePhase {
    Opening,
    Open,
    Closing,
    Ended,
}

#[derive(Debug, Clone)]
enum BrowsePurpose {
    Open,
    Close,
    List(RemotePath),
    Address(RemotePath),
    Launch(LaunchKind, RemotePath),
    Toplevel(u64),
    Worktrees(u64),
    WorktreeCheck(u64, usize),
    Status(u64, PathBuf),
}

struct BrowsePending {
    request: BrowseEnvelope,
    progress: BrowseReplyProgress,
    purpose: BrowsePurpose,
    sent: Instant,
    data: Vec<u8>,
    entries: Vec<DirectoryEntry>,
    truncated: bool,
}

struct GitSequence {
    generation: u64,
    target: RemotePath,
    toplevel: Option<RemotePath>,
    worktrees: Vec<WorktreeItem>,
    checks: usize,
    truncated: bool,
}

/// The client side of one screen-scoped browse grant.
pub struct RemoteBrowse {
    identity: BrowseIdentity,
    phase: BrowsePhase,
    next_id: u64,
    root: Option<RemotePath>,
    outbox: VecDeque<BrowseEnvelope>,
    /// Requests on the wire, at most [`MAX_IN_FLIGHT`].
    pending: Vec<BrowsePending>,
    /// Requests waiting for a free in-flight slot, in order.
    queued: VecDeque<BrowsePending>,
    git: Option<GitSequence>,
    open_paused: bool,
}

impl std::fmt::Debug for RemoteBrowse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteBrowse")
            .field("phase", &self.phase)
            .field("pending", &self.pending.len())
            .finish()
    }
}

impl RemoteBrowse {
    /// Start a grant with a fresh random capability; its first frame is
    /// `BrowseOpen`.
    pub fn open(authority: AuthorityId, epoch: u64) -> Result<Self, ProtocolError> {
        let identity = BrowseIdentity {
            authority,
            grant: BrowseGrant::random()?,
            epoch,
        };
        identity.validate()?;
        let mut browse = Self {
            identity,
            phase: BrowsePhase::Opening,
            next_id: 1,
            root: None,
            outbox: VecDeque::new(),
            pending: Vec::new(),
            queued: VecDeque::new(),
            git: None,
            open_paused: false,
        };
        browse.send(BrowseOperation::BrowseOpen, BrowsePurpose::Open)?;
        Ok(browse)
    }

    pub fn identity(&self) -> &BrowseIdentity {
        &self.identity
    }

    pub fn phase(&self) -> BrowsePhase {
        self.phase
    }

    /// Unanswered requests: on the wire or waiting for a slot.
    pub fn pending_count(&self) -> usize {
        self.pending.len() + self.queued.len()
    }

    /// Whether the current repository read listed more than
    /// [`MAX_WORKTREES`] distinct working trees.
    pub fn git_truncated(&self) -> bool {
        self.git.as_ref().is_some_and(|sequence| sequence.truncated)
    }

    /// Pause the open request's [`BROWSE_TIMEOUT`] while an SSH prompt awaits
    /// the user. Resuming gives the open request a fresh window.
    pub fn pause_open_timeout(&mut self, paused: bool) {
        if self.open_paused && !paused {
            let now = Instant::now();
            for pending in &mut self.pending {
                if matches!(pending.purpose, BrowsePurpose::Open) {
                    pending.sent = now;
                }
            }
        }
        self.open_paused = paused;
    }

    /// The next frame to write to the companion, in order.
    pub fn next_frame(&mut self) -> Option<BrowseEnvelope> {
        self.outbox.pop_front()
    }

    /// The tree root; listings outside it are stale and dropped.
    pub fn set_root(&mut self, root: RemotePath) {
        self.root = Some(root);
    }

    /// List the folders of `path`.
    pub fn list(&mut self, path: RemotePath) -> Result<u64, ProtocolError> {
        self.send(
            BrowseOperation::BrowseList { path: path.clone() },
            BrowsePurpose::List(path),
        )
    }

    /// Check an address-bar path on the server.
    pub fn check_address(&mut self, path: RemotePath) -> Result<u64, ProtocolError> {
        self.send(
            BrowseOperation::BrowseCheck { path: path.clone() },
            BrowsePurpose::Address(path),
        )
    }

    /// Revalidate the effective launch path on the server before `kind`
    /// starts.
    pub fn check_launch(
        &mut self,
        kind: LaunchKind,
        path: RemotePath,
    ) -> Result<u64, ProtocolError> {
        self.send(
            BrowseOperation::BrowseCheck { path: path.clone() },
            BrowsePurpose::Launch(kind, path),
        )
    }

    /// Read the git state of `target` for `generation`, superseding any
    /// earlier read: its late replies are dropped and it issues nothing more.
    pub fn read_git(&mut self, generation: u64, target: RemotePath) -> Result<(), ProtocolError> {
        self.git = Some(GitSequence {
            generation,
            target: target.clone(),
            toplevel: None,
            worktrees: Vec::new(),
            checks: 0,
            truncated: false,
        });
        // Waiting reads of a superseded sequence are never sent.
        self.queued
            .retain(|pending| git_generation(&pending.purpose).is_none_or(|g| g == generation));
        self.git_read(
            target,
            GitRead::ShowToplevel,
            BrowsePurpose::Toplevel(generation),
        )?;
        Ok(())
    }

    /// End the grant with the server; pending results are dropped.
    pub fn close(&mut self) {
        if matches!(self.phase, BrowsePhase::Opening | BrowsePhase::Open) {
            self.pending.clear();
            self.queued.clear();
            self.outbox.clear();
            self.git = None;
            if self
                .send(BrowseOperation::BrowseClose, BrowsePurpose::Close)
                .is_ok()
            {
                self.phase = BrowsePhase::Closing;
            } else {
                self.end();
            }
        }
    }

    /// The transport ended or the host changed: drop everything at once.
    pub fn end(&mut self) {
        self.phase = BrowsePhase::Ended;
        self.pending.clear();
        self.queued.clear();
        self.outbox.clear();
        self.git = None;
    }

    /// Fail every request on the wire for [`BROWSE_TIMEOUT`] at `now`, except
    /// the open request while its timeout is paused. Waiting requests do not
    /// age; freed slots send the next ones.
    pub fn expire(&mut self, now: Instant) -> Vec<BrowseEvent> {
        let mut events = Vec::new();
        let mut index = 0;
        while index < self.pending.len() {
            let paused =
                self.open_paused && matches!(self.pending[index].purpose, BrowsePurpose::Open);
            if !paused && now.saturating_duration_since(self.pending[index].sent) >= BROWSE_TIMEOUT
            {
                let pending = self.pending.remove(index);
                self.fail(pending.purpose, &mut events);
            } else {
                index += 1;
            }
        }
        self.release();
        events
    }

    /// Apply one reply. A reply for no pending request, or under another
    /// grant, authority or epoch, is rejected without changing state.
    pub fn accept(&mut self, reply: BrowseEnvelope) -> Result<Vec<BrowseEvent>, ProtocolError> {
        let index = self
            .pending
            .iter()
            .position(|pending| pending.request.request_id == reply.request_id)
            .ok_or(ProtocolError::UnexpectedResponse)?;
        reply.validate_reply(&self.pending[index].request)?;
        let mut events = Vec::new();
        let accepted = self.pending[index].progress.accept(&reply);
        if accepted.is_err() || matches!(reply.operation, BrowseOperation::Error { .. }) {
            let pending = self.pending.remove(index);
            self.release();
            self.fail(pending.purpose, &mut events);
            return Ok(events);
        }
        let pending = &mut self.pending[index];
        match &reply.operation {
            BrowseOperation::BrowseListed {
                entries, truncated, ..
            } => {
                pending.entries.extend(entries.iter().cloned());
                pending.truncated = *truncated;
            }
            BrowseOperation::BrowseGitResult { data, .. } => {
                pending.data.extend_from_slice(data.as_bytes());
            }
            _ => {}
        }
        if !pending.progress.is_done() {
            return Ok(events);
        }
        let pending = self.pending.remove(index);
        self.release();
        self.complete(pending, &reply.operation, &mut events);
        Ok(events)
    }

    fn send(
        &mut self,
        operation: BrowseOperation,
        purpose: BrowsePurpose,
    ) -> Result<u64, ProtocolError> {
        let allowed = match purpose {
            BrowsePurpose::Open => self.phase == BrowsePhase::Opening && self.next_id == 1,
            _ => matches!(self.phase, BrowsePhase::Opening | BrowsePhase::Open),
        };
        if !allowed {
            return Err(ProtocolError::UnexpectedResponse);
        }
        if self.queued.len() >= MAX_QUEUED_BROWSE {
            return Err(ProtocolError::Limit);
        }
        let id = self.next_id;
        let request = BrowseEnvelope::new(self.identity.clone(), id, operation);
        let progress = BrowseReplyProgress::new(request.clone())?;
        self.next_id += 1;
        self.queued.push_back(BrowsePending {
            request,
            progress,
            purpose,
            sent: Instant::now(),
            data: Vec::new(),
            entries: Vec::new(),
            truncated: false,
        });
        self.release();
        Ok(id)
    }

    /// Move waiting requests onto the wire, in order, while fewer than
    /// [`MAX_IN_FLIGHT`] are unanswered. At most one git read is on the wire:
    /// the server runs them one after another, so a waiting read would
    /// otherwise age behind slow ones. Listings and checks pass a waiting git
    /// read. A waiting read of a superseded git sequence is dropped unsent.
    /// The timeout starts here.
    fn release(&mut self) {
        let current = self.git.as_ref().map(|sequence| sequence.generation);
        let mut git_busy = self
            .pending
            .iter()
            .any(|pending| is_git_read(&pending.purpose));
        let mut index = 0;
        while self.pending.len() < MAX_IN_FLIGHT && index < self.queued.len() {
            let purpose = &self.queued[index].purpose;
            if git_generation(purpose).is_some_and(|g| Some(g) != current) {
                self.queued.remove(index);
                continue;
            }
            let git = is_git_read(purpose);
            if git && git_busy {
                index += 1;
                continue;
            }
            git_busy |= git;
            let Some(mut next) = self.queued.remove(index) else {
                return;
            };
            next.sent = Instant::now();
            self.outbox.push_back(next.request.clone());
            self.pending.push(next);
        }
    }

    fn git_read(
        &mut self,
        path: RemotePath,
        command: GitRead,
        purpose: BrowsePurpose,
    ) -> Result<u64, ProtocolError> {
        self.send(BrowseOperation::BrowseGitRead { path, command }, purpose)
    }

    fn current_git(&mut self, generation: u64) -> Option<&mut GitSequence> {
        self.git
            .as_mut()
            .filter(|sequence| sequence.generation == generation)
    }

    fn complete(
        &mut self,
        pending: BrowsePending,
        operation: &BrowseOperation,
        events: &mut Vec<BrowseEvent>,
    ) {
        match pending.purpose {
            BrowsePurpose::Open => {
                self.phase = BrowsePhase::Open;
                let home = match operation {
                    BrowseOperation::BrowseOpened { home } => home.clone(),
                    _ => None,
                };
                events.push(BrowseEvent::Opened { home });
            }
            BrowsePurpose::Close => {
                self.end();
                events.push(BrowseEvent::Ended);
            }
            BrowsePurpose::List(path) => {
                if !self.root.as_ref().is_some_and(|root| path.is_within(root)) {
                    return;
                }
                let folders = pending
                    .entries
                    .iter()
                    .filter(|entry| entry.kind == EntryKind::Directory)
                    .map(|entry| RemoteFolder {
                        path: entry.path.clone(),
                        name: crate::sanitize::sanitize_display(
                            &String::from_utf8_lossy(remote_name(&entry.path)),
                            255,
                        ),
                    })
                    .collect();
                events.push(BrowseEvent::Listed {
                    path,
                    folders,
                    truncated: pending.truncated,
                });
            }
            BrowsePurpose::Address(_) | BrowsePurpose::Launch(..) => {
                let BrowseOperation::BrowseChecked { result, .. } = operation else {
                    return self.fail(pending.purpose, events);
                };
                let result = check_result(result);
                events.push(match pending.purpose {
                    BrowsePurpose::Launch(kind, requested) => BrowseEvent::LaunchChecked {
                        kind,
                        requested,
                        result,
                    },
                    BrowsePurpose::Address(requested) => {
                        BrowseEvent::AddressChecked { requested, result }
                    }
                    _ => return,
                });
            }
            BrowsePurpose::Toplevel(generation) => {
                let toplevel = std::str::from_utf8(&pending.data)
                    .ok()
                    .map(str::trim)
                    .and_then(|text| RemotePath::new(text.as_bytes().to_vec()).ok());
                let Some(toplevel) = toplevel else {
                    return self.fail(pending.purpose, events);
                };
                let Some(sequence) = self.current_git(generation) else {
                    return;
                };
                sequence.toplevel = Some(toplevel);
                let target = sequence.target.clone();
                if self
                    .git_read(
                        target,
                        GitRead::WorktreeList,
                        BrowsePurpose::Worktrees(generation),
                    )
                    .is_err()
                {
                    self.fail(BrowsePurpose::Worktrees(generation), events);
                }
            }
            BrowsePurpose::Worktrees(generation) => {
                let entries = made_remote::git::parse_worktree_entries(&String::from_utf8_lossy(
                    &pending.data,
                ));
                let Some(sequence) = self.current_git(generation) else {
                    return;
                };
                let toplevel = sequence.toplevel.clone();
                // Distinct paths only, at most `MAX_WORKTREES`, in list order.
                let mut seen = HashSet::new();
                let mut truncated = false;
                let mut items: Vec<(WorktreeItem, Option<RemotePath>)> = Vec::new();
                for entry in &entries {
                    let Some(remote) = entry
                        .path
                        .to_str()
                        .and_then(|text| RemotePath::new(text.as_bytes().to_vec()).ok())
                    else {
                        continue;
                    };
                    let Some(path) = remote_path_buf(&remote) else {
                        continue;
                    };
                    if !seen.insert(path.clone()) {
                        continue;
                    }
                    if items.len() == MAX_WORKTREES {
                        truncated = true;
                        break;
                    }
                    items.push((
                        WorktreeItem {
                            path,
                            label: entry.head_label(),
                            missing: entry.missing,
                            counts: CountsState::Pending,
                        },
                        (!entry.missing).then_some(remote),
                    ));
                }
                if !items
                    .iter()
                    .any(|(_, remote)| remote.is_some() && *remote == toplevel)
                {
                    let Some(top) = toplevel.as_ref().and_then(remote_path_buf) else {
                        return self.fail(BrowsePurpose::Toplevel(generation), events);
                    };
                    items = vec![(
                        WorktreeItem {
                            path: top,
                            label: None,
                            missing: false,
                            counts: CountsState::Pending,
                        },
                        None,
                    )];
                }
                let sequence = self.current_git(generation).expect("current sequence");
                sequence.truncated = truncated;
                sequence.worktrees = items.iter().map(|(item, _)| item.clone()).collect();
                let checks: Vec<(usize, RemotePath)> = items
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, (_, remote))| Some((index, remote?)))
                    .collect();
                sequence.checks = checks.len();
                if checks.is_empty() {
                    return self.publish_git(generation, events);
                }
                for (index, path) in checks {
                    if self
                        .send(
                            BrowseOperation::BrowseCheck { path },
                            BrowsePurpose::WorktreeCheck(generation, index),
                        )
                        .is_err()
                    {
                        self.fail(BrowsePurpose::WorktreeCheck(generation, index), events);
                    }
                }
            }
            BrowsePurpose::WorktreeCheck(generation, _) => {
                let usable = matches!(
                    operation,
                    BrowseOperation::BrowseChecked {
                        result: DirectoryCheck::Directory { .. },
                        ..
                    }
                );
                if !usable {
                    return self.fail(pending.purpose, events);
                }
                let Some(sequence) = self.current_git(generation) else {
                    return;
                };
                sequence.checks = sequence.checks.saturating_sub(1);
                if sequence.checks == 0 {
                    self.publish_git(generation, events);
                }
            }
            BrowsePurpose::Status(generation, path) => {
                if self.current_git(generation).is_none() {
                    return;
                }
                let records =
                    made_remote::git::parse_porcelain_v1_z(&String::from_utf8_lossy(&pending.data));
                events.push(BrowseEvent::Counts {
                    generation,
                    path,
                    counts: CountsState::Ready(made_remote::git::ChangeCounts::from_records(
                        &records,
                    )),
                });
            }
        }
    }

    fn fail(&mut self, purpose: BrowsePurpose, events: &mut Vec<BrowseEvent>) {
        match purpose {
            BrowsePurpose::Open | BrowsePurpose::Close => {
                self.end();
                events.push(BrowseEvent::Ended);
            }
            BrowsePurpose::List(path) => {
                if self.root.as_ref().is_some_and(|root| path.is_within(root)) {
                    events.push(BrowseEvent::ListFailed { path });
                }
            }
            BrowsePurpose::Address(requested) => events.push(BrowseEvent::AddressChecked {
                requested,
                result: Err(AddressError::NotReadable),
            }),
            BrowsePurpose::Launch(kind, requested) => events.push(BrowseEvent::LaunchChecked {
                kind,
                requested,
                result: Err(AddressError::NotReadable),
            }),
            BrowsePurpose::Toplevel(generation) => {
                if self.current_git(generation).is_some() {
                    self.git = None;
                    events.push(BrowseEvent::Git {
                        generation,
                        state: GitState::NoRepository,
                    });
                }
            }
            BrowsePurpose::Worktrees(generation) => {
                // Without a list the working tree itself is the only item.
                let Some(top) = self
                    .current_git(generation)
                    .and_then(|sequence| sequence.toplevel.as_ref())
                    .and_then(remote_path_buf)
                else {
                    return;
                };
                let sequence = self.current_git(generation).expect("current sequence");
                sequence.worktrees = vec![WorktreeItem {
                    path: top,
                    label: None,
                    missing: false,
                    counts: CountsState::Pending,
                }];
                self.publish_git(generation, events);
            }
            BrowsePurpose::WorktreeCheck(generation, index) => {
                let Some(sequence) = self.current_git(generation) else {
                    return;
                };
                if let Some(item) = sequence.worktrees.get_mut(index) {
                    item.missing = true;
                }
                sequence.checks = sequence.checks.saturating_sub(1);
                if sequence.checks == 0 {
                    self.publish_git(generation, events);
                }
            }
            BrowsePurpose::Status(generation, path) => {
                if self.current_git(generation).is_some() {
                    events.push(BrowseEvent::Counts {
                        generation,
                        path,
                        counts: CountsState::Failed,
                    });
                }
            }
        }
    }

    /// Publish the repository state, then read the counts of every listed
    /// worktree when the target is a top level with linked worktrees, else of
    /// the working tree that contains the target.
    fn publish_git(&mut self, generation: u64, events: &mut Vec<BrowseEvent>) {
        let Some(sequence) = self.current_git(generation) else {
            return;
        };
        let Some(toplevel) = sequence.toplevel.as_ref().and_then(remote_path_buf) else {
            return;
        };
        let target = remote_path_buf(&sequence.target);
        let worktrees = sequence.worktrees.clone();
        let counted: Vec<PathBuf> = if Some(&toplevel) == target.as_ref() && worktrees.len() > 1 {
            worktrees
                .iter()
                .filter(|item| !item.missing)
                .map(|item| item.path.clone())
                .collect()
        } else {
            vec![toplevel.clone()]
        };
        events.push(BrowseEvent::Git {
            generation,
            state: GitState::Repository {
                toplevel,
                worktrees,
                counts: CountsState::Pending,
            },
        });
        for path in counted {
            let Some(remote) = path
                .to_str()
                .and_then(|text| RemotePath::new(text.as_bytes().to_vec()).ok())
            else {
                continue;
            };
            if self
                .git_read(
                    remote,
                    GitRead::Status,
                    BrowsePurpose::Status(generation, path.clone()),
                )
                .is_err()
            {
                self.fail(BrowsePurpose::Status(generation, path), events);
            }
        }
    }
}

/// Whether a request is a git read rather than a directory check.
fn is_git_read(purpose: &BrowsePurpose) -> bool {
    matches!(
        purpose,
        BrowsePurpose::Toplevel(_) | BrowsePurpose::Worktrees(_) | BrowsePurpose::Status(..)
    )
}

/// The git sequence a request belongs to, if any.
fn git_generation(purpose: &BrowsePurpose) -> Option<u64> {
    match purpose {
        BrowsePurpose::Toplevel(generation)
        | BrowsePurpose::Worktrees(generation)
        | BrowsePurpose::WorktreeCheck(generation, _)
        | BrowsePurpose::Status(generation, _) => Some(*generation),
        _ => None,
    }
}

fn remote_name(path: &RemotePath) -> &[u8] {
    let bytes = path.as_bytes();
    bytes
        .iter()
        .rposition(|byte| *byte == b'/')
        .map_or(bytes, |index| &bytes[index + 1..])
}

fn remote_path_buf(path: &RemotePath) -> Option<PathBuf> {
    crate::open_workspace::remote_key(path)
}

/// Pumps one [`RemoteBrowse`] over the companion's stdio stream. Every call
/// returns at once: writes are queued by the stream and reads are polled.
pub struct BrowseConnection<S> {
    stream: S,
    decoder: BrowseFrameDecoder,
    browse: RemoteBrowse,
    /// An encoded frame the stream refused with `WouldBlock`; written first
    /// on the next pump.
    unwritten: Option<Vec<u8>>,
    end_failure: Option<SshFailureClass>,
}

impl<S: made_remote::client::ClientStream> BrowseConnection<S> {
    /// Pump `browse` over `stream`, whose companion header is already written.
    pub fn new(stream: S, browse: RemoteBrowse) -> Self {
        Self {
            stream,
            decoder: BrowseFrameDecoder::new(),
            browse,
            unwritten: None,
            end_failure: None,
        }
    }

    /// Why the transport ended the grant, when SSH reported a failure: the
    /// same classification as the connection test, so a changed host key is
    /// [`SshFailureClass::ChangedHostKey`]. `None` while running and after a
    /// plain end (close, end of stream, protocol error or timeout).
    pub fn end_failure(&self) -> Option<SshFailureClass> {
        self.end_failure
    }

    /// The transport's next interactive SSH prompt. While it awaits the user
    /// the open request does not time out.
    pub fn poll_prompt(&mut self) -> Option<PromptRequest> {
        let prompt = self.stream.poll_prompt();
        if prompt.is_some() {
            self.browse.pause_open_timeout(true);
        }
        prompt
    }

    /// Answer (or, with `None`, cancel) prompt `id`; the open request's
    /// timeout resumes with a fresh window.
    pub fn answer_prompt(&mut self, id: u64, answer: Option<PromptAnswer>) -> std::io::Result<()> {
        self.browse.pause_open_timeout(false);
        self.stream.answer_prompt(id, answer)
    }

    pub fn browse(&self) -> &RemoteBrowse {
        &self.browse
    }

    pub fn browse_mut(&mut self) -> &mut RemoteBrowse {
        &mut self.browse
    }

    /// The transport, for answering its interactive SSH prompts.
    pub fn stream_mut(&mut self) -> &mut S {
        &mut self.stream
    }

    /// Write queued frames, apply the replies received so far, up to a
    /// per-pump budget, and expire stalled requests. A frame the stream
    /// cannot take yet is retried on the next pump. Malformed or foreign
    /// replies are discarded; a broken stream ends the grant only.
    pub fn pump(&mut self, now: Instant) -> Vec<BrowseEvent> {
        let mut events = Vec::new();
        if self.browse.phase() == BrowsePhase::Ended {
            return events;
        }
        loop {
            let bytes = match self.unwritten.take() {
                Some(bytes) => bytes,
                None => match self.browse.next_frame() {
                    Some(frame) => match made_remote::protocol::encode_browse(&frame) {
                        Ok(bytes) => bytes,
                        Err(_) => return self.end(events),
                    },
                    None => break,
                },
            };
            match self.stream.write(&bytes) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    self.unwritten = Some(bytes);
                    break;
                }
                Err(_) => return self.end(events),
            }
        }
        let (mut frames, mut read) = (0, 0);
        while frames < PUMP_FRAME_BUDGET && read < PUMP_BYTE_BUDGET {
            let Some(event) = self.stream.poll() else {
                break;
            };
            match event {
                made_platform::ssh::SshEvent::Data(bytes) => {
                    read += bytes.len();
                    let mut rest = bytes.as_slice();
                    while !rest.is_empty() {
                        match self.decoder.push(rest) {
                            Ok((used, frame)) => {
                                rest = &rest[used..];
                                if let Some(frame) = frame {
                                    frames += 1;
                                    if let Ok(found) = self.browse.accept(frame) {
                                        events.extend(found);
                                    }
                                }
                                if used == 0 {
                                    break;
                                }
                            }
                            Err(_) => return self.end(events),
                        }
                    }
                    if self.browse.phase() == BrowsePhase::Ended {
                        self.stream.detach();
                        return events;
                    }
                }
                made_platform::ssh::SshEvent::Failed(failure) => {
                    self.end_failure = Some(crate::remote_session::class_from_kind(failure.kind()));
                    return self.end(events);
                }
                _ => return self.end(events),
            }
        }
        events.extend(self.browse.expire(now));
        events
    }

    fn end(&mut self, mut events: Vec<BrowseEvent>) -> Vec<BrowseEvent> {
        self.browse.end();
        self.stream.detach();
        events.push(BrowseEvent::Ended);
        events
    }
}
