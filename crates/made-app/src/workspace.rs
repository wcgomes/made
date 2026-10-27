//! Workspace validation, launch state, and non-blocking error handling.
//!
//! A workspace is the folder Made opens at launch, from the current user's
//! home directory (the default), the launch argument, or the in-app picker.
//! The terminal starts with the canonical folder as its working directory and
//! the explorer is rooted at the same canonical folder.
//!
//! Validation failures never block the terminal. An explicit folder or default
//! home that cannot be used produces a [`WorkspaceError`] and a terminal-only
//! fallback; only an explicit `--picker` (or the in-app Open folder action)
//! shows [`Surfaces::Picker`] before a folder is chosen. Home resolution itself
//! stays behind [`made_platform::Platform::home_dir`]; this module never reads
//! `HOME` or `USERPROFILE`.

use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use made_platform::{HomeDirectoryError, HomeDirectoryErrorKind, Platform};

use crate::workspace_picker::{PickerError, WorkspacePicker};

/// The workspace argument supplied to the process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceSelection {
    /// No workspace was supplied; startup resolves the current user's home.
    None,
    /// The folder picker was requested explicitly with `--picker`.
    Picker,
    /// An explicit workspace path was supplied. It is validated when opened.
    Explicit(PathBuf),
}

impl WorkspaceSelection {
    /// Parse a CLI-equivalent workspace argument.
    ///
    /// An absent or empty argument requests the default home workspace. Every
    /// other value is treated as a path and validated by
    /// [`WorkspaceRoot::resolve`].
    pub fn from_arg(arg: Option<&OsStr>) -> Self {
        match arg {
            Some(value) if !value.is_empty() => Self::Explicit(PathBuf::from(value)),
            _ => Self::None,
        }
    }

    /// The requested path, when one was supplied.
    pub fn requested_path(&self) -> Option<&Path> {
        match self {
            Self::None | Self::Picker => None,
            Self::Explicit(path) => Some(path),
        }
    }
}

/// Where a launch workspace came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSource {
    /// The current user's platform-resolved home directory was used.
    DefaultHome,
    /// The folder was supplied on the command line.
    LaunchArgument,
    /// The folder was chosen in the in-app picker.
    Picker,
}

impl WorkspaceSource {
    /// Stable lowercase name for diagnostics.
    pub const fn label(self) -> &'static str {
        match self {
            Self::DefaultHome => "default-home",
            Self::LaunchArgument => "launch-argument",
            Self::Picker => "picker",
        }
    }
}

/// Classification of a workspace validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceErrorKind {
    /// The requested path does not exist.
    Missing,
    /// The requested path exists but is not a directory.
    NotADirectory,
    /// The directory exists but cannot be read.
    Unreadable,
    /// The path itself is invalid, such as an empty value.
    InvalidPath,
}

impl WorkspaceErrorKind {
    /// Stable machine-readable code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Missing => "workspace_missing",
            Self::NotADirectory => "workspace_not_a_directory",
            Self::Unreadable => "workspace_unreadable",
            Self::InvalidPath => "workspace_invalid_path",
        }
    }

    /// Short, actionable description.
    pub const fn headline(self) -> &'static str {
        match self {
            Self::Missing => "the workspace folder does not exist",
            Self::NotADirectory => "the workspace path is not a folder",
            Self::Unreadable => "the workspace folder is not readable",
            Self::InvalidPath => "the workspace path is invalid",
        }
    }
}

/// A workspace validation failure.
///
/// Workspace errors are deliberately non-blocking: the terminal can keep
/// running in a fallback directory while the error is displayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceError {
    kind: WorkspaceErrorKind,
    requested: PathBuf,
    detail: String,
}

