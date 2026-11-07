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

use std::any::Any;
use std::ffi::OsString;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Instant;

use made_platform::AppPaths;
use made_terminal::PtyOptions;

use crate::diagnostics::DiagnosticLog;
use crate::session_follow::SessionFollowOutcome;
use crate::terminal_view::TerminalPane;
use crate::tui::TerminalTarget;

pub(crate) mod claude_code;
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

    /// How the integration takes part in explorer-root follow.
    fn follow(&self) -> FollowKind {
        FollowKind::None
    }

    /// Where the integration's launch variables go.
    fn env_policy(&self) -> EnvPolicy {
        EnvPolicy::NONE
    }

    /// The integration's app-wide state, or `None` when it keeps none.
    fn new_app_state(&self, _init: &AppInit<'_>) -> Option<Box<dyn AppState>> {
        None
    }

    /// The integration's state for one tab, or `None` when it keeps none.
    fn new_tab_state(
        &self,
        _target: TerminalTarget,
        _init: &TabInit<'_>,
    ) -> Option<Box<dyn TabState>> {
        None
    }

    /// Whether remote tabs poll this integration's tab state.
    fn polls_remote_tabs(&self) -> bool {
        false
    }
}

/// How an integration takes part in explorer-root follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowKind {
    /// The integration does not follow a directory.
    None,
    /// The integration owns the bridge session channel (active session, view
    /// directory, connection epoch). At most one entry of a list does, and it
    /// has the lowest rank.
    SessionChannel,
    /// The integration reports the directory of its session, ranked against
    /// the other directory holders by its declared policy.
    Directory(FollowPolicy),
}

/// What a directory-follow integration does with a report. The differences
/// between integrations are declared here, never decided by identity.
///
/// An integration that retains while outranked processes a report as usual and
/// only withholds the root move. One that ignores while outranked drops the
/// report before looking at it. A report is checked against the session
/// channel first, then against higher-ranked directory holders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FollowPolicy {
    /// What a relative, missing or unreadable directory does.
    pub on_unusable: OnUnusable,
    /// Behavior while the session channel holds the root.
    pub under_session_channel: WhileOutranked,
    /// Behavior while a higher-ranked directory holder holds the root.
    pub under_higher_directory: WhileOutranked,
}

/// How an unusable reported directory is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnUnusable {
    /// Reject the report with this detail. The root and the previously
    /// retained directory stay unchanged.
    Reject(&'static str),
    /// Drop the report without a rejection. The previously retained directory
    /// stays.
    Ignore,
}

/// What an integration does with a report while a higher-precedence holder
/// owns the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhileOutranked {
    /// Retain or clear the reported directory so it can take the root when the
    /// holder releases it.
    Retain,
    /// Drop the report: nothing is retained or cleared.
    Ignore,
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
        if entry.follow() == FollowKind::SessionChannel && Some(entry.descriptor().rank) != lowest {
            return Err(ListError::SessionChannelOwnerNotLowest(
                entry.descriptor().key,
            ));
        }
    }
    Ok(())
}

/// Where an integration's launch variables go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvPolicy {
    /// The variables apply to a shell tab.
    pub shell: bool,
    /// The variables apply to the integration's own direct target.
    pub own_direct: bool,
    /// Variables removed from every launch the policy does not cover, and from
    /// a covered launch that does not bind them.
    pub strip: &'static [&'static str],
}

impl EnvPolicy {
    /// No variables anywhere.
    pub const NONE: Self = Self {
        shell: false,
        own_direct: false,
        strip: &[],
    };

    /// Whether `target` is one the policy puts the integration's variables on:
    /// the shell, or the integration's own direct target.
    pub fn covers(&self, entry: IntegrationRef, target: TerminalTarget) -> bool {
        match target {
            TerminalTarget::Shell => self.shell,
            TerminalTarget::Tui(tui) => self.own_direct && tui == entry,
            // Remote targets never receive local variables.
            _ => false,
        }
    }
}

/// What one integration adds to a launch: variables to set, then variables to
/// remove.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvContribution {
    /// Variables set on the PTY child.
    pub set: Vec<(OsString, OsString)>,
    /// Variables removed from the PTY child.
    pub strip: Vec<&'static str>,
}

impl EnvContribution {
    /// Apply the contribution to PTY options: sets first, then removals.
    pub fn apply(&self, mut options: PtyOptions) -> PtyOptions {
        for (key, value) in &self.set {
            options = options.with_env(key.clone(), value.clone());
        }
        for key in &self.strip {
            options = options.without_env(*key);
        }
        options
    }
}

/// What a tab poll asks the core to do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TabReport {
    /// A directory for the follow coordinator. `Some(None)` is a release.
    pub directory: Option<Option<PathBuf>>,
}

/// Inputs for creating an integration's app-wide state.
#[derive(Debug, Clone, Copy)]
pub struct AppInit<'a> {
    /// PATH detection was requested at startup.
    pub detecting: bool,
    /// The detection worker started.
    pub detection_started: bool,
    /// The integrations the catalog offers.
    pub catalog: &'a [IntegrationRef],
}

/// Inputs for creating an integration's state for one tab.
#[derive(Debug, Clone, Copy)]
pub struct TabInit<'a> {
    /// Resolved application paths.
    pub paths: Option<&'a AppPaths>,
    /// The tab is a remote tab.
    pub remote: bool,
}

