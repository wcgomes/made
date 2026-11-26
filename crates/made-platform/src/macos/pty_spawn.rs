//! PTY child spawn through `posix_spawn(2)` instead of `fork(2)`.
//!
//! `portable-pty` starts the child with std's `Command` and a `pre_exec`
//! closure, which forks; see [`super::spawn`] for why that can kill the child
//! on macOS. The `pre_exec` closure also allocated (`read_dir("/dev/fd")`)
//! between fork and exec. The spawn attributes keep the `portable-pty`
//! semantics: a new session with the PTY as controlling terminal and stdio,
//! only stdio inherited, the same signals reset to their defaults, and an
//! empty mask.

use std::ffi::{CStr, CString, OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use portable_pty::{Child, ChildKiller, ExitStatus};

use super::spawn::{env_refs, prepare, wait_pid, FileActions, SpawnRequest};

/// Signals `portable-pty` resets to `SIG_DFL` in the child, plus `SIGPIPE`,
/// which std's spawn also resets. `SIGHUP` must stay here: a detached spawn
/// can briefly ignore it process-wide (see `spawn_detached_child`), and a PTY
/// child started meanwhile must not inherit that.
const DEFAULT_SIGNALS: [libc::c_int; 7] = [
    libc::SIGCHLD,
    libc::SIGHUP,
    libc::SIGINT,
    libc::SIGQUIT,
    libc::SIGTERM,
    libc::SIGALRM,
    libc::SIGPIPE,
];

/// Spawn `program` on the PTY whose slave device is `tty`.
///
/// Resolution matches `portable-pty`'s `CommandBuilder` with `env_clear`:
/// `cwd` falls back to `HOME` (then the password database, then `/`) when it
/// is not a directory, a relative `program` is searched in the child's `PATH`,
/// `argv[0]` is `program` as given, and `SHELL` defaults to the user's shell.
/// Like `execvp`, a program the kernel rejects with `ENOEXEC` is run by
/// `/bin/sh`.
pub(crate) fn spawn_pty_child(
    program: &OsStr,
    args: &[OsString],
    cwd: &Path,
    env: &[(OsString, OsString)],
    env_remove: &[OsString],
    tty: &Path,
) -> io::Result<Box<dyn Child + Send + Sync>> {
    let env = child_environment(env, env_remove);
    let cwd = child_cwd(cwd, &env);
    let resolved = resolve_program(program, &cwd, env_value(&env, "PATH"))?;

    let argv = std::iter::once(program)
        .chain(args.iter().map(OsString::as_os_str))
        .collect();
    let request = SpawnRequest {
        what: "PTY child",
        path: &resolved,
        argv,
        env: env_refs(&env),
        default_signals: &DEFAULT_SIGNALS,
    };
    let mut actions = FileActions::new("PTY child")?;
    actions.chdir(&cwd)?;
    // A session leader that opens a terminal without O_NOCTTY acquires it as
    // its controlling terminal, which replaces portable-pty's TIOCSCTTY.
    // POSIX_SPAWN_SETSID runs before the file actions.
    actions.open(0, tty, libc::O_RDWR)?;
    actions.dup2(0, 1)?;
    actions.dup2(0, 2)?;
    let pid = prepare(&request, actions)?.spawn()?;
    Ok(Box::new(SpawnedChild { pid, status: None }))
}

/// `env_clear()` then the request's variables, minus `env_remove`, with
/// `SHELL` defaulted the way `CommandBuilder::as_command` does.
fn child_environment(
    env: &[(OsString, OsString)],
    env_remove: &[OsString],
) -> Vec<(OsString, OsString)> {
    let mut result: Vec<(OsString, OsString)> = Vec::with_capacity(env.len() + 1);
    for (key, value) in env {
        match result.iter_mut().find(|(existing, _)| existing == key) {
            Some(entry) => entry.1 = value.clone(),
            None => result.push((key.clone(), value.clone())),
        }
    }
    result.retain(|(key, _)| !env_remove.contains(key));
    if env_value(&result, "SHELL").is_none() {
        result.push(("SHELL".into(), user_shell()));
    }
    result
}

fn env_value<'a>(env: &'a [(OsString, OsString)], key: &str) -> Option<&'a OsStr> {
    env.iter()
        .find(|(existing, _)| existing == key)
        .map(|(_, value)| value.as_os_str())
}

/// The password database shell when it is executable, else `/bin/sh`.
fn user_shell() -> OsString {
    passwd_field(|entry| entry.pw_shell)
        .filter(|shell| is_executable(Path::new(shell)))
        .unwrap_or_else(|| "/bin/sh".into())
}

