//! Shared read-only Git observer. No mutation or arbitrary argv entry point.
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
pub const GIT_PROGRAM_ENV: &str = "MADE_GIT_BIN";
pub const WORKTREE_MANAGEMENT_AVAILABLE: bool = false;
pub const GROUP_MERGE: &str = "Merge Changes";
pub const GROUP_STAGED: &str = "Staged Changes";
pub const GROUP_CHANGES: &str = "Changes";
pub const GROUP_UNTRACKED: &str = "Untracked";
pub const READ_ONLY_GIT_ARGUMENTS: &[&[&str]] = &[
    &["rev-parse", "--show-toplevel"],
    &["rev-parse", "--abbrev-ref", "HEAD"],
    &["rev-parse", "--git-dir"],
    &["rev-parse", "--git-common-dir"],
    &["worktree", "list", "--porcelain"],
    &["rev-parse", "--short=8", "HEAD"],
    &["log", "-1", "--format=%s"],
    &[
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--",
        ".",
    ],
];
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GitCommand {
    ShowToplevel,
    Branch,
    GitDir,
    CommonDir,
    WorktreeList,
    CommitId,
    CommitSubject,
    Status,
}
impl GitCommand {
    pub const ALL: [Self; 8] = [
        Self::ShowToplevel,
        Self::Branch,
        Self::GitDir,
        Self::CommonDir,
        Self::WorktreeList,
        Self::CommitId,
        Self::CommitSubject,
        Self::Status,
    ];
    pub const fn args(self) -> &'static [&'static str] {
        READ_ONLY_GIT_ARGUMENTS[self as usize]
    }
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
pub fn is_read_only_git_args(args: &[&str]) -> bool {
    READ_ONLY_GIT_ARGUMENTS.contains(&args)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataStatus {
    Known,
    Unknown,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeMetadata {
    pub status: MetadataStatus,
    pub is_worktree: bool,
    pub toplevel: Option<PathBuf>,
    pub branch: Option<String>,
    pub detached: bool,
    pub commit_abbrev: Option<String>,
    pub commit_subject: Option<String>,
    pub is_linked_worktree: bool,
    pub directory_name: Option<String>,
    pub worktree_label: Option<String>,
    pub git_dir: Option<PathBuf>,
    pub common_dir: Option<PathBuf>,
    pub worktree_count: usize,
    pub reason: Option<String>,
}
impl WorktreeMetadata {
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
    pub fn unavailable(reason: impl Into<String>) -> Self {
        let mut value = Self::unknown(reason);
        value.status = MetadataStatus::Unavailable;
        value
    }
    pub const fn is_known(&self) -> bool {
        matches!(self.status, MetadataStatus::Known)
    }
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
#[derive(Debug, Clone)]
pub struct GitRunner {
    program: OsString,
}
impl Default for GitRunner {
    fn default() -> Self {
        Self::from_env()
    }
}
impl GitRunner {
    pub fn new() -> Self {
        Self::from_env()
    }
    /// Resolve `git` on `PATH` only. `MADE_GIT_BIN` is not a program selector:
    /// an arbitrary path would bypass the closed allowlist by replacing the
    /// executable. `with_program` remains available for an explicit test binary.
    pub fn from_env() -> Self {
        let _ = env::var_os(GIT_PROGRAM_ENV);
        Self::from_env_value(None)
    }
    pub fn from_env_value(program: Option<OsString>) -> Self {
        let _ = program;
        let resolved = made_platform::Environment::current()
            .get("PATH")
            .and_then(|path| {
                made_platform::find_executable_on_path(std::ffi::OsStr::new("git"), path)
            });
        Self::with_program(resolved.unwrap_or_else(|| PathBuf::from("git")))
    }
    pub fn with_program(program: impl Into<OsString>) -> Self {
        let program = program.into();
        Self {
            program: if program.is_empty() {
                "git".into()
            } else {
                program
            },
        }
    }
    pub fn program(&self) -> &OsStr {
        &self.program
    }
    /// Fixed command, bounded stdout/stderr and total deadline, no Git writes.
    pub fn read_bytes(&self, command: GitCommand, directory: &Path) -> io::Result<Vec<u8>> {
        let mut child = Command::new(&self.program)
            .arg("-C")
            .arg(directory)
            .args(command.args())
            .current_dir(directory)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("git stdout missing"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("git stderr missing"))?;
        let (tx, rx) = mpsc::sync_channel(2);
        for (is_stdout, stream) in [
            (true, Box::new(stdout) as Box<dyn Read + Send>),
            (false, Box::new(stderr) as Box<dyn Read + Send>),
        ] {
            let tx = tx.clone();
            thread::spawn(move || {
                let mut bytes = Vec::new();
                let result = stream
                    .take(crate::protocol::MAX_GIT_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .and_then(|_| {
                        if bytes.len() as u64 > crate::protocol::MAX_GIT_BYTES {
                            Err(io::Error::other("git output limit"))
                        } else {
                            Ok(bytes)
                        }
                    });
                let _ = tx.send((is_stdout, result));
            });
        }
        drop(tx);
        let deadline = Instant::now() + Duration::from_secs(9);
        let mut output = None;
        let mut streams = 0;
        let mut status = None;
        let result = loop {
            match rx.try_recv() {
                Ok((is_stdout, Ok(bytes))) => {
                    streams += 1;
                    if is_stdout {
                        output = Some(bytes);
                    }
                }
                Ok((_, Err(e))) => break Err(e),
                Err(_) => {}
            }
            match child.try_wait() {
                Ok(Some(value)) => status = Some(value),
                Ok(None) => {}
                Err(e) => break Err(e),
            }
            if streams == 2 && status.is_some() {
                break if status.is_some_and(|s| s.success()) {
                    Ok(output.unwrap_or_default())
                } else {
                    Err(io::Error::other("git read failed"))
                };
            }
            if Instant::now() >= deadline {
                break Err(io::Error::new(io::ErrorKind::TimedOut, "git read deadline"));
            }
            thread::sleep(Duration::from_millis(5));
        };
        if result.is_err() {
            let _ = child.kill();
        }
        let _ = child.wait();
        result
    }
    fn run(&self, command: GitCommand, directory: &Path) -> io::Result<String> {
        self.read_bytes(command, directory)
            .map(|b| String::from_utf8_lossy(&b).trim().to_owned())
    }
    pub fn read_metadata(&self, directory: impl AsRef<Path>) -> WorktreeMetadata {
        let directory = directory.as_ref();
        let top = match self.run(GitCommand::ShowToplevel, directory) {
            Ok(top) if !top.is_empty() => PathBuf::from(top),
            Ok(_) => return WorktreeMetadata::unknown("git did not report a worktree root"),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return WorktreeMetadata::unavailable("git executable or root unavailable")
            }
            Err(_) => return WorktreeMetadata::unknown("not a git repository or git read failed"),
        };
        let top = fs::canonicalize(&top).unwrap_or(top);
        let mut result = WorktreeMetadata::unknown("incomplete git metadata");
        result.is_worktree = true;
        result.directory_name = top.file_name().map(|n| n.to_string_lossy().into_owned());
        result.toplevel = Some(top);
        if let Ok(branch) = self.run(GitCommand::Branch, directory) {
            result.detached = branch == "HEAD";
            if !result.detached && !branch.is_empty() {
                result.branch = Some(branch);
            }
        }
        if result.detached {
            result.commit_abbrev = self
                .run(GitCommand::CommitId, directory)
                .ok()
                .filter(|s| !s.is_empty());
            result.commit_subject = self.run(GitCommand::CommitSubject, directory).ok();
        }
        result.git_dir = self
            .run(GitCommand::GitDir, directory)
            .ok()
            .and_then(|s| normalize_git_path(directory, &s));
        result.common_dir = self
            .run(GitCommand::CommonDir, directory)
            .ok()
            .and_then(|s| normalize_git_path(directory, &s));
        result.is_linked_worktree =
            matches!((&result.git_dir, &result.common_dir), (Some(a), Some(b)) if a != b);
        if let Ok(worktrees) = self.run(GitCommand::WorktreeList, directory) {
            result.worktree_count = worktrees
                .lines()
                .filter(|l| l.starts_with("worktree "))
                .count();
        }
        result.worktree_label = result
            .branch
            .clone()
            .or_else(|| result.directory_name.clone());
        if (result.branch.is_some() || result.detached)
            && result.git_dir.is_some()
            && result.common_dir.is_some()
        {
            result.status = MetadataStatus::Known;
            result.reason = None;
        }
        result
    }
    pub fn read_status(&self, directory: &Path, toplevel: Option<&Path>) -> GitStatusRead {
        let directory = fs::canonicalize(directory).unwrap_or_else(|_| directory.into());
        let toplevel = toplevel.map(|p| fs::canonicalize(p).unwrap_or_else(|_| p.into()));
        match self.read_bytes(GitCommand::Status, &directory) {
            Ok(bytes) => GitStatusRead::Ready(group_status(
                &parse_porcelain_v1_z(&String::from_utf8_lossy(&bytes)),
                &directory,
                toplevel.as_deref(),
            )),
            Err(e) if e.kind() == io::ErrorKind::NotFound => GitStatusRead::Unavailable {
                reason: "git unavailable".into(),
            },
            Err(_) => GitStatusRead::Failed {
                reason: "git status failed or exceeded its budget".into(),
            },
        }
    }
}
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangeKind {
    New,
    Modified,
    Deleted,
    Renamed,
    Conflict,
}
impl ChangeKind {
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorcelainRecord {
    pub index: char,
    pub worktree: char,
    pub path: String,
    pub original: Option<String>,
    pub is_directory: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusRow {
    pub letter: char,
    pub kind: ChangeKind,
    pub path: PathBuf,
    pub relative: PathBuf,
    pub is_directory: bool,
    pub original_relative: Option<PathBuf>,
}
impl GitStatusRow {
    pub const fn opens_preview(&self) -> bool {
        !self.is_directory
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusGroup {
    pub title: &'static str,
    pub rows: Vec<GitStatusRow>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderMark {
    pub kinds: Vec<ChangeKind>,
}
impl FolderMark {
    pub fn is_mixed(&self) -> bool {
        self.kinds.len() > 1
    }
    pub fn single(&self) -> Option<ChangeKind> {
        if self.kinds.len() == 1 {
            self.kinds.first().copied()
        } else {
            None
        }
    }
    pub fn tooltip(&self) -> String {
        self.kinds
            .iter()
            .map(|k| k.label())
            .collect::<Vec<_>>()
            .join(", ")
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitStatusSnapshot {
    pub groups: Vec<GitStatusGroup>,
    pub folders: BTreeMap<PathBuf, FolderMark>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitStatusRead {
    Ready(GitStatusSnapshot),
    Failed { reason: String },
    Unavailable { reason: String },
}
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
