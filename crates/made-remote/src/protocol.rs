//! Versioned, authority-bound SSH stdio protocol. No host path conversion API.
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

pub const PROTOCOL_MAJOR: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 256 * 1024;
pub const MAX_CHUNK_BYTES: usize = 64 * 1024;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_UPLOAD_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_PREVIEW_BYTES: u64 = made_explorer::preview::PREVIEW_SIZE_LIMIT;
pub const MAX_IN_FLIGHT: usize = 16;
pub const MAX_QUEUED_CHUNKS: usize = 256;
pub const MAX_LIST_BATCH: usize = 128;
pub const MAX_DIRECTORY_ENTRIES: usize = made_explorer::tree::MAX_DIR_ENTRIES;
pub const MAX_TREE_ROWS: usize = made_explorer::tree::MAX_TREE_ROWS;
pub const MAX_TREE_DEPTH: usize = made_explorer::tree::MAX_TREE_DEPTH;
pub const MAX_GIT_BYTES: u64 = 2 * 1024 * 1024;
pub const RPC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
pub const HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(120);
pub const DETACHED_LEASE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
pub const CLOSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ProtocolError {
    #[error("invalid remote POSIX path")]
    Path,
    #[error("protocol bound exceeded")]
    Limit,
    #[error("unsupported protocol version")]
    Version,
    #[error("invalid or stale routing identity")]
    Identity,
    #[error("malformed protocol frame")]
    Malformed,
    #[error("unexpected response operation")]
    UnexpectedResponse,
    #[error("incomplete or out-of-order transfer")]
    Transfer,
    #[error("secure randomness unavailable")]
    Random,
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Vec<u8>", into = "Vec<u8>")]
pub struct RemotePath(Vec<u8>);
impl RemotePath {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, ProtocolError> {
        let bytes = bytes.into();
        if bytes.len() > MAX_PATH_BYTES
            || bytes.first() != Some(&b'/')
            || bytes.contains(&0)
            || (bytes.len() > 1
                && bytes[1..]
                    .split(|b| *b == b'/')
                    .any(|s| s.is_empty() || s == b"." || s == b".."))
        {
            return Err(ProtocolError::Path);
        }
        Ok(Self(bytes))
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn to_utf8(&self) -> Result<&str, ProtocolError> {
        std::str::from_utf8(&self.0).map_err(|_| ProtocolError::Path)
    }
    pub fn is_within(&self, root: &Self) -> bool {
        self == root
            || (self.0.starts_with(&root.0)
                && (root.0 == b"/" || self.0.get(root.0.len()) == Some(&b'/')))
    }
    pub fn is_direct_child_of(&self, root: &Self) -> bool {
        if self == root || !self.is_within(root) {
            return false;
        }
        let start = if root.0 == b"/" { 1 } else { root.0.len() + 1 };
        !self.0[start..].contains(&b'/')
    }
}
impl TryFrom<Vec<u8>> for RemotePath {
    type Error = ProtocolError;
    fn try_from(v: Vec<u8>) -> Result<Self, Self::Error> {
        Self::new(v)
    }
}
impl From<RemotePath> for Vec<u8> {
    fn from(v: RemotePath) -> Self {
        v.0
    }
}
impl fmt::Debug for RemotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RemotePath(<{} bytes>)", self.0.len())
    }
}

/// Random immutable saved-server identity; never derived from a display name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuthorityId(pub [u8; 16]);
impl AuthorityId {
    pub fn random() -> Result<Self, ProtocolError> {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|_| ProtocolError::Random)?;
        Ok(Self(bytes))
    }
}

/// Per-live-tab bearer capability. Never persist on the desktop or use in argv.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TabCapability([u8; 32]);
impl TabCapability {
    pub fn random() -> Result<Self, ProtocolError> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| ProtocolError::Random)?;
        Ok(Self(bytes))
    }
    /// Constant-work comparison for the privileged authentication boundary.
    pub fn authenticates(&self, other: &Self) -> bool {
        self.0
            .iter()
            .zip(other.0)
            .fold(0, |diff, (a, b)| diff | (a ^ b))
            == 0
    }
}
impl fmt::Debug for TabCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TabCapability(<redacted>)")
    }
}

