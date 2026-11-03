//! Schema-v1 SSH metadata. Authentication secrets and live tabs have no storage
//! representation. The platform adapter owns private, atomic filesystem I/O.
use std::{collections::HashMap, fmt};

use made_platform::Platform;
use made_remote::protocol::{AuthorityId, Capabilities, ErrorCode, RemotePath};
use serde::{Deserialize, Serialize};

pub const REGISTRY_SCHEMA: u32 = 1;
pub const MAX_SERVERS: usize = 256;
pub const MAX_REGISTRY_BYTES: usize = 1024 * 1024;

/// Implement in the platform adapter: owner-only directory/file, reject
/// symlinks, bounded read, atomic durable replacement; never truncate on failure.
/// Calls are blocking storage operations: invoke off the GUI thread.
pub trait ServerStorage {
    fn read(&mut self, limit: usize) -> Result<Option<Vec<u8>>, StorageError>;
    fn replace(&mut self, bytes: &[u8]) -> Result<(), StorageError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    Unavailable,
    UnsafeLocation,
    WriteFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryError {
    Storage(StorageError),
    Corrupt,
    Version(u32),
    InvalidMetadata,
    Duplicate,
    NotFound,
    Limit,
    StaleProbe,
}
impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SSH registry: {self:?}")
    }
}
impl std::error::Error for RegistryError {}
impl From<StorageError> for RegistryError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

/// References only: there is deliberately no password/token/key-content field.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerMetadata {
    pub display_name: String,
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub key_file: Option<String>,
    pub initial_directory: Option<RemotePath>,
}
impl fmt::Debug for ServerMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ServerMetadata(<redacted>)")
    }
}
impl ServerMetadata {
    pub fn validate(&self) -> Result<(), RegistryError> {
        fn text(value: &str, max: usize) -> bool {
            !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
        }
        fn target(value: &str) -> bool {
            text(value, 255)
                && !value.starts_with('-')
                && !value.chars().any(char::is_whitespace)
                && !value.contains(['@', '/', '\\'])
        }
        if !text(&self.display_name, 128)
            || !target(&self.host)
            || self.user.as_deref().is_some_and(|s| !target(s))
            || self.port == Some(0)
            || self.key_file.as_deref().is_some_and(|s| !text(s, 4096))
        {
            return Err(RegistryError::InvalidMetadata);
        }
        Ok(())
    }
}

/// Explicit provisioning approval for one packaged helper/plugin version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvisionConsent {
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedServer {
    id: AuthorityId,
    metadata: ServerMetadata,
    consent: Option<ProvisionConsent>,
}
impl SavedServer {
    pub fn id(&self) -> AuthorityId {
        self.id
    }
    pub fn metadata(&self) -> &ServerMetadata {
        &self.metadata
    }
    pub fn consent(&self) -> Option<&ProvisionConsent> {
        self.consent.as_ref()
    }
    pub fn allows_provision(&self, version: &str) -> bool {
        self.consent
            .as_ref()
            .is_some_and(|consent| consent.version == version)
    }
}

