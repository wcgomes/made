//! Grok Build integration.

use super::{Descriptor, Integration, Mark, Rank};

/// Grok Build, started directly as `grok dashboard`.
pub(super) struct Grok;

static DESCRIPTOR: Descriptor = Descriptor {
    key: "grok",
    program: "grok",
    label: "Grok Build",
    menu_item_id: "grok",
    scope_id: "grok-build",
    scope_name: "Grok Build",
    root_source_id: Some("grok"),
    mark: Mark::Image {
        uri: "bytes://grok.svg",
        bytes: include_bytes!("../../../assets/grok.svg"),
    },
    rank: Rank(300),
};

impl Integration for Grok {
    fn descriptor(&self) -> &'static Descriptor {
        &DESCRIPTOR
    }

    fn launch_args(&self) -> &'static [&'static str] {
        &["dashboard"]
    }
}
