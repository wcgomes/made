//! Parent-process lookup.
//!
//! This is the only supported query. Other crates must not read `/proc`, call
//! `proc_pidinfo`, or walk the Toolhelp snapshot themselves.

/// Parent process id of `pid`, when the operating system can report one.
///
/// Returns `None` when the process does not exist or the query fails. The
/// value is the raw parent id, including `0` and `1`; callers decide how to
/// treat those sentinels.
pub fn process_parent(pid: u32) -> Option<u32> {
    parent_pid(pid)
}

#[cfg(target_os = "macos")]
fn parent_pid(pid: u32) -> Option<u32> {
    #[repr(C)]
    struct ProcBsdShortInfo {
        pid: u32,
        ppid: u32,
        pgid: u32,
        status: u32,
        comm: [u8; 16],
        flags: u32,
        uid: u32,
        gid: u32,
        ruid: u32,
        rgid: u32,
        svuid: u32,
        svgid: u32,
        rfu: u32,
    }

    #[link(name = "proc", kind = "dylib")]
    extern "C" {
        fn proc_pidinfo(
            pid: i32,
            flavor: i32,
            arg: u64,
            buffer: *mut std::ffi::c_void,
            buffersize: i32,
        ) -> i32;
    }

    const PROC_PIDT_SHORTBSDINFO: i32 = 13;
    let mut info = std::mem::MaybeUninit::<ProcBsdShortInfo>::zeroed();
    // SAFETY: `info` is a writable `ProcBsdShortInfo` of the size passed in.
    // `proc_pidinfo` only writes that buffer. `assume_init` runs only after the
    // kernel reports that at least the pid and ppid bytes were written.
    let wrote = unsafe {
        proc_pidinfo(
            pid as i32,
            PROC_PIDT_SHORTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            std::mem::size_of::<ProcBsdShortInfo>() as i32,
        )
    };
    if wrote < 8 {
        return None;
    }
    // SAFETY: `wrote >= 8` covers the leading `pid` and `ppid` fields, and
    // `proc_pidinfo` wrote them into this `MaybeUninit`.
    Some(unsafe { info.assume_init() }.ppid)
}

#[cfg(target_os = "linux")]
fn parent_pid(pid: u32) -> Option<u32> {
    // `comm` is wrapped in parentheses and may itself contain spaces or
    // parentheses. The state and ppid are the first two fields after it.
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let end = stat.rfind(')')?;
    let mut fields = stat[end + 1..].split_whitespace();
    let _state = fields.next()?;
    fields.next()?.parse().ok()
}

#[cfg(target_os = "windows")]
fn parent_pid(pid: u32) -> Option<u32> {
    #[repr(C)]
    struct ProcessEntry32W {
        size: u32,
        cnt_usage: u32,
        process_id: u32,
        default_heap_id: usize,
        module_id: u32,
        threads: u32,
        parent_process_id: u32,
        pri_class_base: i32,
        flags: u32,
        exe_file: [u16; 260],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut std::ffi::c_void;
        fn Process32FirstW(snapshot: *mut std::ffi::c_void, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: *mut std::ffi::c_void, entry: *mut ProcessEntry32W) -> i32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }

    struct Snapshot(*mut std::ffi::c_void);

    impl Drop for Snapshot {
        fn drop(&mut self) {
            // SAFETY: the pointer was returned by CreateToolhelp32Snapshot and
            // is not null or invalid. This drop is the only close.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    // SAFETY: snapshot of the process list; a null or invalid handle means the
    // call failed and there is nothing to close.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot.is_null() || snapshot == usize::MAX as *mut std::ffi::c_void {
        return None;
    }
    let _snapshot = Snapshot(snapshot);
    let mut entry = ProcessEntry32W {
        size: std::mem::size_of::<ProcessEntry32W>() as u32,
        cnt_usage: 0,
        process_id: 0,
        default_heap_id: 0,
        module_id: 0,
        threads: 0,
        parent_process_id: 0,
        pri_class_base: 0,
        flags: 0,
        exe_file: [0; 260],
    };
    let mut found = None;
    // SAFETY: `entry.size` is the struct size Toolhelp requires, and `snapshot`
    // is a live process snapshot. The loop stops when Toolhelp returns false.
    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) };
    while ok != 0 {
        if entry.process_id == pid {
            found = Some(entry.parent_process_id);
            break;
        }
        // SAFETY: same snapshot and entry as Process32FirstW.
        ok = unsafe { Process32NextW(snapshot, &mut entry) };
    }
    found
}

#[cfg(test)]
mod tests {
    use super::process_parent;

    #[test]
    fn current_process_has_a_parent() {
        let parent = process_parent(std::process::id());
        assert!(parent.is_some());
        assert_ne!(parent, Some(std::process::id()));
    }
}