/// All routing fields must match before any result is disclosed/applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub authority: AuthorityId,
    pub tab: TabCapability,
    pub epoch: u64,
    pub generation: u64,
}
impl Identity {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.epoch == 0 || self.generation == 0 {
            Err(ProtocolError::Identity)
        } else {
            Ok(())
        }
    }
    pub fn matches(&self, expected: &Self) -> bool {
        self.tab.authenticates(&expected.tab)
            && self.authority == expected.authority
            && self.epoch == expected.epoch
            && self.generation == expected.generation
    }
}

/// Base64 on the wire; bounded before decode allocation. Debug never emits data.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Chunk(Vec<u8>);
impl Chunk {
    pub fn new(bytes: Vec<u8>) -> Result<Self, ProtocolError> {
        if bytes.len() > MAX_CHUNK_BYTES {
            return Err(ProtocolError::Limit);
        }
        Ok(Self(bytes))
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}
impl TryFrom<String> for Chunk {
    type Error = ProtocolError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        use base64::Engine;
        if value.len() > MAX_CHUNK_BYTES.div_ceil(3) * 4 {
            return Err(ProtocolError::Limit);
        }
        Self::new(
            base64::engine::general_purpose::STANDARD
                .decode(value)
                .map_err(|_| ProtocolError::Malformed)?,
        )
    }
}
impl From<Chunk> for String {
    fn from(value: Chunk) -> Self {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(value.0)
    }
}
impl fmt::Debug for Chunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Chunk(<{} bytes>)", self.0.len())
    }
}

/// Wire selection of the existing eight read-only Git operations, not argv.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitRead {
    ShowToplevel,
    Branch,
    GitDir,
    CommonDir,
    WorktreeList,
    CommitId,
    CommitSubject,
    Status,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchTarget {
    Shell,
    OpenCode,
    Grok,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    Directory,
    File,
    Symlink,
    Other,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryEntry {
    pub path: RemotePath,
    pub kind: EntryKind,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreviewKind {
    Text,
    Binary,
    InvalidUtf8,
    TooLarge,
    Missing,
    Unreadable,
    NotRegular,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    Unsupported,
    Unavailable,
    Unauthorized,
    InvalidRequest,
    Stale,
    Cancelled,
    Timeout,
    Limit,
    SessionLost,
    CleanupUnconfirmed,
    Io,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteTarget {
    LinuxX64,
    LinuxArm64,
    MacosX64,
    MacosArm64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub target: RemoteTarget,
    pub tmux: bool,
    pub opencode: bool,
    pub shell_cwd: bool,
    pub helper_version: String,
    /// Additive within major 1: an older companion frame without these fields
    /// deserializes as false, so absence stays honest inventory tier. Detection
    /// is remote-only and never implies launch or install support.
    #[serde(default)]
    pub grok: bool,
    /// Frozen within major 1, like `tmux`: always encoded false and ignored by
    /// every consumer. No interpreter is a prerequisite for any Grok tier, so
    /// it is never probed; the field stays only for wire compatibility.
    #[serde(default)]
    pub python3: bool,
}

/// Bounded Grok activity vocabulary shared by snapshots and events; mirrors the
/// shipped grok-made hook statuses (`idle`, `working`, `attention`, `error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GrokActivity {
    Idle,
    Working,
    Attention,
    Error,
}

/// Closed classification of one Grok event, carrying what the local Grok Build
/// contract needs: which session, what happened, and the resulting activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GrokEventKind {
    SessionStart,
    Prompt,
    Stop,
    Error,
    Cancel,
    SessionEnd,
}

/// One owned remote Grok session in a revisioned snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrokSessionInfo {
    pub session_id: String,
    /// Absolute remote POSIX directory, byte-validated by `RemotePath`. Never
    /// canonicalized or opened by either side; root authorization is the
    /// owning tab's, like any other remote report.
    #[serde(default)]
    pub cwd: Option<RemotePath>,
    /// Companion-assigned attachment-order key. Only relative ordering within
    /// one snapshot is contractual; never wall-clock or cross-epoch semantics.
    pub opened_at: u64,
    pub activity: GrokActivity,
}

