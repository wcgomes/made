//! Platform capabilities and OS backends.
//!
//! This is the only crate in the Rust workspace allowed to contain OS-specific
//! code. It owns PTY/ConPTY spawn and resize, shell and environment resolution,
//! process lifecycle, app-owned paths, clipboard, and IME normalization. It
//! exposes capabilities; it does not own product behavior and must not depend
//! on UI or domain crates.
//!
//! # App-owned paths
//!
//! [`AppPaths`] separates read-only installation data from user-writable state:
//!
//! - [`AppPaths::install_root`] (also exposed as [`AppPaths::resource_dir`]) is
//!   resolved from the running executable or bundle: the macOS
//!   `Contents/Resources` directory, the Windows install directory, or the
//!   Linux directory containing the executable. It is never a user-writable
//!   XDG data directory.
//! - [`AppPaths::data_dir`] is the per-user data directory:
//!   `$XDG_DATA_HOME/made`, `~/Library/Application Support/Made`, or
//!   `%APPDATA%\Made`.
//! - [`AppPaths::log_dir`] is user-writable.
//! - [`AppPaths::temp_dir`] is a per-user private directory:
//!   `$XDG_RUNTIME_DIR/made-<uid>` on Linux when available, otherwise
//!   `<data_dir>/temp-<uid>`, and `<temp>/made-<user>` on macOS/Windows. It is
//!   created/validated with mode 0700 on Unix, and a pre-existing symlink or
//!   non-directory at that path is rejected.
//!
//! In release builds [`AppPaths::plugin_dir`] is always
//! `<install_root>/plugins/opencode-made`. Development builds first prefer a
//! complete staged package at `<install_root>/plugins/opencode-made`
//! (`package.json` + `tui.ts`), which makes staged debug artifacts work; when
//! that package is absent they use the repository's `plugins/opencode-made`
//! package (the compile-time manifest path). The documented
//! [`PLUGIN_DIR_ENV`] (`MADE_PLUGIN_DIR`) override wins in development, and
//! that choice is marked development-only in [`AppPaths::plugin_source`]. In
//! release builds the override is accepted only when it canonicalizes inside
//! the install root; an arbitrary user path is rejected.
//!
//! [`AppPaths::ensure_directories`] creates the user-writable data, log, and
//! temp directories with owner-only permissions on Unix (0700).
//!
//! [`open_private_log`] opens or creates an append-only log file with 0600
//! permissions on Unix, rejecting symlinks and non-regular files (including
//! `O_NOFOLLOW` where available). This is the shared path for local logs; the
//! helper exists so diagnostics cannot accidentally follow a symlink or create a
//! world-readable log.
//!
//! [`append_private_log_rotated`] serializes one record under a persistent
//! `<log>.lock` (0600, read+write so Windows whole-file locking works) with a
//! bounded one-second lock wait, rotates the active file directly over `<log>.1`
//! when the record would exceed the caller's byte threshold, and never
//! truncates, rewrites, copies, or deletes existing log content. If the lock is
//! unavailable/times out, or a needed rotation cannot be performed, the append
//! is refused and both logs are left untouched. Current-version cooperative
//! appends keep the active log at most the threshold; mixed-version writers can
//! exceed it, and a legacy oversized active file may survive as `.1` without
//! being rewritten. POSIX replacement is atomic; Windows lock/rename behavior is
//! cross-compiled but unverified.
//!
//! [`AppPaths::opencode_config_path`] is the user's OpenCode `cli.json`:
//! `$XDG_CONFIG_HOME/opencode/cli.json` on Linux,
//! `~/Library/Application Support/opencode/cli.json` on macOS, and
//! `%APPDATA%\opencode\cli.json` on Windows.
//!
//! # User home directory
//!
//! [`Platform::home_dir`] is the app's only home-directory capability. It
//! resolves the host's home environment variable first (`HOME` on Linux/macOS,
//! `USERPROFILE` on Windows), falls back to the other variable, ignores empty
//! values, and validates that the result is an existing readable directory.
//! [`resolve_user_home`] and [`validate_user_home`] expose the two steps for
//! deterministic tests; callers outside this crate must not read `HOME` or
//! `USERPROFILE` directly.
//!
//! # PTY lifecycle
//!
//! [`Platform::spawn_pty`] returns a [`PtySession`]. The session hands out one
//! reader and one writer, can resize the terminal, reports the child's exit
//! status, and terminates and reaps the child. `terminate` is the cleanup path
//! used when the app window closes; on Unix it signals the child's process
//! session (SIGHUP then SIGKILL) so PTY descendants are cleaned up with the
//! shell, including job-control groups that split away from the shell's stored
//! group. Sibling PTYs have distinct sessions and are not signalled. Windows
//! assigns the ConPTY child to a kill-on-close Job Object so descendants stay
//! scoped to the owning workspace tab.
//!
//! # Native backdrop
//!
//! [`native_backdrop`] optionally applies macOS vibrancy or Windows Mica to the
//! app window once at startup. Platforms without a portable native backdrop,
//! disabled configurations, and failed attempts keep Made's current in-app
//! glass. The module never uses Windows acrylic/blur, whose resize lag is a
//! documented regression risk.
//!
//! # Compatible TUIs
//!
//! [`detect_compatible_tuis`] performs a metadata-only `PATH` lookup for
//! optional terminal UIs the app can launch directly. It never spawns the
//! candidate, probes a version, or waits for a service. Empty and relative
//! `PATH` segments are ignored so detection never resolves an executable from
//! the current directory.
//!
//! # Backends
//!
//! Only the host backend is compiled. Each backend implements the same
//! [`Platform`] trait, so callers never branch on the operating system.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod native_backdrop;
#[cfg(target_os = "windows")]
pub mod windows;

/// Environment variable that overrides the app-owned plugin directory.
///
/// Development builds may point this at any staged plugin package. Release
/// builds accept the value only when it canonicalizes inside the app install
/// root; otherwise path resolution fails rather than loading user-writable
/// code.
pub const PLUGIN_DIR_ENV: &str = "MADE_PLUGIN_DIR";

/// Environment variable that overrides the user's default shell.
pub const SHELL_ENV: &str = "MADE_SHELL";

/// The operating-system family behind a backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlatformFamily {
    Linux,
    MacOs,
    Windows,
}

impl PlatformFamily {
    /// The family compiled into this build.
    pub const fn current() -> Self {
        cfg::CURRENT_FAMILY
    }

    /// Stable lowercase name, suitable for diagnostics.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::MacOs => "macos",
            Self::Windows => "windows",
        }
    }
}

impl fmt::Display for PlatformFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(target_os = "linux")]
mod cfg {
    use super::PlatformFamily;
    pub const CURRENT_FAMILY: PlatformFamily = PlatformFamily::Linux;
}

#[cfg(target_os = "macos")]
mod cfg {
    use super::PlatformFamily;
    pub const CURRENT_FAMILY: PlatformFamily = PlatformFamily::MacOs;
}

#[cfg(target_os = "windows")]
mod cfg {
    use super::PlatformFamily;
    pub const CURRENT_FAMILY: PlatformFamily = PlatformFamily::Windows;
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("made-platform supports Linux, macOS, and Windows only");

/// Errors from platform capabilities.
#[derive(Debug)]
pub enum PlatformError {
    /// An operating-system operation failed.
    Io {
        operation: &'static str,
        source: io::Error,
    },
    /// The capability is not available in this build or environment.
    Unsupported(String),
    /// A platform-level operation failed for another reason.
    Message(String),
}

impl PlatformError {
    /// Build an I/O error with an operation label.
    pub fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }

    /// Build a generic platform error.
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { operation, source } => write!(f, "{operation}: {source}"),
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
            Self::Message(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for PlatformError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for PlatformError {
    fn from(source: io::Error) -> Self {
        Self::io("io", source)
    }
}

/// Why the platform could not provide a usable user home directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HomeDirectoryErrorKind {
    /// Neither the platform's home variable nor its fallback is set.
    Unresolved,
    /// The resolved path does not exist.
    Missing,
    /// The resolved path exists but is not a directory.
    NotADirectory,
    /// The directory exists but cannot be read.
    Unreadable,
}

impl HomeDirectoryErrorKind {
    /// Stable machine-readable code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Unresolved => "home_unresolved",
            Self::Missing => "home_missing",
            Self::NotADirectory => "home_not_a_directory",
            Self::Unreadable => "home_unreadable",
        }
    }

    /// Short, actionable description.
    pub const fn headline(self) -> &'static str {
        match self {
            Self::Unresolved => "the user home directory cannot be determined",
            Self::Missing => "the user home directory does not exist",
            Self::NotADirectory => "the user home path is not a folder",
            Self::Unreadable => "the user home directory is not readable",
        }
    }
}

/// A user-home resolution or validation failure.
///
/// The app maps this into its own workspace fallback so the terminal remains
/// usable; the platform layer never opens a picker or blocks startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeDirectoryError {
    kind: HomeDirectoryErrorKind,
    path: Option<PathBuf>,
    detail: String,
}

impl HomeDirectoryError {
    fn new(kind: HomeDirectoryErrorKind, path: Option<PathBuf>, detail: impl Into<String>) -> Self {
        Self {
            kind,
            path,
            detail: detail.into(),
        }
    }

    fn from_io(path: PathBuf, error: io::Error) -> Self {
        let kind = match error.kind() {
            io::ErrorKind::NotFound => HomeDirectoryErrorKind::Missing,
            io::ErrorKind::NotADirectory => HomeDirectoryErrorKind::NotADirectory,
            io::ErrorKind::PermissionDenied => HomeDirectoryErrorKind::Unreadable,
            _ => HomeDirectoryErrorKind::Unreadable,
        };
        Self::new(kind, Some(path), error.to_string())
    }

