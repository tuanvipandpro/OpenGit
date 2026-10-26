#[allow(dead_code)]
mod support;
use opengit_lib::domain::contracts::*;
use support::daily::Fixture;
const MESSAGE: &str = "STASH_MESSAGE_CANARY_MEMORY_AND_GIT_ONLY";
fn setup(f: &Fixture) {
    f.git.run(&["config", "core.autocrlf", "false"], None);
    for name in ["alpha.txt", "beta.txt"] {
        std::fs::write(f.git.path.join(name), b"base\n").unwrap();
    }
    std::fs::write(f.git.path.join(".gitignore"), b"ignored.tmp\n").unwrap();
    f.git
        .run(&["add", "--", "alpha.txt", "beta.txt", ".gitignore"], None);
    f.git.run(&["commit", "-m", "stash base"], None);
}
async fn catalog(f: &Fixture) -> RepositoryStashes {
    let value = f
        .backend
        .get_repository_stashes(
            "main",
            GetRepositoryStashesRequest {
                scope: f.scope.clone(),
            },
        )
        .await
        .unwrap();
    value
}
async fn execute(f: &Fixture, intent: ActionIntent) -> StashActionOperationResult {
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
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    f.wait(&op.id).await;
    f.backend
        .get_stash_action_outcome(
            "main",
            GetStashActionOutcomeRequest {
                operation_id: op.id.clone(),
            },
        )
        .unwrap()
}
async fn save(f: &Fixture, untracked: bool) -> StashActionOperationResult {
    Box::pin(catalog(f)).await;
    Box::pin(execute(
        f,
        ActionIntent::StashPush {
            message: MESSAGE.into(),
            include_untracked: untracked,
        },
    ))
    .await
}
async fn apply(f: &Fixture, ordinal: usize) -> StashActionOperationResult {
    let catalog = Box::pin(catalog(f)).await;
    let target = f
        .backend
        .resolve_stash_target(
            "main",
            ResolveStashTargetRequest {
                scope: f.scope.clone(),
                catalog_id: catalog.catalog_id,
                entry_id: catalog.entries[ordinal].entry_id.clone(),
            },
        )
        .await
        .unwrap();
    Box::pin(execute(
        f,
        ActionIntent::StashApply {
            stash_handle: target.stash_handle,
        },
    ))
    .await
}
async fn stage(f: &Fixture, name: &str) -> ChangeActionOperationResult {
    let catalog = f
        .backend
        .get_repository_changes(
            "main",
            GetRepositoryChangesRequest {
                scope: f.scope.clone(),
            },
        )
        .await
        .unwrap();
    assert!(!catalog.unsupported, "{catalog:?}");
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.path_display == name)
        .unwrap();
    assert_eq!(
        entry.stage_capability,
        ChangeCapability::Eligible,
        "{entry:?}"
    );
    assert_eq!(
        entry.unstage_capability,
        ChangeCapability::Unsupported,
        "{entry:?}"
    );
    let target = f
        .backend
        .resolve_change_targets(
            "main",
            ResolveChangeTargetsRequest {
                scope: f.scope.clone(),
                catalog_id: catalog.catalog_id,
                targets: vec![ChangeSelection {
                    entry_id: entry.entry_id.clone(),
                    side: ChangeSide::Unstaged,
                }],
            },
        )
        .await
        .unwrap();
    let plan = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::Stage {
                    change_handles: vec![target.targets[0].change_handle.clone()],
                },
            },
        )
        .await
        .unwrap();
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    f.wait(&op.id).await;
    f.backend
        .get_change_action_outcome(
            "main",
            GetActionOutcomeRequest {
                operation_id: op.id,
            },
        )
        .unwrap()
}
fn assert_effects(result: &StashActionOperationResult, expected: ActionEffects) {
    assert_eq!(
        result.operation.outcome.as_ref().unwrap().effects,
        expected,
        "{result:?}"
    );
}
fn rewind_owned_stash_checkpoint(
    database: &opengit_lib::persistence::workspace::Database,
    snapshot: &OperationSnapshot,
    phase: &str,
) {
    let mut record =
        serde_json::to_value(opengit_lib::actions::journal::read(database, &snapshot.id).unwrap())
            .unwrap();
    record["phase"] = "writeStarted".into();
    record["outcome"] = serde_json::Value::Null;
    record["stashAction"]["phase"] = phase.into();
    record["stashAction"]["effects"] = "unknown".into();
    record["stashAction"]["result"]["recoveryRefRetention"] = "unknown".into();
    let mut operation = serde_json::to_value(snapshot).unwrap();
    operation["state"] = "Running".into();
    operation["phase"] = "publish".into();
    operation["outcome"] = serde_json::Value::Null;
    operation["error"] = serde_json::Value::Null;
    let mut connection = database.lock().unwrap();
    let tx = connection.transaction().unwrap();
    tx.execute(
        "UPDATE repository_actions SET record=?2 WHERE operation_id=?1",
        rusqlite::params![snapshot.id, record.to_string()],
    )
    .unwrap();
    tx.execute(
        "UPDATE operations SET snapshot=?2 WHERE id=?1",
        rusqlite::params![snapshot.id, operation.to_string()],
    )
    .unwrap();
    tx.commit().unwrap();
}
fn database_dir(f: &Fixture) -> std::path::PathBuf {
    let database = f.backend.registry.database.lock().unwrap();
    let file: String = database
        .query_row("PRAGMA database_list", [], |row| row.get(2))
        .unwrap();
    std::path::PathBuf::from(file)
        .parent()
        .unwrap()
        .to_path_buf()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_exact_staged_working_and_opted_untracked_then_apply_keeps_stash_without_index_partition(
) {
    let f = Box::pin(Fixture::new("sha1", 6, "files")).await;
    setup(&f);
    std::fs::write(f.git.path.join("alpha.txt"), b"staged alpha\n").unwrap();
    f.git.run(&["add", "--", "alpha.txt"], None);
    std::fs::write(f.git.path.join("alpha.txt"), b"working alpha\n").unwrap();
    std::fs::write(f.git.path.join("beta.txt"), b"working beta\n").unwrap();
    std::fs::write(f.git.path.join("loose.txt"), b"loose data\n").unwrap();
    std::fs::write(f.git.path.join("ignored.tmp"), b"ignored data\n").unwrap();
    let before_head = f.git.oid("HEAD");
    let saved = Box::pin(save(&f, true)).await;
    assert_effects(&saved, ActionEffects::Applied);
    assert_eq!(
        saved.operation.state,
        OperationState::Succeeded,
        "{saved:?}"
    );
    let value = saved.result.as_ref().unwrap().stash_oid.clone().unwrap();
    assert_eq!(f.git.oid("refs/stash"), value);
    assert_eq!(
        f.git.run(&["show", &format!("{value}:alpha.txt")], None),
        b"working alpha\n"
    );
    assert_eq!(
        f.git.run(&["show", &format!("{value}:beta.txt")], None),
        b"working beta\n"
    );
    assert_eq!(
        f.git.run(&["show", &format!("{value}^2:alpha.txt")], None),
        b"staged alpha\n"
    );
    assert_eq!(
        f.git.run(&["show", &format!("{value}^2:beta.txt")], None),
        b"base\n"
    );
    assert_eq!(
        f.git.run(&["show", &format!("{value}^3:loose.txt")], None),
        b"loose data\n"
    );
    assert!(!String::from_utf8(f.git.run(
        &["ls-tree", "-r", "--name-only", &format!("{value}^3")],
        None
    ))
    .unwrap()
    .contains("ignored.tmp"));
    assert!(!f.git.path.join("loose.txt").exists());
    for name in ["alpha.txt", "beta.txt"] {
        assert_eq!(std::fs::read(f.git.path.join(name)).unwrap(), b"base\n");
    }
    let applied = Box::pin(apply(&f, 0)).await;
    assert_effects(&applied, ActionEffects::Applied);
    assert_eq!(
        applied.operation.state,
        OperationState::Succeeded,
        "{applied:?}"
    );
    assert_eq!(f.git.oid("HEAD"), before_head);
    assert_eq!(f.git.oid("refs/stash"), value);
    assert_eq!(f.git.run(&["show", ":alpha.txt"], None), b"base\n");
    assert_eq!(f.git.run(&["show", ":beta.txt"], None), b"base\n");
    assert_eq!(
        std::fs::read(f.git.path.join("alpha.txt")).unwrap(),
        b"working alpha\n"
    );
    assert_eq!(
        std::fs::read(f.git.path.join("beta.txt")).unwrap(),
        b"working beta\n"
    );
    assert_eq!(
        std::fs::read(f.git.path.join("loose.txt")).unwrap(),
        b"loose data\n"
    );
    assert_eq!(
        std::fs::read(f.git.path.join("ignored.tmp")).unwrap(),
        b"ignored data\n"
    );
    assert!(f
        .git
        .run(
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/opengit/recovery/"
            ],
            None
        )
        .is_empty());
    let tracked = Box::pin(save(&f, false)).await;
    assert_effects(&tracked, ActionEffects::Applied);
    let second = tracked.result.as_ref().unwrap().stash_oid.clone().unwrap();
    assert_ne!(second, value);
    assert_eq!(
        f.git
            .run(&["cat-file", "-p", &second], None)
            .split(|b| *b == b'\n')
            .filter(|line| line.starts_with(b"parent "))
            .count(),
        2
    );
    assert_eq!(
        std::fs::read(f.git.path.join("loose.txt")).unwrap(),
        b"loose data\n"
    );
    assert_eq!(
        std::fs::read(f.git.path.join("ignored.tmp")).unwrap(),
        b"ignored data\n"
    );
    for operation in [
        saved.operation.id,
        applied.operation.id,
        tracked.operation.id,
    ] {
        let record =
            opengit_lib::actions::journal::read(&f.backend.registry.database, &operation).unwrap();
        let record = serde_json::to_string(&record).unwrap();
        for secret in [MESSAGE, "working alpha", "loose data", "ignored data"] {
            assert!(!record.contains(secret), "{secret}");
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apply_conflicts_without_merge_head_stage_one_preserves_other_stages_then_reconcile_is_read_only(
) {
    let f = Box::pin(Fixture::new("sha256", 6, "reftable")).await;
    setup(&f);
    for name in ["alpha.txt", "beta.txt"] {
        std::fs::write(f.git.path.join(name), b"stashed version\n").unwrap();
    }
    let saved = Box::pin(save(&f, false)).await;
    assert_effects(&saved, ActionEffects::Applied);
    let value = saved.result.unwrap().stash_oid.unwrap();
    for name in ["alpha.txt", "beta.txt"] {
        std::fs::write(f.git.path.join(name), b"external version\n").unwrap();
    }
    f.git.run(&["add", "--", "alpha.txt", "beta.txt"], None);
    f.git
        .run(&["commit", "-m", "external conflict baseline"], None);
    let head = f.git.oid("HEAD");
    let applied = Box::pin(apply(&f, 0)).await;
    assert_effects(&applied, ActionEffects::Conflict);
    assert_eq!(
        applied.result.as_ref().unwrap().resolution,
        StashResolution::Unmerged
    );
    assert!(!f.git.path.join(".git/MERGE_HEAD").exists());
    assert_eq!(f.git.oid("refs/stash"), value);
    let beta_stages = f
        .git
        .run(&["ls-files", "--unmerged", "-z", "--", "beta.txt"], None);
    assert_eq!(
        beta_stages
            .split(|b| *b == 0)
            .filter(|b| !b.is_empty())
            .count(),
        3
    );
    let beta_work = std::fs::read(f.git.path.join("beta.txt")).unwrap();
    std::fs::write(f.git.path.join("alpha.txt"), b"resolved alpha\n").unwrap();
    let staged = Box::pin(stage(&f, "alpha.txt")).await;
    assert_eq!(
        staged.operation.state,
        OperationState::Succeeded,
        "{staged:?}"
    );
    assert_eq!(staged.result.unwrap().tree_oid, None);
    assert_eq!(
        f.git
            .run(&["ls-files", "--unmerged", "-z", "--", "beta.txt"], None),
        beta_stages
    );
    assert_eq!(
        std::fs::read(f.git.path.join("beta.txt")).unwrap(),
        beta_work
    );
    assert_eq!(
        f.git.run(&["show", ":alpha.txt"], None),
        b"resolved alpha\n"
    );
    assert!(f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::Commit {
                    message: "must not commit unresolved beta".into(),
                    amend: false
                }
            }
        )
        .await
        .is_err());
    std::fs::write(f.git.path.join("beta.txt"), b"resolved beta\n").unwrap();
    let staged = Box::pin(stage(&f, "beta.txt")).await;
    assert_eq!(
        staged.operation.state,
        OperationState::Succeeded,
        "{staged:?}"
    );
    assert!(staged.result.unwrap().tree_oid.is_some());
    Box::pin(catalog(&f)).await;
    let state = f.worktree_state();
    let reconciled = f
        .backend
        .reconcile_stash_action(
            "main",
            ReconcileStashActionRequest {
                scope: f.scope.clone(),
                operation_id: applied.operation.id.clone(),
            },
        )
        .await
        .unwrap();
    assert_effects(&reconciled, ActionEffects::Conflict);
    assert_eq!(
        reconciled.result.as_ref().unwrap().resolution,
        StashResolution::ResolvedExternally
    );
    assert_eq!(
        reconciled.result.as_ref().unwrap().stash_retention,
        StashRetention::Present
    );
    assert_eq!(f.worktree_state(), state);
    assert_eq!(f.git.oid("HEAD"), head);
    assert_eq!(f.git.oid("refs/stash"), value);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_stack_dirty_tracked_and_untracked_collision_reject_before_pin_or_apply() {
    let f = Box::pin(Fixture::new("sha1", 6, "files")).await;
    setup(&f);
    std::fs::write(f.git.path.join("alpha.txt"), b"saved alpha\n").unwrap();
    std::fs::write(f.git.path.join("loose.txt"), b"saved loose\n").unwrap();
    let saved = Box::pin(save(&f, true)).await;
    assert_effects(&saved, ActionEffects::Applied);
    let saved_oid = saved.result.unwrap().stash_oid.unwrap();

    let catalog = Box::pin(catalog(&f)).await;
    let entry = catalog.entries[0].entry_id.clone();
    let forged = f
        .backend
        .resolve_stash_target(
            "main",
            ResolveStashTargetRequest {
                scope: f.scope.clone(),
                catalog_id: "forged-catalog".into(),
                entry_id: entry.clone(),
            },
        )
        .await;
    assert!(forged.is_err());
    let resolved = f
        .backend
        .resolve_stash_target(
            "main",
            ResolveStashTargetRequest {
                scope: f.scope.clone(),
                catalog_id: catalog.catalog_id,
                entry_id: entry,
            },
        )
        .await
        .unwrap();
    let handle = resolved.stash_handle;
    for intent in [
        ActionIntent::StashPop {
            stash_handle: handle.clone(),
        },
        ActionIntent::StashDrop {
            stash_handle: handle.clone(),
        },
    ] {
        assert!(f
            .backend
            .prepare_repository_action(
                "main",
                PrepareRepositoryActionRequest {
                    scope: f.scope.clone(),
                    intent,
                },
            )
            .await
            .is_err());
    }
    let prepare_apply = || PrepareRepositoryActionRequest {
        scope: f.scope.clone(),
        intent: ActionIntent::StashApply {
            stash_handle: handle.clone(),
        },
    };

    std::fs::write(f.git.path.join("alpha.txt"), b"unsaved alpha\n").unwrap();
    assert!(f
        .backend
        .prepare_repository_action("main", prepare_apply())
        .await
        .is_err());
    assert_eq!(
        std::fs::read(f.git.path.join("alpha.txt")).unwrap(),
        b"unsaved alpha\n"
    );
    std::fs::write(f.git.path.join("alpha.txt"), b"base\n").unwrap();
    std::fs::write(f.git.path.join("loose.txt"), b"foreign loose\n").unwrap();
    assert!(f
        .backend
        .prepare_repository_action("main", prepare_apply())
        .await
        .is_err());
    assert_eq!(
        std::fs::read(f.git.path.join("loose.txt")).unwrap(),
        b"foreign loose\n"
    );
    assert!(f
        .git
        .run(
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/opengit/recovery/"
            ],
            None
        )
        .is_empty());

    std::fs::remove_file(f.git.path.join("loose.txt")).unwrap();
    std::fs::write(f.git.path.join("beta.txt"), b"external stack change\n").unwrap();
    f.git
        .run(&["stash", "push", "-m", "external stack change"], None);
    assert!(f
        .backend
        .prepare_repository_action("main", prepare_apply())
        .await
        .is_err());
    assert_eq!(f.git.oid("stash@{1}"), saved_oid);
    assert!(f
        .git
        .run(
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/opengit/recovery/"
            ],
            None
        )
        .is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn durable_before_write_db_fault_keeps_native_save_unstarted() {
    let f = Box::pin(Fixture::new("sha1", 6, "files")).await;
    setup(&f);
    std::fs::write(f.git.path.join("alpha.txt"), b"unsaved data\n").unwrap();
    Box::pin(catalog(&f)).await;
    let plan = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::StashPush {
                    message: MESSAGE.into(),
                    include_untracked: false,
                },
            },
        )
        .await
        .unwrap();
    f.backend
        .registry
        .database
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER stash_write_fault BEFORE UPDATE ON repository_actions \
         WHEN json_extract(NEW.record, '$.phase') = 'writeStarted' \
         BEGIN SELECT RAISE(ABORT, 'owned stash fixture'); END;",
        )
        .unwrap();
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    f.wait(&op.id).await;
    f.backend
        .registry
        .database
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER stash_write_fault;")
        .unwrap();
    let outcome = f
        .backend
        .get_stash_action_outcome(
            "main",
            GetStashActionOutcomeRequest {
                operation_id: op.id.clone(),
            },
        )
        .unwrap();
    assert_effects(&outcome, ActionEffects::NotApplied);
    assert!(!f
        .git
        .run(&["status", "--porcelain=v2", "-z"], None)
        .is_empty());
    assert_eq!(
        std::fs::read(f.git.path.join("alpha.txt")).unwrap(),
        b"unsaved data\n"
    );
    assert!(f
        .git
        .run(&["for-each-ref", "--format=%(refname)", "refs/stash"], None)
        .is_empty());
    let record = opengit_lib::actions::journal::read(&f.backend.registry.database, &op.id).unwrap();
    assert_eq!(
        record.phase,
        opengit_lib::actions::journal::WritePhase::Observed
    );
    assert_eq!(
        serde_json::to_value(record).unwrap()["stashAction"]["phase"],
        "reserved"
    );
}

