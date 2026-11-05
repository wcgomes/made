//! Terminal launch targets for workspace tabs.
//!
//! A workspace tab normally starts the platform-resolved shell. The `+` menu
//! can instead start a compatible TUI directly as the PTY child, without a
//! shell in between.

use std::ffi::OsStr;
use std::path::PathBuf;

use made_platform::{find_executable_on_path, CompatibleTui};
use made_remote::protocol::AuthorityId;
use made_terminal::PtyOptions;

/// Which child process a workspace terminal starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalTarget {
    /// The platform-resolved user shell.
    Shell,
    /// A detected TUI started directly as the PTY child.
    Tui(CompatibleTui),
    /// A remote shell on a saved server. Never resolved against the host PATH.
    RemoteShell { authority: AuthorityId },
    /// Remote OpenCode on a saved server. Never substituted with a host binary.
    RemoteOpenCode { authority: AuthorityId },
    /// Remote Grok on a saved server, started directly as `grok dashboard`.
    /// Never substituted with a host binary and never given the OpenCode
    /// plugin environment.
    RemoteGrok { authority: AuthorityId },
}

impl TerminalTarget {
    /// Build the PTY options for this target in `cwd`.
    ///
    /// [`TerminalTarget::Shell`] leaves shell resolution to the platform.
    /// [`TerminalTarget::Tui`] resolves the program against `path` with the
    /// same lookup as detection, then spawns that absolute path with no shell
    /// in between. Grok's only argument is `dashboard`. OpenCode gets none.
    /// It returns `None` when the program is not found there, so nothing is
    /// spawned.
    pub fn pty_options(self, cwd: impl Into<PathBuf>, path: &OsStr) -> Option<PtyOptions> {
        let options = PtyOptions::new(cwd);
        match self {
            Self::Shell => Some(options),
            Self::Tui(tui) => {
                let program = find_executable_on_path(OsStr::new(tui.program()), path)?;
                let options = options.with_shell(program.into_os_string());
                Some(match tui {
                    CompatibleTui::Grok => options.with_args(["dashboard"]),
                    CompatibleTui::OpenCode => options,
                })
            }
            // Remote launches must not resolve a host executable or host cwd.
            Self::RemoteShell { .. } | Self::RemoteOpenCode { .. } | Self::RemoteGrok { .. } => {
                None
            }
        }
    }

    /// Whether this target belongs to a saved SSH authority.
    pub fn is_remote(self) -> bool {
        matches!(
            self,
            Self::RemoteShell { .. } | Self::RemoteOpenCode { .. } | Self::RemoteGrok { .. }
        )
    }

    /// Saved server for a remote target.
    pub fn authority(self) -> Option<AuthorityId> {
        match self {
            Self::RemoteShell { authority }
            | Self::RemoteOpenCode { authority }
            | Self::RemoteGrok { authority } => Some(authority),
            Self::Shell | Self::Tui(_) => None,
        }
    }

