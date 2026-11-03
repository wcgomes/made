//! Portable supervisor detach and the private file operations the companion
//! already calls.
//!
//! Unix detach starts a new session, ignores `SIGHUP`, closes inherited stdio,
//! and changes to a private runtime directory. Windows and other unsupported
//! families fail preflight. There is no multiplexer fallback.
//!
//! Companion-facing signatures, kept stable for the parallel remote runtime:
//! - `remote_target() -> Result<String, _>` so callers can use `.as_str()`
//! - `remote_program_version_supported(&Path, &str) -> Result<bool, _>`
//! - `remote_path_bytes(&Path) -> Result<Vec<u8>, _>`
//! - `remote_path_from_bytes(&[u8]) -> Result<PathBuf, _>`
//! - `ensure_private_directory(&Path) -> Result<(), _>`
//! - `create_private_directory_new(&Path) -> Result<(), _>` exclusive create
//! - `create_private_file_new(&Path, bool) -> Result<File, _>`
//! - `lock_private_file(&Path, Duration) -> Result<File, _>` held until drop
//! - `read_private_file(&Path, usize) -> Result<Vec<u8>, _>`
//! - `spawn_detached_command(&Path, &[OsString], &[u8]) -> Result<(), _>`
//! - `exec_remote_child(&PtyRequest) -> Result<(), _>` does not return on success
//!
//! `exec_remote_child` requires an absolute program. A relative shell name fails
//! closed instead of searching `PATH`. `remote_program_version_supported` never
//! executes `tmux`; that name is `Ok(false)`.

use crate::{PlatformError, PtyRequest};
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const REMOTE_PATH_MAX_BYTES: usize = 4096;
const PRIVATE_READ_MAX_BYTES: usize = 16 * 1024 * 1024;
const DETACH_STDIN_MAX_BYTES: usize = 4 + 256 * 1024;
const VERSION_PROBE_LIMIT: usize = 256;
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Compile-time remote companion triple, or a preflight error.
///
/// Windows, musl, and unknown architectures fail. This does not execute a
/// multiplexer or any other user program.
pub fn remote_target() -> Result<String, PlatformError> {
    let name = if cfg!(all(
        target_os = "linux",
        target_env = "gnu",
        target_arch = "x86_64"
    )) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(
        target_os = "linux",
        target_env = "gnu",
        target_arch = "aarch64"
    )) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else {
        return Err(preflight("remote target"));
    };
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    check_glibc()?;
    Ok(name.to_owned())
}

/// Whether a remote program is a supported optional capability.
///
/// `tmux` is never executed and is never supported. `opencode` may run a
/// bounded `--version` with no PTY and no inherited secret environment. Any
/// other name is rejected without execution.
pub fn remote_program_version_supported(path: &Path, program: &str) -> Result<bool, PlatformError> {
    if program == "tmux" {
        return Ok(false);
    }
    if program != "opencode" {
        return Err(PlatformError::message(
            "remote version probe is limited to opencode",
        ));
    }
    if !path.is_absolute() || !file_name_is(path, program) {
        return Err(PlatformError::io(
            "remote version probe",
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "version probe program must be an absolute opencode path",
            ),
        ));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        return Err(preflight("remote program version probe"));
    }
    #[cfg(unix)]
    {
        probe_opencode_version(path)
    }
}

/// Lossless POSIX path bytes. Not a host display conversion.
pub fn remote_path_bytes(path: &Path) -> Result<Vec<u8>, PlatformError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let bytes = path.as_os_str().as_bytes();
        validate_remote_path(bytes)?;
        Ok(bytes.to_vec())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(preflight("remote POSIX path"))
    }
}

