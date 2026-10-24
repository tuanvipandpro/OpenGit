use super::{
    journal::{CleanupDisposition, CleanupReason, CloneRecord},
    path::{
        delete_empty_object, final_path, handle_info, info_identity, publish_pinned, unsafe_path,
        PublishOutcome, StableDirectory,
    },
};
use crate::{domain::contracts::*, git::error::error};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::windows::fs::OpenOptionsExt,
    path::PathBuf,
    sync::Arc,
};
use windows_sys::Win32::Storage::FileSystem::*;
pub const MARKER: &str = ".opengit-stage-owner";
pub struct Stage {
    pub parent: Arc<StableDirectory>,
    pub container: PathBuf,
    pub repository: PathBuf,
    container_handle: Option<Arc<StableDirectory>>,
    repository_handle: Option<StableDirectory>,
    marker_handle: Option<File>,
    published: bool,
}
pub struct CleanupResult {
    pub disposition: CleanupDisposition,
    _published_guard: Option<StableDirectory>,
}
impl CleanupResult {
    fn retained(reason: CleanupReason, published: Option<StableDirectory>) -> Self {
        Self {
            disposition: CleanupDisposition::Retained(reason),
            _published_guard: published,
        }
    }
    pub fn warning(&self) -> Option<AppError> {
        self.disposition
            .retained()
            .then(|| error(ErrorCode::Io, Phase::Recovery))
    }
}
#[cfg(test)]
type BoundaryHook = Box<dyn FnOnce(&std::path::Path)>;
#[cfg(test)]
thread_local! {
 static BEFORE_PIN:std::cell::RefCell<Option<BoundaryHook>> = const {std::cell::RefCell::new(None)};
 static BEFORE_REPIN:std::cell::RefCell<Option<BoundaryHook>> = const {std::cell::RefCell::new(None)};
 static FAIL_AFTER_RENAME:std::cell::Cell<bool> = const {std::cell::Cell::new(false)};
}
impl Stage {
    pub fn create(
        parent: Arc<StableDirectory>,
        record: &mut CloneRecord,
    ) -> Result<Self, AppError> {
        parent.recheck()?;
        let container = parent.path.join(&record.container);
        fs::create_dir(&container).map_err(|_| error(ErrorCode::Io, Phase::Transfer))?;
        #[cfg(test)]
        BEFORE_PIN.with(|hook| {
            if let Some(hook) = hook.borrow_mut().take() {
                hook(&container);
            }
        });
        // No canonicalize/follow and no marker/repository write before pin.
        let container_handle = Arc::new(StableDirectory::pin_child(
            parent.clone(),
            &record.container,
        )?);
        if fs::read_dir(&container)
            .map_err(|_| unsafe_path())?
            .next()
            .is_some()
        {
            return Err(unsafe_path());
        }
        record.container_identity = Some(container_handle.identity.clone());
        let mut marker = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .access_mode(FILE_GENERIC_READ | FILE_GENERIC_WRITE | 0x10000)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(container.join(MARKER))
            .map_err(|_| unsafe_path())?;
        marker
            .write_all(record.nonce.as_bytes())
            .map_err(|_| unsafe_path())?;
        marker.sync_all().map_err(|_| unsafe_path())?;
        record.marker_identity = Some(info_identity(&handle_info(&marker)?));
        let repository = container.join("repository");
        fs::create_dir(&repository).map_err(|_| unsafe_path())?;
        let repository_handle =
            StableDirectory::pin_child_read(container_handle.clone(), "repository")?;
        if fs::read_dir(&repository)
            .map_err(|_| unsafe_path())?
            .next()
            .is_some()
        {
            return Err(unsafe_path());
        }
        record.repository_identity = Some(repository_handle.identity.clone());
        Ok(Self {
            parent,
            container,
            repository,
            container_handle: Some(container_handle),
            repository_handle: Some(repository_handle),
            marker_handle: Some(marker),
            published: false,
        })
    }
    fn mutation_pin(&mut self, record: &CloneRecord) -> Result<(), AppError> {
        // Git and its owned helper tree have stopped before this boundary.
        // The gap permits no mutation; only the original identity authorizes it.
        self.repository_handle.take();
        #[cfg(test)]
        BEFORE_REPIN.with(|hook| {
            if let Some(hook) = hook.borrow_mut().take() {
                hook(&self.repository);
            }
        });
        let source = StableDirectory::pin_child(
            self.container_handle
                .as_ref()
                .ok_or_else(unsafe_path)?
                .clone(),
            "repository",
        )?;
        if record.repository_identity.as_ref() != Some(&source.identity) {
            return Err(error(ErrorCode::ExternalChange, Phase::Verification));
        }
        self.repository_handle = Some(source);
        Ok(())
    }
    pub fn publish(&mut self, record: &CloneRecord) -> Result<PublishOutcome, AppError> {
        self.mutation_pin(record)?;
        let source = self.repository_handle.as_mut().ok_or_else(unsafe_path)?;
        let result = publish_pinned(
            source,
            record
                .repository_identity
                .as_deref()
                .ok_or_else(unsafe_path)?,
            &self.parent,
            &record.leaf,
        )?;
        self.published = true;
        if result == PublishOutcome::Committed {
            #[cfg(test)]
            let reanchor = if FAIL_AFTER_RENAME.with(|fault| fault.replace(false)) {
                Err(error(ErrorCode::ExternalChange, Phase::Recovery))
            } else {
                source.reanchor(self.parent.clone(), &record.leaf)
            };
            #[cfg(not(test))]
            let reanchor = source.reanchor(self.parent.clone(), &record.leaf);
            if reanchor.is_err() {
                // The syscall committed already. An in-memory guard update
                // cannot turn this into an ordinary pre-publication error.
                return Ok(PublishOutcome::Uncertain);
            }
        }
        Ok(result)
    }
    pub(crate) fn may_have_published(&self) -> bool {
        self.published
    }
    pub fn cleanup(mut self, record: &CloneRecord) -> CleanupResult {
        if !self.published && self.mutation_pin(record).is_err() {
            return CleanupResult::retained(CleanupReason::IdentityChanged, None);
        }
        let published = if self.published {
            self.repository_handle.take()
        } else {
            None
        };
        cleanup_handles(
            self.parent.clone(),
            self.container_handle.take(),
            self.repository_handle.take(),
            self.marker_handle.take(),
            record,
            published,
        )
    }
}
/// Recovery never retries Retained artifacts. There is no recursive path delete.
pub fn cleanup(parent: Arc<StableDirectory>, record: &CloneRecord) -> CleanupResult {
    if let CleanupDisposition::Retained(reason) = &record.cleanup {
        return CleanupResult::retained(reason.clone(), None);
    }
    match fs::symlink_metadata(parent.path.join(&record.container)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CleanupResult {
                disposition: CleanupDisposition::AlreadyAbsent,
                _published_guard: None,
            }
        }
        Err(_) => return CleanupResult::retained(CleanupReason::OwnershipUnproven, None),
        Ok(_) => {}
    }
    let container = match StableDirectory::pin_child(parent.clone(), &record.container) {
        Ok(value) => Arc::new(value),
        Err(_) => return CleanupResult::retained(CleanupReason::OwnershipUnproven, None),
    };
    if record.container_identity.as_ref() != Some(&container.identity) {
        return CleanupResult::retained(CleanupReason::IdentityChanged, None);
    }
    let repository = if fs::symlink_metadata(container.path.join("repository")).is_ok() {
        match StableDirectory::pin_child(container.clone(), "repository") {
            Ok(value) => Some(value),
            Err(_) => return CleanupResult::retained(CleanupReason::OwnershipUnproven, None),
        }
    } else {
        None
    };
    let marker = OpenOptions::new()
        .read(true)
        .access_mode(FILE_GENERIC_READ | 0x10000)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(container.path.join(MARKER))
        .ok();
    cleanup_handles(parent, Some(container), repository, marker, record, None)
}