/// Owned copy: editing/removing the saved record cannot retarget a live tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSnapshot(SavedServer);
impl ServerSnapshot {
    pub fn server(&self) -> &SavedServer {
        &self.0
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    servers: Vec<SavedServer>,
}

/// A probe is read-only; the executor must not launch OpenCode or provision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeRequest {
    pub ticket: ProbeTicket,
    pub server: ServerSnapshot,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeTicket {
    pub authority: AuthorityId,
    pub attempt: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeState {
    Idle,
    Running(ProbeTicket),
    Cancelled,
    Complete(Capabilities),
    Failed(ErrorCode),
}

pub struct ServerRegistry<S> {
    storage: S,
    servers: Vec<SavedServer>,
    probes: HashMap<AuthorityId, ProbeState>,
    next_attempt: u64,
}
impl<S: ServerStorage> ServerRegistry<S> {
    /// Failure never calls replace: preserve corrupt/foreign-version originals.
    /// Local startup can continue without a registry and display this error.
    pub fn load(mut storage: S) -> Result<Self, RegistryError> {
        let servers = match storage.read(MAX_REGISTRY_BYTES)? {
            None => Vec::new(),
            Some(bytes) => {
                if bytes.len() > MAX_REGISTRY_BYTES {
                    return Err(RegistryError::Limit);
                }
                // Inspect version first so a future schema is not mislabelled corrupt.
                let value: serde_json::Value =
                    serde_json::from_slice(&bytes).map_err(|_| RegistryError::Corrupt)?;
                let version = value
                    .get("schema_version")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or(RegistryError::Corrupt)?;
                if version != REGISTRY_SCHEMA {
                    return Err(RegistryError::Version(version));
                }
                let document: Document =
                    serde_json::from_value(value).map_err(|_| RegistryError::Corrupt)?;
                if document.servers.len() > MAX_SERVERS {
                    return Err(RegistryError::Limit);
                }
                let mut seen = std::collections::HashSet::new();
                for server in &document.servers {
                    server
                        .metadata
                        .validate()
                        .map_err(|_| RegistryError::Corrupt)?;
                    if !seen.insert(server.id) {
                        return Err(RegistryError::Corrupt);
                    }
                    if let Some(consent) = &server.consent {
                        validate_version(&consent.version).map_err(|_| RegistryError::Corrupt)?;
                    }
                }
                document.servers
            }
        };
        Ok(Self {
            storage,
            servers,
            probes: HashMap::new(),
            next_attempt: 1,
        })
    }
    pub fn servers(&self) -> &[SavedServer] {
        &self.servers
    }
    pub fn into_storage(self) -> S {
        self.storage
    }
    pub fn snapshot(&self, id: AuthorityId) -> Result<ServerSnapshot, RegistryError> {
        self.servers
            .iter()
            .find(|s| s.id == id)
            .cloned()
            .map(ServerSnapshot)
            .ok_or(RegistryError::NotFound)
    }
    fn persist(&mut self, servers: Vec<SavedServer>) -> Result<(), RegistryError> {
        let bytes = serde_json::to_vec(&Document {
            schema_version: REGISTRY_SCHEMA,
            servers: servers.clone(),
        })
        .map_err(|_| RegistryError::InvalidMetadata)?;
        if bytes.len() > MAX_REGISTRY_BYTES {
            return Err(RegistryError::Limit);
        }
        self.storage.replace(&bytes)?;
        self.servers = servers;
        Ok(())
    }
    pub fn add(&mut self, metadata: ServerMetadata) -> Result<AuthorityId, RegistryError> {
        metadata.validate()?;
        if self.servers.len() >= MAX_SERVERS {
            return Err(RegistryError::Limit);
        }
        let id =
            AuthorityId::random().map_err(|_| RegistryError::Storage(StorageError::Unavailable))?;
        if self.servers.iter().any(|s| s.id == id) {
            return Err(RegistryError::Duplicate);
        }
        let mut servers = self.servers.clone();
        servers.push(SavedServer {
            id,
            metadata,
            consent: None,
        });
        self.persist(servers)?;
        Ok(id)
    }
    /// Returns an attempt to cancel in the executor after durable success.
    /// Every edit clears consent and detection, including host/user/key edits.
    pub fn edit(
        &mut self,
        id: AuthorityId,
        metadata: ServerMetadata,
    ) -> Result<Option<ProbeTicket>, RegistryError> {
        metadata.validate()?;
        let mut servers = self.servers.clone();
        let server = servers
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or(RegistryError::NotFound)?;
        server.metadata = metadata;
        server.consent = None;
        self.persist(servers)?;
        Ok(self.invalidate_probe(id))
    }
    pub fn remove(&mut self, id: AuthorityId) -> Result<Option<ProbeTicket>, RegistryError> {
        let mut servers = self.servers.clone();
        let index = servers
            .iter()
            .position(|s| s.id == id)
            .ok_or(RegistryError::NotFound)?;
        servers.remove(index);
        self.persist(servers)?;
        Ok(self.invalidate_probe(id))
    }
    pub fn set_consent(
        &mut self,
        id: AuthorityId,
        version: Option<String>,
    ) -> Result<(), RegistryError> {
        if let Some(v) = &version {
            validate_version(v)?;
        }
        let mut servers = self.servers.clone();
        servers
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or(RegistryError::NotFound)?
            .consent = version.map(|version| ProvisionConsent { version });
        self.persist(servers)
    }
    fn invalidate_probe(&mut self, id: AuthorityId) -> Option<ProbeTicket> {
        match self.probes.remove(&id) {
            Some(ProbeState::Running(ticket)) => Some(ticket),
            _ => None,
        }
    }
    pub fn probe_state(&self, id: AuthorityId) -> ProbeState {
        self.probes.get(&id).cloned().unwrap_or(ProbeState::Idle)
    }
    pub fn begin_probe(
        &mut self,
        id: AuthorityId,
    ) -> Result<(ProbeRequest, Option<ProbeTicket>), RegistryError> {
        let server = self.snapshot(id)?;
        let ticket = ProbeTicket {
            authority: id,
            attempt: self.next_attempt,
        };
        self.next_attempt = self
            .next_attempt
            .checked_add(1)
            .ok_or(RegistryError::Limit)?;
        let cancel = self.invalidate_probe(id);
        self.probes.insert(id, ProbeState::Running(ticket));
        Ok((ProbeRequest { ticket, server }, cancel))
    }
    /// Local cancellation rejects completion immediately; caller cancels only
    /// this attempt's OpenSSH/prompt resources, not sibling tabs.
    pub fn cancel_probe(&mut self, ticket: ProbeTicket) -> bool {
        if self.probes.get(&ticket.authority) != Some(&ProbeState::Running(ticket)) {
            return false;
        }
        self.probes.insert(ticket.authority, ProbeState::Cancelled);
        true
    }
    pub fn finish_probe(
        &mut self,
        ticket: ProbeTicket,
        result: Result<Capabilities, ErrorCode>,
    ) -> Result<(), RegistryError> {
        if self.probes.get(&ticket.authority) != Some(&ProbeState::Running(ticket)) {
            return Err(RegistryError::StaleProbe);
        }
        if let Ok(capabilities) = &result {
            made_remote::protocol::Operation::CapabilitiesResult {
                capabilities: capabilities.clone(),
            }
            .validate()
            .map_err(|_| RegistryError::InvalidMetadata)?;
        }
        self.probes.insert(
            ticket.authority,
            match result {
                Ok(capabilities) => ProbeState::Complete(capabilities),
                Err(code) => ProbeState::Failed(code),
            },
        );
        Ok(())
    }
    pub fn can_launch_opencode(&self, id: AuthorityId) -> bool {
        matches!(self.probes.get(&id), Some(ProbeState::Complete(capabilities)) if capabilities.opencode)
    }
}
fn validate_version(version: &str) -> Result<(), RegistryError> {
    if version.is_empty() || version.len() > 128 || version.chars().any(char::is_control) {
        return Err(RegistryError::InvalidMetadata);
    }
    Ok(())
}

/// Private app-owned registry file. The name is a metadata key, not a path
/// the renderer can choose.
pub const REGISTRY_FILE: &str = "ssh-servers.json";

/// Platform private-metadata adapter. Reads never create the file. Writes use
/// the platform's atomic replace and never truncate the original on failure.
pub struct PrivateMetadataStorage {
    directory: std::path::PathBuf,
}

impl PrivateMetadataStorage {
    pub fn new(directory: impl Into<std::path::PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    /// Resolve the host app-data directory. Failure is a registry error, not a
    /// reason to block local startup or to fall back to a shared temp file.
    pub fn from_app_paths() -> Result<Self, StorageError> {
        let paths = made_platform::host_platform()
            .app_paths()
            .map_err(|_| StorageError::Unavailable)?;
        Ok(Self::new(paths.data_dir))
    }
}

impl ServerStorage for PrivateMetadataStorage {
    fn read(&mut self, limit: usize) -> Result<Option<Vec<u8>>, StorageError> {
        made_platform::read_private_metadata(&self.directory, REGISTRY_FILE, limit).map_err(
            |error| match error {
                made_platform::PlatformError::Io { operation, .. }
                    if operation.contains("validate") || operation.contains("inspect") =>
                {
                    StorageError::UnsafeLocation
                }
                made_platform::PlatformError::Io { operation, .. }
                    if operation.contains("write") || operation.contains("commit") =>
                {
                    StorageError::WriteFailed
                }
                _ => StorageError::Unavailable,
            },
        )
    }

    fn replace(&mut self, bytes: &[u8]) -> Result<(), StorageError> {
        made_platform::write_private_metadata_atomic(
            &self.directory,
            REGISTRY_FILE,
            bytes,
            MAX_REGISTRY_BYTES,
        )
        .map_err(|error| match error {
            made_platform::PlatformError::Io { operation, .. }
                if operation.contains("validate") || operation.contains("inspect") =>
            {
                StorageError::UnsafeLocation
            }
            _ => StorageError::WriteFailed,
        })
    }
}

/// Parse the fixed OpenSSH probe script. This is not a TUI launch and it does
/// not look for tmux. `Capabilities.tmux` is encoded false and must be ignored.
pub fn parse_probe_output(bytes: &[u8]) -> Result<Capabilities, ErrorCode> {
    let text = std::str::from_utf8(bytes).map_err(|_| ErrorCode::InvalidRequest)?;
    let mut lines = text.split('\n');
    if lines.next().unwrap_or("") != "MADE-PROBE-1" {
        return Err(ErrorCode::InvalidRequest);
    }
    let os = lines.next().unwrap_or("").trim();
    let arch = lines.next().unwrap_or("").trim();
    let opencode = lines.next().unwrap_or("").trim();
    if os.chars().any(char::is_control) || arch.chars().any(char::is_control) {
        return Err(ErrorCode::InvalidRequest);
    }
    let target = match (os, arch) {
        ("Linux", "x86_64") => made_remote::protocol::RemoteTarget::LinuxX64,
        ("Linux", "aarch64") => made_remote::protocol::RemoteTarget::LinuxArm64,
        ("Darwin", "x86_64") => made_remote::protocol::RemoteTarget::MacosX64,
        ("Darwin", "arm64") => made_remote::protocol::RemoteTarget::MacosArm64,
        _ => return Err(ErrorCode::Unsupported),
    };
    // An absolute path is the only accepted detection signal. Empty, relative,
    // or option-like text is absence, not a host executable to run.
    let opencode_present = opencode.starts_with('/')
        && !opencode.chars().any(char::is_control)
        && opencode.len() <= 4096;
    Ok(Capabilities {
        target,
        tmux: false,
        opencode: opencode_present,
        shell_cwd: false,
        helper_version: "probe".to_owned(),
    })
}

/// Sanitized outcome of a probe, stage, or companion spawn. No stderr, no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptError {
    pub class: crate::remote_session::SshFailureClass,
    pub detail: String,
    pub fingerprint: Option<String>,
}

impl AttemptError {
    pub fn from_code(code: ErrorCode) -> Self {
        Self {
            class: crate::remote_session::classify_error(code),
            detail: probe_failure_detail(code).to_owned(),
            fingerprint: None,
        }
    }

    pub fn from_failure(failure: &made_platform::ssh::SshFailure) -> Self {
        let shown = crate::remote_session::present_ssh_failure(failure);
        Self {
            class: shown.class,
            detail: shown.detail,
            fingerprint: shown.fingerprint,
        }
    }

    fn code(code: ErrorCode) -> Self {
        Self::from_code(code)
    }

    pub fn protocol_code(&self) -> ErrorCode {
        use crate::remote_session::SshFailureClass;
        use made_remote::protocol::ErrorCode;
        match self.class {
            SshFailureClass::Timeout => ErrorCode::Timeout,
            SshFailureClass::Cancelled => ErrorCode::Cancelled,
            SshFailureClass::MissingCapability => ErrorCode::Unavailable,
            SshFailureClass::AuthFailed => ErrorCode::Unauthorized,
            SshFailureClass::UnknownHost
            | SshFailureClass::ChangedHostKey
            | SshFailureClass::Other => ErrorCode::Io,
        }
    }
}

/// In-flight OpenSSH probe. Answers are delivered once and are not retained here.
pub struct ProbeAttempt {
    ticket: ProbeTicket,
    prompts: std::sync::mpsc::Receiver<made_platform::ssh_askpass::PromptRequest>,
    answers: std::sync::mpsc::Sender<ProbeAnswer>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    done: std::sync::mpsc::Receiver<Result<Capabilities, AttemptError>>,
}

struct ProbeAnswer {
    id: u64,
    answer: Option<made_platform::ssh_askpass::PromptAnswer>,
}

impl ProbeAttempt {
    pub fn ticket(&self) -> ProbeTicket {
        self.ticket
    }

    pub fn poll_prompt(&self) -> Option<made_platform::ssh_askpass::PromptRequest> {
        self.prompts.try_recv().ok()
    }

    pub fn answer(
        &self,
        id: u64,
        answer: Option<made_platform::ssh_askpass::PromptAnswer>,
    ) -> bool {
        self.answers.send(ProbeAnswer { id, answer }).is_ok()
    }

    pub fn cancel(&self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
    }

    pub fn poll(&self) -> Option<Result<Capabilities, AttemptError>> {
        self.done.try_recv().ok()
    }
}

/// Start a read-only probe on a worker. Missing OpenSSH or askpass fails closed.
pub fn spawn_probe(request: ProbeRequest) -> ProbeAttempt {
    let (prompt_tx, prompts) = std::sync::mpsc::sync_channel(4);
    let (answers, answer_rx) = std::sync::mpsc::channel();
    let (done_tx, done) = std::sync::mpsc::channel();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_cancel = std::sync::Arc::clone(&cancel);
    let ticket = request.ticket;
    let metadata = request.server.server().metadata().clone();
    let _ = std::thread::Builder::new()
        .name("made-ssh-probe".to_owned())
        .spawn(move || {
            let result = run_probe(&metadata, &prompt_tx, &answer_rx, &worker_cancel);
            let _ = done_tx.send(result);
        });
    ProbeAttempt {
        ticket,
        prompts,
        answers,
        cancel,
        done,
    }
}

fn run_probe(
    metadata: &ServerMetadata,
    prompts: &std::sync::mpsc::SyncSender<made_platform::ssh_askpass::PromptRequest>,
    answers: &std::sync::mpsc::Receiver<ProbeAnswer>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Capabilities, AttemptError> {
    let mut process = spawn_ssh(
        metadata,
        made_platform::ssh::SshCommand::Probe,
        Some(std::time::Duration::from_secs(120)),
    )?;
    let mut stdout = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            process.cancel();
            return Err(AttemptError::code(ErrorCode::Cancelled));
        }
        if std::time::Instant::now() >= deadline {
            process.cancel();
            return Err(AttemptError::code(ErrorCode::Timeout));
        }
        while let Ok(answer) = answers.try_recv() {
            if process.answer_prompt(answer.id, answer.answer).is_err() {
                process.cancel();
                return Err(AttemptError::code(ErrorCode::Io));
            }
        }
        if let Some(prompt) = process.poll_prompt() {
            if prompts.try_send(prompt).is_err() {
                process.cancel();
                return Err(AttemptError::code(ErrorCode::Limit));
            }
        }
        match process.poll() {
            Some(made_platform::ssh::SshEvent::Data(bytes)) => {
                if stdout.len().saturating_add(bytes.len()) > 64 * 1024 {
                    process.cancel();
                    return Err(AttemptError::code(ErrorCode::Limit));
                }
                stdout.extend(bytes);
            }
            Some(made_platform::ssh::SshEvent::Eof)
            | Some(made_platform::ssh::SshEvent::Exited(_)) => break,
            Some(made_platform::ssh::SshEvent::TimedOut) => {
                return Err(AttemptError::code(ErrorCode::Timeout))
            }
            Some(made_platform::ssh::SshEvent::Failed(failure)) => {
                return Err(AttemptError::from_failure(&failure))
            }
            None => std::thread::sleep(std::time::Duration::from_millis(15)),
        }
    }
    parse_probe_output(&stdout).map_err(AttemptError::from_code)
}

fn spawn_ssh(
    metadata: &ServerMetadata,
    command: made_platform::ssh::SshCommand,
    timeout: Option<std::time::Duration>,
) -> Result<made_platform::ssh::SshProcess, AttemptError> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let executable = made_platform::find_executable_on_path(std::ffi::OsStr::new("ssh"), &path)
        .ok_or_else(|| AttemptError::code(ErrorCode::Unavailable))?;
    let askpass = packaged_askpass().ok_or_else(|| AttemptError::code(ErrorCode::Unavailable))?;
    let paths = made_platform::host_platform()
        .app_paths()
        .map_err(|_| AttemptError::code(ErrorCode::Unavailable))?;
    let _ = paths.ensure_directories();
    let known_hosts = paths.data_dir.join("ssh-known-hosts");
    let known = known_hosts
        .to_str()
        .filter(|text| {
            known_hosts.is_absolute()
                && !text
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '%' | '"' | '\\'))
        })
        .ok_or_else(|| AttemptError::code(ErrorCode::Unavailable))?;
    let _ = known;
    let identity_file = match metadata.key_file.as_deref() {
        None => None,
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            if !path.is_absolute() {
                return Err(AttemptError::code(ErrorCode::InvalidRequest));
            }
            Some(path)
        }
    };
    let options = made_platform::ssh::SshOptions {
        executable,
        askpass_executable: askpass,
        private_directory: paths.temp_dir,
        known_hosts,
        destination: made_platform::ssh::SshDestination {
            host: metadata.host.clone(),
            user: metadata.user.clone(),
            port: metadata.port,
            identity_file,
        },
        command,
        timeout,
    };
    made_platform::ssh::SshProcess::spawn(options)
        .map_err(|failure| AttemptError::from_failure(&failure))
}

