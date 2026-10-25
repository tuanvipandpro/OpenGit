#[allow(dead_code)]
mod support;
use opengit_lib::domain::contracts::*;
async fn catalog(f: &support::daily::Fixture) -> RefActionCatalog {
    let future = f.backend.get_ref_actions(
        "main",
        GetRefActionsRequest {
            scope: f.scope.clone(),
        },
    );
    eprintln!(
        "ref_catalog_future_bytes={}",
        std::mem::size_of_val(&future)
    );
    future.await.unwrap()
}
async fn handle(f: &support::daily::Fixture, c: &RefActionCatalog, name: &str) -> String {
    let source = if name == "HEAD" {
        RefActionSource::Head
    } else {
        RefActionSource::Ref {
            entry_id: c
                .refs
                .iter()
                .find(|r| r.name == name)
                .unwrap()
                .entry_id
                .clone(),
        }
    };
    f.backend
        .resolve_ref_action_target(
            "main",
            ResolveRefActionTargetRequest {
                scope: f.scope.clone(),
                catalog_id: c.catalog_id.clone(),
                source,
            },
        )
        .await
        .unwrap()
        .handle
}
async fn execute(f: &support::daily::Fixture, intent: ActionIntent) -> RefActionOperationResult {
    let plan = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent,
            },
        )
        .await
        .unwrap();
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id.clone(),
            },
        )
        .unwrap();
    let result = f.wait(&op.id).await;
    let replay = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    assert_eq!(replay.id, op.id);
    f.backend
        .get_ref_action_outcome(
            "main",
            GetActionOutcomeRequest {
                operation_id: result.operation.id,
            },
        )
        .unwrap()
}
fn applied(r: &RefActionOperationResult) {
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::Applied,
        "{r:?}"
    );
    assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
}
#[tokio::test]
async fn actual_branch_create_rename_switch_tracking_and_safe_delete() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    let c = catalog(&f).await;
    assert!(
        c.refs
            .iter()
            .find(|r| r.name == "refs/heads/main")
            .unwrap()
            .checked_out_here
    );
    let start = handle(&f, &c, "HEAD").await;
    applied(
        &execute(
            &f,
            ActionIntent::BranchCreate {
                name: "daily-Ω".into(),
                start_handle: start,
                checkout: true,
            },
        )
        .await,
    );
    assert_eq!(f.git.oid("HEAD"), f.git.oid("refs/heads/main"));
    assert_eq!(
        String::from_utf8(f.git.run(&["symbolic-ref", "HEAD"], None))
            .unwrap()
            .trim(),
        "refs/heads/daily-Ω"
    );
    let c = catalog(&f).await;
    let branch = handle(&f, &c, "refs/heads/daily-Ω").await;
    applied(
        &execute(
            &f,
            ActionIntent::BranchRename {
                branch_handle: branch,
                new_name: "renamed-Ω".into(),
            },
        )
        .await,
    );
    let c = catalog(&f).await;
    let branch = handle(&f, &c, "refs/heads/renamed-Ω").await;
    let upstream = handle(&f, &c, "refs/remotes/origin/main").await;
    applied(
        &execute(
            &f,
            ActionIntent::BranchTracking {
                branch_handle: branch,
                upstream_handle: Some(upstream),
            },
        )
        .await,
    );
    assert_eq!(
        String::from_utf8(f.git.run(&["config", "branch.renamed-Ω.remote"], None))
            .unwrap()
            .trim(),
        "origin"
    );
    let c = catalog(&f).await;
    let branch = handle(&f, &c, "refs/heads/renamed-Ω").await;
    applied(
        &execute(
            &f,
            ActionIntent::BranchTracking {
                branch_handle: branch,
                upstream_handle: None,
            },
        )
        .await,
    );
    let c = catalog(&f).await;
    let main = handle(&f, &c, "refs/heads/main").await;
    applied(
        &execute(
            &f,
            ActionIntent::BranchSwitch {
                branch_handle: main,
            },
        )
        .await,
    );
    let c = catalog(&f).await;
    let branch = handle(&f, &c, "refs/heads/renamed-Ω").await;
    applied(
        &execute(
            &f,
            ActionIntent::BranchDelete {
                branch_handle: branch,
            },
        )
        .await,
    );
    assert!(
        !String::from_utf8(f.git.run(&["for-each-ref", "--format=%(refname)"], None))
            .unwrap()
            .contains("renamed-Ω")
    );
}
#[tokio::test]
async fn actual_tags_sha256_reftable_and_annotation_never_journaled() {
    let f = support::daily::Fixture::new("sha256", 8, "reftable").await;
    f.git.run(&["config", "user.name", "Fixture"], None);
    f.git
        .run(&["config", "user.email", "fixture@example.invalid"], None);
    for (name, annotation) in [
        ("light", TagAnnotation::Lightweight),
        (
            "annotated",
            TagAnnotation::Annotated {
                message: "private annotation Ω\nsecond line".into(),
                sign: false,
            },
        ),
    ] {
        let c = catalog(&f).await;
        let start = handle(&f, &c, "HEAD").await;
        let r = execute(
            &f,
            ActionIntent::TagCreate {
                name: name.into(),
                target_handle: start,
                annotation,
            },
        )
        .await;
        applied(&r);
        assert_eq!(
            f.git.oid(&format!("refs/tags/{name}^{{commit}}")),
            f.git.oid("HEAD")
        );
        let record =
            opengit_lib::actions::journal::read(&f.backend.registry.database, &r.operation.id)
                .unwrap();
        let text = serde_json::to_string(&record).unwrap();
        assert!(!text.contains("private annotation"));
        let c = catalog(&f).await;
        let tag = handle(&f, &c, &format!("refs/tags/{name}")).await;
        applied(&execute(&f, ActionIntent::TagDelete { tag_handle: tag }).await);
    }
}
#[tokio::test]
async fn stale_catalog_forgery_dirty_unmerged_and_other_worktree_refuse_without_write() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    let c = catalog(&f).await;
    let mut request = ResolveRefActionTargetRequest {
        scope: f.scope.clone(),
        catalog_id: c.catalog_id.clone(),
        source: RefActionSource::Head,
    };
    request.catalog_id = "a".repeat(64);
    assert_eq!(
        f.backend
            .resolve_ref_action_target("main", request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    let unmerged = handle(&f, &c, "refs/heads/orphan").await;
    assert!(f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::BranchDelete {
                    branch_handle: unmerged
                }
            }
        )
        .await
        .is_err());
    std::fs::write(f.git.path.join("dirty"), b"preserve").unwrap();
    let c = catalog(&f).await;
    let alpha = handle(&f, &c, "refs/heads/alpha").await;
    assert!(f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::BranchSwitch {
                    branch_handle: alpha
                }
            }
        )
        .await
        .is_err());
    std::fs::remove_file(f.git.path.join("dirty")).unwrap();
    let linked = f.git.owner.child("linked Ω").unwrap();
    f.git.run(
        &["worktree", "add", "--", &f.git.git_path(&linked), "alpha"],
        None,
    );
    let c = catalog(&f).await;
    assert!(
        c.refs
            .iter()
            .find(|r| r.name == "refs/heads/alpha")
            .unwrap()
            .checked_out_elsewhere
    );
    let alpha = handle(&f, &c, "refs/heads/alpha").await;
    assert!(f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::BranchSwitch {
                    branch_handle: alpha
                }
            }
        )
        .await
        .is_err());
    let c = catalog(&f).await;
    f.git.run(&["branch", "external", "HEAD"], None);
    assert_eq!(
        f.backend
            .resolve_ref_action_target(
                "main",
                ResolveRefActionTargetRequest {
                    scope: f.scope.clone(),
                    catalog_id: c.catalog_id,
                    source: RefActionSource::Head
                }
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::ExternalChange
    );
    assert_eq!(
        std::fs::read(f.git.path.join(".git/HEAD")).unwrap(),
        b"ref: refs/heads/main\n"
    );
}
#[tokio::test]
async fn stale_era_catalog_cannot_adopt_reopened_window_or_mutate_scope() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    let c = catalog(&f).await;
    f.backend.close_window("main");
    f.backend.get_workspace("main").unwrap();
    let err = f
        .backend
        .resolve_ref_action_target(
            "main",
            ResolveRefActionTargetRequest {
                scope: f.scope.clone(),
                catalog_id: c.catalog_id,
                source: RefActionSource::Head,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidHandle);
    let c = catalog(&f).await;
    assert_eq!(handle(&f, &c, "HEAD").await.len(), 64);
}

