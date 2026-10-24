use crate::{domain::contracts::*, git::error::error, repositories::discovery::identity};
use std::{
    fs::{self, File, OpenOptions},
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
};
use windows_sys::Win32::Storage::FileSystem::*;
pub struct StableDirectory {
    pub path: PathBuf,
    pub identity: Vec<u8>,
    handles: Vec<File>,
    parent: Option<std::sync::Arc<StableDirectory>>,
}
impl StableDirectory {
    pub fn open(path: &Path) -> Result<Self, AppError> {
        let path =
            fs::canonicalize(path).map_err(|_| error(ErrorCode::UnsafePath, Phase::Validation))?;
        let mut handles = Vec::new();
        for ancestor in path.ancestors() {
            let metadata = fs::symlink_metadata(ancestor).map_err(|_| unsafe_path())?;
            if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err(unsafe_path());
            }
            let handle = OpenOptions::new()
                .access_mode(FILE_READ_ATTRIBUTES | FILE_TRAVERSE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .open(ancestor)
                .map_err(|_| unsafe_path())?;
            handles.push(handle);
        }
        let identity = identity(&path)?;
        Ok(Self {
            path,
            identity,
            handles,
            parent: None,
        })
    }
    pub fn pin_child(parent: std::sync::Arc<Self>, leaf: &str) -> Result<Self, AppError> {
        Self::pin_child_access(parent, leaf, true)
    }
    pub fn pin_child_read(parent: std::sync::Arc<Self>, leaf: &str) -> Result<Self, AppError> {
        Self::pin_child_access(parent, leaf, false)
    }
    fn pin_child_access(
        parent: std::sync::Arc<Self>,
        leaf: &str,
        delete: bool,
    ) -> Result<Self, AppError> {
        super::request::directory_name(leaf)?;
        parent.recheck()?;
        let path = parent.path.join(leaf);
        let handle = OpenOptions::new()
            .access_mode(
                FILE_READ_ATTRIBUTES
                    | FILE_LIST_DIRECTORY
                    | FILE_TRAVERSE
                    | if delete { 0x10000 } else { 0 },
            )
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .share_mode(FILE_SHARE_READ | if delete { 0 } else { FILE_SHARE_WRITE })
            .open(&path)
            .map_err(|_| unsafe_path())?;
        let info = handle_info(&handle)?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
            || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(unsafe_path());
        }
        if !same_path(&final_path(&handle)?, &path) {
            return Err(error(ErrorCode::ExternalChange, Phase::Verification));
        }
        let identity = info_identity(&info);
        if crate::repositories::discovery::identity(&path)? != identity {
            return Err(error(ErrorCode::ExternalChange, Phase::Verification));
        }
        Ok(Self {
            path,
            identity,
            handles: vec![handle],
            parent: Some(parent),
        })
    }
    pub fn reanchor(&mut self, parent: std::sync::Arc<Self>, leaf: &str) -> Result<(), AppError> {
        let path = parent.path.join(leaf);
        if !same_path(&final_path(self.handle())?, &path) {
            return Err(error(ErrorCode::ExternalChange, Phase::Recovery));
        }
        self.path = path;
        self.parent = Some(parent);
        Ok(())
    }
    pub fn recheck(&self) -> Result<(), AppError> {
        if let Some(parent) = &self.parent {
            parent.recheck()?;
        }
        let info = handle_info(self.handle())?;
        if info_identity(&info) != self.identity
            || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(error(ErrorCode::ExternalChange, Phase::Verification));
        }
        if identity(&self.path)? != self.identity {
            return Err(error(ErrorCode::ExternalChange, Phase::Verification));
        }
        for ancestor in self.path.ancestors() {
            if fs::symlink_metadata(ancestor)
                .map_err(|_| unsafe_path())?
                .file_attributes()
                & FILE_ATTRIBUTE_REPARSE_POINT
                != 0
            {
                return Err(unsafe_path());
            }
        }
        // The no-delete-sharing handles remain live across every boundary.
        if self.handles.is_empty() {
            return Err(unsafe_path());
        }
        Ok(())
    }
    pub fn handle(&self) -> &File {
        &self.handles[0]
    }
}
pub(crate) fn handle_info(handle: &File) -> Result<BY_HANDLE_FILE_INFORMATION, AppError> {
    let mut info = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle() as _, &mut info) } == 0 {
        return Err(unsafe_path());
    }
    Ok(info)
}
pub(crate) fn info_identity(info: &BY_HANDLE_FILE_INFORMATION) -> Vec<u8> {
    [
        info.dwVolumeSerialNumber.to_le_bytes(),
        info.nFileIndexHigh.to_le_bytes(),
        info.nFileIndexLow.to_le_bytes(),
    ]
    .concat()
}
pub(crate) fn final_path(handle: &File) -> Result<PathBuf, AppError> {
    use std::os::windows::ffi::OsStringExt;
    let mut buffer = vec![0u16; 32768];
    let count = unsafe {
        GetFinalPathNameByHandleW(
            handle.as_raw_handle() as _,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            0,
        )
    } as usize;
    if count == 0 || count >= buffer.len() {
        return Err(unsafe_path());
    }
    Ok(std::ffi::OsString::from_wide(&buffer[..count]).into())
}
fn same_path(a: &Path, b: &Path) -> bool {
    let a: Vec<u16> = a.as_os_str().encode_wide().collect();
    let b: Vec<u16> = b.as_os_str().encode_wide().collect();
    unsafe {
        windows_sys::Win32::Globalization::CompareStringOrdinal(
            a.as_ptr(),
            a.len() as i32,
            b.as_ptr(),
            b.len() as i32,
            1,
        ) == 2
    }
}
pub(crate) fn delete_empty_object(handle: &File) -> Result<(), AppError> {
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    if unsafe {
        SetFileInformationByHandle(
            handle.as_raw_handle() as _,
            FileDispositionInfo,
            &info as *const _ as _,
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    } == 0
    {
        return Err(error(ErrorCode::Io, Phase::Recovery));
    }
    Ok(())
}
#[derive(Debug, PartialEq, Eq)]
pub enum PublishOutcome {
    Committed,
    Uncertain,
}
pub fn publish_no_replace(
    source: &Path,
    expected: &[u8],
    parent: &StableDirectory,
    leaf: &str,
) -> Result<PublishOutcome, AppError> {
    super::request::directory_name(leaf)?;
    parent.recheck()?;
    let handle = OpenOptions::new()
        .access_mode(0x00010000 | FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(source)
        .map_err(|_| error(ErrorCode::PermissionDenied, Phase::Publish))?;
    if fs::symlink_metadata(source)
        .map_err(|_| unsafe_path())?
        .file_attributes()
        & FILE_ATTRIBUTE_REPARSE_POINT
        != 0
        || identity(source)? != expected
    {
        return Err(error(ErrorCode::ExternalChange, Phase::Publish));
    }
    // Revalidate the opened source handle identity, not just the path that can
    // change after it was opened. The rename operates on this exact directory.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle() as _, &mut info) } == 0 {
        return Err(unsafe_path());
    }
    let actual = [
        info.dwVolumeSerialNumber.to_le_bytes(),
        info.nFileIndexHigh.to_le_bytes(),
        info.nFileIndexLow.to_le_bytes(),
    ]
    .concat();
    if actual != expected {
        return Err(error(ErrorCode::ExternalChange, Phase::Publish));
    }
    publish_handle(&handle, expected, parent, leaf)
}
pub(crate) fn publish_pinned(
    source: &StableDirectory,
    expected: &[u8],
    parent: &StableDirectory,
    leaf: &str,
) -> Result<PublishOutcome, AppError> {
    source.recheck()?;
    if source.identity != expected {
        return Err(error(ErrorCode::ExternalChange, Phase::Publish));
    }
    publish_handle(source.handle(), expected, parent, leaf)
}
fn publish_handle(
    handle: &File,
    expected: &[u8],
    parent: &StableDirectory,
    leaf: &str,
) -> Result<PublishOutcome, AppError> {
    super::request::directory_name(leaf)?;
    parent.recheck()?;
    rename_absolute(handle, parent, leaf).map_err(|failure| {
        let code = failure.raw_os_error();
        error(
            if matches!(code, Some(80 | 183))
                || fs::symlink_metadata(parent.path.join(leaf)).is_ok()
            {
                ErrorCode::DestinationExists
            } else if code == Some(112) {
                ErrorCode::DiskFull
            } else {
                ErrorCode::PermissionDenied
            },
            Phase::Publish,
        )
    })?;
    // OS success alone is not publication evidence. Preserve the intent and
    // all data if the exact target cannot be proven to be the opened source.
    let target = parent.path.join(leaf);
    if fs::symlink_metadata(&target).is_ok_and(|metadata| {
        metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }) && identity(&target).as_deref() == Ok(expected)
    {
        Ok(PublishOutcome::Committed)
    } else {
        Ok(PublishOutcome::Uncertain)
    }
}
struct RenameBuffer {
    storage: Vec<usize>,
    length: u32,
}
impl RenameBuffer {
    fn new(name: &[u16]) -> std::io::Result<Self> {
        let invalid = || {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid rename buffer size",
            )
        };
        let bytes = name.len().checked_mul(2).ok_or_else(invalid)?;
        let name_bytes = u32::try_from(bytes).map_err(|_| invalid())?;
        let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
        let length = offset
            .checked_add(bytes)
            .and_then(|size| size.checked_add(2))
            .ok_or_else(invalid)?
            .max(std::mem::size_of::<FILE_RENAME_INFO>());
        let length = u32::try_from(length).map_err(|_| invalid())?;
        let mut storage = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
        unsafe {
            let rename = storage.as_mut_ptr() as *mut FILE_RENAME_INFO;
            (*rename).Anonymous.ReplaceIfExists = false;
            (*rename).RootDirectory = std::ptr::null_mut();
            (*rename).FileNameLength = name_bytes;
            let tail = std::ptr::addr_of_mut!((*rename).FileName).cast::<u16>();
            std::ptr::copy_nonoverlapping(name.as_ptr(), tail, name.len());
            tail.add(name.len()).write(0);
        }
        Ok(Self { storage, length })
    }
}
fn rename_absolute(handle: &File, parent: &StableDirectory, leaf: &str) -> std::io::Result<()> {
    let name: Vec<u16> = parent.path.join(leaf).as_os_str().encode_wide().collect();
    let buffer = RenameBuffer::new(&name)?;
    rename_buffer(handle, &buffer)
}
fn rename_buffer(handle: &File, buffer: &RenameBuffer) -> std::io::Result<()> {
    unsafe {
        if SetFileInformationByHandle(
            handle.as_raw_handle() as _,
            FileRenameInfo,
            buffer.storage.as_ptr() as _,
            buffer.length,
        ) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}
pub fn unsafe_path() -> AppError {
    error(ErrorCode::UnsafePath, Phase::Validation)
}
/// Preserve UTF-16 while removing the Win32 extended prefix Git's argv path
/// parser does not accept as a local repository source. Identity storage keeps
/// the original canonical OS path; no display string is ever executed.
pub fn git_path(path: &Path) -> Result<std::ffi::OsString, AppError> {
    use std::os::windows::ffi::OsStringExt;
    let mut units: Vec<u16> = path.as_os_str().encode_wide().collect();
    let prefix: Vec<u16> = r"\\?\".encode_utf16().collect();
    if units.starts_with(&prefix) {
        units.drain(..4);
        if units.starts_with(&[85, 78, 67, 92]) {
            units.splice(..4, [92, 92]);
        }
    }
    let drive = units.len() >= 3
        && (units[0] as u8).is_ascii_alphabetic()
        && units[0] <= 127
        && units[1] == 58
        && units[2] == 92;
    let unc = units.starts_with(&[92, 92])
        && !units.starts_with(&[92, 92, 46, 92])
        && !units.starts_with(&[92, 92, 63, 92]);
    if !drive && !unc {
        return Err(unsafe_path());
    }
    let converted = std::ffi::OsString::from_wide(&units);
    let converted_path = Path::new(&converted);
    for component in converted_path.components() {
        if let std::path::Component::Normal(value) = component {
            if value
                .encode_wide()
                .last()
                .is_some_and(|c| c == 32 || c == 46)
            {
                return Err(unsafe_path());
            }
        }
    }
    if identity(path)? != identity(converted_path).map_err(|_| unsafe_path())? {
        return Err(unsafe_path());
    }
    Ok(converted)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absolute_handle_rename_probe() {
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("source");
        fs::create_dir(&source).unwrap();
        let parent = StableDirectory::open(fixture.path()).unwrap();
        let handle = OpenOptions::new()
            .access_mode(0x10000 | FILE_READ_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(&source)
            .unwrap();
        let result = rename_absolute(&handle, &parent, "destination");
        assert!(
            result.is_ok(),
            "documented absolute rename OS error {:?}",
            result.err().and_then(|e| e.raw_os_error())
        );
    }
    #[test]
    fn explicit_terminator_matrix_preserves_exact_names_identities_and_collision_data() {
        fn entries(path: &Path) -> std::collections::BTreeSet<std::ffi::OsString> {
            fs::read_dir(path)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect()
        }
        let mut cases = 0;
        for variant in [
            "plain".to_string(),
            "space name".into(),
            "Ω name".into(),
            "supplementary 😀".into(),
            format!("long {}", "a".repeat(160)),
        ] {
            for residue in 0..4 {
                for collision in 0..3 {
                    let fixture = tempfile::tempdir().unwrap();
                    let parent = StableDirectory::open(fixture.path()).unwrap();
                    let mut leaf = variant.clone();
                    while parent.path.join(&leaf).as_os_str().encode_wide().count() % 4 != residue {
                        leaf.push('a');
                    }
                    super::super::request::directory_name(&leaf).unwrap();
                    let source = parent.path.join("source");
                    fs::create_dir(&source).unwrap();
                    fs::write(source.join("payload"), b"source payload").unwrap();
                    let source_id = identity(&source).unwrap();
                    let target = parent.path.join(&leaf);
                    if collision == 1 {
                        fs::create_dir(&target).unwrap();
                        fs::write(target.join("sentinel"), b"destination sentinel").unwrap();
                    } else if collision == 2 {
                        fs::write(&target, b"destination sentinel").unwrap();
                    }
                    let target_id = if collision == 0 {
                        None
                    } else {
                        Some(identity(&target).unwrap())
                    };
                    let before = entries(&parent.path);
                    let handle = OpenOptions::new()
                        .access_mode(0x10000 | FILE_READ_ATTRIBUTES)
                        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                        .open(&source)
                        .unwrap();
                    let name: Vec<u16> = target.as_os_str().encode_wide().collect();
                    let mut buffer = RenameBuffer::new(&name).unwrap();
                    let guard_start = buffer.storage.len();
                    buffer
                        .storage
                        .extend_from_slice(&[0xa5a5a5a5a5a5a5a5usize; 2]);
                    unsafe {
                        let header = buffer.storage.as_ptr() as *const FILE_RENAME_INFO;
                        assert!(!(*header).Anonymous.ReplaceIfExists);
                        assert!((*header).RootDirectory.is_null());
                        assert_eq!((*header).FileNameLength as usize, name.len() * 2);
                        assert_eq!(
                            std::ptr::addr_of!((*header).FileName)
                                .cast::<u16>()
                                .add(name.len())
                                .read(),
                            0
                        );
                        assert!(buffer.length as usize >= std::mem::size_of::<FILE_RENAME_INFO>());
                        assert!(
                            buffer.length as usize
                                >= std::mem::offset_of!(FILE_RENAME_INFO, FileName)
                                    + name.len() * 2
                                    + 2
                        );
                    }
                    let result = rename_buffer(&handle, &buffer);
                    assert_eq!(
                        &buffer.storage[guard_start..],
                        &[0xa5a5a5a5a5a5a5a5usize; 2]
                    );
                    if collision == 0 {
                        assert!(
                            result.is_ok(),
                            "matrix absent case {cases}: {:?}",
                            result.err().and_then(|error| error.raw_os_error())
                        );
                        assert!(!source.exists());
                        assert_eq!(identity(&target).unwrap(), source_id);
                        assert_eq!(fs::read(target.join("payload")).unwrap(), b"source payload");
                        assert_eq!(
                            entries(&parent.path),
                            [std::ffi::OsString::from(&leaf)].into_iter().collect()
                        );
                    } else {
                        if result.is_ok() {
                            let mut path = vec![0u16; 32768];
                            let count = unsafe {
                                GetFinalPathNameByHandleW(
                                    handle.as_raw_handle() as _,
                                    path.as_mut_ptr(),
                                    path.len() as u32,
                                    0,
                                )
                            };
                            eprintln!(
                                "unexpected matrix source final handle path: {:?}",
                                String::from_utf16_lossy(&path[..(count as usize).min(path.len())])
                            );
                        }
                        assert!(
                            result.is_err(),
                            "matrix collision case {cases} unexpectedly succeeded"
                        );
                        assert_eq!(identity(&source).unwrap(), source_id);
                        assert_eq!(fs::read(source.join("payload")).unwrap(), b"source payload");
                        assert_eq!(identity(&target).unwrap(), target_id.unwrap());
                        assert_eq!(
                            fs::read(if collision == 1 {
                                target.join("sentinel")
                            } else {
                                target.clone()
                            })
                            .unwrap(),
                            b"destination sentinel"
                        );
                        assert_eq!(entries(&parent.path), before);
                    }
                    cases += 1;
                }
            }
        }
        assert_eq!(cases, 60);
    }
}