fn packaged_askpass() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let path = dir.join(made_platform::askpass_file_name());
    path.is_file().then_some(path)
}

/// Where a packaged companion binary would live. Absence is a preflight
/// failure, not permission to run a host shell instead.
///
/// The file is always `made-remote`, including on a Windows client. CI stages
/// that Unix name and the remote bootstrap executes it. Askpass stays
/// platform-specific; this binary does not.
pub fn companion_artifact(target: &str) -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let path = dir.join("remote").join(target).join("made-remote");
    path.is_file().then_some(path)
}

/// Attach a companion over OpenSSH. The caller must already have verified a
/// packaged artifact. This does not stage files and does not start a host shell.
/// In-flight Stage. Prompts are forwarded; answers are not retained.
pub struct StageAttempt {
    prompts: std::sync::mpsc::Receiver<made_platform::ssh_askpass::PromptRequest>,
    answers: std::sync::mpsc::Sender<ProbeAnswer>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    done: std::sync::mpsc::Receiver<Result<(), AttemptError>>,
}

impl StageAttempt {
    pub fn poll_prompt(&self) -> Option<made_platform::ssh_askpass::PromptRequest> {
        self.prompts.try_recv().ok()
    }

    pub fn answer(
        &self,
        id: u64,
        answer: Option<made_platform::ssh_askpass::PromptAnswer>,
    ) -> bool {
        self.answers.send(ProbeAnswer { id, answer }).is_ok()
    }