#[tokio::test]
async fn deletion_config_failure_retains_partial_evidence_and_config_without_retry() {
    let mut f = support::daily::Fixture::new("sha1", 8, "files").await;
    f.git.run(&["branch", "retire", "HEAD"], None);
    f.git
        .run(&["config", "branch.retire.remote", "origin"], None);
    f.git
        .run(&["config", "branch.retire.merge", "refs/heads/main"], None);
    let proxy = support::daily::Proxy::new(&f.git);
    f.rebind(proxy.runner.clone(), None).await;
    let c = catalog(&f).await;
    let branch = handle(&f, &c, "refs/heads/retire").await;
    proxy.set("cleanupfail");
    let r = execute(
        &f,
        ActionIntent::BranchDelete {
            branch_handle: branch,
        },
    )
    .await;
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::Partial,
        "{r:?}"
    );
    let steps = r.result.unwrap().steps;
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].effects, ActionEffects::Applied);
    assert_eq!(steps[1].effects, ActionEffects::NotApplied);
    assert_eq!(
        steps[1].failure_reason,
        Some(RefActionFailureReason::Native)
    );
    assert!(
        !String::from_utf8(f.git.run(&["for-each-ref", "--format=%(refname)"], None))
            .unwrap()
            .contains("refs/heads/retire")
    );
    assert_eq!(
        String::from_utf8(f.git.run(&["config", "branch.retire.remote"], None))
            .unwrap()
            .trim(),
        "origin"
    );
    let record =
        opengit_lib::actions::journal::read(&f.backend.registry.database, &r.operation.id).unwrap();
    assert!(!serde_json::to_string(&record).unwrap().contains("CANARY"));
}
#[tokio::test]
async fn ignored_collision_keeps_bytes_and_reports_create_checkout_partial() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    let blob = String::from_utf8(
        f.git
            .run(&["hash-object", "-w", "--stdin"], Some(b"tracked target")),
    )
    .unwrap()
    .trim()
    .to_owned();
    let tree = String::from_utf8(f.git.run(
        &["mktree"],
        Some(format!("100644 blob {blob}\tcollision\n").as_bytes()),
    ))
    .unwrap()
    .trim()
    .to_owned();
    let oid = String::from_utf8(f.git.run(
        &["commit-tree", &tree, "-p", "HEAD"],
        Some(b"target collision\n"),
    ))
    .unwrap()
    .trim()
    .to_owned();
    f.git
        .run(&["update-ref", "refs/heads/target-collision", &oid], None);
    std::fs::write(f.git.path.join(".git/info/exclude"), b"collision\n").unwrap();
    std::fs::write(f.git.path.join("collision"), b"ignored preserve").unwrap();
    let c = catalog(&f).await;
    assert!(!c.dirty);
    let start = handle(&f, &c, "refs/heads/target-collision").await;
    let r = execute(
        &f,
        ActionIntent::BranchCreate {
            name: "collision-checkout".into(),
            start_handle: start,
            checkout: true,
        },
    )
    .await;
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::Partial,
        "{r:?}"
    );
    assert_eq!(
        std::fs::read(f.git.path.join("collision")).unwrap(),
        b"ignored preserve"
    );
    assert_eq!(f.git.oid("refs/heads/collision-checkout"), oid);
    assert_eq!(
        std::fs::read(f.git.path.join(".git/HEAD")).unwrap(),
        b"ref: refs/heads/main\n"
    );
}
#[tokio::test]
async fn identity_and_signer_failures_are_safe_hints_never_noeffects_proof() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    f.git.run(&["config", "user.name", ""], None);
    f.git.run(&["config", "user.email", ""], None);
    let c = catalog(&f).await;
    let target = handle(&f, &c, "HEAD").await;
    let r = execute(
        &f,
        ActionIntent::TagCreate {
            name: "no-identity".into(),
            target_handle: target,
            annotation: TagAnnotation::Annotated {
                message: "MESSAGE_CANARY".into(),
                sign: false,
            },
        },
    )
    .await;
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::NotApplied,
        "{r:?}"
    );
    assert_eq!(
        r.result.unwrap().steps[0].failure_reason,
        Some(RefActionFailureReason::Identity)
    );
    f.git.run(&["config", "user.name", "Fixture"], None);
    f.git
        .run(&["config", "user.email", "fixture@example.invalid"], None);
    let signer = f.git.owner.child("signer-fails.cmd").unwrap();
    std::fs::write(
        &signer,
        b"@echo off\r\necho signing CANARY 1>&2\r\nexit /b 1\r\n",
    )
    .unwrap();
    f.git
        .run(&["config", "gpg.program", &f.git.git_path(&signer)], None);
    f.git.run(
        &["config", "user.signingkey", "FIXTURE_KEY_NOT_A_SECRET"],
        None,
    );
    let c = catalog(&f).await;
    let target = handle(&f, &c, "HEAD").await;
    let r = execute(
        &f,
        ActionIntent::TagCreate {
            name: "no-signature".into(),
            target_handle: target,
            annotation: TagAnnotation::Annotated {
                message: "MESSAGE_CANARY".into(),
                sign: true,
            },
        },
    )
    .await;
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::NotApplied,
        "{r:?}"
    );
    assert_eq!(
        r.result.unwrap().steps[0].failure_reason,
        Some(RefActionFailureReason::Signing)
    );
    let record =
        opengit_lib::actions::journal::read(&f.backend.registry.database, &r.operation.id).unwrap();
    let text = serde_json::to_string(&record).unwrap();
    assert!(!text.contains("MESSAGE_CANARY"));
    assert!(!text.contains("signing CANARY"));
    assert!(!serde_json::to_string(&r.operation.error)
        .unwrap()
        .contains("CANARY"));
}

