use crate::{domain::contracts::*, git::error::error};
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};
pub struct Database {
    connection: Mutex<Connection>,
    _ownership: std::fs::File,
}
impl Database {
    pub fn open(directory: &Path) -> Result<Self, AppError> {
        std::fs::create_dir_all(directory).map_err(|_| db_error())?;
        let mut ownership = std::fs::OpenOptions::new();
        ownership
            .read(true)
            .write(true)
            .create(true)
            .truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            ownership.share_mode(0);
        }
        let ownership = ownership
            .open(directory.join("workspace.owner"))
            .map_err(|_| db_error())?;
        let mut connection =
            Connection::open(directory.join("workspace.sqlite3")).map_err(|_| db_error())?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| db_error())?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|_| db_error())?;
        super::migrations::migrate(&mut connection).map_err(|_| db_error())?;
        Ok(Self {
            connection: Mutex::new(connection),
            _ownership: ownership,
        })
    }
    pub fn lock(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.connection.lock().map_err(|_| db_error())
    }
}
pub fn db_error() -> AppError {
    error(ErrorCode::Database, Phase::Registration)
}
#[cfg(windows)]
pub fn encode(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}
#[cfg(windows)]
pub fn decode(bytes: Vec<u8>) -> Result<PathBuf, AppError> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    if !bytes.len().is_multiple_of(2) {
        return Err(db_error());
    }
    Ok(PathBuf::from(OsString::from_wide(
        &bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect::<Vec<_>>(),
    )))
}
#[cfg(unix)]
pub fn encode(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}
#[cfg(unix)]
pub fn decode(bytes: Vec<u8>) -> Result<PathBuf, AppError> {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    Ok(OsString::from_vec(bytes).into())
}
