//! Integration diagnostics and structured log access.
//!
//! Diagnostics expose the selected workspace identity, shell cwd capability,
//! effective root/source, optional integration status and reason, active session
//! and root, worktree metadata, plugin/protocol/app versions, the last revision
//! and heartbeat, a copyable log location, and a bounded recent-log view.
//!
//! Every log entry is local and bounded. Messages passed through the
//! token-aware API are redacted before storage; the snapshot text never
//! contains the bridge secret.

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use made_bridge::security::{redact_secret, Token};
use made_platform::{Clipboard, PlatformError};

use crate::integration_status::{IntegrationState, IntegrationStatus, StatusChange};
use crate::sanitize::{sanitize_copy_text, sanitize_display, sanitize_log_text};
use crate::session_follow::FollowedSession;
use crate::worktree::{MetadataStatus, WorktreeMetadata};

/// Maximum number of retained log entries.
pub const MAX_LOG_ENTRIES: usize = 500;

/// Default log file name under the platform log directory.
pub const DEFAULT_LOG_FILE: &str = "integration.log";

/// Log severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Normal lifecycle information.
    Info,
    /// A recoverable failure.
    Warn,
    /// A fatal protocol or connection failure.
    Error,
}

impl LogLevel {
    /// Stable lowercase label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// One structured log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// Monotonic sequence number assigned by the log.
    pub sequence: u64,
    /// Severity.
    pub level: LogLevel,
    /// Redacted message.
    pub message: String,
    /// Related reason code, when known.
    pub reason: Option<String>,
    /// Related session identifier, when known.
    pub session_id: Option<String>,
    /// Related bridge connection identifier, when known.
    pub connection_id: Option<u64>,
}

impl LogEntry {
    /// Build an entry; the log assigns the sequence number.
    pub fn new(level: LogLevel, message: impl Into<String>) -> Self {
        Self {
            sequence: 0,
            level,
            message: message.into(),
            reason: None,
            session_id: None,
            connection_id: None,
        }
    }

    /// Attach a reason code.
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Attach a session identifier.
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Attach a connection identifier.
    pub fn with_connection(mut self, connection_id: u64) -> Self {
        self.connection_id = Some(connection_id);
        self
    }
}

impl fmt::Display for LogEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "[{}] {}", self.level.label(), self.message)
    }
}

/// Bounded recent-log buffer.
#[derive(Debug, Clone)]
pub struct DiagnosticLog {
    entries: VecDeque<LogEntry>,
    capacity: usize,
    next_sequence: u64,
}

impl Default for DiagnosticLog {
    fn default() -> Self {
        Self::new()
    }
}

impl DiagnosticLog {
    /// Log with the documented 500-entry bound.
    pub fn new() -> Self {
        Self::with_capacity(MAX_LOG_ENTRIES)
    }

    /// Log with an explicit bound. Zero is treated as one.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity.max(1)),
            capacity: capacity.max(1),
            next_sequence: 0,
        }
    }

    /// Retained entry bound.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Number of retained entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Monotonic revision of appended/cleared log activity.
    ///
    /// The UI uses this cheap counter to invalidate a cached diagnostics
    /// snapshot without cloning every entry on each frame.
    pub fn revision(&self) -> u64 {
        self.next_sequence
    }

    /// Whether the log is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries from oldest to newest.
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &LogEntry> + ExactSizeIterator {
        self.entries.iter()
    }

    /// The newest `count` entries.
    pub fn recent(&self, count: usize) -> Vec<&LogEntry> {
        let skip = self.entries.len().saturating_sub(count);
        self.entries.iter().skip(skip).collect()
    }

    /// Append an entry, sanitizing every untrusted field first.
    ///
    /// The caller is responsible for token redaction; this method removes
    /// control sequences, bidi controls, and line breaks from messages.
    pub fn push(&mut self, mut entry: LogEntry) -> Option<LogEntry> {
        entry.message = sanitize_log_text(&entry.message);
        entry.reason = entry.reason.map(|reason| sanitize_log_text(&reason));
        entry.session_id = entry.session_id.map(|session| sanitize_log_text(&session));
        self.next_sequence = self.next_sequence.saturating_add(1);
        entry.sequence = self.next_sequence;
        let evicted = if self.entries.len() >= self.capacity {
            self.entries.pop_front()
        } else {
            None
        };
        self.entries.push_back(entry);
        evicted
    }

    /// Append an unredacted message. Prefer [`Self::record_redacted`] when the
    /// message may include the bridge token.
    /// Append a message with token-shaped secrets redacted.
    ///
    /// The bridge token is 256 bits encoded as 64 hexadecimal characters. Any
    /// 64-or-longer hexadecimal run is replaced with `[redacted]`, so a caller
    /// that forgets [`Self::record_redacted`] cannot leak the per-instance
    /// secret. Use [`Self::push`] only for already-safe structured data.
    pub fn record(&mut self, level: LogLevel, message: impl Into<String>) -> &LogEntry {
        let message = redact_hex_secrets(&message.into());
        let entry = LogEntry::new(level, message);
        self.push(entry);
        self.entries.back().expect("a push always leaves an entry")
    }

    /// Append a message with every occurrence of `token` redacted.
    pub fn record_redacted(
        &mut self,
        level: LogLevel,
        message: impl AsRef<str>,
        token: &Token,
    ) -> &LogEntry {
        let message = redact_secret(message.as_ref(), token);
        self.record(level, message)
    }

    /// Redact every retained entry. Used when a token is learned after early
    /// launch logging.
    pub fn redact_all(&mut self, token: &Token) {
        for entry in &mut self.entries {
            entry.message = sanitize_log_text(&redact_secret(&entry.message, token));
            if let Some(reason) = &entry.reason {
                entry.reason = Some(sanitize_log_text(&redact_secret(reason, token)));
            }
            if let Some(session_id) = &entry.session_id {
                entry.session_id = Some(sanitize_log_text(&redact_secret(session_id, token)));
            }
        }
    }

    /// Remove every entry.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.next_sequence = self.next_sequence.saturating_add(1);
    }
}

