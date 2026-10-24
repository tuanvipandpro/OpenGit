use crate::{
    domain::contracts::*,
    git::{
        error::error,
        runner::{argv, Cancellation, GitRunner},
    },
};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug)]
pub struct Discovered {
    pub common: PathBuf,
    pub common_identity: Vec<u8>,
    pub root: Option<PathBuf>,
    pub root_identity: Option<Vec<u8>>,
    pub git_dir: PathBuf,
    pub bare: bool,
}
pub async fn discover(runner: &GitRunner, path: &Path) -> Result<Discovered, AppError> {
    crate::git::version::ensure_supported(runner).await?;
    let path = std::fs::canonicalize(path)
        .map_err(|_| error(ErrorCode::RepositoryUnavailable, Phase::Discovery))?;
    let bare = query(runner, &path, &["rev-parse", "--is-bare-repository"]).await? == "true";
    let common = std::fs::canonicalize(
        query(
            runner,
            &path,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .await?,
    )
    .map_err(|_| error(ErrorCode::NotRepository, Phase::Discovery))?;
    let git_dir =
        std::fs::canonicalize(query(runner, &path, &["rev-parse", "--absolute-git-dir"]).await?)
            .map_err(|_| error(ErrorCode::NotRepository, Phase::Discovery))?;
    let root = if bare {
        None
    } else {
        Some(
            std::fs::canonicalize(query(runner, &path, &["rev-parse", "--show-toplevel"]).await?)
                .map_err(|_| error(ErrorCode::NotRepository, Phase::Discovery))?,
        )
    };
    Ok(Discovered {
        common_identity: identity(&common)?,
        root_identity: root.as_ref().map(|p| identity(p)).transpose()?,
        common,
        root,
        git_dir,
        bare,
    })
}
pub async fn query(runner: &GitRunner, cwd: &Path, args: &[&str]) -> Result<String, AppError> {
    let bytes = runner
        .run(
            Some(cwd),
            &argv(args),
            &Cancellation::default(),
            Phase::Discovery,
        )
        .await?;
    let text = String::from_utf8(bytes)
        .map_err(|_| error(ErrorCode::MalformedOutput, Phase::Discovery))?;
    let line = text.strip_suffix('\n').unwrap_or(&text);
    Ok(line.strip_suffix('\r').unwrap_or(line).to_string())
}
#[cfg(windows)]
pub fn identity(path: &Path) -> Result<Vec<u8>, AppError> {
    use std::{
        mem::zeroed,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    };
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|_| error(ErrorCode::RepositoryUnavailable, Phase::Discovery))?;
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) } == 0 {
        return Err(error(ErrorCode::RepositoryUnavailable, Phase::Discovery));
    }
    Ok([
        information.dwVolumeSerialNumber.to_le_bytes(),
        information.nFileIndexHigh.to_le_bytes(),
        information.nFileIndexLow.to_le_bytes(),
    ]
    .concat())
}
#[cfg(unix)]
pub fn identity(path: &Path) -> Result<Vec<u8>, AppError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path)
        .map_err(|_| error(ErrorCode::RepositoryUnavailable, Phase::Discovery))?;
    Ok([metadata.dev().to_le_bytes(), metadata.ino().to_le_bytes()].concat())
}
