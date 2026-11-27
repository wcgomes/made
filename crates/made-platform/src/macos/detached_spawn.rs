//! Detached child spawn through `posix_spawn(2)` instead of `fork(2)`.
//!
//! The generic Unix path forks with a `pre_exec` closure; see
//! [`super::spawn`] for why that can kill the child on macOS. The child keeps
//! that path's setup: a new session with no controlling terminal, stdin from
//! the caller's pipe, stdout and stderr on `/dev/null`, only those three
//! descriptors inherited, `SIGHUP` ignored, `SIGPIPE` at its default, an empty
//! signal mask, and this process's environment minus `TMUX` and `TMUX_PANE`.

use std::ffi::OsString;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::path::Path;
use std::sync::Mutex;

use super::spawn::{env_refs, prepare, wait_pid, FileActions, SpawnRequest};

/// Serializes the process-wide `SIGHUP` disposition change in
/// [`spawn_detached_child`].
static SIGHUP_IGNORED: Mutex<()> = Mutex::new(());

/// A detached child started by [`spawn_detached_child`].
#[derive(Debug)]
pub(crate) struct DetachedChild {
    pid: libc::pid_t,
}

impl DetachedChild {
    pub(crate) fn id(&self) -> u32 {
        self.pid as u32
    }

    /// Block until the child exits and reap it.
    pub(crate) fn wait(&self) {
        let _ = wait_pid(self.pid, 0);
    }
}

/// Start `program` (absolute) with `args`, in the open directory `cwd`,
/// reading `stdin`.
pub(crate) fn spawn_detached_child(
    program: &Path,
    args: &[OsString],
    cwd: BorrowedFd<'_>,
    stdin: BorrowedFd<'_>,
) -> io::Result<DetachedChild> {
    let env: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(key, _)| key != "TMUX" && key != "TMUX_PANE")
        .collect();
    let request = SpawnRequest {
        what: "detached child",
        path: program,
        argv: std::iter::once(program.as_os_str())
            .chain(args.iter().map(OsString::as_os_str))
            .collect(),
        env: env_refs(&env),
        default_signals: &[libc::SIGPIPE],
    };
    let mut actions = FileActions::new("detached child")?;
    // By descriptor: the directory the caller validated, never a path that
    // could have been replaced since. The descriptor is not dup'd onto 0-2, so
    // POSIX_SPAWN_CLOEXEC_DEFAULT closes it at exec, after the file actions.
    actions.fchdir(cwd.as_raw_fd())?;
    actions.dup2(stdin.as_raw_fd(), 0)?;
    actions.open(1, Path::new("/dev/null"), libc::O_WRONLY)?;
    actions.open(2, Path::new("/dev/null"), libc::O_WRONLY)?;

    let prepared = prepare(&request, actions)?;

    // posix_spawn has no attribute that ignores a signal; a child only
    // inherits SIG_IGN from the parent's disposition at spawn time. So SIGHUP
    // is ignored process-wide only around the posix_spawn call itself (about a
    // millisecond; everything else is prepared above), and the guard restores
    // the saved action on every path. During that window a SIGHUP sent to this
    // process is discarded, and any other child spawned concurrently by
    // another thread inherits the ignored SIGHUP unless it resets it (the PTY
    // spawn does). The restore is unconditional, so a SIGHUP handler that
    // another thread installed during the window would be reverted to the
    // saved action; nothing in Made installs one. The mutex keeps concurrent
    // detached spawns from restoring each other's saved disposition.
    let pid = {
        let _ignored = SighupIgnored::new()?;
        prepared.spawn()?
    };
    Ok(DetachedChild { pid })
}

/// Ignores `SIGHUP` until dropped, then restores the previous action.
struct SighupIgnored {
    previous: libc::sigaction,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl SighupIgnored {
    fn new() -> io::Result<Self> {
        let lock = SIGHUP_IGNORED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: both sigaction structs are fully initialized; SIG_IGN is a
        // sentinel, not a function pointer that is called.
        unsafe {
            let mut ignore: libc::sigaction = std::mem::zeroed();
            ignore.sa_sigaction = libc::SIG_IGN;
            libc::sigemptyset(&mut ignore.sa_mask);
            let mut previous: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(libc::SIGHUP, &ignore, &mut previous) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                previous,
                _lock: lock,
            })
        }
    }
}

impl Drop for SighupIgnored {
    fn drop(&mut self) {
        // SAFETY: restores the action saved by `new`.
        unsafe { libc::sigaction(libc::SIGHUP, &self.previous, std::ptr::null_mut()) };
    }
}