    pub fn cancel(&self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
    }

    pub fn poll(&self) -> Option<Result<(), AttemptError>> {
        self.done.try_recv().ok()
    }
}

/// Stage only after explicit consent. The payload must already include the
/// platform staging header. This does not start a host shell.
pub fn spawn_stage(metadata: ServerMetadata, payload: Vec<u8>) -> StageAttempt {
    let (prompt_tx, prompts) = std::sync::mpsc::sync_channel(4);
    let (answers, answer_rx) = std::sync::mpsc::channel();
    let (done_tx, done) = std::sync::mpsc::channel();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_cancel = std::sync::Arc::clone(&cancel);
    let _ = std::thread::Builder::new()
        .name("made-ssh-stage".to_owned())
        .spawn(move || {
            let result = run_stage(&metadata, &payload, &prompt_tx, &answer_rx, &worker_cancel);
            let _ = done_tx.send(result);
        });
    StageAttempt {
        prompts,
        answers,
        cancel,
        done,
    }
}

fn run_stage(
    metadata: &ServerMetadata,
    payload: &[u8],
    prompts: &std::sync::mpsc::SyncSender<made_platform::ssh_askpass::PromptRequest>,
    answers: &std::sync::mpsc::Receiver<ProbeAnswer>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), AttemptError> {
    let mut process = spawn_ssh(
        metadata,
        made_platform::ssh::SshCommand::Stage,
        Some(std::time::Duration::from_secs(180)),
    )?;
    // SshProcess::write accepts one bounded chunk, not the whole multi-megabyte
    // Stage payload. Send it in chunks and keep draining prompts so the writer
    // cannot deadlock on a full queue.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    let mut stdout = Vec::new();
    let mut offset = 0;
    while offset < payload.len() {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            process.cancel();
            return Err(AttemptError::code(ErrorCode::Cancelled));
        }
        if std::time::Instant::now() >= deadline {
            process.cancel();
            return Err(AttemptError::code(ErrorCode::Timeout));
        }
        let end = (offset + made_platform::ssh::SSH_CHUNK_BYTES).min(payload.len());
        match process.write(&payload[offset..end]) {
            Ok(()) => offset = end,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                while let Ok(answer) = answers.try_recv() {
                    if process.answer_prompt(answer.id, answer.answer).is_err() {
                        process.cancel();
                        return Err(AttemptError::code(ErrorCode::Io));
                    }
                }
                if let Some(prompt) = process.poll_prompt() {
                    if prompts.try_send(prompt).is_err() {
                        process.cancel();
                        return Err(AttemptError::code(ErrorCode::Limit));
                    }
                }
                match process.poll() {
                    Some(made_platform::ssh::SshEvent::Data(bytes)) => {
                        if stdout.len().saturating_add(bytes.len()) > 64 * 1024 {
                            process.cancel();
                            return Err(AttemptError::code(ErrorCode::Limit));
                        }
                        let mut bytes = bytes;
                        stdout.append(&mut bytes);
                    }
                    Some(made_platform::ssh::SshEvent::TimedOut) => {
                        return Err(AttemptError::code(ErrorCode::Timeout))
                    }
                    Some(made_platform::ssh::SshEvent::Failed(failure)) => {
                        return Err(AttemptError::from_failure(&failure))
                    }
                    Some(made_platform::ssh::SshEvent::Eof)
                    | Some(made_platform::ssh::SshEvent::Exited(_)) => {
                        return Err(AttemptError::code(ErrorCode::Io))
                    }
                    None => {}
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(_) => return Err(AttemptError::code(ErrorCode::Io)),
        }
    }
    loop {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            process.cancel();
            return Err(AttemptError::code(ErrorCode::Cancelled));
        }
        if std::time::Instant::now() >= deadline {
            process.cancel();
            return Err(AttemptError::code(ErrorCode::Timeout));
        }
        while let Ok(answer) = answers.try_recv() {
            if process.answer_prompt(answer.id, answer.answer).is_err() {
                process.cancel();
                return Err(AttemptError::code(ErrorCode::Io));
            }
        }
        if let Some(prompt) = process.poll_prompt() {
            if prompts.try_send(prompt).is_err() {
                process.cancel();
                return Err(AttemptError::code(ErrorCode::Limit));
            }
        }
        match process.poll() {
            Some(made_platform::ssh::SshEvent::Data(bytes)) => {
                if stdout.len().saturating_add(bytes.len()) > 64 * 1024 {
                    process.cancel();
                    return Err(AttemptError::code(ErrorCode::Limit));
                }
                stdout.extend(bytes);
            }
            Some(made_platform::ssh::SshEvent::Eof)
            | Some(made_platform::ssh::SshEvent::Exited(_)) => break,
            Some(made_platform::ssh::SshEvent::TimedOut) => {
                return Err(AttemptError::code(ErrorCode::Timeout))
            }
            Some(made_platform::ssh::SshEvent::Failed(failure)) => {
                return Err(AttemptError::from_failure(&failure))
            }
            None => std::thread::sleep(std::time::Duration::from_millis(15)),
        }
    }
    if stdout
        .windows(b"MADE-STAGED-1\n".len())
        .any(|window| window == b"MADE-STAGED-1\n")
    {
        Ok(())
    } else {
        Err(AttemptError::code(ErrorCode::Io))
    }
}