#[tokio::test]
async fn unborn_bare_and_detached_catalogs_have_explicit_local_behavior() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    let ordinary = catalog(&f).await;
    assert!(
        ordinary
            .refs
            .iter()
            .find(|r| r.name == "refs/heads/main")
            .unwrap()
            .checked_out_here
    );
    let unborn = f.git.owner.child("unborn refs").unwrap();
    std::fs::create_dir(&unborn).unwrap();
    f.git.run_at(&unborn, &["init", "-b", "main"], None);
    let (b, hs) = f.git.backend_at(&unborn, "unborn").await;
    let scope = ActionScope {
        repo_id: hs.repo_id,
        worktree_id: hs.worktree_id,
        generation: hs.generation,
    };
    let c = b
        .get_ref_actions(
            "unborn",
            GetRefActionsRequest {
                scope: scope.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(c.head.kind, HeadKind::Unborn);
    assert!(c.refs.is_empty());
    assert_eq!(
        b.resolve_ref_action_target(
            "unborn",
            ResolveRefActionTargetRequest {
                scope,
                catalog_id: c.catalog_id,
                source: RefActionSource::Head
            }
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::InvalidHandle
    );
    assert!(b.get_operations("unborn").unwrap().operations.is_empty());
    let (b, hs) = f.git.backend_at(&f.remote, "bare").await;
    let b = std::sync::Arc::new(b);
    let scope = ActionScope {
        repo_id: hs.repo_id,
        worktree_id: hs.worktree_id,
        generation: hs.generation,
    };
    let c = b
        .get_ref_actions(
            "bare",
            GetRefActionsRequest {
                scope: scope.clone(),
            },
        )
        .await
        .unwrap();
    assert!(c.bare);
    assert!(c.refs.iter().all(|r| !r.checked_out_here));
    let current_branch = c.refs.iter().find(|r| r.name == "refs/heads/main").unwrap();
    let branch = b
        .resolve_ref_action_target(
            "bare",
            ResolveRefActionTargetRequest {
                scope: scope.clone(),
                catalog_id: c.catalog_id.clone(),
                source: RefActionSource::Ref {
                    entry_id: current_branch.entry_id.clone(),
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(
        b.prepare_repository_action(
            "bare",
            PrepareRepositoryActionRequest {
                scope: scope.clone(),
                intent: ActionIntent::BranchDelete {
                    branch_handle: branch.handle
                }
            }
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::UnsupportedAction
    );
    let target = b
        .resolve_ref_action_target(
            "bare",
            ResolveRefActionTargetRequest {
                scope: scope.clone(),
                catalog_id: c.catalog_id,
                source: RefActionSource::Head,
            },
        )
        .await
        .unwrap();
    assert!(b
        .prepare_repository_action(
            "bare",
            PrepareRepositoryActionRequest {
                scope: scope.clone(),
                intent: ActionIntent::BranchCreate {
                    name: "bare-checkout".into(),
                    start_handle: target.handle.clone(),
                    checkout: true
                }
            }
        )
        .await
        .is_err());
    let plan = b
        .prepare_repository_action(
            "bare",
            PrepareRepositoryActionRequest {
                scope: scope.clone(),
                intent: ActionIntent::BranchCreate {
                    name: "bare-created".into(),
                    start_handle: target.handle,
                    checkout: false,
                },
            },
        )
        .await
        .unwrap();
    let op = b
        .execute_repository_action(
            "bare",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    let op = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let value = b
                .get_ref_action_outcome(
                    "bare",
                    GetActionOutcomeRequest {
                        operation_id: op.id.clone(),
                    },
                )
                .unwrap();
            if opengit_lib::operations::state::terminal(&value.operation.state) {
                break value;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    applied(&op);
    assert_eq!(
        String::from_utf8(
            f.git
                .run_at(&f.remote, &["rev-parse", "refs/heads/bare-created"], None)
        )
        .unwrap()
        .trim(),
        f.git.oid("HEAD")
    );
    f.git.run(&["switch", "--detach", "HEAD"], None);
    let c = catalog(&f).await;
    assert_eq!(c.head.kind, HeadKind::Detached);
    let target = handle(&f, &c, "HEAD").await;
    applied(
        &execute(
            &f,
            ActionIntent::BranchCreate {
                name: "from-detached".into(),
                start_handle: target,
                checkout: true,
            },
        )
        .await,
    );
}
#[tokio::test]
async fn in_progress_unmerged_index_stale_plan_and_database_failure_prevent_local_writes() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    let c = catalog(&f).await;
    let target = handle(&f, &c, "HEAD").await;
    let plan = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::BranchCreate {
                    name: "stale-plan".into(),
                    start_handle: target,
                    checkout: false,
                },
            },
        )
        .await
        .unwrap();
    f.git
        .run(&["config", "localchange.value", "external"], None);
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    let outcome = f.wait(&op.id).await;
    assert_eq!(
        outcome.operation.outcome.unwrap().effects,
        ActionEffects::NotApplied
    );
    std::fs::write(
        f.git.path.join(".git/MERGE_HEAD"),
        format!("{}\n", f.git.oid("HEAD")),
    )
    .unwrap();
    let c = catalog(&f).await;
    let target = handle(&f, &c, "HEAD").await;
    assert!(f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::BranchCreate {
                    name: "busy-ref".into(),
                    start_handle: target,
                    checkout: false
                }
            }
        )
        .await
        .is_err());
    std::fs::remove_file(f.git.path.join(".git/MERGE_HEAD")).unwrap();
    let blob = String::from_utf8(
        f.git
            .run(&["hash-object", "-w", "--stdin"], Some(b"conflict")),
    )
    .unwrap()
    .trim()
    .to_owned();
    f.git.run(
        &["update-index", "--index-info"],
        Some(
            format!(
                "100644 {blob} 1\tconflict\n100644 {blob} 2\tconflict\n100644 {blob} 3\tconflict\n"
            )
            .as_bytes(),
        ),
    );
    let c = catalog(&f).await;
    let target = handle(&f, &c, "HEAD").await;
    assert!(f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::BranchCreate {
                    name: "unmerged-ref".into(),
                    start_handle: target,
                    checkout: false
                }
            }
        )
        .await
        .is_err());
    f.git.run(&["read-tree", "HEAD"], None);
    let c = catalog(&f).await;
    let target = handle(&f, &c, "HEAD").await;
    let plan = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::BranchCreate {
                    name: "db-refused".into(),
                    start_handle: target,
                    checkout: false,
                },
            },
        )
        .await
        .unwrap();
    f.backend.registry.database.lock().unwrap().execute_batch("CREATE TRIGGER refuse_ref_write BEFORE UPDATE ON repository_actions BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    let result = f.wait(&op.id).await;
    assert_eq!(
        result.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::NotApplied
    );
    let refs =
        String::from_utf8(f.git.run(&["for-each-ref", "--format=%(refname)"], None)).unwrap();
    for name in ["stale-plan", "busy-ref", "unmerged-ref", "db-refused"] {
        assert!(!refs.contains(name));
    }
}

