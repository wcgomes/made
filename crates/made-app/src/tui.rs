//! Terminal launch targets for workspace tabs.
//!
//! A workspace tab normally starts the platform-resolved shell. The `+` menu
//! can instead start a compatible TUI directly as the PTY child, without a
//! shell in between.

use std::ffi::OsStr;
use std::path::PathBuf;

use made_platform::{find_executable_on_path, CompatibleTui};
use made_terminal::PtyOptions;

/// Which child process a workspace terminal starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalTarget {
    /// The platform-resolved user shell.
    Shell,
    /// A detected TUI started directly as the PTY child.
    Tui(CompatibleTui),
}

impl TerminalTarget {
    /// Build the PTY options for this target in `cwd`.
    ///
    /// [`TerminalTarget::Shell`] leaves shell resolution to the platform.
    /// [`TerminalTarget::Tui`] resolves the program against `path` with the
    /// same lookup as detection, then spawns that absolute path with no shell
    /// arguments. It returns `None` when the program is not found there, so
    /// nothing is spawned.
    pub fn pty_options(self, cwd: impl Into<PathBuf>, path: &OsStr) -> Option<PtyOptions> {
        let options = PtyOptions::new(cwd);
        match self {
            Self::Shell => Some(options),
            Self::Tui(tui) => {
                let program = find_executable_on_path(OsStr::new(tui.program()), path)?;
                Some(options.with_shell(program.into_os_string()))
            }
        }
    }

    /// Program name in the exit overlay and restart notice.
    pub fn program_label(self) -> &'static str {
        match self {
            Self::Shell => "Shell",
            Self::Tui(tui) => tui.label(),
        }
    }

    /// Program name for diagnostic log lines: `shell` or the TUI label.
    pub fn log_label(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Tui(tui) => tui.label(),
        }
    }

    /// Restart button text in the exit overlay.
    pub fn restart_label(self) -> String {
        match self {
            Self::Shell => "Restart shell".to_owned(),
            Self::Tui(tui) => format!("Restart {}", tui.label()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

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
        let executable = root.path().join("opencode");
        std::fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths([root.path()]).unwrap();

        let options = TerminalTarget::Tui(CompatibleTui::OpenCode)
            .pty_options("/tmp", &path)
            .expect("the detected executable must resolve");
        assert_eq!(options.shell.as_deref(), Some(executable.as_os_str()));
        assert!(options.args.is_empty());
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
