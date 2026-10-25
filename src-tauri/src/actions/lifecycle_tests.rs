use super::*;
pub(crate) async fn setup() -> (tempfile::TempDir, Arc<Backend>, ActionScope) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("repository");
    let remote = directory.path().join("remote.git");
    std::fs::create_dir(&path).unwrap();
    std::fs::create_dir(&remote).unwrap();
    let runner = GitRunner::discover().unwrap();
    runner
        .run(
            Some(&path),
            &argv(&["init", "-b", "main"]),
            &Default::default(),
            Phase::Inspection,
        )
        .await
        .unwrap();
    runner
        .run(
            Some(&remote),
            &argv(&["init", "--bare"]),
            &Default::default(),
            Phase::Inspection,
        )
        .await
        .unwrap();
    let endpoint = crate::actions::path_text(&std::fs::canonicalize(remote).unwrap()).unwrap();
    runner
        .run(
            Some(&path),
            &["remote".into(), "add".into(), "origin".into(), endpoint],
            &Default::default(),
            Phase::Inspection,
        )
        .await
        .unwrap();
    let backend = Arc::new(Backend::with_git(&directory.path().join("database"), runner).unwrap());
    let selection = backend
        .register_repository("main", &path)
        .await
        .unwrap()
        .active_selection
        .unwrap();
    (
        directory,
        backend,
        ActionScope {
            repo_id: selection.repo_id,
            worktree_id: selection.worktree_id,
            generation: "9".into(),
        },
    )
}
#[tokio::test]
async fn terminal_stage_event_preserves_the_consumers_fresh_catalog_grant_and_unused_plan() {
    let (_dir, backend, scope) = setup().await;
    let location = backend
        .registry
        .location(&scope.repo_id, scope.worktree_id.as_deref())
        .unwrap();
    std::fs::write(location.root.join("selected.txt"), b"whole file\n").unwrap();
    async fn plan(backend: &Backend, scope: &ActionScope, side: ChangeSide) -> PreparedAction {
        let c = backend
            .get_repository_changes(
                "main",
                GetRepositoryChangesRequest {
                    scope: scope.clone(),
                },
            )
            .await
            .unwrap();
        let r = backend
            .resolve_change_targets(
                "main",
                ResolveChangeTargetsRequest {
                    scope: scope.clone(),
                    catalog_id: c.catalog_id,
                    targets: vec![ChangeSelection {
                        entry_id: c
                            .entries
                            .iter()
                            .find(|e| e.path_display == "selected.txt")
                            .unwrap()
                            .entry_id
                            .clone(),
                        side,
                    }],
                },
            )
            .await
            .unwrap();
        backend
            .prepare_repository_action(
                "main",
                PrepareRepositoryActionRequest {
                    scope: scope.clone(),
                    intent: if side == ChangeSide::Unstaged {
                        ActionIntent::Stage {
                            change_handles: vec![r.targets[0].change_handle.clone()],
                        }
                    } else {
                        ActionIntent::Unstage {
                            change_handles: vec![r.targets[0].change_handle.clone()],
                        }
                    },
                },
            )
            .await
            .unwrap()
    }
    let original = plan(&backend, &scope, ChangeSide::Unstaged).await;
    let pause = Arc::new(PreparePause::default());
    let done = Arc::new(tokio::sync::Notify::new());
    *backend.action_test.after_terminal_pause.lock().unwrap() = Some(pause.clone());
    *backend.action_test.completion_done.lock().unwrap() = Some(done.clone());
    let operation = backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: original.plan_id,
            },
        )
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(60), pause.entered.notified())
        .await
        .unwrap();
    assert_eq!(
        backend.scheduler.get(&operation.id).unwrap().state,
        OperationState::Succeeded
    );
    let current = ActionScope {
        generation: "10".into(),
        ..scope
    };
    let fresh = plan(&backend, &current, ChangeSide::Staged).await;
    let history = backend
        .open_history(
            "main",
            OpenHistoryRequest {
                scope: HistoryScope {
                    repo_id: current.repo_id.clone(),
                    worktree_id: current.worktree_id.clone(),
                    generation: current.generation.clone(),
                },
                filter: HistoryFilter {
                    hidden_refs: vec![],
                    solo_refs: vec![],
                },
                page_size: Some(20),
            },
        )
        .await
        .unwrap();
    let sync = backend
        .get_repository_actions(
            "main",
            GetRepositoryActionsRequest {
                scope: current.clone(),
            },
        )
        .await
        .unwrap();
    pause.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), done.notified())
        .await
        .unwrap();
    let accepted = backend
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: fresh.plan_id,
            },
        )
        .expect("a terminal event consumer's fresh plan must survive completion cleanup");
    assert!(accepted.plan.is_some());
    backend
        .history
        .sessions
        .get("main", &history.context, &history.session_id)
        .expect("the consumer's fresh history session must survive terminal cleanup")
        .valid()
        .unwrap();
    let sync_after = backend
        .get_repository_actions(
            "main",
            GetRepositoryActionsRequest {
                scope: current.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(sync_after.remotes[0].handle, sync.remotes[0].handle);
    drop(accepted);
    assert_eq!(
        backend
            .get_repository_changes("main", GetRepositoryChangesRequest { scope: current })
            .await
            .unwrap()
            .scope
            .generation,
        "10"
    );
}
fn request(scope: &ActionScope, handle: String) -> PrepareRepositoryActionRequest {
    PrepareRepositoryActionRequest {
        scope: scope.clone(),
        intent: ActionIntent::Fetch {
            remote_handles: vec![handle],
            prune: false,
        },
    }
}
#[tokio::test]
async fn not_applied_completion_keeps_existing_catalog_grants_and_unused_plans() {
    let (_dir, backend, scope) = setup().await;
    let catalog = backend
        .get_repository_actions(
            "main",
            GetRepositoryActionsRequest {
                scope: scope.clone(),
            },
        )
        .await
        .unwrap();
    let handle = catalog.remotes[0].handle.clone().unwrap();
    let first = backend
        .prepare_repository_action("main", request(&scope, handle.clone()))
        .await
        .unwrap();
    let spare = backend
        .prepare_repository_action("main", request(&scope, handle.clone()))
        .await
        .unwrap();
    let execution = backend
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: first.plan_id,
            },
        )
        .unwrap();
    let operation = backend
        .run_action("main", execution, |_| async {
            Ok(crate::actions::completed(ActionEffects::NotApplied, None))
        })
        .await
        .unwrap();
    assert_eq!(
        operation.outcome.as_ref().unwrap().effects,
        ActionEffects::NotApplied
    );
    let unused = backend
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: spare.plan_id,
            },
        )
        .expect("a verified NotApplied outcome must not revoke unused plans");
    drop(unused);
    backend
        .prepare_repository_action("main", request(&scope, handle))
        .await
        .expect("a verified NotApplied outcome must preserve the catalog grant");
}
#[tokio::test]
async fn s2_life02_prepare_keeps_original_era_through_capability_and_cannot_poison_recreated_window(
) {
    let (_directory, backend, old_scope) = setup().await;
    let old_catalog = backend
        .get_repository_actions(
            "main",
            GetRepositoryActionsRequest {
                scope: old_scope.clone(),
            },
        )
        .await
        .unwrap();
    let pause = Arc::new(PreparePause::default());
    *backend.action_test.prepare_pause.lock().unwrap() = Some(pause.clone());
    let service = backend.clone();
    let old_request = request(&old_scope, old_catalog.remotes[0].handle.clone().unwrap());
    let old =
        tokio::spawn(async move { service.prepare_repository_action("main", old_request).await });
    tokio::time::timeout(std::time::Duration::from_secs(30), pause.entered.notified())
        .await
        .unwrap();
    backend.close_window("main");
    backend.get_workspace("main").unwrap();
    let mut new_scope = old_scope.clone();
    new_scope.generation = "1".into();
    let begun = Arc::new(tokio::sync::Notify::new());
    *backend.action_test.scope_begun.lock().unwrap() = Some(begun.clone());
    let service = backend.clone();
    let scope = new_scope.clone();
    let catalog = tokio::spawn(async move {
        service
            .get_repository_actions("main", GetRepositoryActionsRequest { scope })
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), begun.notified())
        .await
        .unwrap();
    pause.release.notify_one();
    let old = old.await.unwrap();
    let catalog = catalog.await.unwrap();
    assert!(
        matches!(
            old,
            Err(AppError {
                code: ErrorCode::InvalidHandle,
                ..
            })
        ),
        "pre-close prepare must not adopt a new era"
    );
    let catalog =
        catalog.expect("the legitimate new-window generation must survive the stale prepare");
    assert!(backend
        .prepare_repository_action(
            "main",
            request(&new_scope, catalog.remotes[0].handle.clone().unwrap())
        )
        .await
        .is_ok());
    assert!(backend.scheduler.snapshot().unwrap().operations.is_empty());
}
#[tokio::test]
async fn existing_clone_recovery_blocks_new_write_admission_but_keeps_catalog_and_outcome_reads() {
    let (_directory, backend, scope) = setup().await;
    let catalog = backend
        .get_repository_actions(
            "main",
            GetRepositoryActionsRequest {
                scope: scope.clone(),
            },
        )
        .await
        .unwrap();
    let request = request(&scope, catalog.remotes[0].handle.clone().unwrap());
    let plan = backend
        .prepare_repository_action("main", request.clone())
        .await
        .unwrap();
    backend.clones.fail_closed();
    assert_eq!(
        backend
            .prepare_repository_action("main", request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Database
    );
    assert_eq!(
        backend
            .execute_repository_action(
                "main",
                ExecuteRepositoryActionRequest {
                    plan_id: plan.plan_id
                }
            )
            .unwrap_err()
            .code,
        ErrorCode::Database
    );
    assert!(backend.scheduler.snapshot().unwrap().operations.is_empty());
    assert!(backend
        .get_repository_actions("main", GetRepositoryActionsRequest { scope })
        .await
        .is_ok());
}
#[tokio::test]
async fn queued_action_rechecks_existing_clone_recovery_before_native_preflight_or_job() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (_directory, backend, scope) = setup().await;
    let catalog = backend
        .get_repository_actions(
            "main",
            GetRepositoryActionsRequest {
                scope: scope.clone(),
            },
        )
        .await
        .unwrap();
    let plan = backend
        .prepare_repository_action(
            "main",
            request(&scope, catalog.remotes[0].handle.clone().unwrap()),
        )
        .await
        .unwrap();
    let execution = backend
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    let entered = Arc::new(AtomicBool::new(false));
    let called = entered.clone();
    backend.clones.fail_closed();
    let operation = backend
        .run_action("main", execution, move |_context| async move {
            called.store(true, Ordering::SeqCst);
            Ok(crate::actions::completed(ActionEffects::Applied, None))
        })
        .await
        .unwrap();
    assert!(!entered.load(Ordering::SeqCst));
    assert_eq!(
        operation.outcome.unwrap().effects,
        ActionEffects::NotApplied
    );
    let outcome = backend
        .get_action_outcome(
            "main",
            GetActionOutcomeRequest {
                operation_id: operation.id,
            },
        )
        .unwrap();
    assert_eq!(outcome.operation.state, OperationState::Failed);
    assert!(outcome.fetch.is_none());
}

