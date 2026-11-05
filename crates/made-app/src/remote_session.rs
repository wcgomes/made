//! Desktop policy for opening, prompting, and closing an SSH workspace.
//!
//! The companion rejects any authenticated frame before `Open` or `Attach`.
//! This module decides that frame and classifies prompts. It does not speak
//! SSH and does not store credentials.
//!
//! Platform reports sanitized failures as `SshEvent::Failed(SshFailure)`.
//! Display uses `SshFailure::detail` and, for an unknown host,
//! `SshFailure::unknown_host_fingerprint`. Stderr is never read. A changed
//! host key is a refusal, not a prompt: the platform does not deliver that
//! prompt, and this module never answers it with `yes`.

use made_remote::protocol::{LaunchTarget, Operation, RemotePath};

/// Why an SSH attempt failed, for the workspace card only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshFailureClass {
    UnknownHost,
    ChangedHostKey,
    AuthFailed,
    Timeout,
    Cancelled,
    MissingCapability,
    /// The installed same-version companion differs and needs its own
    /// per-attempt approval. This is not a generic transport failure.
    UpdateRequired,
    Other,
}

impl SshFailureClass {
    pub const fn detail(self) -> &'static str {
        match self {
            Self::UnknownHost => {
                "The server host key is not trusted yet. Approve the fingerprint or cancel."
            }
            Self::ChangedHostKey => {
                "The server key changed. Made refused the connection and will not accept it through first-use approval."
            }
            Self::AuthFailed => "Authentication failed. The in-memory password was discarded.",
            Self::Timeout => "The SSH attempt timed out.",
            Self::Cancelled => "The SSH attempt was cancelled. Other workspaces were not touched.",
            Self::MissingCapability => {
                "OpenSSH or the packaged askpass helper is unavailable. Made will not fall back to an insecure login."
            }
            Self::UpdateRequired => {
                "The installed companion under this version differs from the packaged files. Approve updating it before the session can open."
            }
            Self::Other => "The SSH attempt failed. No local shell was started.",
        }
    }
}

/// How one askpass prompt must be shown. Changed keys get no input field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshPromptClass {
    UnknownHost { text: String },
    ChangedHostKey { text: String },
    Secret { text: String },
}

/// Classify an askpass prompt. The text is shown to the user and never logged.
///
/// Changed-key detection uses the platform predicate. A bare process exit is
/// not a prompt; [`failure_from_platform`] covers `SshEvent::Failed`.
pub fn classify_prompt(text: &str) -> SshPromptClass {
    if made_platform::ssh::prompt_is_changed_host_key(text) {
        return SshPromptClass::ChangedHostKey {
            text: text.to_owned(),
        };
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains("authenticity of host")
        || lower.contains("can't be established")
        || lower.contains("cannot be established")
        || (lower.contains("fingerprint") && lower.contains("continue connecting"))
    {
        return SshPromptClass::UnknownHost {
            text: text.to_owned(),
        };
    }
    SshPromptClass::Secret {
        text: text.to_owned(),
    }
}

/// Visible SSH failure. `detail` is the platform's static string. The
/// fingerprint is separate so diagnostics can omit it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownFailure {
    pub class: SshFailureClass,
    pub detail: String,
    pub fingerprint: Option<String>,
}

impl ShownFailure {
    /// Unknown host is not a dead-end notice. The user must approve the
    /// fingerprint before another attempt may send `yes`.
    pub const fn requires_host_approval(&self) -> bool {
        matches!(self.class, SshFailureClass::UnknownHost)
    }

    /// A changed key is never an acceptance prompt.
    pub const fn refuses_answer(&self) -> bool {
        matches!(self.class, SshFailureClass::ChangedHostKey)
    }
}