/// Diagnostics container owned by the app.
#[derive(Debug, Clone)]
pub struct Diagnostics {
    log: DiagnosticLog,
    log_location: PathBuf,
    log_location_display: String,
    app_version: String,
    token_fingerprint: Option<String>,
}

impl Diagnostics {
    /// Create diagnostics with the bounded default log.
    pub fn new(log_location: impl Into<PathBuf>) -> Self {
        let log_location = log_location.into();
        let log_location_display =
            sanitize_display(&redact_hex_secrets(&log_location.to_string_lossy()), 2048);
        Self {
            log: DiagnosticLog::new(),
            log_location,
            log_location_display,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            token_fingerprint: None,
        }
    }

    /// Diagnostics whose on-disk log is deliberately unavailable.
    ///
    /// No predictable fallback path is used; the visible log location explains
    /// why and the app fails closed instead of writing to a shared temporary
    /// directory.
    pub fn unavailable(reason: impl AsRef<str>) -> Self {
        let message = format!("unavailable: {}", reason.as_ref());
        Self {
            log: DiagnosticLog::new(),
            log_location: PathBuf::new(),
            log_location_display: sanitize_display(&message, 2048),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            token_fingerprint: None,
        }
    }

    /// Override the app version shown in diagnostics.
    pub fn with_app_version(mut self, version: impl Into<String>) -> Self {
        self.app_version = version.into();
        self
    }

    /// Attach the safe bridge token fingerprint.
    pub fn with_token_fingerprint(mut self, fingerprint: impl Into<String>) -> Self {
        self.token_fingerprint = Some(fingerprint.into());
        self
    }

    /// Current log.
    pub fn log(&self) -> &DiagnosticLog {
        &self.log
    }

    /// Mutable log for integration lifecycle entries.
    pub fn log_mut(&mut self) -> &mut DiagnosticLog {
        &mut self.log
    }

    /// Log file location.
    pub fn log_location(&self) -> &Path {
        &self.log_location
    }

    /// Copyable, sanitized log path.
    pub fn copyable_log_path(&self) -> &str {
        &self.log_location_display
    }

    /// Copy the sanitized log path to the supplied clipboard.
    ///
    /// Token redaction is applied by the caller when it holds the bridge token;
    /// this method only guarantees control/bidi sanitization.
    pub fn copy_log_path(&self, clipboard: &mut dyn Clipboard) -> Result<(), PlatformError> {
        let path = sanitize_copy_text(&self.log_location_display, None, 2048);
        let path = sanitize_log_text(&redact_hex_secrets(&path));
        clipboard.set_text(&path)
    }

    /// Record an integration status transition.
    pub fn record_status(&mut self, change: &StatusChange) {
        let level = match change.current {
            IntegrationStatus::Connected | IntegrationStatus::Inactive => LogLevel::Info,
            IntegrationStatus::Degraded => LogLevel::Warn,
            IntegrationStatus::Unsupported => LogLevel::Error,
        };
        self.log.push(
            LogEntry::new(
                level,
                redact_hex_secrets(&format!(
                    "{}: {}",
                    change.current.label(),
                    change.reason.human()
                )),
            )
            .with_reason(change.reason.code.code()),
        );
    }