    /// Program name in the exit overlay and restart notice.
    pub fn program_label(self) -> &'static str {
        match self {
            Self::Shell => "Shell",
            Self::Tui(tui) => tui.label(),
            Self::RemoteShell { .. } => "Terminal SSH",
            Self::RemoteOpenCode { .. } => "OpenCode SSH",
            Self::RemoteGrok { .. } => "Grok SSH",
        }
    }

    /// Program name for diagnostic log lines: `shell` or the TUI label.
    pub fn log_label(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Tui(tui) => tui.label(),
            Self::RemoteShell { .. } => "terminal-ssh",
            Self::RemoteOpenCode { .. } => "opencode-ssh",
            Self::RemoteGrok { .. } => "grok-ssh",
        }
    }

    /// Restart button text in the exit overlay.
    pub fn restart_label(self) -> String {
        match self {
            Self::Shell => "Restart shell".to_owned(),
            Self::Tui(tui) => format!("Restart {}", tui.label()),
            Self::RemoteShell { .. } => "Reconnect Terminal SSH".to_owned(),
            Self::RemoteOpenCode { .. } => "Reconnect OpenCode SSH".to_owned(),
            Self::RemoteGrok { .. } => "Reconnect Grok SSH".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn remote_targets_do_not_resolve_a_host_executable_or_cwd() {
        let authority = AuthorityId([7; 16]);
        let shell = TerminalTarget::RemoteShell { authority };
        let opencode = TerminalTarget::RemoteOpenCode { authority };
        let grok = TerminalTarget::RemoteGrok { authority };
        assert!(shell.pty_options("/tmp", OsStr::new("/usr/bin")).is_none());
        assert!(opencode
            .pty_options("/tmp", OsStr::new("/usr/bin"))
            .is_none());
        assert!(
            grok.pty_options("/tmp", OsStr::new("/usr/bin")).is_none(),
            "a remote Grok launch must never resolve a host executable"
        );
        assert_eq!(shell.program_label(), "Terminal SSH");
        assert_eq!(opencode.program_label(), "OpenCode SSH");
        assert_eq!(grok.program_label(), "Grok SSH");
        assert!(shell.is_remote());
        assert!(grok.is_remote());
        assert_eq!(opencode.authority(), Some(authority));
        assert_eq!(grok.authority(), Some(authority));
        assert!(!TerminalTarget::Shell.is_remote());
        assert_eq!(grok.restart_label(), "Reconnect Grok SSH");
        assert_eq!(grok.log_label(), "grok-ssh");
    }

    #[test]
    fn shell_target_keeps_platform_shell_resolution() {
        let options = TerminalTarget::Shell
            .pty_options("/tmp", OsStr::new(""))
            .expect("the shell is always available");
        assert_eq!(options.shell, None);
        assert!(options.args.is_empty());
    }

    #[test]
    fn tui_target_wording_names_the_program() {
        let opencode = TerminalTarget::Tui(CompatibleTui::OpenCode);
        assert_eq!(opencode.program_label(), "OpenCode");
        assert_eq!(opencode.restart_label(), "Restart OpenCode");
        assert_eq!(TerminalTarget::Shell.program_label(), "Shell");
        assert_eq!(TerminalTarget::Shell.restart_label(), "Restart shell");
        assert_eq!(opencode.log_label(), "OpenCode");
        assert_eq!(TerminalTarget::Shell.log_label(), "shell");
    }

    #[test]
    fn tui_target_is_none_when_the_program_is_not_on_path() {
        let root = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([root.path()]).unwrap();
        let target = TerminalTarget::Tui(CompatibleTui::OpenCode);
        assert!(target.pty_options("/tmp", &path).is_none());
        assert!(target.pty_options("/tmp", OsStr::new("")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn tui_target_names_the_detected_absolute_path() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        for name in ["opencode", "grok"] {
            let executable = root.path().join(name);
            std::fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = std::env::join_paths([root.path()]).unwrap();

        let opencode = TerminalTarget::Tui(CompatibleTui::OpenCode)
            .pty_options("/tmp", &path)
            .expect("the detected executable must resolve");
        assert_eq!(
            opencode.shell.as_deref(),
            Some(root.path().join("opencode").as_os_str())
        );
        assert!(opencode.args.is_empty());

        let grok = TerminalTarget::Tui(CompatibleTui::Grok)
            .pty_options("/tmp", &path)
            .expect("the detected executable must resolve");
        assert_eq!(
            grok.shell.as_deref(),
            Some(root.path().join("grok").as_os_str())
        );
        // Restart clones these options, so the same argument is the relaunch.
        assert_eq!(grok.args, vec![std::ffi::OsString::from("dashboard")]);
    }

    #[cfg(unix)]
    #[test]
    fn tui_target_ignores_empty_and_relative_path_segments() {
        use std::os::unix::fs::PermissionsExt;
        use std::path::Path;

        // The executable is reachable only through a relative PATH entry, so a
        // lookup that resolved relative segments would find it. `tempdir_in`
        // makes its path absolute, so the relative directory is created by hand.
        let relative = format!(".made-tui-path-{}", std::process::id());
        let directory = Path::new(&relative);
        std::fs::create_dir_all(directory).unwrap();
        let executable = directory.join("opencode");
        std::fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths([Path::new(""), directory]).unwrap();

        let detected = TerminalTarget::Tui(CompatibleTui::OpenCode).pty_options("/tmp", &path);
        let _ = std::fs::remove_dir_all(directory);
        assert!(
            detected.is_none(),
            "empty and relative PATH segments must be ignored"
        );
    }
}
