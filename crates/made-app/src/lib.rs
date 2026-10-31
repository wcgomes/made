//! Made application shell: composition root, views, and cross-domain coordination.
//!
//! This crate wires together the terminal, platform, explorer, and bridge
//! domains. It depends on all four; none of them may depend on this crate.
//! `wiki/architecture.md` records the full dependency and module contract.
//!
//! The core modules in this crate are headless and UI-agnostic. They provide the
//! launch/workspace state, folder picker, session follow, session map, focus
//! controller, worktree metadata, integration status, diagnostics, metrics, and
//! runtime/scope guards. The egui shell composes these APIs.

pub mod app;
mod cell_glyphs;
pub mod cli;
pub mod diagnostics;
pub mod explorer_icons;
pub mod explorer_view;
pub mod integration_status;
pub mod metrics;
pub mod render;
pub mod runtime_guard;
pub mod sanitize;
pub mod session_focus;
pub mod session_follow;
pub mod session_map;
pub mod terminal_view;
pub mod theme;
pub mod tui;
pub mod ui;
pub mod workbench;
pub mod workspace;
pub mod workspace_picker;
pub mod worktree;

pub use app::{native_options, AppModel, AppOptions, WorkspaceId};
pub use cli::{CliAction, CliError, LaunchArgs};
pub use diagnostics::{
    DiagnosticLog, Diagnostics, DiagnosticsSnapshot, LogEntry, LogLevel, WorkspaceDiagnostics,
};
pub use explorer_view::{
    document_text, preview_shortcut_action, ExplorerPane, PreviewAction, PreviewDocument,
    PreviewLine, PreviewSpan, PreviewUiAction, PREVIEW_ACTIONS,
};
pub use integration_status::{
    AgentIntegration, IntegrationReport, IntegrationState, IntegrationStatus, ReasonCode,
    ScopeBoundary, StatusChange, StatusReason, V1_SCOPE,
};
pub use made_platform::CompatibleTui;
pub use metrics::{BudgetStatus, Metrics, PerformanceReport};
pub use render::{
    paint_grid, CellMetrics, CellPoint, GridPalette, GridRenderer, PaintOptions, SelectionRange,
};
pub use sanitize::{sanitize_copy_text, sanitize_display, sanitize_log_text, sanitize_text};
pub use session_focus::{
    FocusError, FocusErrorKind, FocusOutcome, FocusSink, FocusUpdate, PendingFocus,
    SessionFocusController, StateRefresh,
};
pub use session_follow::{
    FollowError, FollowedSession, SessionFollowCoordinator, SessionFollowOutcome,
    SessionFollowUpdate,
};
pub use session_map::{SessionMap, SessionMapUpdate, SessionRow};
pub use terminal_view::{TerminalPane, TerminalUiOutput};
pub use theme::TRANSLUCENT_WINDOW;
pub use tui::TerminalTarget;
pub use workbench::{
    platform_family_label, worktree_indicator, LeftSurface, Workbench, WorktreeState,
};
pub use workspace::{
    LaunchPlan, Surfaces, WorkspaceError, WorkspaceErrorKind, WorkspaceLaunch, WorkspaceRoot,
    WorkspaceSelection, WorkspaceSource,
};
pub use workspace_picker::{PickerEntry, PickerError, WorkspacePicker};
pub use worktree::{
    GitCommand, GitRunner, MetadataStatus, WorktreeMetadata, GIT_PROGRAM_ENV,
    WORKTREE_MANAGEMENT_AVAILABLE,
};

/// Product name used by the binary, window chrome, and diagnostics.
pub const APP_NAME: &str = "Made";

/// Application crate version.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Short product description shown by About.
pub const APP_DESCRIPTION: &str = "A native, terminal-first desktop workspace for developers.";
