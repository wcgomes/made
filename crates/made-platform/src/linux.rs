//! Linux platform backend.
//!
//! Uses a POSIX PTY through `portable-pty`, XDG app-owned paths, the `arboard`
//! clipboard, and the shared IME normalizer. X11 and Wayland are both supported
//! display-server targets; no per-OS UI behavior lives here.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::{
    host_environment, resolve_shell_for, spawn_portable_pty, AppPaths, Clipboard, Environment,
    PathsInputs, Platform, PlatformError, PlatformFamily, PtyRequest, PtySession, ShellSpec,
    SystemClipboard,
};

/// Linux backend implementing [`Platform`].
#[derive(Debug, Clone, Copy, Default)]
pub struct LinuxPlatform;

impl LinuxPlatform {
    pub fn new() -> Self {
        Self
    }
}

impl Platform for LinuxPlatform {
    fn family(&self) -> PlatformFamily {
        PlatformFamily::Linux
    }

    fn resolve_shell(&self, requested: Option<&OsStr>) -> Result<ShellSpec, PlatformError> {
        resolve_shell_for(PlatformFamily::Linux, requested)
    }

    fn environment(&self, overrides: &[(OsString, OsString)], removed: &[OsString]) -> Environment {
        host_environment(overrides, removed)
    }

    fn app_paths(&self) -> Result<AppPaths, PlatformError> {
        AppPaths::resolve(&PathsInputs::from_env(PlatformFamily::Linux))
    }

    fn clipboard(&self) -> Result<Box<dyn Clipboard>, PlatformError> {
        Ok(Box::new(SystemClipboard::new()?))
    }

    fn spawn_pty(&self, request: &PtyRequest) -> Result<Box<dyn PtySession>, PlatformError> {
        spawn_portable_pty(request)
    }
}

/// The default Linux log directory without consulting the process environment.
///
/// Exposed for diagnostics that need to display the expected location before
/// [`Platform::app_paths`] succeeds.
pub fn default_log_dir(home: &std::path::Path) -> PathBuf {
    home.join(".local").join("state").join("made").join("log")
}
