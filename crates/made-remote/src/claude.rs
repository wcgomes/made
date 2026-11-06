//! Direct remote Claude Code launch (SSH-11).
//!
//! The companion starts the absolute `claude` found by a metadata-only PATH
//! lookup with no arguments, no shell first, no plugin or Claude environment,
//! no bridge and no file write. Status reaches Made through the remote
//! terminal's status protocol, not through the companion.
use made_platform::{find_executable_on_path, Environment};
use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

/// The program and arguments of a direct Claude Code launch.
pub struct ClaudeLaunch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

/// Resolve the launch from the companion's environment. Empty and relative
/// PATH entries are ignored; nothing is spawned or version-checked. `None`
/// means claude is gone and the launch must fail closed.
pub fn launch(environment: &Environment) -> Option<ClaudeLaunch> {
    let path = environment.get("PATH")?;
    find_executable_on_path(OsStr::new("claude"), path).map(|program| ClaudeLaunch {
        program,
        args: Vec::new(),
    })
}