/// Inverse of [`remote_path_bytes`]. Rejects the same lexical escapes as the
/// protocol path type, without touching the filesystem.
pub fn remote_path_from_bytes(bytes: &[u8]) -> Result<PathBuf, PlatformError> {
    validate_remote_path(bytes)?;
    #[cfg(unix)]
    {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        Ok(PathBuf::from(OsStr::from_bytes(bytes)))
    }
    #[cfg(not(unix))]
    {
        Err(preflight("remote POSIX path"))
    }
}

/// Create missing directories and require the final directory to be owner-only.
///
/// Existing ancestors are not chmod'd. A symlink at the final path, or at a
/// missing component's first existing ancestor, is refused. Newly created
/// directories are mode `0700`.
pub fn ensure_private_directory(path: &Path) -> Result<(), PlatformError> {
    #[cfg(unix)]
    {
        unix::ensure_private_directory(path)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(preflight("private directory"))
    }
}

/// Exclusive private directory create. Fails if the final component exists.
///
/// The parent must already be an owner-only directory. This is the call used
/// to refuse replacing a live runtime or staging directory.
pub fn create_private_directory_new(path: &Path) -> Result<(), PlatformError> {
    #[cfg(unix)]
    {
        unix::create_private_directory_new(path)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(preflight("private directory"))
    }
}

/// Create a new regular file that did not exist. `executable` selects `0700`
/// instead of `0600`. The returned file is readable and writable.
pub fn create_private_file_new(path: &Path, executable: bool) -> Result<File, PlatformError> {
    #[cfg(unix)]
    {
        unix::create_private_file_new(path, executable)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, executable);
        Err(preflight("private file"))
    }
}

/// Open or create a lock file and take an exclusive advisory lock.
///
/// `Duration::ZERO` tries once. The lock is released when the returned file is
/// dropped. Symlinks and non-regular files are refused.
pub fn lock_private_file(path: &Path, timeout: Duration) -> Result<File, PlatformError> {
    #[cfg(unix)]
    {
        unix::lock_private_file(path, timeout)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, timeout);
        Err(preflight("private file lock"))
    }
}

/// Read a private regular file without following a symlink.
///
/// Group and other permission bits must be clear. Hard links are refused.
pub fn read_private_file(path: &Path, max_bytes: usize) -> Result<Vec<u8>, PlatformError> {
    if max_bytes == 0 || max_bytes > PRIVATE_READ_MAX_BYTES {
        return Err(PlatformError::io(
            "read private file",
            io::Error::new(io::ErrorKind::InvalidInput, "private read bound is invalid"),
        ));
    }
    #[cfg(unix)]
    {
        unix::read_private_file(path, max_bytes)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(preflight("private file"))
    }
}

/// Spawn `program` outside the caller's session.
///
/// `stdin_payload` is written to a fresh pipe and that pipe is closed. Stdout
/// and stderr are `/dev/null`. Extra inherited descriptors are closed. The
/// child ignores `SIGHUP` and uses a new private directory as its cwd. The
/// caller does not wait. Windows fails preflight.
pub fn spawn_detached_command(
    program: &Path,
    args: &[OsString],
    stdin_payload: &[u8],
) -> Result<(), PlatformError> {
    if stdin_payload.len() > DETACH_STDIN_MAX_BYTES {
        return Err(PlatformError::io(
            "spawn detached command",
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "detached stdin payload exceeds the frame bound",
            ),
        ));
    }
    #[cfg(unix)]
    {
        unix::spawn_detached_command(program, args, stdin_payload)
    }
    #[cfg(not(unix))]
    {
        let _ = (program, args);
        Err(preflight("supervisor detach"))
    }
}

/// Replace this process with `request`'s program. Does not return on success.
///
/// Arguments, cwd, and environment come from the request. Nothing is joined
/// into a shell command. The program must be absolute. Inherited stdio is kept
/// so a PTY slave arranged by the caller remains attached; descriptors above
/// stderr are closed.
pub fn exec_remote_child(request: &PtyRequest) -> Result<(), PlatformError> {
    #[cfg(unix)]
    {
        unix::exec_remote_child(request)
    }
    #[cfg(not(unix))]
    {
        let _ = request;
        Err(preflight("remote child exec"))
    }
}

