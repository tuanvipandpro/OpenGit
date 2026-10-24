use opengit_lib::{
    backend::Backend,
    domain::contracts::*,
    git::{
        refs::safe_remote,
        runner::{argv, Cancellation, GitRunner},
        status,
    },
};
use std::path::Path;
async fn git(r: &GitRunner, p: &Path, args: &[&str]) {
    r.run(
        Some(p),
        &argv(args),
        &Cancellation::default(),
        Phase::Discovery,
    )
    .await
    .unwrap();
}
fn request(workspace: &Workspace, generation: &str) -> InspectRepositoryRequest {
    let selection = workspace.active_selection.clone().unwrap();
    InspectRepositoryRequest {
        repo_id: selection.repo_id,
        worktree_id: selection.worktree_id,
        generation: generation.into(),
    }
}
#[test]
fn nul_parser_preserves_rename_unmerged_and_raw_non_utf8_paths() {
    let input=b"# branch.oid 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\0# branch.head main\0# future ignored\0\
1 .M N... 100644 100644 100644 ab cd path with \t space\0\
2 R. N... 100644 100644 100644 ab cd R100 renamed\nname\0original\tname\0\
u UU N... 100644 100644 100644 100644 ab cd ef conflict\0\
? raw\xffpath\0";
    let parsed = status::parse(input).unwrap();
    assert_eq!(parsed.entries.len(), 4);
    assert_eq!(parsed.head.kind, HeadKind::Branch);
    assert_eq!(parsed.head.oid.unwrap().len(), 64);
    assert_eq!(parsed.entries[1].path_display, "renamed\nname");
    assert_eq!(
        parsed.entries[1].previous_path_display.as_deref(),
        Some("original\tname")
    );
    assert_eq!(parsed.entries[2].kind, StatusEntryKind::Unmerged);
    assert_eq!(parsed.raw_paths[3].0, b"raw\xffpath");
    for bad in [
        b"1 bad\0".as_slice(),
        b"# branch.head main\0? no terminator",
        b"# branch.head main\0z unexpected\0",
        b"# branch.head main\0# branch.oid not-oid\0",
        b"# branch.head main\0\x32 R. N... 1 1 1 ab cd R100 file\0",
    ] {
        assert!(status::parse(bad).is_err());
    }
    assert_eq!(
        safe_remote("https://user:CANARY@example.invalid/repo?token=CANARY#CANARY").as_deref(),
        Some("https://example.invalid/repo")
    );
    assert!(safe_remote("ext::CANARY").is_none());
    assert_eq!(
        safe_remote("user@example.invalid:repo.git").as_deref(),
        Some("example.invalid:repo.git")
    );
    assert_eq!(
        safe_remote(
            "git@example.invalid:repo.git?token=REVIEW_SYNTHETIC_CANARY#REVIEW_SYNTHETIC_CANARY"
        )
        .as_deref(),
        Some("example.invalid:repo.git")
    );
    assert_eq!(safe_remote("REVIEW_SYNTHETIC_CANARY@example.invalid:repo.git#REVIEW_SYNTHETIC_CANARY?token=REVIEW_SYNTHETIC_CANARY").as_deref(),Some("example.invalid:repo.git"));
}