    /// Failure classification.
    pub fn kind(&self) -> HomeDirectoryErrorKind {
        self.kind
    }

    /// Resolved path, when one was available before validation failed.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Lower-level detail, suitable for diagnostics.
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for HomeDirectoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(
                formatter,
                "{}: {} ({})",
                self.kind.headline(),
                path.display(),
                self.detail
            ),
            None => write!(formatter, "{}: {}", self.kind.headline(), self.detail),
        }
    }
}

impl std::error::Error for HomeDirectoryError {}

/// Resolve the user home path from explicit environment values.
///
/// Windows prefers `USERPROFILE` and falls back to `HOME`; Linux and macOS
/// prefer `HOME` and fall back to `USERPROFILE`. Empty values are ignored. The
/// result is not validated; use [`validate_user_home`] before opening it.
pub fn resolve_user_home(
    family: PlatformFamily,
    home: Option<&OsStr>,
    user_profile: Option<&OsStr>,
) -> Result<PathBuf, HomeDirectoryError> {
    let home = home.filter(|value| !value.is_empty());
    let user_profile = user_profile.filter(|value| !value.is_empty());
    let selected = match family {
        PlatformFamily::Windows => user_profile.or(home),
        PlatformFamily::Linux | PlatformFamily::MacOs => home.or(user_profile),
    };
    selected.map(PathBuf::from).ok_or_else(|| {
        let detail = match family {
            PlatformFamily::Windows => "USERPROFILE and HOME are unset or empty",
            PlatformFamily::Linux | PlatformFamily::MacOs => {
                "HOME and USERPROFILE are unset or empty"
            }
        };
        HomeDirectoryError::new(HomeDirectoryErrorKind::Unresolved, None, detail)
    })
}

/// Validate that `path` is an existing, readable directory.
///
/// Returns the same path on success so callers can chain resolution and
/// validation without cloning. Missing paths, non-directories, and unreadable
/// directories are distinct classifications for the app's fallback message.
pub fn validate_user_home(path: impl Into<PathBuf>) -> Result<PathBuf, HomeDirectoryError> {
    let path = path.into();
    if path.as_os_str().is_empty() {
        return Err(HomeDirectoryError::new(
            HomeDirectoryErrorKind::Unresolved,
            None,
            "the user home path is empty",
        ));
    }
    let metadata = std::fs::metadata(&path)
        .map_err(|error| HomeDirectoryError::from_io(path.clone(), error))?;
    if !metadata.is_dir() {
        return Err(HomeDirectoryError::new(
            HomeDirectoryErrorKind::NotADirectory,
            Some(path),
            "the user home path exists but is not a directory",
        ));
    }
    match std::fs::read_dir(&path) {
        Ok(mut entries) => {
            if let Some(Err(error)) = entries.next() {
                return Err(HomeDirectoryError::from_io(path, error));
            }
        }
        Err(error) => return Err(HomeDirectoryError::from_io(path, error)),
    }
    Ok(path)
}

fn host_home_dir(family: PlatformFamily) -> Result<PathBuf, HomeDirectoryError> {
    let home = env::var_os("HOME");
    let user_profile = env::var_os("USERPROFILE");
    let path = resolve_user_home(family, home.as_deref(), user_profile.as_deref())?;
    validate_user_home(path)
}

/// Exit status of a PTY child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitStatus {
    /// Process exit code (or the platform's substitute for signal exits).
    pub code: u32,
    /// Signal name when the child was terminated by a signal.
    pub signal: Option<String>,
}

impl ExitStatus {
    /// Whether the child exited successfully.
    pub fn success(&self) -> bool {
        self.signal.is_none() && self.code == 0
    }
}

impl fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.signal {
            Some(signal) => write!(f, "signal {signal}"),
            None => write!(f, "exit code {}", self.code),
        }
    }
}

/// Size of the PTY viewport in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

impl Default for PtySize {
    fn default() -> Self {
        Self { rows: 24, cols: 80 }
    }
}

/// Fully-resolved command used to spawn a PTY child.
///
/// `env` is the complete environment for the child, not an overlay. Build it
/// with [`Platform::environment`] or [`Environment`]. The caller decides the
/// shell and arguments, so the platform layer never invents product behavior.
///
/// Debug output never prints environment values or argument contents: a child
/// environment can include `OPENCODE_CLI_CONFIG_CONTENT` (with the bridge
/// token) and command arguments can include secrets. Use [`Self::env`] and
/// [`Self::args`] when the actual values are needed.
#[derive(Clone, PartialEq, Eq)]
pub struct PtyRequest {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub env_remove: Vec<OsString>,
    pub size: PtySize,
}

impl fmt::Debug for PtyRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PtyRequest")
            .field("program", &self.program)
            .field("arg_count", &self.args.len())
            .field("cwd", &self.cwd)
            .field("env", &RedactedEnv::new(&self.env))
            .field("env_remove", &self.env_remove)
            .field("size", &self.size)
            .finish()
    }
}

impl fmt::Display for PtyRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PtyRequest {{ program: {:?}, cwd: {}, size: {}x{}, env: {} vars }}",
            self.program,
            self.cwd.display(),
            self.size.rows,
            self.size.cols,
            self.env.len()
        )
    }
}

impl PtyRequest {
    /// Base request for a shell in `cwd` with an empty environment.
    pub fn new(program: impl Into<OsString>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: cwd.into(),
            env: Vec::new(),
            env_remove: Vec::new(),
            size: PtySize::default(),
        }
    }
}

/// A spawned PTY child.
///
/// Methods take `&self` so a reader thread, a monitor thread, and the UI thread
/// can share one session without serializing unrelated operations. The session
/// hands out at most one reader and one writer.
pub trait PtySession: Send + Sync {
    /// Take the child's output stream. Returns an error on the second call.
    fn take_reader(&self) -> Result<Box<dyn Read + Send>, PlatformError>;

    /// Take the child's input stream. Returns an error on the second call.
    fn take_writer(&self) -> Result<Box<dyn Write + Send>, PlatformError>;

    /// Resize the PTY viewport and notify the child.
    fn resize(&self, size: PtySize) -> Result<(), PlatformError>;

    /// Current kernel viewport size.
    fn size(&self) -> Result<PtySize, PlatformError>;

    /// Poll for child exit without blocking. Also reaps the child.
    fn try_wait(&self) -> Result<Option<ExitStatus>, PlatformError>;

    /// Terminate the child.
    ///
    /// The Unix backend captures the stored PTY session/child process group
    /// (even when the direct shell already exited), the current foreground
    /// group re-queried with `tcgetpgrp`, and every process group in the PTY
    /// child's session. It sends `SIGHUP` followed by `SIGKILL` to each group.
    /// The session scan reaches default interactive job-control background
    /// groups while leaving sibling workspaces alone, because each PTY child is
    /// its own session. It never signals PID 1 or Made's own process group.
    ///
    /// Remaining limits: processes that call `setsid`/detach into another
    /// session escape the session scan; signaling a stored group or session
    /// after the shell has been reaped is best effort and carries the usual
    /// PID/PGID/session-id reuse window. Windows assigns the ConPTY child to a
    /// kill-on-close Job Object,
    /// so descendants created after assignment are scoped to the owning
    /// workspace tab; a descendant that explicitly breaks away from the job is
    /// outside that guarantee.
    fn kill(&self) -> Result<(), PlatformError>;

    /// Terminate the child and wait until it has exited and been reaped.
    ///
    /// This is the process-cleanup path used when the app window closes. The
    /// default implementation polls [`PtySession::try_wait`] after
    /// [`PtySession::kill`] for up to three seconds.
    fn terminate(&self) -> Result<ExitStatus, PlatformError> {
        if let Some(status) = self.try_wait()? {
            return Ok(status);
        }
        self.kill()?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(PlatformError::message(
                    "PTY child did not exit after terminate",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Child process id, when the platform exposes one.
    fn pid(&self) -> Option<u32>;
}

/// Resolved shell command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
}

impl ShellSpec {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }
}

/// A complete environment for a child process.
///
/// Debug and Display output deliberately never prints values. The environment
/// can contain `OPENCODE_CLI_CONFIG_CONTENT` (whose plugin options carry the
/// bridge token) and unrelated secrets inherited from the user session.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Environment {
    vars: Vec<(OsString, OsString)>,
}

impl Environment {
    /// Snapshot the current process environment.
    pub fn current() -> Self {
        let mut vars: Vec<_> = env::vars_os().collect();
        vars.sort_by(|a, b| a.0.cmp(&b.0));
        Self { vars }
    }

    /// Build from explicit pairs.
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        let mut env = Self::default();
        for (key, value) in pairs {
            env.set(key, value);
        }
        env
    }

    /// Set or replace a variable.
    pub fn set(&mut self, key: impl Into<OsString>, value: impl Into<OsString>) {
        let key = key.into();
        let value = value.into();
        if let Some(existing) = self.vars.iter_mut().find(|(k, _)| *k == key) {
            existing.1 = value;
        } else {
            self.vars.push((key, value));
        }
    }

    /// Remove a variable if present.
    pub fn remove(&mut self, key: impl AsRef<OsStr>) {
        let key = key.as_ref();
        self.vars.retain(|(k, _)| k.as_os_str() != key);
    }

    /// Look up a variable.
    pub fn get(&self, key: impl AsRef<OsStr>) -> Option<&OsStr> {
        let key = key.as_ref();
        self.vars
            .iter()
            .find(|(k, _)| k.as_os_str() == key)
            .map(|(_, value)| value.as_os_str())
    }

    /// All variables in stable order.
    pub fn vars(&self) -> &[(OsString, OsString)] {
        &self.vars
    }

    /// Consume the environment into owned pairs.
    pub fn into_vec(self) -> Vec<(OsString, OsString)> {
        self.vars
    }

    /// Apply the terminal defaults Made promises to its children.
    ///
    /// This deliberately replaces inherited values: a terminal app must know
    /// which color and cursor protocol the child negotiates.
    pub fn with_terminal_defaults(mut self) -> Self {
        self.set("TERM", "xterm-256color");
        self.set("COLORTERM", "truecolor");
        self.set("TERM_PROGRAM", "Made");
        self.set("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        self
    }
}

