//! Shared read-only Git observer. No mutation or arbitrary argv entry point.
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fmt, fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const SHOW_TOPLEVEL_ARGS: &[&str] = &["rev-parse", "--show-toplevel"];
const BRANCH_ARGS: &[&str] = &["rev-parse", "--abbrev-ref", "HEAD"];
const GIT_DIR_ARGS: &[&str] = &["rev-parse", "--git-dir"];
const COMMON_DIR_ARGS: &[&str] = &["rev-parse", "--git-common-dir"];
const WORKTREE_LIST_ARGS: &[&str] = &["worktree", "list", "--porcelain"];
const COMMIT_ID_ARGS: &[&str] = &["rev-parse", "--short=8", "HEAD"];
const COMMIT_SUBJECT_ARGS: &[&str] = &["log", "-1", "--format=%s"];
const STATUS_ARGS: &[&str] = &[
    "status",
    "--porcelain=v1",
    "-z",
    "--untracked-files=all",
    "--ignore-submodules=dirty",
    "--",
    ".",
];
const FILTER_DRIVERS_UNSCOPED_ARGS: &[&str] = &[
    "config",
    "--name-only",
    "--get-regexp",
    r"^filter\..*\.(clean|process|smudge|required)$",
];
const FILTER_DRIVERS_ARGS: &[&str] = &[
    "config",
    "--show-scope",
    "--name-only",
    "--get-regexp",
    r"^filter\..*\.(clean|process|smudge|required)$",
];

/// Configuration forced on every read so repository or inherited settings
/// cannot make a read execute a program or leave this working tree.
///
/// - `core.fsmonitor=false`: no repository-chosen fsmonitor hook runs.
/// - `log.showSignature=false`: the subject read never runs `gpg.program`.
/// - `protocol.allow=never` and every `protocol.<name>.allow=never`: a
///   partial clone's lazy fetch cannot start a transport, so neither
///   `core.sshCommand` nor `remote.<name>.uploadpack` runs. Together with
///   `GIT_NO_LAZY_FETCH=1` (git 2.45 and later) no fetch is attempted.
///
/// Command-line `-c` wins over every configuration file. Status also passes
/// `--ignore-submodules=dirty`, which beats `submodule.<name>.ignore`, so it
/// never enters a submodule worktree, and status disables every filter
/// driver the repository's own configuration defines (see
/// [`GitCommand::FilterDrivers`]).
const HARDENED_CONFIG: &[&str] = &[
    "core.fsmonitor=false",
    "log.showSignature=false",
    "protocol.allow=never",
    "protocol.file.allow=never",
    "protocol.git.allow=never",
    "protocol.ssh.allow=never",
    "protocol.http.allow=never",
    "protocol.https.allow=never",
    "protocol.ftp.allow=never",
    "protocol.ftps.allow=never",
    "protocol.ext.allow=never",
    "protocol.fd.allow=never",
];

/// Inherited variables that would redirect a read to another repository,
/// index or object store, or inject configuration or an external program.
/// `GIT_CONFIG_KEY_<n>`/`GIT_CONFIG_VALUE_<n>` are removed by prefix.
const SCRUBBED_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
    "GIT_CONFIG_NOSYSTEM",
    "GIT_EXTERNAL_DIFF",
    "GIT_DIFF_OPTS",
    "GIT_EXEC_PATH",
    "GIT_ATTR_SOURCE",
];
const SCRUBBED_ENV_PREFIXES: &[&str] = &["GIT_CONFIG_KEY_", "GIT_CONFIG_VALUE_"];

/// Environment variable that overrides the git executable in Made's local app.
///
/// Intended for packaged builds and tests. An empty value falls back to `git`
/// on `PATH`. The shared observer and the remote companion never read it; only
/// the app compatibility wrapper applies this override.
pub const GIT_PROGRAM_ENV: &str = "MADE_GIT_BIN";

/// Whether Made exposes worktree lifecycle management. Always false in v1.
pub const WORKTREE_MANAGEMENT_AVAILABLE: bool = false;

/// Section title for unmerged paths. Empty sections are omitted.
pub const GROUP_MERGE: &str = "Merge Changes";
/// Section title for index changes.
pub const GROUP_STAGED: &str = "Staged Changes";
/// Section title for worktree changes that are not staged.
pub const GROUP_CHANGES: &str = "Changes";
/// Section title for untracked paths.
pub const GROUP_UNTRACKED: &str = "Untracked";

/// Complete read-only git argument allowlist.
pub const READ_ONLY_GIT_ARGUMENTS: &[&[&str]] = &[
    SHOW_TOPLEVEL_ARGS,
    BRANCH_ARGS,
    GIT_DIR_ARGS,
    COMMON_DIR_ARGS,
    WORKTREE_LIST_ARGS,
    COMMIT_ID_ARGS,
    COMMIT_SUBJECT_ARGS,
    STATUS_ARGS,
    FILTER_DRIVERS_ARGS,
    FILTER_DRIVERS_UNSCOPED_ARGS,
];

