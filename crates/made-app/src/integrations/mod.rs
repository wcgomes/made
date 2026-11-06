//! The integrations Made ships and the one static list that names them.
//!
//! An integration is an agent terminal UI Made can launch directly, follow and
//! diagnose. Each one is a module of this directory that implements
//! [`Integration`]; this file holds the interface and [`PRODUCT`], the only
//! place the product names its integrations. Everything here is compile-time:
//! every integration is built into the binary, and nothing is loaded,
//! discovered or selected by configuration at run time. This is not a plugin
//! API.
//!
//! Precedence between integrations is the declared [`Rank`], never the position
//! in [`PRODUCT`], which is the catalog (new-tab menu) order.

use std::fmt;
use std::hash::{Hash, Hasher};

mod claude_code;
mod grok;
mod opencode;

/// A static reference to one integration. Two references are equal when their
/// descriptors have the same key.
pub type IntegrationRef = &'static dyn Integration;

/// A list of integrations in catalog order.
pub type IntegrationList = &'static [IntegrationRef];

/// OpenCode v2.
pub const OPENCODE: IntegrationRef = &opencode::OpenCode;

/// Grok Build.
pub const GROK: IntegrationRef = &grok::Grok;

/// Claude Code.
pub const CLAUDE_CODE: IntegrationRef = &claude_code::ClaudeCode;

/// The product's integrations, in catalog (menu) order. The only place the
/// product names them. Precedence is the declared rank, not this order.
pub const PRODUCT: IntegrationList = &[OPENCODE, GROK, CLAUDE_CODE];

/// Precedence of an integration. A lower rank wins. Ranks leave gaps so an
/// integration can be inserted between two others without renumbering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rank(pub u16);

/// The mark an integration shows in the new-tab menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// An icon vendored with the explorer icons, by SVG file name.
    Vendored(&'static str),
    /// An image owned by the integration.
    Image {
        /// Image cache identifier.
        uri: &'static str,
        /// Image bytes.
        bytes: &'static [u8],
    },
}

/// Identity and display data of one integration.
///
/// Several identifiers exist for the same integration with different values;
/// each stays its own field so every literal is stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Descriptor {
    /// Unique internal key.
    pub key: &'static str,
    /// Executable name resolved through `PATH`.
    pub program: &'static str,
    /// Product label for the new-tab menu and an open workspace tab.
    pub label: &'static str,
    /// Identifier of the integration's new-tab menu item.
    pub menu_item_id: &'static str,
    /// Stable identifier in the v1 scope declaration.
    pub scope_id: &'static str,
    /// Human-readable name in the v1 scope declaration.
    pub scope_name: &'static str,
    /// Identifier reported when the integration owns the explorer root.
    pub root_source_id: Option<&'static str>,
    /// Mark shown in the new-tab menu.
    pub mark: Mark,
    /// Precedence rank.
    pub rank: Rank,
}

/// One integration, implemented by exactly one module of this directory.
pub trait Integration: Sync + 'static {
    /// Identity and display data.
    fn descriptor(&self) -> &'static Descriptor;

    /// Arguments of a direct launch of the integration's program.
    fn launch_args(&self) -> &'static [&'static str] {
        &[]
    }

    /// Whether the integration owns the bridge session channel.
    fn owns_session_channel(&self) -> bool {
        false
    }
}

impl PartialEq for dyn Integration {
    fn eq(&self, other: &Self) -> bool {
        self.descriptor().key == other.descriptor().key
    }
}

impl Eq for dyn Integration {}

impl Hash for dyn Integration {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.descriptor().key.hash(state);
    }
}

impl fmt::Debug for dyn Integration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Integration")
            .field(&self.descriptor().key)
            .finish()
    }
}

/// Why a list is not a valid integration list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListError {
    /// Two entries have this key.
    DuplicateKey(&'static str),
    /// Two entries have this rank.
    DuplicateRank(Rank),
    /// The entry with this key owns the session channel without having the
    /// lowest rank.
    SessionChannelOwnerNotLowest(&'static str),
}

/// Check that keys and ranks are unique and that every session-channel owner
/// has the lowest rank of the list.
pub fn validate(list: IntegrationList) -> Result<(), ListError> {
    for (index, entry) in list.iter().enumerate() {
        let descriptor = entry.descriptor();
        for other in &list[..index] {
            let other = other.descriptor();
            if other.key == descriptor.key {
                return Err(ListError::DuplicateKey(descriptor.key));
            }
            if other.rank == descriptor.rank {
                return Err(ListError::DuplicateRank(descriptor.rank));
            }
        }
    }
    let lowest = list.iter().map(|entry| entry.descriptor().rank).min();
    for entry in list {
        if entry.owns_session_channel() && Some(entry.descriptor().rank) != lowest {
            return Err(ListError::SessionChannelOwnerNotLowest(
                entry.descriptor().key,
            ));
        }
    }
    Ok(())
}