impl fmt::Debug for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Environment")
            .field("vars", &RedactedEnv::new(&self.vars))
            .finish()
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Environment({} variables)", self.vars.len())
    }
}

/// Placeholder printed instead of a secret-bearing environment value.
pub const REDACTED_VALUE: &str = "<redacted>";

/// Whether an environment variable's value may carry a secret.
///
/// This covers `OPENCODE_CLI_CONFIG_CONTENT` (composed plugin options include
/// the bridge token), `OPENCODE_CONFIG_CONTENT`, and any name containing
/// `TOKEN`, `SECRET`, `PASSWORD`, or `API_KEY`. The broad match deliberately
/// includes `MADE_*TOKEN*` overrides and similar variables a caller may add.
pub fn is_sensitive_env_key(key: impl AsRef<OsStr>) -> bool {
    let key = key.as_ref().to_string_lossy().to_ascii_uppercase();
    key == "OPENCODE_CLI_CONFIG_CONTENT"
        || key == "OPENCODE_CONFIG_CONTENT"
        || key.contains("TOKEN")
        || key.contains("SECRET")
        || key.contains("PASSWORD")
        || key.contains("API_KEY")
}

/// Debug adapter for environment pairs that never prints a full environment.
///
/// Variable names are printed. Values are replaced with [`REDACTED_VALUE`] for
/// secret-bearing names and with `<set>` for everything else, so an accidental
/// `{:?}` on an options or request struct cannot disclose a token or an
/// unrelated inherited secret. This is shared by [`Environment`],
/// [`PtyRequest`], and `made-terminal`'s `PtyOptions`.
#[derive(Clone, Copy)]
pub struct RedactedEnv<'a> {
    vars: &'a [(OsString, OsString)],
}

impl<'a> RedactedEnv<'a> {
    /// Wrap environment pairs for redacted formatting.
    pub fn new(vars: &'a [(OsString, OsString)]) -> Self {
        Self { vars }
    }
}

impl fmt::Debug for RedactedEnv<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = f.debug_map();
        for (key, _) in self.vars {
            if is_sensitive_env_key(key) {
                map.entry(key, &REDACTED_VALUE);
            } else {
                map.entry(key, &"<set>");
            }
        }
        map.finish()
    }
}

/// Where [`AppPaths::plugin_dir`] was resolved from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginDirSource {
    /// The packaged install/resource root, or a release override that
    /// canonicalized inside it.
    Installed,
    /// The repository plugin package in a development build.
    DevelopmentRepository,
    /// The [`PLUGIN_DIR_ENV`] override in a development build.
    DevelopmentOverride,
}

impl PluginDirSource {
    /// Stable lowercase name for diagnostics.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::DevelopmentRepository => "development-repository",
            Self::DevelopmentOverride => "development-override",
        }
    }

    /// Whether this source is only permitted in a development build.
    pub const fn is_development_only(self) -> bool {
        matches!(
            self,
            Self::DevelopmentRepository | Self::DevelopmentOverride
        )
    }
}

impl fmt::Display for PluginDirSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// App-owned directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    /// Read-only application install/resource root resolved from the running
    /// executable or bundle: macOS `Contents/Resources`, the Windows install
    /// directory, or the Linux directory containing the executable. This is
    /// never a user-writable XDG data directory.
    pub install_root: PathBuf,
    /// Packaged resource root. Retained as an alias of
    /// [`Self::install_root`] for callers that resolved resources before the
    /// explicit install root existed.
    pub resource_dir: PathBuf,
    /// Per-user application data directory. Never used for bundled code.
    pub data_dir: PathBuf,
    /// Root that contains [`Self::plugin_dir`], suitable as the app-owned root
    /// for `made-bridge` composition. In release builds this is the install
    /// root; in development it is the repository root or the selected override.
    pub plugin_root: PathBuf,
    /// Directory containing the app-owned OpenCode plugin package.
    pub plugin_dir: PathBuf,
    /// Where [`Self::plugin_dir`] came from.
    pub plugin_source: PluginDirSource,
    /// User OpenCode CLI configuration (`cli.json`), read only by composition.
    ///
    /// Linux: `$XDG_CONFIG_HOME/opencode/cli.json` (or `~/.config/...`).
    /// macOS: `~/Library/Application Support/opencode/cli.json`.
    /// Windows: `%APPDATA%\opencode\cli.json`.
    pub opencode_config_path: PathBuf,
    /// Directory for local logs.
    pub log_dir: PathBuf,
    /// Per-user private temporary directory.
    ///
    /// Linux: `$XDG_RUNTIME_DIR/made-<uid>` when available, otherwise
    /// `<data_dir>/temp-<uid>`. macOS/Windows: `<temp>/made-<user>`.
    /// [`AppPaths::ensure_directories`] creates and validates it with mode 0700
    /// on Unix and rejects pre-existing symlinks/non-directories.
    pub temp_dir: PathBuf,
}

impl AppPaths {
    /// Whether the selected plugin directory is a development-only location.
    pub fn plugin_is_development_only(&self) -> bool {
        self.plugin_source.is_development_only()
    }

    /// App-owned root suitable for `made-bridge` plugin containment checks.
    ///
    /// This is [`Self::plugin_root`]; the accessor exists so the app can pass a
    /// named contract to composition without depending on the field shape.
    pub fn app_owned_root(&self) -> &Path {
        &self.plugin_root
    }
}

/// Inputs used to resolve [`AppPaths`].
///
/// Kept explicit so path policy is testable without mutating process-wide
/// environment variables. [`PathsInputs::from_env`] builds the real host values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathsInputs {
    pub family: PlatformFamily,
    pub home: Option<PathBuf>,
    pub xdg_data_home: Option<PathBuf>,
    pub xdg_state_home: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    /// Per-user runtime directory (Linux). Used for the private temp root.
    pub xdg_runtime_dir: Option<PathBuf>,
    pub app_data: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    /// Read-only application install/resource root. Required; resolution fails
    /// when it cannot be derived from the executable or supplied by the caller.
    pub install_root: Option<PathBuf>,
    /// Base temp directory used by macOS/Windows and as a fallback input.
    pub temp_dir: PathBuf,
    /// Stable per-user token used in the private temp directory name. On Unix
    /// this is the numeric uid; on Windows it is a sanitized user name.
    pub user_token: String,
    /// [`PLUGIN_DIR_ENV`] value.
    pub plugin_override: Option<PathBuf>,
    /// Repository plugin package for development builds.
    pub dev_plugin_dir: Option<PathBuf>,
    /// Development-build policy switch. `from_env` sets this from
    /// `cfg!(debug_assertions)`; tests set it explicitly to exercise both
    /// release and development resolution.
    pub development: bool,
}

/// Stable per-user token for the private temp directory.
///
/// Unix uses the numeric uid. Windows uses a sanitized `USERNAME`; separators
/// and non-portable characters are removed so the value cannot escape the
/// app-owned temp root.
pub fn current_user_token(family: PlatformFamily) -> String {
    match family {
        #[cfg(unix)]
        PlatformFamily::Linux | PlatformFamily::MacOs => {
            let uid = unsafe { libc::getuid() };
            uid.to_string()
        }
        #[cfg(windows)]
        PlatformFamily::Windows => env::var("USERNAME")
            .ok()
            .map(|value| sanitize_user_token(&value))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "user".to_owned()),
        _ => "user".to_owned(),
    }
}

fn sanitize_user_token(value: &str) -> String {
    let token: String = value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || *character == '-' || *character == '_'
        })
        .take(64)
        .collect();
    if token.is_empty() {
        "user".to_owned()
    } else {
        token
    }
}

impl PathsInputs {
    /// Read host environment values for `family`.
    pub fn from_env(family: PlatformFamily) -> Self {
        let var_path = |name: &str| {
            env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let home = {
            let home = env::var_os("HOME");
            let user_profile = env::var_os("USERPROFILE");
            resolve_user_home(family, home.as_deref(), user_profile.as_deref()).ok()
        };
        let development = cfg!(debug_assertions);
        Self {
            family,
            home,
            xdg_data_home: var_path("XDG_DATA_HOME"),
            xdg_state_home: var_path("XDG_STATE_HOME"),
            xdg_config_home: var_path("XDG_CONFIG_HOME"),
            xdg_runtime_dir: var_path("XDG_RUNTIME_DIR"),
            app_data: var_path("APPDATA"),
            local_app_data: var_path("LOCALAPPDATA"),
            install_root: install_root_from_current_executable(family)
                .or_else(|| development.then(repository_root)),
            temp_dir: env::temp_dir(),
            user_token: current_user_token(family),
            plugin_override: var_path(PLUGIN_DIR_ENV),
            dev_plugin_dir: development_plugin_dir(),
            development,
        }
    }
}

/// The repository root containing the development plugin package.
pub fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .to_path_buf()
}

