#[allow(dead_code)]
mod support;
use opengit_lib::{
    actions,
    backend::Backend,
    domain::contracts::*,
    git::runner::{argv, RunOptions},
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
async fn setup(
    owner: &support::OwnedFixture,
) -> (Arc<Backend>, support::repos::GitFixtures, ActionScope) {
    let fixture = support::repos::GitFixtures::create(owner).await.unwrap();
    let backend =
        Arc::new(Backend::with_git(&owner.child("data").unwrap(), fixture.runner.clone()).unwrap());
    let selection = backend
        .register_repository("main", &fixture.working)
        .await
        .unwrap()
        .active_selection
        .unwrap();
    let scope = ActionScope {
        repo_id: selection.repo_id,
        worktree_id: selection.worktree_id,
        generation: "1".into(),
    };
    (backend, fixture, scope)
}
fn intent() -> ActionIntent {
    ActionIntent::Commit {
        message: "SYNTHETIC_MESSAGE_CANARY_never_journal".into(),
        amend: false,
    }
}
async fn prepared(b: &Backend, s: &ActionScope) -> PreparedAction {
    b.prepare_action(
        "main",
        PrepareRepositoryActionRequest {
            scope: s.clone(),
            intent: intent(),
        },
    )
    .await
    .unwrap()
}
async fn actual_branch_handle(b: &Backend, scope: &ActionScope) -> String {
    let catalog = b
        .get_ref_actions(
            "main",
            GetRefActionsRequest {
                scope: scope.clone(),
            },
        )
        .await
        .unwrap();
    let entry = catalog
        .refs
        .iter()
        .find(|r| r.name == "refs/heads/main")
        .unwrap();
    b.resolve_ref_action_target(
        "main",
        ResolveRefActionTargetRequest {
            scope: scope.clone(),
            catalog_id: catalog.catalog_id,
            source: RefActionSource::Ref {
                entry_id: entry.entry_id.clone(),
            },
        },
    )
    .await
    .unwrap()
    .handle
}
#[tokio::test]
async fn scoped_forgery_and_consumption_replay_cannot_create_another_execution() {
    let owner = support::OwnedFixture::new().unwrap();
    let (b, f, s) = setup(&owner).await;
    b.get_workspace("other").unwrap();
    let bad = b
        .prepare_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: s.clone(),
                intent: ActionIntent::BranchSwitch {
                    branch_handle: "a".repeat(64),
                },
            },
        )
        .await
        .unwrap_err();
    assert_eq!(bad.code, ErrorCode::InvalidHandle);
    let grant = actual_branch_handle(&b, &s).await;
    let plan = b
        .prepare_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: s.clone(),
                intent: ActionIntent::BranchSwitch {
                    branch_handle: grant,
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(
        b.consume_action(
            "other",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id.clone()
            }
        )
        .err()
        .unwrap()
        .code,
        ErrorCode::InvalidHandle
    );
    let first = b
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id.clone(),
            },
        )
        .unwrap();
    let id = first.operation.id.clone();
    let replay = b
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(replay.operation.id, id);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let done = b
        .run_action("main", first, move |_ctx| async move {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(actions::completed(ActionEffects::NotApplied, None))
        })
        .await
        .unwrap();
    assert_eq!(done.outcome.unwrap().effects, ActionEffects::NotApplied);
    b.run_action("main", replay, |_| async {
        panic!("idempotent replay executed")
    })
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        b.cancel_operation(
            "other",
            CancelOperationRequest {
                operation_id: id.clone()
            }
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidHandle
    );
    assert_eq!(
        b.respond_auth_prompt(
            "other",
            RespondAuthPromptRequest {
                operation_id: id,
                prompt_handle: "a".repeat(64),
                value: "SECRET_CANARY".into()
            }
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidHandle
    );
    let c = b.registry.database.lock().unwrap();
    let rows: String = c
        .query_row(
            "SELECT group_concat(record) FROM repository_actions",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!rows.contains("CANARY"));
    assert!(!rows.contains(&plan.plan_id));
    drop(c);
    let _ = f;
    drop(b);
}
#[tokio::test]
async fn cancellation_after_a_verified_git_effect_stays_applied_and_queued_cancel_has_none() {
    let owner = support::OwnedFixture::new().unwrap();
    let (b, f, s) = setup(&owner).await;
    let plan = prepared(&b, &s).await;
    let execution = b
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    let id = execution.operation.id.clone();
    let cancel = b.clone();
    let result = b
        .run_action("main", execution, move |ctx| async move {
            assert!(ctx.before_write().await?);
            ctx.run(
                &argv(&["update-ref", "refs/heads/s1-authority-proof", "HEAD"]),
                Phase::Publish,
                RunOptions::default(),
            )
            .await?;
            cancel.cancel_operation("main", CancelOperationRequest { operation_id: id })?;
            Ok(actions::completed(
                ActionEffects::Applied,
                Some("a".repeat(64)),
            ))
        })
        .await
        .unwrap();
    assert_eq!(result.state, OperationState::Succeeded);
    let outcome = result.outcome.unwrap();
    assert_eq!(outcome.effects, ActionEffects::Applied);
    assert!(outcome.cancellation_requested);
    f.runner
        .run(
            Some(&f.working),
            &argv(&["show-ref", "--verify", "refs/heads/s1-authority-proof"]),
            &Default::default(),
            Phase::Verification,
        )
        .await
        .unwrap();
    let mut fresh = s.clone();
    fresh.generation = "2".into();
    let plan = prepared(&b, &fresh).await;
    let execution = b
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    b.cancel_operation(
        "main",
        CancelOperationRequest {
            operation_id: execution.operation.id.clone(),
        },
    )
    .unwrap();
    let result = b
        .run_action("main", execution, |_| async { panic!("queued cancel ran") })
        .await
        .unwrap();
    assert_eq!(result.state, OperationState::Cancelled);
    assert_eq!(result.outcome.unwrap().effects, ActionEffects::NotApplied);
}
#[tokio::test]
async fn external_index_or_ref_change_after_prepare_rejects_the_entire_write() {
    let owner = support::OwnedFixture::new().unwrap();
    let (b, f, s) = setup(&owner).await;
    let plan = prepared(&b, &s).await;
    support::repos::run(
        &f.runner,
        &f.working,
        &["update-ref", "refs/heads/external", "HEAD"],
    )
    .await
    .unwrap();
    let execution = b
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    let result = b
        .run_action("main", execution, |_| async {
            panic!("stale preflight ran")
        })
        .await
        .unwrap();
    assert_eq!(result.state, OperationState::Failed);
    assert_eq!(result.error.unwrap().code, ErrorCode::ExternalChange);
    assert_eq!(result.outcome.unwrap().effects, ActionEffects::NotApplied);
}
#[tokio::test]
async fn switched_or_closed_windows_cannot_cross_the_before_write_gate() {
    let owner = support::OwnedFixture::new().unwrap();
    let (b, f, s) = setup(&owner).await;
    let plan = prepared(&b, &s).await;
    let execution = b
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    let close = b.clone();
    let result = b
        .run_action("main", execution, move |ctx| async move {
            close.close_window("main");
            assert!(!ctx.before_write().await.unwrap_or(false));
            Err(opengit_lib::git::error::error(
                ErrorCode::Interrupted,
                Phase::Validation,
            ))
        })
        .await
        .unwrap();
    assert_eq!(result.outcome.unwrap().effects, ActionEffects::NotApplied);
    assert!(matches!(
        result.state,
        OperationState::Cancelled | OperationState::Failed
    ));
    let _ = f;
}