/// Read the six packaged files for one target. Missing files are a preflight
/// failure. File bytes are not included in the error.
pub fn load_stage_files(target: &str) -> Result<Vec<Vec<u8>>, String> {
    let exe = std::env::current_exe().map_err(|_| {
        "The remote companion is not packaged with this build. Made did not start a local shell."
            .to_owned()
    })?;
    let Some(dir) = exe.parent() else {
        return Err(
            "The remote companion is not packaged with this build. Made did not start a local shell."
                .to_owned(),
        );
    };
    let root = dir.join("remote").join(target);
    let mut files = Vec::with_capacity(crate::remote_session::STAGE_FILES.len());
    for name in crate::remote_session::STAGE_FILES {
        let path = root.join(name);
        let bytes = std::fs::read(&path).map_err(|_| {
            "A packaged companion file is missing. Made did not start a local shell.".to_owned()
        })?;
        if bytes.is_empty() {
            return Err(
                "A packaged companion file is empty. Made did not start a local shell.".to_owned(),
            );
        }
        files.push(bytes);
    }
    Ok(files)
}

/// Stage after consent. The worker reads packaged files and never starts a host shell.
pub fn spawn_consented_stage(
    metadata: ServerMetadata,
    version: String,
    target: String,
) -> StageAttempt {
    let (prompt_tx, prompts) = std::sync::mpsc::sync_channel(4);
    let (answers, answer_rx) = std::sync::mpsc::channel();
    let (done_tx, done) = std::sync::mpsc::channel();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_cancel = std::sync::Arc::clone(&cancel);
    let _ = std::thread::Builder::new()
        .name("made-ssh-stage".to_owned())
        .spawn(move || {
            let result = load_stage_files(&target).and_then(|files| {
                crate::remote_session::stage_body(true, &version, &target, &files)
            });
            let result = match result {
                Ok(payload) => {
                    run_stage(&metadata, &payload, &prompt_tx, &answer_rx, &worker_cancel)
                }
                Err(detail) => Err(AttemptError {
                    class: crate::remote_session::SshFailureClass::MissingCapability,
                    detail,
                    fingerprint: None,
                }),
            };
            let _ = done_tx.send(result);
        });
    StageAttempt {
        prompts,
        answers,
        cancel,
        done,
    }
}

