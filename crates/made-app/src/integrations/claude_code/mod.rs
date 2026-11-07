//! Claude Code integration.

pub mod diagnostics;
pub mod follow;
pub mod launch;
pub(crate) mod tab;

use super::{
    AppInit, AppState, Descriptor, DiagnosticsDecl, EnvPolicy, FollowKind, FollowPolicy,
    Integration, IntegrationList, Mark, OnUnusable, Rank, RowDecl, RowReport, Slots, TabInit,
    TabState, WhileOutranked, CLAUDE_CODE,
};
use crate::diagnostics::DiagnosticsPill;
use crate::tui::TerminalTarget;
use crate::workbench::Workbench;
use diagnostics::ClaudeAppState;
use launch::CLAUDE_SPOOL_ENV;
pub(crate) use tab::ClaudeTabState;

/// Claude Code, started directly with no arguments.
pub(super) struct ClaudeCode;

static DESCRIPTOR: Descriptor = Descriptor {
    key: "claude-code",
    program: "claude",
    label: "Claude Code",
    menu_item_id: "claude",
    scope_id: "claude-code",
    scope_name: "Claude Code",
    root_source_id: Some("claude-code"),
    mark: Mark::Image {
        uri: "bytes://claude-code.svg",
        bytes: include_bytes!("../../../assets/claude-code.svg"),
    },
    rank: Rank(200),
};

impl Integration for ClaudeCode {
    fn descriptor(&self) -> &'static Descriptor {
        &DESCRIPTOR
    }

    fn follow(&self) -> FollowKind {
        FollowKind::Directory(FollowPolicy {
            on_unusable: OnUnusable::Reject("Claude Code directory is not usable"),
            under_session_channel: WhileOutranked::Retain,
            under_higher_directory: WhileOutranked::Retain,
        })
    }

    fn env_policy(&self) -> EnvPolicy {
        EnvPolicy {
            shell: true,
            own_direct: true,
            strip: &[CLAUDE_SPOOL_ENV],
        }
    }

    fn new_app_state(&self, init: &AppInit<'_>) -> Option<Box<dyn AppState>> {
        Some(Box::new(ClaudeAppState::new(init)))
    }

    fn new_tab_state(
        &self,
        _target: TerminalTarget,
        init: &TabInit<'_>,
    ) -> Option<Box<dyn TabState>> {
        Some(Box::new(ClaudeTabState::new(init)))
    }

    fn polls_remote_tabs(&self) -> bool {
        true
    }

    fn diagnostics(&self) -> DiagnosticsDecl {
        DiagnosticsDecl::Row(RowDecl {
            status_key: "claude status",
            sentence_key: "claude",
            initial: RowReport::new(DiagnosticsPill::Inactive, "Claude Code is not on PATH."),
        })
    }
}

/// Claude Code state of a workbench's tab: presence, dots and title inputs.
/// `None` for tabs that do not load Made's Claude Code plugin.
#[doc(hidden)]
pub fn tab(workbench: &Workbench) -> Option<&follow::ClaudeTab> {
    tab_state(workbench.integrations(), workbench.tab_states()).and_then(ClaudeTabState::tab)
}

/// The Claude Code state of a tab built over `list`.
pub(crate) fn tab_state(
    list: IntegrationList,
    slots: &Slots<Box<dyn TabState>>,
) -> Option<&ClaudeTabState> {
    super::tab_state::<ClaudeTabState>(list, slots, CLAUDE_CODE)
}

/// The Claude Code state of a tab built over `list`, mutably.
pub(crate) fn tab_state_mut(
    list: IntegrationList,
    slots: &mut Slots<Box<dyn TabState>>,
) -> Option<&mut ClaudeTabState> {
    super::tab_state_mut::<ClaudeTabState>(list, slots, CLAUDE_CODE)
}
