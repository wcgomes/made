//! Atomic app-owned metadata, not a workspace file API. The caller supplies a
//! trusted app-data directory; its parent ancestry must be trusted. The final
//! private directory and files are pinned/checked, symlinks/reparse points and
//! nonregular files are refused. Unix uses dirfd-relative no-follow operations;
//! Windows uses protected owner-only DACLs and a non-delete-shared directory
//! handle. Metadata is intentionally opaque: schema/version/corruption policy
//! belongs to the caller, never this capability.
use crate::PlatformError;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Hard allocation bound; callers normally choose a much smaller registry cap.
pub const MAX_PRIVATE_METADATA_BYTES: usize = 16 * 1024 * 1024;
static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "unsafe private metadata path, kind, permissions or bounds",
    )
}
fn validate(name: &str, max_bytes: usize) -> Result<(), PlatformError> {
    // Reject Windows device aliases on every host so a portable caller cannot
    // accidentally persist a record that changes meaning on another target.
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    if max_bytes == 0
        || max_bytes > MAX_PRIVATE_METADATA_BYTES
        || name.len() > 128
        || name.ends_with('.')
        || device
        || !name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(PlatformError::io("validate private metadata", invalid()));
    }
    Ok(())
}

/// Read bounded opaque metadata without creating/mutating anything. Missing
/// directory/file returns None; unsafe/oversized/unreadable data returns error.
pub fn read_private_metadata(
    directory: impl AsRef<Path>,
    name: &str,
    max_bytes: usize,
) -> Result<Option<Vec<u8>>, PlatformError> {
    validate(name, max_bytes)?;
    let directory = directory.as_ref();
    let Some(root) = backend::open_directory(directory)
        .map_err(|e| PlatformError::io("open private metadata directory", e))?
    else {
        return Ok(None);
    };
    let Some(file) = backend::read(&root, directory, name)
        .map_err(|e| PlatformError::io("open private metadata", e))?
    else {
        return Ok(None);
    };
    let size = file
        .metadata()
        .map_err(|e| PlatformError::io("inspect private metadata", e))?
        .len();
    if size > max_bytes as u64 {
        return Err(PlatformError::io("read private metadata", invalid()));
    }
    let mut data = Vec::with_capacity(size as usize);
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|e| PlatformError::io("read private metadata", e))?;
    if data.len() > max_bytes {
        return Err(PlatformError::io("read private metadata", invalid()));
    }
    Ok(Some(data))
}

/// Atomically replace bounded metadata in a private directory. Writes/syncs a
/// create-new sibling first, then atomically renames it; never truncate the old
/// file. Failed pre-commit operations preserve the original. A directory-sync
/// failure after rename is an error with uncertain durability, not a rollback.
/// Concurrent writers are last-successful-rename-wins; callers serialize policy.
pub fn write_private_metadata_atomic(
    directory: impl AsRef<Path>,
    name: &str,
    data: &[u8],
    max_bytes: usize,
) -> Result<(), PlatformError> {
    validate(name, max_bytes)?;
    if data.len() > max_bytes {
        return Err(PlatformError::io("write private metadata", invalid()));
    }
    let directory = directory.as_ref();
    ensure_directory(directory)
        .map_err(|e| PlatformError::io("create private metadata directory", e))?;
    let root = backend::open_directory(directory)
        .map_err(|e| PlatformError::io("open private metadata directory", e))?
        .ok_or_else(|| PlatformError::io("open private metadata directory", invalid()))?;
    // Verify existing target before staging; never overwrite a symlink/special
    // file or a file whose privacy cannot be established.
    drop(
        backend::read(&root, directory, name)
            .map_err(|e| PlatformError::io("inspect private metadata target", e))?,
    );
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| PlatformError::io("name private metadata stage", invalid()))?
        .as_nanos();
    let stage = format!(
        ".made-metadata-{}-{stamp}-{}.tmp",
        std::process::id(),
        NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
    );
    let mut file = backend::create(&root, directory, &stage)
        .map_err(|e| PlatformError::io("create private metadata stage", e))?;
    let result = (|| {
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        // Recheck immediately before commit; rename replaces a swapped final
        // link itself, never follows it. Same-user malicious writers remain a
        // residual, as with other per-user private runtime state.
        drop(backend::read(&root, directory, name)?);
        backend::replace(&root, directory, &stage, name)?;
        #[cfg(unix)]
        root.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        backend::remove(&root, directory, &stage);
    }
    result.map_err(|e| PlatformError::io("commit private metadata", e))
}

