//! Launch argument parsing for the native shell.
//!
//! Supported forms:
//!
//! - `made` (opens the current user's home directory)
//! - `made [workspace]`
//! - `made --workspace <path>`
//! - `made --picker`
//! - `made --smoke-test [<milliseconds>]` (also `MADE_SMOKE_TEST_MS`)
//!
//! An explicit workspace (`--workspace` or positional) takes precedence over
//! `--picker`; with neither, startup resolves the platform home. Parsing never
//! touches the filesystem; workspace validation and home resolution happen in
//! [`crate::workspace`].
//!
//! The hidden smoke harness deliberately treats a folder-less run as the
//! picker surface (see `AppModel::new`) so automated checks never start a real
//! shell in the user's home directory.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use crate::workspace::WorkspaceSelection;

/// Default duration of the headless smoke run.
pub const DEFAULT_SMOKE_DURATION: Duration = Duration::from_millis(2_000);

/// Environment variable that enables the smoke run outside the CLI flag.
pub const SMOKE_ENV: &str = "MADE_SMOKE_TEST_MS";

/// Parsed launch arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchArgs {
    /// Explicit workspace or the picker.
    pub workspace: WorkspaceSelection,
    /// When set, create the real window, pump frames for this long, then exit.
    pub smoke_test: Option<Duration>,
}

/// What the process should do after parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliAction {
    /// Run the native application.
    Run(LaunchArgs),
    /// Print usage and exit successfully.
    Help,
    /// Print the version and exit successfully.
    Version,
}

/// Argument parsing failure. The process exits with status 2 and prints usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    message: String,
}

impl CliError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Human-readable failure.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

/// Parse process arguments, skipping the executable name.
///
/// The `MADE_SMOKE_TEST_MS` environment override is applied here, not in
/// [`parse_args`], so argument parsing stays deterministic in tests.
pub fn parse_process_args() -> Result<CliAction, CliError> {
    let action = parse_args(std::env::args_os().skip(1))?;
    let CliAction::Run(mut args) = action else {
        return Ok(action);
    };
    if args.smoke_test.is_none() {
        if let Some(value) = std::env::var_os(SMOKE_ENV) {
            if let Ok(milliseconds) = value.to_string_lossy().parse::<u64>() {
                args.smoke_test = Some(Duration::from_millis(milliseconds.max(50)));
            } else if !value.is_empty() {
                return Err(CliError::new(format!(
                    "{SMOKE_ENV} must be an integer number of milliseconds"
                )));
            }
        }
    }
    Ok(CliAction::Run(args))
}

/// Parse an argument iterator that does **not** include the executable name.
pub fn parse_args<I>(args: I) -> Result<CliAction, CliError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut workspace: Option<PathBuf> = None;
    let mut picker = false;
    let mut smoke = false;
    let mut smoke_duration = DEFAULT_SMOKE_DURATION;
    let mut args = args.into_iter().peekable();
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        match text.as_ref() {
            "--help" | "-h" => return Ok(CliAction::Help),
            "--version" | "-V" => return Ok(CliAction::Version),
            "--picker" => picker = true,
            "--smoke-test" => {
                smoke = true;
                // Accept an optional numeric duration. A following token that
                // is not a number remains available as the positional
                // workspace.
                if let Some(next) = args.peek() {
                    let next_text = next.to_string_lossy();
                    if let Ok(milliseconds) = next_text.parse::<u64>() {
                        smoke_duration = Duration::from_millis(milliseconds.max(50));
                        args.next();
                    }
                }
            }
            "--workspace" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::new("--workspace requires a folder path"))?;
                set_workspace(&mut workspace, value)?;
            }
            _ if text.starts_with("--workspace=") => {
                let value = text
                    .strip_prefix("--workspace=")
                    .unwrap_or_default()
                    .to_owned();
                if value.is_empty() {
                    return Err(CliError::new("--workspace requires a folder path"));
                }
                set_workspace(&mut workspace, OsString::from(value))?;
            }
            _ if text.starts_with("--smoke-test=") => {
                smoke = true;
                let value = text.strip_prefix("--smoke-test=").unwrap_or_default();
                let milliseconds = value.parse::<u64>().map_err(|_| {
                    CliError::new("--smoke-test expects a duration in milliseconds")
                })?;
                smoke_duration = Duration::from_millis(milliseconds.max(50));
            }
            _ if text.starts_with('-') && text != "-" => {
                return Err(CliError::new(format!("unrecognized option: {text}")));
            }
            _ => set_workspace(&mut workspace, arg)?,
        }
    }

    let workspace = match workspace {
        Some(path) => WorkspaceSelection::Explicit(path),
        None if picker => WorkspaceSelection::Picker,
        None => WorkspaceSelection::None,
    };
    Ok(CliAction::Run(LaunchArgs {
        workspace,
        smoke_test: smoke.then_some(smoke_duration),
    }))
}

