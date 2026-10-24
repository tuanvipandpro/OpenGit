#![cfg(windows)]
use opengit_lib::{
    backend::Backend,
    clone::{
        path::{publish_no_replace, StableDirectory},
        request,
    },
    domain::contracts::*,
    git::runner::{argv, Cancellation, GitRunner},
    operations::state::terminal,
    repositories::discovery::identity,
};
use std::{path::Path, sync::Arc, time::Duration};
async fn git(runner: &GitRunner, path: &Path, args: &[&str]) -> Vec<u8> {
    runner
        .run(
            Some(path),
            &argv(args),
            &Cancellation::default(),
            Phase::Inspection,
        )
        .await
        .unwrap()
}
async fn finished(backend: &Backend, id: &str) -> OperationSnapshot {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let snapshot = backend.scheduler.snapshot().unwrap();
            let operation = snapshot
                .operations
                .into_iter()
                .find(|operation| operation.id == id)
                .unwrap();
            if terminal(&operation.state) {
                return operation;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[test]
fn source_and_windows_component_validation_reject_unsafe_inputs() {
    for name in [
        "",
        ".",
        "..",
        "../escape",
        "C:\\escape",
        "CON",
        "con.txt",
        "COM¹.txt",
        "LPT9",
        "trailing.",
        "trailing ",
        "bad:name",
        "bad\nname",
    ] {
        assert!(
            request::directory_name(name).is_err(),
            "invalid component accepted"
        );
    }
    assert!(request::directory_name(&"😀".repeat(101)).is_err());
    assert!(request::directory_name("clone Ω space").is_ok());
    for source in [
        "--upload-pack=evil",
        "ext::evil",
        "file:///repo",
        "http://example.invalid/repo",
        "https://user:secret@example.invalid/repo",
        "https://user@example.invalid/repo",
        "https://example.invalid/repo?token=secret",
        "ssh://user:secret@example.invalid/repo",
        "C:\\repo",
        "user@-host:repo",
        "user name@host:repo",
    ] {
        assert!(request::remote(source).is_err(), "invalid source accepted");
    }
    for source in [
        "https://example.invalid/repo",
        "ssh://git@example.invalid:2222/path",
        "git@example.invalid:repo.git",
        "example.invalid:/repo.git",
    ] {
        assert!(request::remote(source).is_ok(), "valid source rejected");
    }
}
#[test]
fn handle_rename_is_no_replace_and_parent_handles_block_relocation() {
    let fixture = tempfile::tempdir().unwrap();
    let parent = fixture.path().join("parent Ω");
    std::fs::create_dir(&parent).unwrap();
    let source = parent.join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("payload"), b"owned source").unwrap();
    let id = identity(&source).unwrap();
    let stable = StableDirectory::open(&parent).unwrap_or_else(|error| {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        let path = std::fs::canonicalize(&parent).unwrap();
        for ancestor in path.ancestors() {
            let metadata = std::fs::symlink_metadata(ancestor);
            let opened = std::fs::OpenOptions::new()
                .access_mode(0x80)
                .custom_flags(0x02000000 | 0x00200000)
                .share_mode(3)
                .open(ancestor);
            eprintln!(
                "owned fixture ancestor {:?}: metadata={:?}, open={:?}",
                ancestor.file_name(),
                metadata.map(|m| m.file_attributes()),
                opened.err().map(|error| error.raw_os_error())
            );
        }
        panic!("stable fixture directory failed: {:?}", error.code)
    });
    let occupied = parent.join("occupied");
    std::fs::create_dir(&occupied).unwrap();
    std::fs::write(occupied.join("sentinel"), b"outside destination").unwrap();
    let occupied_id = identity(&occupied).unwrap();
    let attempted = publish_no_replace(&source, &id, &stable, "occupied");
    if attempted.is_ok() {
        eprintln!("owned fixture collision effects: source_exists={}, dest_identity_same={}, sentinel_preserved={}, source_nested={}",source.exists(),identity(&occupied).as_ref()==Ok(&occupied_id),std::fs::read(occupied.join("sentinel")).is_ok_and(|bytes|bytes==b"outside destination"),occupied.join("source").exists());
    }
    assert_eq!(attempted.unwrap_err().code, ErrorCode::DestinationExists);
    assert_eq!(
        std::fs::read(occupied.join("sentinel")).unwrap(),
        b"outside destination"
    );
    assert!(source.join("payload").exists());
    assert!(std::fs::rename(&parent, fixture.path().join("moved")).is_err());
    let published = publish_no_replace(&source, &id, &stable, "published Ω");
    if published.is_err() {
        eprintln!(
            "owned fixture publish Win32 last-error: {:?}",
            std::io::Error::last_os_error().raw_os_error()
        );
    }
    published.unwrap();
    assert!(!source.exists());
    assert_eq!(identity(&parent.join("published Ω")).unwrap(), id);
    assert_eq!(
        std::fs::read(parent.join("published Ω/payload")).unwrap(),
        b"owned source"
    );
    drop(stable);
}
#[test]
fn owned_descendant_no_delete_handle_blocks_ancestor_directory_rename() {
    use std::os::windows::fs::OpenOptionsExt;
    let fixture = tempfile::tempdir().unwrap();
    let ancestor = fixture.path().join("ancestor");
    let child = ancestor.join("parent");
    std::fs::create_dir_all(&child).unwrap();
    let handle = std::fs::OpenOptions::new()
        .access_mode(0x80)
        .custom_flags(0x02000000 | 0x00200000)
        .share_mode(3)
        .open(&child)
        .unwrap();
    let result = std::fs::rename(&ancestor, fixture.path().join("moved"));
    assert!(
        result.is_err(),
        "owned child handle did not protect ancestor relocation"
    );
    drop(handle);
    std::fs::rename(&ancestor, fixture.path().join("moved")).unwrap();
}
#[test]
fn two_actual_publishers_racing_one_destination_preserve_the_loser() {
    let fixture = tempfile::tempdir().unwrap();
    let parent = Arc::new(StableDirectory::open(fixture.path()).unwrap());
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let mut workers = Vec::new();
    for name in ["first", "second"] {
        let source = parent.path.join(name);
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("payload"), name.as_bytes()).unwrap();
        let id = identity(&source).unwrap();
        let parent = parent.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            (
                name,
                source.clone(),
                id.clone(),
                publish_no_replace(&source, &id, &parent, "destination"),
            )
        }));
    }
    barrier.wait();
    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .filter(|(_, _, _, result)| matches!(
                result,
                Ok(opengit_lib::clone::path::PublishOutcome::Committed)
            ))
            .count(),
        1
    );
    for (name, source, id, result) in results {
        match result {
            Ok(opengit_lib::clone::path::PublishOutcome::Committed) => {
                assert!(!source.exists());
                assert_eq!(identity(&parent.path.join("destination")).unwrap(), id);
                assert_eq!(
                    std::fs::read(parent.path.join("destination/payload")).unwrap(),
                    name.as_bytes()
                );
            }
            Err(error) => {
                assert_eq!(error.code, ErrorCode::DestinationExists);
                assert_eq!(identity(&source).unwrap(), id);
                assert_eq!(
                    std::fs::read(source.join("payload")).unwrap(),
                    name.as_bytes()
                );
            }
            _ => panic!("uncertain publication in owned race fixture"),
        }
    }
    assert_eq!(std::fs::read_dir(&parent.path).unwrap().count(), 2);
}
#[test]
fn git_argv_paths_preserve_identity_and_reject_unsupported_namespaces() {
    use opengit_lib::clone::path::git_path;
    let fixture = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(fixture.path()).unwrap();
    let converted = git_path(&canonical).unwrap();
    assert_eq!(
        identity(Path::new(&converted)).unwrap(),
        identity(&canonical).unwrap()
    );
    assert!(git_path(Path::new(
        r"\\?\Volume{00000000-0000-0000-0000-000000000000}\repo"
    ))
    .is_err());
    assert!(git_path(Path::new(r"relative\repo")).is_err());
    assert!(git_path(Path::new(r"\\.\PhysicalDrive0")).is_err());
    let trailing = canonical.join("trailing.");
    std::fs::create_dir(&trailing).unwrap();
    assert!(git_path(&trailing).is_err());
    std::fs::remove_dir(&trailing).unwrap();
}
#[tokio::test]
async fn local_clone_registers_atomically_preserves_source_and_cancel_collision_scope() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    let parent = fixture.path().join("destination parent Ω");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&parent).unwrap();
    let runner = GitRunner::discover().unwrap();
    git(&runner, &source, &["init", "-b", "main"]).await;
    std::fs::write(source.join("source.sentinel"), b"original content").unwrap();
    git(&runner, &source, &["add", "."]).await;
    git(
        &runner,
        &source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    )
    .await;
    let original = git(&runner, &source, &["rev-parse", "HEAD"]).await;
    let data = fixture.path().join("database");
    let backend = Arc::new(Backend::with_git(&data, runner.clone()).unwrap());
    backend.get_workspace("main").unwrap();
    let source_handle = backend.grant_clone_source("main", &source).await.unwrap();
    let destination = backend.grant_clone_destination("main", &parent).unwrap();
    assert_eq!(source_handle.source_handle.len(), 64);
    assert_eq!(destination.destination_handle.len(), 64);
    let request = |name: &str| StartCloneRequest {
        source: CloneSource::Local {
            source_handle: source_handle.source_handle.clone(),
        },
        destination_handle: destination.destination_handle.clone(),
        directory_name: name.into(),
    };
    let operation = backend
        .start_clone("main", request("published Ω"))
        .await
        .unwrap();
    let result = finished(&backend, &operation.id).await;
    if result.state != OperationState::Succeeded {
        let raw = std::fs::canonicalize(&source).unwrap();
        let destination = fixture.path().join("diagnostic clone");
        let args = vec![
            "clone".into(),
            "--no-hardlinks".into(),
            "--".into(),
            raw.into_os_string(),
            destination.into_os_string(),
        ];
        let diagnostic = runner
            .output(None, &args, &Cancellation::default(), Phase::Transfer)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&diagnostic.stderr).to_ascii_lowercase();
        eprintln!("owned local fixture Git failure classes: success={}, protocol={}, directory={}, permission={}",diagnostic.success,text.contains("protocol"),text.contains("directory"),text.contains("permission"));
    }
    assert_eq!(
        result.state,
        OperationState::Succeeded,
        "safe clone error: {:?}",
        result.error
    );
    assert_eq!(result.progress.unwrap().completed, 4);
    assert!(parent.join("published Ω/source.sentinel").is_file());
    assert_eq!(
        std::fs::read(source.join("source.sentinel")).unwrap(),
        b"original content"
    );
    assert_eq!(
        git(&runner, &source, &["rev-parse", "HEAD"]).await,
        original
    );
    let workspace = backend.get_workspace("main").unwrap();
    assert_eq!(
        workspace.active_selection.as_ref().unwrap().repo_id,
        result.repo_id.unwrap()
    );
    assert!(
        backend
            .cancel_operation(
                "main",
                CancelOperationRequest {
                    operation_id: operation.id
                }
            )
            .unwrap()
            .state
            == OperationState::Succeeded
    );
    assert_eq!(
        backend
            .start_clone("main", request("published Ω"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::DestinationExists
    );
    let queued = backend
        .start_clone("main", request("cancelled"))
        .await
        .unwrap();
    backend
        .cancel_operation(
            "main",
            CancelOperationRequest {
                operation_id: queued.id.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        finished(&backend, &queued.id).await.state,
        OperationState::Cancelled
    );
    assert!(!parent.join("cancelled").exists());
    backend.get_workspace("other").unwrap();
    assert_eq!(
        backend
            .start_clone("other", request("wrong-window"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    backend.close_window("main");
    assert_eq!(
        backend
            .start_clone("main", request("closed"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    let weak = Arc::downgrade(&backend);
    drop(backend);
    tokio::time::timeout(Duration::from_secs(3), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let reopened = Backend::with_git(&data, runner).unwrap();
    assert_eq!(reopened.get_workspace("main").unwrap(), workspace);
    assert_eq!(
        std::fs::read_dir(&parent)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".opengit-stage-"))
            .count(),
        0
    );
}
async fn source_repository(fixture: &Path, runner: &GitRunner) -> std::path::PathBuf {
    let source = fixture.join("source repository");
    std::fs::create_dir(&source).unwrap();
    git(runner, &source, &["init", "-b", "main"]).await;
    std::fs::write(source.join("source.sentinel"), b"original source").unwrap();
    git(runner, &source, &["add", "."]).await;
    git(
        runner,
        &source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    )
    .await;
    source
}
async fn drop_backend(backend: Arc<Backend>) {
    let weak = Arc::downgrade(&backend);
    drop(backend);
    tokio::time::timeout(Duration::from_secs(3), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn restart_reconciles_real_pre_and_post_publish_filesystem_without_resuming_clone() {
    use opengit_lib::{
        clone::{
            execute,
            journal::{self, ClonePhase},
            staging::Stage,
        },
        operations::scheduler::{OperationSpec, Scheduler},
        persistence::workspace::Database,
    };
    for published in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let runner = GitRunner::discover().unwrap();
        let source = source_repository(fixture.path(), &runner).await;
        let parent = fixture.path().join("destinations");
        std::fs::create_dir(&parent).unwrap();
        let data = fixture.path().join("data");
        let database = Arc::new(Database::open(&data).unwrap());
        let scheduler = Scheduler::new(database.clone()).unwrap();
        let operation = scheduler
            .enqueue(OperationSpec {
                source_display: "Local repository".into(),
                destination_display: "Selected destination".into(),
                reservation: None,
            })
            .unwrap();
        scheduler
            .transition(
                &operation.id,
                OperationState::Running,
                Phase::Transfer,
                None,
            )
            .unwrap();
        let stable = Arc::new(StableDirectory::open(&parent).unwrap());
        let mut record = execute::record(&stable, &operation.id, "destination");
        journal::write(&database, &record).unwrap();
        let mut stage = Stage::create(stable.clone(), &mut record).unwrap();
        record.phase = ClonePhase::Staged;
        journal::write(&database, &record).unwrap();
        let args = vec![
            "clone".into(),
            "--no-hardlinks".into(),
            "--".into(),
            opengit_lib::clone::path::git_path(&std::fs::canonicalize(&source).unwrap()).unwrap(),
            opengit_lib::clone::path::git_path(&stage.repository).unwrap(),
        ];
        runner
            .run(None, &args, &Cancellation::default(), Phase::Transfer)
            .await
            .unwrap();
        let id = identity(&stage.repository).unwrap();
        if published {
            record.phase = ClonePhase::PublishIntent;
            journal::write(&database, &record).unwrap();
            assert_eq!(
                stage.publish(&record).unwrap(),
                opengit_lib::clone::path::PublishOutcome::Committed
            );
        }
        drop(stage);
        drop(stable);
        drop(scheduler);
        drop(database);
        let source_moved = fixture.path().join("source moved away");
        std::fs::rename(&source, &source_moved).unwrap();
        let backend = Backend::with_git(&data, runner.clone()).unwrap();
        let operations = backend.scheduler.snapshot().unwrap();
        let operation = operations
            .operations
            .iter()
            .find(|entry| entry.id == operation.id)
            .unwrap();
        assert_eq!(
            operation.state,
            if published {
                OperationState::Succeeded
            } else {
                OperationState::Interrupted
            }
        );
        let workspace = backend.get_workspace("main").unwrap();
        if published {
            assert_eq!(identity(&parent.join("destination")).unwrap(), id);
            assert_eq!(
                std::fs::read(parent.join("destination/source.sentinel")).unwrap(),
                b"original source"
            );
            assert_eq!(workspace.repositories.len(), 1);
        } else {
            assert!(!parent.join("destination").exists());
            assert!(workspace.repositories.is_empty());
        }
        assert_eq!(
            std::fs::read(source_moved.join("source.sentinel")).unwrap(),
            b"original source"
        );
        let retained = parent.join(&record.container);
        let moved = parent.join("retained owned tree moved");
        if published {
            assert!(!retained.exists());
        } else {
            assert_eq!(
                std::fs::read(retained.join("repository/source.sentinel")).unwrap(),
                b"original source"
            );
            assert!(operation
                .error
                .as_ref()
                .unwrap()
                .message
                .ends_with(opengit_lib::git::error::RETENTION_NOTICE));
        }
        drop(backend);
        if !published {
            std::fs::rename(&retained, &moved).unwrap();
            std::fs::create_dir(&retained).unwrap();
            std::fs::write(
                retained.join("replacement sentinel"),
                b"replacement unchanged",
            )
            .unwrap();
        }
        let reopened = Backend::with_git(&data, runner.clone()).unwrap();
        assert_eq!(reopened.get_workspace("main").unwrap(), workspace);
        assert_eq!(
            reopened.scheduler.snapshot().unwrap().operations[0].state,
            operation.state
        );
        drop(reopened);
        let repeated = Backend::with_git(&data, runner).unwrap();
        assert_eq!(repeated.get_workspace("main").unwrap(), workspace);
        if !published {
            assert_eq!(
                std::fs::read(retained.join("replacement sentinel")).unwrap(),
                b"replacement unchanged"
            );
            assert_eq!(
                std::fs::read(moved.join("repository/source.sentinel")).unwrap(),
                b"original source"
            );
            assert!(repeated.scheduler.snapshot().unwrap().operations[0]
                .error
                .as_ref()
                .unwrap()
                .message
                .ends_with(opengit_lib::git::error::RETENTION_NOTICE));
            assert!(
                opengit_lib::clone::journal::load(&repeated.registry.database).unwrap()[0]
                    .cleanup
                    .retained()
            );
        }
    }
}

#[tokio::test]
async fn post_publish_registration_failure_preserves_target_and_recovers_atomically() {
    let fixture = tempfile::tempdir().unwrap();
    let runner = GitRunner::discover().unwrap();
    let source = source_repository(fixture.path(), &runner).await;
    let parent = fixture.path().join("destinations");
    std::fs::create_dir(&parent).unwrap();
    let data = fixture.path().join("data");
    let backend = Arc::new(Backend::with_git(&data, runner.clone()).unwrap());
    let before = backend.get_workspace("main").unwrap();
    let source_handle = backend.grant_clone_source("main", &source).await.unwrap();
    let destination = backend.grant_clone_destination("main", &parent).unwrap();
    let connection = rusqlite::Connection::open(data.join("workspace.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER fixture_fail_registration BEFORE INSERT ON repositories BEGIN SELECT RAISE(ABORT,'fixture registration failure'); END;").unwrap();
    drop(connection);
    let request = |leaf: &str| StartCloneRequest {
        source: CloneSource::Local {
            source_handle: source_handle.source_handle.clone(),
        },
        destination_handle: destination.destination_handle.clone(),
        directory_name: leaf.into(),
    };
    let operation = backend
        .start_clone("main", request("published"))
        .await
        .unwrap();
    let result = finished(&backend, &operation.id).await;
    assert_eq!(result.state, OperationState::Interrupted);
    assert_eq!(
        backend.get_operations("main").unwrap_err().code,
        ErrorCode::Database
    );
    assert_eq!(backend.get_workspace("main").unwrap(), before);
    assert!(parent.join("published/source.sentinel").is_file());
    let published_id = identity(&parent.join("published")).unwrap();
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
        backend
            .start_clone("main", request("must-not-start"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Database
    );
    assert!(
        opengit_lib::persistence::workspace::Database::open(&data).is_err(),
        "another backend must not reconcile a live owner"
    );
    let connection = rusqlite::Connection::open(data.join("workspace.sqlite3")).unwrap();
    connection
        .execute_batch("DROP TRIGGER fixture_fail_registration;")
        .unwrap();
    drop(connection);
    drop_backend(backend).await;
    std::fs::rename(&source, fixture.path().join("source moved away")).unwrap();
    let reopened = Backend::with_git(&data, runner.clone()).unwrap();
    let restored = reopened
        .scheduler
        .snapshot()
        .unwrap()
        .operations
        .into_iter()
        .find(|entry| entry.id == operation.id)
        .unwrap();
    assert_eq!(restored.state, OperationState::Succeeded);
    assert_eq!(identity(&parent.join("published")).unwrap(), published_id);
    let workspace = reopened.get_workspace("main").unwrap();
    assert_eq!(workspace.repositories.len(), 1);
    assert_eq!(
        workspace.active_selection.as_ref().unwrap().repo_id,
        restored.repo_id.unwrap()
    );
    drop(reopened);
    let repeated = Backend::with_git(&data, runner).unwrap();
    assert_eq!(repeated.get_workspace("main").unwrap(), workspace);
}

#[tokio::test]
async fn successful_publication_retains_unexpected_artifact_with_durable_warning() {
    use opengit_lib::clone::{execute, journal, staging::Stage};
    use opengit_lib::operations::scheduler::OperationSpec;
    let fixture = tempfile::tempdir().unwrap();
    let runner = GitRunner::discover().unwrap();
    let source = source_repository(fixture.path(), &runner).await;
    let parent_path = fixture.path().join("destinations");
    std::fs::create_dir(&parent_path).unwrap();
    let data = fixture.path().join("data");
    let backend = Backend::with_git(&data, runner.clone()).unwrap();
    let operation = backend
        .scheduler
        .enqueue(OperationSpec {
            source_display: "Local repository".into(),
            destination_display: "Selected destination".into(),
            reservation: None,
        })
        .unwrap();
    backend
        .scheduler
        .transition(
            &operation.id,
            OperationState::Running,
            Phase::Transfer,
            None,
        )
        .unwrap();
    let parent = Arc::new(StableDirectory::open(&parent_path).unwrap());
    let mut record = execute::record(&parent, &operation.id, "published");
    let mut stage = Stage::create(parent.clone(), &mut record).unwrap();
    journal::write(&backend.registry.database, &record).unwrap();
    let args = vec![
        "clone".into(),
        "--no-hardlinks".into(),
        "--".into(),
        opengit_lib::clone::path::git_path(&std::fs::canonicalize(&source).unwrap()).unwrap(),
        opengit_lib::clone::path::git_path(&stage.repository).unwrap(),
    ];
    runner
        .run(None, &args, &Cancellation::default(), Phase::Transfer)
        .await
        .unwrap();
    std::fs::write(
        stage.container.join("unexpected sentinel"),
        b"retain unchanged",
    )
    .unwrap();
    record.phase = journal::ClonePhase::PublishIntent;
    journal::write(&backend.registry.database, &record).unwrap();
    assert_eq!(
        stage.publish(&record).unwrap(),
        opengit_lib::clone::path::PublishOutcome::Committed
    );
    let outcome = stage.cleanup(&record);
    assert!(outcome.disposition.retained());
    record.cleanup = outcome.disposition.clone();
    let published =
        opengit_lib::repositories::discovery::discover(&runner, &parent.path.join("published"))
            .await
            .unwrap();
    backend
        .scheduler
        .succeed_clone(&operation.id, &published, &mut record, outcome.warning())
        .unwrap();
    drop(outcome);
    drop(parent);
    drop(backend);
    for _ in 0..2 {
        let reopened = Backend::with_git(&data, runner.clone()).unwrap();
        let snapshot = reopened.scheduler.snapshot().unwrap().operations.remove(0);
        assert_eq!(snapshot.state, OperationState::Succeeded);
        assert!(snapshot
            .error
            .unwrap()
            .message
            .ends_with(opengit_lib::git::error::RETENTION_NOTICE));
        assert_eq!(
            std::fs::read(parent_path.join("published/source.sentinel")).unwrap(),
            b"original source"
        );
        assert_eq!(
            std::fs::read(
                parent_path
                    .join(&record.container)
                    .join("unexpected sentinel")
            )
            .unwrap(),
            b"retain unchanged"
        );
        assert!(journal::load(&reopened.registry.database).unwrap()[0]
            .cleanup
            .retained());
    }
}
fn controlled_git(fixture: &Path, real: &GitRunner) -> GitRunner {
    let source = fixture.join("controlled_git.rs");
    let executable = fixture.join("controlled git.exe");
    let code = format!(
        r###"fn main() {{
        let root=std::env::current_exe().unwrap().parent().unwrap().to_owned();let args:Vec<_>=std::env::args_os().skip(1).collect();
        if args.first().is_some_and(|arg|arg=="clone") {{
            let container=std::path::Path::new(args.last().unwrap()).parent().unwrap().file_name().unwrap();std::fs::write(root.join(format!("spawn-{{}}",container.to_string_lossy())),b"started").unwrap();
            std::fs::write(root.join("clone-started"),b"started").unwrap();let mode=std::fs::read_to_string(root.join("mode")).unwrap_or_default();
            let message=match mode.as_str() {{"network"=>Some("failed to connect"),"disk"=>Some("No space left on device"),"permission"=>Some("Permission denied"),_=>None}};
            if let Some(message)=message{{eprintln!("FAULT_SYNTHETIC_CANARY {{message}}");std::process::exit(128);}}
            if mode=="wait"{{while !root.join("release").exists(){{std::thread::sleep(std::time::Duration::from_millis(10));}}}}
        }}
        let result=std::process::Command::new(r#"{}"#).args(args).status().unwrap();std::process::exit(result.code().unwrap_or(128));
    }}"###,
        real.executable().display()
    );
    std::fs::write(&source, code).unwrap();
    let rustc = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .filter(|path| path.is_absolute())
        .map(|path| path.join("rustc.exe"))
        .find(|path| path.is_file())
        .unwrap();
    let result = std::process::Command::new(rustc)
        .arg(source)
        .args(["--edition", "2021", "-o"])
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "controlled Git fixture compilation failed"
    );
    GitRunner::trusted(executable).unwrap()
}
#[tokio::test]
async fn injected_transfer_errors_and_running_cancel_leave_source_target_and_staging_safe() {
    let fixture = tempfile::tempdir().unwrap();
    let real = GitRunner::discover().unwrap();
    let source = source_repository(fixture.path(), &real).await;
    let helper = fixture.path().join("helper");
    std::fs::create_dir(&helper).unwrap();
    let runner = controlled_git(&helper, &real);
    for (mode, expected) in [
        ("network", ErrorCode::Network),
        ("disk", ErrorCode::DiskFull),
        ("permission", ErrorCode::PermissionDenied),
        ("wait", ErrorCode::Interrupted),
    ] {
        let _ = std::fs::remove_file(helper.join("clone-started"));
        std::fs::write(helper.join("mode"), mode).unwrap();
        let data = fixture.path().join(format!("data-{mode}"));
        let parent = fixture.path().join(format!("parent-{mode}"));
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(parent.join("outside.sentinel"), b"destination data").unwrap();
        let backend = Arc::new(Backend::with_git(&data, runner.clone()).unwrap());
        backend.get_workspace("main").unwrap();
        let source_handle = backend.grant_clone_source("main", &source).await.unwrap();
        let destination = backend.grant_clone_destination("main", &parent).unwrap();
        let operation = backend
            .start_clone(
                "main",
                StartCloneRequest {
                    source: CloneSource::Local {
                        source_handle: source_handle.source_handle,
                    },
                    destination_handle: destination.destination_handle,
                    directory_name: "target".into(),
                },
            )
            .await
            .unwrap();
        if mode == "wait" {
            tokio::time::timeout(Duration::from_secs(5), async {
                while !helper.join("clone-started").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            backend
                .cancel_operation(
                    "main",
                    CancelOperationRequest {
                        operation_id: operation.id.clone(),
                    },
                )
                .unwrap();
        }
        let result = finished(&backend, &operation.id).await;
        if mode == "wait" {
            assert_eq!(result.state, OperationState::Cancelled);
        } else {
            assert_eq!(result.state, OperationState::Failed);
            assert_eq!(result.error.as_ref().unwrap().code, expected);
        }
        assert!(!parent.join("target").exists());
        assert_eq!(
            std::fs::read(parent.join("outside.sentinel")).unwrap(),
            b"destination data"
        );
        assert_eq!(
            std::fs::read(source.join("source.sentinel")).unwrap(),
            b"original source"
        );
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 1);
        assert!(
            !serde_json::to_string(&backend.scheduler.snapshot().unwrap())
                .unwrap()
                .contains("FAULT_SYNTHETIC_CANARY")
        );
        let bytes = std::fs::read(data.join("workspace.sqlite3")).unwrap();
        assert!(!bytes
            .windows(b"FAULT_SYNTHETIC_CANARY".len())
            .any(|part| part == b"FAULT_SYNTHETIC_CANARY"));
        drop_backend(backend).await;
    }
}
#[tokio::test]
async fn fail_closed_after_two_active_jobs_blocks_queued_admission_and_new_ipc() {
    let fixture = tempfile::tempdir().unwrap();
    let real = GitRunner::discover().unwrap();
    let helper = fixture.path().join("helper");
    std::fs::create_dir(&helper).unwrap();
    let runner = controlled_git(&helper, &real);
    std::fs::write(helper.join("mode"), "wait").unwrap();
    let parent = fixture.path().join("destinations");
    std::fs::create_dir(&parent).unwrap();
    std::fs::write(parent.join("outside sentinel"), b"unchanged").unwrap();
    let backend = Arc::new(Backend::with_git(&fixture.path().join("data"), runner).unwrap());
    backend.get_workspace("main").unwrap();
    let grant = backend.grant_clone_destination("main", &parent).unwrap();
    let request = |leaf: &str| StartCloneRequest {
        source: CloneSource::Remote {
            url: "https://example.invalid/never-contacted".into(),
        },
        destination_handle: grant.destination_handle.clone(),
        directory_name: leaf.into(),
    };
    let first = backend.start_clone("main", request("first")).await.unwrap();
    let second = backend
        .start_clone("main", request("second"))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let count = std::fs::read_dir(&helper)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("spawn-"))
                .count();
            if count == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let queued = backend
        .start_clone("main", request("queued"))
        .await
        .unwrap();
    assert_eq!(
        backend
            .scheduler
            .snapshot()
            .unwrap()
            .operations
            .iter()
            .find(|entry| entry.id == queued.id)
            .unwrap()
            .state,
        OperationState::Queued
    );
    let record = opengit_lib::clone::journal::load(&backend.registry.database)
        .unwrap()
        .into_iter()
        .find(|record| record.operation_id == queued.id)
        .unwrap();
    assert!(!parent.join(&record.container).exists());
    // Existing internal fault boundary models an unconfirmed stop closing
    // admission. This does not claim a real OS query failure occurred.
    backend.clones.fail_closed();
    assert_eq!(
        backend
            .start_clone("main", request("new forbidden"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Database
    );
    backend.scheduler.cancel(&first.id).unwrap();
    let stopped = finished(&backend, &first.id).await;
    assert_eq!(stopped.state, OperationState::Cancelled);
    let rejected = finished(&backend, &queued.id).await;
    assert_eq!(rejected.state, OperationState::Interrupted);
    assert_eq!(rejected.error.unwrap().code, ErrorCode::Database);
    assert!(!parent.join(&record.container).exists());
    assert!(!helper.join(format!("spawn-{}", record.container)).exists());
    assert_eq!(
        std::fs::read_dir(&helper)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("spawn-"))
            .count(),
        2
    );
    let stored = opengit_lib::clone::journal::load(&backend.registry.database)
        .unwrap()
        .into_iter()
        .find(|record| record.operation_id == queued.id)
        .unwrap();
    assert!(stored.container_identity.is_none());
    assert!(stored.repository_identity.is_none());
    assert_eq!(
        stored.phase,
        opengit_lib::clone::journal::ClonePhase::Preparing
    );
    assert!(backend
        .scheduler
        .snapshot()
        .unwrap()
        .operations
        .iter()
        .any(|entry| entry.id == second.id && entry.state == OperationState::Running));
    backend.scheduler.cancel(&second.id).unwrap();
    assert_eq!(
        finished(&backend, &second.id).await.state,
        OperationState::Cancelled
    );
    assert_eq!(
        std::fs::read(parent.join("outside sentinel")).unwrap(),
        b"unchanged"
    );
    assert!(!parent.join("queued").exists());
    assert!(!parent.join("new forbidden").exists());
    drop_backend(backend).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_races_publish_without_deleting_or_relabelling_committed_data() {
    let fixture = tempfile::tempdir().unwrap();
    let runner = GitRunner::discover().unwrap();
    let source = source_repository(fixture.path(), &runner).await;
    let parent = fixture.path().join("destinations");
    std::fs::create_dir(&parent).unwrap();
    let backend = Arc::new(Backend::with_git(&fixture.path().join("data"), runner).unwrap());
    backend.get_workspace("main").unwrap();
    let source_handle = backend.grant_clone_source("main", &source).await.unwrap();
    let destination = backend.grant_clone_destination("main", &parent).unwrap();
    for number in 0..4 {
        let mut events = backend.scheduler.subscribe();
        let leaf = format!("race-{number}");
        let operation = backend
            .start_clone(
                "main",
                StartCloneRequest {
                    source: CloneSource::Local {
                        source_handle: source_handle.source_handle.clone(),
                    },
                    destination_handle: destination.destination_handle.clone(),
                    directory_name: leaf.clone(),
                },
            )
            .await
            .unwrap();
        let service = backend.clone();
        let id = operation.id.clone();
        let cancel = tokio::spawn(async move {
            loop {
                let event = events.recv().await.unwrap();
                if event.operation.id == id && event.operation.phase == Phase::Publish {
                    return service
                        .cancel_operation("main", CancelOperationRequest { operation_id: id });
                }
            }
        });
        let result = finished(&backend, &operation.id).await;
        cancel.await.unwrap().unwrap();
        match result.state {
            OperationState::Succeeded => {
                assert_eq!(
                    std::fs::read(parent.join(&leaf).join("source.sentinel")).unwrap(),
                    b"original source"
                );
                assert_eq!(
                    backend
                        .cancel_operation(
                            "main",
                            CancelOperationRequest {
                                operation_id: operation.id.clone()
                            }
                        )
                        .unwrap()
                        .state,
                    OperationState::Succeeded
                );
            }
            OperationState::Cancelled => assert!(!parent.join(&leaf).exists()),
            _ => panic!("publish/cancel race must have one safe winner"),
        }
        let record = opengit_lib::clone::journal::load(&backend.registry.database)
            .unwrap()
            .into_iter()
            .find(|record| record.operation_id == operation.id)
            .unwrap();
        if record.cleanup.retained() {
            assert_eq!(result.state, OperationState::Cancelled);
            assert!(parent.join(&record.container).is_dir());
            assert!(result
                .error
                .as_ref()
                .unwrap()
                .message
                .ends_with(opengit_lib::git::error::RETENTION_NOTICE));
        } else {
            assert!(!parent.join(&record.container).exists());
        }
    }
}
