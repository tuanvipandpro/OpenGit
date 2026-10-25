#[allow(dead_code)]
mod support;
use opengit_lib::{
    actions::{
        self,
        journal::{self, WritePhase},
    },
    backend::Backend,
    domain::contracts::*,
    git::runner::{argv, RunOptions},
    persistence::workspace::Database,
};
use std::sync::Arc;
async fn setup(
    owner: &support::OwnedFixture,
) -> (Arc<Backend>, support::repos::GitFixtures, ActionScope) {
    let f = support::repos::GitFixtures::create(owner).await.unwrap();
    let b = Arc::new(Backend::with_git(&owner.child("data").unwrap(), f.runner.clone()).unwrap());
    let s = b
        .register_repository("main", &f.working)
        .await
        .unwrap()
        .active_selection
        .unwrap();
    (
        b,
        f,
        ActionScope {
            repo_id: s.repo_id,
            worktree_id: s.worktree_id,
            generation: "1".into(),
        },
    )
}
async fn execution(b: &Backend, s: ActionScope) -> actions::Execution {
    let plan = b
        .prepare_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: s,
                intent: ActionIntent::Commit {
                    message: "MESSAGE_CANARY".into(),
                    amend: false,
                },
            },
        )
        .await
        .unwrap();
    b.consume_action(
        "main",
        ExecuteRepositoryActionRequest {
            plan_id: plan.plan_id,
        },
    )
    .unwrap()
}
fn fault(b: &Backend) {
    b.registry.database.lock().unwrap().execute_batch("CREATE TRIGGER refuse_action BEFORE UPDATE ON repository_actions BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
}
#[tokio::test]
async fn database_failure_before_write_prevents_git_and_restart_never_retries() {
    let owner = support::OwnedFixture::new().unwrap();
    let (b, f, s) = setup(&owner).await;
    let e = execution(&b, s).await;
    let id = e.operation.id.clone();
    fault(&b);
    let result = b
        .run_action("main", e, |ctx| async move {
            ctx.before_write().await?;
            panic!("DB failure allowed a write")
        })
        .await
        .unwrap();
    assert_eq!(result.outcome.unwrap().effects, ActionEffects::NotApplied);
    assert_eq!(
        journal::read(&b.registry.database, &id).unwrap().phase,
        WritePhase::Prepared
    );
    b.registry
        .database
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_action")
        .unwrap();
    drop(b);
    let b = Backend::with_git(&owner.child("data").unwrap(), f.runner).unwrap();
    let result = b.scheduler.get(&id).unwrap();
    assert_eq!(result.state, OperationState::Interrupted);
    assert_eq!(result.outcome.unwrap().effects, ActionEffects::NotApplied);
}
#[tokio::test]
async fn database_failure_after_git_effect_retains_unknown_and_recovery_evidence_across_forget_restart(
) {
    let owner = support::OwnedFixture::new().unwrap();
    let (b, f, s) = setup(&owner).await;
    let e = execution(&b, s.clone()).await;
    let id = e.operation.id.clone();
    let change = b.clone();
    let result = b
        .run_action("main", e, move |ctx| async move {
            assert!(ctx.before_write().await?);
            ctx.run(
                &argv(&["update-ref", "refs/heads/recovery-proof", "HEAD"]),
                Phase::Publish,
                RunOptions::default(),
            )
            .await?;
            fault(&change);
            Ok(actions::completed(
                ActionEffects::Applied,
                Some("a".repeat(64)),
            ))
        })
        .await
        .unwrap();
    assert_eq!(result.state, OperationState::Interrupted);
    assert_eq!(result.outcome.unwrap().effects, ActionEffects::Unknown);
    assert_eq!(
        journal::read(&b.registry.database, &id).unwrap().phase,
        WritePhase::WriteStarted
    );
    b.registry
        .database
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_action")
        .unwrap();
    b.forget_repository("main", ForgetRepositoryRequest { repo_id: s.repo_id })
        .unwrap();
    drop(b);
    let b = Backend::with_git(&owner.child("data").unwrap(), f.runner.clone()).unwrap();
    let result = b.scheduler.get(&id).unwrap();
    assert_eq!(result.outcome.unwrap().effects, ActionEffects::Unknown);
    assert_eq!(result.state, OperationState::Interrupted);
    let record = journal::read(&b.registry.database, &id).unwrap();
    assert!(!record.location.common.is_empty());
    assert!(record.artifacts.is_empty());
    let verified = actions::recovery::verify_location(&record).unwrap();
    drop(verified);
    let mut wrong = record.clone();
    wrong.location.git_identity = vec![255; 12];
    assert_eq!(
        actions::recovery::verify_location(&wrong)
            .err()
            .unwrap()
            .code,
        ErrorCode::ExternalChange
    );
    assert_eq!(
        journal::read(&b.registry.database, &id)
            .unwrap()
            .precondition,
        record.precondition
    );
    f.runner
        .run(
            Some(&f.working),
            &argv(&["show-ref", "--verify", "refs/heads/recovery-proof"]),
            &Default::default(),
            Phase::Verification,
        )
        .await
        .unwrap();
}
#[tokio::test]
async fn dropping_queued_context_or_aborting_after_write_never_auto_resumes_or_claims_no_effects() {
    let owner = support::OwnedFixture::new().unwrap();
    let (b, _, s) = setup(&owner).await;
    let e = execution(&b, s.clone()).await;
    let id = e.operation.id.clone();
    drop(e);
    assert_eq!(
        b.scheduler.get(&id).unwrap().outcome.unwrap().effects,
        ActionEffects::NotApplied
    );
    let e = execution(&b, s).await;
    let id = e.operation.id.clone();
    let running = b.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        running
            .run_action("main", e, |ctx| async move {
                assert!(ctx.before_write().await?);
                tx.send(()).unwrap();
                std::future::pending().await
            })
            .await
    });
    rx.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let result = b.scheduler.get(&id).unwrap();
    assert_eq!(result.state, OperationState::Interrupted);
    assert_eq!(result.outcome.unwrap().effects, ActionEffects::Unknown);
}
#[test]
fn upgrades_zero_through_three_preserve_data_and_action_table_fault_rolls_back() {
    for version in 0..=3 {
        for fail in [false, true] {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("workspace.sqlite3");
            let mut c = rusqlite::Connection::open(&path).unwrap();
            if version >= 1 {
                c.execute_batch(include_str!("../migrations/0001_initial.sql"))
                    .unwrap();
                c.execute_batch("INSERT INTO workspace(id,revision) VALUES('sentinel',9); INSERT INTO operations VALUES('op','operation-sentinel');").unwrap();
            }
            if version >= 2 {
                c.execute_batch(include_str!("../migrations/0002_clone_intents.sql"))
                    .unwrap();
                c.execute_batch("INSERT INTO clone_intents VALUES('op','clone-sentinel')")
                    .unwrap();
            }
            if version >= 3 {
                c.execute_batch(include_str!("../migrations/0003_repository_views.sql"))
                    .unwrap();
            }
            if fail {
                c.execute_batch("CREATE TABLE repository_actions(sentinel TEXT); INSERT INTO repository_actions VALUES('preserve');").unwrap();
            }
            c.pragma_update(None, "user_version", version).unwrap();
            let result = opengit_lib::persistence::migrations::migrate(&mut c);
            assert_eq!(result.is_err(), fail);
            assert_eq!(
                c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                if fail { version } else { 6 }
            );
            if version >= 1 {
                assert_eq!(
                    c.query_row("SELECT snapshot FROM operations", [], |r| r
                        .get::<_, String>(0))
                        .unwrap(),
                    "operation-sentinel"
                );
                assert_eq!(
                    c.query_row("SELECT revision FROM workspace", [], |r| r.get::<_, i64>(0))
                        .unwrap(),
                    9
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
            if fail {
                assert_eq!(
                    c.query_row("SELECT sentinel FROM repository_actions", [], |r| r
                        .get::<_, String>(0))
                        .unwrap(),
                    "preserve"
                );
            }
        }
    }
    let _ = std::mem::size_of::<Database>();
}