#[tokio::test]
async fn real_linked_worktrees_share_one_common_write_lease_including_an_initially_unregistered_worktree(
) {
    let owner = support::OwnedFixture::new().unwrap();
    let (b, f, s) = setup(&owner).await;
    let linked =
        std::path::PathBuf::from(opengit_lib::clone::path::git_path(owner.root()).unwrap())
            .join("linked");
    support::repos::run(
        &f.runner,
        &f.working,
        &[
            "worktree",
            "add",
            "-b",
            "linked",
            linked.to_str().unwrap(),
            "HEAD",
        ],
    )
    .await
    .unwrap();
    let plan = prepared(&b, &s).await;
    let first = b
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = tokio::sync::oneshot::channel();
    let (release, wait) = tokio::sync::oneshot::channel();
    let running = b.clone();
    let count = active.clone();
    let max = maximum.clone();
    let first = tokio::spawn(async move {
        running
            .run_action("main", first, move |ctx| async move {
                assert!(ctx.before_write().await?);
                let n = count.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(n, Ordering::SeqCst);
                tx.send(()).unwrap();
                wait.await.unwrap();
                count.fetch_sub(1, Ordering::SeqCst);
                Ok(actions::completed(ActionEffects::NotApplied, None))
            })
            .await
            .unwrap()
    });
    rx.await.unwrap();
    let selection = b
        .register_repository("other", &linked)
        .await
        .unwrap()
        .active_selection
        .unwrap();
    let linked_scope = ActionScope {
        repo_id: selection.repo_id,
        worktree_id: selection.worktree_id,
        generation: "1".into(),
    };
    let preparing_backend = b.clone();
    let mut preparing = tokio::spawn(async move {
        preparing_backend
            .prepare_action(
                "other",
                PrepareRepositoryActionRequest {
                    scope: linked_scope,
                    intent: intent(),
                },
            )
            .await
    });
    // Prepare now owns a read scope too: it must wait behind the active writer.
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut preparing)
            .await
            .is_err()
    );
    assert_eq!(active.load(Ordering::SeqCst), 1);
    release.send(()).unwrap();
    assert_eq!(first.await.unwrap().state, OperationState::Succeeded);
    let plan = preparing.await.unwrap().unwrap();
    let second = b
        .consume_action(
            "other",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    assert_eq!(
        b.scheduler.get(&second.operation.id).unwrap().state,
        OperationState::Queued
    );
    let count = active.clone();
    let max = maximum.clone();
    let result = b
        .run_action("other", second, move |_ctx| async move {
            let n = count.fetch_add(1, Ordering::SeqCst) + 1;
            max.fetch_max(n, Ordering::SeqCst);
            count.fetch_sub(1, Ordering::SeqCst);
            Ok(actions::completed(ActionEffects::NotApplied, None))
        })
        .await
        .unwrap();
    assert_eq!(result.state, OperationState::Succeeded);
    assert_eq!(maximum.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn read_capacity_busy_unmerged_dirty_and_bare_preflight_refuse_without_any_write() {
    let owner = support::OwnedFixture::new().unwrap();
    let (b, f, s) = setup(&owner).await;
    let permits = (0..4)
        .map(|_| b.actions.reads.try_acquire().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        b.prepare_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: s.clone(),
                intent: intent()
            }
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::OutputLimit
    );
    drop(permits);
    std::fs::write(f.working.join(".git/MERGE_HEAD"), b"a").unwrap();
    assert_eq!(
        b.prepare_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: s.clone(),
                intent: intent()
            }
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::ExternalChange
    );
    std::fs::remove_file(f.working.join(".git/MERGE_HEAD")).unwrap();
    std::fs::write(f.working.join("dirty.txt"), b"user edit").unwrap();
    let branch = actual_branch_handle(&b, &s).await;
    assert_eq!(
        b.prepare_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: s.clone(),
                intent: ActionIntent::BranchSwitch {
                    branch_handle: branch
                }
            }
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::ExternalChange
    );
    std::fs::remove_file(f.working.join("dirty.txt")).unwrap();
    for args in [
        vec!["config", "user.name", "Fixture"],
        vec!["config", "user.email", "fixture@example.invalid"],
    ] {
        support::repos::run(&f.runner, &f.working, &args)
            .await
            .unwrap();
    }
    std::fs::write(f.working.join("conflict.txt"), b"base\n").unwrap();
    support::repos::run(&f.runner, &f.working, &["add", "--", "conflict.txt"])
        .await
        .unwrap();
    support::repos::run(&f.runner, &f.working, &["commit", "-m", "base"])
        .await
        .unwrap();
    support::repos::run(&f.runner, &f.working, &["checkout", "-b", "side"])
        .await
        .unwrap();
    std::fs::write(f.working.join("conflict.txt"), b"side\n").unwrap();
    support::repos::run(&f.runner, &f.working, &["commit", "-am", "side"])
        .await
        .unwrap();
    support::repos::run(&f.runner, &f.working, &["checkout", "main"])
        .await
        .unwrap();
    std::fs::write(f.working.join("conflict.txt"), b"main\n").unwrap();
    support::repos::run(&f.runner, &f.working, &["commit", "-am", "main"])
        .await
        .unwrap();
    assert!(
        support::repos::run(&f.runner, &f.working, &["merge", "side"])
            .await
            .is_err()
    );
    // A stash-like unresolved index must still block commit without MERGE_HEAD.
    std::fs::remove_file(f.working.join(".git/MERGE_HEAD")).unwrap();
    let before_index = std::fs::read(f.working.join(".git/index")).unwrap();
    let before_worktree = std::fs::read(f.working.join("conflict.txt")).unwrap();
    assert_eq!(
        b.prepare_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: s.clone(),
                intent: intent()
            }
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::ExternalChange
    );
    assert_eq!(
        std::fs::read(f.working.join(".git/index")).unwrap(),
        before_index
    );
    assert_eq!(
        std::fs::read(f.working.join("conflict.txt")).unwrap(),
        before_worktree
    );
    let selection = b
        .register_repository("main", &f.bare)
        .await
        .unwrap()
        .active_selection
        .unwrap();
    let bare = ActionScope {
        repo_id: selection.repo_id,
        worktree_id: None,
        generation: "2".into(),
    };
    assert_eq!(
        b.prepare_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: bare,
                intent: intent()
            }
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::InvalidRequest
    );
    assert!(b.get_workspace("main").is_ok());
    assert!(b.get_operations("main").unwrap().operations.is_empty());
}