/// Read-only git command surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GitCommand {
    /// `rev-parse --show-toplevel`
    ShowToplevel,
    /// `rev-parse --abbrev-ref HEAD`
    Branch,
    /// `rev-parse --git-dir`
    GitDir,
    /// `rev-parse --git-common-dir`
    CommonDir,
    /// `worktree list --porcelain`
    WorktreeList,
    /// `rev-parse --short=8 HEAD`
    ///
    /// Git may return more than 8 characters when a shorter id is ambiguous.
    /// The header displays the first 8 and keeps the full abbreviation for hover.
    CommitId,
    /// `log -1 --format=%s`
    CommitSubject,
    /// `status --porcelain=v1 -z --untracked-files=all
    /// --ignore-submodules=dirty -- .`
    ///
    /// `all` lists every untracked file. `normal` would collapse a new
    /// directory to one entry, so those files would never get a file dot.
    /// Ignored paths stay omitted. A submodule shows when its commit
    /// changed; its worktree is never entered. The runner reads
    /// [`Self::FilterDrivers`] first and disables every driver that any
    /// repository-controlled scope defines.
    Status,
    /// `config --show-scope --name-only --get-regexp
    /// '^filter\..*\.(clean|process|smudge|required)$'`: the configured
    /// filter driver keys with their scope (git 2.26 and later). Reading
    /// configuration runs no program.
    FilterDrivers,
    /// `config --name-only --get-regexp
    /// '^filter\..*\.(clean|process|smudge|required)$'`: the same keys
    /// without scope, read only when git predates `--show-scope`. Every
    /// driver it names is disabled.
    FilterDriversUnscoped,
}

impl GitCommand {
    /// Every command the module can run.
    pub const ALL: [Self; 10] = [
        Self::ShowToplevel,
        Self::Branch,
        Self::GitDir,
        Self::CommonDir,
        Self::WorktreeList,
        Self::CommitId,
        Self::CommitSubject,
        Self::Status,
        Self::FilterDrivers,
        Self::FilterDriversUnscoped,
    ];

    /// Fixed argument vector for this command.
    pub const fn args(self) -> &'static [&'static str] {
        match self {
            Self::ShowToplevel => SHOW_TOPLEVEL_ARGS,
            Self::Branch => BRANCH_ARGS,
            Self::GitDir => GIT_DIR_ARGS,
            Self::CommonDir => COMMON_DIR_ARGS,
            Self::WorktreeList => WORKTREE_LIST_ARGS,
            Self::CommitId => COMMIT_ID_ARGS,
            Self::CommitSubject => COMMIT_SUBJECT_ARGS,
            Self::Status => STATUS_ARGS,
            Self::FilterDrivers => FILTER_DRIVERS_ARGS,
            Self::FilterDriversUnscoped => FILTER_DRIVERS_UNSCOPED_ARGS,
        }
    }

    /// Stable name for diagnostics.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ShowToplevel => "show-toplevel",
            Self::Branch => "branch",
            Self::GitDir => "git-dir",
            Self::CommonDir => "common-dir",
            Self::WorktreeList => "worktree-list",
            Self::CommitId => "commit-id",
            Self::CommitSubject => "commit-subject",
            Self::Status => "status",
            Self::FilterDrivers => "filter-drivers",
            Self::FilterDriversUnscoped => "filter-drivers-unscoped",
        }
    }
}
impl From<crate::protocol::GitRead> for GitCommand {
    fn from(value: crate::protocol::GitRead) -> Self {
        use crate::protocol::GitRead as R;
        match value {
            R::ShowToplevel => Self::ShowToplevel,
            R::Branch => Self::Branch,
            R::GitDir => Self::GitDir,
            R::CommonDir => Self::CommonDir,
            R::WorktreeList => Self::WorktreeList,
            R::CommitId => Self::CommitId,
            R::CommitSubject => Self::CommitSubject,
            R::Status => Self::Status,
        }
    }
}
/// Whether an argument vector is one of the fixed read-only commands.
pub fn is_read_only_git_args(args: &[&str]) -> bool {
    READ_ONLY_GIT_ARGUMENTS.contains(&args)
}

/// Process execution limits for one Git command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitLimits {
    /// Maximum wall-clock time for the whole command, or `None` to wait.
    pub deadline: Option<Duration>,
    /// Maximum bytes read from stdout and from stderr separately, or `None`.
    pub max_output_bytes: Option<u64>,
}

impl GitLimits {
    /// Remote companion limits: a nine-second deadline and the protocol's cap.
    pub const BOUNDED: Self = Self {
        deadline: Some(Duration::from_secs(9)),
        max_output_bytes: Some(crate::protocol::MAX_GIT_BYTES),
    };

    /// Local app compatibility limits: no deadline and no output cap.
    pub const UNBOUNDED: Self = Self {
        deadline: None,
        max_output_bytes: None,
    };
}

/// Failure to run or complete one fixed Git command.
#[derive(Debug)]
pub enum GitRunError {
    /// The process could not be started.
    Spawn(io::Error),
    /// Reading a pipe or waiting for the child failed.
    Io(io::Error),
    /// Git exited unsuccessfully.
    Failed {
        /// Exit code, or `None` when a signal terminated Git.
        code: Option<i32>,
        /// Trimmed stderr, preserving Git's diagnostic.
        stderr: String,
    },
    /// A stream exceeded [`GitLimits::max_output_bytes`].
    OutputLimit,
    /// The command exceeded [`GitLimits::deadline`].
    Deadline,
    /// The command is not one of the fixed read-only commands.
    Rejected,
    /// A configured filter driver could not be disabled, so status did not
    /// run.
    UnsafeFilter,
}

impl GitRunError {
    /// The closest [`io::ErrorKind`] for callers that classify failures.
    pub fn kind(&self) -> io::ErrorKind {
        match self {
            Self::Spawn(error) | Self::Io(error) => error.kind(),
            Self::Failed { .. } | Self::OutputLimit | Self::Rejected | Self::UnsafeFilter => {
                io::ErrorKind::Other
            }
            Self::Deadline => io::ErrorKind::TimedOut,
        }
    }

    /// Trimmed stderr captured from a failed Git process, when available.
    pub fn stderr(&self) -> &str {
        match self {
            Self::Failed { stderr, .. } => stderr,
            _ => "",
        }
    }

    /// Exit code captured from a failed Git process, when available.
    pub fn code(&self) -> Option<i32> {
        match self {
            Self::Failed { code, .. } => *code,
            _ => None,
        }
    }
}