fn preflight(what: &str) -> PlatformError {
    PlatformError::Unsupported(format!(
        "{what}; Windows and unsupported remote families fail preflight with no multiplexer fallback"
    ))
}

fn validate_remote_path(bytes: &[u8]) -> Result<(), PlatformError> {
    let invalid = || {
        PlatformError::io(
            "remote POSIX path",
            io::Error::new(io::ErrorKind::InvalidInput, "invalid remote POSIX path"),
        )
    };
    if bytes.len() > REMOTE_PATH_MAX_BYTES
        || bytes.first() != Some(&b'/')
        || bytes.contains(&0)
        || (bytes.len() > 1
            && bytes[1..]
                .split(|byte| *byte == b'/')
                .any(|part| part.is_empty() || part == b"." || part == b".."))
    {
        return Err(invalid());
    }
    Ok(())
}

fn file_name_is(path: &Path, program: &str) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == program || name.strip_suffix(".exe") == Some(program)
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn check_glibc() -> Result<(), PlatformError> {
    // SAFETY: gnu_get_libc_version returns a non-null, static, NUL-terminated
    // version string on glibc. It has no pointer arguments.
    let raw = unsafe { libc::gnu_get_libc_version() };
    if raw.is_null() {
        return Err(preflight("glibc version"));
    }
    // SAFETY: the pointer above is a valid C string for the process lifetime.
    let text = unsafe { std::ffi::CStr::from_ptr(raw) }.to_string_lossy();
    let mut parts = text.split('.');
    let major: u32 = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| preflight("glibc version"))?;
    let minor: u32 = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| preflight("glibc version"))?;
    if major > 2 || (major == 2 && minor >= 31) {
        Ok(())
    } else {
        Err(preflight("glibc older than 2.31"))
    }
}

#[cfg(unix)]
fn probe_opencode_version(path: &Path) -> Result<bool, PlatformError> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let mut child = Command::new(path)
        .arg("--version")
        .env_clear()
        .env("PATH", path_var)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|error| PlatformError::io("probe opencode version", error))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| PlatformError::message("opencode version probe has no stdout"))?;
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout
            .take(VERSION_PROBE_LIMIT as u64)
            .read_to_end(&mut output);
        output
    });
    let deadline = Instant::now() + VERSION_PROBE_TIMEOUT;
    let finished = loop {
        match child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => break false,
            Err(error) => {
                signal_process_group(child.id(), libc::SIGKILL);
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(PlatformError::io("probe opencode version", error));
            }
        }
    };
    if !finished {
        signal_process_group(child.id(), libc::SIGKILL);
        let _ = child.kill();
        let _ = child.wait();
        let _ = reader.join();
        return Ok(false);
    }
    let output = reader.join().unwrap_or_default();
    Ok(output_is_opencode_v2(&output))
}

fn output_is_opencode_v2(output: &[u8]) -> bool {
    let text = String::from_utf8_lossy(output);
    text.split(|character: char| !character.is_ascii_digit() && character != '.')
        .filter(|token| !token.is_empty())
        .any(|token| {
            let mut parts = token.split('.');
            matches!((parts.next(), parts.next()), (Some("2"), Some(minor)) if !minor.is_empty())
        })
}

#[cfg(unix)]
fn signal_process_group(pid: u32, signal: libc::c_int) {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return;
    };
    if pid <= 1 {
        return;
    }
    // SAFETY: pid came from a child spawned by this process. A negative value
    // names that child's process group. ESRCH means it has already exited.
    unsafe {
        libc::kill(-pid, signal);
        libc::kill(pid, signal);
    }
}