impl WorkspaceError {
    /// Build an error for a requested path.
    pub fn new(
        kind: WorkspaceErrorKind,
        requested: impl Into<PathBuf>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            requested: requested.into(),
            detail: detail.into(),
        }
    }

    /// Classify an I/O error for a workspace path.
    pub fn from_io(requested: impl Into<PathBuf>, error: &io::Error) -> Self {
        let requested = requested.into();
        let kind = match error.kind() {
            io::ErrorKind::NotFound => WorkspaceErrorKind::Missing,
            io::ErrorKind::NotADirectory => WorkspaceErrorKind::NotADirectory,
            io::ErrorKind::PermissionDenied => WorkspaceErrorKind::Unreadable,
            _ => WorkspaceErrorKind::Unreadable,
        };
        Self::new(kind, requested, error.to_string())
    }

    /// Failure classification.
    pub fn kind(&self) -> WorkspaceErrorKind {
        self.kind
    }

    /// Path that was requested, before canonicalization.
    pub fn requested(&self) -> &Path {
        &self.requested
    }

    /// Lower-level detail, suitable for diagnostics.
    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// Whether this failure must prevent the app from being usable.
    ///
    /// Always `false`: the terminal remains available with a fallback working
    /// directory.
    pub const fn is_blocking(&self) -> bool {
        false
    }

    /// Human-readable message for a status area or diagnostics panel.
    pub fn message(&self) -> String {
        let base = format!("{}: {}", self.kind.headline(), self.requested.display());
        if self.detail.is_empty() {
            base
        } else {
            format!("{base} ({})", self.detail)
        }
    }
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl Error for WorkspaceError {}

/// A validated workspace folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRoot {
    path: PathBuf,
    requested: PathBuf,
    display_name: String,
    source: WorkspaceSource,
}

impl WorkspaceRoot {
    /// Validate `path` and capture its canonical form.
    ///
    /// Relative paths are resolved against the process working directory. The
    /// folder must exist, be a directory, and be readable.
    pub fn resolve(
        path: impl AsRef<Path>,
        source: WorkspaceSource,
    ) -> Result<Self, WorkspaceError> {
        let requested = path.as_ref();
        if requested.as_os_str().is_empty() {
            return Err(WorkspaceError::new(
                WorkspaceErrorKind::InvalidPath,
                requested,
                "workspace path is empty",
            ));
        }

        let absolute = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|error| {
                    WorkspaceError::new(
                        WorkspaceErrorKind::InvalidPath,
                        requested,
                        format!("cannot resolve relative workspace path: {error}"),
                    )
                })?
                .join(requested)
        };

        let metadata =
            fs::metadata(&absolute).map_err(|error| WorkspaceError::from_io(requested, &error))?;
        if !metadata.is_dir() {
            return Err(WorkspaceError::new(
                WorkspaceErrorKind::NotADirectory,
                requested,
                "workspace path exists but is a file",
            ));
        }

        let canonical = fs::canonicalize(&absolute)
            .map_err(|error| WorkspaceError::from_io(requested, &error))?;
        let metadata =
            fs::metadata(&canonical).map_err(|error| WorkspaceError::from_io(requested, &error))?;
        if !metadata.is_dir() {
            return Err(WorkspaceError::new(
                WorkspaceErrorKind::NotADirectory,
                requested,
                "canonical workspace path is not a directory",
            ));
        }

        let mut entries =
            fs::read_dir(&canonical).map_err(|error| WorkspaceError::from_io(requested, &error))?;
        if let Some(Err(error)) = entries.next() {
            return Err(WorkspaceError::from_io(requested, &error));
        }

        let display_name = canonical
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| canonical.display().to_string());

        Ok(Self {
            path: canonical,
            requested: requested.to_path_buf(),
            display_name,
            source,
        })
    }

    /// Canonical absolute workspace path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Path exactly as requested, before canonicalization.
    pub fn requested(&self) -> &Path {
        &self.requested
    }

    /// Folder name used in the window chrome and diagnostics.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// How this workspace was selected.
    pub fn source(&self) -> WorkspaceSource {
        self.source
    }
}

/// The state needed to open the workspace surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    /// The validated workspace.
    pub workspace: WorkspaceRoot,
    /// Canonical explorer root. Equal to `workspace.path()`.
    pub explorer_root: PathBuf,
}

impl LaunchPlan {
    /// Validate `path` and build a launch plan.
    pub fn open(path: impl AsRef<Path>, source: WorkspaceSource) -> Result<Self, WorkspaceError> {
        let workspace = WorkspaceRoot::resolve(path, source)?;
        let explorer_root = workspace.path().to_path_buf();
        Ok(Self {
            workspace,
            explorer_root,
        })
    }

    /// Initial shell working directory.
    pub fn terminal_cwd(&self) -> &Path {
        &self.explorer_root
    }
}

/// Which surfaces the launch state says to present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surfaces {
    /// No workspace has been chosen; show only the folder picker.
    Picker,
    /// Show the workspace surfaces. An invalid explicit workspace can keep the
    /// terminal while the explorer is unavailable.
    Workspace {
        /// Whether to create the terminal pane.
        terminal: bool,
        /// Whether to create the explorer pane.
        explorer: bool,
    },
}