#[tokio::test]
async fn cancelling_queued_save_never_starts_native_stash() {
    let f = Box::pin(Fixture::new("sha1", 6, "files")).await;
    setup(&f);
    std::fs::write(f.git.path.join("alpha.txt"), b"queued data\n").unwrap();
    Box::pin(catalog(&f)).await;
    let plan = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::StashPush {
                    message: MESSAGE.into(),
                    include_untracked: false,
                },
            },
        )
        .await
        .unwrap();
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    f.backend.scheduler.cancel(&op.id).unwrap();
    f.wait(&op.id).await;
    let outcome = f
        .backend
        .get_stash_action_outcome(
            "main",
            GetStashActionOutcomeRequest {
                operation_id: op.id,
            },
        )
        .unwrap();
    assert_effects(&outcome, ActionEffects::NotApplied);
    assert_eq!(
        std::fs::read(f.git.path.join("alpha.txt")).unwrap(),
        b"queued data\n"
    );
    assert!(f
        .git
        .run(&["for-each-ref", "--format=%(refname)", "refs/stash"], None)
        .is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_refuses_recreated_head_path_and_directory_collision_for_both_untracked_modes() {
    let f = Box::pin(Fixture::new("sha1", 6, "files")).await;
    setup(&f);
    std::fs::write(f.git.path.join("tracked.txt"), b"HEAD base\n").unwrap();
    f.git.run(&["add", "--", "tracked.txt"], None);
    f.git.run(&["commit", "-m", "tracked safety base"], None);
    let head = f.git.oid("HEAD");
    f.git.run(&["rm", "--cached", "--", "tracked.txt"], None);
    std::fs::write(
        f.git.path.join("tracked.txt"),
        b"UNTRACKED keep user data\n",
    )
    .unwrap();
    let index = std::fs::read(f.git.path.join(".git/index")).unwrap();
    for include_untracked in [false, true] {
        Box::pin(catalog(&f)).await;
        assert!(f
            .backend
            .prepare_repository_action(
                "main",
                PrepareRepositoryActionRequest {
                    scope: f.scope.clone(),
                    intent: ActionIntent::StashPush {
                        message: MESSAGE.into(),
                        include_untracked,
                    },
                },
            )
            .await
            .is_err());
        assert_eq!(
            std::fs::read(f.git.path.join("tracked.txt")).unwrap(),
            b"UNTRACKED keep user data\n"
        );
        assert_eq!(std::fs::read(f.git.path.join(".git/index")).unwrap(), index);
        assert_eq!(f.git.oid("HEAD"), head);
        assert!(f
            .git
            .run(&["for-each-ref", "--format=%(refname)", "refs/stash"], None)
            .is_empty());
    }
    std::fs::remove_file(f.git.path.join("tracked.txt")).unwrap();
    std::fs::create_dir(f.git.path.join("tracked.txt")).unwrap();
    std::fs::write(
        f.git.path.join("tracked.txt/child.txt"),
        b"prefix child retained\n",
    )
    .unwrap();
    Box::pin(catalog(&f)).await;
    assert!(f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::StashPush {
                    message: MESSAGE.into(),
                    include_untracked: false,
                },
            },
        )
        .await
        .is_err());
    assert_eq!(
        std::fs::read(f.git.path.join("tracked.txt/child.txt")).unwrap(),
        b"prefix child retained\n"
    );
    assert_eq!(std::fs::read(f.git.path.join(".git/index")).unwrap(), index);
    assert_eq!(f.git.oid("HEAD"), head);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_generation_reconcile_rejects_without_poisoning_fresh_catalog_or_plan() {
    tokio::spawn(old_generation_case()).await.unwrap();
}

