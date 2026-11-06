//! Claude Code integration.

use super::{
    Descriptor, FollowKind, FollowPolicy, Integration, Mark, OnUnusable, Rank, WhileOutranked,
};

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
}
