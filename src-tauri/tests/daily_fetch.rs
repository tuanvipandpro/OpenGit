#[allow(dead_code)]
mod support;
use opengit_lib::domain::contracts::*;
#[tokio::test]
async fn actual_fetch_reports_only_covered_refs_overrides_prune_and_preserves_worktree() {
    for (format, storage) in [("sha1", "files"), ("sha256", "reftable")] {
        let f = support::daily::Fixture::new(format, 12, storage).await;
        let before = f.worktree_state();
        let fetch_head = f.git.path.join(".git/FETCH_HEAD");
        std::fs::write(&fetch_head, b"owned FETCH_HEAD sentinel").unwrap();
        f.git.run(&["config", "fetch.prune", "true"], None);
        f.git.run(&["config", "remote.origin.prune", "true"], None);
        f.git.run(&["config", "fetch.pruneTags", "true"], None);
        f.git
            .run(&["config", "fetch.writeCommitGraph", "true"], None);
        let new = f.advance("refs/heads/main");
        let (plan, result) = f.fetch(&["origin"], false).await;
        assert_eq!(
            result.operation.state,
            OperationState::Succeeded,
            "{:?}",
            result.operation.error
        );
        assert_eq!(
            result.operation.outcome.as_ref().unwrap().effects,
            ActionEffects::Applied
        );
        let ledger = result.fetch.unwrap();
        assert_eq!(ledger.remotes[0].effects, ActionEffects::Applied);
        assert!(ledger.remotes[0]
            .reported_refs
            .iter()
            .any(|r| r.name == "refs/remotes/origin/main"
                && r.new_oid.as_deref() == Some(new.as_str())));
        assert_eq!(f.git.oid("refs/remotes/origin/main"), new);
        assert_eq!(f.worktree_state(), before);
        assert_eq!(
            std::fs::read(&fetch_head).unwrap(),
            b"owned FETCH_HEAD sentinel"
        );
        let replay = f
            .backend
            .execute_repository_action(
                "main",
                ExecuteRepositoryActionRequest {
                    plan_id: plan.plan_id,
                },
            )
            .unwrap();
        assert_eq!(replay.id, result.operation.id);
        let catalog = f.catalog().await;
        let first_time = catalog
            .branches
            .iter()
            .find(|b| b.name == "refs/remotes/origin/orphan")
            .unwrap()
            .observation
            .as_ref()
            .unwrap()
            .observed_at
            .clone();
        f.git
            .run_at(&f.remote, &["update-ref", "-d", "refs/heads/orphan"], None);
        let (_, result) = f.fetch(&["origin"], false).await;
        assert_eq!(result.operation.state, OperationState::Succeeded);
        assert!(!result.fetch.unwrap().remotes[0]
            .reported_refs
            .iter()
            .any(|r| r.name == "refs/remotes/origin/orphan"));
        let catalog = f.catalog().await;
        let orphan = catalog
            .branches
            .iter()
            .find(|b| b.name == "refs/remotes/origin/orphan")
            .unwrap();
        assert_eq!(orphan.observation.as_ref().unwrap().observed_at, first_time);
        f.git.run(
            &[
                "config",
                "--replace-all",
                "remote.origin.fetch",
                "+refs/heads/main:refs/remotes/origin/main",
            ],
            None,
        );
        assert!(f
            .catalog()
            .await
            .branches
            .iter()
            .all(|b| b.observation.is_none()));
        let (_, narrow) = f.fetch(&["origin"], false).await;
        let rows = &narrow.fetch.unwrap().remotes[0].reported_refs;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, FetchRefStatus::UpToDate);
        assert!(f
            .catalog()
            .await
            .branches
            .iter()
            .find(|b| b.name == "refs/remotes/origin/orphan")
            .unwrap()
            .observation
            .is_none());
        f.git.run(
            &[
                "config",
                "--replace-all",
                "remote.origin.fetch",
                "+refs/heads/*:refs/remotes/origin/*",
            ],
            None,
        );
        let (_, pruned) = f.fetch(&["origin"], true).await;
        assert_eq!(
            pruned.operation.state,
            OperationState::Succeeded,
            "{:?}",
            pruned.operation.error
        );
        assert!(
            pruned.fetch.unwrap().remotes[0]
                .reported_refs
                .iter()
                .any(|r| r.name == "refs/remotes/origin/orphan"
                    && r.status == FetchRefStatus::Deleted)
        );
        assert!(!f
            .catalog()
            .await
            .branches
            .iter()
            .any(|b| b.name == "refs/remotes/origin/orphan"));
        assert_eq!(f.worktree_state(), before);
        assert_eq!(
            std::fs::read(fetch_head).unwrap(),
            b"owned FETCH_HEAD sentinel"
        );
        assert!(!f.git.path.join(".git/objects/info/commit-graph").exists());
    }
}
#[tokio::test]
async fn fetch_all_records_partial_failure_and_never_calls_it_cancelled() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    let missing = f.git.owner.child("missing remote.git").unwrap();
    let value = missing.to_string_lossy().replace('\\', "/");
    f.git.run(&["remote", "add", "zbad", &value], None);
    let before = f.worktree_state();
    let new = f.advance("refs/heads/main");
    let (_, result) = f.fetch(&["origin", "zbad"], false).await;
    assert_eq!(
        result.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::Partial
    );
    assert_ne!(result.operation.state, OperationState::Cancelled);
    let ledger = result.fetch.unwrap();
    assert_eq!(ledger.remotes[0].effects, ActionEffects::Applied);
    assert_eq!(ledger.remotes[1].effects, ActionEffects::Unknown);
    let record: String = f
        .backend
        .registry
        .database
        .lock()
        .unwrap()
        .query_row(
            "SELECT record FROM fetch_actions WHERE operation_id=?1",
            [&result.operation.id],
            |r| r.get(0),
        )
        .unwrap();
    let record: serde_json::Value = serde_json::from_str(&record).unwrap();
    let first = record["proofs"][0]["refsDigest"].as_str().unwrap();
    let second = record["proofs"][1]["refsDigest"].as_str().unwrap();
    assert_eq!(first.len(), 64);
    assert_eq!(second.len(), 64);
    assert_ne!(
        first, second,
        "second write proof must include the first verified fetch effects"
    );
    assert!(opengit_lib::actions::journal::read(
        &f.backend.registry.database,
        &result.operation.id
    )
    .unwrap()
    .postcondition
    .is_none());
    assert!(ledger.remotes[1].reported_refs.is_empty());
    assert!(ledger.remotes[1].error.is_some());
    assert_eq!(f.git.oid("refs/remotes/origin/main"), new);
    assert_eq!(f.worktree_state(), before);
}
#[tokio::test]
async fn unsafe_custom_refspecs_and_future_intents_have_no_mutation_authority() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    let before = f.git.snapshot();
    for mapping in [
        "+refs/*:refs/*",
        "+refs/heads/*:refs/heads/*",
        "+refs/tags/*:refs/tags/*",
    ] {
        f.git.run(
            &["config", "--replace-all", "remote.origin.fetch", mapping],
            None,
        );
        let catalog = f.catalog().await;
        assert_eq!(
            catalog.remotes[0].fetch,
            FetchCapability::UnsupportedRefspec
        );
        assert!(catalog.remotes[0].handle.is_none());
    }
    let error = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::Commit {
                    message: "not exposed".into(),
                    amend: false,
                },
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnsupportedAction);
    assert!(f
        .backend
        .get_operations("main")
        .unwrap()
        .operations
        .is_empty());
    assert_eq!(f.git.snapshot(), before);
}
#[tokio::test]
async fn capability_prepare_is_entirely_readonly_for_files_reftable_unborn_and_bare() {
    for (format, storage) in [("sha1", "files"), ("sha256", "reftable")] {
        let f = support::daily::Fixture::new(format, 8, storage).await;
        let before = support::daily::metadata_tree(&f.git.path);
        f.prepare_fetch(&["origin"], false).await;
        assert_eq!(support::daily::metadata_tree(&f.git.path), before);
        for bare in [false, true] {
            let path = f
                .git
                .owner
                .child(if bare {
                    "empty-bare.git"
                } else {
                    "unborn Ω = '"
                })
                .unwrap();
            std::fs::create_dir(&path).unwrap();
            let mut args = vec!["init", "-b", "main"];
            let object = format!("--object-format={format}");
            let refs = format!("--ref-format={storage}");
            args.extend([object.as_str(), refs.as_str()]);
            if bare {
                args.push("--bare");
            }
            f.git.run_at(&path, &args, None);
            f.git.run_at(
                &path,
                &["remote", "add", "origin", &f.git.git_path(&f.remote)],
                None,
            );
            let (backend, scope) = f.git.backend_at(&path, "main").await;
            let scope = ActionScope {
                repo_id: scope.repo_id,
                worktree_id: scope.worktree_id,
                generation: scope.generation,
            };
            let catalog = backend
                .get_repository_actions(
                    "main",
                    GetRepositoryActionsRequest {
                        scope: scope.clone(),
                    },
                )
                .await
                .unwrap();
            assert_eq!(catalog.head.kind, HeadKind::Unborn);
            let before = support::daily::metadata_tree(&path);
            backend
                .prepare_repository_action(
                    "main",
                    PrepareRepositoryActionRequest {
                        scope,
                        intent: ActionIntent::Fetch {
                            remote_handles: vec![catalog.remotes[0].handle.clone().unwrap()],
                            prune: false,
                        },
                    },
                )
                .await
                .unwrap();
            assert_eq!(support::daily::metadata_tree(&path), before);
        }
    }
}
#[tokio::test]
async fn old_porcelain_and_malformed_capabilities_fail_before_enqueue_without_breaking_inspection()
{
    let mut f = support::daily::Fixture::new("sha1", 8, "files").await;
    let proxy = support::daily::Proxy::new(&f.git);
    f.rebind(proxy.runner.clone(), None).await;
    for (mode, code) in [
        ("old", ErrorCode::UnsupportedGit),
        ("malformed", ErrorCode::MalformedOutput),
        ("permission", ErrorCode::PermissionDenied),
    ] {
        proxy.set(mode);
        let catalog = f.catalog().await;
        let before = f.worktree_state();
        let error = f
            .backend
            .prepare_repository_action(
                "main",
                PrepareRepositoryActionRequest {
                    scope: f.scope.clone(),
                    intent: ActionIntent::Fetch {
                        remote_handles: vec![catalog.remotes[0].handle.clone().unwrap()],
                        prune: false,
                    },
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, code, "{mode}");
        assert!(!error.message.contains("CANARY"));
        if mode == "old" {
            assert!(error.message.contains("Update Git"));
        }
        let inspection = f
            .backend
            .inspect_repository(
                "main",
                InspectRepositoryRequest {
                    repo_id: f.scope.repo_id.clone(),
                    worktree_id: f.scope.worktree_id.clone(),
                    generation: "1".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(inspection.head.kind, HeadKind::Branch);
        assert!(f
            .backend
            .get_operations("main")
            .unwrap()
            .operations
            .is_empty());
        assert_eq!(f.worktree_state(), before);
    }
}
#[tokio::test]
async fn auth_output_and_timeout_failures_keep_truthful_unknown_evidence_and_no_secret_text() {
    let mut f = support::daily::Fixture::new("sha1", 8, "files").await;
    let proxy = support::daily::Proxy::new(&f.git);
    for (mode, code) in [
        ("auth", ErrorCode::AuthenticationFailed),
        ("output", ErrorCode::OutputLimit),
        ("hold", ErrorCode::Timeout),
    ] {
        f.rebind(
            proxy.runner.clone(),
            Some(if mode == "hold" {
                std::time::Duration::from_millis(200)
            } else {
                std::time::Duration::from_secs(5)
            }),
        )
        .await;
        proxy.set(mode);
        let before = f.worktree_state();
        let (_, result) = f.fetch(&["origin"], false).await;
        assert_eq!(
            result.operation.outcome.as_ref().unwrap().effects,
            ActionEffects::Unknown,
            "{mode}"
        );
        assert_ne!(result.operation.state, OperationState::Cancelled);
        let error = result.fetch.as_ref().unwrap().remotes[0]
            .error
            .as_ref()
            .unwrap();
        assert_eq!(error.code, code, "{mode}");
        assert!(!serde_json::to_string(&result).unwrap().contains("CANARY"));
        let connection = f.backend.registry.database.lock().unwrap();
        let raw: String = connection
            .query_row(
                "SELECT record FROM fetch_actions WHERE operation_id=?1",
                [&result.operation.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!raw.contains("CANARY"));
        assert!(!raw.contains("opengit-captured"));
        drop(connection);
        assert!(opengit_lib::actions::journal::read(
            &f.backend.registry.database,
            &result.operation.id
        )
        .unwrap()
        .postcondition
        .is_none());
        assert_eq!(f.worktree_state(), before);
    }
}
#[tokio::test]
async fn cancellation_during_fetch_is_unknown_but_after_ack_preserves_applied() {
    for mode in ["hold", "verifyhold"] {
        let mut f = support::daily::Fixture::new("sha1", 8, "files").await;
        let proxy = support::daily::Proxy::new(&f.git);
        f.rebind(proxy.runner.clone(), None).await;
        let before = f.worktree_state();
        let new = f.advance("refs/heads/main");
        let plan = f.prepare_fetch(&["origin"], false).await;
        proxy.set(mode);
        let operation = f
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
                    operation_id: operation.id.clone(),
                },
            )
            .unwrap();
        proxy.release();
        let result = f.wait(&operation.id).await;
        let outcome = result.operation.outcome.unwrap();
        assert!(outcome.cancellation_requested);
        if mode == "hold" {
            assert_eq!(outcome.effects, ActionEffects::Unknown);
            assert_ne!(result.operation.state, OperationState::Cancelled);
        } else {
            assert_eq!(outcome.effects, ActionEffects::Applied);
            assert_eq!(result.operation.state, OperationState::Succeeded);
            assert_eq!(f.git.oid("refs/remotes/origin/main"), new);
        }
        assert_eq!(f.worktree_state(), before);
    }
}
#[tokio::test]
async fn journal_failure_before_write_prevents_fetch_and_after_ack_survives_restart_without_retry()
{
    for after in [false, true] {
        let mut f = support::daily::Fixture::new("sha1", 8, "files").await;
        let proxy = support::daily::Proxy::new(&f.git);
        f.rebind(proxy.runner.clone(), None).await;
        let before = f.git.snapshot();
        let new = f.advance("refs/heads/main");
        let plan = f.prepare_fetch(&["origin"], false).await;
        if after {
            proxy.set("verifyhold");
        } else {
            f.backend.registry.database.lock().unwrap().execute_batch("CREATE TRIGGER refuse_fetch_insert BEFORE INSERT ON fetch_actions BEGIN SELECT RAISE(ABORT,'owned database fault'); END;").unwrap();
        }
        if !after {
            let error = f
                .backend
                .execute_repository_action(
                    "main",
                    ExecuteRepositoryActionRequest {
                        plan_id: plan.plan_id.clone(),
                    },
                )
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::Database);
            assert!(f
                .backend
                .get_operations("main")
                .unwrap()
                .operations
                .is_empty());
            assert_eq!(
                f.backend
                    .registry
                    .database
                    .lock()
                    .unwrap()
                    .query_row("SELECT COUNT(*) FROM fetch_actions", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(f.git.snapshot(), before);
            assert!(!proxy.fetched.exists());
            f.backend
                .registry
                .database
                .lock()
                .unwrap()
                .execute_batch("DROP TRIGGER refuse_fetch_insert;")
                .unwrap();
            let retry = f
                .backend
                .execute_repository_action(
                    "main",
                    ExecuteRepositoryActionRequest {
                        plan_id: plan.plan_id,
                    },
                )
                .unwrap();
            assert_eq!(
                f.wait(&retry.id).await.operation.state,
                OperationState::Succeeded
            );
            continue;
        }
        let operation = f
            .backend
            .execute_repository_action(
                "main",
                ExecuteRepositoryActionRequest {
                    plan_id: plan.plan_id,
                },
            )
            .unwrap();
        if after {
            proxy.wait_ready().await;
            f.backend.registry.database.lock().unwrap().execute_batch("CREATE TRIGGER refuse_fetch_observation BEFORE UPDATE ON fetch_actions WHEN instr(NEW.record,'\"phase\":\"observed\"')>0 BEGIN SELECT RAISE(ABORT,'owned database fault'); END;").unwrap();
            proxy.release();
        }
        let result = f.wait(&operation.id).await;
        assert_eq!(
            result.operation.outcome.as_ref().unwrap().effects,
            ActionEffects::Unknown
        );
        assert_eq!(f.git.oid("refs/remotes/origin/main"), new);
        let path: String = f
            .backend
            .registry
            .database
            .lock()
            .unwrap()
            .query_row("PRAGMA database_list", [], |r| r.get(2))
            .unwrap();
        let directory = std::path::PathBuf::from(path)
            .parent()
            .unwrap()
            .to_path_buf();
        let completed = std::fs::read(&proxy.fetched).unwrap();
        while std::sync::Arc::strong_count(&f.backend) > 1 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let support::daily::Fixture { backend, git, .. } = f;
        drop(backend);
        let reopened =
            opengit_lib::backend::Backend::with_git(&directory, proxy.runner.clone()).unwrap();
        reopened.get_workspace("main").unwrap();
        let result = reopened
            .get_action_outcome(
                "main",
                GetActionOutcomeRequest {
                    operation_id: operation.id,
                },
            )
            .unwrap();
        assert_eq!(
            result.operation.outcome.unwrap().effects,
            ActionEffects::Unknown
        );
        assert_eq!(
            result.fetch.unwrap().remotes[0].phase,
            FetchStepPhase::Started
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(std::fs::read(proxy.fetched).unwrap(), completed);
        assert_eq!(git.oid("refs/remotes/origin/main"), new);
    }
}
#[test]
fn migration_five_preserves_zero_through_four_and_rolls_back_every_new_table_on_failure() {
    for version in 0..=4 {
        for fail in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut c =
                rusqlite::Connection::open(directory.path().join("workspace.sqlite3")).unwrap();
            if version >= 1 {
                c.execute_batch(include_str!("../migrations/0001_initial.sql"))
                    .unwrap();
                c.execute_batch("INSERT INTO workspace(id,revision) VALUES('workspace-sentinel',13); INSERT INTO operations VALUES('op','operation-sentinel');").unwrap();
            }
            if version >= 2 {
                c.execute_batch(include_str!("../migrations/0002_clone_intents.sql"))
                    .unwrap();
                c.execute_batch("INSERT INTO clone_intents VALUES('op','clone-sentinel');")
                    .unwrap();
            }
            if version >= 3 {
                c.execute_batch(include_str!("../migrations/0003_repository_views.sql"))
                    .unwrap();
            }
            if version >= 4 {
                c.execute_batch(include_str!("../migrations/0004_repository_actions.sql"))
                    .unwrap();
                c.execute_batch("INSERT INTO repository_actions VALUES('op','action-sentinel');")
                    .unwrap();
            }
            if fail {
                c.execute_batch("CREATE TABLE fetch_observations(sentinel TEXT); INSERT INTO fetch_observations VALUES('preserve');").unwrap();
            }
            c.pragma_update(None, "user_version", version).unwrap();
            assert_eq!(
                opengit_lib::persistence::migrations::migrate(&mut c).is_err(),
                fail
            );
            assert_eq!(
                c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                if fail { version } else { 5 }
            );
            if version >= 1 {
                assert_eq!(
                    c.query_row("SELECT revision FROM workspace", [], |r| r.get::<_, i64>(0))
                        .unwrap(),
                    13
                );
                assert_eq!(
                    c.query_row("SELECT snapshot FROM operations", [], |r| r
                        .get::<_, String>(0))
                        .unwrap(),
                    "operation-sentinel"
                );
            }
            if version >= 2 {
                assert_eq!(
                    c.query_row("SELECT context FROM clone_intents", [], |r| r
                        .get::<_, String>(0))
                        .unwrap(),
                    "clone-sentinel"
                );
            }
            if version >= 4 {
                assert_eq!(
                    c.query_row("SELECT record FROM repository_actions", [], |r| r
                        .get::<_, String>(0))
                        .unwrap(),
                    "action-sentinel"
                );
            }
            if fail {
                assert_eq!(
                    c.query_row("SELECT sentinel FROM fetch_observations", [], |r| r
                        .get::<_, String>(0))
                        .unwrap(),
                    "preserve"
                );
                assert_eq!(
                    c.query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE name='fetch_actions'",
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
#[tokio::test]
async fn measured_small_fixture_phases_keep_the_same_fetch_guards() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    f.catalog().await;
    let clock = std::time::Instant::now();
    let catalog = f.catalog().await;
    let catalog_ms = clock.elapsed().as_millis();
    let clock = std::time::Instant::now();
    let plan = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::Fetch {
                    remote_handles: vec![catalog.remotes[0].handle.clone().unwrap()],
                    prune: false,
                },
            },
        )
        .await
        .unwrap();
    let prepare_ms = clock.elapsed().as_millis();
    let clock = std::time::Instant::now();
    let operation = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    let result = f.wait(&operation.id).await;
    let execute_ms = clock.elapsed().as_millis();
    assert_eq!(result.operation.state, OperationState::Succeeded);
    println!("S2 small fixture warmed_catalog_ms={catalog_ms} prepare_ms={prepare_ms} execute_wait_ms={execute_ms}; native command count unavailable");
}
#[tokio::test]
async fn protected_prepare_never_triggers_promisor_activity_or_metadata_writes() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    let canary = support::daily::Proxy::new(&f.git);
    canary.set("canary");
    let tree = f.git.oid("main^{tree}");
    let parent = f.git.oid("main");
    let raw=format!("tree {tree}\nparent {parent}\nauthor Fixture <fixture@example.invalid> 1700000000 +0000\ncommitter Fixture <fixture@example.invalid> 1700000000 +0000\n\nMissing promisor fixture\n");
    let missing = String::from_utf8(f.git.run(
        &["hash-object", "-t", "commit", "-w", "--stdin"],
        Some(raw.as_bytes()),
    ))
    .unwrap()
    .trim()
    .to_owned();
    f.git
        .run(&["update-ref", "refs/heads/missing-object", &missing], None);
    let object = f
        .git
        .path
        .join(".git/objects")
        .join(&missing[..2])
        .join(&missing[2..]);
    f.git.owner.require_owned_file(&object).unwrap();
    let bytes = std::fs::read(&object).unwrap();
    std::fs::remove_file(&object).unwrap();
    f.git.run(
        &["remote", "add", "promisor", &f.git.git_path(&f.remote)],
        None,
    );
    f.git
        .run(&["config", "core.repositoryFormatVersion", "1"], None);
    f.git
        .run(&["config", "extensions.partialClone", "promisor"], None);
    f.git
        .run(&["config", "remote.promisor.promisor", "true"], None);
    let helper = format!("\"{}\"", f.git.git_path(canary.runner.executable()));
    f.git
        .run(&["config", "remote.promisor.uploadpack", &helper], None);
    let control = f
        .git
        .command(&f.git.path)
        .env_remove("GIT_NO_LAZY_FETCH")
        .env("GIT_ALLOW_PROTOCOL", "file")
        .args(["cat-file", "-t", &missing])
        .output()
        .unwrap();
    assert!(!control.status.success());
    assert!(
        canary.ready.exists(),
        "negative control must really attempt a promisor fetch"
    );
    f.git.owner.require_owned_file(&canary.ready).unwrap();
    std::fs::remove_file(&canary.ready).unwrap();
    let catalog = f.catalog().await;
    assert_eq!(catalog.sync.completeness, AncestryCompleteness::Unavailable);
    let before = support::daily::metadata_tree(&f.git.path);
    let result = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::Fetch {
                    remote_handles: vec![catalog
                        .remotes
                        .iter()
                        .find(|r| r.name == "origin")
                        .unwrap()
                        .handle
                        .clone()
                        .unwrap()],
                    prune: false,
                },
            },
        )
        .await;
    if let Err(error) = result {
        assert_ne!(error.code, ErrorCode::UnsupportedGit);
    }
    assert!(
        !canary.ready.exists(),
        "protected prepare must not run any promisor upload-pack"
    );
    assert_eq!(support::daily::metadata_tree(&f.git.path), before);
    assert!(f
        .backend
        .get_operations("main")
        .unwrap()
        .operations
        .is_empty());
    std::fs::write(object, bytes).unwrap();
}
#[tokio::test]
async fn explicit_branch_prune_never_prunes_local_tags_from_ambient_prune_tags_settings() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    f.git
        .run(&["update-ref", "refs/tags/local-only-canary", "main"], None);
    f.git
        .run_at(&f.remote, &["update-ref", "-d", "refs/tags/v1"], None);
    f.git.run(&["config", "fetch.pruneTags", "true"], None);
    f.git
        .run(&["config", "remote.origin.pruneTags", "true"], None);
    let tags = f.git.run(
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/tags/",
        ],
        None,
    );
    let (_, result) = f.fetch(&["origin"], true).await;
    assert_eq!(result.operation.state, OperationState::Succeeded);
    assert_eq!(
        f.git.run(
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/tags/"
            ],
            None
        ),
        tags
    );
}

