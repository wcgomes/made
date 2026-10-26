//! Windows platform backend.
//!
//! `portable-pty` selects ConPTY on Windows, so the backend inherits real
//! console semantics (VT input/output, resize, and process cleanup) without
//! embedding a second terminal implementation. The supported OpenCode
//! distribution is the native Windows build, not WSL.
//!
//! Process cleanup: Windows has no POSIX process-group signal, so each ConPTY
//! child is assigned to an unnamed Job Object configured with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. [`crate::PtySession::kill`] terminates
//! that Job Object, which covers descendants created after assignment and keeps
//! cleanup scoped to the owning workspace tab. Known limits: assignment happens
//! after `portable-pty` spawns the child, so a descendant created in that short
//! window can escape; a process that explicitly breaks away from the Job Object
//! is outside the guarantee; and environments that prevent nested job
//! assignment fall back to the immediate-child terminator.
//!
//! Delivery gate: `pty_terminate_cleans_job_descendants_after_shell_exit` in
//! `tests/contract.rs` must pass on a native Windows runner. Cross-compiling
//! this module does not certify runtime cleanup; until that test is green on
//! Windows, descendant cleanup remains pending evidence rather than a pass.
//!
//! Private-log residual: [`crate::open_private_log`] checks
//! `symlink_metadata` and then opens with the standard Rust file API. Windows
//! can replace the path with a reparse point between that check and the open;
//! the helper does not use `FILE_FLAG_OPEN_REPARSE_POINT`, so this is a
//! documented non-atomic residual. Keep log files inside the app-owned,
//! owner-only directory and do not expose log paths to untrusted writers.

use std::ffi::{c_void, OsStr, OsString};
use std::mem::size_of;
use std::path::PathBuf;
use std::ptr::null;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

use crate::{
    host_environment, resolve_shell_for, spawn_portable_pty, AppPaths, Clipboard, Environment,
    PathsInputs, Platform, PlatformError, PlatformFamily, PtyRequest, PtySession, ShellSpec,
    SystemClipboard,
};

/// Windows backend implementing [`Platform`].
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsPlatform;

impl WindowsPlatform {
    pub fn new() -> Self {
        Self
    }
}

impl Platform for WindowsPlatform {
    fn family(&self) -> PlatformFamily {
        PlatformFamily::Windows
    }

    fn resolve_shell(&self, requested: Option<&OsStr>) -> Result<ShellSpec, PlatformError> {
        resolve_shell_for(PlatformFamily::Windows, requested)
    }

    fn environment(&self, overrides: &[(OsString, OsString)], removed: &[OsString]) -> Environment {
        host_environment(overrides, removed)
    }

    fn app_paths(&self) -> Result<AppPaths, PlatformError> {
        AppPaths::resolve(&PathsInputs::from_env(PlatformFamily::Windows))
    }

    fn clipboard(&self) -> Result<Box<dyn Clipboard>, PlatformError> {
        Ok(Box::new(SystemClipboard::new()?))
    }

    fn spawn_pty(&self, request: &PtyRequest) -> Result<Box<dyn PtySession>, PlatformError> {
        spawn_portable_pty(request)
    }
}

/// Workspace-scoped Job Object owning a ConPTY child and its descendants.
///
/// `KILL_ON_JOB_CLOSE` also makes dropping the session terminate any remaining
/// descendant, even when the direct shell already exited. If creation or
/// assignment fails, the session falls back to the `portable-pty` immediate
/// child terminator rather than failing the shell launch.
pub(crate) struct ProcessJob(HANDLE);

// SAFETY: the handle is uniquely owned by this value. Win32 job and process
// handle operations are thread-safe; no Rust aliasing rules apply to HANDLE.
unsafe impl Send for ProcessJob {}
// SAFETY: the handle is only read for terminate/drop, which are Win32 calls
// that accept the handle from any thread.
unsafe impl Sync for ProcessJob {}

impl ProcessJob {
    /// Create a kill-on-close job and assign `pid` to it.
    pub(crate) fn assign(pid: u32) -> Option<Self> {
        // SAFETY: null security attributes and name request an unnamed job.
        let handle = unsafe { CreateJobObjectW(null(), null()) };
        if handle.is_null() {
            return None;
        }
        let job = Self(handle);

        // SAFETY: the info block is fully initialized by Default and the
        // length matches its concrete type.
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `job.0` is a live job handle; `limits` is valid for the
        // documented JobObjectExtendedLimitInformation size.
        let configured = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            return None;
        }

        // SAFETY: OpenProcess only reads the process id we just spawned and
        // returns a new owned handle.
        let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };
        if process.is_null() {
            return None;
        }
        // SAFETY: both handles are valid and owned by this scope.
        let assigned = unsafe { AssignProcessToJobObject(job.0, process) };
        // SAFETY: the process handle was created by OpenProcess above and is
        // not used after this close.
        unsafe {
            CloseHandle(process);
        }
        if assigned == 0 {
            return None;
        }
        Some(job)
    }

    /// Terminate every process currently assigned to the job.
    pub(crate) fn terminate(&self) -> bool {
        // SAFETY: `self.0` is a live job handle owned by this value.
        unsafe { TerminateJobObject(self.0, 1) != 0 }
    }
}

impl Drop for ProcessJob {
    fn drop(&mut self) {
        // KILL_ON_JOB_CLOSE terminates any descendants still assigned after the
        // direct shell exits.
        // SAFETY: `self.0` is owned and not used after this call.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// The default Windows log directory without consulting the process environment.
pub fn default_log_dir(local_app_data: &std::path::Path) -> PathBuf {
    local_app_data.join("Made").join("logs")
}
