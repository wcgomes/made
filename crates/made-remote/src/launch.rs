//! The environment a remote launch's terminal child receives, composed on the
//! server. The companion's launch and the follow-up shell that takes a
//! launch-bar TUI's place both come from here, so the shell that follows a TUI
//! gets what a fresh `Terminal SSH` shell gets.
use crate::{claude, protocol::LaunchTarget};
use made_platform::Environment;
use std::{ffi::OsString, path::Path};

/// Variables as the child receives them.
pub type Vars = Vec<(OsString, OsString)>;

/// The environment of one launch and what its follow-up shell adds to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchEnvironment {
    /// What the child of the launch target receives.
    pub launch: Vars,
    /// What a plain shell launch receives beyond `launch` that needs no
    /// running service: the Claude Code plugin entry and spool binding.
    /// Empty for a shell launch.
    pub follow_up: Vars,
}

/// Compose the environment of a `target` launch.
///
/// `base` is the platform environment with terminal defaults; `integration`
/// is the OpenCode integration's variable when the launch owns one (a shell
/// or OpenCode launch; Grok and Claude Code launches own none); `staged` is
/// the server-staged Claude Code plugin package; `user` is the remote user's
/// own environment, whose plugin list is composed with the staged entry; and
/// `spool` is the tab's spool binding. No host value is involved, and an
/// inherited spool binding never reaches another tab's processes.
pub fn compose(
    target: LaunchTarget,
    base: &Environment,
    integration: Option<(&str, &str)>,
    staged: Option<&Path>,
    user: &Environment,
    spool: &Path,
) -> LaunchEnvironment {
    let build = |target| {
        let mut env = base.clone();
        if let Some((key, value)) = integration {
            env.set(key, value);
        }
        env.remove(claude::CLAUDE_SPOOL_ENV);
        for (key, value) in claude::launch_environment(target, staged, user, spool).env {
            env.set(key, value);
        }
        env.into_vec()
    };
    let launch = build(target);
    let follow_up = build(LaunchTarget::Shell)
        .into_iter()
        .filter(|(key, value)| {
            !launch
                .iter()
                .any(|(launch_key, launch_value)| launch_key == key && launch_value == value)
        })
        .collect();
    LaunchEnvironment { launch, follow_up }
}
