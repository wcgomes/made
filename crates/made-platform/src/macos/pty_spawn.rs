//! PTY child spawn through `posix_spawn(2)` instead of `fork(2)`.
//!
//! `portable-pty` starts the child with `std::process::Command` and a
//! `pre_exec` closure, which makes std `fork()`. In a multithreaded process,
//! macOS's `fork()` runs libSystem's atfork child handlers in the child; if
//! another thread was inside a one-time libSystem initialization at that
//! moment (for example the first libnotify use), the child's handler waits on
//! a once-gate whose owner thread does not exist in the child and traps:
//! `BUG IN CLIENT OF LIBPLATFORM: os_once_t is corrupt`, seen as
//! `Illegal instruction: 4` on Intel. The `pre_exec` closure itself also
//! allocates (`read_dir("/dev/fd")`) between fork and exec.
//!
//! `posix_spawn` creates the child in the kernel: no atfork handlers and no
//! user code run between the parent and the new image. Everything is prepared
//! in the parent, and the spawn attributes keep the `portable-pty` semantics:
//! a new session with the PTY as controlling terminal and stdio, only stdio
//! inherited, the same signals reset to their defaults, and an empty mask.

use std::ffi::{CStr, CString, OsStr, OsString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use portable_pty::{Child, ChildKiller, ExitStatus};

/// `<spawn.h>`: start the child in a new session (macOS 10.15+). Not exported
/// by the `libc` crate.
const POSIX_SPAWN_SETSID: libc::c_int = 0x0400;

/// Signals `portable-pty` resets to `SIG_DFL` in the child, plus `SIGPIPE`,
/// which std's spawn also resets.
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

    let path = cstring(resolved.into_os_string().into_vec())?;
    let tty = cstring(tty.as_os_str().as_bytes().to_vec())?;
    let cwd = cstring(cwd.into_os_string().into_vec())?;
    let argv = std::iter::once(program)
        .chain(args.iter().map(OsString::as_os_str))
        .map(|arg| cstring(arg.as_bytes().to_vec()))
        .collect::<io::Result<Vec<_>>>()?;
    let envp = env
        .iter()
        .map(|(key, value)| {
            let mut entry = key.as_bytes().to_vec();
            entry.push(b'=');
            entry.extend_from_slice(value.as_bytes());
            cstring(entry)
        })
        .collect::<io::Result<Vec<_>>>()?;
    let argv_ptrs = null_terminated(&argv);
    let envp_ptrs = null_terminated(&envp);

    let mut attrs = SpawnAttrs::new()?;
    let mut actions = FileActions::new()?;
    // SAFETY: every pointer is to a live, NUL-terminated buffer owned above
    // for the whole call; the attribute and action objects are initialized
    // and destroyed by their guards.
    unsafe {
        let flags = POSIX_SPAWN_SETSID
            | libc::POSIX_SPAWN_CLOEXEC_DEFAULT
            | libc::POSIX_SPAWN_SETSIGDEF
            | libc::POSIX_SPAWN_SETSIGMASK;
        check(libc::posix_spawnattr_setflags(
            attrs.as_mut_ptr(),
            flags as _,
        ))?;
        let mut defaults: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut defaults);
        for signal in DEFAULT_SIGNALS {
            libc::sigaddset(&mut defaults, signal);
        }
        check(libc::posix_spawnattr_setsigdefault(
            attrs.as_mut_ptr(),
            &defaults,
        ))?;
        let mut mask: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut mask);
        check(libc::posix_spawnattr_setsigmask(attrs.as_mut_ptr(), &mask))?;

        check(libc::posix_spawn_file_actions_addchdir_np(
            actions.as_mut_ptr(),
            cwd.as_ptr(),
        ))?;
        // A session leader that opens a terminal without O_NOCTTY acquires it
        // as its controlling terminal, which replaces portable-pty's
        // TIOCSCTTY. POSIX_SPAWN_SETSID runs before the file actions.
        check(libc::posix_spawn_file_actions_addopen(
            actions.as_mut_ptr(),
            0,
            tty.as_ptr(),
            libc::O_RDWR,
            0,
        ))?;
        check(libc::posix_spawn_file_actions_adddup2(
            actions.as_mut_ptr(),
            0,
            1,
        ))?;
        check(libc::posix_spawn_file_actions_adddup2(
            actions.as_mut_ptr(),
            0,
            2,
        ))?;

        let mut pid: libc::pid_t = 0;
        check(libc::posix_spawn(
            &mut pid,
            path.as_ptr(),
            actions.as_mut_ptr(),
            attrs.as_mut_ptr(),
            argv_ptrs.as_ptr(),
            envp_ptrs.as_ptr(),
        ))?;
        Ok(Box::new(SpawnedChild { pid, status: None }))
    }
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
    // SAFETY: getpwuid returns null or a pointer to static storage that stays
    // valid until the next getpw* call; the field is copied out immediately.
    unsafe {
        let entry = libc::getpwuid(libc::getuid());
        if entry.is_null() {
            return None;
        }
        let value = field(&*entry);
        (!value.is_null()).then(|| OsStr::from_bytes(CStr::from_ptr(value).to_bytes()).into())
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

fn cstring(bytes: Vec<u8>) -> io::Result<CString> {
    CString::new(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "PTY child argument, path or environment contains a NUL byte",
        )
    })
}

fn null_terminated(values: &[CString]) -> Vec<*mut libc::c_char> {
    values
        .iter()
        .map(|value| value.as_ptr().cast_mut())
        .chain(std::iter::once(std::ptr::null_mut()))
        .collect()
}

/// posix_spawn functions return the error number instead of setting errno.
fn check(code: libc::c_int) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code))
    }
}

struct SpawnAttrs(libc::posix_spawnattr_t);

impl SpawnAttrs {
    fn new() -> io::Result<Self> {
        let mut attrs: libc::posix_spawnattr_t = std::ptr::null_mut();
        // SAFETY: initializes the opaque object the guard destroys on drop.
        check(unsafe { libc::posix_spawnattr_init(&mut attrs) })?;
        Ok(Self(attrs))
    }

    fn as_mut_ptr(&mut self) -> *mut libc::posix_spawnattr_t {
        &mut self.0
    }
}

impl Drop for SpawnAttrs {
    fn drop(&mut self) {
        // SAFETY: initialized in `new` and destroyed exactly once.
        unsafe { libc::posix_spawnattr_destroy(&mut self.0) };
    }
}

struct FileActions(libc::posix_spawn_file_actions_t);

impl FileActions {
    fn new() -> io::Result<Self> {
        let mut actions: libc::posix_spawn_file_actions_t = std::ptr::null_mut();
        // SAFETY: initializes the opaque object the guard destroys on drop.
        check(unsafe { libc::posix_spawn_file_actions_init(&mut actions) })?;
        Ok(Self(actions))
    }

    fn as_mut_ptr(&mut self) -> *mut libc::posix_spawn_file_actions_t {
        &mut self.0
    }
}

impl Drop for FileActions {
    fn drop(&mut self) {
        // SAFETY: initialized in `new` and destroyed exactly once.
        unsafe { libc::posix_spawn_file_actions_destroy(&mut self.0) };
    }
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
        loop {
            let mut raw = 0;
            // SAFETY: waits only for this child's pid; `raw` is a valid out
            // pointer.
            let result = unsafe { libc::waitpid(self.pid, &mut raw, flags) };
            if result == 0 {
                return Ok(None);
            }
            if result == -1 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            let status = exit_status(raw);
            self.status = Some(status.clone());
            return Ok(Some(status));
        }
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