#[tokio::test]
async fn cancellation_during_owned_local_process_is_not_false_cancelled_or_late_write() {
    let mut f = support::daily::Fixture::new("sha1", 8, "files").await;
    let proxy = support::daily::Proxy::new(&f.git);
    f.rebind(proxy.runner.clone(), None).await;
    let c = catalog(&f).await;
    let target = handle(&f, &c, "HEAD").await;
    let plan = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::BranchCreate {
                    name: "cancelled-native".into(),
                    start_handle: target,
                    checkout: false,
                },
            },
        )
        .await
        .unwrap();
    proxy.set("localhold");
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    proxy.wait_ready().await;
    f.backend
        .cancel_operation(
            "main",
            CancelOperationRequest {
                operation_id: op.id.clone(),
            },
        )
        .unwrap();
    let r = f.wait(&op.id).await;
    assert_ne!(r.operation.state, OperationState::Cancelled);
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::Unknown
    );
    assert!(r.operation.outcome.as_ref().unwrap().cancellation_requested);
    proxy.release();
    proxy.set("normal");
    let refs =
        String::from_utf8(f.git.run(&["for-each-ref", "--format=%(refname)"], None)).unwrap();
    assert!(!refs.contains("cancelled-native"));
    let c = catalog(&f).await;
    let target = handle(&f, &c, "HEAD").await;
    applied(
        &execute(
            &f,
            ActionIntent::BranchCreate {
                name: "after-verified-stop".into(),
                start_handle: target,
                checkout: false,
            },
        )
        .await,
    );
    assert!(
        !String::from_utf8(f.git.run(&["for-each-ref", "--format=%(refname)"], None))
            .unwrap()
            .contains("cancelled-native")
    );
}