fn cleanup_handles(
    parent: Arc<StableDirectory>,
    container: Option<Arc<StableDirectory>>,
    repository: Option<StableDirectory>,
    marker: Option<File>,
    record: &CloneRecord,
    published: Option<StableDirectory>,
) -> CleanupResult {
    let retain = |reason, published| CleanupResult::retained(reason, published);
    if parent.recheck().is_err() {
        return retain(CleanupReason::OwnershipUnproven, published);
    }
    let Some(container) = container else {
        return retain(CleanupReason::OwnershipUnproven, published);
    };
    if container.recheck().is_err()
        || record.container_identity.as_ref() != Some(&container.identity)
    {
        return retain(CleanupReason::IdentityChanged, published);
    }
    let Some(marker) = marker else {
        return retain(CleanupReason::OwnershipUnproven, published);
    };
    let marker_valid = (|| {
        let info = handle_info(&marker)?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || record.marker_identity.as_ref() != Some(&info_identity(&info))
            || final_path(&marker)? != container.path.join(MARKER)
        {
            return Err(unsafe_path());
        }
        let mut cursor = &marker;
        cursor.seek(SeekFrom::Start(0)).map_err(|_| unsafe_path())?;
        let mut bytes = Vec::new();
        cursor
            .take((record.nonce.len() + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| unsafe_path())?;
        if bytes != record.nonce.as_bytes() {
            return Err(unsafe_path());
        }
        Ok(())
    })();
    if marker_valid.is_err() {
        return retain(CleanupReason::OwnershipUnproven, published);
    }
    let entries = match fs::read_dir(&container.path) {
        Ok(entries) => entries.collect::<Result<Vec<_>, _>>(),
        Err(_) => return retain(CleanupReason::IoFailure, published),
    };
    let entries = match entries {
        Ok(entries) => entries,
        Err(_) => return retain(CleanupReason::IoFailure, published),
    };
    if entries
        .iter()
        .any(|entry| entry.file_name() != MARKER && entry.file_name() != "repository")
    {
        return retain(CleanupReason::UnexpectedEntries, published);
    }
    drop(entries);
    if let Some(repository) = repository {
        if repository.recheck().is_err()
            || record.repository_identity.as_ref() != Some(&repository.identity)
        {
            return retain(CleanupReason::IdentityChanged, published);
        }
        match fs::read_dir(&repository.path) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return retain(CleanupReason::NonEmpty, published);
                }
            }
            Err(_) => return retain(CleanupReason::IoFailure, published),
        };
        if delete_empty_object(repository.handle()).is_err() {
            return retain(CleanupReason::IoFailure, published);
        }
        drop(repository);
    } else if fs::symlink_metadata(container.path.join("repository")).is_ok() {
        return retain(CleanupReason::OwnershipUnproven, published);
    }
    if delete_empty_object(&marker).is_err() {
        return retain(CleanupReason::IoFailure, published);
    }
    drop(marker);
    if delete_empty_object(container.handle()).is_err() {
        return retain(CleanupReason::IoFailure, published);
    }
    drop(container);
    CleanupResult {
        disposition: CleanupDisposition::Removed,
        _published_guard: published,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn post_rename_fault_preserves_intent_target_and_idempotent_recovery() {
        use crate::{
            backend::Backend,
            git::runner::{argv, Cancellation, GitRunner},
            operations::state::terminal,
        };
        let fixture = tempfile::tempdir().unwrap();
        let runner = GitRunner::discover().unwrap();
        let source = fixture.path().join("source");
        fs::create_dir(&source).unwrap();
        runner
            .run(
                Some(&source),
                &argv(&["init"]),
                &Cancellation::default(),
                Phase::Inspection,
            )
            .await
            .unwrap();
        fs::write(source.join("sentinel"), b"original payload").unwrap();
        runner
            .run(
                Some(&source),
                &argv(&["add", "."]),
                &Cancellation::default(),
                Phase::Inspection,
            )
            .await
            .unwrap();
        runner
            .run(
                Some(&source),
                &argv(&[
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "-m",
                    "fixture",
                ]),
                &Cancellation::default(),
                Phase::Inspection,
            )
            .await
            .unwrap();
        let destination = fixture.path().join("destinations");
        fs::create_dir(&destination).unwrap();
        let data = fixture.path().join("data");
        let backend = Arc::new(Backend::with_git(&data, runner.clone()).unwrap());
        backend.get_workspace("main").unwrap();
        let source_handle = backend.grant_clone_source("main", &source).await.unwrap();
        let grant = backend
            .grant_clone_destination("main", &destination)
            .unwrap();
        FAIL_AFTER_RENAME.with(|fault| fault.set(true));
        let operation = backend
            .start_clone(
                "main",
                StartCloneRequest {
                    source: CloneSource::Local {
                        source_handle: source_handle.source_handle,
                    },
                    destination_handle: grant.destination_handle,
                    directory_name: "published".into(),
                },
            )
            .await
            .unwrap();
        let snapshot = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let snapshot = backend
                    .scheduler
                    .snapshot()
                    .unwrap()
                    .operations
                    .into_iter()
                    .find(|entry| entry.id == operation.id)
                    .unwrap();
                if terminal(&snapshot.state) {
                    break snapshot;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(snapshot.state, OperationState::Interrupted);
        assert_eq!(snapshot.error.as_ref().unwrap().code, ErrorCode::Database);
        let record = super::super::journal::load(&backend.registry.database)
            .unwrap()
            .remove(0);
        assert_eq!(
            record.phase,
            super::super::journal::ClonePhase::PublishIntent
        );
        assert_eq!(record.cleanup, CleanupDisposition::NotAttempted);
        let target = destination.join("published");
        assert_eq!(
            crate::repositories::discovery::identity(&target).unwrap(),
            record.repository_identity.clone().unwrap()
        );
        assert_eq!(
            fs::read(target.join("sentinel")).unwrap(),
            b"original payload"
        );
        assert!(destination.join(&record.container).join(MARKER).exists());
        assert!(backend
            .get_workspace("main")
            .unwrap()
            .repositories
            .is_empty());
        assert_eq!(
            backend
                .cancel_operation(
                    "main",
                    CancelOperationRequest {
                        operation_id: operation.id.clone()
                    }
                )
                .unwrap_err()
                .code,
            ErrorCode::Database
        );
        assert_eq!(
            backend.scheduler.cancel(&operation.id).unwrap().state,
            OperationState::Interrupted
        );
        let weak = Arc::downgrade(&backend);
        drop(backend);
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while weak.upgrade().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut stable_selection = None;
        for _ in 0..2 {
            let reopened = Backend::with_git(&data, runner.clone()).unwrap();
            let workspace = reopened.get_workspace("main").unwrap();
            assert_eq!(workspace.repositories.len(), 1);
            if let Some(expected) = &stable_selection {
                assert_eq!(&workspace.active_selection, expected);
            } else {
                stable_selection = Some(workspace.active_selection.clone());
            }
            let snapshot = reopened.scheduler.snapshot().unwrap().operations.remove(0);
            assert_eq!(snapshot.state, OperationState::Succeeded);
            assert_eq!(
                crate::repositories::discovery::identity(&target).unwrap(),
                record.repository_identity.clone().unwrap()
            );
            assert_eq!(
                fs::read(target.join("sentinel")).unwrap(),
                b"original payload"
            );
            assert_eq!(
                fs::read(source.join("sentinel")).unwrap(),
                b"original payload"
            );
            assert_eq!(
                super::super::journal::load(&reopened.registry.database).unwrap()[0].phase,
                super::super::journal::ClonePhase::Registered
            );
        }
    }
    fn junction(path: &std::path::Path, target: &std::path::Path) {
        use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
        fs::create_dir(path).unwrap();
        let target = fs::canonicalize(target).unwrap();
        let print: Vec<u16> = target.as_os_str().encode_wide().skip(4).collect();
        let substitute: Vec<u16> = "\\??\\"
            .encode_utf16()
            .chain(print.iter().copied())
            .collect();
        let sub_len = u16::try_from(substitute.len() * 2).unwrap();
        let print_len = u16::try_from(print.len() * 2).unwrap();
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&0xa0000003u32.to_le_bytes());
        buffer.extend_from_slice(&(12 + sub_len + print_len).to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        for field in [0, sub_len, sub_len + 2, print_len] {
            buffer.extend_from_slice(&field.to_le_bytes());
        }
        for value in substitute.into_iter().chain([0]).chain(print).chain([0]) {
            buffer.extend_from_slice(&value.to_le_bytes());
        }
        let handle = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .unwrap();
        let mut returned = 0;
        assert_ne!(
            unsafe {
                windows_sys::Win32::System::IO::DeviceIoControl(
                    handle.as_raw_handle() as _,
                    0x900a4,
                    buffer.as_ptr() as _,
                    buffer.len() as u32,
                    std::ptr::null_mut(),
                    0,
                    &mut returned,
                    std::ptr::null_mut(),
                )
            },
            0
        );
    }
    #[tokio::test]
    async fn initial_substitution_never_writes_outside_or_spawns_git() {
        use crate::{
            backend::Backend,
            git::runner::{Cancellation, GitRunner},
        };
        for reparse in [false, true] {
            let fixture = tempfile::tempdir().unwrap();
            let outside = fixture.path().join("outside");
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("sentinel"), b"outside unchanged").unwrap();
            let destination = fixture.path().join("destination");
            fs::create_dir(&destination).unwrap();
            let parent = Arc::new(StableDirectory::open(&destination).unwrap());
            let mut record = super::super::execute::record(&parent, "fixture", "published");
            let original = parent.path.join(&record.container);
            let moved = parent.path.join("moved original");
            let outside_hook = outside.clone();
            let moved_hook = moved.clone();
            BEFORE_PIN.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |path| {
                    fs::rename(path, &moved_hook).unwrap();
                    if reparse {
                        junction(path, &outside_hook);
                    } else {
                        fs::create_dir(path).unwrap();
                        fs::write(path.join("replacement sentinel"), b"replacement unchanged")
                            .unwrap();
                    }
                }))
            });
            // A controlled executable would leave a durable sentinel if the
            // transfer boundary were reached. No test-only production branch.
            let marker = fixture.path().join("git spawned");
            let source = fixture.path().join("git.rs");
            fs::write(
                &source,
                format!(
                    "fn main(){{std::fs::write(r#\"{}\"#,b\"spawned\").unwrap();}}",
                    marker.display()
                ),
            )
            .unwrap();
            let exe = fixture.path().join("git.exe");
            let result = std::process::Command::new("rustc")
                .arg(&source)
                .arg("-o")
                .arg(&exe)
                .output()
                .unwrap();
            assert!(result.status.success());
            let backend = Arc::new(
                Backend::with_git(
                    &fixture.path().join("data"),
                    GitRunner::trusted(exe).unwrap(),
                )
                .unwrap(),
            );
            record.operation_id = backend
                .scheduler
                .enqueue(crate::operations::scheduler::OperationSpec {
                    source_display: "Local repository".into(),
                    destination_display: "Selected destination".into(),
                    reservation: None,
                })
                .unwrap()
                .id;
            let prepared = super::super::execute::Prepared {
                parent: parent.clone(),
                source: "unused".into(),
                source_guard: None,
                interactive: false,
                record: record.clone(),
                control: Arc::new(super::super::grants::Control {
                    window: "main".into(),
                    parent_identity: parent.identity.clone(),
                    leaf: "published".into(),
                    gate: std::sync::Mutex::new(()),
                    published: std::sync::atomic::AtomicBool::new(false),
                }),
            };
            let failure =
                super::super::execute::run(backend.clone(), prepared, Cancellation::default())
                    .await
                    .err()
                    .unwrap();
            assert!(failure
                .message
                .ends_with(crate::git::error::RETENTION_NOTICE));
            let stored = super::super::journal::load(&backend.registry.database).unwrap();
            assert!(stored[0].cleanup.retained());
            assert_eq!(stored[0].phase, super::super::journal::ClonePhase::Aborted);
            assert!(!marker.exists());
            assert_eq!(
                fs::read(outside.join("sentinel")).unwrap(),
                b"outside unchanged"
            );
            assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
            assert_eq!(fs::read_dir(&moved).unwrap().count(), 0);
            assert!(!original.join(MARKER).exists());
            assert!(!original.join("repository").exists());
            if reparse {
                fs::remove_dir(&original).unwrap();
            } else {
                assert_eq!(
                    fs::read(original.join("replacement sentinel")).unwrap(),
                    b"replacement unchanged"
                );
            }
        }
    }
    #[test]
    fn reauthentication_gap_and_pinned_boundary_preserve_both_objects() {
        for reparse in [false, true] {
            let fixture = tempfile::tempdir().unwrap();
            let parent = Arc::new(StableDirectory::open(fixture.path()).unwrap());
            let mut record = super::super::execute::record(&parent, "fixture", "destination");
            let mut stage = Stage::create(parent.clone(), &mut record).unwrap();
            fs::write(stage.repository.join("owned sentinel"), b"owned unchanged").unwrap();
            let moved = parent.path.join("moved repository");
            let moved_hook = moved.clone();
            let outside = fixture.path().join("outside");
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("sentinel"), b"outside unchanged").unwrap();
            let outside_hook = outside.clone();
            BEFORE_REPIN.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |path| {
                    fs::rename(path, &moved_hook).unwrap();
                    if reparse {
                        junction(path, &outside_hook);
                    } else {
                        fs::create_dir(path).unwrap();
                        fs::write(path.join("replacement sentinel"), b"replacement unchanged")
                            .unwrap();
                    }
                }))
            });
            assert!(stage.publish(&record).is_err());
            assert!(!parent.path.join("destination").exists());
            let outcome = stage.cleanup(&record);
            assert!(outcome.disposition.retained());
            assert_eq!(
                fs::read(moved.join("owned sentinel")).unwrap(),
                b"owned unchanged"
            );
            assert_eq!(
                fs::read(outside.join("sentinel")).unwrap(),
                b"outside unchanged"
            );
            let replacement = parent.path.join(&record.container).join("repository");
            if reparse {
                fs::remove_dir(&replacement).unwrap();
            } else {
                assert_eq!(
                    fs::read(replacement.join("replacement sentinel")).unwrap(),
                    b"replacement unchanged"
                );
            }
        }
        let fixture = tempfile::tempdir().unwrap();
        let parent = Arc::new(StableDirectory::open(fixture.path()).unwrap());
        let mut record = super::super::execute::record(&parent, "fixture", "destination");
        let mut stage = Stage::create(parent.clone(), &mut record).unwrap();
        stage.mutation_pin(&record).unwrap();
        assert!(fs::rename(&stage.repository, stage.container.join("moved")).is_err());
        assert!(fs::rename(&stage.container, parent.path.join("moved container")).is_err());
        assert_eq!(
            stage.cleanup(&record).disposition,
            CleanupDisposition::Removed
        );
    }
    #[test]
    fn same_initial_handles_remove_only_owned_empty_objects_on_close() {
        let fixture = tempfile::tempdir().unwrap();
        let parent = Arc::new(StableDirectory::open(fixture.path()).unwrap());
        let mut record = super::super::execute::record(&parent, "fixture", "destination");
        let stage = Stage::create(parent.clone(), &mut record).unwrap();
        assert!(fs::rename(&stage.container, parent.path.join("moved")).is_err());
        assert!(fs::rename(&stage.repository, parent.path.join("moved repo")).is_err());
        let outcome = stage.cleanup(&record);
        assert_eq!(outcome.disposition, CleanupDisposition::Removed);
        assert!(!parent.path.join(&record.container).exists());
    }
    #[tokio::test]
    async fn real_git_directory_compatibility_probe() {
        use crate::git::runner::{argv, Cancellation, GitRunner};
        let fixture = tempfile::tempdir().unwrap();
        let runner = GitRunner::discover().unwrap();
        let source = fixture.path().join("source");
        fs::create_dir(&source).unwrap();
        runner
            .run(
                Some(&source),
                &argv(&["init"]),
                &Cancellation::default(),
                Phase::Inspection,
            )
            .await
            .unwrap();
        fs::write(source.join("tracked"), b"tracked").unwrap();
        runner
            .run(
                Some(&source),
                &argv(&["add", "."]),
                &Cancellation::default(),
                Phase::Inspection,
            )
            .await
            .unwrap();
        runner
            .run(
                Some(&source),
                &argv(&[
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "-m",
                    "fixture",
                ]),
                &Cancellation::default(),
                Phase::Inspection,
            )
            .await
            .unwrap();
        let parent_path = fixture.path().join("destinations");
        fs::create_dir(&parent_path).unwrap();
        let parent = Arc::new(StableDirectory::open(&parent_path).unwrap());
        let mut record = super::super::execute::record(&parent, "fixture", "destination");
        let stage = Stage::create(parent, &mut record).unwrap();
        let args = vec![
            "clone".into(),
            "--no-hardlinks".into(),
            "--".into(),
            super::super::path::git_path(&fs::canonicalize(source).unwrap()).unwrap(),
            super::super::path::git_path(&stage.repository).unwrap(),
        ];
        let output = runner
            .output(None, &args, &Cancellation::default(), Phase::Transfer)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        let base = fixture.path().to_string_lossy().to_ascii_lowercase();
        let redacted = text
            .replace(&base, "[owned fixture]")
            .replace(&base.replace('\\', "/"), "[owned fixture]");
        assert!(
            output.success,
            "controlled local Git redacted diagnostic: {redacted}"
        );
    }
}
