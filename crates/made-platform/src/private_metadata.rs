//! Atomic app-owned metadata, not a workspace file API. The caller supplies a
//! trusted app-data directory; its parent ancestry must be trusted. The final
//! private directory and files are pinned/checked, symlinks/reparse points and
//! nonregular files are refused. Unix uses dirfd-relative no-follow operations
//! and modes 0700/0600. Windows uses a protected owner-only DACL and a
//! non-delete-shared directory handle. The Windows check is semantic: it does
//! not compare canonicalized ACL bytes with the SDDL form. Metadata is
//! intentionally opaque: schema/version/corruption policy belongs to the
//! caller, never this capability.
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

/// Who an ACE grants or denies, relative to the object owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateAceTrustee {
    /// The object's owner SID.
    Owner,
    /// Well-known owner-rights SID. Windows may keep this after canonicalizing `OW`.
    OwnerRights,
    /// Any other trustee.
    Other,
}

/// One discretionary ACE, already classified. Layout and padding are not part of the check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivateAce {
    /// The ACE was inherited from a parent.
    pub inherited: bool,
    /// `ACCESS_ALLOWED` rather than deny, audit, or an object ACE.
    pub allow: bool,
    /// The mask includes full file access, or is `GENERIC_ALL`.
    pub full_access: bool,
    /// Trustee relative to the object owner.
    pub trustee: PrivateAceTrustee,
}

/// Semantic DACL view. Windows may canonicalize the bytes of an equivalent ACL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateDacl {
    /// `SE_DACL_PRESENT` and `SE_DACL_PROTECTED` are both set.
    pub protected: bool,
    /// The object owner is the current user.
    pub owner_is_current: bool,
    /// Explicit ACEs. A null DACL is not represented here; the caller rejects it.
    pub aces: Vec<PrivateAce>,
}

/// Why a DACL is not owner-only full access, or `None` when it is.
///
/// The string names the failed condition. Windows may canonicalize ACL layout
/// after the descriptor is written; this does not compare raw ACL bytes.
pub fn private_dacl_refusal(dacl: &PrivateDacl) -> Option<&'static str> {
    if !dacl.protected {
        return Some("DACL is not protected");
    }
    if !dacl.owner_is_current {
        return Some("owner is not the current user");
    }
    if dacl.aces.is_empty() {
        return Some("DACL has no ACE");
    }
    for ace in &dacl.aces {
        if ace.inherited {
            return Some("ACE is inherited");
        }
        if !matches!(
            ace.trustee,
            PrivateAceTrustee::Owner | PrivateAceTrustee::OwnerRights
        ) {
            return Some("ACE trustee is not the owner");
        }
        if !ace.allow {
            return Some("ACE is not an allow");
        }
        if !ace.full_access {
            return Some("ACE mask is not full access");
        }
    }
    None
}