/// Usage text printed for `--help`, parse errors, and `--version`.
pub fn usage() -> &'static str {
    "Usage: made [workspace]\n       made --workspace <folder> [--smoke-test [ms]]\n       made --picker [--smoke-test [ms]]\n\n\
     With no workspace, Made opens the current user's home folder.\n\
     --picker opens the in-app folder picker instead; an explicit workspace\n\
     (positional or --workspace) takes precedence over --picker.\n\
     --smoke-test creates the real window, pumps frames for ~2 seconds, then exits."
}

fn set_workspace(slot: &mut Option<PathBuf>, value: OsString) -> Result<(), CliError> {
    if slot.is_some() {
        return Err(CliError::new(
            "only one workspace folder may be supplied at launch",
        ));
    }
    if value.is_empty() {
        return Ok(());
    }
    *slot = Some(PathBuf::from(value));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn positional_and_flag_forms_are_equivalent() {
        let positional = parse_args(args(&["/tmp/ws"])).unwrap();
        let flag = parse_args(args(&["--workspace", "/tmp/ws"])).unwrap();
        let equals = parse_args(args(&["--workspace=/tmp/ws"])).unwrap();
        let expected = CliAction::Run(LaunchArgs {
            workspace: WorkspaceSelection::Explicit(PathBuf::from("/tmp/ws")),
            smoke_test: None,
        });
        assert_eq!(positional, expected);
        assert_eq!(flag, expected);
        assert_eq!(equals, expected);
    }

    #[test]
    fn no_workspace_requests_the_default_home() {
        let action = parse_args(args(&[])).unwrap();
        assert_eq!(
            action,
            CliAction::Run(LaunchArgs {
                workspace: WorkspaceSelection::None,
                smoke_test: None,
            })
        );
    }

    #[test]
    fn picker_flag_selects_the_picker() {
        let action = parse_args(args(&["--picker"])).unwrap();
        assert_eq!(
            action,
            CliAction::Run(LaunchArgs {
                workspace: WorkspaceSelection::Picker,
                smoke_test: None,
            })
        );
    }

    #[test]
    fn explicit_workspace_overrides_picker() {
        let positional = parse_args(args(&["--picker", "/tmp/ws"])).unwrap();
        let flag = parse_args(args(&["--picker", "--workspace", "/tmp/ws"])).unwrap();
        let before = parse_args(args(&["/tmp/ws", "--picker"])).unwrap();
        let expected = CliAction::Run(LaunchArgs {
            workspace: WorkspaceSelection::Explicit(PathBuf::from("/tmp/ws")),
            smoke_test: None,
        });
        assert_eq!(positional, expected);
        assert_eq!(flag, expected);
        assert_eq!(before, expected);
    }

    #[test]
    fn duplicate_workspace_folders_are_rejected() {
        let error = parse_args(args(&["/tmp/one", "--workspace", "/tmp/two"])).unwrap_err();
        assert!(error.message().contains("only one workspace"));
    }

    #[test]
    fn usage_documents_home_and_picker_startup() {
        let usage = usage();
        assert!(usage.contains("home folder"));
        assert!(usage.contains("--picker"));
        assert!(usage.contains("takes precedence"));
    }

    #[test]
    fn smoke_flag_accepts_an_optional_duration() {
        let action = parse_args(args(&["--smoke-test", "750", "/tmp/ws"])).unwrap();
        match action {
            CliAction::Run(args) => {
                assert_eq!(args.smoke_test, Some(Duration::from_millis(750)));
                assert_eq!(
                    args.workspace,
                    WorkspaceSelection::Explicit(PathBuf::from("/tmp/ws"))
                );
            }
            other => panic!("unexpected action {other:?}"),
        }
    }

    #[test]
    fn smoke_without_a_folder_still_parses_as_a_home_request() {
        // AppModel::new turns this test-only combination into the picker so CI
        // never starts a real shell in the user's home. Parsing keeps the
        // workspace request honest.
        let action = parse_args(args(&["--smoke-test", "100"])).unwrap();
        match action {
            CliAction::Run(args) => {
                assert_eq!(args.workspace, WorkspaceSelection::None);
                assert_eq!(args.smoke_test, Some(Duration::from_millis(100)));
            }
            other => panic!("unexpected action {other:?}"),
        }
    }

    #[test]
    fn unknown_options_fail_loudly() {
        let error = parse_args(args(&["--nope"])).unwrap_err();
        assert!(error.message().contains("--nope"));
    }

    #[test]
    fn help_and_version_short_circuit_run_arguments() {
        assert_eq!(parse_args(args(&["--help"])).unwrap(), CliAction::Help);
        assert_eq!(parse_args(args(&["-V"])).unwrap(), CliAction::Version);
        assert_eq!(
            parse_args(args(&["--help", "/tmp/ws"])).unwrap(),
            CliAction::Help
        );
    }
}