fn child_cwd(cwd: &Path, env: &[(OsString, OsString)]) -> PathBuf {
    if cwd.is_dir() {
        return cwd.to_path_buf();
    }
    env_value(env, "HOME")
        .map(OsString::from)
        .or_else(|| passwd_field(|entry| entry.pw_dir))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn passwd_field(field: impl Fn(&libc::passwd) -> *mut libc::c_char) -> Option<OsString> {
    let mut buffer = vec![0 as libc::c_char; 4096];
    loop {
        // SAFETY: `entry` and `buffer` outlive the call, and the string fields
        // getpwuid_r stores point into `buffer`; the field is copied out before
        // `buffer` is dropped or resized.
        unsafe {
            let mut entry: libc::passwd = std::mem::zeroed();
            let mut found: *mut libc::passwd = std::ptr::null_mut();
            let code = libc::getpwuid_r(
                libc::getuid(),
                &mut entry,
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut found,
            );
            if code == libc::ERANGE && buffer.len() < 1 << 20 {
                buffer.resize(buffer.len() * 2, 0);
                continue;
            }
            if code != 0 || found.is_null() {
                return None;
            }
            let value = field(&entry);
            return (!value.is_null())
                .then(|| OsStr::from_bytes(CStr::from_ptr(value).to_bytes()).into());
        }
    }
}

/// `CommandBuilder::search_path`: cwd-relative names are checked only in
/// `cwd`, other relative names in each `PATH` entry, absolute ones as is.
fn resolve_program(program: &OsStr, cwd: &Path, path: Option<&OsStr>) -> io::Result<PathBuf> {
    let program_path = Path::new(program);
    if program_path.is_absolute() {
        return check_candidate(program_path.to_path_buf());
    }
    if matches!(
        program_path.components().next(),
        Some(std::path::Component::CurDir | std::path::Component::ParentDir)
    ) {
        return check_candidate(cwd.join(program_path));
    }
    let path = path.ok_or_else(|| spawn_error(program_path, "unable to resolve the PATH"))?;
    std::env::split_paths(path)
        .map(|dir| cwd.join(dir).join(program_path))
        .find(|candidate| !candidate.is_dir() && is_executable(candidate))
        .ok_or_else(|| spawn_error(program_path, "no viable candidates found in PATH"))
}

fn check_candidate(candidate: PathBuf) -> io::Result<PathBuf> {
    if candidate.is_dir() {
        Err(spawn_error(&candidate, "it is a directory"))
    } else if is_executable(&candidate) {
        Ok(candidate)
    } else if candidate.exists() {
        Err(spawn_error(&candidate, "it is not executable"))
    } else {
        Err(spawn_error(&candidate, "it does not exist"))
    }
}

fn spawn_error(program: &Path, reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("unable to spawn {} because {reason}", program.display()),
    )
}

fn is_executable(path: &Path) -> bool {
    CString::new(path.as_os_str().as_bytes())
        // SAFETY: access only reads the NUL-terminated path.
        .is_ok_and(|path| unsafe { libc::access(path.as_ptr(), libc::X_OK) } == 0)
}

/// The spawned PTY child, with the same wait and kill behavior as
/// `portable-pty`'s `std::process::Child` implementation.
#[derive(Debug)]
struct SpawnedChild {
    pid: libc::pid_t,
    status: Option<ExitStatus>,
}

impl SpawnedChild {
    fn reap(&mut self, flags: libc::c_int) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = &self.status {
            return Ok(Some(status.clone()));
        }
        let Some(raw) = wait_pid(self.pid, flags)? else {
            return Ok(None);
        };
        let status = exit_status(raw);
        self.status = Some(status.clone());
        Ok(Some(status))
    }
}

fn exit_status(raw: libc::c_int) -> ExitStatus {
    if libc::WIFSIGNALED(raw) {
        let signal = libc::WTERMSIG(raw);
        // SAFETY: strsignal returns null or a NUL-terminated string that is
        // copied before the next call.
        let name = unsafe { libc::strsignal(signal) };
        let name = if name.is_null() {
            format!("Signal {signal}")
        } else {
            unsafe { CStr::from_ptr(name) }
                .to_string_lossy()
                .into_owned()
        };
        ExitStatus::with_signal(&name)
    } else {
        ExitStatus::with_exit_code(libc::WEXITSTATUS(raw) as u32)
    }
}

impl Child for SpawnedChild {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.reap(libc::WNOHANG)
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        self.reap(0)
            .map(|status| status.expect("blocking waitpid reaps"))
    }

    fn process_id(&self) -> Option<u32> {
        Some(self.pid as u32)
    }
}

impl ChildKiller for SpawnedChild {
    /// SIGHUP, up to about 200ms for the child to exit, then SIGKILL.
    fn kill(&mut self) -> io::Result<()> {
        if self.status.is_some() {
            return Ok(());
        }
        signal(self.pid, libc::SIGHUP)?;
        for attempt in 0..5 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(50));
            }
            if let Ok(Some(_)) = self.try_wait() {
                return Ok(());
            }
        }
        signal(self.pid, libc::SIGKILL)
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(Signaller { pid: self.pid })
    }
}

/// A killer split from the child: sends SIGHUP, like portable-pty's.
#[derive(Debug)]
struct Signaller {
    pid: libc::pid_t,
}

impl ChildKiller for Signaller {
    fn kill(&mut self) -> io::Result<()> {
        signal(self.pid, libc::SIGHUP)
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(Signaller { pid: self.pid })
    }
}

fn signal(pid: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
    // SAFETY: kill has no pointer arguments.
    if unsafe { libc::kill(pid, signal) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