/// Complete revisioned Grok state for one tab: inventory plus activity. Grok
/// state never rides the OpenCode `SessionSnapshot` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrokSessionSnapshot {
    /// Monotonic revision within the current connection epoch. Must be positive.
    pub revision: u64,
    /// Whether the tab's managed child is a living Grok process, even before
    /// any session exists.
    pub running: bool,
    /// Whether the companion armed the installed plugin's status hooks. Older
    /// companions omit this additive field and retain inventory-only behavior.
    #[serde(default)]
    pub hooks_ready: bool,
    pub sessions: Vec<GrokSessionInfo>,
}
impl GrokSessionSnapshot {
    /// Wire-shape validation only. Envelope authority/tab/epoch/request and
    /// root-generation binding stay with `WireEnvelope`, and revision
    /// progression is the client's, never re-checked here.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.revision == 0 || self.sessions.len() > made_bridge::state::MAX_SESSIONS {
            return Err(ProtocolError::Malformed);
        }
        let mut seen = std::collections::HashSet::with_capacity(self.sessions.len());
        for session in &self.sessions {
            if !bounded_session_id(&session.session_id) || !seen.insert(session.session_id.as_str())
            {
                return Err(ProtocolError::Malformed);
            }
        }
        Ok(())
    }
}

/// One server-global Grok event the companion has filtered to sessions owned by
/// this tab's managed child, pushed unsolicited under the current attach id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrokSessionEvent {
    pub kind: GrokEventKind,
    pub session_id: String,
    /// Absolute remote POSIX directory for explorer follow, when the event
    /// carries one. Absent keeps the current root.
    #[serde(default)]
    pub cwd: Option<RemotePath>,
    /// The activity dot implied by this event; `SessionEnd` removes the row.
    pub activity: GrokActivity,
}

/// Closed verbs and their replies. Content-bearing Debug is deliberately opaque.
/// Each streamed result keeps the request id, identity and an explicit final bit.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", content = "data", deny_unknown_fields)]
pub enum Operation {
    Capabilities,
    CapabilitiesResult {
        capabilities: Capabilities,
    },
    Open {
        directory: RemotePath,
        target: LaunchTarget,
        rows: u16,
        cols: u16,
    },
    Attach {
        rows: u16,
        cols: u16,
    },
    Attached,
    Close,
    Closed {
        confirmed: bool,
    },
    Heartbeat,
    Ack,
    Cancel {
        request_id: u64,
    },
    RootValidate {
        path: RemotePath,
    },
    RootSelect {
        path: RemotePath,
    },
    RootResult {
        requested: RemotePath,
        path: RemotePath,
        generation: u64,
    },
    List {
        path: RemotePath,
    },
    Listed {
        path: RemotePath,
        entries: Vec<DirectoryEntry>,
        truncated: bool,
        done: bool,
    },
    Preview {
        path: RemotePath,
    },
    Refresh {
        path: RemotePath,
    },
    PreviewResult {
        path: RemotePath,
        kind: PreviewKind,
        offset: u64,
        data: Chunk,
        done: bool,
    },
    Watch {
        path: RemotePath,
    },
    Rescan {
        path: RemotePath,
    },
    Changed {
        path: RemotePath,
    },
    GitRead {
        command: GitRead,
    },
    GitResult {
        command: GitRead,
        offset: u64,
        data: Chunk,
        done: bool,
    },
    SessionState,
    SessionSnapshot {
        snapshot: made_bridge::state::SessionSnapshot,
    },
    Focus {
        session_id: String,
    },
    FocusResult {
        session_id: String,
        accepted: bool,
    },
    GrokState,
    GrokSnapshot {
        snapshot: GrokSessionSnapshot,
    },
    GrokEvent {
        event: GrokSessionEvent,
    },
    ShellCwd {
        path: RemotePath,
    },
    TerminalInput {
        data: Chunk,
    },
    TerminalOutput {
        data: Chunk,
        /// Required. Missing is malformed, not completion. `false` frames must
        /// carry bytes. The single `true` frame ends a resnapshot and may be empty.
        done: bool,
    },
    TerminalResize {
        rows: u16,
        cols: u16,
    },
    TerminalExit {
        code: u32,
    },
    UploadBegin {
        upload_id: u64,
        total: u64,
    },
    UploadChunk {
        upload_id: u64,
        offset: u64,
        data: Chunk,
    },
    UploadCommit {
        upload_id: u64,
    },
    UploadResult {
        upload_id: u64,
        path: RemotePath,
    },
    UploadAbort {
        upload_id: u64,
    },
    Error {
        code: ErrorCode,
    },
}
impl fmt::Debug for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Operation(<redacted>)")
    }
}