impl Surfaces {
    /// Both workspace surfaces.
    pub const fn full() -> Self {
        Self::Workspace {
            terminal: true,
            explorer: true,
        }
    }

    /// Terminal only, used when the workspace folder is invalid.
    pub const fn terminal_only() -> Self {
        Self::Workspace {
            terminal: true,
            explorer: false,
        }
    }

    /// Whether the picker is the only surface.
    pub const fn has_picker(self) -> bool {
        matches!(self, Self::Picker)
    }

    /// Whether the terminal should be created.
    pub const fn has_terminal(self) -> bool {
        matches!(self, Self::Workspace { terminal: true, .. })
    }

    /// Whether the explorer should be created.
    pub const fn has_explorer(self) -> bool {
        matches!(self, Self::Workspace { explorer: true, .. })
    }
}

/// Launch-time workspace state consumed by the app shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceLaunch {
    /// No workspace was supplied. The picker is shown and no terminal or
    /// explorer exists until a folder is confirmed.
    Picker {
        /// Headless picker model.
        picker: WorkspacePicker,
        /// Error from a failed picker confirmation, if any.
        error: Option<WorkspaceError>,
    },
    /// A validated workspace is ready.
    Ready(LaunchPlan),
    /// An explicit workspace or the default home was invalid. The terminal
    /// stays usable in a fallback directory while the explorer is not shown.
    TerminalFallback {
        /// Why the workspace could not be opened.
        error: WorkspaceError,
        /// Working directory for the fallback terminal.
        cwd: PathBuf,
        /// Where the failed launch was requested from.
        source: WorkspaceSource,
    },
}

impl WorkspaceLaunch {
    /// Resolve launch-time state from the CLI-equivalent argument.
    pub fn from_arg(arg: Option<&OsStr>) -> Self {
        Self::from_selection(WorkspaceSelection::from_arg(arg))
    }

    /// Resolve launch-time state from a parsed selection.
    ///
    /// No selection resolves and validates the host platform's user home.
    /// `--picker` waits for an explicit folder choice. Every other selection
    /// opens its path.
    pub fn from_selection(selection: WorkspaceSelection) -> Self {
        match selection {
            WorkspaceSelection::None => {
                Self::from_default_home(made_platform::host_platform().home_dir())
            }
            WorkspaceSelection::Picker => Self::waiting_for_picker(),
            WorkspaceSelection::Explicit(path) => Self::from_explicit(path),
        }
    }

    /// Resolve launch-time state with an explicit home result.
    ///
    /// The `home` value is only consulted for [`WorkspaceSelection::None`], so
    /// tests and embedders can resolve the platform home once and avoid
    /// process-global environment races. Pass the output of
    /// [`made_platform::Platform::home_dir`], or construct the same error with
    /// [`made_platform::resolve_user_home`] and
    /// [`made_platform::validate_user_home`] for hermetic tests.
    pub fn from_selection_with_home(
        selection: WorkspaceSelection,
        home: Result<PathBuf, HomeDirectoryError>,
    ) -> Self {
        match selection {
            WorkspaceSelection::None => Self::from_default_home(home),
            WorkspaceSelection::Picker => Self::waiting_for_picker(),
            WorkspaceSelection::Explicit(path) => Self::from_explicit(path),
        }
    }

    fn from_explicit(path: PathBuf) -> Self {
        match LaunchPlan::open(&path, WorkspaceSource::LaunchArgument) {
            Ok(plan) => Self::Ready(plan),
            Err(error) => Self::TerminalFallback {
                error,
                cwd: fallback_terminal_cwd(),
                source: WorkspaceSource::LaunchArgument,
            },
        }
    }

    fn from_default_home(home: Result<PathBuf, HomeDirectoryError>) -> Self {
        let path = match home {
            Ok(path) => path,
            Err(error) => {
                return Self::TerminalFallback {
                    error: home_directory_error(error),
                    cwd: fallback_terminal_cwd(),
                    source: WorkspaceSource::DefaultHome,
                }
            }
        };
        match LaunchPlan::open(&path, WorkspaceSource::DefaultHome) {
            Ok(plan) => Self::Ready(plan),
            Err(error) => Self::TerminalFallback {
                error,
                cwd: fallback_terminal_cwd(),
                source: WorkspaceSource::DefaultHome,
            },
        }
    }