/// Shared slot for a failure the remote client would otherwise discard.
/// The fingerprint is not written by Debug.
#[derive(Clone, Default)]
pub struct SshFailureSlot {
    failure: std::sync::Arc<std::sync::Mutex<Option<made_platform::ssh::SshFailure>>>,
}

impl SshFailureSlot {
    pub fn take(&self) -> Option<made_platform::ssh::SshFailure> {
        self.failure.lock().ok().and_then(|mut slot| slot.take())
    }
}

struct ObservingStream {
    inner: made_platform::ssh::SshProcess,
    failure: std::sync::Arc<std::sync::Mutex<Option<made_platform::ssh::SshFailure>>>,
}

impl made_remote::client::ClientStream for ObservingStream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.inner.write(bytes)
    }
    fn poll(&mut self) -> Option<made_platform::ssh::SshEvent> {
        match self.inner.poll() {
            Some(made_platform::ssh::SshEvent::Failed(failure)) => {
                if let Ok(mut slot) = self.failure.lock() {
                    *slot = Some(failure);
                }
                // The client treats every non-data event as a generic drop.
                // Keep the sanitized failure in the slot and let the app disconnect.
                None
            }
            other => other,
        }
    }
    fn detach(&mut self) {
        self.inner.cancel();
    }
    fn poll_prompt(&mut self) -> Option<made_platform::ssh_askpass::PromptRequest> {
        self.inner.poll_prompt()
    }
    fn answer_prompt(
        &mut self,
        id: u64,
        answer: Option<made_platform::ssh_askpass::PromptAnswer>,
    ) -> std::io::Result<()> {
        self.inner.answer_prompt(id, answer)
    }
}