/// Session-identifier shape shared by OpenCode focus and the Grok surface:
/// non-empty, bounded in characters, no control bytes (NUL included).
fn bounded_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && session_id.chars().count() <= made_bridge::state::MAX_SESSION_ID_CHARS
        && !session_id.chars().any(char::is_control)
}

impl Operation {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        match self {
            Self::Open { rows, cols, .. }
            | Self::Attach { rows, cols }
            | Self::TerminalResize { rows, cols }
                if *rows == 0 || *cols == 0 =>
            {
                return Err(ProtocolError::Limit)
            }
            Self::Cancel { request_id: 0 }
            | Self::RootResult { generation: 0, .. }
            | Self::UploadBegin { upload_id: 0, .. }
            | Self::UploadChunk { upload_id: 0, .. }
            | Self::UploadCommit { upload_id: 0 }
            | Self::UploadAbort { upload_id: 0 }
            | Self::UploadResult { upload_id: 0, .. } => return Err(ProtocolError::Identity),
            Self::UploadBegin { total, .. } if *total == 0 || *total > MAX_UPLOAD_BYTES => {
                return Err(ProtocolError::Limit)
            }
            Self::UploadChunk { offset, data, .. } => {
                if offset
                    .checked_add(data.0.len() as u64)
                    .is_none_or(|n| n > MAX_UPLOAD_BYTES)
                {
                    return Err(ProtocolError::Limit);
                }
            }
            Self::PreviewResult {
                kind,
                offset,
                data,
                done,
                ..
            } => {
                if offset
                    .checked_add(data.0.len() as u64)
                    .is_none_or(|n| n > MAX_PREVIEW_BYTES)
                    || (*kind != PreviewKind::Text && (!data.0.is_empty() || *offset != 0 || !done))
                {
                    return Err(ProtocolError::Limit);
                }
                // UTF-8 sequences may span frames. ReplyProgress validates the
                // complete byte stream incrementally, including its final tail.
                let sniff = made_explorer::preview::BINARY_SNIFF_LEN;
                if *kind == PreviewKind::Text && *offset < sniff as u64 {
                    let start = usize::try_from(*offset).map_err(|_| ProtocolError::Limit)?;
                    let n = (sniff - start).min(data.0.len());
                    if data.0[..n].contains(&0) {
                        return Err(ProtocolError::Malformed);
                    }
                }
            }
            Self::GitResult { offset, data, .. }
                if offset
                    .checked_add(data.0.len() as u64)
                    .is_none_or(|n| n > MAX_GIT_BYTES) =>
            {
                return Err(ProtocolError::Limit)
            }
            Self::Listed { path, entries, .. } => {
                if entries.len() > MAX_LIST_BATCH {
                    return Err(ProtocolError::Limit);
                }
                let mut seen = std::collections::HashSet::new();
                for entry in entries {
                    if !entry.path.is_direct_child_of(path) || !seen.insert(&entry.path) {
                        return Err(ProtocolError::Path);
                    }
                }
            }
            Self::SessionSnapshot { snapshot } => snapshot
                .validate_with_path_semantics(made_bridge::state::PathSemantics::RemotePosix)
                .map_err(|_| ProtocolError::Malformed)?,
            Self::Focus { session_id } | Self::FocusResult { session_id, .. } => {
                if !bounded_session_id(session_id) {
                    return Err(ProtocolError::Malformed);
                }
            }
            Self::GrokSnapshot { snapshot } => {
                snapshot.validate().map_err(|_| ProtocolError::Malformed)?
            }
            Self::GrokEvent { event } => {
                if !bounded_session_id(&event.session_id) {
                    return Err(ProtocolError::Malformed);
                }
            }
            Self::UploadResult { path, .. } => {
                if path.to_utf8()?.chars().any(char::is_control) {
                    return Err(ProtocolError::Path);
                }
            }
            Self::CapabilitiesResult { capabilities }
                if capabilities.helper_version.is_empty()
                    || capabilities.helper_version.len() > 128
                    || capabilities.helper_version.chars().any(char::is_control) =>
            {
                return Err(ProtocolError::Malformed);
            }
            // No serde default: a missing bit never deserializes as false.
            // An empty non-final frame is not a resnapshot end.
            Self::TerminalOutput { data, done: false } if data.as_bytes().is_empty() => {
                return Err(ProtocolError::Malformed);
            }
            _ => {}
        }
        Ok(())
    }
    /// True only for interactive terminal traffic; never send through the RPC
    /// retry queue. Unknown delivery after a break is not exactly-once delivery.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::TerminalInput { .. }
                | Self::TerminalOutput { .. }
                | Self::TerminalResize { .. }
                | Self::TerminalExit { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireEnvelope {
    pub major: u16,
    pub identity: Identity,
    pub request_id: u64,
    pub operation: Operation,
}
impl WireEnvelope {
    pub fn new(identity: Identity, request_id: u64, operation: Operation) -> Self {
        Self {
            major: PROTOCOL_MAJOR,
            identity,
            request_id,
            operation,
        }
    }
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.major != PROTOCOL_MAJOR {
            return Err(ProtocolError::Version);
        }
        self.identity.validate()?;
        if self.request_id == 0 {
            return Err(ProtocolError::Identity);
        }
        self.operation.validate()
    }
    /// Client request confinement. An outside-root transition needs an exact
    /// grant from the owning tab's explicit selection/validated cwd/session
    /// report. This is lexical authorization, NOT remote filesystem attestation;
    /// the server must still use PathGuard on the actual filesystem.
    pub fn validate_request(
        &self,
        root: &RemotePath,
        root_grant: Option<&RemotePath>,
    ) -> Result<(), ProtocolError> {
        self.validate()?;
        use Operation::*;
        match &self.operation {
            Open {
                directory: path, ..
            }
            | RootValidate { path }
            | RootSelect { path } => {
                if !path.is_within(root) && root_grant != Some(path) {
                    return Err(ProtocolError::Path);
                }
            }
            List { path }
            | Preview { path }
            | Refresh { path }
            | Watch { path }
            | Rescan { path } => {
                if !path.is_within(root) {
                    return Err(ProtocolError::Path);
                }
            }
            Capabilities
            | Attach { .. }
            | Close
            | Heartbeat
            | Cancel { .. }
            | GitRead { .. }
            | SessionState
            | Focus { .. }
            | GrokState
            | TerminalInput { .. }
            | TerminalResize { .. }
            | UploadBegin { .. }
            | UploadChunk { .. }
            | UploadCommit { .. }
            | UploadAbort { .. } => {}
            _ => return Err(ProtocolError::UnexpectedResponse),
        }
        Ok(())
    }
    /// Validate ONE frame's routing and reply shape/path. This does not validate
    /// stream offsets, aggregate bounds, cross-batch uniqueness, completion or
    /// UTF-8 across chunks. Consumers MUST use ReplyProgress for each request
    /// before accepting/accumulating replies. Neither API attests provenance.
    pub fn validate_reply(&self, request: &Self) -> Result<(), ProtocolError> {
        self.validate()?;
        request.validate()?;
        if !self.identity.matches(&request.identity) || self.request_id != request.request_id {
            return Err(ProtocolError::Identity);
        }
        use Operation::*;
        let valid = match (&request.operation, &self.operation) {
            (_, Error { .. }) => true,
            (Capabilities, CapabilitiesResult { .. })
            | (Open { .. } | Attach { .. }, Attached)
            | (Close, Closed { .. })
            | (
                Heartbeat
                | Cancel { .. }
                | TerminalResize { .. }
                | TerminalInput { .. }
                | UploadBegin { .. }
                | UploadChunk { .. }
                | UploadAbort { .. },
                Ack,
            )
            | (SessionState, SessionSnapshot { .. })
            | (GrokState, GrokSnapshot { .. }) => true,
            (
                RootValidate { path: a },
                RootResult {
                    requested: b,
                    generation,
                    ..
                },
            ) => a == b && *generation == request.identity.generation,
            (
                RootSelect { path: a },
                RootResult {
                    requested: b,
                    generation,
                    ..
                },
            ) => a == b && *generation > request.identity.generation,
            (List { path: a }, Listed { path: b, .. })
            | (Preview { path: a } | Refresh { path: a }, PreviewResult { path: b, .. })
            | (Watch { path: a } | Rescan { path: a }, Changed { path: b }) => a == b,
            (GitRead { command: a }, GitResult { command: b, .. }) => a == b,
            (Focus { session_id: a }, FocusResult { session_id: b, .. }) => a == b,
            (UploadCommit { upload_id: a }, UploadResult { upload_id: b, .. }) => a == b,
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(ProtocolError::UnexpectedResponse)
        }
    }
}

