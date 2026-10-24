use opengit_lib::{
    domain::contracts::*,
    git::runner::{argv, Cancellation, GitRunner},
    persistence::workspace::Database,
    repositories::{discovery, registry::Registry},
};
use std::sync::Arc;
async fn git(r: &GitRunner, p: &std::path::Path, args: &[&str]) {
    r.run(
        Some(p),
        &argv(args),
        &Cancellation::default(),
        Phase::Discovery,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn stable_ids_alias_linked_nested_bare_and_forget_without_delete() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("db");
    let root = temp.path().join("repo Ω space");
    std::fs::create_dir(&root).unwrap();
    let runner = GitRunner::discover().unwrap();
    git(&runner, &root, &["init"]).await;
    git(
        &runner,
        &root,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "fixture",
        ],
    )
    .await;
    let registry = Registry {
        database: Arc::new(Database::open(&data).unwrap()),
    };
    let discovered = discovery::discover(&runner, &root).await.unwrap();
    let first = registry.register(&discovered).unwrap();
    let id = first.repositories[0].id.clone();
    let tree = first.repositories[0].worktrees[0].id.clone();
    let alias = discovery::discover(&runner, &root.join(".")).await.unwrap();
    let again = registry.register(&alias).unwrap();
    assert_eq!(again.repositories.len(), 1);
    assert_eq!(again.repositories[0].id, id);
    assert_eq!(again.repositories[0].worktrees[0].id, tree);
    let linked = temp.path().join("linked Ω");
    git(
        &runner,
        &root,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    )
    .await;
    registry
        .register(&discovery::discover(&runner, &linked).await.unwrap())
        .unwrap();
    assert_eq!(
        registry.workspace().unwrap().repositories[0]
            .worktrees
            .len(),
        2
    );
    let nested = root.join("nested");
    std::fs::create_dir(&nested).unwrap();
    git(&runner, &nested, &["init"]).await;
    assert_eq!(registry.workspace().unwrap().repositories.len(), 1);
    registry
        .register(&discovery::discover(&runner, &nested).await.unwrap())
        .unwrap();
    let bare = temp.path().join("bare.git");
    std::fs::create_dir(&bare).unwrap();
    git(&runner, &bare, &["init", "--bare"]).await;
    let last = registry
        .register(&discovery::discover(&runner, &bare).await.unwrap())
        .unwrap();
    assert_eq!(last.repositories.len(), 3);
    assert!(last.repositories[0].is_bare);
    assert!(last.repositories[0].worktrees.is_empty());
    let bare_id = last.repositories[0].id.clone();
    drop(registry);
    let reopened = Registry {
        database: Arc::new(Database::open(&data).unwrap()),
    };
    let workspace = reopened.workspace().unwrap();
    assert_eq!(last, workspace);
    let workspace = reopened.forget(&bare_id).unwrap();
    assert!(workspace.active_selection.is_none());
    assert!(bare.join("HEAD").is_file());
    assert!(reopened.location(&id, Some(&tree)).is_ok());
    assert!(reopened.location(&id, None).is_err());
}
#[test]
fn corrupt_and_failed_migration_preserve_original_database() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("workspace.sqlite3");
    let original = b"not a sqlite file CANARY";
    std::fs::write(&path, original).unwrap();
    assert!(matches!(
        Database::open(temp.path()),
        Err(AppError {
            code: ErrorCode::Database,
            ..
        })
    ));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let second = tempfile::tempdir().unwrap();
    let path = second.path().join("workspace.sqlite3");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("CREATE TABLE worktrees(sentinel TEXT)", [])
        .unwrap();
    drop(connection);
    assert!(Database::open(second.path()).is_err());
    let connection = rusqlite::Connection::open(path).unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='workspace'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 0);
}
#[tokio::test]
async fn registry_transaction_rolls_back_if_selection_update_fails() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("db");
    let registry = Registry {
        database: Arc::new(Database::open(&data).unwrap()),
    };
    let before = registry.workspace().unwrap();
    let connection = rusqlite::Connection::open(data.join("workspace.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_selection BEFORE UPDATE ON workspace BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let repo = temp.path().join("repository");
    std::fs::create_dir(&repo).unwrap();
    let runner = GitRunner::discover().unwrap();
    git(&runner, &repo, &["init"]).await;
    assert_eq!(
        registry
            .register(&discovery::discover(&runner, &repo).await.unwrap())
            .unwrap_err()
            .code,
        ErrorCode::Database
    );
    assert_eq!(registry.workspace().unwrap(), before);
    assert!(repo.join(".git/HEAD").is_file());
}
#[test]
fn readonly_database_migration_is_rejected_without_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("workspace.sqlite3");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("CREATE TABLE sentinel(value TEXT)", [])
        .unwrap();
    drop(connection);
    let original = std::fs::read(&path).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    let original_permissions = permissions.clone();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).unwrap();
    assert!(Database::open(temp.path()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::set_permissions(&path, original_permissions).unwrap();
}
#[cfg(windows)]
#[test]
fn raw_windows_path_blob_preserves_unpaired_utf16() {
    use opengit_lib::persistence::workspace::{decode, encode};
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};
    let path = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xd800, 0x03a9]));
    assert_eq!(decode(encode(&path)).unwrap(), path);
    assert!(decode(vec![1]).is_err());
}
#[tokio::test]
async fn schema_zero_one_two_migrations_preserve_ids_and_failed_upgrade_rolls_back() {
    let fixture = tempfile::tempdir().unwrap();
    let data = fixture.path().join("data");
    let repository = fixture.path().join("repository");
    std::fs::create_dir(&repository).unwrap();
    let runner = GitRunner::discover().unwrap();
    git(&runner, &repository, &["init"]).await;
    let registry = Registry {
        database: Arc::new(Database::open(&data).unwrap()),
    };
    let workspace = registry
        .register(&discovery::discover(&runner, &repository).await.unwrap())
        .unwrap();
    drop(registry);
    let file = data.join("workspace.sqlite3");
    let connection = rusqlite::Connection::open(&file).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 2);
    connection
        .execute_batch("DROP TABLE clone_intents; PRAGMA user_version=1;")
        .unwrap();
    drop(connection);
    let registry = Registry {
        database: Arc::new(Database::open(&data).unwrap()),
    };
    assert_eq!(registry.workspace().unwrap(), workspace);
    drop(registry);
    let registry = Registry {
        database: Arc::new(Database::open(&data).unwrap()),
    };
    assert_eq!(registry.workspace().unwrap(), workspace);
    drop(registry);
    let connection = rusqlite::Connection::open(&file).unwrap();
    connection.execute_batch("PRAGMA user_version=1;").unwrap();
    drop(connection);
    assert!(Database::open(&data).is_err());
    let connection = rusqlite::Connection::open(&file).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 1);
    let id: String = connection
        .query_row("SELECT id FROM workspace", [], |row| row.get(0))
        .unwrap();
    assert_eq!(id, workspace.id);
    let active: Option<String> = connection
        .query_row("SELECT active_repo FROM workspace", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        active,
        workspace
            .active_selection
            .map(|selection| selection.repo_id)
    );
}