fn branch_config_bytes(path: &std::path::Path, name: &str) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap();
    let header = format!("[branch \"{name}\"]");
    let mut found = false;
    let mut result = Vec::new();
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        if line.starts_with(b"[") {
            if found {
                break;
            }
            found = line.trim_ascii_end() == header.as_bytes();
        }
        if found {
            result.extend_from_slice(line);
        }
    }
    assert!(found, "owned fixture subsection absent");
    result
}

#[tokio::test]
async fn config_subsections_rename_and_delete_preserve_dotted_and_case_distinct_siblings() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    for name in ["foo", "foo.bar"] {
        f.git.run(&["branch", name, "HEAD"], None);
    }
    for (name, merge) in [("foo", "main"), ("foo.bar", "alpha"), ("Foo", "beta")] {
        f.git.run(
            &["config", &format!("branch.{name}.remote"), "origin"],
            None,
        );
        f.git.run(
            &[
                "config",
                &format!("branch.{name}.merge"),
                &format!("refs/heads/{merge}"),
            ],
            None,
        );
        f.git.run(
            &[
                "config",
                &format!("branch.{name}.description"),
                &format!("preserve {name} bytes"),
            ],
            None,
        );
    }
    let path = f.git.path.join(".git/config");
    let sibling = branch_config_bytes(&path, "foo.bar");
    let case = branch_config_bytes(&path, "Foo");
    let sibling_oid = f.git.oid("refs/heads/foo.bar");
    let before = f.worktree_state();
    let c = catalog(&f).await;
    let branch = handle(&f, &c, "refs/heads/foo").await;
    applied(
        &execute(
            &f,
            ActionIntent::BranchRename {
                branch_handle: branch,
                new_name: "renamed".into(),
            },
        )
        .await,
    );
    assert_eq!(branch_config_bytes(&path, "foo.bar"), sibling);
    assert_eq!(branch_config_bytes(&path, "Foo"), case);
    assert_eq!(f.git.oid("refs/heads/foo.bar"), sibling_oid);
    assert_eq!(f.worktree_state(), before);
    f.git.run(&["branch", "foo", "HEAD"], None);
    f.git.run(&["config", "branch.foo.remote", "origin"], None);
    f.git
        .run(&["config", "branch.foo.merge", "refs/heads/main"], None);
    let c = catalog(&f).await;
    let branch = handle(&f, &c, "refs/heads/foo").await;
    let result = execute(
        &f,
        ActionIntent::BranchDelete {
            branch_handle: branch,
        },
    )
    .await;
    applied(&result);
    assert_eq!(result.result.unwrap().steps.len(), 2);
    assert_eq!(branch_config_bytes(&path, "foo.bar"), sibling);
    assert_eq!(branch_config_bytes(&path, "Foo"), case);
    assert_eq!(f.git.oid("refs/heads/foo.bar"), sibling_oid);
    assert_eq!(f.worktree_state(), before);
}