/// Four-byte big-endian length + UTF-8 JSON. Allocation is bounded by the prefix.
pub fn encode(frame: &WireEnvelope) -> Result<Vec<u8>, ProtocolError> {
    frame.validate()?;
    let payload = serde_json::to_vec(frame).map_err(|_| ProtocolError::Malformed)?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::Limit);
    }
    let length = u32::try_from(payload.len()).map_err(|_| ProtocolError::Limit)?;
    let mut bytes = length.to_be_bytes().to_vec();
    bytes.extend(payload);
    Ok(bytes)
}

/// Incremental one-frame-at-a-time parser. Feed only bytes consumed by `push`;
/// drain each returned frame before feeding the remaining input. Any error is
/// terminal for this transport: discard the decoder and disconnect.
#[derive(Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
    length: Option<usize>,
}
impl fmt::Debug for FrameDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameDecoder")
            .field("buffered_bytes", &self.buffer.len())
            .field("expected_bytes", &self.length)
            .finish()
    }
}
impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<(usize, Option<WireEnvelope>), ProtocolError> {
        let mut consumed = 0;
        if self.length.is_none() {
            let n = (4 - self.buffer.len()).min(bytes.len());
            self.buffer.extend_from_slice(&bytes[..n]);
            consumed += n;
            if self.buffer.len() < 4 {
                return Ok((consumed, None));
            }
            let length = u32::from_be_bytes(
                self.buffer[..4]
                    .try_into()
                    .map_err(|_| ProtocolError::Malformed)?,
            ) as usize;
            if length == 0 {
                return Err(ProtocolError::Malformed);
            }
            if length > MAX_FRAME_BYTES {
                return Err(ProtocolError::Limit);
            }
            self.buffer.clear();
            self.length = Some(length);
        }
        let length = self.length.ok_or(ProtocolError::Malformed)?;
        let n = (length - self.buffer.len()).min(bytes.len() - consumed);
        self.buffer
            .extend_from_slice(&bytes[consumed..consumed + n]);
        consumed += n;
        if self.buffer.len() != length {
            return Ok((consumed, None));
        }
        let frame: WireEnvelope =
            serde_json::from_slice(&self.buffer).map_err(|_| ProtocolError::Malformed)?;
        frame.validate()?;
        self.buffer.clear();
        self.length = None;
        Ok((consumed, Some(frame)))
    }
    pub fn finish(&self) -> Result<(), ProtocolError> {
        if self.buffer.is_empty() && self.length.is_none() {
            Ok(())
        } else {
            Err(ProtocolError::Malformed)
        }
    }
}