impl fmt::Display for GitRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(formatter, "could not start git: {error}"),
            Self::Io(error) => error.fmt(formatter),
            Self::Failed {
                code: Some(code),
                stderr,
            } => write!(formatter, "git exited with {code}: {stderr}"),
            Self::Failed { code: None, stderr } => {
                write!(formatter, "git was terminated: {stderr}")
            }
            Self::OutputLimit => formatter.write_str("git output limit"),
            Self::Deadline => formatter.write_str("git read deadline"),
            Self::Rejected => formatter.write_str("git arguments are not read-only"),
            Self::UnsafeFilter => {
                formatter.write_str("a configured git filter driver cannot be disabled")
            }
        }
    }
}

impl std::error::Error for GitRunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) | Self::Spawn(error) => Some(error),
            _ => None,
        }
    }
}

impl From<GitRunError> for io::Error {
    fn from(error: GitRunError) -> Self {
        match error {
            GitRunError::Io(error) => error,
            other => io::Error::new(other.kind(), other),
        }
    }
}

/// Metadata availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataStatus {
    /// Git answered every read command used by the indicator.
    Known,
    /// The directory is not a git repository or a read failed.
    Unknown,
    /// The git executable is missing or could not be started.
    Unavailable,
}
impl MetadataStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Known => "known",
            Self::Unknown => "unknown",
            Self::Unavailable => "unavailable",
        }
    }
}
/// Best-effort read-only worktree metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeMetadata {
    /// Availability classification.
    pub status: MetadataStatus,
    /// Whether the path is inside a git worktree.
    pub is_worktree: bool,
    /// Canonical worktree top-level, when known.
    pub toplevel: Option<PathBuf>,
    /// Current branch, when attached.
    pub branch: Option<String>,
    /// Whether HEAD is detached.
    pub detached: bool,
    /// `rev-parse --short=8 HEAD` when detached. May be longer than 8 characters.
    pub commit_abbrev: Option<String>,
    /// Commit subject when detached. An empty string is a successful empty subject.
    pub commit_subject: Option<String>,
    /// Whether this directory is a linked worktree.
    pub is_linked_worktree: bool,
    /// Folder name of the worktree top-level. The REPOS header uses this, not the branch.
    pub directory_name: Option<String>,
    /// Branch-first label used by diagnostics. Not the REPOS worktree name.
    pub worktree_label: Option<String>,
    /// Git directory, normalized to an absolute path.
    pub git_dir: Option<PathBuf>,
    /// Common git directory shared by linked worktrees.
    pub common_dir: Option<PathBuf>,
    /// Number of entries in `git worktree list`.
    pub worktree_count: usize,
    /// Why metadata is unknown or unavailable.
    pub reason: Option<String>,
}

impl WorktreeMetadata {
    /// Metadata that could not be established.
    pub fn unknown(reason: impl Into<String>) -> Self {
        Self {
            status: MetadataStatus::Unknown,
            is_worktree: false,
            toplevel: None,
            branch: None,
            detached: false,
            commit_abbrev: None,
            commit_subject: None,
            is_linked_worktree: false,
            directory_name: None,
            worktree_label: None,
            git_dir: None,
            common_dir: None,
            worktree_count: 0,
            reason: Some(reason.into()),
        }
    }
    /// Metadata known to be unavailable because git cannot be run.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        let mut value = Self::unknown(reason);
        value.status = MetadataStatus::Unavailable;
        value
    }

    /// Whether the metadata is fully known.
    pub const fn is_known(&self) -> bool {
        matches!(self.status, MetadataStatus::Known)
    }

    /// Header text for the explorer; never used as markup.
    pub fn header_label(&self) -> String {
        match self.status {
            MetadataStatus::Unavailable => "git unavailable".into(),
            MetadataStatus::Unknown => "unknown git metadata".into(),
            MetadataStatus::Known => {
                let kind = if self.is_linked_worktree {
                    "linked worktree"
                } else {
                    "worktree"
                };
                match &self.branch {
                    Some(branch) => format!("{kind} · {branch}"),
                    None if self.detached => format!("{kind} · detached HEAD"),
                    None => kind.into(),
                }
            }
        }
    }
}
/// Runs the fixed read-only git commands.
#[derive(Debug, Clone)]
pub struct GitRunner {
    program: OsString,
    limits: GitLimits,
}

impl Default for GitRunner {
    fn default() -> Self {
        Self::from_env()
    }
}

impl GitRunner {
    /// Runner using `git` on `PATH` with the remote companion's bounds.
    pub fn new() -> Self {
        Self::from_env()
    }

    /// Resolve `git` on `PATH` only. `MADE_GIT_BIN` is not a program selector:
    /// an arbitrary path would bypass the closed allowlist by replacing the
    /// executable. The app compatibility wrapper applies the environment
    /// override; the companion never does. `with_program` remains available for
    /// an explicit test binary.
    pub fn from_env() -> Self {
        let resolved = made_platform::Environment::current()
            .get("PATH")
            .and_then(|path| {
                made_platform::find_executable_on_path(std::ffi::OsStr::new("git"), path)
            });
        Self::with_program(resolved.unwrap_or_else(|| PathBuf::from("git")))
    }

    /// Compatibility form of [`GitRunner::from_env`].
    ///
    /// The argument is ignored: unlike the app wrapper, the shared observer
    /// never treats an environment value as an executable override.
    pub fn from_env_value(program: Option<OsString>) -> Self {
        let _ = program;
        Self::from_env()
    }