#[cfg(unix)]
fn close_extra_fds() {
    // SAFETY: sysconf(_SC_OPEN_MAX) has no pointer arguments. close on an
    // unused descriptor returns EBADF, which is ignored. Descriptors 0, 1, and
    // 2 are the caller's arranged stdio and are left open.
    let max = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
    let max = if !(3..=1024).contains(&max) {
        256
    } else {
        max as i32
    };
    for fd in 3..max {
        unsafe {
            libc::close(fd);
        }
    }
}

#[cfg(unix)]
fn ignore_sighup() -> io::Result<()> {
    // SAFETY: the sigaction is fully initialized before use. SIG_IGN is a
    // sentinel handler, not a function pointer we call. sa_mask is emptied.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = libc::SIG_IGN;
        libc::sigemptyset(&mut action.sa_mask);
        if libc::sigaction(libc::SIGHUP, &action, std::ptr::null_mut()) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(unix)]
mod unix {
    use super::{close_extra_fds, ignore_sighup, signal_process_group, PlatformError};
    use crate::PtyRequest;
    use std::ffi::{CString, OsStr, OsString};
    use std::fs::File;
    use std::io::Read;
    use std::io::{self, Write};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    pub(super) fn ensure_private_directory(path: &Path) -> Result<(), PlatformError> {
        let (ancestor, pending) = split_existing_ancestor(path)?;
        let mut directory = open_directory(&ancestor)?;
        if pending.is_empty() {
            publish_directory(&directory)?;
            return Ok(());
        }
        for (index, name) in pending.iter().enumerate() {
            let created = match mkdir_at(&directory, name) {
                Ok(()) => true,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => false,
                Err(error) => return Err(PlatformError::io("create private directory", error)),
            };
            let next = open_at(
                &directory,
                name,
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0,
            )
            .map_err(|error| PlatformError::io("open private directory", error))?;
            if created || index + 1 == pending.len() {
                publish_directory(&next)?;
            } else if !next.metadata().map(|meta| meta.is_dir()).unwrap_or(false) {
                return Err(denied("private directory ancestor is not a directory"));
            }
            directory = next;
        }
        Ok(())
    }