#[tokio::test]
async fn config_subsections_inherited_siblings_do_not_block_but_exact_own_inheritance_does() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    for name in ["foo", "foo.bar"] {
        f.git.run(&["branch", name, "HEAD"], None);
    }
    f.git.run(&["config", "branch.foo.remote", "origin"], None);
    f.git
        .run(&["config", "branch.foo.merge", "refs/heads/main"], None);
    let include = f.git.owner.child("inherited branch config.inc").unwrap();
    let original = b"# owned include sentinel\n[branch \"foo.bar\"]\n remote = origin\n merge = refs/heads/alpha\n[branch \"destination.bar\"]\n remote = origin\n merge = refs/heads/beta\n[branch \"Foo\"]\n description = case-sensitive sibling\n";
    std::fs::write(&include, original).unwrap();
    f.git.run(
        &["config", "--add", "include.path", &f.git.git_path(&include)],
        None,
    );
    let c = catalog(&f).await;
    let branch = handle(&f, &c, "refs/heads/foo").await;
    applied(
        &execute(
            &f,
            ActionIntent::BranchRename {
                branch_handle: branch,
                new_name: "destination".into(),
            },
        )
        .await,
    );
    assert_eq!(std::fs::read(&include).unwrap(), original);
    assert_eq!(f.git.oid("refs/heads/foo.bar"), f.git.oid("HEAD"));
    let c = catalog(&f).await;
    let branch = handle(&f, &c, "refs/heads/destination").await;
    applied(
        &execute(
            &f,
            ActionIntent::BranchDelete {
                branch_handle: branch,
            },
        )
        .await,
    );
    assert_eq!(std::fs::read(&include).unwrap(), original);
    f.git.run(&["branch", "foo", "HEAD"], None);
    let mut inherited = original.to_vec();
    inherited.extend_from_slice(b"[branch \"foo\"]\n remote = origin\n merge = refs/heads/main\n");
    std::fs::write(&include, &inherited).unwrap();
    let c = catalog(&f).await;
    let branch = handle(&f, &c, "refs/heads/foo").await;
    let before = f.worktree_state();
    let refs = f
        .git
        .run(&["for-each-ref", "--format=%(refname) %(objectname)"], None);
    let config = std::fs::read(f.git.path.join(".git/config")).unwrap();
    for intent in [
        ActionIntent::BranchRename {
            branch_handle: branch.clone(),
            new_name: "blocked".into(),
        },
        ActionIntent::BranchDelete {
            branch_handle: branch.clone(),
        },
        ActionIntent::BranchTracking {
            branch_handle: branch,
            upstream_handle: None,
        },
    ] {
        assert_eq!(
            f.backend
                .prepare_repository_action(
                    "main",
                    PrepareRepositoryActionRequest {
                        scope: f.scope.clone(),
                        intent
                    }
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedAction
        );
    }
    assert_eq!(std::fs::read(&include).unwrap(), inherited);
    assert_eq!(
        std::fs::read(f.git.path.join(".git/config")).unwrap(),
        config
    );
    assert_eq!(f.worktree_state(), before);
    assert_eq!(
        f.git
            .run(&["for-each-ref", "--format=%(refname) %(objectname)"], None),
        refs
    );
}

#[tokio::test]
async fn config_subsections_sibling_only_config_does_not_schedule_nonexistent_cleanup() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    for name in ["foo", "foo.bar"] {
        f.git.run(&["branch", name, "HEAD"], None);
    }
    f.git
        .run(&["config", "branch.foo.bar.remote", "origin"], None);
    f.git.run(
        &["config", "branch.foo.bar.merge", "refs/heads/alpha"],
        None,
    );
    let path = f.git.path.join(".git/config");
    let before = std::fs::read(&path).unwrap();
    let c = catalog(&f).await;
    let branch = handle(&f, &c, "refs/heads/foo").await;
    let result = execute(
        &f,
        ActionIntent::BranchDelete {
            branch_handle: branch,
        },
    )
    .await;
    applied(&result);
    assert_eq!(result.result.unwrap().steps.len(), 1);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(f.git.oid("refs/heads/foo.bar"), f.git.oid("HEAD"));
}