    /// Build the view rendered by the diagnostics panel.
    pub fn snapshot(
        &self,
        integration: &IntegrationState,
        active: Option<&FollowedSession>,
        root: Option<&Path>,
        worktree: Option<&WorktreeMetadata>,
    ) -> DiagnosticsSnapshot {
        self.snapshot_at(integration, active, root, worktree, Instant::now())
    }

    /// Build the view with an explicit instant, for deterministic heartbeat
    /// ages in tests.
    pub fn snapshot_at(
        &self,
        integration: &IntegrationState,
        active: Option<&FollowedSession>,
        root: Option<&Path>,
        worktree: Option<&WorktreeMetadata>,
        now: Instant,
    ) -> DiagnosticsSnapshot {
        let report = integration.report();
        let active_session_id = active
            .map(|session| session.session_id.clone())
            .or(report.active_session_id)
            .map(|value| sanitize_log_text(&value));
        let active_session_title = active
            .map(|session| session.title.clone())
            .or(report.active_session_title)
            .map(|value| sanitize_display(&value, 512));
        let active_subpath = active
            .and_then(|session| session.subpath.clone())
            .map(|value| sanitize_display(&value, 4096));
        let active_root = root
            .map(Path::to_path_buf)
            .or(report.last_known_root.clone());

        DiagnosticsSnapshot {
            status: report.status,
            status_label: report.status.label().to_owned(),
            reason_code: report.reason.code.code(),
            reason: sanitize_log_text(&redact_hex_secrets(&report.reason.human())),
            workspace_identity: None,
            workspace_label: None,
            workspace_path: None,
            shell_cwd: None,
            cwd_supported: None,
            cwd_reason: None,
            root_source: None,
            watcher: None,
            active_session_id,
            active_session_title,
            active_subpath,
            active_root,
            worktree_status: worktree.map(|metadata| metadata.status),
            worktree_branch: worktree
                .and_then(|metadata| metadata.branch.clone())
                .map(|value| sanitize_display(&value, 512)),
            worktree_label: worktree
                .and_then(|metadata| metadata.worktree_label.clone())
                .map(|value| sanitize_display(&value, 512)),
            worktree_is_linked: worktree.is_some_and(|metadata| metadata.is_linked_worktree),
            worktree_reason: worktree
                .and_then(|metadata| metadata.reason.clone())
                .map(|value| sanitize_display(&value, 1024)),
            plugin_version: report
                .plugin_version
                .map(|value| sanitize_display(&value, 128)),
            opencode_version: report
                .opencode_version
                .map(|value| sanitize_display(&value, 128)),
            protocol_version: report.protocol_version,
            app_version: self.app_version.clone(),
            last_revision: report.last_revision,
            last_heartbeat: report
                .last_heartbeat
                .map(|last| now.saturating_duration_since(last)),
            tabs_enabled: report.tabs_enabled,
            log_location: sanitize_display(&self.log_location_display, 2048),
            token_fingerprint: self
                .token_fingerprint
                .clone()
                .map(|value| sanitize_display(&value, 128)),
            entries: self
                .log
                .entries()
                .map(|entry| {
                    let mut entry = entry.clone();
                    entry.message = sanitize_log_text(&redact_hex_secrets(&entry.message));
                    if let Some(reason) = &entry.reason {
                        entry.reason = Some(sanitize_log_text(&redact_hex_secrets(reason)));
                    }
                    entry.session_id = entry.session_id.map(|session| sanitize_log_text(&session));
                    entry
                })
                .collect(),
        }
    }
}

/// Selected-workspace context attached to a diagnostics snapshot by the UI.
///
/// These fields are intentionally separate from the integration snapshot: they
/// describe the shell workspace even while optional OpenCode integration is
/// inactive. All text is sanitized/redacted when attached.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceDiagnostics {
    /// Authenticated pane/workspace identity.
    pub identity: String,
    /// Human-readable workspace label.
    pub label: String,
    /// Canonical workspace folder, when known.
    pub path: Option<PathBuf>,
    /// Latest validated shell working directory.
    pub shell_cwd: Option<PathBuf>,
    /// Whether a launch-local cwd emitter was installed.
    pub cwd_supported: Option<bool>,
    /// Diagnostic capability/fallback reason.
    pub cwd_reason: Option<String>,
    /// `shell` or `opencode`: which owner selected the effective root.
    pub root_source: Option<String>,
    /// Explorer watcher state/reason.
    pub watcher: Option<String>,
}