/// The repository's app-owned plugin package directory.
///
/// This is a compile-time path into the repository. Release builds never use
/// it; they load `<install_root>/plugins/opencode-made` instead.
pub fn repository_plugin_dir() -> PathBuf {
    repository_root().join("plugins").join("opencode-made")
}

/// Development-build plugin directory, if this build should prefer the repo.
pub fn development_plugin_dir() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        Some(repository_plugin_dir())
    } else {
        None
    }
}

/// Resolve the read-only install/resource root for an executable path.
///
/// macOS bundle executables live in `Foo.app/Contents/MacOS/`; the resource
/// root is the sibling `Contents/Resources/`. Windows and Linux use the
/// directory containing the executable. A path that does not look like a macOS
/// bundle falls back to the executable directory.
pub fn install_root_for_executable(family: PlatformFamily, executable: &Path) -> Option<PathBuf> {
    let executable_dir = executable.parent()?;
    if family == PlatformFamily::MacOs && executable_dir.file_name() == Some(OsStr::new("MacOS")) {
        if let Some(contents) = executable_dir.parent() {
            if contents.file_name() == Some(OsStr::new("Contents")) {
                return Some(contents.join("Resources"));
            }
        }
    }
    Some(executable_dir.to_path_buf())
}

/// Resolve the install/resource root of the running executable.
pub fn install_root_from_current_executable(family: PlatformFamily) -> Option<PathBuf> {
    env::current_exe()
        .ok()
        .and_then(|executable| install_root_for_executable(family, &executable))
}

fn plugin_package_is_complete(path: &Path) -> bool {
    path.join("package.json").is_file() && path.join("tui.ts").is_file()
}

fn resolve_plugin_dir(
    inputs: &PathsInputs,
    install_root: &Path,
    installed_plugin: &Path,
) -> Result<(PathBuf, PathBuf, PluginDirSource), PlatformError> {
    if inputs.development {
        if let Some(override_dir) = &inputs.plugin_override {
            // Development-only: never reachable from a release build.
            return Ok((
                override_dir.clone(),
                override_dir.clone(),
                PluginDirSource::DevelopmentOverride,
            ));
        }
        // Staged debug artifacts carry the packaged plugin under their install
        // root. Prefer that complete package over the source-tree plugin so a
        // locally packaged build exercises the same layout as a release build
        // without weakening the release containment rule.
        // Only prefer the staged package when the caller actually resolved an
        // install root. When development resolution had to fall back to the
        // repository root, the repository package remains the development
        // source and diagnostics must say so.
        if inputs.install_root.is_some() && plugin_package_is_complete(installed_plugin) {
            return Ok((
                installed_plugin.to_path_buf(),
                install_root.to_path_buf(),
                PluginDirSource::Installed,
            ));
        }
        let dev_dir = inputs
            .dev_plugin_dir
            .clone()
            .unwrap_or_else(repository_plugin_dir);
        return Ok((
            dev_dir,
            repository_root(),
            PluginDirSource::DevelopmentRepository,
        ));
    }

    if let Some(override_dir) = &inputs.plugin_override {
        let canonical_install_root = std::fs::canonicalize(install_root)
            .map_err(|error| PlatformError::io("canonicalize app install root", error))?;
        let canonical_override = std::fs::canonicalize(override_dir)
            .map_err(|error| PlatformError::io("canonicalize plugin override", error))?;
        if !canonical_override.starts_with(&canonical_install_root) {
            return Err(PlatformError::message(format!(
                "{PLUGIN_DIR_ENV} must resolve inside the app install root in release builds: \
                 {} is outside {}",
                canonical_override.display(),
                canonical_install_root.display()
            )));
        }
        return Ok((
            canonical_override,
            install_root.to_path_buf(),
            PluginDirSource::Installed,
        ));
    }
    Ok((
        installed_plugin.to_path_buf(),
        install_root.to_path_buf(),
        PluginDirSource::Installed,
    ))
}

impl AppPaths {
    /// Resolve app-owned paths from explicit inputs.
    pub fn resolve(inputs: &PathsInputs) -> Result<Self, PlatformError> {
        let home = inputs
            .home
            .as_deref()
            .ok_or_else(|| PlatformError::message("cannot determine the user home directory"))?;
        let install_root = inputs
            .install_root
            .clone()
            .or_else(|| inputs.development.then(repository_root))
            .ok_or_else(|| {
                PlatformError::message("cannot determine the app install or resource root")
            })?;

        let (data_dir, log_dir, opencode_config_path) = match inputs.family {
            PlatformFamily::Linux => {
                let data = inputs
                    .xdg_data_home
                    .clone()
                    .unwrap_or_else(|| home.join(".local").join("share"));
                let state = inputs
                    .xdg_state_home
                    .clone()
                    .unwrap_or_else(|| home.join(".local").join("state"));
                let config = inputs
                    .xdg_config_home
                    .clone()
                    .unwrap_or_else(|| home.join(".config"));
                (
                    data.join("made"),
                    state.join("made").join("log"),
                    config.join("opencode").join("cli.json"),
                )
            }
            PlatformFamily::MacOs => {
                let application_support = home.join("Library").join("Application Support");
                (
                    application_support.join("Made"),
                    home.join("Library").join("Logs").join("Made"),
                    application_support.join("opencode").join("cli.json"),
                )
            }
            PlatformFamily::Windows => {
                let roaming = inputs
                    .app_data
                    .clone()
                    .unwrap_or_else(|| home.join("AppData").join("Roaming"));
                let local = inputs
                    .local_app_data
                    .clone()
                    .unwrap_or_else(|| home.join("AppData").join("Local"));
                (
                    roaming.join("Made"),
                    local.join("Made").join("logs"),
                    roaming.join("opencode").join("cli.json"),
                )
            }
        };

        let installed_plugin = install_root.join("plugins").join("opencode-made");
        let (plugin_dir, plugin_root, plugin_source) =
            resolve_plugin_dir(inputs, &install_root, &installed_plugin)?;

        let user_token = sanitize_user_token(&inputs.user_token);
        let temp_dir = match inputs.family {
            PlatformFamily::Linux => inputs
                .xdg_runtime_dir
                .as_ref()
                .filter(|runtime| !runtime.as_os_str().is_empty() && runtime.has_root())
                .map(|runtime| runtime.join(format!("made-{user_token}")))
                .unwrap_or_else(|| data_dir.join(format!("temp-{user_token}"))),
            PlatformFamily::MacOs | PlatformFamily::Windows => {
                inputs.temp_dir.join(format!("made-{user_token}"))
            }
        };
        validate_private_temp_path(&temp_dir)?;

        Ok(Self {
            install_root: install_root.clone(),
            resource_dir: install_root,
            data_dir,
            plugin_root,
            plugin_dir,
            plugin_source,
            opencode_config_path,
            log_dir,
            temp_dir,
        })
    }

    /// Create the user-writable data, log, and private temp directories.
    ///
    /// On Unix data/log are created with mode 0700 (owner-only) and existing
    /// directories are tightened where permitted. The private temp directory is
    /// additionally validated: pre-existing symlinks/non-directories are
    /// rejected and the final mode must be 0700. The install/resource and plugin
    /// directories are app-owned installation data; Made never creates or
    /// writes them at runtime.
    pub fn ensure_directories(&self) -> Result<(), PlatformError> {
        for dir in [&self.data_dir, &self.log_dir] {
            create_private_dir_all(dir)?;
        }
        ensure_private_temp_dir(&self.temp_dir)?;
        Ok(())
    }
}

/// Reject a pre-existing temp path that is a symlink or not a directory.
fn validate_private_temp_path(path: &Path) -> Result<(), PlatformError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                return Err(PlatformError::message(format!(
                    "refusing to use symlink as private temp directory: {}",
                    path.display()
                )));
            }
            if !file_type.is_dir() {
                return Err(PlatformError::message(format!(
                    "refusing to use non-directory private temp path: {}",
                    path.display()
                )));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(PlatformError::io("inspect private temp path", error)),
    }
    Ok(())
}

/// Create and validate the per-user private temp directory.
fn ensure_private_temp_dir(path: &Path) -> Result<(), PlatformError> {
    validate_private_temp_path(path)?;
    create_private_dir_all(path)?;
    validate_private_temp_path(path)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| PlatformError::io("inspect private temp directory", error))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(PlatformError::message(format!(
                "private temp path is not a directory: {}",
                path.display()
            )));
        }
        let mode = metadata.permissions().mode() & 0o777;
        if mode != 0o700 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| PlatformError::io("set private temp permissions", error))?;
        }
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| PlatformError::io("inspect private temp directory", error))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(PlatformError::message(format!(
                "private temp path is not a directory: {}",
                path.display()
            )));
        }
        let mode = metadata.permissions().mode() & 0o777;
        if mode != 0o700 {
            return Err(PlatformError::message(format!(
                "private temp directory is not owner-only (mode {mode:o}): {}",
                path.display()
            )));
        }
    }

    Ok(())
}

/// Maximum size of the active diagnostics log.
///
/// The previous log may exceed this only when it preserves a legacy active file
/// that was already oversized; that content is never truncated or rewritten.
pub const DIAGNOSTICS_LOG_MAX_BYTES: u64 = 1024 * 1024;

/// Default maximum time to wait for the diagnostics log lock.
pub const DIAGNOSTICS_LOG_LOCK_TIMEOUT: Duration = Duration::from_secs(1);

/// Upper bound for a caller-supplied log-lock wait.
pub const DIAGNOSTICS_LOG_MAX_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Suffix of the single retained previous diagnostics log.
pub const DIAGNOSTICS_LOG_BACKUP_SUFFIX: &str = ".1";

/// Suffix of the persistent diagnostics log lock file.
pub const DIAGNOSTICS_LOG_LOCK_SUFFIX: &str = ".lock";