#[tokio::test]
async fn failed_atomic_consume_keeps_the_plan_retryable_without_operation_or_execution() {
    let owner = support::OwnedFixture::new().unwrap();
    let (b, _f, s) = setup(&owner).await;
    let p = prepared(&b, &s).await;
    b.registry.database.lock().unwrap().execute_batch("CREATE TRIGGER refuse_insert BEFORE INSERT ON repository_actions BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert_eq!(
        b.consume_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: p.plan_id.clone()
            }
        )
        .err()
        .unwrap()
        .code,
        ErrorCode::Database
    );
    assert!(b.scheduler.snapshot().unwrap().operations.is_empty());
    assert_eq!(
        b.registry
            .database
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM operations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    b.registry
        .database
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_insert")
        .unwrap();
    let e = b
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: p.plan_id.clone(),
            },
        )
        .unwrap();
    let id = e.operation.id.clone();
    let replay = b
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest { plan_id: p.plan_id },
        )
        .unwrap();
    assert_eq!(replay.operation.id, id);
    let result = b
        .run_action("main", e, |_| async {
            Ok(actions::completed(ActionEffects::NotApplied, None))
        })
        .await
        .unwrap();
    assert_eq!(result.state, OperationState::Succeeded);
    b.run_action("main", replay, |_| async {
        panic!("replayed operation executed")
    })
    .await
    .unwrap();
    assert_eq!(b.scheduler.snapshot().unwrap().operations.len(), 1);
}

