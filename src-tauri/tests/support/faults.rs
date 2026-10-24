use super::{FixtureError, OwnedFixture, SYNTHETIC_SECRET};
use opengit_lib::git::runner::GitRunner;
use std::{ffi::OsString, fs, path::Path, time::Duration};

pub struct ReadOnlyFile {
    path: std::path::PathBuf,
    original: fs::Permissions,
}
impl ReadOnlyFile {
    pub fn install(fixture: &OwnedFixture, path: &Path) -> Result<Self, FixtureError> {
        fixture.require_owned_file(path)?;
        let original = fs::metadata(path)?.permissions();
        let mut readonly = original.clone();
        readonly.set_readonly(true);
        fs::set_permissions(path, readonly)?;
        Ok(Self {
            path: path.into(),
            original,
        })
    }
}
impl Drop for ReadOnlyFile {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, self.original.clone());
    }
}

pub fn refuse_journal_updates(fixture: &OwnedFixture, path: &Path) -> Result<(), FixtureError> {
    fixture.require_owned_file(path)?;
    rusqlite::Connection::open(path).and_then(|c|c.execute_batch("CREATE TRIGGER fixture_refuse_journal BEFORE UPDATE ON operations BEGIN SELECT RAISE(ABORT,'fixture journal failure'); END;")).map_err(|_|FixtureError::Refused("journal fault installation failed"))
}

#[cfg(windows)]
pub enum ProcessFault {
    Failure,
    Timeout,
    OutputLimit,
}
#[cfg(windows)]
pub fn process_fault(
    fixture: &OwnedFixture,
    fault: ProcessFault,
) -> Result<(GitRunner, Vec<OsString>), FixtureError> {
    // Fixed test scripts only. The production runner still launches an absolute
    // trusted executable with argv; it receives no generic test IPC endpoint.
    let (name, script) = match fault {
        ProcessFault::Failure => (
            "failure.ps1",
            format!(
                "[Console]::Error.WriteLine('{SYNTHETIC_SECRET} authentication failed'); exit 17"
            ),
        ),
        ProcessFault::Timeout => ("timeout.ps1", "Start-Sleep -Seconds 60".into()),
        ProcessFault::OutputLimit => ("output.ps1", "[Console]::Out.Write('x' * 16384)".into()),
    };
    let path = fixture.child(name)?;
    fs::write(&path, script)?;
    let executable = std::path::PathBuf::from(
        std::env::var_os("SystemRoot").ok_or(FixtureError::Refused("Windows root unavailable"))?,
    )
    .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut runner = GitRunner::trusted(executable)
        .map_err(|_| FixtureError::Refused("fixture executable unavailable"))?;
    runner.timeout = Duration::from_secs(3);
    if matches!(fault, ProcessFault::OutputLimit) {
        runner.output_limit = 1024;
    }
    let args = vec![
        "-NoProfile".into(),
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-File".into(),
        path.into_os_string(),
    ];
    Ok((runner, args))
}

#[cfg(windows)]
pub fn junction(
    fixture: &OwnedFixture,
    name: &str,
    target: &OwnedFixture,
) -> Result<std::path::PathBuf, FixtureError> {
    use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::{
        Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT},
        System::IO::DeviceIoControl,
    };
    fixture.guard(fixture.root())?;
    target.guard(target.root())?;
    let path = fixture.child(name)?;
    fs::create_dir(&path)?;
    let print = target.root().as_os_str().to_string_lossy();
    let print = print.strip_prefix(r"\\?\").unwrap_or(&print);
    if !print.as_bytes().get(1).is_some_and(|b| *b == b':') {
        return Err(FixtureError::Refused(
            "junction fixture requires a local volume",
        ));
    }
    let substitute = std::ffi::OsString::from(format!(r"\??\{print}"));
    let substitute: Vec<u16> = substitute.encode_wide().collect();
    let print: Vec<u16> = std::ffi::OsStr::new(print).encode_wide().collect();
    let sub_len = u16::try_from(substitute.len() * 2)
        .map_err(|_| FixtureError::Refused("junction target too long"))?;
    let print_len = u16::try_from(print.len() * 2)
        .map_err(|_| FixtureError::Refused("junction target too long"))?;
    let data_len = 12usize + usize::from(sub_len) + usize::from(print_len);
    let data_len =
        u16::try_from(data_len).map_err(|_| FixtureError::Refused("junction target too long"))?;
    let mut buffer = Vec::new();
    buffer.extend_from_slice(&0xa0000003u32.to_le_bytes());
    buffer.extend_from_slice(&data_len.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    for field in [0, sub_len, sub_len + 2, print_len] {
        buffer.extend_from_slice(&field.to_le_bytes());
    }
    for value in substitute.into_iter().chain([0]).chain(print).chain([0]) {
        buffer.extend_from_slice(&value.to_le_bytes());
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(&path)?;
    let mut returned = 0;
    // FSCTL_SET_REPARSE_POINT, documented mount-point reparse buffer. This is
    // fixture creation only; no command interpreter or recursive shell action.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle() as _,
            0x000900a4,
            buffer.as_ptr() as _,
            buffer.len() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(FixtureError::Io(std::io::Error::last_os_error()));
    }
    Ok(path)
}
