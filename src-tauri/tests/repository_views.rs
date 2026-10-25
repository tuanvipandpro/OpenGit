#[allow(dead_code)]
mod support;
use opengit_lib::{domain::contracts::*, persistence::workspace::Database};
use std::sync::Arc;
use support::history::*;
async fn backend(fixture: &Fixture) -> (opengit_lib::backend::Backend, HistoryScope) {
    let backend = opengit_lib::backend::Backend::with_git(
        &fixture.owner.child("database").unwrap(),
        fixture.git.clone(),
    )
    .unwrap();
    let selection = backend
        .register_repository("main", &fixture.path)
        .await
        .unwrap()
        .active_selection
        .unwrap();
    (
        backend,
        HistoryScope {
            repo_id: selection.repo_id,
            worktree_id: selection.worktree_id,
            generation: "1".into(),
        },
    )
}

fn get(backend: &opengit_lib::backend::Backend, scope: &HistoryScope) -> RepositoryViewState {
    backend
        .get_repository_view(
            "main",
            GetRepositoryViewRequest {
                scope: scope.clone(),
            },
        )
        .unwrap()
}
fn request(state: &RepositoryViewState) -> SaveRepositoryViewRequest {
    SaveRepositoryViewRequest {
        scope: state.scope.clone(),
        expected_revision: state.view_revision.clone(),
        view: state.view.clone(),
    }
}
#[tokio::test]
async fn per_repository_view_reopens_with_oid_hints_filters_layout_and_no_runtime_state() {
    let fixture = Fixture::new("sha1", 8);
    let tracked = fixture.path.join("tracked");
    std::fs::write(&tracked, b"base").unwrap();
    fixture.run(&["add", "--", "tracked"], None);
    fixture.run(&["commit", "-m", "Tracked base"], None);
    std::fs::write(&tracked, b"changed").unwrap();
    fixture.run(&["stash", "push", "-m", "Persist selected stash"], None);
    let stash_oid = fixture.oid("refs/stash");
    let (backend, scope) = backend(&fixture).await;
    let initial = get(&backend, &scope);
    assert_eq!(initial.view_revision, "0");
    assert_eq!(initial.view.sidebar_width, 280);
    assert_eq!(initial.view.details_height, 320);
    let mut save = request(&initial);
    save.view.filter.hidden_refs = vec!["refs/tags/v1".into(), "refs/tags/v1".into()];
    save.view.filter.solo_refs = vec!["refs/heads/main".into(), "refs/stash".into()];
    save.view.selected_oid = Some("a".repeat(64));
    save.view.anchor_oid = Some("b".repeat(40));
    save.view.anchor_offset = -32.5;
    save.view.sidebar_collapsed = true;
    save.view.diff_layout = DiffLayout::Inline;
    let before = fixture.snapshot();
    let stored = backend.save_repository_view("main", save).unwrap();
    assert_eq!(stored.view_revision, "1");
    assert_eq!(stored.view.filter.hidden_refs, ["refs/tags/v1"]);
    assert_eq!(stored.view.selected_oid, Some("a".repeat(64)));
    assert_eq!(fixture.snapshot(), before);
    let mut selected_stash = request(&stored);
    selected_stash.view.selected_oid = Some(stash_oid.clone());
    let stored = backend
        .save_repository_view("main", selected_stash)
        .unwrap();
    assert_eq!(stored.view.selected_oid, Some(stash_oid));
    assert_eq!(stored.view_revision, "2");
    assert_eq!(fixture.snapshot(), before);
    let data = fixture.owner.child("database").unwrap();
    // Fixture backend uses this data directory; persisted contents are checked
    // through the database's public path-independent SQL, not runtime handles.
    let connection = rusqlite::Connection::open(data.join("workspace.sqlite3")).unwrap();
    let value: String = connection
        .query_row("SELECT value FROM repository_views", [], |r| r.get(0))
        .unwrap();
    for forbidden in [
        "sessionId",
        "snapshotId",
        "commitHandle",
        "fileHandle",
        "cursor",
        "credential",
    ] {
        assert!(!value.contains(forbidden));
    }
    drop(connection);
    drop(backend);
    let backend = opengit_lib::backend::Backend::new(&data).unwrap();
    backend.get_workspace("main").unwrap();
    assert_eq!(get(&backend, &scope), stored);
}
#[tokio::test]
async fn invalid_layout_oids_and_snapshot_stash_filters_are_rejected_without_updating_revision() {
    let fixture = Fixture::new("sha1", 8);
    let (backend, scope) = backend(&fixture).await;
    let state = get(&backend, &scope);
    let mut cases = Vec::new();
    for oid in [
        "a".repeat(39),
        "a".repeat(41),
        "g".repeat(40),
        "a".repeat(63),
        "a".repeat(65),
    ] {
        let mut bad = request(&state);
        bad.view.selected_oid = Some(oid);
        cases.push(bad);
    }
    for offset in [f64::NAN, f64::INFINITY, 10_001.0, -10_001.0] {
        let mut bad = request(&state);
        bad.view.anchor_offset = offset;
        cases.push(bad);
    }
    let mut bad = request(&state);
    bad.view.sidebar_width = 159;
    cases.push(bad);
    let mut bad = request(&state);
    bad.view.details_height = 1201;
    cases.push(bad);
    let mut bad = request(&state);
    bad.view.filter.solo_refs = vec!["--all".into()];
    cases.push(bad);
    let mut bad = request(&state);
    bad.view.filter.hidden_refs = vec!["refs/stash/1".into()];
    cases.push(bad);
    for bad in cases {
        assert_eq!(
            backend.save_repository_view("main", bad).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(get(&backend, &scope), state);
    }
    let mut good = request(&state);
    good.view.selected_oid = Some(fixture.oid("main"));
    assert_eq!(
        backend
            .save_repository_view("main", good)
            .unwrap()
            .view_revision,
        "1"
    );
}
#[tokio::test]
async fn concurrent_cas_stale_generation_closed_window_and_switch_cannot_overwrite_newer_view() {
    let fixture = Fixture::new("sha1", 8);
    let (backend, scope) = backend(&fixture).await;
    let backend = Arc::new(backend);
    let initial = get(&backend, &scope);
    let mut first = request(&initial);
    first.view.sidebar_width = 300;
    let mut second = first.clone();
    second.view.sidebar_width = 400;
    let a = backend.clone();
    let b = backend.clone();
    let results = tokio::join!(
        tokio::task::spawn_blocking(move || a.save_repository_view("main", first)),
        tokio::task::spawn_blocking(move || b.save_repository_view("main", second))
    );
    let results = [results.0.unwrap(), results.1.unwrap()];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .next()
            .unwrap()
            .code,
        ErrorCode::ExternalChange
    );
    let winner = get(&backend, &scope);
    assert_eq!(winner.view_revision, "1");
    let old = request(&winner);
    let mut current = scope.clone();
    current.generation = "2".into();
    get(&backend, &current);
    assert_eq!(
        backend
            .save_repository_view("main", old.clone())
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    backend.close_window("main");
    assert_eq!(
        backend
            .save_repository_view("main", old.clone())
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    backend.get_workspace("main").unwrap();
    let second = fixture.owner.child("second").unwrap();
    std::fs::create_dir(&second).unwrap();
    fixture.run_at(&second, &["init"], None);
    backend.register_repository("main", &second).await.unwrap();
    assert_eq!(
        backend.save_repository_view("main", old).unwrap_err().code,
        ErrorCode::InvalidHandle
    );
    backend
        .select_repository(
            "main",
            SelectRepositoryRequest {
                repo_id: scope.repo_id.clone(),
                worktree_id: scope.worktree_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(get(&backend, &current).view, winner.view);
}
#[tokio::test]
async fn linked_worktrees_share_view_revision_and_forget_cascades_without_git_mutation() {
    let fixture = Fixture::new("sha1", 8);
    let (backend, scope) = backend(&fixture).await;
    let stored = backend
        .save_repository_view("main", request(&get(&backend, &scope)))
        .unwrap();
    let linked = fixture.owner.child("linked").unwrap();
    fixture.run(
        &[
            "worktree",
            "add",
            "-b",
            "linked",
            &fixture.git_path(&linked),
        ],
        None,
    );
    let workspace = backend.register_repository("main", &linked).await.unwrap();
    let selection = workspace.active_selection.unwrap();
    assert_eq!(selection.repo_id, scope.repo_id);
    assert_ne!(selection.worktree_id, scope.worktree_id);
    let linked_scope = HistoryScope {
        repo_id: selection.repo_id,
        worktree_id: selection.worktree_id,
        generation: "2".into(),
    };
    let current = get(&backend, &linked_scope);
    assert_eq!(current.view, stored.view);
    assert_eq!(current.view_revision, stored.view_revision);
    backend
        .forget_repository(
            "main",
            ForgetRepositoryRequest {
                repo_id: scope.repo_id,
            },
        )
        .unwrap();
    assert_eq!(
        backend
            .get_repository_view(
                "main",
                GetRepositoryViewRequest {
                    scope: linked_scope
                }
            )
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    let connection = rusqlite::Connection::open(
        fixture
            .owner
            .child("database")
            .unwrap()
            .join("workspace.sqlite3"),
    )
    .unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM repository_views", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(linked.is_dir());
}
#[tokio::test]
async fn sql_failure_and_revision_overflow_preserve_previous_view_atomically() {
    let fixture = Fixture::new("sha1", 8);
    let (backend, scope) = backend(&fixture).await;
    let stored = backend
        .save_repository_view("main", request(&get(&backend, &scope)))
        .unwrap();
    let connection = rusqlite::Connection::open(
        fixture
            .owner
            .child("database")
            .unwrap()
            .join("workspace.sqlite3"),
    )
    .unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_view BEFORE UPDATE ON repository_views BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let mut next = request(&stored);
    next.view.sidebar_width = 500;
    assert_eq!(
        backend.save_repository_view("main", next).unwrap_err().code,
        ErrorCode::Database
    );
    assert_eq!(get(&backend, &scope), stored);
    connection
        .execute_batch(
            "DROP TRIGGER refuse_view; UPDATE repository_views SET revision=9223372036854775807;",
        )
        .unwrap();
    let max = get(&backend, &scope);
    assert_eq!(
        backend
            .save_repository_view("main", request(&max))
            .unwrap_err()
            .code,
        ErrorCode::OutputLimit
    );
    assert_eq!(get(&backend, &scope), max);
    let malformed = "{\"fileHandle\":\"not-a-preference\"}";
    connection
        .execute("UPDATE repository_views SET value=?1", [malformed])
        .unwrap();
    assert_eq!(
        backend
            .get_repository_view(
                "main",
                GetRepositoryViewRequest {
                    scope: scope.clone()
                }
            )
            .unwrap_err()
            .code,
        ErrorCode::Database
    );
    assert_eq!(
        connection
            .query_row("SELECT value FROM repository_views", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        malformed
    );
    connection
        .execute_batch("PRAGMA ignore_check_constraints=ON")
        .unwrap();
    connection
        .execute("UPDATE repository_views SET value=?1", ["x".repeat(20_000)])
        .unwrap();
    assert_eq!(
        backend
            .get_repository_view("main", GetRepositoryViewRequest { scope })
            .unwrap_err()
            .code,
        ErrorCode::Database
    );
}
#[test]
fn migrations_zero_one_two_to_three_preserve_rows_and_failures_roll_back_the_entire_upgrade() {
    for version in 0..=2 {
        for fail in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let file = directory.path().join("workspace.sqlite3");
            let connection = rusqlite::Connection::open(&file).unwrap();
            if version >= 1 {
                connection
                    .execute_batch(include_str!("../migrations/0001_initial.sql"))
                    .unwrap();
                connection
                    .execute(
                        "INSERT INTO workspace(id,revision) VALUES('workspace-sentinel',7)",
                        [],
                    )
                    .unwrap();
                connection.execute("INSERT INTO operations(id,snapshot) VALUES('operation-sentinel','snapshot-sentinel')",[]).unwrap();
            }
            if version >= 2 {
                connection
                    .execute_batch(include_str!("../migrations/0002_clone_intents.sql"))
                    .unwrap();
                connection.execute("INSERT INTO clone_intents(operation_id,context) VALUES('operation-sentinel','context-sentinel')",[]).unwrap();
            }
            if fail {
                connection
                    .execute("CREATE TABLE repository_views(sentinel TEXT)", [])
                    .unwrap();
                connection
                    .execute("INSERT INTO repository_views VALUES('preserve-me')", [])
                    .unwrap();
            }
            connection
                .pragma_update(None, "user_version", version)
                .unwrap();
            drop(connection);
            let result = Database::open(directory.path());
            assert_eq!(result.is_err(), fail);
            drop(result);
            let connection = rusqlite::Connection::open(&file).unwrap();
            assert_eq!(
                connection
                    .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                if fail { version } else { 3 }
            );
            if version >= 1 {
                assert_eq!(
                    connection
                        .query_row("SELECT id FROM workspace", [], |r| r.get::<_, String>(0))
                        .unwrap(),
                    "workspace-sentinel"
                );
                assert_eq!(
                    connection
                        .query_row("SELECT snapshot FROM operations", [], |r| r
                            .get::<_, String>(0))
                        .unwrap(),
                    "snapshot-sentinel"
                );
            }
            if version >= 2 {
                assert_eq!(
                    connection
                        .query_row("SELECT context FROM clone_intents", [], |r| r
                            .get::<_, String>(0))
                        .unwrap(),
                    "context-sentinel"
                );
            }
            if fail {
                assert_eq!(
                    connection
                        .query_row("SELECT sentinel FROM repository_views", [], |r| r
                            .get::<_, String>(0))
                        .unwrap(),
                    "preserve-me"
                );
                if version == 0 {
                    assert_eq!(
                        connection
                            .query_row(
                                "SELECT count(*) FROM sqlite_master WHERE name='workspace'",
                                [],
                                |r| r.get::<_, i64>(0)
                            )
                            .unwrap(),
                        0
                    );
                }
            }
        }
    }
}