    /// Runner with an explicit executable and the remote companion's bounds.
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self::with_limits(program, GitLimits::BOUNDED)
    }

    /// Runner with an explicit executable and explicit execution limits.
    pub fn with_limits(program: impl Into<OsString>, limits: GitLimits) -> Self {
        let program = program.into();
        Self {
            program: if program.is_empty() {
                "git".into()
            } else {
                program
            },
            limits,
        }
    }

    /// Configured git executable.
    pub fn program(&self) -> &OsStr {
        &self.program
    }

    /// Configured process execution limits.
    pub const fn limits(&self) -> GitLimits {
        self.limits
    }
    /// Run one fixed command and return stdout.
    ///
    /// The command is bounded by this runner's [`GitLimits`]. The remote
    /// companion uses the bounded defaults; the app compatibility wrapper uses
    /// [`GitLimits::UNBOUNDED`] to preserve local behavior.
    pub fn read_bytes(&self, command: GitCommand, directory: &Path) -> io::Result<Vec<u8>> {
        let status_cwd = matches!(command, GitCommand::Status);
        self.execute(command, directory, status_cwd)
            .map(|output| output.stdout)
            .map_err(io::Error::from)
    }

    fn run(&self, command: GitCommand, directory: &Path) -> Result<String, GitRunError> {
        self.execute(command, directory, false)
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    fn execute(
        &self,
        command: GitCommand,
        directory: &Path,
        use_current_dir: bool,
    ) -> Result<GitOutput, GitRunError> {
        if !is_read_only_git_args(command.args()) {
            return Err(GitRunError::Rejected);
        }
        let deadline = self.limits.deadline.map(|limit| Instant::now() + limit);
        let overrides = if command == GitCommand::Status {
            self.filter_overrides(directory, use_current_dir, deadline)?
        } else {
            Vec::new()
        };
        self.spawn(command, &overrides, directory, use_current_dir, deadline)
    }

    /// `-c` settings that disable every filter driver with a key from a
    /// scope the repository controls, read with [`GitCommand::FilterDrivers`]
    /// under the same deadline. Drivers defined only in the user's global or
    /// the system configuration (such as `git lfs install`) are trusted and
    /// kept; any key from `local`, `worktree`, `command` or another scope,
    /// includes pulled into those scopes too, disables the whole driver.
    ///
    /// A git before 2.26 rejects `--show-scope` as an unknown option; then
    /// [`GitCommand::FilterDriversUnscoped`] is read and every driver is
    /// disabled. Fails closed otherwise: any other failed read, an
    /// unparseable line, or a disabled driver name that `-c` cannot express
    /// exactly is an error and status does not run.
    fn filter_overrides(
        &self,
        directory: &Path,
        use_current_dir: bool,
        deadline: Option<Instant>,
    ) -> Result<Vec<String>, GitRunError> {
        let read = |command| {
            match self.spawn(command, &[], directory, use_current_dir, deadline) {
                Ok(output) => Ok(output.stdout),
                // `git config --get-regexp` exits 1 when nothing matches.
                Err(GitRunError::Failed {
                    code: Some(1),
                    stderr,
                }) if stderr.is_empty() => Ok(Vec::new()),
                Err(error) => Err(error),
            }
        };
        let (scoped, keys) = match read(GitCommand::FilterDrivers) {
            Ok(keys) => (true, keys),
            Err(error) if rejects_show_scope(&error) => {
                (false, read(GitCommand::FilterDriversUnscoped)?)
            }
            Err(error) => return Err(error),
        };
        let keys = String::from_utf8(keys).map_err(|_| GitRunError::UnsafeFilter)?;
        let mut drivers = BTreeSet::new();
        for line in keys.lines() {
            let (scope, key) = if scoped {
                line.split_once('\t').ok_or(GitRunError::UnsafeFilter)?
            } else {
                ("", line)
            };
            let name = key
                .strip_prefix("filter.")
                .and_then(|rest| rest.rsplit_once('.'))
                .map(|(name, _)| name)
                .ok_or(GitRunError::UnsafeFilter)?;
            if scoped {
                if scope.is_empty() || !scope.bytes().all(|byte| byte.is_ascii_lowercase()) {
                    return Err(GitRunError::UnsafeFilter);
                }
                if matches!(scope, "system" | "global") {
                    continue;
                }
            }
            // `-c` splits at the first `=`; git config names hold no
            // newline, and control characters are refused outright.
            if name.is_empty() || name.contains('=') || name.chars().any(char::is_control) {
                return Err(GitRunError::UnsafeFilter);
            }
            drivers.insert(name);
        }
        Ok(drivers
            .into_iter()
            .flat_map(|name| {
                [
                    format!("filter.{name}.clean="),
                    format!("filter.{name}.process="),
                    format!("filter.{name}.required=false"),
                ]
            })
            .collect())
    }

    fn spawn(
        &self,
        command: GitCommand,
        overrides: &[String],
        directory: &Path,
        use_current_dir: bool,
        deadline: Option<Instant>,
    ) -> Result<GitOutput, GitRunError> {
        let mut process = Command::new(&self.program);
        for setting in HARDENED_CONFIG {
            process.arg("-c").arg(setting);
        }
        for setting in overrides {
            process.arg("-c").arg(setting);
        }
        for name in SCRUBBED_ENV {
            process.env_remove(name);
        }
        for (name, _) in std::env::vars_os() {
            if name.to_str().is_some_and(|name| {
                SCRUBBED_ENV_PREFIXES
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
            }) {
                process.env_remove(name);
            }
        }
        process
            .arg("-C")
            .arg(directory)
            .args(command.args())
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if use_current_dir {
            process.current_dir(directory);
        }
        let mut child = process.spawn().map_err(GitRunError::Spawn)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| GitRunError::Io(io::Error::other("git stdout missing")))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| GitRunError::Io(io::Error::other("git stderr missing")))?;
        let (tx, rx) = mpsc::sync_channel(2);
        for (is_stdout, stream) in [
            (true, Box::new(stdout) as Box<dyn Read + Send>),
            (false, Box::new(stderr) as Box<dyn Read + Send>),
        ] {
            let tx = tx.clone();
            let max_output_bytes = self.limits.max_output_bytes;
            thread::spawn(move || {
                let mut stream = stream;
                let mut bytes = Vec::new();
                let result = match max_output_bytes {
                    Some(limit) => {
                        match stream
                            .by_ref()
                            .take(limit.saturating_add(1))
                            .read_to_end(&mut bytes)
                        {
                            Ok(_) if bytes.len() as u64 > limit => Err(GitRunError::OutputLimit),
                            Ok(_) => Ok(bytes),
                            Err(error) => Err(GitRunError::Io(error)),
                        }
                    }
                    None => match stream.read_to_end(&mut bytes) {
                        Ok(_) => Ok(bytes),
                        Err(error) => Err(GitRunError::Io(error)),
                    },
                };
                let _ = tx.send((is_stdout, result));
            });
        }
        drop(tx);
        let mut output = None;
        let mut error_output = None;
        let mut streams = 0;
        let mut status = None;
        let result = loop {
            match rx.try_recv() {
                Ok((is_stdout, Ok(bytes))) => {
                    streams += 1;
                    if is_stdout {
                        output = Some(bytes);
                    } else {
                        error_output = Some(bytes);
                    }
                }
                Ok((_, Err(error))) => break Err(error),
                Err(_) => {}
            }
            match child.try_wait() {
                Ok(Some(value)) => status = Some(value),
                Ok(None) => {}
                Err(error) => break Err(GitRunError::Io(error)),
            }
            if streams == 2 {
                if let Some(status) = status {
                    break if status.success() {
                        Ok(GitOutput {
                            stdout: output.unwrap_or_default(),
                        })
                    } else {
                        Err(GitRunError::Failed {
                            code: status.code(),
                            stderr: String::from_utf8_lossy(&error_output.unwrap_or_default())
                                .trim()
                                .to_owned(),
                        })
                    };
                }
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                break Err(GitRunError::Deadline);
            }
            thread::sleep(Duration::from_millis(5));
        };
        if result.is_err() {
            let _ = child.kill();
        }
        let _ = child.wait();
        result
    }

    /// Read metadata for `directory`.
    ///
    /// Missing directories, non-repositories, and partial read failures return
    /// `Unknown`; a spawn failure returns `Unavailable`. The function never
    /// returns an error and never panics.
    pub fn read_metadata(&self, directory: impl AsRef<Path>) -> WorktreeMetadata {
        let directory = directory.as_ref();
        let toplevel = match self.run(GitCommand::ShowToplevel, directory) {
            Ok(output) if !output.is_empty() => {
                let path = PathBuf::from(output);
                Some(fs::canonicalize(&path).unwrap_or(path))
            }
            Ok(_) => {
                return WorktreeMetadata::unknown(format!(
                    "git did not report a worktree root for {}",
                    directory.display()
                ))
            }
            Err(GitRunError::Spawn(error)) => {
                return WorktreeMetadata::unavailable(format!(
                    "could not run git ({}): {error}",
                    self.program.to_string_lossy()
                ))
            }
            Err(GitRunError::Failed { stderr, .. }) => {
                return WorktreeMetadata::unknown(if stderr.is_empty() {
                    "not a git repository".to_owned()
                } else {
                    format!("not a git repository or git failed: {stderr}")
                })
            }
            Err(error) => {
                return WorktreeMetadata::unknown(format!(
                    "not a git repository or git read failed: {error}"
                ))
            }
        };

        let mut unknown_reason: Option<String> = None;
        let branch_output = self.run(GitCommand::Branch, directory);
        let (branch, detached) = match branch_output {
            Ok(value) if value == "HEAD" => (None, true),
            Ok(value) if !value.is_empty() => (Some(value), false),
            Ok(_) => {
                unknown_reason.get_or_insert_with(|| "git did not report a branch".to_owned());
                (None, false)
            }
            Err(error) => {
                unknown_reason.get_or_insert_with(|| format!("branch read failed: {error}"));
                (None, false)
            }
        };
        let (commit_abbrev, commit_subject) = if detached {
            let abbrev = self
                .run(GitCommand::CommitId, directory)
                .ok()
                .filter(|value| !value.is_empty());
            let subject = self.run(GitCommand::CommitSubject, directory).ok();
            (abbrev, subject)
        } else {
            (None, None)
        };

        let git_dir = self
            .run(GitCommand::GitDir, directory)
            .ok()
            .and_then(|value| normalize_git_path(directory, &value));
        let common_dir = self
            .run(GitCommand::CommonDir, directory)
            .ok()
            .and_then(|value| normalize_git_path(directory, &value));
        if git_dir.is_none() {
            unknown_reason.get_or_insert_with(|| "git directory was not reported".to_owned());
        }
        if common_dir.is_none() {
            unknown_reason
                .get_or_insert_with(|| "git common directory was not reported".to_owned());
        }

        let is_linked_worktree =
            matches!((&git_dir, &common_dir), (Some(git), Some(common)) if git != common);

        let worktrees = self
            .run(GitCommand::WorktreeList, directory)
            .map(|output| parse_worktree_list(&output))
            .unwrap_or_default();
        let worktree_count = worktrees.len();

        let matching = toplevel.as_ref().and_then(|toplevel| {
            worktrees
                .iter()
                .find(|entry| entry.path.as_ref() == Some(toplevel))
        });
        let directory_name = toplevel.as_ref().and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
        });
        let worktree_label = matching
            .and_then(|entry| entry.branch.clone())
            .or_else(|| branch.clone())
            .or_else(|| directory_name.clone());

        let status = if unknown_reason.is_some() {
            MetadataStatus::Unknown
        } else {
            MetadataStatus::Known
        };

        WorktreeMetadata {
            status,
            is_worktree: true,
            toplevel,
            branch,
            detached,
            commit_abbrev,
            commit_subject,
            is_linked_worktree,
            directory_name,
            worktree_label,
            git_dir,
            common_dir,
            worktree_count,
            reason: unknown_reason,
        }
    }

    /// Read porcelain status for `directory`.
    ///
    /// `directory` is both the process cwd and git's `-C` path. The fixed
    /// pathspec is `.`, so a subdirectory explorer root does not list the rest
    /// of the repository. `toplevel`, when known, interprets porcelain paths,
    /// which are relative to the repository root. A spawn failure is
    /// [`GitStatusRead::Unavailable`]; a failed read is [`GitStatusRead::Failed`].
    /// Neither panics.
    pub fn read_status(&self, directory: &Path, toplevel: Option<&Path>) -> GitStatusRead {
        // Porcelain paths are relative to the repository root. Canonicalize
        // both sides so a symlink prefix (`/var` vs `/private/var`) cannot
        // drop every entry.
        let directory = fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf());
        let toplevel =
            toplevel.map(|path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()));
        match self.execute(GitCommand::Status, &directory, true) {
            Ok(output) => GitStatusRead::Ready(group_status(
                &parse_porcelain_v1_z(&String::from_utf8_lossy(&output.stdout)),
                &directory,
                toplevel.as_deref(),
            )),
            Err(GitRunError::Spawn(error)) => GitStatusRead::Unavailable {
                reason: format!(
                    "could not run git ({}): {error}",
                    self.program.to_string_lossy()
                ),
            },
            Err(error) => GitStatusRead::Failed {
                reason: error.to_string(),
            },
        }
    }
}