/// Immutable diagnostics data consumed by the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsSnapshot {
    /// Integration status.
    pub status: IntegrationStatus,
    /// Status label (`connected`, `degraded`, `unsupported`).
    pub status_label: String,
    /// Machine-readable reason code.
    pub reason_code: &'static str,
    /// Human-readable reason.
    pub reason: String,
    /// Selected workspace identity, when attached.
    pub workspace_identity: Option<String>,
    /// Selected workspace label, when attached.
    pub workspace_label: Option<String>,
    /// Selected workspace folder, when attached.
    pub workspace_path: Option<PathBuf>,
    /// Latest validated shell cwd, when attached.
    pub shell_cwd: Option<PathBuf>,
    /// Whether launch-local shell cwd reporting is installed.
    pub cwd_supported: Option<bool>,
    /// Shell cwd capability/fallback reason.
    pub cwd_reason: Option<String>,
    /// Effective root owner (`shell` or `opencode`).
    pub root_source: Option<String>,
    /// Explorer watcher state/reason.
    pub watcher: Option<String>,
    /// Active session identifier.
    pub active_session_id: Option<String>,
    /// Active session title.
    pub active_session_title: Option<String>,
    /// Active session subpath, when reported.
    pub active_subpath: Option<String>,
    /// Active workspace or worktree path.
    pub active_root: Option<PathBuf>,
    /// Worktree metadata availability.
    pub worktree_status: Option<MetadataStatus>,
    /// Worktree branch.
    pub worktree_branch: Option<String>,
    /// Worktree branch or path label.
    pub worktree_label: Option<String>,
    /// Whether the active root is a linked worktree.
    pub worktree_is_linked: bool,
    /// Why worktree metadata is unknown or unavailable.
    pub worktree_reason: Option<String>,
    /// Plugin package version.
    pub plugin_version: Option<String>,
    /// OpenCode version.
    pub opencode_version: Option<String>,
    /// Bridge protocol version.
    pub protocol_version: Option<u32>,
    /// App version.
    pub app_version: String,
    /// Last accepted state revision.
    pub last_revision: Option<u64>,
    /// Age of the last heartbeat at snapshot time.
    pub last_heartbeat: Option<Duration>,
    /// Whether OpenCode reports tabs as enabled.
    pub tabs_enabled: bool,
    /// Copyable log location.
    pub log_location: String,
    /// Safe bridge token fingerprint.
    pub token_fingerprint: Option<String>,
    /// Recent log entries, oldest to newest.
    pub entries: Vec<LogEntry>,
}

impl DiagnosticsSnapshot {
    /// Attach the selected workspace's shell/root context.
    ///
    /// Every field is sanitized and token-shaped hex runs are redacted before
    /// the snapshot can be displayed or copied.
    pub fn attach_workspace(&mut self, workspace: WorkspaceDiagnostics) {
        self.workspace_identity = Some(sanitize_log_text(&redact_hex_secrets(&workspace.identity)));
        self.workspace_label = Some(sanitize_display(&redact_hex_secrets(&workspace.label), 256));
        self.workspace_path = workspace.path.map(|path| {
            PathBuf::from(sanitize_log_text(&redact_hex_secrets(
                &path.to_string_lossy(),
            )))
        });
        self.shell_cwd = workspace.shell_cwd.map(|path| {
            PathBuf::from(sanitize_log_text(&redact_hex_secrets(
                &path.to_string_lossy(),
            )))
        });
        self.cwd_supported = workspace.cwd_supported;
        self.cwd_reason = workspace
            .cwd_reason
            .map(|reason| sanitize_log_text(&redact_hex_secrets(&reason)));
        self.root_source = workspace
            .root_source
            .map(|source| sanitize_log_text(&redact_hex_secrets(&source)));
        self.watcher = workspace
            .watcher
            .map(|watcher| sanitize_log_text(&redact_hex_secrets(&watcher)));
    }