async fn old_generation_case() {
    let mut f = Box::pin(Fixture::new("sha1", 6, "files")).await;
    setup(&f);
    f.scope.generation = "9".into();
    std::fs::write(f.git.path.join("alpha.txt"), b"generation nine saved\n").unwrap();
    let shared = std::sync::Arc::new(f);
    let for_save = shared.clone();
    let saved = tokio::spawn(async move { Box::pin(save(&for_save, false)).await })
        .await
        .unwrap();
    let mut f = std::sync::Arc::try_unwrap(shared).ok().unwrap();
    assert_effects(&saved, ActionEffects::Applied);
    let old_operation = saved.operation.id;
    let old_scope = f.scope.clone();
    f.backend.close_window("main");
    f.backend.get_workspace("main").unwrap();
    f.scope.generation = "1".into();

    let fresh = Box::pin(catalog(&f)).await;
    let resolved = f
        .backend
        .resolve_stash_target(
            "main",
            ResolveStashTargetRequest {
                scope: f.scope.clone(),
                catalog_id: fresh.catalog_id.clone(),
                entry_id: fresh.entries[0].entry_id.clone(),
            },
        )
        .await
        .unwrap();
    let plan = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::StashApply {
                    stash_handle: resolved.stash_handle,
                },
            },
        )
        .await
        .unwrap();
    let state = f.worktree_state();
    assert!(f
        .backend
        .reconcile_stash_action(
            "main",
            ReconcileStashActionRequest {
                scope: old_scope,
                operation_id: old_operation.clone(),
            },
        )
        .await
        .is_err());
    assert_eq!(f.worktree_state(), state);
    let current = f
        .backend
        .reconcile_stash_action(
            "main",
            ReconcileStashActionRequest {
                scope: f.scope.clone(),
                operation_id: old_operation,
            },
        )
        .await
        .unwrap();
    assert_effects(&current, ActionEffects::Applied);
    assert_eq!(f.worktree_state(), state);
    assert!(f
        .backend
        .resolve_stash_target(
            "main",
            ResolveStashTargetRequest {
                scope: f.scope.clone(),
                catalog_id: fresh.catalog_id,
                entry_id: fresh.entries[0].entry_id.clone(),
            },
        )
        .await
        .is_ok());
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    f.wait(&op.id).await;
    let applied = f
        .backend
        .get_stash_action_outcome(
            "main",
            GetStashActionOutcomeRequest {
                operation_id: op.id,
            },
        )
        .unwrap();
    assert_effects(&applied, ActionEffects::Applied);
    assert_eq!(
        std::fs::read(f.git.path.join("alpha.txt")).unwrap(),
        b"generation nine saved\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_held_prewrite_stash_child_prevents_late_native_save() {
    let mut f = Box::pin(Fixture::new("sha1", 6, "files")).await;
    setup(&f);
    let proxy = support::daily::Proxy::new(&f.git);
    f.rebind(proxy.runner.clone(), None).await;
    std::fs::write(f.git.path.join("alpha.txt"), b"held before native\n").unwrap();
    Box::pin(catalog(&f)).await;
    let plan = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::StashPush {
                    message: MESSAGE.into(),
                    include_untracked: false,
                },
            },
        )
        .await
        .unwrap();
    proxy.set("stash-prewrite-hold");
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
    assert_eq!(
        opengit_lib::actions::journal::read(&f.backend.registry.database, &op.id)
            .unwrap()
            .phase,
        opengit_lib::actions::journal::WritePhase::Prepared
    );
    f.backend
        .cancel_operation(
            "main",
            CancelOperationRequest {
                operation_id: op.id.clone(),
            },
        )
        .unwrap();
    std::fs::write(
        f.git.path.join("new_edit.txt"),
        b"new user edit after cancel\n",
    )
    .unwrap();
    proxy.release();
    let terminal = f.wait(&op.id).await;
    assert_eq!(
        terminal.operation.outcome.unwrap().effects,
        ActionEffects::NotApplied
    );
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    assert_eq!(
        std::fs::read(f.git.path.join("alpha.txt")).unwrap(),
        b"held before native\n"
    );
    assert_eq!(
        std::fs::read(f.git.path.join("new_edit.txt")).unwrap(),
        b"new user edit after cancel\n"
    );
    assert!(f
        .git
        .run(&["for-each-ref", "--format=%(refname)", "refs/stash"], None)
        .is_empty());
    assert_eq!(
        opengit_lib::actions::journal::read(&f.backend.registry.database, &op.id)
            .unwrap()
            .phase,
        opengit_lib::actions::journal::WritePhase::Observed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backend_restart_preserves_new_edits_foreign_ref_and_historical_stash_evidence() {
    tokio::spawn(save_restart(false)).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn push_started_restart_never_replays_save_or_cleans_new_edits() {
    tokio::spawn(save_restart(true)).await.unwrap();
}

async fn save_restart(transient_push: bool) {
    let f = Box::pin(Fixture::new("sha1", 6, "files")).await;
    setup(&f);
    std::fs::write(f.git.path.join("alpha.txt"), b"restart stash\n").unwrap();
    let saved = Box::pin(save(&f, false)).await;
    assert_effects(&saved, ActionEffects::Applied);
    let operation = saved.operation.id.clone();
    let stash_oid = saved.result.unwrap().stash_oid.unwrap();
    std::fs::write(f.git.path.join("new_edit.txt"), b"new edit after Save\n").unwrap();
    f.git.run(
        &["update-ref", "refs/opengit/recovery/foreign", "HEAD"],
        None,
    );
    let foreign_oid = f.git.oid("refs/opengit/recovery/foreign");
    let database = database_dir(&f);
    if transient_push {
        rewind_owned_stash_checkpoint(
            &f.backend.registry.database,
            &saved.operation,
            "pushStarted",
        );
    }
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while std::sync::Arc::strong_count(&f.backend) != 1 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let Fixture {
        backend,
        history_scope: _,
        scope: _,
        remote: _,
        git,
    } = f;
    drop(backend);
    let reopened = opengit_lib::backend::Backend::with_git(&database, git.git.clone()).unwrap();
    let selection = reopened
        .register_repository("main", &git.path)
        .await
        .unwrap()
        .active_selection
        .unwrap();
    let scope = ActionScope {
        repo_id: selection.repo_id,
        worktree_id: selection.worktree_id,
        generation: "1".into(),
    };
    let historical = reopened
        .get_stash_action_outcome(
            "main",
            GetStashActionOutcomeRequest {
                operation_id: operation.clone(),
            },
        )
        .unwrap();
    assert_effects(
        &historical,
        if transient_push {
            ActionEffects::Unknown
        } else {
            ActionEffects::Applied
        },
    );
    if transient_push {
        assert_eq!(historical.operation.state, OperationState::Interrupted);
        assert_eq!(
            historical.result.as_ref().unwrap().stash_retention,
            StashRetention::Unknown
        );
    }
    assert_eq!(
        historical.result.unwrap().stash_oid,
        Some(stash_oid.clone())
    );
    assert_eq!(
        std::fs::read(git.path.join("new_edit.txt")).unwrap(),
        b"new edit after Save\n"
    );
    assert_eq!(git.oid("refs/stash"), stash_oid);
    assert_eq!(git.oid("refs/opengit/recovery/foreign"), foreign_oid);
    let before = git.run(&["status", "--porcelain=v2", "-z"], None);
    reopened
        .get_repository_stashes(
            "main",
            GetRepositoryStashesRequest {
                scope: scope.clone(),
            },
        )
        .await
        .unwrap();
    let reconciled = reopened
        .reconcile_stash_action(
            "main",
            ReconcileStashActionRequest {
                scope,
                operation_id: operation,
            },
        )
        .await
        .unwrap();
    assert_effects(
        &reconciled,
        if transient_push {
            ActionEffects::Unknown
        } else {
            ActionEffects::Applied
        },
    );
    assert_eq!(
        reconciled.result.unwrap().stash_retention,
        StashRetention::Present
    );
    assert_eq!(git.run(&["status", "--porcelain=v2", "-z"], None), before);
    assert_eq!(
        std::fs::read(git.path.join("new_edit.txt")).unwrap(),
        b"new edit after Save\n"
    );
    assert_eq!(git.oid("refs/opengit/recovery/foreign"), foreign_oid);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ignored_collision_on_tracked_added_wip_after_pin_cannot_be_adopted() {
    let mut f = Box::pin(Fixture::new("sha1", 6, "files")).await;
    setup(&f);
    let proxy = support::daily::Proxy::new(&f.git);
    f.rebind(proxy.runner.clone(), None).await;
    std::fs::write(
        f.git.path.join("collision.txt"),
        b"saved tracked addition\n",
    )
    .unwrap();
    f.git.run(&["add", "--", "collision.txt"], None);
    let saved = Box::pin(save(&f, false)).await;
    assert_effects(&saved, ActionEffects::Applied);
    let stash_oid = saved.result.unwrap().stash_oid.unwrap();
    assert_eq!(
        f.git
            .run(&["show", &format!("{stash_oid}:collision.txt")], None),
        b"saved tracked addition\n"
    );
    assert_eq!(
        f.git
            .run(&["show", &format!("{stash_oid}^2:collision.txt")], None),
        b"saved tracked addition\n"
    );
    std::fs::write(f.git.path.join(".git/info/exclude"), b"collision.txt\n").unwrap();
    let current = Box::pin(catalog(&f)).await;
    let target = f
        .backend
        .resolve_stash_target(
            "main",
            ResolveStashTargetRequest {
                scope: f.scope.clone(),
                catalog_id: current.catalog_id,
                entry_id: current.entries[0].entry_id.clone(),
            },
        )
        .await
        .unwrap();
    let plan = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::StashApply {
                    stash_handle: target.stash_handle,
                },
            },
        )
        .await
        .unwrap();
    let head = f.git.oid("HEAD");
    let index = std::fs::read(f.git.path.join(".git/index")).unwrap();
    proxy.set("stash-after-pin-hold");
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
    assert!(
        proxy.fetched.is_file(),
        "owned recovery pin must precede collision read"
    );
    let pins = f.git.run(
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/opengit/recovery/",
        ],
        None,
    );
    assert!(!pins.is_empty());
    std::fs::write(
        f.git.path.join("collision.txt"),
        b"late ignored user data\n",
    )
    .unwrap();
    proxy.release();
    f.wait(&op.id).await;
    let native_marker = std::fs::read_to_string(&proxy.fetched).unwrap();
    assert!(native_marker.contains("pin created"));
    assert!(
        !native_marker.contains("apply resumed"),
        "native stash apply must not resume after a late ignored collision"
    );
    let outcome = f
        .backend
        .get_stash_action_outcome(
            "main",
            GetStashActionOutcomeRequest {
                operation_id: op.id,
            },
        )
        .unwrap();
    assert_ne!(
        outcome.operation.outcome.unwrap().effects,
        ActionEffects::Applied
    );
    assert_eq!(
        std::fs::read(f.git.path.join("collision.txt")).unwrap(),
        b"late ignored user data\n"
    );
    assert_eq!(std::fs::read(f.git.path.join(".git/index")).unwrap(), index);
    assert_eq!(f.git.oid("HEAD"), head);
    assert_eq!(f.git.oid("refs/stash"), stash_oid);
    assert_eq!(
        f.git.run(
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/opengit/recovery/"
            ],
            None
        ),
        pins
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn symlink_index_mode_is_reported_as_unsupported_before_stash_save() {
    let f = Box::pin(Fixture::new("sha1", 6, "files")).await;
    setup(&f);
    let oid = String::from_utf8(
        f.git
            .run(&["hash-object", "-w", "--stdin"], Some(b"target")),
    )
    .unwrap()
    .trim()
    .to_owned();
    f.git.run(
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("120000,{oid},link.txt"),
        ],
        None,
    );
    std::fs::write(f.git.path.join("alpha.txt"), b"dirty for stash\n").unwrap();
    let catalog = Box::pin(catalog(&f)).await;
    assert!(!catalog.save_available);
    let failure = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::StashPush {
                    message: MESSAGE.into(),
                    include_untracked: false,
                },
            },
        )
        .await
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::UnsupportedAction);
    assert!(failure.message.contains("regular tracked files"));
    assert!(f
        .git
        .run(&["for-each-ref", "--format=%(refname)", "refs/stash"], None)
        .is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_entry_removal_after_pin_refuses_apply_and_retains_exact_recovery_object() {
    let mut f = Box::pin(Fixture::new("sha1", 6, "files")).await;
    setup(&f);
    let proxy = support::daily::Proxy::new(&f.git);
    f.rebind(proxy.runner.clone(), None).await;
    std::fs::write(f.git.path.join("alpha.txt"), b"pinned apply content\n").unwrap();
    let saved = Box::pin(save(&f, false)).await;
    assert_effects(&saved, ActionEffects::Applied);
    let oid = saved.result.unwrap().stash_oid.unwrap();
    let current = Box::pin(catalog(&f)).await;
    let target = f
        .backend
        .resolve_stash_target(
            "main",
            ResolveStashTargetRequest {
                scope: f.scope.clone(),
                catalog_id: current.catalog_id,
                entry_id: current.entries[0].entry_id.clone(),
            },
        )
        .await
        .unwrap();
    let plan = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::StashApply {
                    stash_handle: target.stash_handle,
                },
            },
        )
        .await
        .unwrap();
    let head = f.git.oid("HEAD");
    let index = std::fs::read(f.git.path.join(".git/index")).unwrap();
    let working = std::fs::read(f.git.path.join("alpha.txt")).unwrap();
    proxy.set("stash-after-pin-hold");
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
    assert!(proxy.fetched.is_file());
    let pins = f.git.run(
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/opengit/recovery/",
        ],
        None,
    );
    let pin = std::str::from_utf8(&pins).unwrap().trim();
    assert!(!pin.is_empty());
    f.git.run(&["stash", "drop", "stash@{0}"], None);
    assert!(f
        .git
        .run(&["for-each-ref", "--format=%(refname)", "refs/stash"], None)
        .is_empty());
    assert_eq!(f.git.oid(pin), oid);
    proxy.release();
    f.wait(&op.id).await;
    let result = f
        .backend
        .get_stash_action_outcome(
            "main",
            GetStashActionOutcomeRequest {
                operation_id: op.id.clone(),
            },
        )
        .unwrap();
    assert_effects(&result, ActionEffects::NotApplied);
    assert_eq!(
        std::fs::read(f.git.path.join("alpha.txt")).unwrap(),
        working
    );
    assert_eq!(std::fs::read(f.git.path.join(".git/index")).unwrap(), index);
    assert_eq!(f.git.oid("HEAD"), head);
    assert!(f
        .git
        .run(&["for-each-ref", "--format=%(refname)", "refs/stash"], None)
        .is_empty());
    assert_eq!(f.git.oid(pin), oid);
    assert_eq!(
        result.result.as_ref().unwrap().stash_retention,
        StashRetention::Absent
    );
    assert_eq!(
        result.result.unwrap().recovery_ref_retention,
        StashRetention::Present
    );
    Box::pin(catalog(&f)).await;
    f.git.run(&["update-ref", "-d", pin, &oid], None);
    let missing = f
        .backend
        .reconcile_stash_action(
            "main",
            ReconcileStashActionRequest {
                scope: f.scope.clone(),
                operation_id: op.id.clone(),
            },
        )
        .await
        .unwrap();
    assert_effects(&missing, ActionEffects::NotApplied);
    assert_eq!(
        missing.result.unwrap().recovery_ref_retention,
        StashRetention::Absent
    );
    f.git.run(&["update-ref", pin, "HEAD"], None);
    let foreign = f.git.oid(pin);
    assert_ne!(foreign, oid);
    let replaced = f
        .backend
        .reconcile_stash_action(
            "main",
            ReconcileStashActionRequest {
                scope: f.scope.clone(),
                operation_id: op.id.clone(),
            },
        )
        .await
        .unwrap();
    assert_effects(&replaced, ActionEffects::NotApplied);
    assert_eq!(
        replaced.result.unwrap().recovery_ref_retention,
        StashRetention::Unknown
    );
    assert_eq!(f.git.oid(pin), foreign);
    f.git.run(&["update-ref", "-d", pin, &foreign], None);
    let foreign_target = "refs/opengit/recovery/foreign-target";
    f.git.run(&["update-ref", foreign_target, &oid], None);
    f.git.run(&["symbolic-ref", pin, foreign_target], None);
    assert_eq!(f.git.oid(pin), oid, "symref dereferences to the owned OID");
    let symbolic = f
        .backend
        .reconcile_stash_action(
            "main",
            ReconcileStashActionRequest {
                scope: f.scope.clone(),
                operation_id: op.id.clone(),
            },
        )
        .await
        .unwrap();
    assert_effects(&symbolic, ActionEffects::NotApplied);
    assert_eq!(
        symbolic.result.unwrap().recovery_ref_retention,
        StashRetention::Unknown
    );
    assert_eq!(f.git.oid(pin), oid);
    assert_eq!(f.git.oid(foreign_target), oid);
    assert_eq!(
        std::fs::read(f.git.path.join("alpha.txt")).unwrap(),
        working
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn db_fault_after_owned_pin_and_pin_created_restart_never_replays_apply() {
    tokio::spawn(db_after_pin_restart("pinCreated", true))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apply_started_restart_keeps_owned_pin_and_new_edits_without_replay() {
    tokio::spawn(db_after_pin_restart("applyStarted", true))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn matching_pin_without_durable_ownership_is_never_adopted_on_restart() {
    tokio::spawn(db_after_pin_restart("pinCreated", false))
        .await
        .unwrap();
}

async fn db_after_pin_restart(phase: &'static str, owned: bool) {
    let mut f = Box::pin(Fixture::new("sha1", 6, "files")).await;
    setup(&f);
    let proxy = support::daily::Proxy::new(&f.git);
    f.rebind(proxy.runner.clone(), None).await;
    std::fs::write(f.git.path.join("alpha.txt"), b"saved for pinned apply\n").unwrap();
    let saved = Box::pin(save(&f, false)).await;
    assert_effects(&saved, ActionEffects::Applied);
    let stash_oid = saved.result.unwrap().stash_oid.unwrap();
    let current = Box::pin(catalog(&f)).await;
    let target = f
        .backend
        .resolve_stash_target(
            "main",
            ResolveStashTargetRequest {
                scope: f.scope.clone(),
                catalog_id: current.catalog_id,
                entry_id: current.entries[0].entry_id.clone(),
            },
        )
        .await
        .unwrap();
    let plan = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::StashApply {
                    stash_handle: target.stash_handle,
                },
            },
        )
        .await
        .unwrap();
    proxy.set("stash-after-pin-hold");
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
    let pins = f.git.run(
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/opengit/recovery/",
        ],
        None,
    );
    let pin = std::str::from_utf8(&pins).unwrap().trim().to_owned();
    assert!(!pin.is_empty());
    assert_eq!(f.git.oid(&pin), stash_oid);
    f.backend
        .registry
        .database
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER stash_after_pin_fault BEFORE UPDATE ON repository_actions \
         WHEN json_extract(NEW.record, '$.stashAction.phase')='applyStarted' \
         BEGIN SELECT RAISE(ABORT, 'owned after-pin fixture'); END;",
        )
        .unwrap();
    proxy.release();
    f.wait(&op.id).await;
    f.backend
        .registry
        .database
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER stash_after_pin_fault;")
        .unwrap();
    let outcome = f
        .backend
        .get_stash_action_outcome(
            "main",
            GetStashActionOutcomeRequest {
                operation_id: op.id.clone(),
            },
        )
        .unwrap();
    assert_effects(&outcome, ActionEffects::Unknown);
    assert_eq!(f.git.oid(&pin), stash_oid);
    assert_eq!(f.git.oid("refs/stash"), stash_oid);
    assert_eq!(
        std::fs::read(f.git.path.join("alpha.txt")).unwrap(),
        b"base\n"
    );
    std::fs::write(f.git.path.join("new_edit.txt"), b"after fault user edit\n").unwrap();
    let database = database_dir(&f);
    rewind_owned_stash_checkpoint(&f.backend.registry.database, &outcome.operation, phase);
    if !owned {
        // Model a crash after native ref creation but before the journal
        // ownership checkpoint. Name+OID match alone grants no cleanup right.
        let mut record = serde_json::to_value(
            opengit_lib::actions::journal::read(&f.backend.registry.database, &op.id).unwrap(),
        )
        .unwrap();
        record["artifacts"] = serde_json::json!([]);
        f.backend
            .registry
            .database
            .lock()
            .unwrap()
            .execute(
                "UPDATE repository_actions SET record=?2 WHERE operation_id=?1",
                rusqlite::params![&op.id, record.to_string()],
            )
            .unwrap();
    }
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while std::sync::Arc::strong_count(&f.backend) != 1 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let Fixture {
        backend,
        history_scope: _,
        scope: _,
        remote: _,
        git,
    } = f;
    drop(backend);
    let reopened = opengit_lib::backend::Backend::with_git(&database, git.git.clone()).unwrap();
    let selection = reopened
        .register_repository("main", &git.path)
        .await
        .unwrap()
        .active_selection
        .unwrap();
    let scope = ActionScope {
        repo_id: selection.repo_id,
        worktree_id: selection.worktree_id,
        generation: "1".into(),
    };
    let interrupted = reopened
        .get_stash_action_outcome(
            "main",
            GetStashActionOutcomeRequest {
                operation_id: op.id.clone(),
            },
        )
        .unwrap();
    assert_eq!(interrupted.operation.state, OperationState::Interrupted);
    assert_effects(&interrupted, ActionEffects::Unknown);
    assert_eq!(
        interrupted.result.unwrap().recovery_ref_retention,
        StashRetention::Unknown
    );
    assert_eq!(git.oid(&pin), stash_oid);
    assert_eq!(git.oid("refs/stash"), stash_oid);
    assert_eq!(
        std::fs::read(git.path.join("alpha.txt")).unwrap(),
        b"base\n"
    );
    assert_eq!(
        std::fs::read(git.path.join("new_edit.txt")).unwrap(),
        b"after fault user edit\n"
    );
    reopened
        .get_repository_stashes(
            "main",
            GetRepositoryStashesRequest {
                scope: scope.clone(),
            },
        )
        .await
        .unwrap();
    let reconciled = reopened
        .reconcile_stash_action(
            "main",
            ReconcileStashActionRequest {
                scope: scope.clone(),
                operation_id: op.id.clone(),
            },
        )
        .await
        .unwrap();
    assert_effects(&reconciled, ActionEffects::Unknown);
    assert_eq!(
        reconciled.result.unwrap().recovery_ref_retention,
        if owned {
            StashRetention::Present
        } else {
            StashRetention::Unknown
        }
    );
    assert_eq!(git.oid(&pin), stash_oid);
}
