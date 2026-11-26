//! Fork-free child creation shared by the macOS PTY and detached spawns.
//!
//! std's `Command` with a `pre_exec` closure makes std `fork()`. In a
//! multithreaded process, macOS's `fork()` runs libSystem's atfork child
//! handlers in the child; if another thread was inside a one-time libSystem
//! initialization at that moment (for example the first libnotify use), the
//! child's handler waits on a once-gate whose owner thread does not exist in
//! the child and traps: `BUG IN CLIENT OF LIBPLATFORM: os_once_t is corrupt`,
//! seen as `Illegal instruction: 4` on Intel.
//!
//! `posix_spawn` creates the child in the kernel: no atfork handlers and no
//! user code run between the parent and the new image. Callers prepare every
//! path, argument and environment entry in the parent.

use std::ffi::{CStr, CString, OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// `<spawn.h>`: start the child in a new session (macOS 10.15+). Not exported
/// by the `libc` crate.
const POSIX_SPAWN_SETSID: libc::c_int = 0x0400;

/// A child that [`prepare`] readies: its own session, only the descriptors
/// the file actions set up, an empty signal mask, and `default_signals` reset
/// to `SIG_DFL` (other dispositions as inherited from this process).
pub(super) struct SpawnRequest<'a> {
    /// Names the child in errors, for example `"PTY child"`.
    pub(super) what: &'static str,
    /// Absolute path of the image to execute.
    pub(super) path: &'a Path,
    /// `argv`, including `argv[0]`.
    pub(super) argv: Vec<&'a OsStr>,
    /// The complete child environment.
    pub(super) env: Vec<(&'a OsStr, &'a OsStr)>,
    pub(super) default_signals: &'a [libc::c_int],
}

/// Build every C string, pointer array and spawn attribute for `request`, so
/// [`PreparedSpawn::spawn`] does nothing but call `posix_spawn`.
pub(super) fn prepare(
    request: &SpawnRequest<'_>,
    actions: FileActions,
) -> io::Result<PreparedSpawn> {
    let cstring = |bytes: &[u8]| cstring(request.what, bytes);
    let path = cstring(request.path.as_os_str().as_bytes())?;
    let argv = request
        .argv
        .iter()
        .map(|arg| cstring(arg.as_bytes()))
        .collect::<io::Result<Vec<_>>>()?;
    let envp = request
        .env
        .iter()
        .map(|(key, value)| {
            let mut entry = key.as_bytes().to_vec();
            entry.push(b'=');
            entry.extend_from_slice(value.as_bytes());
            cstring(&entry)
        })
        .collect::<io::Result<Vec<_>>>()?;
    // execvp's ENOEXEC fallback: `sh <path> <args...>`, as Apple's execvp.
    let script_argv = [cstring(b"sh")?, path.clone()]
        .into_iter()
        .chain(argv.iter().skip(1).cloned())
        .collect::<Vec<_>>();

    let mut attrs = SpawnAttrs::new()?;
    // SAFETY: the sigsets are initialized before use and the attribute object
    // is initialized and destroyed by its guard.
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
        for signal in request.default_signals {
            libc::sigaddset(&mut defaults, *signal);
        }
        check(libc::posix_spawnattr_setsigdefault(
            attrs.as_mut_ptr(),
            &defaults,
        ))?;
        let mut mask: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut mask);
        check(libc::posix_spawnattr_setsigmask(attrs.as_mut_ptr(), &mask))?;
    }
    let argv_ptrs = null_terminated(&argv);
    let script_argv_ptrs = null_terminated(&script_argv);
    let envp_ptrs = null_terminated(&envp);
    Ok(PreparedSpawn {
        path,
        _argv: argv,
        _script_argv: script_argv,
        _envp: envp,
        argv_ptrs,
        script_argv_ptrs,
        envp_ptrs,
        attrs,
        actions,
    })
}

/// A spawn whose inputs are all built. The pointer arrays point into the
/// owned `CString`s, whose heap buffers do not move with the struct.
pub(super) struct PreparedSpawn {
    path: CString,
    _argv: Vec<CString>,
    _script_argv: Vec<CString>,
    _envp: Vec<CString>,
    argv_ptrs: Vec<*mut libc::c_char>,
    script_argv_ptrs: Vec<*mut libc::c_char>,
    envp_ptrs: Vec<*mut libc::c_char>,
    attrs: SpawnAttrs,
    actions: FileActions,
}

