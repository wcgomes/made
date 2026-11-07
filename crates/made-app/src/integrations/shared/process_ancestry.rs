//! Process ancestry: whether a process descends from a terminal's process.

/// Whether `pid` descends from `root` through the live parent chain.
pub(crate) fn process_belongs_to(pid: u32, root: u32) -> bool {
    process_belongs_to_with(pid, root, made_platform::process_parent)
}

/// [`process_belongs_to`] over an injected parent lookup.
pub(crate) fn process_belongs_to_with(
    mut pid: u32,
    root: u32,
    mut parent_of: impl FnMut(u32) -> Option<u32>,
) -> bool {
    for _ in 0..32 {
        let Some(parent) = parent_of(pid) else {
            return false;
        };
        if parent == root {
            return true;
        }
        if parent <= 1 || parent == pid {
            return false;
        }
        pid = parent;
    }
    false
}