#[tokio::test]
async fn accepted_fetch_all_survives_repeated_same_selection_refresh_and_replay_without_deadline_rebind(
) {
    let mut f = support::daily::Fixture::new("sha1", 8, "files").await;
    f.git.run(
        &["remote", "add", "second", &f.git.git_path(&f.remote)],
        None,
    );
    let expected = f.advance("refs/heads/main");
    let proxy = support::daily::Proxy::new(&f.git);
    f.rebind(proxy.runner.clone(), None).await;
    let plan = f.prepare_fetch(&["origin", "second"], false).await;
    let mut scope = f.scope.clone();
    let mut epoch = f
        .backend
        .actions
        .authority
        .begin("main", &scope, || Ok(()))
        .unwrap();
    proxy.set("verifyhold");
    let operation = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id.clone(),
            },
        )
        .unwrap();
    proxy.wait_ready().await;
    assert_eq!(f.git.oid("refs/remotes/origin/main"), expected);
    let mut reads = Vec::new();
    for generation in 2..=4 {
        let prior = scope.clone();
        scope.generation = generation.to_string();
        let backend = f.backend.clone();
        let next = scope.clone();
        reads.push(tokio::spawn(async move {
            backend
                .get_repository_actions("main", GetRepositoryActionsRequest { scope: next })
                .await
        }));
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while f
                .backend
                .actions
                .authority
                .checked("main", &prior, epoch, || Ok(()))
                .is_ok()
            {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        epoch = f
            .backend
            .actions
            .authority
            .begin("main", &scope, || Ok(()))
            .unwrap();
        let replay = f
            .backend
            .execute_repository_action(
                "main",
                ExecuteRepositoryActionRequest {
                    plan_id: plan.plan_id.clone(),
                },
            )
            .unwrap();
        assert_eq!(replay.id, operation.id);
        assert_eq!(replay.created_at, operation.created_at);
    }
    proxy.release();
    let result = f.wait(&operation.id).await;
    for read in reads {
        let _ = read.await.unwrap();
    }
    assert_eq!(
        result.operation.state,
        OperationState::Succeeded,
        "{result:?}"
    );
    assert_eq!(
        result.operation.outcome.unwrap().effects,
        ActionEffects::Applied
    );
    let remotes = result.fetch.unwrap().remotes;
    assert_eq!(remotes.len(), 2);
    assert!(remotes
        .iter()
        .all(|r| r.phase == FetchStepPhase::Observed && r.effects == ActionEffects::Applied));
    assert_eq!(f.git.oid("refs/remotes/second/main"), expected);
    assert_eq!(
        std::fs::read_to_string(&proxy.fetched)
            .unwrap()
            .lines()
            .count(),
        2
    );
}
#[tokio::test]
async fn closing_owner_after_first_fetch_effect_keeps_partial_and_stops_remote_two_without_catalog()
{
    let mut f = support::daily::Fixture::new("sha1", 8, "files").await;
    f.git.run(
        &["remote", "add", "second", &f.git.git_path(&f.remote)],
        None,
    );
    let expected = f.advance("refs/heads/main");
    let proxy = support::daily::Proxy::new(&f.git);
    f.rebind(proxy.runner.clone(), None).await;
    let plan = f.prepare_fetch(&["origin", "second"], false).await;
    proxy.set("verifyhold");
    let operation = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id.clone(),
            },
        )
        .unwrap();
    proxy.wait_ready().await;
    assert_eq!(f.git.oid("refs/remotes/origin/main"), expected);
    f.backend.close_window("main");
    proxy.release();
    let snapshot = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let op = f.backend.scheduler.get(&operation.id).unwrap();
            if opengit_lib::operations::state::terminal(&op.state) {
                break op;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_ne!(snapshot.state, OperationState::Cancelled);
    assert!(matches!(
        snapshot.outcome.as_ref().unwrap().effects,
        ActionEffects::Partial | ActionEffects::Unknown
    ));
    assert!(snapshot.outcome.as_ref().unwrap().cancellation_requested);
    assert!(!String::from_utf8(f.git.run(
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/remotes/second/"
        ],
        None
    ))
    .unwrap()
    .contains("refs/remotes/second/"));
    f.backend.get_workspace("main").unwrap();
    let result = f
        .backend
        .get_action_outcome(
            "main",
            GetActionOutcomeRequest {
                operation_id: operation.id,
            },
        )
        .unwrap();
    assert_ne!(
        result.fetch.unwrap().remotes[1].phase,
        FetchStepPhase::Started
    );
    assert_eq!(
        f.backend
            .execute_repository_action(
                "main",
                ExecuteRepositoryActionRequest {
                    plan_id: plan.plan_id
                }
            )
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
}