/// Owner-only full access.
///
/// Accepts a protected DACL whose owner is the current user and whose every ACE
/// is an explicit allow of full access for that owner or for owner-rights.
/// Rejects an unprotected or inherited ACL, a partial mask, a deny/audit ACE,
/// and any trustee other than the owner.
#[must_use]
pub fn private_dacl_is_owner_only(dacl: &PrivateDacl) -> bool {
    private_dacl_refusal(dacl).is_none()
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
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS};
    use windows_sys::Win32::Security::Authorization::SetNamedSecurityInfoW;
    use windows_sys::Win32::Security::Authorization::SetSecurityInfo;
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        AclSizeInformation, AddAccessAllowedAce, CreateWellKnownSid, EqualSid, GetAce,
        GetAclInformation, GetLengthSid, GetSecurityDescriptorControl, GetTokenInformation,
        InitializeAcl, IsValidSid, TokenUser, WinCreatorOwnerRightsSid, ACCESS_ALLOWED_ACE, ACL,
        ACL_REVISION, ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION, INHERITED_ACE,
        OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSID,
        SECURITY_MAX_SID_SIZE, SE_DACL_PRESENT, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{FILE_ALL_ACCESS, FILE_ATTRIBUTE_REPARSE_POINT};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    type Pointer = *mut c_void;
    #[repr(C)]
    struct Attributes {
        length: u32,
        descriptor: Pointer,
        inherit: i32,
    }
    #[link(name = "advapi32")]
    extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text: *const u16,
            revision: u32,
            descriptor: *mut Pointer,
            size: *mut u32,
        ) -> i32;
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
        let metadata = file.metadata()?;
        // Reparse points are not regular files or plain directories, even when
        // the reparse tag would otherwise look like a file or directory.
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(refuse("path is a reparse point"));
        }
        if directory {
            if !metadata.is_dir() {
                return Err(refuse("path is not a directory"));
            }
        } else if !metadata.is_file() {
            return Err(refuse("path is not a regular file"));
        }
        let mut owner = std::ptr::null_mut();
        let mut dacl = std::ptr::null_mut();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: the handle is live. GetSecurityInfo allocates `descriptor`;
        // the owner and DACL pointers alias that allocation until LocalFree.
        let status = unsafe {
            GetSecurityInfo(
                file.as_raw_handle() as _,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let _descriptor = Descriptor(descriptor);
        let mut control = 0u16;
        let mut revision = 0u32;
        // SAFETY: `descriptor` is the allocation just returned above.
        if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // A null DACL grants everyone. That is not owner-only.
        if dacl.is_null() {
            return Err(refuse("DACL is null"));
        }
        let policy = PrivateDacl {
            protected: control & SE_DACL_PRESENT != 0 && control & SE_DACL_PROTECTED != 0,
            owner_is_current: owner_is_current_user(owner)?,
            aces: read_aces(dacl, owner)?,
        };
        match private_dacl_refusal(&policy) {
            None => Ok(()),
            Some(reason) => Err(refuse(reason)),
        }
    }

    fn refuse(reason: &str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidInput, reason)
    }

    #[repr(C, align(4))]
    struct SidBuf([u8; SECURITY_MAX_SID_SIZE as usize]);

    #[repr(C, align(4))]
    struct AclBuf([u8; 256]);

    fn owner_is_current_user(owner: PSID) -> io::Result<bool> {
        if owner.is_null() || unsafe { IsValidSid(owner) } == 0 {
            return Ok(false);
        }
        let user = current_user_sid()?;
        Ok(unsafe { EqualSid(owner, user.0.as_ptr().cast()) } != 0)
    }

    fn current_user_sid() -> io::Result<SidBuf> {
        let mut token = std::ptr::null_mut();
        // SAFETY: GetCurrentProcess is a pseudo-handle and must not be closed.
        // OpenProcessToken returns a new owned token.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Token(token);
        let mut needed = 0u32;
        unsafe {
            GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        }
        if needed == 0 || needed > 1024 {
            return Err(refuse("current user SID is unusable"));
        }
        let mut bytes = vec![0u8; needed as usize];
        // SAFETY: `bytes` is writable for `needed` bytes. The SID pointer stays
        // inside that buffer until it is copied below.
        let ok = unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                bytes.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: GetTokenInformation wrote a TOKEN_USER at the start of `bytes`.
        let user = unsafe { &*(bytes.as_ptr() as *const TOKEN_USER) };
        if user.User.Sid.is_null() || unsafe { IsValidSid(user.User.Sid) } == 0 {
            return Err(refuse("current user SID is unusable"));
        }
        let len = unsafe { GetLengthSid(user.User.Sid) } as usize;
        if len == 0 || len > SECURITY_MAX_SID_SIZE as usize {
            return Err(refuse("current user SID is unusable"));
        }
        let mut sid = SidBuf([0; SECURITY_MAX_SID_SIZE as usize]);
        // SAFETY: GetLengthSid reported `len` bytes, the source SID is valid,
        // and `sid` is aligned and at least that large.
        unsafe {
            std::ptr::copy_nonoverlapping(user.User.Sid.cast::<u8>(), sid.0.as_mut_ptr(), len);
        }
        Ok(sid)
    }

    /// Write the current user as owner and a protected DACL that allows only
    /// that user full access. An elevated token would otherwise leave
    /// `BUILTIN\Administrators` as owner of a `CreateDirectoryW` result.
    fn assign_owner_only(handle: windows_sys::Win32::Foundation::HANDLE) -> io::Result<()> {
        let user = current_user_sid()?;
        let mut acl = AclBuf([0; 256]);
        // SAFETY: the ACL buffer is 256 aligned bytes, larger than one allow
        // ACE for a maximum SID. The user SID remains live for the call.
        if unsafe { InitializeAcl(acl.0.as_mut_ptr().cast(), acl.0.len() as u32, ACL_REVISION) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        if unsafe {
            AddAccessAllowedAce(
                acl.0.as_mut_ptr().cast(),
                ACL_REVISION,
                FILE_ALL_ACCESS,
                user.0.as_ptr().cast(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let status = unsafe {
            SetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION
                    | DACL_SECURITY_INFORMATION
                    | PROTECTED_DACL_SECURITY_INFORMATION,
                user.0.as_ptr().cast(),
                std::ptr::null_mut(),
                acl.0.as_ptr().cast(),
                std::ptr::null(),
            )
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(status as i32))
        }
    }

    fn assign_owner_only_path(wide: &[u16]) -> io::Result<()> {
        let user = current_user_sid()?;
        let mut acl = AclBuf([0; 256]);
        // SAFETY: same owner-only ACL as `assign_owner_only`. The path is
        // NUL-terminated and the buffers stay live for SetNamedSecurityInfoW.
        if unsafe { InitializeAcl(acl.0.as_mut_ptr().cast(), acl.0.len() as u32, ACL_REVISION) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        if unsafe {
            AddAccessAllowedAce(
                acl.0.as_mut_ptr().cast(),
                ACL_REVISION,
                FILE_ALL_ACCESS,
                user.0.as_ptr().cast(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let status = unsafe {
            SetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION
                    | DACL_SECURITY_INFORMATION
                    | PROTECTED_DACL_SECURITY_INFORMATION,
                user.0.as_ptr().cast(),
                std::ptr::null_mut(),
                acl.0.as_ptr().cast(),
                std::ptr::null(),
            )
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(status as i32))
        }
    }

    fn read_aces(dacl: *mut ACL, owner: PSID) -> io::Result<Vec<PrivateAce>> {
        let mut info = ACL_SIZE_INFORMATION::default();
        // SAFETY: `dacl` is the non-null DACL from the live security descriptor.
        if unsafe {
            GetAclInformation(
                dacl,
                (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if info.AceCount > 64 {
            return Err(refuse("DACL has too many ACEs"));
        }
        let owner_rights = owner_rights_sid()?;
        let mut aces = Vec::with_capacity(info.AceCount as usize);
        for index in 0..info.AceCount {
            let mut ace_ptr = std::ptr::null_mut();
            // SAFETY: `index` is below the ACL's AceCount, so GetAce returns an
            // ACE inside `dacl` or fails.
            if unsafe { GetAce(dacl, index, &mut ace_ptr) } == 0 || ace_ptr.is_null() {
                return Err(refuse("ACE is unreadable"));
            }
            // SAFETY: every ACE begins with ACE_HEADER. GetAce returns that header.
            let header = unsafe { &*(ace_ptr as *const windows_sys::Win32::Security::ACE_HEADER) };
            let inherited = u32::from(header.AceFlags) & INHERITED_ACE != 0;
            if header.AceType != 0 || header.AceSize < 8 {
                aces.push(PrivateAce {
                    inherited,
                    allow: false,
                    full_access: false,
                    trustee: PrivateAceTrustee::Other,
                });
                continue;
            }
            let ace_len = header.AceSize as usize;
            // SAFETY: AceSize is the ACE length GetAce returned. The SID starts
            // at byte 8. SubAuthorityCount is the second SID byte; 15 is the
            // SID maximum, so the computed length cannot overflow the ACE
            // before IsValidSid/EqualSid read it.
            let bytes = unsafe { std::slice::from_raw_parts(ace_ptr.cast::<u8>(), ace_len) };
            if bytes.len() < 16 {
                return Err(refuse("ACE is unreadable"));
            }
            let sub_count = bytes[9] as usize;
            let sid_len = 8 + sub_count.saturating_mul(4);
            if sub_count > 15 || bytes.len() < 8 + sid_len {
                return Err(refuse("ACE is unreadable"));
            }
            let sid = unsafe { ace_ptr.cast::<u8>().add(8) as PSID };
            if unsafe { IsValidSid(sid) } == 0 {
                return Err(refuse("ACE is unreadable"));
            }
            let trustee = if !owner.is_null() && unsafe { EqualSid(sid, owner) } != 0 {
                PrivateAceTrustee::Owner
            } else if unsafe { EqualSid(sid, owner_rights.as_ptr() as PSID) } != 0 {
                PrivateAceTrustee::OwnerRights
            } else {
                PrivateAceTrustee::Other
            };
            // SAFETY: AceSize is at least 16, so the ACCESS_ALLOWED_ACE header
            // and mask are inside the ACE. The SID was bounded above.
            let mask = unsafe { (*(ace_ptr as *const ACCESS_ALLOWED_ACE)).Mask };
            let full_access = mask & FILE_ALL_ACCESS == FILE_ALL_ACCESS || mask == 0x1000_0000;
            aces.push(PrivateAce {
                inherited,
                allow: true,
                full_access,
                trustee,
            });
        }
        Ok(aces)
    }

    fn owner_rights_sid() -> io::Result<[u8; SECURITY_MAX_SID_SIZE as usize]> {
        let mut sid = [0u8; SECURITY_MAX_SID_SIZE as usize];
        let mut len = SECURITY_MAX_SID_SIZE;
        // SAFETY: the buffer is SECURITY_MAX_SID_SIZE. A null domain is the
        // documented input for WinCreatorOwnerRightsSid.
        if unsafe {
            CreateWellKnownSid(
                WinCreatorOwnerRightsSid,
                std::ptr::null_mut(),
                sid.as_mut_ptr().cast(),
                &mut len,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(sid)
    }

    struct Token(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            // SAFETY: the handle was returned by OpenProcessToken and is not
            // the process pseudo-handle.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn open(path: &Path, directory: bool, create: bool) -> io::Result<Option<File>> {
        let wide_path = wide(path)?;
        let descriptor = Descriptor::private()?;
        let attributes = descriptor.attributes();
        let access = if directory {
            0x20080
        } else if create {
            // GENERIC_WRITE | READ_CONTROL | WRITE_DAC | WRITE_OWNER. The extra
            // rights let this handle replace the elevated default owner.
            0x4002_0000 | 0x0004_0000 | 0x0008_0000
        } else {
            0x8002_0000
        };
        // OPEN_REPARSE_POINT, plus BACKUP_SEMANTICS for directories. Excluding
        // FILE_SHARE_DELETE pins the directory against replacement while held.
        let flags = 0x00200000 | if directory { 0x02000000 } else { 0 };
        // SAFETY: terminated path and descriptor live across CreateFileW; handle
        // is uniquely transferred to File below on successful creation/open.
        let handle = unsafe {
            CreateFileW(
                wide_path.as_ptr(),
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
        if create {
            if let Err(error) = assign_owner_only(file.as_raw_handle() as _) {
                drop(file);
                let _ = std::fs::remove_file(path);
                return Err(error);
            }
        }
        if let Err(error) = check(&file, directory) {
            if create {
                drop(file);
                let _ = std::fs::remove_file(path);
            }
            return Err(error);
        }
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
        let wide_path = wide(path)?;
        let descriptor = Descriptor::private()?;
        let attributes = descriptor.attributes();
        // SAFETY: terminated path and live protected DACL descriptor.
        // CreateDirectoryW on an elevated token still records Administrators as
        // owner; assign_owner_only_path replaces that before the check.
        if unsafe { CreateDirectoryW(wide_path.as_ptr(), &attributes) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if let Err(error) = assign_owner_only_path(&wide_path) {
            let _ = std::fs::remove_dir(path);
            return Err(error);
        }
        Ok(())
    }
    pub fn replace(_root: &File, directory: &Path, from: &str, to: &str) -> io::Result<()> {
        std::fs::rename(directory.join(from), directory.join(to))
    }
    pub fn remove(_root: &File, directory: &Path, name: &str) {
        let _ = std::fs::remove_file(directory.join(name));
    }
}
