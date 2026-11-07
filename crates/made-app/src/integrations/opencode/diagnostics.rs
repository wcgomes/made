//! OpenCode app-wide state: whether OpenCode is known to be missing.

use std::any::Any;

use made_platform::AppPaths;

use crate::integrations::{AppCtx, AppInit, AppState, OPENCODE};

/// OpenCode was omitted by an explicit catalog, or PATH detection finished
/// without it.
pub(crate) struct OpenCodeAppState {
    /// Stays false while detection is still running, so the row keeps the
    /// inactive not-running sentence.
    missing: bool,
    /// Every workspace open when OpenCode became missing was told.
    announced: bool,
}

impl OpenCodeAppState {
    pub(crate) fn new(init: &AppInit<'_>) -> Self {
        // A live worker has not answered yet. A thread that never started is
        // the same as detection finishing without OpenCode.
        let missing = if init.detecting {
            !init.detection_started
        } else {
            !init.catalog.contains(&OPENCODE)
        };
        Self {
            missing,
            // No workspace exists while the model is built.
            announced: missing,
        }
    }

    /// Whether OpenCode is known to be missing.
    pub(crate) fn missing(&self) -> bool {
        self.missing
    }

    /// Whether the open workspaces still have to be told that OpenCode is
    /// missing. Asking clears it.
    pub(crate) fn take_missing_notice(&mut self) -> bool {
        let pending = self.missing && !self.announced;
        self.announced = self.missing;
        pending
    }
}

impl AppState for OpenCodeAppState {
    fn on_detection(&mut self, detected: bool, _ctx: &mut AppCtx) {
        if !detected {
            self.missing = true;
        }
    }

    fn on_detection_failed(&mut self, _ctx: &mut AppCtx) {
        self.missing = true;
    }

    /// Where the OpenCode plugin source resolves from, and whether its package
    /// is there.
    fn smoke_marker(&self, paths: Option<&AppPaths>) -> Option<(&'static str, String)> {
        let plugin = paths
            .map(|paths| {
                let found = paths.plugin_dir.join("package.json").is_file();
                format!(
                    "{}{}",
                    paths.plugin_source.name(),
                    if found { "" } else { ":missing" }
                )
            })
            .unwrap_or_else(|| "unresolved".to_owned());
        Some(("opencode_plugin", plugin))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