/// Whether `git config` refused `--show-scope` as an unknown option, as
/// git before 2.26 does: a usage exit (129) naming that option. Every other
/// failure keeps status failed closed.
fn rejects_show_scope(error: &GitRunError) -> bool {
    matches!(
        error,
        GitRunError::Failed { code: Some(129), stderr }
            if stderr.contains("unknown option") && stderr.contains("show-scope")
    )
}

struct GitOutput {
    stdout: Vec<u8>,
}

/// Read metadata with the environment-configured runner.
pub fn read_metadata(directory: impl AsRef<Path>) -> WorktreeMetadata {
    GitRunner::from_env().read_metadata(directory)
}
fn normalize_git_path(directory: &Path, raw: &str) -> Option<PathBuf> {
    if raw.is_empty() {
        return None;
    }
    let p = PathBuf::from(raw);
    let p = if p.is_absolute() {
        p
    } else {
        directory.join(p)
    };
    Some(fs::canonicalize(&p).unwrap_or(p))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PorcelainWorktree {
    path: Option<PathBuf>,
    branch: Option<String>,
}

/// Every record of the local metadata read, bare and empty paths included,
/// with each path canonicalized for comparison with the top level.
fn parse_worktree_list(text: &str) -> Vec<PorcelainWorktree> {
    parse_worktree_records(text)
        .into_iter()
        .map(|(entry, _)| PorcelainWorktree {
            path: Some(fs::canonicalize(&entry.path).unwrap_or(entry.path)),
            branch: entry.branch,
        })
        .collect()
}

/// One working tree from `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    /// Working-tree path exactly as git reported it.
    pub path: PathBuf,
    /// Branch name without `refs/heads/`, when one is checked out.
    pub branch: Option<String>,
    /// Full `HEAD` commit id, when reported.
    pub head: Option<String>,
    /// Whether `HEAD` is detached.
    pub detached: bool,
    /// Whether git reports the directory as missing (`prunable`).
    pub missing: bool,
}