pub fn class_from_kind(kind: made_platform::ssh::SshFailureKind) -> SshFailureClass {
    use made_platform::ssh::SshFailureKind;
    match kind {
        SshFailureKind::UnknownHost => SshFailureClass::UnknownHost,
        SshFailureKind::ChangedHostKey => SshFailureClass::ChangedHostKey,
        SshFailureKind::AuthFailed => SshFailureClass::AuthFailed,
        SshFailureKind::Timeout => SshFailureClass::Timeout,
        SshFailureKind::Cancelled => SshFailureClass::Cancelled,
        SshFailureKind::MissingCapability => SshFailureClass::MissingCapability,
        SshFailureKind::UpdateRequired => SshFailureClass::UpdateRequired,
        SshFailureKind::Io => SshFailureClass::Other,
    }
}

/// Build the card from already-sanitized fields. Unknown-host approval keeps
/// the fingerprint beside the static detail. Every other kind drops it.
pub fn shown_failure(
    class: SshFailureClass,
    detail: impl Into<String>,
    fingerprint: Option<&str>,
) -> ShownFailure {
    let fingerprint = if class == SshFailureClass::UnknownHost {
        fingerprint
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    } else {
        None
    };
    ShownFailure {
        class,
        detail: detail.into(),
        fingerprint,
    }
}

/// Map `SshEvent::Failed`. The detail is copied from the platform value and
/// is not assembled from stderr.
pub fn present_ssh_failure(failure: &made_platform::ssh::SshFailure) -> ShownFailure {
    shown_failure(
        class_from_kind(failure.kind()),
        failure.detail(),
        failure.unknown_host_fingerprint(),
    )
}

/// `yes` is only the explicit unknown-host approval of a fingerprint this tab
/// already showed. A changed key never matches.
pub fn may_answer_yes(
    class: &SshPromptClass,
    approved_fingerprint: Option<&str>,
    prompt: &str,
) -> bool {
    match class {
        SshPromptClass::ChangedHostKey { .. } | SshPromptClass::Secret { .. } => false,
        SshPromptClass::UnknownHost { .. } => approved_fingerprint
            .is_some_and(|fingerprint| !fingerprint.is_empty() && prompt.contains(fingerprint)),
    }
}

/// Map a probe/stage error code. This is not a host-key attestation.
pub fn classify_error(code: made_remote::protocol::ErrorCode) -> SshFailureClass {
    use made_remote::protocol::ErrorCode;
    match code {
        ErrorCode::Timeout => SshFailureClass::Timeout,
        ErrorCode::Cancelled => SshFailureClass::Cancelled,
        ErrorCode::Unavailable | ErrorCode::Unsupported => SshFailureClass::MissingCapability,
        ErrorCode::Unauthorized => SshFailureClass::AuthFailed,
        _ => SshFailureClass::Other,
    }
}

/// Whether this tab is opening a new PTY child or reattaching the same one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOpen {
    New,
    Reattach,
}

/// Resnapshot gate. An empty poll is not a delimiter and never completes the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SnapshotGate {
    fed: bool,
    complete: bool,
}

/// What one poll means for the held terminal. `Live` is output after the single
/// completing frame and must not reset or resume the terminal again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotStep {
    StillWaiting,
    Feed { reset: bool },
    Complete { reset: bool },
    Live,
}

impl SnapshotGate {
    pub const fn is_complete(self) -> bool {
        self.complete
    }

    /// Empty poll. Input stays paused until a real `done: true` frame arrives.
    pub const fn observe_empty(self) -> (Self, SnapshotStep) {
        (self, SnapshotStep::StillWaiting)
    }

    /// One `TerminalOutput` frame. A second `done: true` is live output.
    pub const fn observe_frame(mut self, done: bool) -> (Self, SnapshotStep) {
        if self.complete {
            return (self, SnapshotStep::Live);
        }
        let reset = !self.fed;
        self.fed = true;
        if done {
            self.complete = true;
            (self, SnapshotStep::Complete { reset })
        } else {
            (self, SnapshotStep::Feed { reset })
        }
    }
}

/// The only frame allowed before `Attached`.
pub fn first_operation(
    open: SessionOpen,
    directory: RemotePath,
    target: LaunchTarget,
    rows: u16,
    cols: u16,
) -> Operation {
    match open {
        SessionOpen::New => Operation::Open {
            directory,
            target,
            rows,
            cols,
        },
        SessionOpen::Reattach => Operation::Attach { rows, cols },
    }
}