    /// Show the picker at the first usable start directory.
    pub fn waiting_for_picker() -> Self {
        let mut last_error = None;
        for candidate in picker_start_candidates() {
            match WorkspacePicker::new(&candidate) {
                Ok(picker) => {
                    return Self::Picker {
                        picker,
                        error: None,
                    }
                }
                Err(error) => last_error = Some(error),
            }
        }

        let error = last_error.unwrap_or_else(|| {
            WorkspaceError::new(
                WorkspaceErrorKind::Unreadable,
                fallback_terminal_cwd(),
                "no readable directory is available for the folder picker",
            )
        });
        Self::TerminalFallback {
            error,
            cwd: fallback_terminal_cwd(),
            source: WorkspaceSource::Picker,
        }
    }

    /// Build a ready launch state from an already validated plan.
    pub fn from_plan(plan: LaunchPlan) -> Self {
        Self::Ready(plan)
    }

    /// Where the current launch surface came from.
    ///
    /// This stays meaningful for a terminal fallback, which has no validated
    /// [`WorkspaceRoot`] to carry the source.
    pub fn source(&self) -> WorkspaceSource {
        match self {
            Self::Picker { .. } => WorkspaceSource::Picker,
            Self::Ready(plan) => plan.workspace.source(),
            Self::TerminalFallback { source, .. } => *source,
        }
    }

    /// Which surfaces to present.
    pub fn surfaces(&self) -> Surfaces {
        match self {
            Self::Picker { .. } => Surfaces::Picker,
            Self::Ready(_) => Surfaces::full(),
            Self::TerminalFallback { .. } => Surfaces::terminal_only(),
        }
    }

    /// The validated launch plan, when one exists.
    pub fn plan(&self) -> Option<&LaunchPlan> {
        match self {
            Self::Ready(plan) => Some(plan),
            _ => None,
        }
    }

    /// The picker model, when the picker is visible.
    pub fn picker(&self) -> Option<&WorkspacePicker> {
        match self {
            Self::Picker { picker, .. } => Some(picker),
            _ => None,
        }
    }

    /// Mutable picker model, when the picker is visible.
    pub fn picker_mut(&mut self) -> Option<&mut WorkspacePicker> {
        match self {
            Self::Picker { picker, .. } => Some(picker),
            _ => None,
        }
    }

    /// Terminal working directory, if a terminal should exist.
    pub fn terminal_cwd(&self) -> Option<&Path> {
        match self {
            Self::Picker { .. } => None,
            Self::Ready(plan) => Some(plan.terminal_cwd()),
            Self::TerminalFallback { cwd, .. } => Some(cwd),
        }
    }

    /// Explorer root, when the explorer should exist.
    pub fn explorer_root(&self) -> Option<&Path> {
        match self {
            Self::Ready(plan) => Some(plan.explorer_root.as_path()),
            _ => None,
        }
    }

    /// Visible workspace error, if any.
    pub fn error(&self) -> Option<&WorkspaceError> {
        match self {
            Self::Picker { error, .. } => error.as_ref(),
            Self::TerminalFallback { error, .. } => Some(error),
            Self::Ready(_) => None,
        }
    }

    /// Replace the state with a validated plan, for example after the picker
    /// confirms a folder.
    pub fn open_plan(&mut self, plan: LaunchPlan) {
        *self = Self::Ready(plan);
    }

    /// Confirm the picker's selected row path (or root) and open both surfaces.
    ///
    /// On failure the picker stays visible, the error is stored, and the
    /// terminal is not created because no workspace has been confirmed.
    pub fn confirm_picker(&mut self) -> Result<(), WorkspaceError> {
        let plan = match self {
            Self::Picker { picker, .. } => match picker.confirm_current() {
                Ok(plan) => plan,
                Err(error) => {
                    if let Self::Picker {
                        error: stored_error,
                        ..
                    } = self
                    {
                        *stored_error = Some(error.clone());
                    }
                    return Err(error);
                }
            },
            _ => {
                return Err(WorkspaceError::new(
                    WorkspaceErrorKind::InvalidPath,
                    PathBuf::new(),
                    "workspace launch is not waiting for a folder choice",
                ))
            }
        };

        *self = Self::Ready(plan);
        Ok(())
    }

    /// Toggle expansion of one visible picker row.
    pub fn picker_toggle(&mut self, index: usize) -> Result<(), PickerError> {
        let result = match self {
            Self::Picker { picker, .. } => picker.toggle(index),
            _ => return Ok(()),
        };
        if let Err(error) = &result {
            if let Self::Picker { picker, .. } = self {
                picker.set_error(error.clone());
            }
        } else if let Self::Picker { error, .. } = self {
            *error = None;
        }
        result
    }

