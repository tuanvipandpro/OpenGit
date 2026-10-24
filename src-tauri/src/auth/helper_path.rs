use super::prompts;
use crate::domain::contracts::AppError;
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
};
use windows_sys::Win32::Storage::FileSystem::*;

/// Contains only path capabilities, never a broker, database or credential.
pub(super) struct HelperPaths {
    pub git: OsString,
    pub ssh: Option<OsString>,
    _handles: Vec<File>,
}
pub(super) fn select(path: &Path) -> Result<HelperPaths, AppError> {
    select_with(path, short_path)
}
fn select_with(
    path: &Path,
    resolver: impl FnOnce(&Path) -> std::io::Result<PathBuf>,
) -> Result<HelperPaths, AppError> {
    if !path.is_absolute() {
        return Err(prompts::invalid());
    }
    let mut handles = pin_chain(path)?;
    let original = information(&handles[0])?;
    let ssh = if ascii(path) {
        Some(path.as_os_str().to_owned())
    } else {
        resolver(path).ok().and_then(|alias| {
            if !alias.is_absolute() || !ascii(&alias) {
                return None;
            }
            let mut alias_handles = pin_chain(&alias).ok()?;
            if information(&alias_handles[0]).ok()? != original {
                return None;
            }
            handles.append(&mut alias_handles);
            Some(alias.into_os_string())
        })
    };
    Ok(HelperPaths {
        git: path.as_os_str().to_owned(),
        ssh,
        _handles: handles,
    })
}
fn ascii(path: &Path) -> bool {
    path.as_os_str()
        .encode_wide()
        .all(|unit| (1..128).contains(&unit))
}
fn information(file: &File) -> Result<[u32; 3], AppError> {
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
        return Err(prompts::invalid());
    }
    Ok([
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
    ])
}
fn pin_chain(path: &Path) -> Result<Vec<File>, AppError> {
    let expected = information(
        &OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|_| prompts::invalid())?,
    )?;
    let mut handles = Vec::new();
    // Acquire ancestor guards from the root before opening the leaf. A path
    // substitution during acquisition must still match the original snapshot.
    let chain: Vec<_> = path.ancestors().enumerate().collect();
    for (index, part) in chain.into_iter().rev() {
        let handle = OpenOptions::new()
            .access_mode(
                FILE_READ_ATTRIBUTES
                    | if index == 0 {
                        FILE_READ_DATA
                    } else {
                        FILE_LIST_DIRECTORY | FILE_TRAVERSE
                    },
            )
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .share_mode(if index == 0 {
                FILE_SHARE_READ
            } else {
                FILE_SHARE_READ | FILE_SHARE_WRITE
            })
            .open(part)
            .map_err(|_| prompts::invalid())?;
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(handle.as_raw_handle() as _, &mut info) } == 0
            || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != (index != 0)
        {
            return Err(prompts::invalid());
        }
        handles.push(handle);
        #[cfg(test)]
        AFTER_FIRST_PIN.with(|hook| {
            if let Some(hook) = hook.borrow_mut().take() {
                hook(part);
            }
        });
    }
    handles.rotate_right(1);
    if information(&handles[0])? != expected {
        return Err(prompts::invalid());
    }
    Ok(handles)
}
#[cfg(test)]
type PinHook = Box<dyn FnOnce(&Path)>;
#[cfg(test)]
thread_local! {static AFTER_FIRST_PIN:std::cell::RefCell<Option<PinHook>> = const {std::cell::RefCell::new(None)};}
fn short_path(path: &Path) -> std::io::Result<PathBuf> {
    let input: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut buffer = vec![0u16; 256];
    for _ in 0..4 {
        let length =
            unsafe { GetShortPathNameW(input.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) }
                as usize;
        if length == 0 {
            return Err(std::io::Error::last_os_error());
        }
        if length < buffer.len() {
            return Ok(OsString::from_wide(&buffer[..length]).into());
        }
        if length > 32768 {
            break;
        }
        buffer.resize(length + 1, 0);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "Helper alias unavailable",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn no_alias_disables_only_ssh_prompt_and_keeps_noninteractive_openssh_usable() {
        use crate::{
            auth::AuthBroker,
            domain::contracts::*,
            git::runner::{Cancellation, GitRunner},
            operations::scheduler::{OperationSpec, Scheduler},
            persistence::workspace::Database,
        };
        let fixture = tempfile::tempdir().unwrap();
        let original = fixture.path().join("helper Ω.exe");
        std::fs::write(&original, b"unused helper").unwrap();
        let paths = std::sync::Arc::new(
            select_with(&original, |_| Err(std::io::Error::from_raw_os_error(2))).unwrap(),
        );
        let scheduler = std::sync::Arc::new(
            Scheduler::new(std::sync::Arc::new(
                Database::open(&fixture.path().join("data")).unwrap(),
            ))
            .unwrap(),
        );
        let operation = scheduler
            .enqueue(OperationSpec {
                source_display: "SSH repository".into(),
                destination_display: "Selected destination".into(),
                reservation: None,
            })
            .unwrap();
        scheduler
            .transition(
                &operation.id,
                OperationState::Running,
                Phase::Authentication,
                None,
            )
            .unwrap();
        let broker = AuthBroker::new(scheduler.clone());
        let session = broker
            .session("main", &operation.id, Cancellation::default())
            .unwrap();
        let options = session.options_for_paths(paths).unwrap();
        assert!(!session.ssh_prompt_available());
        assert_eq!(
            options
                .environment
                .iter()
                .find(|(key, _)| key == "SSH_ASKPASS_REQUIRE")
                .unwrap()
                .1,
            "never"
        );
        assert!(options
            .environment
            .iter()
            .find(|(key, _)| key == "SSH_ASKPASS")
            .unwrap()
            .1
            .is_empty());
        assert_eq!(
            options
                .environment
                .iter()
                .find(|(key, _)| key == "GIT_ASKPASS")
                .unwrap()
                .1,
            original.as_os_str()
        );
        let keygen = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/OpenSSH/ssh-keygen.exe");
        let key = fixture.path().join("unencrypted owned key");
        assert!(std::process::Command::new(&keygen)
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&key)
            .output()
            .unwrap()
            .status
            .success());
        let output = GitRunner::trusted(keygen)
            .unwrap()
            .run_with(
                None,
                &["-y".into(), "-f".into(), key.into_os_string()],
                &Cancellation::default(),
                Phase::Authentication,
                options,
            )
            .await
            .unwrap();
        assert!(output.starts_with(b"ssh-ed25519 "));
        assert!(scheduler.snapshot().unwrap().operations[0]
            .auth_prompt
            .is_none());
        session.close();
    }
    #[test]
    fn parent_substitution_during_root_first_acquisition_rejects_changed_original() {
        let fixture = tempfile::tempdir().unwrap();
        let parent = fixture.path().join("parent");
        std::fs::create_dir(&parent).unwrap();
        let original = parent.join("helper.exe");
        std::fs::write(&original, b"original payload").unwrap();
        let moved = fixture.path().join("moved original");
        let moved_hook = moved.clone();
        let parent_hook = parent.clone();
        AFTER_FIRST_PIN.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_| {
                std::fs::rename(&parent_hook, &moved_hook).unwrap();
                std::fs::create_dir(&parent_hook).unwrap();
                std::fs::write(parent_hook.join("helper.exe"), b"replacement sentinel").unwrap();
            }))
        });
        assert!(select(&original).is_err());
        assert_eq!(
            std::fs::read(moved.join("helper.exe")).unwrap(),
            b"original payload"
        );
        assert_eq!(std::fs::read(original).unwrap(), b"replacement sentinel");
    }
    #[tokio::test]
    async fn runner_guard_survives_caller_release_and_releases_after_confirmed_cancel() {
        use crate::{
            domain::contracts::{ErrorCode, Phase},
            git::runner::{Cancellation, GitRunner, RunOptions},
        };
        let fixture = tempfile::tempdir().unwrap();
        let original = fixture.path().join("helper Ω.exe");
        std::fs::write(&original, b"owned helper lease").unwrap();
        let paths = std::sync::Arc::new(select(&original).unwrap());
        let weak = std::sync::Arc::downgrade(&paths);
        let runner = GitRunner::trusted(
            PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        )
        .unwrap();
        let cancel = Cancellation::default();
        let trigger = cancel.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started = std::sync::Mutex::new(Some(started_tx));
        let options = RunOptions {
            lifetime_guard: Some(paths.clone()),
            before_resume: Some(std::sync::Arc::new(move |_| {
                started.lock().unwrap().take().unwrap().send(()).unwrap();
                Ok(())
            })),
            ..Default::default()
        };
        drop(paths);
        let task = tokio::spawn(async move {
            runner
                .run_with(
                    None,
                    &crate::git::runner::argv(&[
                        "-NoProfile",
                        "-Command",
                        "Start-Sleep -Seconds 30",
                    ]),
                    &cancel,
                    Phase::Authentication,
                    options,
                )
                .await
        });
        started_rx.await.unwrap();
        assert!(weak.upgrade().is_some());
        assert!(std::fs::rename(&original, fixture.path().join("moved.exe")).is_err());
        trigger.cancel();
        assert_eq!(
            task.await.unwrap().unwrap_err().code,
            ErrorCode::Interrupted
        );
        assert!(weak.upgrade().is_none());
        std::fs::rename(&original, fixture.path().join("moved.exe")).unwrap();
    }
    #[test]
    fn unavailable_unchanged_or_wrong_identity_alias_never_enables_ssh_prompt() {
        let fixture = tempfile::tempdir().unwrap();
        let original = fixture.path().join("helper Ω.exe");
        std::fs::write(&original, b"original").unwrap();
        let wrong = fixture.path().join("wrong.exe");
        std::fs::write(&wrong, b"wrong").unwrap();
        for mode in 0..3 {
            let selected = select_with(&original, |_| match mode {
                0 => Err(std::io::Error::from_raw_os_error(2)),
                1 => Ok(original.clone()),
                _ => Ok(wrong.clone()),
            })
            .unwrap();
            assert_eq!(selected.git, original.as_os_str());
            assert!(selected.ssh.is_none());
            assert!(std::fs::rename(&original, fixture.path().join("moved.exe")).is_err());
            drop(selected);
        }
        std::fs::rename(&original, fixture.path().join("moved.exe")).unwrap();
        assert_eq!(std::fs::read(wrong).unwrap(), b"wrong");
    }
    #[test]
    fn actual_short_alias_keeps_same_file_and_ancestor_pinned_until_release() {
        let fixture = tempfile::tempdir().unwrap();
        let directory = fixture.path().join("helper Ω space");
        std::fs::create_dir(&directory).unwrap();
        let original = directory.join("helper Ω quote's.exe");
        std::fs::write(&original, b"payload").unwrap();
        let selected = select(&original).unwrap();
        let alias = selected
            .ssh
            .as_ref()
            .expect("fixture volume must provide ASCII short alias");
        assert!(ascii(Path::new(alias)));
        assert_eq!(
            crate::repositories::discovery::identity(Path::new(alias)).unwrap(),
            crate::repositories::discovery::identity(&original).unwrap()
        );
        assert!(std::fs::rename(&original, directory.join("moved.exe")).is_err());
        assert!(std::fs::rename(&directory, fixture.path().join("moved directory")).is_err());
        drop(selected);
        std::fs::rename(&directory, fixture.path().join("moved directory")).unwrap();
    }
}
