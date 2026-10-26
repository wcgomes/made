//! Explorer domain: lazy tree, filesystem watcher, path guard, and read-only
//! preview.
//!
//! The explorer has no OpenCode dependency. It receives an active workspace root
//! and a root generation from `made-app` and reports tree and preview state
//! back. See `wiki/architecture.md` for the data flow.
//!
//! # Public API contract
//!
//! All entry points take owned or borrowed filesystem paths plus a
//! [`RootGeneration`], never app or OpenCode types. `PathGuard` canonicalizes
//! the active root and every target, rejects `..` traversal, and resolves
//! symlinks before containment checks. Extra roots can be granted explicitly;
//! v1's UI grants none.
//!
//! Loading is lazy. Creating a [`TreeModel`] or [`PreviewLoader`] does not scan
//! the workspace. Directory loads read exactly one directory, preview reads are
//! capped at 2 MiB and binary-guarded, highlighting is capped at
//! [`highlight::HIGHLIGHT_SIZE_LIMIT`] so it stays off the hot path for large
//! files, and there is no write/save API anywhere in this crate.
//!
//! # Thread safety
//!
//! Public values own their data and contain no UI handles. [`TreeModel`] is
//! single-owner; [`PreviewLoader`] is cheap to clone and can be sent to a
//! background worker. [`highlight::Highlighter`] and preview loads are plain
//! value operations that can run off the UI thread. [`FsWatcher`] and
//! [`PreviewRefresher`] own their watcher/debounce threads and provide polling
//! APIs intended for the egui thread; their internal threads only publish
//! channel messages, and watcher queues are bounded with rescan/coalescing
//! behavior when they overflow.
//!
//! Results carry the root generation they were started with. Apply loads and
//! events through the model APIs so older work is rejected after a root switch.

pub mod highlight;
pub mod paths;
pub mod preview;
pub mod preview_refresh;
pub mod root_switch;
pub mod tree;
pub mod watch;

pub use highlight::HIGHLIGHT_SIZE_LIMIT;
pub use paths::{PathError, PathGuard};
pub use preview::{Preview, PreviewKind, PreviewLoader, BINARY_SNIFF_LEN, PREVIEW_SIZE_LIMIT};
pub use preview_refresh::{
    PreviewError, PreviewRefresh, PreviewRefreshEvent, PreviewRefresher, PreviewSelection,
};
pub use root_switch::{GenerationGate, RootGeneration, StaleGeneration};
pub use tree::{
    DirListing, DirRequest, EntryKind, TreeEntry, TreeError, TreeModel, TreeRow, TreeUpdate,
    MAX_DIR_ENTRIES, MAX_TREE_DEPTH, MAX_TREE_ROWS,
};
pub use watch::{FsWatcher, WatchError, WatchEvent, WatchEventKind, MAX_PENDING_EVENTS};

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}
    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn public_values_honor_the_documented_threading_contract() {
        assert_send_sync::<PathGuard>();
        assert_send_sync::<PreviewLoader>();
        assert_send_sync::<highlight::Highlighter>();
        assert_send_sync::<RootGeneration>();
        assert_send::<TreeModel>();
        assert_send::<FsWatcher>();
        assert_send::<PreviewRefresher>();
        assert_send_sync::<WatchEvent>();
    }
}