#[tokio::test]
async fn forgetting_a_repository_cancels_queued_actions_in_every_owner_window() {
    let owner = support::OwnedFixture::new().unwrap();
    let (b, _f, s) = setup(&owner).await;
    b.get_workspace("other").unwrap();
    let unused = prepared(&b, &s).await;
    let other = b
        .prepare_action(
            "other",
            PrepareRepositoryActionRequest {
                scope: s.clone(),
                intent: intent(),
            },
        )
        .await
        .unwrap();
    let queued = b
        .consume_action(
            "other",
            ExecuteRepositoryActionRequest {
                plan_id: other.plan_id,
            },
        )
        .unwrap();
    let id = queued.operation.id.clone();
    b.forget_repository("main", ForgetRepositoryRequest { repo_id: s.repo_id })
        .unwrap();
    assert_eq!(
        b.scheduler.get(&id).unwrap().state,
        OperationState::Cancelled
    );
    assert_eq!(
        b.consume_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: unused.plan_id
            }
        )
        .err()
        .unwrap()
        .code,
        ErrorCode::InvalidHandle
    );
    let result = b
        .run_action("other", queued, |_| async {
            panic!("forgotten queued action ran")
        })
        .await
        .unwrap();
    assert_eq!(result.outcome.unwrap().effects, ActionEffects::NotApplied);
}