/// Severity of a diagnostics-log retention or write issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogIssueSeverity {
    /// Retention may be weakened, but the operation continues.
    Warning,
    /// The operation was refused or failed.
    Error,
}

/// A bounded, user-visible diagnostics-log issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogIssue {
    /// Warning or error.
    pub severity: LogIssueSeverity,
    /// Sanitized, user-readable explanation.
    pub message: String,
}

impl LogIssue {
    /// Build a warning issue.
    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: LogIssueSeverity::Warning,
            message: message.into(),
        }
    }

    /// Build an error issue.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            severity: LogIssueSeverity::Error,
            message: message.into(),
        }
    }
}

/// Result of one serialized, rotation-aware log append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogAppendOutcome {
    /// The record was appended while the lock was held.
    Written {
        /// Whether the active log was renamed to the previous-log path first.
        rotated: bool,
        /// A non-fatal retention issue (for example a previous-log mode that
        /// could not be tightened).
        issue: Option<LogIssue>,
    },
    /// The record and both logs were left untouched.
    Refused {
        /// Error explaining why the append was refused.
        issue: LogIssue,
    },
}

impl LogAppendOutcome {
    fn refused(message: impl Into<String>) -> Self {
        Self::Refused {
            issue: LogIssue::error(message),
        }
    }
}

/// Previous-log path derived from the active log path (`<log>.1`).
pub fn previous_private_log_path(path: impl AsRef<Path>) -> PathBuf {
    let mut name = path.as_ref().as_os_str().to_owned();
    name.push(DIAGNOSTICS_LOG_BACKUP_SUFFIX);
    PathBuf::from(name)
}

/// Lock path derived from the active log path (`<log>.lock`).
pub fn private_log_lock_path(path: impl AsRef<Path>) -> PathBuf {
    let mut name = path.as_ref().as_os_str().to_owned();
    name.push(DIAGNOSTICS_LOG_LOCK_SUFFIX);
    PathBuf::from(name)
}

/// Append one record under a bounded lock wait, rotating first when needed.
///
/// Every append holds the app-owned lock. The lock is acquired with `try_lock`
/// for at most [`DIAGNOSTICS_LOG_LOCK_TIMEOUT`] (caller-supplied waits are
/// capped at [`DIAGNOSTICS_LOG_MAX_LOCK_TIMEOUT`]), so a stuck peer can delay
/// one workspace's cleanup by at most that bound. The record is written as one
/// `write_all` call; that call is not failure-atomic on a partial I/O error.
/// When `current_len + record.len() > max_bytes`, the active log is renamed
/// directly over the previous log (`<path>.1`); the previous log is never
/// removed first and existing bytes are never truncated, rewritten, or copied.
/// A missing active log is created. On POSIX the replacement rename is atomic;
/// Windows replacement and whole-file lock behavior are cross-compiled but
/// unverified.
///
/// For current-version appends that take the lock, the active log is at most
/// `max_bytes`. A mixed-version or non-cooperating writer can exceed it. The
/// previous log may exceed `max_bytes` only when it preserves a legacy active
/// file that was already oversized; that content is preserved byte-for-byte.
/// Every successful append tightens an existing regular `.1` to 0600.
///
/// Refusal behavior (both logs untouched, no append):
/// - the lock path is unsafe/unavailable, or lock acquisition times out;
/// - the active log is a symlink or non-regular file;
/// - rotation is needed but the previous-log path is unsafe/uninspectable;
/// - rotation is needed but the rename fails;
/// - the record itself is at or above the threshold.
///
/// An unsafe `.1` is irrelevant when no rotation is needed. An unexpected
/// open/write failure after a successful rotation returns an error naming the
/// previous log, which still holds the earlier content. The lock file is
/// persistent and is never deleted. No `fsync` is issued.
pub fn append_private_log_rotated(
    path: impl AsRef<Path>,
    record: &[u8],
    max_bytes: u64,
) -> Result<LogAppendOutcome, PlatformError> {
    append_private_log_rotated_with_rename(
        path,
        record,
        max_bytes,
        DIAGNOSTICS_LOG_LOCK_TIMEOUT,
        |from: &Path, to: &Path| std::fs::rename(from, to),
    )
}

/// Append with an explicit bounded lock wait. Intended for tests; the wait is
/// capped at [`DIAGNOSTICS_LOG_MAX_LOCK_TIMEOUT`].
pub fn append_private_log_rotated_with_timeout(
    path: impl AsRef<Path>,
    record: &[u8],
    max_bytes: u64,
    lock_timeout: Duration,
) -> Result<LogAppendOutcome, PlatformError> {
    append_private_log_rotated_with_rename(
        path,
        record,
        max_bytes,
        lock_timeout,
        |from: &Path, to: &Path| std::fs::rename(from, to),
    )
}

/// Append with an injected rename operation.
///
/// This is a public seam for deterministic rotation-failure tests; production
/// callers use [`append_private_log_rotated`]. The rename closure is only called
/// after the previous-log path has been validated.
#[doc(hidden)]
pub fn append_private_log_rotated_with_rename<F>(
    path: impl AsRef<Path>,
    record: &[u8],
    max_bytes: u64,
    lock_timeout: Duration,
    rename: F,
) -> Result<LogAppendOutcome, PlatformError>
where
    F: FnOnce(&Path, &Path) -> io::Result<()>,
{
    let path = path.as_ref();
    let lock_timeout = lock_timeout.min(DIAGNOSTICS_LOG_MAX_LOCK_TIMEOUT);
    if record.len() as u64 >= max_bytes {
        return Ok(LogAppendOutcome::refused(format!(
            "refusing to write a {}-byte diagnostics record at the {max_bytes}-byte log threshold",
            record.len()
        )));
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            create_private_dir_all(parent)?;
        }
    }

    let lock = match open_private_log_lock(path) {
        Ok(file) => file,
        Err(error) => {
            return Ok(LogAppendOutcome::refused(format!(
                "diagnostics log lock unavailable ({error}); refusing to append"
            )));
        }
    };
    if let Err(error) = acquire_log_lock(&lock, lock_timeout) {
        return Ok(LogAppendOutcome::refused(format!(
            "diagnostics log lock unavailable ({error}); refusing to append"
        )));
    }

    let (current_len, current_exists) = match inspect_current_log(path) {
        Ok(inspected) => inspected,
        Err(error) => return Ok(LogAppendOutcome::refused(error.to_string())),
    };
    let mut rotated = false;
    let mut issue = None;
    if current_exists && current_len.saturating_add(record.len() as u64) > max_bytes {
        match rotate_current_log_with(path, rename) {
            Ok(()) => {
                rotated = true;
                issue = tighten_existing_previous_log(path);
            }
            Err(refusal) => return Ok(LogAppendOutcome::Refused { issue: refusal }),
        }
    }

    if !rotated {
        issue = tighten_existing_previous_log(path);
    }

    let backup = previous_private_log_path(path);
    let mut file = open_private_log(path).map_err(|error| {
        if rotated {
            PlatformError::message(format!(
                "diagnostics log append failed after rotation; previous log preserved at {}: {error}",
                backup.display()
            ))
        } else {
            error
        }
    })?;
    file.write_all(record).map_err(|error| {
        if rotated {
            PlatformError::message(format!(
                "diagnostics log append failed after rotation; previous log preserved at {}: {error}",
                backup.display()
            ))
        } else {
            PlatformError::io("append private log record", error)
        }
    })?;
    let _ = lock.unlock();

    Ok(LogAppendOutcome::Written { rotated, issue })
}

/// Tighten an existing regular previous log under the lock.
fn tighten_existing_previous_log(path: &Path) -> Option<LogIssue> {
    let backup = previous_private_log_path(path);
    if !std::fs::symlink_metadata(&backup).is_ok_and(|metadata| metadata.is_file()) {
        return None;
    }
    tighten_private_log_permissions(&backup).err().map(|error| {
        LogIssue::warning(format!(
            "previous diagnostics log permissions could not be tightened: {error}"
        ))
    })
}

/// Wait for the log lock with a bounded `try_lock` retry loop.
fn acquire_log_lock(file: &std::fs::File, timeout: Duration) -> Result<(), io::Error> {
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) => {
                let elapsed = started.elapsed();
                if elapsed >= timeout {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("lock wait exceeded {} ms", timeout.as_millis()),
                    ));
                }
                let remaining = timeout.saturating_sub(elapsed);
                thread::sleep(remaining.min(Duration::from_millis(5)));
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }
    }
}

/// Validate the active path and return its length when it exists.
fn inspect_current_log(path: &Path) -> Result<(u64, bool), PlatformError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(PlatformError::message(format!(
                    "refusing to use symlink as a private log: {}",
                    path.display()
                )));
            }
            if !metadata.is_file() {
                return Err(PlatformError::message(format!(
                    "refusing to use non-regular private log path: {}",
                    path.display()
                )));
            }
            Ok((metadata.len(), true))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok((0, false)),
        Err(error) => Err(PlatformError::io("inspect private log path", error)),
    }
}

/// Refuse to replace an unsafe previous-log path.
fn inspect_backup_log(path: &Path) -> Result<(), LogIssue> {
    let backup = previous_private_log_path(path);
    match std::fs::symlink_metadata(&backup) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(LogIssue::error(format!(
                    "diagnostics log rotation refused: previous log path is not a regular file: {}",
                    backup.display()
                )));
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(LogIssue::error(format!(
            "diagnostics log rotation refused: could not inspect previous log {}: {error}",
            backup.display()
        ))),
    }
}