    pub(super) fn create_private_directory_new(path: &Path) -> Result<(), PlatformError> {
        let (parent, name) = parent_and_name(path)?;
        let parent = open_private_parent(&parent)?;
        match mkdir_at(&parent, name) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(PlatformError::io(
                    "create private directory",
                    io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "private directory already exists",
                    ),
                ));
            }
            Err(error) => return Err(PlatformError::io("create private directory", error)),
        }
        let created = open_at(
            &parent,
            name,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0,
        )
        .map_err(|error| PlatformError::io("open private directory", error))?;
        if let Err(error) = publish_directory(&created) {
            let _ = unlink_at(&parent, name, true);
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn create_private_file_new(
        path: &Path,
        executable: bool,
    ) -> Result<File, PlatformError> {
        let (parent, name) = parent_and_name(path)?;
        let parent = open_private_parent(&parent)?;
        let mode: libc::mode_t = if executable { 0o700 } else { 0o600 };
        let file = open_at(
            &parent,
            name,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode,
        )
        .map_err(|error| PlatformError::io("create private file", error))?;
        if let Err(error) = publish_file(&file, mode) {
            let _ = unlink_at(&parent, name, false);
            return Err(error);
        }
        Ok(file)
    }

    pub(super) fn lock_private_file(path: &Path, timeout: Duration) -> Result<File, PlatformError> {
        let (parent, name) = parent_and_name(path)?;
        let parent = open_private_parent(&parent)?;
        let file = open_at(
            &parent,
            name,
            libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
        .map_err(|error| PlatformError::io("open private lock", error))?;
        publish_file(&file, 0o600)?;
        wait_for_lock(&file, timeout)?;
        Ok(file)
    }

    pub(super) fn read_private_file(
        path: &Path,
        max_bytes: usize,
    ) -> Result<Vec<u8>, PlatformError> {
        let (parent, name) = parent_and_name(path)?;
        let parent = open_directory(&parent)
            .map_err(|error| PlatformError::io("open private file parent", error))?;
        let file = open_at(
            &parent,
            name,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0,
        )
        .map_err(|error| PlatformError::io("open private file", error))?;
        let meta = file
            .metadata()
            .map_err(|error| PlatformError::io("inspect private file", error))?;
        let mode = meta.mode() & 0o777;
        if !meta.is_file()
            || meta.uid() != current_euid()
            || meta.nlink() != 1
            || mode & 0o077 != 0
            || mode & 0o400 == 0
        {
            return Err(denied("private file ownership, mode, or link count"));
        }
        if meta.len() > max_bytes as u64 {
            return Err(PlatformError::io(
                "read private file",
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "private file exceeds the read bound",
                ),
            ));
        }
        let mut bytes = Vec::new();
        file.take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| PlatformError::io("read private file", error))?;
        if bytes.len() > max_bytes {
            return Err(PlatformError::io(
                "read private file",
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "private file exceeds the read bound",
                ),
            ));
        }
        Ok(bytes)
    }

    pub(super) fn spawn_detached_command(
        program: &Path,
        args: &[OsString],
        stdin_payload: &[u8],
    ) -> Result<(), PlatformError> {
        use std::os::unix::process::CommandExt;

        if !program.is_absolute() {
            return Err(PlatformError::io(
                "spawn detached command",
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "detached program must be absolute",
                ),
            ));
        }
        let runtime = create_detach_runtime()?;
        let runtime_c = CString::new(runtime.as_os_str().as_bytes()).map_err(|_| {
            PlatformError::io(
                "spawn detached command",
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "private runtime path is invalid",
                ),
            )
        })?;
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        // SAFETY: the closure runs in the forked child before exec. It only
        // calls setsid, sigaction, chdir, and close. The CString is owned and
        // already allocated. Failure aborts the child and spawn returns that
        // error; the runtime directory is then removed below.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                ignore_sighup()?;
                if libc::chdir(runtime_c.as_ptr()) < 0 {
                    return Err(io::Error::last_os_error());
                }
                close_extra_fds();
                Ok(())
            });
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = std::fs::remove_dir(&runtime);
                return Err(PlatformError::io("spawn detached command", error));
            }
        };
        let write_result = (|| {
            let mut stdin = child.stdin.take().ok_or_else(|| {
                io::Error::new(io::ErrorKind::BrokenPipe, "detached child has no stdin")
            })?;
            stdin.write_all(stdin_payload)?;
            stdin.flush()?;
            Ok(())
        })();
        if let Err(error) = write_result {
            signal_process_group(child.id(), libc::SIGKILL);
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir(&runtime);
            return Err(PlatformError::io("write detached stdin", error));
        }
        // The child is in a new session. Reap it only if it exits while this
        // process is still alive; a parent hangup does not signal that session.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    pub(super) fn exec_remote_child(request: &PtyRequest) -> Result<(), PlatformError> {
        use std::os::unix::process::CommandExt;

        if !Path::new(&request.program).is_absolute() || !request.cwd.is_absolute() {
            return Err(PlatformError::io(
                "exec remote child",
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "remote child program and cwd must be absolute",
                ),
            ));
        }
        let mut command = Command::new(&request.program);
        command
            .args(&request.args)
            .current_dir(&request.cwd)
            .env_clear()
            .envs(request.env.iter().cloned());
        for key in &request.env_remove {
            command.env_remove(key);
        }
        // SAFETY: this process is the child-exec wrapper. Closing descriptors
        // above stderr drops inherited supervisor sockets before the image is
        // replaced. stdio stays as the caller arranged it, typically the PTY.
        unsafe {
            command.pre_exec(|| {
                close_extra_fds();
                Ok(())
            });
        }
        Err(PlatformError::io("exec remote child", command.exec()))
    }

    fn create_detach_runtime() -> Result<PathBuf, PlatformError> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let parent = std::env::temp_dir();
        if std::fs::symlink_metadata(&parent)
            .map(|meta| meta.file_type().is_symlink() || !meta.is_dir())
            .unwrap_or(true)
        {
            return Err(denied("temp directory is not a real directory"));
        }
        let name = format!(
            "made-detach-{}-{}-{}",
            current_euid(),
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = parent.join(name);
        let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            PlatformError::io(
                "create private runtime",
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "private runtime path is invalid",
                ),
            )
        })?;
        // SAFETY: c_path is a NUL-terminated owned path. mkdir creates only the
        // final component and does not follow it if it already exists.
        let rc = unsafe { libc::mkdir(c_path.as_ptr(), 0o700) };
        if rc != 0 {
            return Err(PlatformError::io(
                "create private runtime",
                io::Error::last_os_error(),
            ));
        }
        let file = open_directory(&path)?;
        publish_directory(&file)?;
        Ok(path)
    }

    fn split_existing_ancestor(path: &Path) -> Result<(PathBuf, Vec<OsString>), PlatformError> {
        if !path.is_absolute() {
            return Err(denied("private directory must be absolute"));
        }
        let mut pending = Vec::new();
        let mut current = path.to_path_buf();
        loop {
            match std::fs::symlink_metadata(&current) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(denied("refusing symlink in private directory path"));
                }
                Ok(meta) if meta.is_dir() => {
                    pending.reverse();
                    return Ok((current, pending));
                }
                Ok(_) => return Err(denied("private directory path is not a directory")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    let Some(name) = current.file_name().map(OsStr::to_owned) else {
                        return Err(denied("private directory ancestor is missing"));
                    };
                    if !valid_component(&name) {
                        return Err(denied("private directory component is invalid"));
                    }
                    pending.push(name);
                    if !current.pop() {
                        return Err(denied("private directory ancestor is missing"));
                    }
                }
                Err(error) => return Err(PlatformError::io("inspect private directory", error)),
            }
        }
    }

    fn parent_and_name(path: &Path) -> Result<(PathBuf, &OsStr), PlatformError> {
        if !path.is_absolute() {
            return Err(denied("private path must be absolute"));
        }
        let name = path
            .file_name()
            .filter(|name| valid_component(name))
            .ok_or_else(|| denied("private path component is invalid"))?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| denied("private path has no parent"))?;
        Ok((parent.to_path_buf(), name))
    }

    fn valid_component(name: &OsStr) -> bool {
        let bytes = name.as_bytes();
        !bytes.is_empty() && !bytes.contains(&0) && bytes != b"." && bytes != b".."
    }

    fn open_private_parent(path: &Path) -> Result<File, PlatformError> {
        let file = open_directory(path)
            .map_err(|error| PlatformError::io("open private parent", error))?;
        let meta = file
            .metadata()
            .map_err(|error| PlatformError::io("inspect private parent", error))?;
        if !meta.is_dir() || meta.uid() != current_euid() || meta.mode() & 0o777 != 0o700 {
            return Err(denied("private parent is not an owner-only directory"));
        }
        Ok(file)
    }

    fn open_directory(path: &Path) -> io::Result<File> {
        let c_path = cstring(path.as_os_str())?;
        // SAFETY: c_path is NUL-terminated. O_NOFOLLOW refuses a final symlink.
        // The fd is uniquely owned on success and wrapped in File.
        let fd = unsafe {
            libc::open(
                c_path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        owned_fd(fd)
    }

    fn open_at(
        dir: &File,
        name: &OsStr,
        flags: libc::c_int,
        mode: libc::mode_t,
    ) -> io::Result<File> {
        let c_name = cstring(name)?;
        // SAFETY: dir is an open directory fd. c_name is a single component, so
        // openat cannot walk out of that directory. The new fd is owned.
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                c_name.as_ptr(),
                flags,
                libc::c_uint::from(mode),
            )
        };
        owned_fd(fd)
    }

    fn mkdir_at(dir: &File, name: &OsStr) -> io::Result<()> {
        let c_name = cstring(name)?;
        // SAFETY: name is one component. mkdirat does not follow an existing
        // final symlink; it fails with EEXIST instead.
        let rc = unsafe { libc::mkdirat(dir.as_raw_fd(), c_name.as_ptr(), 0o700) };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn unlink_at(dir: &File, name: &OsStr, directory: bool) -> io::Result<()> {
        let c_name = cstring(name)?;
        // SAFETY: name is one component relative to the owned directory fd.
        let rc = unsafe {
            if directory {
                libc::unlinkat(dir.as_raw_fd(), c_name.as_ptr(), libc::AT_REMOVEDIR)
            } else {
                libc::unlinkat(dir.as_raw_fd(), c_name.as_ptr(), 0)
            }
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn owned_fd(fd: libc::c_int) -> io::Result<File> {
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd is a newly created descriptor not owned by any other File.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    fn publish_directory(file: &File) -> Result<(), PlatformError> {
        fchmod(file, 0o700)?;
        let meta = file
            .metadata()
            .map_err(|error| PlatformError::io("inspect private directory", error))?;
        if !meta.is_dir() || meta.uid() != current_euid() || meta.mode() & 0o777 != 0o700 {
            return Err(denied("private directory is not owner-only"));
        }
        Ok(())
    }

    fn publish_file(file: &File, mode: libc::mode_t) -> Result<(), PlatformError> {
        fchmod(file, mode)?;
        let meta = file
            .metadata()
            .map_err(|error| PlatformError::io("inspect private file", error))?;
        if !meta.is_file()
            || meta.uid() != current_euid()
            || meta.nlink() != 1
            || meta.mode() & 0o777 != u32::from(mode)
        {
            return Err(denied("private file is not owner-only"));
        }
        Ok(())
    }

    fn fchmod(file: &File, mode: libc::mode_t) -> Result<(), PlatformError> {
        // SAFETY: file is an owned descriptor. fchmod changes that inode only,
        // and the mode is a constant owner-only value, not a caller mask.
        let rc = unsafe { libc::fchmod(file.as_raw_fd(), mode) };
        if rc == 0 {
            Ok(())
        } else {
            Err(PlatformError::io(
                "publish private permissions",
                io::Error::last_os_error(),
            ))
        }
    }

    fn wait_for_lock(file: &File, timeout: Duration) -> Result<(), PlatformError> {
        let deadline = Instant::now() + timeout;
        loop {
            // SAFETY: file is an owned descriptor. LOCK_NB never blocks, so a
            // stuck peer cannot stall this call past the caller's timeout.
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc == 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            let blocked = matches!(error.raw_os_error(), Some(code) if code == libc::EAGAIN || code == libc::EWOULDBLOCK);
            if !blocked {
                return Err(PlatformError::io("lock private file", error));
            }
            if timeout.is_zero() || Instant::now() >= deadline {
                return Err(PlatformError::io(
                    "lock private file",
                    io::Error::new(io::ErrorKind::TimedOut, "private file lock timed out"),
                ));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            std::thread::sleep(Duration::from_millis(10).min(remaining));
        }
    }

    fn cstring(value: &OsStr) -> io::Result<CString> {
        CString::new(value.as_bytes()).map_err(|_| invalid_io())
    }

    fn current_euid() -> u32 {
        // SAFETY: geteuid has no arguments or pointer preconditions.
        unsafe { libc::geteuid() }
    }

    fn denied(detail: &str) -> PlatformError {
        PlatformError::io(
            "private runtime",
            io::Error::new(io::ErrorKind::PermissionDenied, detail),
        )
    }

    fn invalid_io() -> io::Error {
        io::Error::new(io::ErrorKind::InvalidInput, "invalid private path")
    }
}