#[tokio::test]
async fn the_same_four_read_cap_applies_to_queued_execution_and_before_write_revalidation() {
    let owner = support::OwnedFixture::new().unwrap();
    let (b, _f, s) = setup(&owner).await;
    let p = prepared(&b, &s).await;
    let e = b
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest { plan_id: p.plan_id },
        )
        .unwrap();
    let permits = (0..4)
        .map(|_| b.actions.reads.clone().try_acquire_owned().unwrap())
        .collect::<Vec<_>>();
    let failed = b
        .run_action("main", e, |_| async {
            panic!("full read capacity entered executor")
        })
        .await
        .unwrap();
    assert_eq!(failed.error.unwrap().code, ErrorCode::OutputLimit);
    assert_eq!(failed.outcome.unwrap().effects, ActionEffects::NotApplied);
    drop(permits);
    let p = prepared(&b, &s).await;
    let e = b
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest { plan_id: p.plan_id },
        )
        .unwrap();
    let reads = b.actions.reads.clone();
    let failed = b
        .run_action("main", e, move |ctx| async move {
            let permits = (0..4)
                .map(|_| reads.clone().try_acquire_owned().unwrap())
                .collect::<Vec<_>>();
            let error = ctx.before_write().await.unwrap_err();
            drop(permits);
            Err(error)
        })
        .await
        .unwrap();
    assert_eq!(failed.error.unwrap().code, ErrorCode::OutputLimit);
    assert_eq!(failed.outcome.unwrap().effects, ActionEffects::NotApplied);
    assert_eq!(b.actions.reads.available_permits(), 4);
}

