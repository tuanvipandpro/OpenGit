use super::*;
async fn activation_race(before_begin: bool, forget: bool) {
    let (root, backend, scope, path) = fixture().await;
    let other_path = root.path().join("other");
    std::fs::create_dir(&other_path).unwrap();
    backend
        .git()
        .unwrap()
        .run(
            Some(&other_path),
            &crate::git::runner::argv(&["init", "-b", "other"]),
            &Default::default(),
            Phase::Discovery,
        )
        .await
        .unwrap();
    let other = backend
        .register_repository("main", &other_path)
        .await
        .unwrap()
        .active_selection
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
    let backend = Arc::new(backend);
    let mut events = backend.subscribe_repository_changes();
    let old = activate(&backend, &scope).await;
    let pause = Arc::new(TestPause::default());
    *if before_begin {
        backend.watch_test.before_begin.lock().unwrap()
    } else {
        backend.watch_test.activation_wait.lock().unwrap()
    } = Some(pause.clone());
    let request = WatchRepositoryRequest {
        scope: scope.clone(),
    };
    let service = backend.clone();
    let task = tokio::spawn(async move { service.watch_repository("main", request).await });
    tokio::time::timeout(Duration::from_secs(5), pause.entered.notified())
        .await
        .unwrap();
    // Both lifecycle operations must finish while activation is suspended;
    // neither native wait nor this test gate may retain DB/session locks.
    if forget {
        backend
            .forget_repository(
                "main",
                ForgetRepositoryRequest {
                    repo_id: scope.repo_id.clone(),
                },
            )
            .unwrap();
    } else {
        backend
            .select_repository(
                "main",
                SelectRepositoryRequest {
                    repo_id: other.repo_id.clone(),
                    worktree_id: other.worktree_id.clone(),
                },
            )
            .unwrap();
    }
    pause.release.notify_one();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    assert!(backend
        .watches
        .inner
        .state
        .lock()
        .unwrap()
        .windows
        .get("main")
        .and_then(|w| w.active.as_ref())
        .is_none());
    zero(&backend.watches.inner.count).await;
    backend
        .unwatch_repository(
            "main",
            UnwatchRepositoryRequest {
                subscription_id: old.subscription_id,
            },
        )
        .unwrap();
    while events.try_recv().is_ok() {}
    std::fs::write(
        path.join("after-lifecycle-race"),
        b"must not produce an old-token event",
    )
    .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(400), events.recv())
            .await
            .is_err()
    );
    assert!(path.is_dir());
    if !forget {
        let current = HistoryScope {
            repo_id: other.repo_id,
            worktree_id: other.worktree_id,
            generation: scope.generation,
        };
        assert_eq!(
            activate(&backend, &current).await.status,
            WatchStatus::Watching
        );
        backend.close_window("main");
        zero(&backend.watches.inner.count).await;
    }
}
#[tokio::test]
async fn s6a01_switch_between_precheck_and_begin_rejects_stale_activation() {
    activation_race(true, false).await;
}
#[tokio::test]
async fn s6a01_forget_between_precheck_and_begin_rejects_stale_activation() {
    activation_race(true, true).await;
}
#[tokio::test]
async fn s6a01_switch_or_forget_while_activation_waits_disposes_old_tokens() {
    for forget in [false, true] {
        activation_race(false, forget).await;
    }
}
#[tokio::test]
async fn s6a01_finalization_rechecks_committed_selection_and_registration_without_a_revocation_signal(
) {
    for registration_change in [false, true] {
        let (root, backend, scope, path) = fixture().await;
        let backend = Arc::new(backend);
        let mut events = backend.subscribe_repository_changes();
        let pause = Arc::new(TestPause::default());
        *backend.watch_test.activation_wait.lock().unwrap() = Some(pause.clone());
        let request = WatchRepositoryRequest {
            scope: scope.clone(),
        };
        let service = backend.clone();
        let task = tokio::spawn(async move { service.watch_repository("main", request).await });
        tokio::time::timeout(Duration::from_secs(5), pause.entered.notified())
            .await
            .unwrap();
        let token = backend.watches.inner.state.lock().unwrap().windows["main"]
            .active
            .as_ref()
            .unwrap()
            .token
            .clone();
        // Model the committed DB boundary before its subsequent session revoke.
        // A still-valid ticket alone must never authorize the final response.
        let connection =
            rusqlite::Connection::open(root.path().join("db/workspace.sqlite3")).unwrap();
        if registration_change {
            connection
                .execute(
                    "UPDATE repositories SET identity=?1 WHERE id=?2",
                    rusqlite::params![vec![0u8; 12], scope.repo_id],
                )
                .unwrap();
        } else {
            connection
                .execute(
                    "UPDATE workspace SET active_repo=NULL,active_worktree=NULL",
                    [],
                )
                .unwrap();
        }
        drop(connection);
        pause.release.notify_one();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err()
                .code,
            ErrorCode::InvalidHandle
        );
        assert!(backend.watches.inner.state.lock().unwrap().windows["main"]
            .active
            .is_none());
        zero(&backend.watches.inner.count).await;
        backend
            .unwatch_repository(
                "main",
                UnwatchRepositoryRequest {
                    subscription_id: token,
                },
            )
            .unwrap();
        while events.try_recv().is_ok() {}
        std::fs::write(
            path.join("after-invalid-finalization"),
            b"no watcher survives",
        )
        .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(400), events.recv())
                .await
                .is_err()
        );
    }
}
#[tokio::test]
async fn startup_ack_timeout_returns_fallback_without_waiting_for_a_native_thread() {
    let watches = Watches::default();
    let scope = HistoryScope {
        repo_id: "registered-repository".into(),
        worktree_id: None,
        generation: "1".into(),
    };
    let token = super::super::sessions::token();
    let signal = Arc::new(Signal::default());
    watches.inner.state.lock().unwrap().windows.insert(
        "main".into(),
        Window {
            active: Some(Active {
                token: token.clone(),
                scope,
                sequence: 0,
                status: WatchStatus::Fallback,
                signal: signal.clone(),
            }),
            stopped: VecDeque::new(),
        },
    );
    let (_sender, receiver) = oneshot::channel();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        watches.finish("main", token, receiver),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.status, WatchStatus::Fallback);
    assert_eq!(result.sequence, "0");
    assert!(signal.late.load(Ordering::SeqCst));
}
#[tokio::test]
async fn debounce_coalesces_bounded_callback_bits_and_continuous_bursts_cannot_starve_hints() {
    let (_root, backend, scope, _) = fixture().await;
    let mut events = backend.subscribe_repository_changes();
    let watch = activate(&backend, &scope).await;
    let input = signal(&backend);
    for _ in 0..100 {
        input.mark(CHANGED);
    }
    let start = Instant::now();
    reason(
        &mut events,
        &watch.subscription_id,
        RepositoryChangeReason::Changed,
    )
    .await;
    assert!(start.elapsed() >= DEBOUNCE - TICK);
    assert!(
        tokio::time::timeout(Duration::from_millis(250), events.recv())
            .await
            .is_err()
    );
    let stop = Arc::new(AtomicBool::new(false));
    let done = stop.clone();
    let emitter = tokio::spawn(async move {
        while !done.load(Ordering::SeqCst) {
            input.mark(CHANGED);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        reason(
            &mut events,
            &watch.subscription_id,
            RepositoryChangeReason::Changed,
        ),
    )
    .await;
    stop.store(true, Ordering::SeqCst);
    emitter.await.unwrap();
    assert!(result.is_ok());
    backend.close_window("main");
    zero(&backend.watches.inner.count).await;
}
#[test]
fn native_recursive_ancestor_cover_probe() {
    for (parent_watch, child_watch) in [(true, false), (true, true), (false, true)] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("repo");
        let child = root.join(".git");
        std::fs::create_dir_all(&child).unwrap();
        let marker = Arc::new(());
        let weak = Arc::downgrade(&marker);
        let mut watcher = RecommendedWatcher::new(
            move |_: notify::Result<notify::Event>| {
                let _ = &marker;
            },
            notify::Config::default(),
        )
        .unwrap();
        if parent_watch {
            watcher.watch(&root, RecursiveMode::Recursive).unwrap();
        }
        if child_watch {
            watcher.watch(&child, RecursiveMode::Recursive).unwrap();
        }
        let result = std::fs::rename(&root, directory.path().join("moved"));
        println!(
            "native recursive watch parent={parent_watch} child={child_watch} rename_error={:?}",
            result.as_ref().err().and_then(std::io::Error::raw_os_error)
        );
        drop(watcher);
        let deadline = Instant::now() + Duration::from_secs(3);
        while weak.upgrade().is_some() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(weak.upgrade().is_none());
        if !child_watch {
            assert!(result.is_ok());
        }
    }
}
use crate::{backend::Backend, git::runner::GitRunner};

async fn fixture() -> (tempfile::TempDir, Backend, HistoryScope, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("repo");
    std::fs::create_dir(&path).unwrap();
    let git = GitRunner::discover().unwrap();
    let mut command = std::process::Command::new(git.executable());
    command.arg("-C").arg(&path).args(["init", "-b", "main"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    assert!(command.output().unwrap().status.success());
    let backend = Backend::with_git(&root.path().join("db"), git).unwrap();
    let selection = backend
        .register_repository("main", &path)
        .await
        .unwrap()
        .active_selection
        .unwrap();
    let scope = HistoryScope {
        repo_id: selection.repo_id,
        worktree_id: selection.worktree_id,
        generation: "1".into(),
    };
    (root, backend, scope, path)
}
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
async fn reason(
    events: &mut broadcast::Receiver<RoutedChange>,
    token: &str,
    reason: RepositoryChangeReason,
) -> RepositoryChangedEvent {
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            match events.recv().await {
                Ok(change)
                    if change.event.subscription_id == token && change.event.reason == reason =>
                {
                    return change.event
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => (),
                Err(error) => panic!("unexpected event channel failure: {error:?}"),
            }
        }
    })
    .await
    .unwrap()
}
fn signal(backend: &Backend) -> Arc<Signal> {
    backend.watches.inner.state.lock().unwrap().windows["main"]
        .active
        .as_ref()
        .unwrap()
        .signal
        .clone()
}
async fn zero(count: &AtomicUsize) {
    tokio::time::timeout(Duration::from_secs(6), async {
        while count.load(Ordering::SeqCst) != 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn missed_rescan_failed_emit_and_native_error_signals_recover_with_bounded_path_free_events()
{
    let (_root, backend, scope, _path) = fixture().await;
    let mut events = backend.subscribe_repository_changes();
    let watch = activate(&backend, &scope).await;
    assert_eq!(watch.status, WatchStatus::Watching);
    backend.watches.delivery_gap();
    let first = reason(
        &mut events,
        &watch.subscription_id,
        RepositoryChangeReason::Overflow,
    )
    .await;
    let route = RoutedChange {
        window: "main".into(),
        event: first.clone(),
    };
    backend.watches.forward_current(&route, || false);
    let second = reason(
        &mut events,
        &watch.subscription_id,
        RepositoryChangeReason::Overflow,
    )
    .await;
    assert!(second.sequence.parse::<u64>().unwrap() > first.sequence.parse::<u64>().unwrap());
    signal(&backend).mark(UNAVAILABLE);
    reason(
        &mut events,
        &watch.subscription_id,
        RepositoryChangeReason::Unavailable,
    )
    .await;
    backend.close_window("main");
    zero(&backend.watches.inner.count).await;
}
#[tokio::test]
async fn broadcast_lag_is_explicit_overflow_and_disposed_tokens_never_forward() {
    let (_root, backend, scope, _) = fixture().await;
    let mut events = backend.subscribe_repository_changes();
    let watch = activate(&backend, &scope).await;
    for _ in 0..EVENTS + 20 {
        assert!(current(
            &backend.watches.inner,
            "main",
            &watch.subscription_id,
            None,
            Some(RepositoryChangeReason::Changed)
        ));
    }
    assert!(matches!(
        events.recv().await,
        Err(broadcast::error::RecvError::Lagged(_))
    ));
    backend.watches.delivery_gap();
    reason(
        &mut events,
        &watch.subscription_id,
        RepositoryChangeReason::Overflow,
    )
    .await;
    let change = RoutedChange {
        window: "main".into(),
        event: RepositoryChangedEvent {
            scope,
            subscription_id: watch.subscription_id.clone(),
            sequence: "999".into(),
            reason: RepositoryChangeReason::Changed,
        },
    };
    backend
        .unwatch_repository(
            "main",
            UnwatchRepositoryRequest {
                subscription_id: watch.subscription_id,
            },
        )
        .unwrap();
    let called = AtomicBool::new(false);
    backend.watches.forward_current(&change, || {
        called.store(true, Ordering::SeqCst);
        true
    });
    assert!(!called.load(Ordering::SeqCst));
    zero(&backend.watches.inner.count).await;
}
#[tokio::test]
async fn unavailable_initial_root_uses_fallback_and_rearms_only_when_original_identity_returns() {
    let (root, backend, scope, path) = fixture().await;
    let moved = root.path().join("moved");
    std::fs::rename(&path, &moved).unwrap();
    let mut events = backend.subscribe_repository_changes();
    let watch = activate(&backend, &scope).await;
    assert_eq!(watch.status, WatchStatus::Fallback);
    reason(
        &mut events,
        &watch.subscription_id,
        RepositoryChangeReason::Unavailable,
    )
    .await;
    std::fs::create_dir(&path).unwrap();
    std::fs::create_dir(path.join(".git")).unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        backend.watches.inner.state.lock().unwrap().windows["main"]
            .active
            .as_ref()
            .unwrap()
            .status,
        WatchStatus::Fallback
    );
    std::fs::remove_dir(path.join(".git")).unwrap();
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&moved, &path).unwrap();
    reason(
        &mut events,
        &watch.subscription_id,
        RepositoryChangeReason::Changed,
    )
    .await;
    std::fs::write(path.join("rearmed"), b"event").unwrap();
    reason(
        &mut events,
        &watch.subscription_id,
        RepositoryChangeReason::Changed,
    )
    .await;
    backend.close_window("main");
    zero(&backend.watches.inner.count).await;
}
#[tokio::test]
async fn isolated_resource_slots_hold_until_last_callback_owner_and_bound_new_admission() {
    let (_root, mut backend, scope, _) = fixture().await;
    let (events, _) = broadcast::channel(EVENTS);
    let count = Arc::new(AtomicUsize::new(1));
    backend.watches = Arc::new(Watches {
        inner: Arc::new(Inner {
            state: Mutex::new(State::default()),
            events,
            count: count.clone(),
            limit: 1,
        }),
    });
    let resource = Arc::new(Resource {
        count: count.clone(),
    });
    let callback_owner = resource.clone();
    drop(resource);
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
        ErrorCode::OutputLimit
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    drop(callback_owner);
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert_eq!(
        activate(&backend, &scope).await.status,
        WatchStatus::Watching
    );
    backend.close_window("main");
    zero(&count).await;
}
#[tokio::test]
async fn stopped_token_cache_is_window_bound_capped_and_expires_and_sequence_never_wraps() {
    let (_root, backend, scope, _) = fixture().await;
    let mut events = backend.subscribe_repository_changes();
    let watch = activate(&backend, &scope).await;
    {
        let mut state = backend.watches.inner.state.lock().unwrap();
        let window = state.windows.get_mut("main").unwrap();
        for i in 0..TOMBSTONES + 1 {
            remember(window, format!("stopped-{i}"));
        }
        assert_eq!(window.stopped.len(), TOMBSTONES);
        window.stopped.push_back((
            "expired".into(),
            Instant::now() - TOKEN_TTL - Duration::from_secs(1),
        ));
        window.active.as_mut().unwrap().sequence = u64::MAX - 1;
    }
    assert_eq!(
        backend
            .watches
            .unwatch("main", "stopped-0")
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    assert_eq!(
        backend
            .watches
            .unwatch("other", "stopped-256")
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    backend.watches.unwatch("main", "stopped-256").unwrap();
    assert_eq!(
        backend.watches.unwatch("main", "expired").unwrap_err().code,
        ErrorCode::InvalidHandle
    );
    assert!(current(
        &backend.watches.inner,
        "main",
        &watch.subscription_id,
        None,
        Some(RepositoryChangeReason::Changed)
    ));
    let last = reason(
        &mut events,
        &watch.subscription_id,
        RepositoryChangeReason::Unavailable,
    )
    .await;
    assert_eq!(last.sequence, u64::MAX.to_string());
    assert!(!current(
        &backend.watches.inner,
        "main",
        &watch.subscription_id,
        None,
        Some(RepositoryChangeReason::Changed)
    ));
    backend.close_window("main");
    zero(&backend.watches.inner.count).await;
}
#[tokio::test]
async fn rapid_replacements_release_native_callback_leases_and_drop_has_no_manager_cycle() {
    let (_root, backend, scope, _) = fixture().await;
    let weak = Arc::downgrade(&backend.watches.inner);
    let count = backend.watches.inner.count.clone();
    for _ in 0..16 {
        assert_eq!(
            activate(&backend, &scope).await.status,
            WatchStatus::Watching
        );
        assert!(count.load(Ordering::SeqCst) <= WORKERS);
    }
    backend.close_window("main");
    zero(&count).await;
    drop(backend);
    assert!(weak.upgrade().is_none());
}