/// Rotate the active log over the previous log, or return an error issue.
fn rotate_current_log_with(
    path: &Path,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<(), LogIssue> {
    inspect_backup_log(path)?;
    rename(path, &previous_private_log_path(path)).map_err(|error| {
        LogIssue::error(format!(
            "diagnostics log rotation refused ({error}); active and previous logs left untouched"
        ))
    })
}

/// Tighten a regular private log file to 0600 on Unix.
///
/// A missing path is accepted. Symlinks and non-regular files are refused; the
/// caller decides whether that is fatal for its operation.
pub fn tighten_private_log_permissions(path: impl AsRef<Path>) -> Result<(), PlatformError> {
    let path = path.as_ref();
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(PlatformError::message(format!(
                    "refusing to tighten symlink as a private log: {}",
                    path.display()
                )));
            }
            if !metadata.is_file() {
                return Err(PlatformError::message(format!(
                    "refusing to tighten non-regular private log path: {}",
                    path.display()
                )));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(PlatformError::io("inspect private log path", error)),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| PlatformError::io("set private log permissions", error))?;
    }
    Ok(())
}

/// Open or create the persistent lock file with read/write access.
///
/// Windows whole-file locks require a read+write handle; an append-only handle
/// is not sufficient. The file is mode 0600 on Unix, rejects symlinks through
/// `O_NOFOLLOW`, and is never removed by the diagnostics path.
fn open_private_log_lock(path: &Path) -> Result<std::fs::File, PlatformError> {
    let lock_path = private_log_lock_path(path);
    match std::fs::symlink_metadata(&lock_path) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                return Err(PlatformError::message(format!(
                    "refusing to open symlink as a private log lock: {}",
                    lock_path.display()
                )));
            }
            if !file_type.is_file() {
                return Err(PlatformError::message(format!(
                    "refusing to open non-regular private log lock: {}",
                    lock_path.display()
                )));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(PlatformError::io("inspect private log lock", error)),
    }

    let file = open_lock_private(&lock_path)?;
    let metadata = file
        .metadata()
        .map_err(|error| PlatformError::io("inspect private log lock", error))?;
    if !metadata.is_file() {
        return Err(PlatformError::message(format!(
            "refusing to open non-regular private log lock: {}",
            lock_path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| PlatformError::io("set private log lock permissions", error))?;
    }
    Ok(file)
}

#[cfg(unix)]
fn open_lock_private(path: &Path) -> Result<std::fs::File, PlatformError> {
    use std::os::unix::fs::OpenOptionsExt;

    std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| PlatformError::io("open private log lock", error))
}

#[cfg(not(unix))]
fn open_lock_private(path: &Path) -> Result<std::fs::File, PlatformError> {
    std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| PlatformError::io("open private log lock", error))
}

/// Open or create an append-only private log file.
///
/// The file is created with mode 0600 on Unix and existing files are tightened
/// to 0600. The parent directory is created with mode 0700 when needed.
/// Symlinks and non-regular files are rejected; on Unix the open uses
/// `O_NOFOLLOW` so a final path component swapped to a symlink after the
/// metadata check still fails. Windows uses the standard Rust file open plus
/// the same symlink/regular-file checks; a path swapped to a reparse point in
/// the check/open race is a documented residual.
///
/// The returned file is opened for append, so multiple writers preserve each
/// other's data and the file is never truncated by diagnostics.
pub fn open_private_log(path: impl AsRef<Path>) -> Result<std::fs::File, PlatformError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            create_private_dir_all(parent)?;
        }
    }

    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                return Err(PlatformError::message(format!(
                    "refusing to open symlink as a private log: {}",
                    path.display()
                )));
            }
            if !file_type.is_file() {
                return Err(PlatformError::message(format!(
                    "refusing to open non-regular private log path: {}",
                    path.display()
                )));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(PlatformError::io("inspect private log path", error)),
    }

    let file = open_append_private(path)?;
    let metadata = file
        .metadata()
        .map_err(|error| PlatformError::io("inspect private log file", error))?;
    if !metadata.is_file() {
        return Err(PlatformError::message(format!(
            "refusing to open non-regular private log path: {}",
            path.display()
        )));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| PlatformError::io("set private log permissions", error))?;
    }

    Ok(file)
}

#[cfg(unix)]
fn open_append_private(path: &Path) -> Result<std::fs::File, PlatformError> {
    use std::os::unix::fs::OpenOptionsExt;

    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| PlatformError::io("open private log", error))
}

#[cfg(not(unix))]
fn open_append_private(path: &Path) -> Result<std::fs::File, PlatformError> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| PlatformError::io("open private log", error))
}

/// Create a directory tree with owner-only permissions on Unix.
fn create_private_dir_all(path: &Path) -> Result<(), PlatformError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(|error| PlatformError::io("create app directory", error))?;
        // `mode` only applies to directories created by this call. Tighten an
        // existing app-owned directory as a best effort; do not fail startup if
        // the caller does not own it.
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
            .map_err(|error| PlatformError::io("create app directory", error))
    }
}

/// Text clipboard behind a trait so tests never require a display server.
pub trait Clipboard: Send {
    /// Read UTF-8 text.
    fn get_text(&mut self) -> Result<String, PlatformError>;

    /// Replace the clipboard contents with UTF-8 text.
    fn set_text(&mut self, text: &str) -> Result<(), PlatformError>;
}

/// In-memory clipboard for tests and headless contexts.
///
/// Debug output reports the text length, not the text: clipboard contents can
/// include secrets copied by the user.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct InMemoryClipboard {
    text: String,
}

impl fmt::Debug for InMemoryClipboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InMemoryClipboard")
            .field("text_len", &self.text.len())
            .finish()
    }
}

impl InMemoryClipboard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_text(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl Clipboard for InMemoryClipboard {
    fn get_text(&mut self) -> Result<String, PlatformError> {
        Ok(self.text.clone())
    }

    fn set_text(&mut self, text: &str) -> Result<(), PlatformError> {
        self.text.clear();
        self.text.push_str(text);
        Ok(())
    }
}

/// Native system clipboard backed by `arboard`.
///
/// Construction fails cleanly when no display server is available, which is
/// why the platform contract tests use [`InMemoryClipboard`] instead.
pub struct SystemClipboard {
    inner: arboard::Clipboard,
}

impl fmt::Debug for SystemClipboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SystemClipboard").finish_non_exhaustive()
    }
}

impl SystemClipboard {
    pub fn new() -> Result<Self, PlatformError> {
        arboard::Clipboard::new()
            .map(|inner| Self { inner })
            .map_err(|err| {
                PlatformError::Unsupported(format!("system clipboard unavailable: {err}"))
            })
    }
}

impl Clipboard for SystemClipboard {
    fn get_text(&mut self) -> Result<String, PlatformError> {
        self.inner
            .get_text()
            .map_err(|err| PlatformError::message(format!("clipboard read failed: {err}")))
    }

    fn set_text(&mut self, text: &str) -> Result<(), PlatformError> {
        self.inner
            .set_text(text.to_owned())
            .map_err(|err| PlatformError::message(format!("clipboard write failed: {err}")))
    }
}

/// Result of normalizing one IME event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeEvent {
    /// Replace the current pre-edit composition.
    Preedit(String),
    /// Commit text to the terminal.
    Commit(String),
    /// Cancel the composition without committing.
    Cancel,
}

/// Basic IME composition state.
///
/// Windowing layers differ in how often they emit a commit: some send the
/// committed string once, others also repeat the final pre-edit as a commit.
/// The normalizer keeps a single pending composition and suppresses an
/// immediately repeated commit that was not preceded by a new pre-edit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImeComposition {
    preedit: String,
    last_commit: Option<String>,
}

impl ImeComposition {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current pre-edit text.
    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    /// Whether a composition is in progress.
    pub fn is_composing(&self) -> bool {
        !self.preedit.is_empty()
    }

    /// Apply an event and return committed text, if the event produced any.
    pub fn apply(&mut self, event: ImeEvent) -> Option<String> {
        match event {
            ImeEvent::Preedit(text) => {
                self.preedit = text;
                self.last_commit = None;
                None
            }
            ImeEvent::Commit(text) => {
                let came_from_preedit = !self.preedit.is_empty();
                let duplicate =
                    !came_from_preedit && self.last_commit.as_deref() == Some(text.as_str());
                self.preedit.clear();
                if text.is_empty() || duplicate {
                    None
                } else {
                    self.last_commit = Some(text.clone());
                    Some(text)
                }
            }
            ImeEvent::Cancel => {
                self.preedit.clear();
                self.last_commit = None;
                None
            }
        }
    }

    /// Drop all composition state.
    pub fn reset(&mut self) {
        self.preedit.clear();
        self.last_commit = None;
    }
}

/// Platform capabilities consumed by `made-terminal` and `made-app`.
pub trait Platform: Send + Sync {
    /// Host family of this backend.
    fn family(&self) -> PlatformFamily;

    /// Resolve the shell to launch.
    ///
    /// `requested` wins when present. Otherwise the platform uses
    /// [`SHELL_ENV`], then its conventional default (`$SHELL` or `/bin/sh` on
    /// POSIX, `%COMSPEC%` or `cmd.exe` on Windows).
    fn resolve_shell(&self, requested: Option<&OsStr>) -> Result<ShellSpec, PlatformError>;

    /// Build a complete child environment from the current process environment
    /// plus the terminal defaults, overrides, and removals.
    fn environment(&self, overrides: &[(OsString, OsString)], removed: &[OsString]) -> Environment;

    /// Resolve app-owned paths for the host.
    fn app_paths(&self) -> Result<AppPaths, PlatformError>;