/// Bounded validation state for one correlated response stream. Owns only the
/// original request, at most MAX_DIRECTORY_ENTRIES unique path keys and three
/// pending UTF-8 bytes, never an accumulated preview/Git payload. Consumers may
/// append accepted bytes under these caps, but must wait for successful finish
/// before treating preview text as a complete UTF-8 document. Any accept error
/// poisons this instance; discard it and the partial result, never retry a frame.
pub struct ReplyProgress {
    request: WireEnvelope,
    entries: std::collections::HashSet<RemotePath>,
    bytes: u64,
    preview_kind: Option<PreviewKind>,
    utf8_tail: Vec<u8>,
    done: bool,
    failed: bool,
    truncated: bool,
}
impl fmt::Debug for ReplyProgress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplyProgress")
            .field("entries", &self.entries.len())
            .field("bytes", &self.bytes)
            .field("pending_utf8_bytes", &self.utf8_tail.len())
            .field("done", &self.done)
            .field("failed", &self.failed)
            .field("truncated", &self.truncated)
            .finish()
    }
}
impl ReplyProgress {
    /// The request must be a request verb, not an event or reply. Root grants
    /// and current-tab authorization are still checked by validate_request.
    pub fn new(request: WireEnvelope) -> Result<Self, ProtocolError> {
        request.validate()?;
        use Operation::*;
        if !matches!(
            &request.operation,
            Capabilities
                | Open { .. }
                | Attach { .. }
                | Close
                | Heartbeat
                | Cancel { .. }
                | RootValidate { .. }
                | RootSelect { .. }
                | List { .. }
                | Preview { .. }
                | Refresh { .. }
                | Watch { .. }
                | Rescan { .. }
                | GitRead { .. }
                | SessionState
                | Focus { .. }
                | GrokState
                | TerminalInput { .. }
                | TerminalResize { .. }
                | UploadBegin { .. }
                | UploadChunk { .. }
                | UploadCommit { .. }
                | UploadAbort { .. }
        ) {
            return Err(ProtocolError::UnexpectedResponse);
        }
        Ok(Self {
            request,
            entries: std::collections::HashSet::new(),
            bytes: 0,
            preview_kind: None,
            utf8_tail: Vec::new(),
            done: false,
            failed: false,
            truncated: false,
        })
    }

