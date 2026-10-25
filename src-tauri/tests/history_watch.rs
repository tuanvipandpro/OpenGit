#[allow(dead_code)]
mod support;
use opengit_lib::{backend::Backend, domain::contracts::*, history::watch::RoutedChange};
use std::time::Duration;
use support::history::*;
use tokio::sync::broadcast;

async fn activate(backend: &Backend, scope: &HistoryScope) -> RepositoryWatch {
    backend
        .watch_repository(
            "main",
            WatchRepositoryRequest {
                scope: scope.clone(),
            },
        )
        .await
        .unwrap()
}
fn drain(receiver: &mut broadcast::Receiver<RoutedChange>) {
    while receiver.try_recv().is_ok() {}
}
async fn event(
    receiver: &mut broadcast::Receiver<RoutedChange>,
    watch: &RepositoryWatch,
    reason: RepositoryChangeReason,
) -> RepositoryChangedEvent {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let change = receiver.recv().await.unwrap();
            if change.event.subscription_id == watch.subscription_id
                && change.event.reason == reason
            {
                assert_eq!(change.window, "main");
                assert_eq!(change.event.scope, watch.scope);
                assert!(change.event.sequence.parse::<u64>().unwrap() > 0);
                return change.event;
            }
        }
    })
    .await
    .unwrap()
}
async fn quiet(receiver: &mut broadcast::Receiver<RoutedChange>) {
    assert!(
        tokio::time::timeout(Duration::from_millis(400), receiver.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn actual_worktree_bursts_are_path_free_and_atomic_git_metadata_rewrites_are_observed() {
    let fixture = Fixture::new("sha1", 8);
    let (backend, scope) = fixture.backend().await;
    let mut events = backend.subscribe_repository_changes();
    let before = fixture.snapshot();
    let watch = activate(&backend, &scope).await;
    assert_eq!(watch.status, WatchStatus::Watching);
    assert_eq!(watch.sequence, "0");
    assert_eq!(fixture.snapshot(), before);
    for i in 0..30 {
        std::fs::write(
            fixture
                .path
                .join(format!("PRIVATE_PATH_NEVER_IN_EVENT-{i}")),
            b"SYNTHETIC_CONTENT_NEVER_IN_EVENT",
        )
        .unwrap();
    }
    let first = event(&mut events, &watch, RepositoryChangeReason::Changed).await;
    let json = serde_json::to_string(&first).unwrap();
    assert!(!json.contains("PRIVATE_PATH"));
    assert!(!json.contains("SYNTHETIC_CONTENT"));
    assert!(!json.contains("paths"));
    assert!(!json.contains("history"));
    tokio::time::sleep(Duration::from_millis(250)).await;
    drain(&mut events);
    fixture.run(&["update-ref", "refs/heads/watched", "main"], None);
    fixture.run(&["pack-refs", "--all", "--prune"], None);
    event(&mut events, &watch, RepositoryChangeReason::Changed).await;
    drain(&mut events);
    fixture.run(&["update-ref", "refs/heads/watched", "alpha"], None);
    fixture.run(&["pack-refs", "--all", "--prune"], None);
    let next = event(&mut events, &watch, RepositoryChangeReason::Changed).await;
    assert!(next.sequence.parse::<u64>().unwrap() > first.sequence.parse::<u64>().unwrap());
    let session = open(&backend, &scope, 200, filter()).await;
    assert_eq!(
        session
            .refs
            .iter()
            .find(|r| r.name == "refs/heads/watched")
            .unwrap()
            .commit_oid,
        Some(fixture.oid("alpha"))
    );
    backend
        .unwatch_repository(
            "main",
            UnwatchRepositoryRequest {
                subscription_id: watch.subscription_id,
            },
        )
        .unwrap();
}
#[tokio::test]
async fn linked_worktree_root_and_resolved_common_git_directories_both_produce_hints() {
    let fixture = Fixture::new("sha1", 8);
    let linked = fixture.owner.child("linked Ω").unwrap();
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
    let backend = Backend::with_git(
        &fixture.owner.child("watch-db").unwrap(),
        fixture.git.clone(),
    )
    .unwrap();
    let selection = backend
        .register_repository("main", &linked)
        .await
        .unwrap()
        .active_selection
        .unwrap();
    let scope = HistoryScope {
        repo_id: selection.repo_id,
        worktree_id: selection.worktree_id,
        generation: "1".into(),
    };
    let mut events = backend.subscribe_repository_changes();
    let watch = activate(&backend, &scope).await;
    assert_eq!(watch.status, WatchStatus::Watching);
    std::fs::write(linked.join("linked-worktree-event"), b"change").unwrap();
    event(&mut events, &watch, RepositoryChangeReason::Changed).await;
    drain(&mut events);
    fixture.run(&["update-ref", "refs/heads/from-common", "alpha"], None);
    event(&mut events, &watch, RepositoryChangeReason::Changed).await;
    drain(&mut events);
    let git_dir = backend
        .registry
        .location(&scope.repo_id, scope.worktree_id.as_deref())
        .unwrap()
        .git_dir;
    std::fs::write(git_dir.join("watched-marker"), b"change").unwrap();
    event(&mut events, &watch, RepositoryChangeReason::Changed).await;
    backend.close_window("main");
}
#[tokio::test]
async fn replacement_unwatch_window_binding_and_close_suppress_old_scope_events() {
    let fixture = Fixture::new("sha1", 8);
    let (backend, scope) = fixture.backend().await;
    let mut events = backend.subscribe_repository_changes();
    let first = activate(&backend, &scope).await;
    let second = activate(&backend, &scope).await;
    assert_ne!(first.subscription_id, second.subscription_id);
    backend.get_workspace("other").unwrap();
    assert_eq!(
        backend
            .unwatch_repository(
                "other",
                UnwatchRepositoryRequest {
                    subscription_id: first.subscription_id.clone()
                }
            )
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    for _ in 0..2 {
        backend
            .unwatch_repository(
                "main",
                UnwatchRepositoryRequest {
                    subscription_id: first.subscription_id.clone(),
                },
            )
            .unwrap();
    }
    drain(&mut events);
    std::fs::write(fixture.path.join("only-new-token"), b"new").unwrap();
    let change = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(change.event.subscription_id, second.subscription_id);
    backend
        .unwatch_repository(
            "main",
            UnwatchRepositoryRequest {
                subscription_id: second.subscription_id.clone(),
            },
        )
        .unwrap();
    drain(&mut events);
    std::fs::write(fixture.path.join("unwatched"), b"none").unwrap();
    quiet(&mut events).await;
    let third = activate(&backend, &scope).await;
    backend.close_window("main");
    drain(&mut events);
    std::fs::write(fixture.path.join("closed"), b"none").unwrap();
    quiet(&mut events).await;
    assert_eq!(
        backend
            .unwatch_repository(
                "main",
                UnwatchRepositoryRequest {
                    subscription_id: third.subscription_id
                }
            )
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
}
#[tokio::test]
async fn external_directory_rename_is_permitted_reports_unavailable_and_same_identity_rearms() {
    let fixture = Fixture::new("sha1", 8);
    let (backend, scope) = fixture.backend().await;
    let mut events = backend.subscribe_repository_changes();
    let watch = activate(&backend, &scope).await;
    assert_eq!(watch.status, WatchStatus::Watching);
    let moved = fixture.owner.child("moved-history").unwrap();
    std::fs::rename(&fixture.path, &moved).unwrap();
    event(&mut events, &watch, RepositoryChangeReason::Unavailable).await;
    std::fs::rename(&moved, &fixture.path).unwrap();
    event(&mut events, &watch, RepositoryChangeReason::Changed).await;
    drain(&mut events);
    std::fs::write(fixture.path.join("after-rearm"), b"actual new native event").unwrap();
    event(&mut events, &watch, RepositoryChangeReason::Changed).await;
    backend.close_window("main");
}
#[tokio::test]
async fn select_and_forget_revoke_watchers_and_invalid_generations_do_not_replace_them() {
    let fixture = Fixture::new("sha1", 8);
    let (backend, scope) = fixture.backend().await;
    let mut events = backend.subscribe_repository_changes();
    let watch = activate(&backend, &scope).await;
    let mut newer = scope.clone();
    newer.generation = "2".into();
    let newest = activate(&backend, &newer).await;
    assert_eq!(
        backend
            .watch_repository(
                "main",
                WatchRepositoryRequest {
                    scope: scope.clone()
                }
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    let other = fixture.owner.child("other-repo").unwrap();
    std::fs::create_dir(&other).unwrap();
    fixture.run_at(&other, &["init"], None);
    backend.register_repository("main", &other).await.unwrap();
    drain(&mut events);
    std::fs::write(fixture.path.join("switched-away"), b"none").unwrap();
    quiet(&mut events).await;
    backend
        .unwatch_repository(
            "main",
            UnwatchRepositoryRequest {
                subscription_id: newest.subscription_id,
            },
        )
        .unwrap();
    backend
        .unwatch_repository(
            "main",
            UnwatchRepositoryRequest {
                subscription_id: watch.subscription_id,
            },
        )
        .unwrap();
    backend
        .select_repository(
            "main",
            SelectRepositoryRequest {
                repo_id: scope.repo_id.clone(),
                worktree_id: scope.worktree_id.clone(),
            },
        )
        .unwrap();
    let watch = activate(&backend, &newer).await;
    backend
        .forget_repository(
            "main",
            ForgetRepositoryRequest {
                repo_id: scope.repo_id,
            },
        )
        .unwrap();
    drain(&mut events);
    std::fs::write(fixture.path.join("forgotten"), b"none").unwrap();
    quiet(&mut events).await;
    backend
        .unwatch_repository(
            "main",
            UnwatchRepositoryRequest {
                subscription_id: watch.subscription_id,
            },
        )
        .unwrap();
    assert!(fixture.path.is_dir());
}