    /// Resolve and validate the current user's home directory.
    ///
    /// This is the only home-directory capability; UI and workspace policy
    /// must not read `HOME` or `USERPROFILE` directly. Failure is classified so
    /// the app can show a non-blocking terminal fallback.
    fn home_dir(&self) -> Result<PathBuf, HomeDirectoryError> {
        host_home_dir(self.family())
    }

    /// Open the system clipboard.
    fn clipboard(&self) -> Result<Box<dyn Clipboard>, PlatformError>;

    /// Spawn a PTY child.
    fn spawn_pty(&self, request: &PtyRequest) -> Result<Box<dyn PtySession>, PlatformError>;

    /// Normalize a basic IME event through the shared composition state.
    fn normalize_ime(&self, composition: &mut ImeComposition, event: ImeEvent) -> Option<String> {
        composition.apply(event)
    }

    /// Last-chance process cleanup hook for application shutdown.
    ///
    /// Individual sessions are terminated with [`PtySession::terminate`]; this
    /// hook exists for backends that need to release OS-wide resources.
    fn cleanup(&self) -> Result<(), PlatformError> {
        Ok(())
    }
}

/// Build the host [`Platform`] backend.
#[cfg(target_os = "linux")]
pub use linux::LinuxPlatform as HostPlatform;
#[cfg(target_os = "macos")]
pub use macos::MacOsPlatform as HostPlatform;
#[cfg(target_os = "windows")]
pub use windows::WindowsPlatform as HostPlatform;

/// Construct the host backend.
pub fn host_platform() -> HostPlatform {
    HostPlatform::new()
}

/// A compatible TUI that Made can launch directly in a new tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CompatibleTui {
    /// OpenCode v2. Detection checks the executable name only; the version is
    /// not verified.
    OpenCode,
}

impl CompatibleTui {
    /// Every TUI Made knows how to launch, in new-tab menu order.
    pub const ALL: &'static [Self] = &[Self::OpenCode];

    /// Executable name resolved through `PATH`.
    pub const fn program(self) -> &'static str {
        match self {
            Self::OpenCode => "opencode",
        }
    }

    /// Display label for the new-tab menu.
    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenCode => "OpenCode",
        }
    }
}

/// Detect compatible TUIs on the current process `PATH`.
///
/// This is a metadata-only executable lookup intended to run once on a
/// background thread. It never spawns the program, probes a version, or waits
/// for a service. Callers must not make shell or explorer startup depend on the
/// result.
pub fn detect_compatible_tuis() -> Vec<CompatibleTui> {
    let path = env::var_os("PATH").unwrap_or_default();
    detect_compatible_tuis_in_path(&path)
}

/// Detect compatible TUIs in an explicit `PATH` value.
///
/// Empty and relative path segments are ignored rather than resolved against
/// the current directory, so detection cannot pick up an untrusted relative
/// executable.
pub fn detect_compatible_tuis_in_path(path: impl AsRef<OsStr>) -> Vec<CompatibleTui> {
    CompatibleTui::ALL
        .iter()
        .copied()
        .filter(|tui| find_executable_on_path(OsStr::new(tui.program()), path.as_ref()).is_some())
        .collect()
}