fn ensure_directory(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(invalid()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                // Existing parents are trusted app-data ancestry, not modified.
                if !parent.exists() {
                    ensure_directory(parent)?;
                }
            }
            backend::create_directory(path)
        }
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
mod backend {
    use super::*;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    fn check(file: &File, directory: bool) -> io::Result<()> {
        let m = file.metadata()?;
        // SAFETY: geteuid has no arguments or pointer preconditions.
        if m.uid() != unsafe { libc::geteuid() }
            || if directory {
                !m.is_dir() || m.mode() & 0o777 != 0o700
            } else {
                !m.is_file() || m.mode() & 0o777 != 0o600
            }
        {
            return Err(invalid());
        }
        Ok(())
    }
    fn cstr(bytes: &[u8]) -> io::Result<CString> {
        CString::new(bytes).map_err(|_| invalid())
    }
    fn file(fd: i32, directory: bool) -> io::Result<Option<File>> {
        if fd < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(e)
            };
        }
        // SAFETY: fd was returned uniquely owned by open/openat on success.
        let file = unsafe { File::from_raw_fd(fd) };
        check(&file, directory)?;
        Ok(Some(file))
    }
    pub fn open_directory(path: &Path) -> io::Result<Option<File>> {
        let path = cstr(path.as_os_str().as_bytes())?;
        // SAFETY: valid NUL-terminated path; no-follow pins the final directory.
        file(
            unsafe {
                libc::open(
                    path.as_ptr(),
                    libc::O_RDONLY
                        | libc::O_CLOEXEC
                        | libc::O_NOFOLLOW
                        | libc::O_DIRECTORY
                        | libc::O_NONBLOCK,
                )
            },
            true,
        )
    }
    pub fn read(root: &File, _directory: &Path, name: &str) -> io::Result<Option<File>> {
        let name = cstr(name.as_bytes())?;
        // SAFETY: root is a live directory fd and name a validated single leaf.
        file(
            unsafe {
                libc::openat(
                    root.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
                )
            },
            false,
        )
    }
    pub fn create(root: &File, _directory: &Path, name: &str) -> io::Result<File> {
        let name = cstr(name.as_bytes())?;
        // SAFETY: live dirfd, single leaf, create-new prevents link following.
        file(
            unsafe {
                libc::openat(
                    root.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_CLOEXEC
                        | libc::O_NOFOLLOW,
                    0o600,
                )
            },
            false,
        )?
        .ok_or_else(invalid)
    }
    pub fn create_directory(path: &Path) -> io::Result<()> {
        std::fs::DirBuilder::new().mode(0o700).create(path)
    }
    pub fn replace(root: &File, _directory: &Path, from: &str, to: &str) -> io::Result<()> {
        let from = cstr(from.as_bytes())?;
        let to = cstr(to.as_bytes())?;
        // SAFETY: both names are single leaves relative to the pinned dirfd.
        if unsafe {
            libc::renameat(
                root.as_raw_fd(),
                from.as_ptr(),
                root.as_raw_fd(),
                to.as_ptr(),
            )
        } == 0
        {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    pub fn remove(root: &File, _directory: &Path, name: &str) {
        if let Ok(name) = cstr(name.as_bytes()) {
            // SAFETY: unlink only the owned stage leaf, without following links.
            unsafe {
                libc::unlinkat(root.as_raw_fd(), name.as_ptr(), 0);
            }
        }
    }
}

#[cfg(windows)]
mod backend {
    use super::*;
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    type Pointer = *mut c_void;
    #[repr(C)]
    struct Attributes {
        length: u32,
        descriptor: Pointer,
        inherit: i32,
    }
    #[repr(C)]
    struct Acl {
        _revision: u8,
        _reserved: u8,
        size: u16,
        _count: u16,
        _reserved2: u16,
    }
    #[link(name = "advapi32")]
    extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text: *const u16,
            revision: u32,
            descriptor: *mut Pointer,
            size: *mut u32,
        ) -> i32;
        fn GetSecurityDescriptorDacl(
            descriptor: Pointer,
            present: *mut i32,
            acl: *mut *mut Acl,
            defaulted: *mut i32,
        ) -> i32;
        fn GetSecurityDescriptorControl(
            descriptor: Pointer,
            control: *mut u16,
            revision: *mut u32,
        ) -> i32;
        fn GetSecurityInfo(
            handle: Pointer,
            kind: u32,
            information: u32,
            owner: *mut Pointer,
            group: *mut Pointer,
            dacl: *mut *mut Acl,
            sacl: *mut Pointer,
            descriptor: *mut Pointer,
        ) -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(
            path: *const u16,
            access: u32,
            sharing: u32,
            attributes: *const Attributes,
            creation: u32,
            flags: u32,
            template: Pointer,
        ) -> Pointer;
        fn CreateDirectoryW(path: *const u16, attributes: *const Attributes) -> i32;
        fn LocalFree(memory: Pointer) -> Pointer;
    }
    struct Descriptor(Pointer);
    impl Drop for Descriptor {
        fn drop(&mut self) {
            // SAFETY: these descriptors are LocalAlloc-owned per the Win32 APIs.
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    impl Descriptor {
        fn private() -> io::Result<Self> {
            let text: Vec<u16> = "D:P(A;;FA;;;OW)\0".encode_utf16().collect();
            let mut descriptor = std::ptr::null_mut();
            // SAFETY: valid terminated SDDL and live output storage, revision 1.
            if unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    text.as_ptr(),
                    1,
                    &mut descriptor,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(descriptor))
        }
        fn attributes(&self) -> Attributes {
            Attributes {
                length: std::mem::size_of::<Attributes>() as u32,
                descriptor: self.0,
                inherit: 0,
            }
        }
    }
    fn wide(path: &Path) -> io::Result<Vec<u16>> {
        let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(invalid());
        }
        value.push(0);
        Ok(value)
    }
    fn check(file: &File, directory: bool) -> io::Result<()> {
        let m = file.metadata()?;
        if m.file_attributes() & 0x400 != 0 || if directory { !m.is_dir() } else { !m.is_file() } {
            return Err(invalid());
        }
        let mut actual = std::ptr::null_mut();
        let mut actual_acl = std::ptr::null_mut();
        // SAFETY: live handle; request DACL plus its owning security descriptor.
        let result = unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                1,
                4,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut actual_acl,
                std::ptr::null_mut(),
                &mut actual,
            )
        };
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        let actual = Descriptor(actual);
        let expected = Descriptor::private()?;
        let mut expected_acl = std::ptr::null_mut();
        let mut present = 0;
        let mut defaulted = 0;
        let mut control = 0;
        let mut revision = 0;
        // SAFETY: descriptors come from successful system calls and remain live;
        // ACL pointers returned here stay valid until their descriptors drop.
        unsafe {
            if GetSecurityDescriptorControl(actual.0, &mut control, &mut revision) == 0
                || control & 0x1000 == 0
                || GetSecurityDescriptorDacl(
                    expected.0,
                    &mut present,
                    &mut expected_acl,
                    &mut defaulted,
                ) == 0
                || present == 0
                || actual_acl.is_null()
                || expected_acl.is_null()
            {
                return Err(invalid());
            }
            let actual_bytes =
                std::slice::from_raw_parts(actual_acl.cast::<u8>(), (*actual_acl).size as usize);
            let expected_bytes = std::slice::from_raw_parts(
                expected_acl.cast::<u8>(),
                (*expected_acl).size as usize,
            );
            if actual_bytes != expected_bytes {
                return Err(invalid());
            }
        }
        Ok(())
    }
    fn open(path: &Path, directory: bool, create: bool) -> io::Result<Option<File>> {
        let path = wide(path)?;
        let descriptor = Descriptor::private()?;
        let attributes = descriptor.attributes();
        let access = if directory {
            0x20080
        } else if create {
            0x40020000
        } else {
            0x80020000
        };
        // OPEN_REPARSE_POINT, plus BACKUP_SEMANTICS for directories. Excluding
        // FILE_SHARE_DELETE pins the directory against replacement while held.
        let flags = 0x00200000 | if directory { 0x02000000 } else { 0 };
        // SAFETY: terminated path and descriptor live across CreateFileW; handle
        // is uniquely transferred to File below on successful creation/open.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                access,
                if directory { 3 } else { 1 },
                &attributes,
                if create { 1 } else { 3 },
                flags,
                std::ptr::null_mut(),
            )
        };
        if handle as isize == -1 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(e)
            };
        }
        // SAFETY: CreateFileW returned this uniquely owned, valid handle.
        let file = unsafe { File::from_raw_handle(handle) };
        check(&file, directory)?;
        Ok(Some(file))
    }
    pub fn open_directory(path: &Path) -> io::Result<Option<File>> {
        open(path, true, false)
    }
    pub fn read(_root: &File, directory: &Path, name: &str) -> io::Result<Option<File>> {
        open(&directory.join(name), false, false)
    }
    pub fn create(_root: &File, directory: &Path, name: &str) -> io::Result<File> {
        open(&directory.join(name), false, true)?.ok_or_else(invalid)
    }
    pub fn create_directory(path: &Path) -> io::Result<()> {
        let path = wide(path)?;
        let descriptor = Descriptor::private()?;
        let attributes = descriptor.attributes();
        // SAFETY: terminated path and live protected DACL descriptor.
        if unsafe { CreateDirectoryW(path.as_ptr(), &attributes) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    pub fn replace(_root: &File, directory: &Path, from: &str, to: &str) -> io::Result<()> {
        std::fs::rename(directory.join(from), directory.join(to))
    }
    pub fn remove(_root: &File, directory: &Path, name: &str) {
        let _ = std::fs::remove_file(directory.join(name));
    }
}