impl PreparedSpawn {
    /// Start the child with the file actions applied after `setsid()`.
    ///
    /// Like `execvp`, an image the kernel rejects with `ENOEXEC` is run once
    /// more as a `/bin/sh` script.
    pub(super) fn spawn(mut self) -> io::Result<libc::pid_t> {
        let mut pid: libc::pid_t = 0;
        let mut spawn = |program: &CStr, argv: &[*mut libc::c_char]| {
            // SAFETY: every pointer is to a live, NUL-terminated buffer owned
            // by `self`, and the attribute and action objects are initialized.
            unsafe {
                libc::posix_spawn(
                    &mut pid,
                    program.as_ptr(),
                    self.actions.as_mut_ptr(),
                    self.attrs.as_mut_ptr(),
                    argv.as_ptr(),
                    self.envp_ptrs.as_ptr(),
                )
            }
        };
        let mut code = spawn(&self.path, &self.argv_ptrs);
        if code == libc::ENOEXEC {
            code = spawn(c"/bin/sh", &self.script_argv_ptrs);
        }
        check(code)?;
        Ok(pid)
    }
}

/// Owned `posix_spawn_file_actions_t`, applied in order in the child.
pub(super) struct FileActions(libc::posix_spawn_file_actions_t, &'static str);

impl FileActions {
    /// `what` names the child in errors, as in [`SpawnRequest::what`].
    pub(super) fn new(what: &'static str) -> io::Result<Self> {
        let mut actions: libc::posix_spawn_file_actions_t = std::ptr::null_mut();
        // SAFETY: initializes the opaque object the guard destroys on drop.
        check(unsafe { libc::posix_spawn_file_actions_init(&mut actions) })?;
        Ok(Self(actions, what))
    }

    pub(super) fn chdir(&mut self, dir: &Path) -> io::Result<()> {
        let dir = cstring(self.1, dir.as_os_str().as_bytes())?;
        // The `_np` form is intentional: the release deployment target is macOS
        // 12.0, and the unsuffixed posix_spawn_file_actions_addchdir only
        // exists from macOS 26. The path is copied into the actions.
        // SAFETY: `dir` is NUL-terminated and outlives the call.
        check(unsafe { libc::posix_spawn_file_actions_addchdir_np(&mut self.0, dir.as_ptr()) })
    }

    pub(super) fn open(
        &mut self,
        fd: libc::c_int,
        path: &Path,
        flags: libc::c_int,
    ) -> io::Result<()> {
        let path = cstring(self.1, path.as_os_str().as_bytes())?;
        // SAFETY: `path` is NUL-terminated and outlives the call, which copies
        // it into the actions.
        check(unsafe {
            libc::posix_spawn_file_actions_addopen(&mut self.0, fd, path.as_ptr(), flags, 0)
        })
    }

    /// `dup2(from, to)` in the child; `to` is inherited despite
    /// `POSIX_SPAWN_CLOEXEC_DEFAULT`.
    pub(super) fn dup2(&mut self, from: libc::c_int, to: libc::c_int) -> io::Result<()> {
        // SAFETY: plain descriptor numbers recorded for the child.
        check(unsafe { libc::posix_spawn_file_actions_adddup2(&mut self.0, from, to) })
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

fn cstring(what: &str, bytes: &[u8]) -> io::Result<CString> {
    CString::new(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{what} argument, path or environment contains a NUL byte"),
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

/// Wait for `pid`, retrying on `EINTR`; `flags` as for `waitpid`.
/// Returns `Ok(None)` when `WNOHANG` finds it still running.
pub(super) fn wait_pid(pid: libc::pid_t, flags: libc::c_int) -> io::Result<Option<libc::c_int>> {
    loop {
        let mut raw = 0;
        // SAFETY: waits only for this child's pid; `raw` is a valid out pointer.
        let result = unsafe { libc::waitpid(pid, &mut raw, flags) };
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
        return Ok(Some(raw));
    }
}

/// Borrow owned environment pairs for a [`SpawnRequest`].
pub(super) fn env_refs(env: &[(OsString, OsString)]) -> Vec<(&OsStr, &OsStr)> {
    env.iter()
        .map(|(key, value)| (key.as_os_str(), value.as_os_str()))
        .collect()
}