    /// Human-readable report suitable for copying into a bug report.
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        text.push_str("Made diagnostics\n");
        push_line(&mut text, "status", &self.status_label);
        push_line(&mut text, "reason code", self.reason_code);
        push_line(&mut text, "reason", &self.reason);
        if let Some(identity) = &self.workspace_identity {
            push_line(&mut text, "workspace identity", identity);
        }
        if let Some(label) = &self.workspace_label {
            push_line(&mut text, "workspace label", label);
        }
        if let Some(path) = &self.workspace_path {
            push_line(&mut text, "workspace path", &path.display().to_string());
        }
        if let Some(cwd) = &self.shell_cwd {
            push_line(&mut text, "shell cwd", &cwd.display().to_string());
        }
        if let Some(supported) = self.cwd_supported {
            push_line(
                &mut text,
                "cwd reporting",
                if supported {
                    "supported"
                } else {
                    "unsupported"
                },
            );
        }
        if let Some(reason) = &self.cwd_reason {
            push_line(&mut text, "cwd reason", reason);
        }
        if let Some(source) = &self.root_source {
            push_line(&mut text, "root source", source);
        }
        if let Some(watcher) = &self.watcher {
            push_line(&mut text, "watcher", watcher);
        }
        push_line(
            &mut text,
            "active session",
            self.active_session_id.as_deref().unwrap_or("none"),
        );
        push_line(
            &mut text,
            "active title",
            self.active_session_title.as_deref().unwrap_or("none"),
        );
        if let Some(subpath) = &self.active_subpath {
            push_line(&mut text, "active subpath", subpath);
        }
        push_line(
            &mut text,
            "effective root",
            &self
                .active_root
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "none".to_owned()),
        );
        push_line(
            &mut text,
            "worktree",
            &format!(
                "{}{}{}",
                self.worktree_status
                    .map(MetadataStatus::label)
                    .unwrap_or("none"),
                self.worktree_branch
                    .as_ref()
                    .map(|branch| format!(" branch={branch}"))
                    .unwrap_or_default(),
                if self.worktree_is_linked {
                    " linked=true"
                } else {
                    ""
                }
            ),
        );
        if let Some(reason) = &self.worktree_reason {
            push_line(&mut text, "worktree reason", reason);
        }
        push_line(
            &mut text,
            "plugin version",
            self.plugin_version.as_deref().unwrap_or("none"),
        );
        push_line(
            &mut text,
            "protocol version",
            &self
                .protocol_version
                .map(|version| version.to_string())
                .unwrap_or_else(|| "none".to_owned()),
        );
        push_line(
            &mut text,
            "opencode version",
            self.opencode_version.as_deref().unwrap_or("none"),
        );
        push_line(&mut text, "app version", &self.app_version);
        push_line(
            &mut text,
            "last revision",
            &self
                .last_revision
                .map(|revision| revision.to_string())
                .unwrap_or_else(|| "none".to_owned()),
        );
        push_line(
            &mut text,
            "last heartbeat",
            &self
                .last_heartbeat
                .map(|age| format!("{:.3}s ago", age.as_secs_f64()))
                .unwrap_or_else(|| "none".to_owned()),
        );
        push_line(
            &mut text,
            "tabs enabled",
            if self.tabs_enabled { "yes" } else { "no" },
        );
        push_line(&mut text, "log location", &self.log_location);
        if let Some(fingerprint) = &self.token_fingerprint {
            push_line(&mut text, "token fingerprint", fingerprint);
        }
        text.push_str("recent log:\n");
        for entry in &self.entries {
            text.push_str(&format!(
                "  #{} [{}] {}{}{}\n",
                entry.sequence,
                entry.level.label(),
                sanitize_log_text(&redact_hex_secrets(&entry.message)),
                entry
                    .reason
                    .as_ref()
                    .map(|reason| format!(
                        " reason={}",
                        sanitize_log_text(&redact_hex_secrets(reason))
                    ))
                    .unwrap_or_default(),
                entry
                    .session_id
                    .as_ref()
                    .map(|session| format!(
                        " session={}",
                        sanitize_log_text(&redact_hex_secrets(session))
                    ))
                    .unwrap_or_default(),
            ));
        }
        text
    }

    /// Whether `token` appears anywhere in the rendered report.
    pub fn contains_secret(&self, token: &Token) -> bool {
        let secret = token.expose_secret();
        !secret.is_empty() && self.to_text().contains(secret)
    }
}

/// Replace hexadecimal runs of 64 or more characters, the encoded length of a
/// 256-bit bridge token.
fn redact_hex_secrets(text: &str) -> String {
    const SECRET_HEX_CHARS: usize = 64;

    let mut redacted = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if !ch.is_ascii_hexdigit() {
            redacted.push(ch);
            continue;
        }

        let mut end = start + ch.len_utf8();
        while let Some(&(next_index, next)) = chars.peek() {
            if !next.is_ascii_hexdigit() {
                break;
            }
            end = next_index + next.len_utf8();
            chars.next();
        }

        if end - start >= SECRET_HEX_CHARS {
            redacted.push_str("[redacted]");
        } else {
            redacted.push_str(&text[start..end]);
        }
    }
    redacted
}

fn push_line(text: &mut String, label: &str, value: &str) {
    text.push_str(&sanitize_log_text(label));
    text.push_str(": ");
    text.push_str(&sanitize_log_text(&redact_hex_secrets(value)));
    text.push('\n');
}