impl WorktreeEntry {
    /// Branch name, else the first 8 characters of a detached commit id.
    pub fn head_label(&self) -> Option<String> {
        self.branch.clone().or_else(|| {
            self.head
                .as_deref()
                .filter(|head| !head.is_empty())
                .map(|head| head.chars().take(8).collect())
        })
    }
}

/// Parse `git worktree list --porcelain` into working trees in repository
/// order (git lists the main worktree first). Pure: no path is resolved,
/// canonicalized or opened, so it is safe for remote output. Bare entries
/// have no working tree and are skipped.
pub fn parse_worktree_entries(text: &str) -> Vec<WorktreeEntry> {
    parse_worktree_records(text)
        .into_iter()
        .filter(|(entry, bare)| !bare && !entry.path.as_os_str().is_empty())
        .map(|(entry, _)| entry)
        .collect()
}

/// Every `worktree` record in order, with whether it is bare. Pure.
fn parse_worktree_records(text: &str) -> Vec<(WorktreeEntry, bool)> {
    let mut entries = Vec::new();
    let mut current: Option<(WorktreeEntry, bool)> = None;
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            entries.extend(current.take());
            current = Some((
                WorktreeEntry {
                    path: PathBuf::from(path),
                    branch: None,
                    head: None,
                    detached: false,
                    missing: false,
                },
                false,
            ));
            continue;
        }
        let Some((entry, bare)) = current.as_mut() else {
            continue;
        };
        if let Some(branch) = line.strip_prefix("branch ") {
            entry.branch = Some(
                branch
                    .strip_prefix("refs/heads/")
                    .unwrap_or(branch)
                    .to_owned(),
            );
        } else if let Some(head) = line.strip_prefix("HEAD ") {
            entry.head = Some(head.to_owned());
        } else if line == "detached" {
            entry.detached = true;
        } else if line == "bare" {
            *bare = true;
        } else if line == "prunable" || line.starts_with("prunable ") {
            entry.missing = true;
        }
    }
    entries.extend(current);
    entries
}