#[tokio::test]
async fn scp_query_fragment_and_username_are_removed_from_snapshot_and_journal() {
    use opengit_lib::{
        operations::scheduler::{OperationSpec, Scheduler},
        persistence::workspace::Database,
    };
    use std::sync::Arc;
    let fixture = tempfile::tempdir().unwrap();
    let repository = fixture.path().join("repository");
    std::fs::create_dir(&repository).unwrap();
    let runner = GitRunner::discover().unwrap();
    git(&runner, &repository, &["init"]).await;
    let source="REVIEW_SYNTHETIC_CANARY@example.invalid:repo.git?token=REVIEW_SYNTHETIC_CANARY#REVIEW_SYNTHETIC_CANARY";
    git(&runner, &repository, &["remote", "add", "origin", source]).await;
    let backend = Backend::with_git(&fixture.path().join("workspace-db"), runner).unwrap();
    let workspace = backend
        .register_repository("main", &repository)
        .await
        .unwrap();
    let snapshot = backend
        .inspect_repository("main", request(&workspace, "1"))
        .await
        .unwrap();
    assert_eq!(
        snapshot.remotes[0].fetch_url_display.as_deref(),
        Some("example.invalid:repo.git")
    );
    assert_eq!(
        snapshot.remotes[0].push_url_display.as_deref(),
        Some("example.invalid:repo.git")
    );
    assert!(!serde_json::to_string(&snapshot)
        .unwrap()
        .contains("REVIEW_SYNTHETIC_CANARY"));
    let directory = fixture.path().join("journal-db");
    let database = Arc::new(Database::open(&directory).unwrap());
    let scheduler = Scheduler::new(database.clone()).unwrap();
    let operation = scheduler
        .enqueue(OperationSpec {
            source_display: source.into(),
            destination_display: "Selected destination".into(),
            reservation: None,
        })
        .unwrap();
    assert_eq!(operation.source_display, "example.invalid:repo.git");
    assert!(!serde_json::to_string(&scheduler.snapshot().unwrap())
        .unwrap()
        .contains("REVIEW_SYNTHETIC_CANARY"));
    drop(scheduler);
    drop(database);
    let bytes = std::fs::read(directory.join("workspace.sqlite3")).unwrap();
    assert!(!bytes
        .windows(b"REVIEW_SYNTHETIC_CANARY".len())
        .any(|window| window == b"REVIEW_SYNTHETIC_CANARY"));
    let reopened = Scheduler::new(Arc::new(Database::open(&directory).unwrap())).unwrap();
    assert!(!serde_json::to_string(&reopened.snapshot().unwrap())
        .unwrap()
        .contains("REVIEW_SYNTHETIC_CANARY"));
}
#[tokio::test]
async fn real_merge_conflict_and_rebase_marker_are_detected_read_only() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("conflict");
    std::fs::create_dir(&root).unwrap();
    let runner = GitRunner::discover().unwrap();
    git(&runner, &root, &["init", "-b", "main"]).await;
    git(&runner, &root, &["config", "user.name", "Fixture"]).await;
    git(
        &runner,
        &root,
        &["config", "user.email", "fixture@example.invalid"],
    )
    .await;
    std::fs::write(root.join("conflict Ω.txt"), "base\n").unwrap();
    git(&runner, &root, &["add", "."]).await;
    git(&runner, &root, &["commit", "-m", "base"]).await;
    git(&runner, &root, &["checkout", "-b", "side"]).await;
    std::fs::write(root.join("conflict Ω.txt"), "side\n").unwrap();
    git(&runner, &root, &["commit", "-am", "side"]).await;
    git(&runner, &root, &["checkout", "main"]).await;
    std::fs::write(root.join("conflict Ω.txt"), "main\n").unwrap();
    git(&runner, &root, &["commit", "-am", "main"]).await;
    assert!(runner
        .run(
            Some(&root),
            &argv(&["merge", "side"]),
            &Cancellation::default(),
            Phase::Inspection
        )
        .await
        .is_err());
    let backend = Backend::with_git(&temp.path().join("db"), runner).unwrap();
    let workspace = backend.register_repository("main", &root).await.unwrap();
    let snapshot = backend
        .inspect_repository("main", request(&workspace, "1"))
        .await
        .unwrap();
    assert_eq!(snapshot.status.len(), 1);
    assert_eq!(snapshot.status[0].kind, StatusEntryKind::Unmerged);
    assert_eq!(snapshot.status[0].index_status, PathStatus::U);
    assert!(snapshot.git_operations.contains(&GitOperation::Merge));
    let before = std::fs::read(root.join("conflict Ω.txt")).unwrap();
    std::fs::create_dir(root.join(".git/rebase-merge")).unwrap();
    let snapshot = backend
        .inspect_repository("main", request(&workspace, "2"))
        .await
        .unwrap();
    assert!(snapshot.git_operations.contains(&GitOperation::Rebase));
    assert_eq!(before, std::fs::read(root.join("conflict Ω.txt")).unwrap());
    assert!(root.join(".git/MERGE_HEAD").exists());
}
#[tokio::test]
async fn unborn_dirty_rename_detached_bare_external_operation_and_stale_scope() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo Ω");
    std::fs::create_dir(&root).unwrap();
    let runner = GitRunner::discover().unwrap();
    git(&runner, &root, &["init"]).await;
    let backend = Backend::with_git(&temp.path().join("db"), runner.clone()).unwrap();
    let workspace = backend.register_repository("main", &root).await.unwrap();
    let snapshot = backend
        .inspect_repository("main", request(&workspace, "9"))
        .await
        .unwrap();
    assert_eq!(snapshot.head.kind, HeadKind::Unborn);
    assert_eq!(snapshot.generation, "9");
    assert_eq!(snapshot.is_dirty, Some(false));
    std::fs::write(root.join("before name Ω.txt"), "initial").unwrap();
    git(&runner, &root, &["add", "."]).await;
    git(
        &runner,
        &root,
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
    std::fs::rename(
        root.join("before name Ω.txt"),
        root.join("after name Ω.txt"),
    )
    .unwrap();
    git(&runner, &root, &["add", "-A"]).await;
    std::fs::write(root.join("untracked & literal.txt"), "dirty").unwrap();
    git(
        &runner,
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://user:CANARY@example.invalid/repo?token=CANARY",
        ],
    )
    .await;
    let snapshot = backend
        .inspect_repository("main", request(&workspace, "10"))
        .await
        .unwrap();
    assert_eq!(snapshot.status.len(), 2);
    assert!(snapshot
        .status
        .iter()
        .any(|e| e.kind == StatusEntryKind::Rename));
    assert_eq!(snapshot.is_dirty, Some(true));
    assert_eq!(
        snapshot.remotes[0].fetch_url_display.as_deref(),
        Some("https://example.invalid/repo")
    );
    std::fs::write(root.join(".git/MERGE_HEAD"), "1234").unwrap();
    assert!(backend
        .inspect_repository("main", request(&workspace, "11"))
        .await
        .unwrap()
        .git_operations
        .contains(&GitOperation::Merge));
    std::fs::remove_file(root.join(".git/MERGE_HEAD")).unwrap();
    git(&runner, &root, &["checkout", "--detach"]).await;
    let snapshot = backend
        .inspect_repository("main", request(&workspace, "12"))
        .await
        .unwrap();
    assert_eq!(snapshot.head.kind, HeadKind::Detached);
    assert!(snapshot.head.branch.is_none());
    assert_eq!(
        backend
            .inspect_repository("unknown-window", request(&workspace, "13"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    assert_eq!(
        backend
            .inspect_repository("main", request(&workspace, "bad"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let bare = temp.path().join("bare.git");
    std::fs::create_dir(&bare).unwrap();
    git(&runner, &bare, &["init", "--bare"]).await;
    let bare_workspace = backend.register_repository("main", &bare).await.unwrap();
    let snapshot = backend
        .inspect_repository("main", request(&bare_workspace, "14"))
        .await
        .unwrap();
    assert!(snapshot.is_bare);
    assert!(snapshot.status.is_empty());
    assert_eq!(snapshot.is_dirty, None);
    assert!(snapshot.git_operations.is_empty());
    // Replacing a repository at its registered display path must not redirect the handle.
    let moved = temp.path().join("moved");
    std::fs::rename(&root, &moved).unwrap();
    std::fs::create_dir(&root).unwrap();
    git(&runner, &root, &["init"]).await;
    assert_eq!(
        backend
            .inspect_repository("main", request(&workspace, "15"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::ExternalChange
    );
    backend.close_window("main");
    assert_eq!(
        backend
            .inspect_repository("main", request(&workspace, "16"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
}