pub struct CompanionSession {
    pub client: made_remote::client::RemoteClient,
    pub failures: SshFailureSlot,
}

/// Forget the app-owned host key after the user explicitly accepts the change.
/// The next probe is an unknown-host approval and shows the new fingerprint.
pub fn forget_saved_host_key(metadata: &ServerMetadata) -> Result<(), AttemptError> {
    metadata
        .validate()
        .map_err(|_| AttemptError::code(ErrorCode::InvalidRequest))?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let ssh_keygen =
        made_platform::find_executable_on_path(std::ffi::OsStr::new("ssh-keygen"), &path)
            .ok_or_else(|| AttemptError::code(ErrorCode::Unavailable))?;
    let paths = made_platform::host_platform()
        .app_paths()
        .map_err(|_| AttemptError::code(ErrorCode::Unavailable))?;
    let known_hosts = paths.data_dir.join("ssh-known-hosts");
    made_platform::ssh::forget_known_host(
        &ssh_keygen,
        &known_hosts,
        &metadata.host,
        metadata.port.unwrap_or(22),
    )
    .map_err(|_| AttemptError::code(ErrorCode::Io))
}

pub fn connect_companion(
    metadata: &ServerMetadata,
    version: &str,
    target: &str,
    identity: made_remote::protocol::Identity,
    root: made_remote::protocol::RemotePath,
) -> Result<CompanionSession, AttemptError> {
    if companion_artifact(target).is_none() {
        return Err(AttemptError {
            class: crate::remote_session::SshFailureClass::MissingCapability,
            detail: "The remote companion is not packaged with this build. Made did not start a local shell."
                .to_owned(),
            fingerprint: None,
        });
    }
    let header =
        made_platform::ssh::companion_header(version, target).map_err(|_| AttemptError {
            class: crate::remote_session::SshFailureClass::Other,
            detail: "remote target is not supported".to_owned(),
            fingerprint: None,
        })?;
    let mut process = spawn_ssh(metadata, made_platform::ssh::SshCommand::Companion, None)?;
    process.write(&header).map_err(|_| AttemptError {
        class: crate::remote_session::SshFailureClass::Other,
        detail: "could not send the companion header".to_owned(),
        fingerprint: None,
    })?;
    let failures = SshFailureSlot::default();
    let client = made_remote::client::RemoteClient::new(
        ObservingStream {
            inner: process,
            failure: std::sync::Arc::clone(&failures.failure),
        },
        identity,
        root,
    )
    .map_err(|_| AttemptError {
        class: crate::remote_session::SshFailureClass::Other,
        detail: "remote client rejected the tab identity".to_owned(),
        fingerprint: None,
    })?;
    Ok(CompanionSession { client, failures })
}

pub fn probe_failure_detail(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::Unsupported => {
            "This server is not a supported remote target. Windows and musl remotes are refused."
        }
        ErrorCode::Unavailable => {
            "OpenSSH or the packaged askpass helper is unavailable. Made will not fall back to an insecure login."
        }
        ErrorCode::Unauthorized => {
            "The server rejected authentication. The in-memory password was discarded."
        }
        ErrorCode::Timeout => "The connection test timed out. Cancel and try again.",
        ErrorCode::Cancelled => "The connection test was cancelled. Other workspaces were not touched.",
        ErrorCode::InvalidRequest => {
            "The server probe did not return the expected read-only banner."
        }
        ErrorCode::Io => "The connection test failed. Check the host, key file, and network.",
        _ => "The connection test failed. No local workspace was started in its place.",
    }
}