/// What the core lends to a launch preparation.
pub struct LaunchCtx<'a> {
    /// Resolved application paths.
    pub paths: Option<&'a AppPaths>,
    /// The tab's identifier.
    pub pane_id: &'a str,
    /// The tab's diagnostics log.
    pub log: &'a mut DiagnosticLog,
}

/// What the core lends to a tab poll.
pub struct TabIo<'a> {
    /// The tab's terminal, once it exists.
    pub terminal: Option<&'a mut TerminalPane>,
    /// The tab's launch target.
    pub target: TerminalTarget,
    /// The tab is a remote tab.
    pub remote: bool,
}

/// What the core lends to app-wide state.
#[derive(Debug, Default)]
pub struct AppCtx {
    diagnostics_changed: bool,
}

impl AppCtx {
    /// Ask the core to push diagnostics to every workspace.
    pub fn mark_diagnostics_changed(&mut self) {
        self.diagnostics_changed = true;
    }

    /// Whether a state asked for a diagnostics push.
    pub fn diagnostics_changed(&self) -> bool {
        self.diagnostics_changed
    }
}

/// An integration's app-wide state, owned by the app model.
pub trait AppState: Any {
    /// PATH detection finished and did or did not find the integration.
    fn on_detection(&mut self, detected: bool, ctx: &mut AppCtx);

    /// PATH detection ended without a result.
    fn on_detection_failed(&mut self, _ctx: &mut AppCtx) {}

    /// Apply the state's background work once per frame.
    fn poll(&mut self, _ctx: &mut AppCtx) {}

    /// This state as `Any`.
    fn as_any(&self) -> &dyn Any;

    /// This state as mutable `Any`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// An integration's state for one tab, owned by the tab. Tabs are prepared on
/// a worker thread, so the state is `Send`.
pub trait TabState: Any + Send {
    /// Prepare a launch of `target`: the variables to set and remove.
    fn prepare_launch(
        &mut self,
        _target: TerminalTarget,
        _ctx: &mut LaunchCtx<'_>,
    ) -> EnvContribution {
        EnvContribution::default()
    }

    /// The prepared launch started a terminal child.
    fn on_spawned(&mut self, _pid: Option<u32>) {}

    /// The prepared launch did not start a terminal child.
    fn on_launch_failed(&mut self) {}

    /// Apply the integration's channels. A directory in the report is handed
    /// to the follow coordinator by the core.
    fn poll(&mut self, _now: Instant, _io: &mut TabIo<'_>) -> TabReport {
        TabReport::default()
    }

    /// The coordinator's answer to the directory this state reported, or
    /// `None` when the tab has no coordinator.
    fn on_follow_outcome(&mut self, _outcome: Option<&SessionFollowOutcome>) {}

    /// The tab closes or its terminal is replaced: drop per-terminal state.
    fn teardown(&mut self) {}

    /// This state as `Any`.
    fn as_any(&self) -> &dyn Any;

    /// This state as mutable `Any`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Optional values parallel to an integration list: slot `i` belongs to entry
/// `i` of the list its owner was built with.
pub struct Slots<T> {
    items: Vec<Option<T>>,
}

impl<T> Slots<T> {
    /// One slot per entry of `list`, filled by `make`.
    pub fn from_list(
        list: IntegrationList,
        mut make: impl FnMut(IntegrationRef) -> Option<T>,
    ) -> Self {
        Self {
            items: list.iter().map(|entry| make(*entry)).collect(),
        }
    }

    /// The value in slot `index`.
    pub fn get(&self, index: usize) -> Option<&T> {
        self.items.get(index).and_then(Option::as_ref)
    }

    /// The value in slot `index`, mutably.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.items.get_mut(index).and_then(Option::as_mut)
    }

    /// Every filled slot, in list order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.items.iter_mut().filter_map(Option::as_mut)
    }
}

/// The position of `entry` in `list`.
pub fn position(list: IntegrationList, entry: IntegrationRef) -> Option<usize> {
    list.iter().position(|candidate| *candidate == entry)
}

/// The tab state of `entry` as its concrete type.
pub fn tab_state<S: TabState>(
    list: IntegrationList,
    slots: &Slots<Box<dyn TabState>>,
    entry: IntegrationRef,
) -> Option<&S> {
    let state = slots.get(position(list, entry)?)?;
    state.as_any().downcast_ref::<S>()
}

/// The tab state of `entry` as its concrete type, mutably.
pub fn tab_state_mut<S: TabState>(
    list: IntegrationList,
    slots: &mut Slots<Box<dyn TabState>>,
    entry: IntegrationRef,
) -> Option<&mut S> {
    let state = slots.get_mut(position(list, entry)?)?;
    state.as_any_mut().downcast_mut::<S>()
}

/// The app state of `entry` as its concrete type.
pub fn app_state<S: AppState>(
    list: IntegrationList,
    slots: &Slots<Box<dyn AppState>>,
    entry: IntegrationRef,
) -> Option<&S> {
    let state = slots.get(position(list, entry)?)?;
    state.as_any().downcast_ref::<S>()
}

/// The app state of `entry` as its concrete type, mutably.
pub fn app_state_mut<S: AppState>(
    list: IntegrationList,
    slots: &mut Slots<Box<dyn AppState>>,
    entry: IntegrationRef,
) -> Option<&mut S> {
    let state = slots.get_mut(position(list, entry)?)?;
    state.as_any_mut().downcast_mut::<S>()
}