/// One change pill kind, in pill order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PillKind {
    /// Unmerged paths (`Merge Changes`).
    Conflicted,
    /// Index changes (`Staged Changes`).
    Staged,
    /// Unstaged changes to tracked paths (`Changes`).
    Modified,
    /// Untracked paths (`Untracked`).
    Untracked,
}

impl PillKind {
    /// Pill text after the count.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Conflicted => "conflicted",
            Self::Staged => "staged",
            Self::Modified => "modified",
            Self::Untracked => "untracked",
        }
    }
}

/// Whole-working-tree change counts with the `Grouped changes` classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChangeCounts {
    /// Rows of `Merge Changes`.
    pub conflicted: usize,
    /// Rows of `Staged Changes`.
    pub staged: usize,
    /// Rows of `Changes`.
    pub modified: usize,
    /// Rows of `Untracked`.
    pub untracked: usize,
}

impl ChangeCounts {
    /// Count the rows of each group of `snapshot`. Group it at the working
    /// tree's root to cover the whole working tree.
    pub fn from_snapshot(snapshot: &GitStatusSnapshot) -> Self {
        let mut counts = Self::default();
        for group in &snapshot.groups {
            let rows = group.rows.len();
            match group.title {
                GROUP_MERGE => counts.conflicted += rows,
                GROUP_STAGED => counts.staged += rows,
                GROUP_CHANGES => counts.modified += rows,
                GROUP_UNTRACKED => counts.untracked += rows,
                _ => {}
            }
        }
        counts
    }

    /// Count porcelain records over the whole working tree (no root filter).
    pub fn from_records(records: &[PorcelainRecord]) -> Self {
        let root = Path::new("/");
        Self::from_snapshot(&group_status(records, root, Some(root)))
    }

    /// Whether every count is zero.
    pub const fn is_clean(&self) -> bool {
        self.conflicted == 0 && self.staged == 0 && self.modified == 0 && self.untracked == 0
    }

    /// Nonzero counts in pill order.
    pub fn nonzero(&self) -> Vec<(PillKind, usize)> {
        [
            (PillKind::Conflicted, self.conflicted),
            (PillKind::Staged, self.staged),
            (PillKind::Modified, self.modified),
            (PillKind::Untracked, self.untracked),
        ]
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .collect()
    }

    /// Pill texts (`N staged`) of the nonzero counts, in pill order.
    pub fn pill_labels(&self) -> Vec<String> {
        self.nonzero()
            .into_iter()
            .map(|(kind, count)| format!("{count} {}", kind.label()))
            .collect()
    }

    /// One tooltip and accessible name for the pills, `None` when clean.
    pub fn summary(&self) -> Option<String> {
        let labels = self.pill_labels();
        (!labels.is_empty()).then(|| format!("Changes: {}", labels.join(" · ")))
    }
}

/// Kind of change used by folder dots and row color.
///
/// Declaration order is the tooltip order for a mixed folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangeKind {
    /// Untracked path or staged addition.
    New,
    /// Content or type change.
    Modified,
    /// Deletion.
    Deleted,
    /// Rename, or a git copy (no separate letter).
    Renamed,
    /// Unmerged path.
    Conflict,
}

impl ChangeKind {
    /// Tooltip name. Never relies on color alone.
    pub const fn label(self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Modified => "Modified",
            Self::Deleted => "Deleted",
            Self::Renamed => "Renamed",
            Self::Conflict => "Conflict",
        }
    }
}
/// One porcelain `-z` record, still relative to the repository root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorcelainRecord {
    /// Index status (`X`).
    pub index: char,
    /// Worktree status (`Y`).
    pub worktree: char,
    /// Path without a trailing slash.
    pub path: String,
    /// Original path for a rename or copy.
    pub original: Option<String>,
    /// `true` when porcelain reported a directory (`path/`).
    pub is_directory: bool,
}

/// One row in a source-control group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusRow {
    /// `U`, `A`, `M`, `D`, `R`, or `C`.
    pub letter: char,
    /// Semantic kind for color. The letter is the non-color cue.
    pub kind: ChangeKind,
    /// Absolute path under the explorer root.
    pub path: PathBuf,
    /// Path relative to the explorer root, for display.
    pub relative: PathBuf,
    /// Directory entries select only; they do not open a preview.
    pub is_directory: bool,
    /// Original path relative to the explorer root, when it is inside the root.
    pub original_relative: Option<PathBuf>,
}

impl GitStatusRow {
    /// Whether activating the row should open the read-only preview.
    pub const fn opens_preview(&self) -> bool {
        !self.is_directory
    }
}

/// One non-empty source-control section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusGroup {
    /// Exact section title.
    pub title: &'static str,
    /// Rows in path order. A partially staged file may appear in two groups.
    pub rows: Vec<GitStatusRow>,
}

/// Kinds present in a folder, from the full status list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderMark {
    /// Unique kinds in tooltip order.
    pub kinds: Vec<ChangeKind>,
}

impl FolderMark {
    /// Two or more kinds share the neutral accent instead of one semantic color.
    pub fn is_mixed(&self) -> bool {
        self.kinds.len() > 1
    }

    /// The only kind, when the folder is not mixed.
    pub fn single(&self) -> Option<ChangeKind> {
        if self.kinds.len() == 1 {
            self.kinds.first().copied()
        } else {
            None
        }
    }