    pub fn accept(&mut self, reply: &WireEnvelope) -> Result<(), ProtocolError> {
        if self.done || self.failed {
            self.failed = true;
            return Err(ProtocolError::Transfer);
        }
        let result = self.accept_inner(reply);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn accept_inner(&mut self, reply: &WireEnvelope) -> Result<(), ProtocolError> {
        reply.validate_reply(&self.request)?;
        match &reply.operation {
            Operation::Listed {
                entries,
                truncated,
                done,
                ..
            } => {
                if (!done && (entries.is_empty() || *truncated))
                    || self
                        .entries
                        .len()
                        .checked_add(entries.len())
                        .is_none_or(|n| n > MAX_DIRECTORY_ENTRIES)
                {
                    return Err(ProtocolError::Limit);
                }
                for entry in entries {
                    if !self.entries.insert(entry.path.clone()) {
                        return Err(ProtocolError::Path);
                    }
                }
                self.truncated = *truncated;
                self.done = *done;
            }
            Operation::PreviewResult {
                kind,
                offset,
                data,
                done,
                ..
            } => {
                if self.preview_kind.is_some_and(|previous| previous != *kind) {
                    return Err(ProtocolError::UnexpectedResponse);
                }
                self.check_chunk(*offset, data, *done, MAX_PREVIEW_BYTES)?;
                if *kind == PreviewKind::Text {
                    self.check_utf8(data.as_bytes(), *done)?;
                }
                self.preview_kind = Some(*kind);
                self.bytes += data.as_bytes().len() as u64;
                self.done = *done;
            }
            Operation::GitResult {
                offset, data, done, ..
            } => {
                self.check_chunk(*offset, data, *done, MAX_GIT_BYTES)?;
                self.bytes += data.as_bytes().len() as u64;
                self.done = *done;
            }
            // Unary replies (including errors) terminate exactly once. A Watch
            // subscription's subsequent notifications are events, not additional
            // replies to this completed request; route/authorize them separately.
            _ => self.done = true,
        }
        Ok(())
    }

    fn check_chunk(
        &self,
        offset: u64,
        data: &Chunk,
        done: bool,
        cap: u64,
    ) -> Result<(), ProtocolError> {
        if offset != self.bytes || (!done && data.as_bytes().is_empty()) {
            return Err(ProtocolError::Transfer);
        }
        if self
            .bytes
            .checked_add(data.as_bytes().len() as u64)
            .is_none_or(|n| n > cap)
        {
            return Err(ProtocolError::Limit);
        }
        Ok(())
    }

    fn check_utf8(&mut self, data: &[u8], done: bool) -> Result<(), ProtocolError> {
        // The temporary allocation is at most MAX_CHUNK_BYTES + 3. Keeping only
        // the incomplete suffix also rejects invalid overlong/surrogate input.
        let mut combined = std::mem::take(&mut self.utf8_tail);
        combined.extend_from_slice(data);
        match std::str::from_utf8(&combined) {
            Ok(_) => {}
            Err(error) if error.error_len().is_none() && !done => {
                self.utf8_tail
                    .extend_from_slice(&combined[error.valid_up_to()..]);
                if self.utf8_tail.len() > 3 {
                    return Err(ProtocolError::Malformed);
                }
            }
            Err(_) => return Err(ProtocolError::Malformed),
        }
        Ok(())
    }

    /// Call at end-of-response/EOF; absence of a final reply is not success.
    pub fn finish(&self) -> Result<(), ProtocolError> {
        if self.done && !self.failed && self.utf8_tail.is_empty() {
            Ok(())
        } else {
            Err(ProtocolError::Transfer)
        }
    }
    pub fn is_done(&self) -> bool {
        self.done && !self.failed
    }
    pub fn received_bytes(&self) -> u64 {
        self.bytes
    }
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }
}

