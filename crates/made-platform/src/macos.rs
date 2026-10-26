//! macOS platform backend.
//!
//! Uses a POSIX PTY through `portable-pty`, standard macOS Library paths, the
//! `arboard` clipboard, and the shared IME normalizer. Builds for arm64 and x64
//! from the same source.

use std::ffi::{OsStr, OsString};

use crate::{
    host_environment, resolve_shell_for, spawn_portable_pty, AppPaths, Clipboard, Environment,
    PathsInputs, Platform, PlatformError, PlatformFamily, PtyRequest, PtySession, ShellSpec,
    SystemClipboard,
};

/// macOS backend implementing [`Platform`].
#[derive(Debug, Clone, Copy, Default)]
pub struct MacOsPlatform;

impl MacOsPlatform {
    pub fn new() -> Self {
        Self
    }
}

impl Platform for MacOsPlatform {
    fn family(&self) -> PlatformFamily {
        PlatformFamily::MacOs
    }

    fn resolve_shell(&self, requested: Option<&OsStr>) -> Result<ShellSpec, PlatformError> {
        resolve_shell_for(PlatformFamily::MacOs, requested)
    }

    fn environment(&self, overrides: &[(OsString, OsString)], removed: &[OsString]) -> Environment {
        host_environment(overrides, removed)
    }

    fn app_paths(&self) -> Result<AppPaths, PlatformError> {
        AppPaths::resolve(&PathsInputs::from_env(PlatformFamily::MacOs))
    }

    fn clipboard(&self) -> Result<Box<dyn Clipboard>, PlatformError> {
        Ok(Box::new(SystemClipboard::new()?))
    }

    fn spawn_pty(&self, request: &PtyRequest) -> Result<Box<dyn PtySession>, PlatformError> {
        spawn_portable_pty(request)
    }
}