/// RootValidate and every other control verb wait until Attached.
pub fn allow_before_attached(operation: &Operation) -> bool {
    matches!(operation, Operation::Open { .. } | Operation::Attach { .. })
}

pub const STAGE_FILES: &[&str] = &[
    "made-remote",
    "plugins/opencode-made/package.json",
    "plugins/opencode-made/tui.ts",
    "plugins/opencode-made/index.ts",
    "plugins/opencode-made/src/protocol.ts",
    "plugins/opencode-made/src/integration.ts",
    "plugins/grok-made/plugin.json",
    "plugins/grok-made/README.md",
    "plugins/grok-made/hooks/hooks.json",
    "plugins/grok-made/hooks/status_test.py",
    "plugins/grok-made/hooks/bin/status.py",
];

/// Build the Stage stdin body. Consent false is a hard refusal from the
/// platform header helper; this function does not invent a bypass.
pub fn stage_body(
    consent: bool,
    version: &str,
    target: &str,
    files: &[Vec<u8>],
) -> Result<Vec<u8>, String> {
    let header = made_platform::ssh::staging_header(consent, version, target)
        .map_err(|_| "staging requires explicit consent and a supported target".to_owned())?;
    stage_body_with_header(header, files)
}

/// The approved update payload. Initial staging stays on `consent-v1`; only an
/// explicit `Update companion` approval builds `consent-v2-update`.
pub fn update_stage_body(
    consent: bool,
    version: &str,
    target: &str,
    files: &[Vec<u8>],
) -> Result<Vec<u8>, String> {
    let header = made_platform::ssh::staging_update_header(consent, version, target)
        .map_err(|_| "an update requires explicit approval and a supported target".to_owned())?;
    stage_body_with_header(header, files)
}

fn stage_body_with_header(mut body: Vec<u8>, files: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    if files.len() != STAGE_FILES.len() {
        return Err("stage payload is missing a packaged companion file".to_owned());
    }
    for bytes in files {
        if bytes.is_empty() || bytes.len() > 128 * 1024 * 1024 {
            return Err("a companion artifact is empty or too large".to_owned());
        }
        let hash = sha256_hex(bytes);
        body.extend(format!("{}\n{hash}\n", bytes.len()).into_bytes());
        body.extend(bytes);
    }
    Ok(body)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Online close waits at most this long, off the UI thread.
pub use made_remote::protocol::CLOSE_TIMEOUT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReport {
    Confirmed,
    Unconfirmed,
}

impl CloseReport {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Confirmed => "remote cleanup confirmed",
            Self::Unconfirmed => "remote cleanup unconfirmed",
        }
    }
}

pub fn close_report(confirmed: bool) -> CloseReport {
    if confirmed {
        CloseReport::Confirmed
    } else {
        CloseReport::Unconfirmed
    }
}

/// Send `Operation::Close` and wait until `deadline`. Never disconnects first.
/// Call this on a worker, not the UI thread.
pub fn await_close(client: made_remote::client::RemoteClient) -> CloseReport {
    await_close_until(client, std::time::Instant::now() + CLOSE_TIMEOUT)
}