    /// Toggle expansion of the selected picker row.
    pub fn picker_toggle_selected(&mut self) -> Result<(), PickerError> {
        let result = match self {
            Self::Picker { picker, .. } => picker.toggle_selected(),
            _ => return Ok(()),
        };
        if let Err(error) = &result {
            if let Self::Picker { picker, .. } = self {
                picker.set_error(error.clone());
            }
        } else if let Self::Picker { error, .. } = self {
            *error = None;
        }
        result
    }

    /// Expand the selected picker folder, or select its first child.
    pub fn picker_expand_selected(&mut self) -> Result<(), PickerError> {
        let result = match self {
            Self::Picker { picker, .. } => picker.expand_selected(),
            _ => return Ok(()),
        };
        if let Err(error) = &result {
            if let Self::Picker { picker, .. } = self {
                picker.set_error(error.clone());
            }
        } else if let Self::Picker { error, .. } = self {
            *error = None;
        }
        result
    }

    /// Collapse the selected picker folder, or select its parent row.
    pub fn picker_collapse_selected(&mut self) -> Result<(), PickerError> {
        let result = match self {
            Self::Picker { picker, .. } => picker.collapse_selected(),
            _ => return Ok(()),
        };
        if let Err(error) = &result {
            if let Self::Picker { picker, .. } = self {
                picker.set_error(error.clone());
            }
        } else if let Self::Picker { error, .. } = self {
            *error = None;
        }
        result
    }

    /// Navigate the picker to the parent directory.
    pub fn picker_up(&mut self) -> Result<(), PickerError> {
        let result = match self {
            Self::Picker { picker, .. } => picker.go_up(),
            _ => return Ok(()),
        };
        if let Err(error) = &result {
            if let Self::Picker { picker, .. } = self {
                picker.set_error(error.clone());
            }
        } else if let Self::Picker { error, .. } = self {
            *error = None;
        }
        result
    }

    /// Move the picker selection by `delta` entries, wrapping at the ends.
    pub fn picker_move(&mut self, delta: isize) {
        if let Self::Picker { picker, error } = self {
            picker.move_selection(delta);
            *error = None;
        }
    }

    /// Select one visible picker row by index.
    pub fn picker_select(&mut self, index: usize) -> Result<(), PickerError> {
        let result = match self {
            Self::Picker { picker, .. } => picker.select(index),
            _ => return Ok(()),
        };
        if let Err(error) = &result {
            if let Self::Picker { picker, .. } = self {
                picker.set_error(error.clone());
            }
        } else if let Self::Picker { error, .. } = self {
            *error = None;
        }
        result
    }

    /// Refresh the picker's visible directories.
    pub fn picker_refresh(&mut self) -> Result<(), WorkspaceError> {
        match self {
            Self::Picker { picker, error } => {
                let result = picker.refresh();
                if result.is_ok() {
                    *error = None;
                }
                result
            }
            _ => Ok(()),
        }
    }
}

/// Directory candidates for the folder picker.
fn picker_start_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    if let Ok(home) = made_platform::host_platform().home_dir() {
        candidates.push(home);
    }
    candidates.push(std::env::temp_dir());
    candidates
}

/// Map a platform home failure into the visible, non-blocking workspace state.
fn home_directory_error(error: HomeDirectoryError) -> WorkspaceError {
    let kind = match error.kind() {
        HomeDirectoryErrorKind::Unresolved => WorkspaceErrorKind::InvalidPath,
        HomeDirectoryErrorKind::Missing => WorkspaceErrorKind::Missing,
        HomeDirectoryErrorKind::NotADirectory => WorkspaceErrorKind::NotADirectory,
        HomeDirectoryErrorKind::Unreadable => WorkspaceErrorKind::Unreadable,
    };
    let requested = error.path().unwrap_or_else(|| Path::new("~"));
    WorkspaceError::new(kind, requested, error.detail())
}

/// Fallback terminal directory when the workspace is invalid.
///
/// Preference: the process working directory, then the platform-resolved home,
/// then the system temp directory. All are ordinary existing directories for a
/// running process; a fallback failure must not block the terminal.
pub fn fallback_terminal_cwd() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        return cwd;
    }
    if let Ok(home) = made_platform::host_platform().home_dir() {
        return home;
    }
    std::env::temp_dir()
}
