//! OpenCode v2 integration.

use super::{Descriptor, FollowKind, Integration, Mark, Rank};

/// OpenCode v2, the owner of the bridge session channel.
pub(super) struct OpenCode;

static DESCRIPTOR: Descriptor = Descriptor {
    key: "opencode",
    program: "opencode",
    label: "OpenCode",
    menu_item_id: "opencode",
    scope_id: "opencode-v2",
    scope_name: "OpenCode v2",
    root_source_id: Some("opencode"),
    mark: Mark::Vendored("opencode.svg"),
    rank: Rank(100),
};

impl Integration for OpenCode {
    fn descriptor(&self) -> &'static Descriptor {
        &DESCRIPTOR
    }

    fn follow(&self) -> FollowKind {
        FollowKind::SessionChannel
    }
}