#[tokio::test]
async fn actual_guarded_worktree_capability_rejects_old_flags_malformed_and_real_failures_without_affecting_foundation(
) {
    let owner = support::OwnedFixture::new().unwrap();
    let f = support::repos::GitFixtures::create(&owner).await.unwrap();
    let marker = owner.child("capability-mode").unwrap();
    let source = owner.child("capability-proxy.rs").unwrap();
    let binary = owner.child("capability proxy Ω.exe").unwrap();
    let program = format!(
        r#"
use std::io::Write;
fn main(){{let args=std::env::args_os().skip(1).collect::<Vec<_>>();
 let mode=std::fs::read_to_string({marker:?}).unwrap();
 if mode=="old-global" && args.iter().any(|a|a=="--no-lazy-fetch"){{eprintln!("unknown global option CANARY");std::process::exit(129)}}
 if args.iter().any(|a|a=="worktree")&&args.iter().any(|a|a=="list")&&args.iter().any(|a|a=="-z"){{
 let mode=std::fs::read_to_string({marker:?}).unwrap();match mode.as_str(){{
 "old"=>{{eprintln!("error: unknown switch z CANARY");std::process::exit(129)}},
 "malformed"=>{{print!("worktree incomplete\n");return}},
 "failure"=>{{eprintln!("fatal: permission denied CANARY");std::process::exit(128)}},
 "partial129"=>{{print!("partial");eprintln!("usage CANARY");std::process::exit(129)}},
 "oversize"=>{{let mut out=std::io::stdout().lock();let block=[b'x';65536];for _ in 0..129{{let _=out.write_all(&block);}}return}},_=>()}}
 }}
 let status=std::process::Command::new({git:?}).args(args).status().unwrap();std::process::exit(status.code().unwrap_or(1));}}
"#,
        marker = marker.to_str().unwrap(),
        git = f.runner.executable().to_str().unwrap()
    );
    std::fs::write(&source, program).unwrap();
    let rustc = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .filter(|p| p.is_absolute())
        .map(|p| p.join("rustc.exe"))
        .find(|p| p.is_file())
        .unwrap();
    assert!(std::process::Command::new(rustc)
        .arg(&source)
        .args(["--edition=2021", "-o"])
        .arg(&binary)
        .status()
        .unwrap()
        .success());
    let before = f
        .runner
        .run(
            Some(&f.working),
            &argv(&["show-ref"]),
            &Default::default(),
            Phase::Inspection,
        )
        .await
        .unwrap();
    let index = std::fs::read(f.working.join(".git/index")).ok();
    for (mode, code) in [
        ("old", ErrorCode::UnsupportedGit),
        ("old-global", ErrorCode::UnsupportedGit),
        ("malformed", ErrorCode::MalformedOutput),
        ("failure", ErrorCode::PermissionDenied),
        ("partial129", ErrorCode::Internal),
        ("oversize", ErrorCode::OutputLimit),
    ] {
        std::fs::write(&marker, mode).unwrap();
        let runner = opengit_lib::git::runner::GitRunner::trusted(binary.clone()).unwrap();
        let b = Backend::with_git(&owner.child(&format!("db-{mode}")).unwrap(), runner).unwrap();
        let selection = b
            .register_repository("main", &f.working)
            .await
            .unwrap()
            .active_selection
            .unwrap();
        let inspection = b
            .inspect_repository(
                "main",
                InspectRepositoryRequest {
                    repo_id: selection.repo_id.clone(),
                    worktree_id: selection.worktree_id.clone(),
                    generation: "1".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(inspection.head.kind, HeadKind::Branch);
        let scope = ActionScope {
            repo_id: selection.repo_id,
            worktree_id: selection.worktree_id,
            generation: "1".into(),
        };
        let error = b
            .prepare_action(
                "main",
                PrepareRepositoryActionRequest {
                    scope,
                    intent: intent(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, code, "{mode}");
        assert!(!error.message.contains("CANARY"));
        if mode == "old" || mode == "old-global" {
            assert!(error.message.contains("Update Git"));
            assert!(!error.message.contains("2.31"));
        }
        assert!(b.get_operations("main").unwrap().operations.is_empty());
    }
    assert_eq!(
        f.runner
            .run(
                Some(&f.working),
                &argv(&["show-ref"]),
                &Default::default(),
                Phase::Inspection
            )
            .await
            .unwrap(),
        before
    );
    assert_eq!(std::fs::read(f.working.join(".git/index")).ok(), index);
}

#[tokio::test]
async fn s1_bq01_unproven_stop_retains_actual_process_options_scope_lease_until_verified_disposal()
{
    let owner = support::OwnedFixture::new().unwrap();
    let (b, f, s) = setup(&owner).await;
    let location = b
        .registry
        .location(&s.repo_id, s.worktree_id.as_deref())
        .unwrap();
    let keys = actions::lock_keys(&location);
    let p = prepared(&b, &s).await;
    let e = b
        .consume_action(
            "main",
            ExecuteRepositoryActionRequest { plan_id: p.plan_id },
        )
        .unwrap();
    let retained = Arc::new(std::sync::Mutex::new(None::<Arc<dyn Send + Sync>>));
    let held = retained.clone();
    let result = b
        .run_action("main", e, move |ctx| async move {
            assert!(ctx.before_write().await?);
            // Deterministic injection at the existing runner contract: an unproven stop
            // retains exactly this pure RunOptions guard after the action future returns.
            *held.lock().unwrap() = ctx.run_options(RunOptions::default())?.lifetime_guard;
            Err(opengit_lib::git::error::error(
                ErrorCode::Interrupted,
                Phase::Recovery,
            ))
        })
        .await
        .unwrap();
    assert_eq!(result.state, OperationState::Interrupted);
    assert_eq!(result.outcome.unwrap().effects, ActionEffects::Unknown);
    let cancel = Default::default();
    assert_eq!(
        b.scheduler
            .acquire_scope(
                keys.clone(),
                false,
                &cancel,
                Some(std::time::Duration::from_millis(30))
            )
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::Timeout
    );
    let unrelated = b
        .scheduler
        .acquire_scope(
            vec![opengit_lib::operations::scheduler::LockKey {
                common: opengit_lib::repositories::discovery::identity(&f.bare).unwrap(),
                worktree: None,
            }],
            false,
            &cancel,
            Some(std::time::Duration::from_millis(100)),
        )
        .await
        .unwrap();
    drop(unrelated);
    // Verified runner disposal releases the opaque guard; then same-common work
    // can be admitted. No operation state/reset or global quarantine is changed.
    retained.lock().unwrap().take();
    let released = b
        .scheduler
        .acquire_scope(
            keys,
            false,
            &cancel,
            Some(std::time::Duration::from_millis(100)),
        )
        .await
        .unwrap();
    drop(released);
    let _ = f;
}