/// Per-upload ordered byte accounting, independent of filesystem policy.
#[derive(Debug)]
pub struct UploadProgress {
    total: u64,
    received: u64,
}
impl UploadProgress {
    pub fn new(total: u64) -> Result<Self, ProtocolError> {
        if total == 0 || total > MAX_UPLOAD_BYTES {
            return Err(ProtocolError::Limit);
        }
        Ok(Self { total, received: 0 })
    }
    pub fn append(&mut self, offset: u64, bytes: &Chunk) -> Result<(), ProtocolError> {
        if offset != self.received || bytes.0.is_empty() {
            return Err(ProtocolError::Transfer);
        }
        let end = offset
            .checked_add(bytes.0.len() as u64)
            .filter(|end| *end <= self.total)
            .ok_or(ProtocolError::Transfer)?;
        self.received = end;
        Ok(())
    }
    pub fn commit(&self) -> Result<(), ProtocolError> {
        if self.received == self.total {
            Ok(())
        } else {
            Err(ProtocolError::Transfer)
        }
    }
    pub fn received(&self) -> u64 {
        self.received
    }
    pub fn total(&self) -> u64 {
        self.total
    }
}

/// Asynchronous, nonblocking root port. Implementation enqueues; poll is the
/// completion boundary. Remote implementations never consult host filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootRequest {
    pub identity: Identity,
    pub request_id: u64,
    pub path: RemotePath,
    pub select: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootResolution {
    pub identity: Identity,
    pub request_id: u64,
    pub result: Result<RemotePath, ErrorCode>,
    pub generation: u64,
}
impl RootRequest {
    pub fn into_envelope(self) -> WireEnvelope {
        let operation = if self.select {
            Operation::RootSelect { path: self.path }
        } else {
            Operation::RootValidate { path: self.path }
        };
        WireEnvelope::new(self.identity, self.request_id, operation)
    }
}
pub trait RootResolver: Send {
    fn resolve(&mut self, request: RootRequest) -> Result<(), ProtocolError>;
    fn poll(&mut self) -> Option<RootResolution>;
    fn cancel(&mut self, request_id: u64);
}
/// Same correlated asynchronous port for state/focus; no local BridgeHandle is
/// constructed for a remote tab. The remote runtime owns that handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCommand {
    State,
    Focus { session_id: String },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCommandRequest {
    pub identity: Identity,
    pub request_id: u64,
    pub command: SessionCommand,
}
impl SessionCommandRequest {
    pub fn into_envelope(self) -> WireEnvelope {
        let operation = match self.command {
            SessionCommand::State => Operation::SessionState,
            SessionCommand::Focus { session_id } => Operation::Focus { session_id },
        };
        WireEnvelope::new(self.identity, self.request_id, operation)
    }
}
pub trait SessionCommandSink: Send {
    fn submit(&mut self, request: SessionCommandRequest) -> Result<(), ProtocolError>;
    fn poll(&mut self) -> Option<WireEnvelope>;
    fn cancel(&mut self, request_id: u64);
}