/// Find an executable named `program` in an explicit `PATH` value.
///
/// Only bare names are searched. A `program` with more than one path component
/// (for example `./opencode` or `/usr/bin/opencode`) returns `None`, so the
/// lookup never resolves from the current directory. The lookup uses filesystem
/// metadata only and never executes the candidate. Empty and relative path
/// segments are ignored. On Windows, a name that already has an extension is
/// used as given. A name without one tries only `.exe` and `.com`: the
/// extensionless `sh` shim that npm writes beside `opencode.cmd` and
/// `opencode.ps1` is never matched, and neither is a batch file, because
/// CreateProcess cannot run either.
pub fn find_executable_on_path(program: &OsStr, path: &OsStr) -> Option<PathBuf> {
    let program = Path::new(program);
    if program.components().count() > 1 {
        return None;
    }
    let file_name = program.file_name()?;
    for directory in env::split_paths(path) {
        if directory.as_os_str().is_empty() || directory.is_relative() {
            continue;
        }
        for candidate in executable_candidates(directory.join(file_name), file_name) {
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(unix)]
fn executable_candidates(candidate: PathBuf, _file_name: &OsStr) -> Vec<PathBuf> {
    vec![candidate]
}

#[cfg(windows)]
fn executable_candidates(candidate: PathBuf, file_name: &OsStr) -> Vec<PathBuf> {
    if Path::new(file_name).extension().is_some() {
        return vec![candidate];
    }
    let mut candidates = Vec::with_capacity(2);
    for extension in ["exe", "com"] {
        let mut with_extension = candidate.clone();
        with_extension.set_extension(extension);
        candidates.push(with_extension);
    }
    candidates
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

/// Shell resolution shared by the POSIX and Windows backends.
pub(crate) fn resolve_shell_for(
    family: PlatformFamily,
    requested: Option<&OsStr>,
) -> Result<ShellSpec, PlatformError> {
    let requested = requested
        .map(OsString::from)
        .filter(|value| !value.is_empty())
        .or_else(|| env::var_os(SHELL_ENV).filter(|value| !value.is_empty()));
    if let Some(program) = requested {
        return Ok(ShellSpec::new(program));
    }

    let candidate = match family {
        PlatformFamily::Linux | PlatformFamily::MacOs => env::var_os("SHELL")
            .filter(|value| !value.is_empty())
            .filter(|value| Path::new(value).is_file())
            .unwrap_or_else(|| OsString::from("/bin/sh")),
        PlatformFamily::Windows => env::var_os("COMSPEC")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| OsString::from("cmd.exe")),
    };

    Ok(ShellSpec::new(candidate))
}

/// Environment construction shared by all backends.
pub(crate) fn host_environment(
    overrides: &[(OsString, OsString)],
    removed: &[OsString],
) -> Environment {
    let mut environment = Environment::current().with_terminal_defaults();
    for key in removed {
        environment.remove(key);
    }
    for (key, value) in overrides {
        environment.set(key.clone(), value.clone());
    }
    environment
}

/// Windows shells do not accept the extended-length `\\?\` path prefix that
/// `fs::canonicalize` returns. Keep canonical paths for app comparisons, but
/// hand the shell a normal absolute path so `cmd.exe`/PowerShell can start in
/// the workspace.
fn shell_compatible_cwd(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

/// Spawn a PTY through `portable-pty`, which uses ConPTY on Windows.
pub(crate) fn spawn_portable_pty(
    request: &PtyRequest,
) -> Result<Box<dyn PtySession>, PlatformError> {
    use portable_pty::{CommandBuilder, PtySize as PortablePtySize};

    if request.program.is_empty() {
        return Err(PlatformError::message(
            "cannot spawn a PTY without a program",
        ));
    }

    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PortablePtySize {
            rows: request.size.rows.max(1),
            cols: request.size.cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|err| PlatformError::message(format!("open PTY failed: {err:#}")))?;

    let mut command = CommandBuilder::new(&request.program);
    command.args(&request.args);
    command.cwd(shell_compatible_cwd(&request.cwd));
    command.env_clear();
    for (key, value) in &request.env {
        command.env(key, value);
    }
    for key in &request.env_remove {
        command.env_remove(key);
    }

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|err| PlatformError::message(format!("spawn PTY child failed: {err:#}")))?;

    // The parent must not keep the slave side open, otherwise EOF on the master
    // never arrives after the child exits.
    drop(pair.slave);

    let pid = child.process_id();
    #[cfg(unix)]
    let process_group = pair
        .master
        .process_group_leader()
        .or_else(|| pid.map(|pid| pid.min(i32::MAX as u32) as i32));
    // portable-pty calls setsid() in the child before exec, so the child is
    // the leader of its own session and its session id equals its pid. This is
    // the workspace-scoped ownership key used for descendant cleanup.
    #[cfg(unix)]
    let session_id = pid
        .map(|pid| pid.min(i32::MAX as u32) as libc::pid_t)
        .filter(|pid| *pid > 1);
    #[cfg(windows)]
    let process_job = pid.and_then(crate::windows::ProcessJob::assign);
    Ok(Box::new(PortablePtySession {
        master: Mutex::new(pair.master),
        child: Mutex::new(child),
        reader_taken: AtomicBool::new(false),
        writer_taken: AtomicBool::new(false),
        exited: Mutex::new(None),
        pid,
        #[cfg(unix)]
        process_group,
        #[cfg(unix)]
        session_id,
        #[cfg(unix)]
        groups_cleaned: AtomicBool::new(false),
        #[cfg(windows)]
        process_job,
        #[cfg(windows)]
        job_cleaned: AtomicBool::new(false),
    }))
}

struct PortablePtySession {
    master: Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    reader_taken: AtomicBool,
    writer_taken: AtomicBool,
    exited: Mutex<Option<ExitStatus>>,
    pid: Option<u32>,
    /// Process group/session leader of the PTY child on Unix.
    #[cfg(unix)]
    process_group: Option<libc::pid_t>,
    /// Session id owned by this PTY child on Unix.
    #[cfg(unix)]
    session_id: Option<libc::pid_t>,
    /// Whether the SIGHUP/SIGKILL process-group cleanup already ran.
    #[cfg(unix)]
    groups_cleaned: AtomicBool,
    /// Kill-on-close Job Object owning the ConPTY child and descendants.
    #[cfg(windows)]
    process_job: Option<crate::windows::ProcessJob>,
    /// Whether the Job Object termination already ran.
    #[cfg(windows)]
    job_cleaned: AtomicBool,
}

/// Process groups belonging to `session_id`, discovered from `/proc`.
///
/// `/proc/<pid>/stat` stores `pgrp` and `session` after the parenthesized
/// command name. Splitting at the last `)` keeps command names that contain
/// spaces or parentheses from shifting the numeric fields.
#[cfg(target_os = "linux")]
fn process_groups_in_session(session_id: libc::pid_t) -> Vec<libc::pid_t> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut groups = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        if pid <= 1 {
            continue;
        }
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some((_, rest)) = stat.rsplit_once(')') else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let _state = fields.next();
        let _parent = fields.next();
        let Some(process_group) = fields
            .next()
            .and_then(|value| value.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        let Some(session) = fields
            .next()
            .and_then(|value| value.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        if session == session_id {
            groups.push(process_group);
        }
    }
    groups
}

/// Process groups belonging to `session_id`, discovered with `ps` and
/// `getsid(2)` on macOS.
#[cfg(target_os = "macos")]
fn process_groups_in_session(session_id: libc::pid_t) -> Vec<libc::pid_t> {
    let Ok(output) = std::process::Command::new("/bin/ps")
        .args(["-axo", "pid=,pgid="])
        .stdin(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let mut groups = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.split_whitespace();
        let Some(pid) = fields
            .next()
            .and_then(|value| value.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        let Some(process_group) = fields
            .next()
            .and_then(|value| value.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        // SAFETY: getsid only reads kernel process metadata for an existing pid
        // and is safe to call from any thread.
        if process_group > 1 && unsafe { libc::getsid(pid) } == session_id {
            groups.push(process_group);
        }
    }
    groups
}

/// Other Unix targets have no session scanner; keep the stored-group and
/// foreground-group cleanup behavior.
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn process_groups_in_session(_session_id: libc::pid_t) -> Vec<libc::pid_t> {
    Vec::new()
}

impl PortablePtySession {
    fn lock_child(&self) -> std::sync::MutexGuard<'_, Box<dyn portable_pty::Child + Send + Sync>> {
        self.child.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// Current foreground process group of the PTY, queried at cleanup time.
    #[cfg(unix)]
    fn foreground_process_group(&self) -> Option<libc::pid_t> {
        let master = self.master.lock().unwrap_or_else(|err| err.into_inner());
        let fd = master.as_raw_fd()?;
        let process_group = unsafe { libc::tcgetpgrp(fd) };
        (process_group > 1).then_some(process_group)
    }

    /// Session owned by this PTY child, unless it is unusable or is Made's own
    /// session.
    #[cfg(unix)]
    fn session_for_cleanup(&self) -> Option<libc::pid_t> {
        let session = self.session_id?;
        // SAFETY: getsid(0) only reads the calling thread's session id.
        let own_session = unsafe { libc::getsid(0) };
        (session > 1 && session != own_session).then_some(session)
    }

    /// Process groups to clean up, captured at kill time.
    ///
    /// The stored group catches descendants in the shell's own group even
    /// after the shell exits. Re-querying `tcgetpgrp` catches an interactive
    /// shell's current foreground job-control group. Scanning the PTY child's
    /// session catches additional job-control groups, including HUP-ignoring
    /// background jobs under default interactive job control. Never include
    /// PID 1 or Made's own process group.
    #[cfg(unix)]
    fn process_groups_for_cleanup(&self) -> Vec<libc::pid_t> {
        let own_group = unsafe { libc::getpgrp() };
        let mut groups = Vec::new();
        if let Some(process_group) = self.process_group {
            groups.push(process_group);
        }
        if let Some(foreground) = self.foreground_process_group() {
            groups.push(foreground);
        }
        if let Some(session) = self.session_for_cleanup() {
            groups.extend(process_groups_in_session(session));
        }
        groups.retain(|process_group| *process_group > 1 && *process_group != own_group);
        groups.sort_unstable();
        groups.dedup();
        groups
    }

    #[cfg(unix)]
    fn signal_process_groups(groups: &[libc::pid_t], signal: libc::c_int) {
        for process_group in groups {
            // Negative pid targets the process group. ESRCH is expected when
            // the group is already gone; callers still reap through try_wait.
            unsafe {
                libc::kill(-process_group, signal);
            }
        }
    }
}

impl PtySession for PortablePtySession {
    fn take_reader(&self) -> Result<Box<dyn Read + Send>, PlatformError> {
        if self.reader_taken.swap(true, Ordering::SeqCst) {
            return Err(PlatformError::message("PTY reader was already taken"));
        }
        self.master
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .try_clone_reader()
            .map_err(|err| PlatformError::message(format!("open PTY reader failed: {err:#}")))
    }

    fn take_writer(&self) -> Result<Box<dyn Write + Send>, PlatformError> {
        if self.writer_taken.swap(true, Ordering::SeqCst) {
            return Err(PlatformError::message("PTY writer was already taken"));
        }
        self.master
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take_writer()
            .map_err(|err| PlatformError::message(format!("open PTY writer failed: {err:#}")))
    }

    fn resize(&self, size: PtySize) -> Result<(), PlatformError> {
        self.master
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .resize(portable_pty::PtySize {
                rows: size.rows.max(1),
                cols: size.cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| PlatformError::message(format!("PTY resize failed: {err:#}")))
    }

    fn size(&self) -> Result<PtySize, PlatformError> {
        let size = self
            .master
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get_size()
            .map_err(|err| PlatformError::message(format!("PTY size query failed: {err:#}")))?;
        Ok(PtySize {
            rows: size.rows,
            cols: size.cols,
        })
    }

    fn try_wait(&self) -> Result<Option<ExitStatus>, PlatformError> {
        if let Some(status) = self
            .exited
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
        {
            return Ok(Some(status));
        }

        let status = self
            .lock_child()
            .try_wait()
            .map_err(|err| PlatformError::io("wait for PTY child", err))?;
        let status = status.map(map_exit_status);
        if let Some(status) = &status {
            *self.exited.lock().unwrap_or_else(|err| err.into_inner()) = Some(status.clone());
        }
        Ok(status)
    }

    fn kill(&self) -> Result<(), PlatformError> {
        let exited = self
            .exited
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .is_some();

        #[cfg(unix)]
        let mut cleanup_groups = (!self.groups_cleaned.swap(true, Ordering::SeqCst))
            .then(|| self.process_groups_for_cleanup());

        #[cfg(windows)]
        let job_terminated = if !self.job_cleaned.swap(true, Ordering::SeqCst) {
            self.process_job
                .as_ref()
                .is_some_and(crate::windows::ProcessJob::terminate)
        } else {
            false
        };
        #[cfg(not(windows))]
        let job_terminated = false;

        // Even when the direct shell already exited, its stored process group
        // and session may still contain descendants. Signal the stored group,
        // the current foreground job group, and every group in the session
        // before reaping/killing the child. The foreground group is captured
        // once because killing the shell changes what `tcgetpgrp` reports.
        #[cfg(unix)]
        if let Some(groups) = &cleanup_groups {
            PortablePtySession::signal_process_groups(groups, libc::SIGHUP);
        }

        let result = if exited || job_terminated {
            Ok(())
        } else {
            self.lock_child()
                .kill()
                .map_err(|err| PlatformError::io("kill PTY child", err))
        };

        // A process that forked between the first scan and the shell kill is
        // still in the PTY session. Re-scan before the terminal SIGKILL so the
        // second signal covers the whole workspace-owned tree.
        #[cfg(unix)]
        if let Some(groups) = cleanup_groups.as_mut() {
            groups.extend(self.process_groups_for_cleanup());
            groups.sort_unstable();
            groups.dedup();
            PortablePtySession::signal_process_groups(groups, libc::SIGKILL);
        }

        if result.is_err() {
            // SIGHUP may have already made the child exit. If it has been
            // reaped, the kill race is not a cleanup failure.
            if let Ok(Some(_)) = self.try_wait() {
                return Ok(());
            }
        }
        result
    }

    /// Always run scoped descendant cleanup, including when the direct shell
    /// has already exited.
    ///
    /// The trait default returns early when `try_wait` reports an exit; this
    /// override must not, because descendants may still be in the Unix session
    /// or the Windows Job Object.
    #[cfg(any(unix, windows))]
    fn terminate(&self) -> Result<ExitStatus, PlatformError> {
        let kill_result = self.kill();
        if let Some(status) = self.try_wait()? {
            return Ok(status);
        }
        kill_result?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(PlatformError::message(
                    "PTY child did not exit after terminate",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn pid(&self) -> Option<u32> {
        self.pid
    }
}

fn map_exit_status(status: portable_pty::ExitStatus) -> ExitStatus {
    ExitStatus {
        code: status.exit_code(),
        signal: status.signal().map(str::to_owned),
    }
}

/// Convenience alias for the in-memory clipboard's default state.
pub fn in_memory_clipboard() -> InMemoryClipboard {
    InMemoryClipboard::new()
}
mod shell_integration;
pub use shell_integration::{local_cwd_path, ShellCwdCapability, ShellLaunchPlan};

#[cfg(test)]
mod diagnostics_log_tests {
    use super::*;

    #[test]
    fn rotation_failure_leaves_the_active_log_untouched() {
        let root = std::env::temp_dir().join(format!(
            "made-platform-rotation-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        let log = root.join("logs").join("integration.log");
        {
            let mut file = open_private_log(&log).expect("seed log");
            file.write_all(b"existing").expect("seed append");
        }
        let before = std::fs::read(&log).expect("read before");

        let result = rotate_current_log_with(&log, |_, _| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected rename failure",
            ))
        });

        assert!(result.is_err(), "rename failure must surface an error");
        assert_eq!(std::fs::read(&log).expect("read after"), before);
        assert!(!previous_private_log_path(&log).exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