    /// Dot tooltip: one kind's name, or the kinds listed in order.
    pub fn tooltip(&self) -> String {
        self.kinds
            .iter()
            .map(|k| k.label())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Grouped status plus folder-dot marks keyed by absolute directory path.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitStatusSnapshot {
    /// Non-empty groups, in Merge / Staged / Changes / Untracked order.
    pub groups: Vec<GitStatusGroup>,
    /// Ancestor folders that contain a change, including collapsed ones.
    pub folders: BTreeMap<PathBuf, FolderMark>,
}

/// Outcome of one read-only status command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitStatusRead {
    /// Git answered. An empty snapshot is a clean tree, not a failure.
    Ready(GitStatusSnapshot),
    /// Git ran and failed, or the path could not be read.
    Failed {
        /// Failure detail, including stderr and exit code when available.
        reason: String,
    },
    /// The git executable is missing or could not be started.
    Unavailable {
        /// Spawn failure detail.
        reason: String,
    },
}
/// Parse `git status --porcelain=v1 -z` output.
///
/// Rename and copy records are `XY new\0old\0`. Ignored (`!!`) records are
/// dropped. A trailing slash marks a directory entry.
pub fn parse_porcelain_v1_z(text: &str) -> Vec<PorcelainRecord> {
    let mut records = Vec::new();
    let mut parts = text.split('\0');
    while let Some(record) = parts.next() {
        let b = record.as_bytes();
        if b.len() < 4 || b[2] != b' ' || !b[..2].is_ascii() {
            continue;
        }
        let index = char::from(b[0]);
        let worktree = char::from(b[1]);
        if index == '!' || worktree == '!' {
            continue;
        }
        let mut path = record[3..].to_owned();
        let original = if matches!(index, 'R' | 'C') || matches!(worktree, 'R' | 'C') {
            parts.next().filter(|s| !s.is_empty()).map(str::to_owned)
        } else {
            None
        };
        let is_directory = path.ends_with('/');
        if is_directory {
            path.pop();
        }
        if path.is_empty() || path_escapes(&path) {
            continue;
        }
        records.push(PorcelainRecord {
            index,
            worktree,
            path,
            original,
            is_directory,
        });
    }
    records
}
fn path_escapes(path: &str) -> bool {
    Path::new(path).components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}
fn resolve(root: &Path, top: Option<&Path>, raw: &str) -> Option<(PathBuf, PathBuf)> {
    if raw.is_empty() || path_escapes(raw) {
        return None;
    }
    let absolute = top.unwrap_or(root).join(raw);
    let relative = absolute.strip_prefix(root).ok()?.to_path_buf();
    if relative.as_os_str().is_empty() {
        None
    } else {
        Some((absolute, relative))
    }
}
fn side(code: char) -> Option<(char, ChangeKind)> {
    match code {
        'A' => Some(('A', ChangeKind::New)),
        'M' | 'T' => Some(('M', ChangeKind::Modified)),
        'D' => Some(('D', ChangeKind::Deleted)),
        'R' | 'C' => Some(('R', ChangeKind::Renamed)),
        _ => None,
    }
}
/// Group porcelain records into source-control sections and folder dots.
///
/// Paths outside `root` are dropped. Porcelain paths are relative to `top`
/// when it is known; otherwise they are treated as relative to `root`.
pub fn group_status(
    records: &[PorcelainRecord],
    root: &Path,
    top: Option<&Path>,
) -> GitStatusSnapshot {
    let mut groups: [Vec<GitStatusRow>; 4] = std::array::from_fn(|_| Vec::new());
    let mut marks: BTreeMap<PathBuf, BTreeSet<ChangeKind>> = BTreeMap::new();
    for record in records {
        let Some((absolute, relative)) = resolve(root, top, &record.path) else {
            continue;
        };
        let original_relative = record
            .original
            .as_deref()
            .and_then(|s| resolve(root, top, s))
            .map(|(_, r)| r);
        let conflict = record.index == 'U'
            || record.worktree == 'U'
            || (record.index == 'A' && record.worktree == 'A')
            || (record.index == 'D' && record.worktree == 'D');
        let mut placements = Vec::with_capacity(2);
        if conflict {
            placements.push((0, 'C', ChangeKind::Conflict));
        } else if record.index == '?' && record.worktree == '?' {
            placements.push((3, 'U', ChangeKind::New));
        } else {
            if let Some((l, k)) = side(record.index) {
                placements.push((1, l, k));
            }
            if let Some((l, k)) = side(record.worktree) {
                placements.push((2, l, k));
            }
        }
        for (group, letter, kind) in placements {
            groups[group].push(GitStatusRow {
                letter,
                kind,
                path: absolute.clone(),
                relative: relative.clone(),
                is_directory: record.is_directory,
                original_relative: original_relative.clone(),
            });
            mark(&mut marks, root, &absolute, record.is_directory, kind);
            if kind == ChangeKind::Renamed {
                if let Some(original) = record
                    .original
                    .as_deref()
                    .and_then(|s| resolve(root, top, s))
                {
                    mark(&mut marks, root, &original.0, false, kind);
                }
            }
        }
    }
    let titles = [GROUP_MERGE, GROUP_STAGED, GROUP_CHANGES, GROUP_UNTRACKED];
    GitStatusSnapshot {
        groups: groups
            .into_iter()
            .enumerate()
            .filter_map(|(i, mut rows)| {
                rows.sort_by(|a, b| a.relative.cmp(&b.relative));
                (!rows.is_empty()).then_some(GitStatusGroup {
                    title: titles[i],
                    rows,
                })
            })
            .collect(),
        folders: marks
            .into_iter()
            .map(|(p, k)| {
                (
                    p,
                    FolderMark {
                        kinds: k.into_iter().collect(),
                    },
                )
            })
            .collect(),
    }
}
fn mark(
    marks: &mut BTreeMap<PathBuf, BTreeSet<ChangeKind>>,
    root: &Path,
    path: &Path,
    is_directory: bool,
    kind: ChangeKind,
) {
    let mut current = if is_directory {
        Some(path)
    } else {
        path.parent()
    };
    while let Some(dir) = current {
        if dir == root || !dir.starts_with(root) {
            break;
        }
        marks.entry(dir.into()).or_default().insert(kind);
        current = dir.parent();
    }
}