pub fn await_close_until(
    client: made_remote::client::RemoteClient,
    deadline: std::time::Instant,
) -> CloseReport {
    let id = match client.request(Operation::Close, None) {
        Ok(id) => id,
        Err(_) => {
            client.disconnect();
            return CloseReport::Unconfirmed;
        }
    };
    while std::time::Instant::now() < deadline {
        while let Some(reply) = client.poll_reply() {
            if reply.request_id != id {
                continue;
            }
            match reply.operation {
                Operation::Closed { confirmed } => {
                    client.disconnect();
                    return close_report(confirmed);
                }
                Operation::Error { .. } => {
                    client.disconnect();
                    return CloseReport::Unconfirmed;
                }
                _ => {}
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    client.disconnect();
    CloseReport::Unconfirmed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_opens_and_reattach_attaches_and_root_validate_waits() {
        let directory = RemotePath::new(b"/project".to_vec()).unwrap();
        let open = first_operation(
            SessionOpen::New,
            directory.clone(),
            LaunchTarget::Shell,
            24,
            80,
        );
        assert!(matches!(open, Operation::Open { .. }));
        let attach = first_operation(
            SessionOpen::Reattach,
            directory,
            LaunchTarget::OpenCode,
            24,
            80,
        );
        assert!(matches!(attach, Operation::Attach { rows: 24, cols: 80 }));
        assert!(allow_before_attached(&open));
        assert!(allow_before_attached(&attach));
        assert!(!allow_before_attached(&Operation::RootValidate {
            path: RemotePath::new(b"/project".to_vec()).unwrap(),
        }));
        assert!(!allow_before_attached(&Operation::List {
            path: RemotePath::new(b"/project".to_vec()).unwrap(),
        }));
    }

    #[test]
    fn unknown_host_shows_fingerprint_and_changed_key_is_not_a_secret_field() {
        let unknown = classify_prompt(
            "The authenticity of host 'server' can't be established.\nECDSA key fingerprint is SHA256:abc.\nAre you sure you want to continue connecting (yes/no)?",
        );
        assert!(
            matches!(unknown, SshPromptClass::UnknownHost { text } if text.contains("SHA256:abc"))
        );
        let changed = classify_prompt(
            "WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!\nHost key verification failed.",
        );
        assert!(matches!(changed, SshPromptClass::ChangedHostKey { .. }));
        assert!(matches!(
            classify_prompt("user@server's password:"),
            SshPromptClass::Secret { .. }
        ));
    }

    #[test]
    fn stage_body_requires_consent_and_embeds_hashes() {
        // One entry per STAGE_FILES element: the fixed staging header, the
        // companion binary, and both complete plugin packages.
        let files: Vec<Vec<u8>> = STAGE_FILES
            .iter()
            .enumerate()
            .map(|(index, _)| vec![b'a' + index as u8])
            .collect();
        assert_eq!(files.len(), 11);
        assert!(stage_body(false, "0.1.0", "x86_64-unknown-linux-gnu", &files).is_err());
        let body = stage_body(true, "0.1.0", "x86_64-unknown-linux-gnu", &files).unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with("consent-v1\n0.1.0\nx86_64-unknown-linux-gnu\n"));
        for (index, bytes) in files.iter().enumerate() {
            let letter = (b'a' + index as u8) as char;
            let hash = sha256_hex(bytes);
            assert!(
                text.contains(&format!("{size}\n{hash}\n", size = bytes.len())),
                "missing length-delimited hash {letter}"
            );
        }
        assert!(!text.contains("password"));
    }

    #[test]
    fn sanitized_failure_keeps_static_detail_and_gates_yes() {
        let unknown = shown_failure(
            SshFailureClass::UnknownHost,
            "the remote host key is not in the app trust file",
            Some("SHA256:abcDEF123"),
        );
        assert!(unknown.requires_host_approval());
        assert_eq!(unknown.fingerprint.as_deref(), Some("SHA256:abcDEF123"));
        assert!(!unknown.detail.contains("stderr"));
        assert!(!may_answer_yes(
            &SshPromptClass::UnknownHost {
                text: "fingerprint SHA256:abcDEF123".into(),
            },
            None,
            "fingerprint SHA256:abcDEF123",
        ));
        assert!(may_answer_yes(
            &SshPromptClass::UnknownHost {
                text: "fingerprint SHA256:abcDEF123".into(),
            },
            unknown.fingerprint.as_deref(),
            "fingerprint SHA256:abcDEF123",
        ));

        let changed = shown_failure(
            SshFailureClass::ChangedHostKey,
            "the remote host key changed and was refused",
            Some("SHA256:should-not-stick"),
        );
        assert!(changed.refuses_answer());
        assert!(changed.fingerprint.is_none());
        assert_eq!(
            changed.detail,
            "the remote host key changed and was refused"
        );
        assert!(!may_answer_yes(
            &SshPromptClass::ChangedHostKey { text: "yes".into() },
            Some("SHA256:abc"),
            "yes",
        ));

        for (class, detail) in [
            (SshFailureClass::AuthFailed, "ssh authentication failed"),
            (SshFailureClass::Timeout, "ssh connection timed out"),
            (SshFailureClass::Cancelled, "ssh connection cancelled"),
            (
                SshFailureClass::MissingCapability,
                "openssh or the private askpass helper is unavailable",
            ),
            (SshFailureClass::Other, "ssh connection failed"),
        ] {
            let shown = shown_failure(class, detail, Some("SHA256:hidden"));
            assert_eq!(shown.detail, detail);
            assert!(shown.fingerprint.is_none());
            assert!(!shown.requires_host_approval());
        }
    }

    #[test]
    fn empty_poll_does_not_end_a_resnapshot_that_keeps_emitting() {
        let mut gate = SnapshotGate::default();
        let (next, step) = gate.observe_empty();
        gate = next;
        assert_eq!(step, SnapshotStep::StillWaiting);
        assert!(!gate.is_complete());

        let (next, step) = gate.observe_frame(false);
        gate = next;
        assert_eq!(step, SnapshotStep::Feed { reset: true });
        assert!(!gate.is_complete());

        let (next, step) = gate.observe_empty();
        gate = next;
        assert_eq!(step, SnapshotStep::StillWaiting, "a gap is not done");

        let (next, step) = gate.observe_frame(false);
        gate = next;
        assert_eq!(step, SnapshotStep::Feed { reset: false });
        assert!(!gate.is_complete());

        let (next, step) = gate.observe_frame(true);
        gate = next;
        assert_eq!(step, SnapshotStep::Complete { reset: false });
        assert!(gate.is_complete());

        let (next, step) = gate.observe_frame(true);
        gate = next;
        assert_eq!(step, SnapshotStep::Live, "a second done must not restart");
        let (gate, step) = gate.observe_frame(false);
        assert_eq!(step, SnapshotStep::Live);
        assert!(gate.is_complete());
    }

    #[test]
    fn close_report_is_honest() {
        assert_eq!(close_report(true), CloseReport::Confirmed);
        assert_eq!(close_report(false).label(), "remote cleanup unconfirmed");
    }

    struct ReplyStream {
        inbound: Vec<u8>,
        replied: bool,
    }

    impl made_remote::client::ClientStream for ReplyStream {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
            self.inbound.extend_from_slice(bytes);
            Ok(())
        }
        fn poll(&mut self) -> Option<made_platform::ssh::SshEvent> {
            if self.replied {
                return None;
            }
            let mut decoder = made_remote::protocol::FrameDecoder::new();
            let mut offset = 0;
            while offset < self.inbound.len() {
                match decoder.push(&self.inbound[offset..]) {
                    Ok((_n, Some(frame)))
                        if matches!(frame.operation, made_remote::protocol::Operation::Close) =>
                    {
                        let reply = made_remote::protocol::WireEnvelope::new(
                            frame.identity,
                            frame.request_id,
                            made_remote::protocol::Operation::Closed { confirmed: true },
                        );
                        let bytes = made_remote::protocol::encode(&reply).expect("reply encodes");
                        self.replied = true;
                        self.inbound.clear();
                        return Some(made_platform::ssh::SshEvent::Data(bytes));
                    }
                    Ok((n, _)) if n > 0 => offset += n,
                    _ => return None,
                }
            }
            None
        }
        fn detach(&mut self) {}
    }

    #[test]
    fn close_waits_for_confirmed_without_disconnecting_first() {
        let identity = made_remote::protocol::Identity {
            authority: made_remote::protocol::AuthorityId([9; 16]),
            tab: made_remote::protocol::TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        };
        let root = made_remote::protocol::RemotePath::new(b"/project".to_vec()).unwrap();
        let client = made_remote::client::RemoteClient::new(
            ReplyStream {
                inbound: Vec::new(),
                replied: false,
            },
            identity,
            root,
        )
        .unwrap();
        let report = await_close_until(
            client,
            std::time::Instant::now() + std::time::Duration::from_secs(2),
        );
        assert_eq!(report, CloseReport::Confirmed);
    }
}