#[tokio::test]
async fn same_repository_refresh_preserves_accepted_queued_fetch_but_revokes_unused_plan() {
    let (directory, backend, old_scope) = setup().await;
    let remote = directory.path().join("remote.git");
    let runner = backend.git().unwrap();
    for args in [
        vec!["config", "user.name", "Fixture"],
        vec!["config", "user.email", "fixture@example.invalid"],
    ] {
        runner
            .run(
                Some(&remote),
                &argv(&args),
                &Default::default(),
                Phase::Inspection,
            )
            .await
            .unwrap();
    }
    let tree = String::from_utf8(
        runner
            .run(
                Some(&remote),
                &argv(&["mktree"]),
                &Default::default(),
                Phase::Inspection,
            )
            .await
            .unwrap(),
    )
    .unwrap()
    .trim()
    .to_owned();
    let oid = String::from_utf8(
        runner
            .run(
                Some(&remote),
                &argv(&["commit-tree", &tree, "-m", "fixture target"]),
                &Default::default(),
                Phase::Inspection,
            )
            .await
            .unwrap(),
    )
    .unwrap()
    .trim()
    .to_owned();
    runner
        .run(
            Some(&remote),
            &argv(&["update-ref", "refs/heads/main", &oid]),
            &Default::default(),
            Phase::Inspection,
        )
        .await
        .unwrap();
    let catalog = backend
        .get_repository_actions(
            "main",
            GetRepositoryActionsRequest {
                scope: old_scope.clone(),
            },
        )
        .await
        .unwrap();
    let handle = catalog.remotes[0].handle.clone().unwrap();
    let accepted = backend
        .prepare_repository_action("main", request(&old_scope, handle.clone()))
        .await
        .unwrap();
    let unused = backend
        .prepare_repository_action("main", request(&old_scope, handle))
        .await
        .unwrap();
    let location = backend
        .registry
        .location(&old_scope.repo_id, old_scope.worktree_id.as_deref())
        .unwrap();
    let hold = backend
        .scheduler
        .acquire_scope(
            crate::actions::lock_keys(&location),
            false,
            &Default::default(),
            Some(std::time::Duration::from_secs(30)),
        )
        .await
        .unwrap();
    let operation = backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: accepted.plan_id,
            },
        )
        .unwrap();
    let begun = Arc::new(tokio::sync::Notify::new());
    *backend.action_test.scope_begun.lock().unwrap() = Some(begun.clone());
    let mut next_scope = old_scope.clone();
    next_scope.generation = "10".into();
    let other = backend.clone();
    let refresh = tokio::spawn(async move {
        other
            .get_repository_actions("main", GetRepositoryActionsRequest { scope: next_scope })
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), begun.notified())
        .await
        .unwrap();
    assert_eq!(
        backend
            .execute_repository_action(
                "main",
                ExecuteRepositoryActionRequest {
                    plan_id: unused.plan_id
                }
            )
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    drop(hold);
    let result = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let result = backend
                .get_action_outcome(
                    "main",
                    GetActionOutcomeRequest {
                        operation_id: operation.id.clone(),
                    },
                )
                .unwrap();
            if crate::operations::state::terminal(&result.operation.state) {
                break result;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let _ = refresh.await.unwrap();
    assert_eq!(
        result.operation.state,
        OperationState::Succeeded,
        "{result:?}"
    );
    assert_eq!(
        result.operation.outcome.unwrap().effects,
        ActionEffects::Applied
    );
    assert_eq!(
        result.fetch.unwrap().remotes[0].phase,
        FetchStepPhase::Observed
    );
    assert_eq!(
        String::from_utf8(
            runner
                .run(
                    Some(&location.root),
                    &argv(&["rev-parse", "refs/remotes/origin/main"]),
                    &Default::default(),
                    Phase::Inspection
                )
                .await
                .unwrap()
        )
        .unwrap()
        .trim(),
        oid
    );
}

async fn accepted(backend: &Backend, scope: ActionScope) -> crate::actions::Execution {
    let c = backend
        .get_remote_actions(
            "main",
            GetRemoteActionsRequest {
                scope: scope.clone(),
            },
        )
        .await
        .unwrap();
    let prepared = backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope,
                intent: ActionIntent::ReadRemoteRefs {
                    remote_handle: c.remotes[0].endpoints[0].handle.clone().unwrap(),
                },
            },
        )
        .await
        .unwrap();
    backend
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: prepared.plan_id,
            },
        )
        .unwrap()
}
fn pending_snapshot(execution: &crate::actions::Execution) -> crate::actions::remote::Snapshot {
    let p = execution.plan.as_ref().unwrap();
    let now = chrono::Utc::now();
    crate::actions::remote::Snapshot {
        wire: RemoteRefSnapshot {
            captured_scope: p.scope.clone(),
            snapshot_id: crate::history::sessions::token(),
            remote_name: "origin".into(),
            role: RemoteEndpointRole::Fetch,
            observed_at: now.to_rfc3339(),
            expires_at: (now + chrono::Duration::seconds(60)).to_rfc3339(),
            object_format: None,
            refs: Vec::new(),
        },
        operation_id: execution.operation.id.clone(),
        owner: "main".into(),
        ticket: p.execution_ticket.clone().unwrap(),
        stamp: p.preflight.fingerprint.clone(),
        config_digest: "a".repeat(64),
        expires: std::time::Instant::now() + std::time::Duration::from_secs(60),
    }
}
#[tokio::test]
async fn b1_delayed_snapshot_publication_cannot_overwrite_reopened_window_result() {
    let (_dir, backend, old_scope) = setup().await;
    let old = accepted(&backend, old_scope).await;
    let old_location = old.plan.as_ref().unwrap().location.clone();
    let late = pending_snapshot(&old);
    backend.close_window("main");
    backend.get_workspace("main").unwrap();
    let mut current = late.wire.captured_scope.clone();
    current.generation = "1".into();
    let new = accepted(&backend, current.clone()).await;
    let location = new.plan.as_ref().unwrap().location.clone();
    let fresh = pending_snapshot(&new);
    let id = fresh.wire.snapshot_id.clone();
    backend.publish_remote_snapshot(&location, fresh).unwrap();
    assert_eq!(
        backend
            .publish_remote_snapshot(&old_location, late)
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    assert_eq!(
        backend
            .actions
            .remote_snapshots
            .get("main", &id)
            .unwrap()
            .operation_id,
        new.operation.id
    );
    // Ordinary view refresh preserves this accepted owner; no epoch adoption occurs.
    current.generation = "2".into();
    backend
        .get_remote_actions("main", GetRemoteActionsRequest { scope: current })
        .await
        .unwrap();
    backend
        .publish_remote_snapshot(&location, pending_snapshot(&new))
        .unwrap();
}

#[tokio::test]
async fn b1_life01_snapshot_rejection_and_close_between_lookup_and_begin_preserve_fresh_authority()
{
    for mode in ["forged", "expired", "replaced", "close"] {
        let (_dir, backend, scope) = setup().await;
        let accepted = accepted(&backend, scope.clone()).await;
        let location = accepted.plan.as_ref().unwrap().location.clone();
        let snapshot = pending_snapshot(&accepted);
        let snapshot_id = snapshot.wire.snapshot_id.clone();
        backend
            .publish_remote_snapshot(&location, snapshot.clone())
            .unwrap();
        let mut next_scope = scope.clone();
        next_scope.generation = "10".into();
        let pause = Arc::new(PreparePause::default());
        if mode != "forged" {
            *backend.action_test.remote_resolve_pause.lock().unwrap() = Some(pause.clone());
        }
        let service = backend.clone();
        let resolver = tokio::spawn(async move {
            service
                .resolve_remote_target(
                    "main",
                    ResolveRemoteTargetRequest {
                        scope: next_scope,
                        snapshot_id: if mode == "forged" {
                            "forged".into()
                        } else {
                            snapshot_id
                        },
                        source: RemoteTargetSource::New {
                            namespace: RemoteRefNamespace::Branch,
                            name: "new".into(),
                        },
                    },
                )
                .await
        });
        if mode != "forged" {
            tokio::time::timeout(std::time::Duration::from_secs(5), pause.entered.notified())
                .await
                .unwrap();
        }
        let mut current = scope;
        match mode {
            "expired" => {
                let mut expired = snapshot;
                expired.expires = std::time::Instant::now();
                backend.publish_remote_snapshot(&location, expired).unwrap();
            }
            "replaced" => {
                backend
                    .publish_remote_snapshot(&location, pending_snapshot(&accepted))
                    .unwrap();
            }
            "close" => {
                backend.close_window("main");
                backend.get_workspace("main").unwrap();
                current.generation = "1".into();
            }
            _ => {}
        }
        let catalog = backend
            .get_remote_actions(
                "main",
                GetRemoteActionsRequest {
                    scope: current.clone(),
                },
            )
            .await
            .unwrap();
        let prepare = || PrepareRepositoryActionRequest {
            scope: current.clone(),
            intent: ActionIntent::ReadRemoteRefs {
                remote_handle: catalog.remotes[0].endpoints[0].handle.clone().unwrap(),
            },
        };
        let plan = backend
            .prepare_repository_action("main", prepare())
            .await
            .unwrap();
        pause.release.notify_one();
        assert_eq!(
            resolver.await.unwrap().unwrap_err().code,
            ErrorCode::InvalidHandle,
            "{mode}"
        );
        backend
            .prepare_repository_action("main", prepare())
            .await
            .expect(mode);
        backend
            .consume_action(
                "main",
                ExecuteRepositoryActionRequest {
                    plan_id: plan.plan_id,
                },
            )
            .expect(mode);
        let after = backend
            .get_remote_actions("main", GetRemoteActionsRequest { scope: current })
            .await
            .expect(mode);
        assert_eq!(
            after.remotes[0].endpoints[0].handle, catalog.remotes[0].endpoints[0].handle,
            "{mode}"
        );
    }
}
