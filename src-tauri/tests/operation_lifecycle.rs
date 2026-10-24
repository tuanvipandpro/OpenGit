use opengit_lib::{
    domain::contracts::*,
    operations::scheduler::{Completion, LockKey, OperationSpec, Scheduler},
    persistence::workspace::Database,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
fn spec(reservation: u8) -> OperationSpec {
    OperationSpec {
        source_display: "https://user:CANARY@example.invalid/repo?token=CANARY".into(),
        destination_display: "Selected destination".into(),
        reservation: Some(vec![reservation]),
    }
}
fn make_scheduler(path: &std::path::Path) -> Arc<Scheduler> {
    Arc::new(Scheduler::new(Arc::new(Database::open(path).unwrap())).unwrap())
}
#[tokio::test]
async fn unconfirmed_process_stop_remains_interrupted_when_cancel_already_won() {
    let temp = tempfile::tempdir().unwrap();
    let scheduler = make_scheduler(temp.path());
    let operation = scheduler.enqueue(spec(90)).unwrap();
    let running = scheduler.clone();
    let id = operation.id.clone();
    let snapshot = scheduler
        .execute(&operation.id, vec![], false, move |cancel| async move {
            running.cancel(&id).unwrap();
            assert!(cancel.is_cancelled());
            Err(opengit_lib::git::error::error(
                ErrorCode::Interrupted,
                Phase::Recovery,
            ))
        })
        .await
        .unwrap();
    assert_eq!(snapshot.state, OperationState::Interrupted);
    assert_eq!(snapshot.phase, Phase::Recovery);
    assert_eq!(snapshot.error.unwrap().code, ErrorCode::Interrupted);
    drop(scheduler);
    let reopened = make_scheduler(temp.path());
    assert_eq!(
        reopened.snapshot().unwrap().operations[0].state,
        OperationState::Interrupted
    );
}
#[tokio::test]
async fn real_contention_running_cancel_progress_and_bounded_network() {
    let temp = tempfile::tempdir().unwrap();
    let scheduler = make_scheduler(temp.path());
    for shared in [true, false] {
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let mut jobs = Vec::new();
        for n in 10..14 {
            let operation = scheduler.enqueue(spec(n)).unwrap();
            let s = scheduler.clone();
            let a = active.clone();
            let m = maximum.clone();
            jobs.push(tokio::spawn(async move {
                s.execute(
                    &operation.id,
                    vec![LockKey {
                        common: vec![if shared { 0 } else { n }],
                        worktree: Some(vec![n]),
                    }],
                    true,
                    move |_| async move {
                        let current = a.fetch_add(1, Ordering::SeqCst) + 1;
                        m.fetch_max(current, Ordering::SeqCst);
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        a.fetch_sub(1, Ordering::SeqCst);
                        Ok(Completion::Succeeded)
                    },
                )
                .await
                .unwrap()
            }));
        }
        for job in jobs {
            assert_eq!(job.await.unwrap().state, OperationState::Succeeded);
        }
        assert_eq!(maximum.load(Ordering::SeqCst), if shared { 1 } else { 2 });
    }
    let operation = scheduler.enqueue(spec(30)).unwrap();
    let s = scheduler.clone();
    let id = operation.id.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let job = tokio::spawn(async move {
        s.execute(&id, vec![], false, move |cancel| async move {
            started_tx.send(()).unwrap();
            while !cancel.is_cancelled() {
                tokio::task::yield_now().await;
            }
            Ok(Completion::Cancelled)
        })
        .await
        .unwrap()
    });
    started_rx.await.unwrap();
    let updated = scheduler
        .progress(
            &operation.id,
            Progress {
                completed: 1,
                total: Some(2),
                unit: ProgressUnit::Steps,
            },
            Phase::Transfer,
        )
        .unwrap();
    assert_eq!(updated.progress.unwrap().completed, 1);
    assert_eq!(
        scheduler.cancel(&operation.id).unwrap().state,
        OperationState::Cancelling
    );
    let terminal = job.await.unwrap();
    assert_eq!(terminal.state, OperationState::Cancelled);
    assert_eq!(
        scheduler
            .progress(
                &operation.id,
                Progress {
                    completed: 2,
                    total: Some(2),
                    unit: ProgressUnit::Steps
                },
                Phase::Transfer
            )
            .unwrap(),
        terminal
    );
}
#[test]
fn failed_journal_write_does_not_emit_or_advance_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let scheduler = make_scheduler(temp.path());
    let operation = scheduler.enqueue(spec(70)).unwrap();
    let before = scheduler.snapshot().unwrap();
    let mut events = scheduler.subscribe();
    let connection = rusqlite::Connection::open(temp.path().join("workspace.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_operation BEFORE UPDATE ON operations BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert_eq!(
        scheduler
            .transition(
                &operation.id,
                OperationState::Running,
                Phase::Transfer,
                None
            )
            .unwrap_err()
            .code,
        ErrorCode::Database
    );
    assert_eq!(before, scheduler.snapshot().unwrap());
    assert!(matches!(
        events.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
}
#[tokio::test]
async fn shared_common_lock_serializes_worktrees_and_queued_cancel_never_runs() {
    let temp = tempfile::tempdir().unwrap();
    let scheduler = make_scheduler(temp.path());
    let first = scheduler.enqueue(spec(1)).unwrap();
    let second = scheduler.enqueue(spec(2)).unwrap();
    assert_eq!(
        scheduler.enqueue(spec(2)).unwrap_err().code,
        ErrorCode::DestinationBusy
    );
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let s = scheduler.clone();
    let a = active.clone();
    let m = maximum.clone();
    let first_id = first.id.clone();
    let task1 = tokio::spawn(async move {
        s.execute(
            &first_id,
            vec![LockKey {
                common: vec![1],
                worktree: Some(vec![1]),
            }],
            false,
            move |_| async move {
                m.fetch_max(a.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                started_tx.send(()).unwrap();
                release_rx.await.unwrap();
                a.fetch_sub(1, Ordering::SeqCst);
                Ok(Completion::Succeeded)
            },
        )
        .await
        .unwrap()
    });
    started_rx.await.unwrap();
    let s = scheduler.clone();
    let a = active.clone();
    let second_id = second.id.clone();
    let task2 = tokio::spawn(async move {
        s.execute(
            &second_id,
            vec![LockKey {
                common: vec![1],
                worktree: Some(vec![2]),
            }],
            false,
            move |_| async move {
                a.fetch_add(1, Ordering::SeqCst);
                panic!("queued cancelled job must never run");
                #[allow(unreachable_code)]
                Ok(Completion::Succeeded)
            },
        )
        .await
        .unwrap()
    });
    let cancelled = scheduler.cancel(&second.id).unwrap();
    assert_eq!(cancelled.state, OperationState::Cancelled);
    release_tx.send(()).unwrap();
    assert_eq!(task1.await.unwrap().state, OperationState::Succeeded);
    assert_eq!(task2.await.unwrap().state, OperationState::Cancelled);
    assert_eq!(maximum.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn terminal_race_sequence_fence_restart_and_safe_journal() {
    let temp = tempfile::tempdir().unwrap();
    let scheduler = make_scheduler(temp.path());
    let mut events = scheduler.subscribe();
    let operation = scheduler.enqueue(spec(3)).unwrap();
    let first = events.recv().await.unwrap();
    assert_eq!(first.operation.sequence, first.sequence);
    let snapshot = scheduler.snapshot().unwrap();
    assert_eq!(first.sequence, snapshot.sequence);
    scheduler
        .transition(
            &operation.id,
            OperationState::Running,
            Phase::Transfer,
            None,
        )
        .unwrap();
    scheduler
        .transition(
            &operation.id,
            OperationState::WaitingForUser,
            Phase::Authentication,
            None,
        )
        .unwrap();
    assert_eq!(
        scheduler.cancel(&operation.id).unwrap().state,
        OperationState::Cancelling
    );
    // Publish won the race: cancellation must never relabel success.
    let success = scheduler
        .transition(
            &operation.id,
            OperationState::Succeeded,
            Phase::Registration,
            None,
        )
        .unwrap();
    let again = scheduler.cancel(&operation.id).unwrap();
    assert_eq!(success, again);
    assert_eq!(
        scheduler
            .transition(&operation.id, OperationState::Failed, Phase::Transfer, None)
            .unwrap(),
        success
    );
    let unfinished = scheduler.enqueue(spec(4)).unwrap();
    scheduler
        .transition(
            &unfinished.id,
            OperationState::Running,
            Phase::Transfer,
            None,
        )
        .unwrap();
    let before = scheduler.snapshot().unwrap();
    let mut sequences = vec![first.sequence.parse::<u64>().unwrap()];
    while let Ok(event) = events.try_recv() {
        assert_eq!(event.sequence, event.operation.sequence);
        sequences.push(event.sequence.parse().unwrap());
    }
    assert!(sequences.windows(2).all(|v| v[1] == v[0] + 1));
    assert_eq!(sequences.last().unwrap().to_string(), before.sequence);
    drop(scheduler);
    let reopened = make_scheduler(temp.path());
    let restored = reopened.snapshot().unwrap();
    assert_ne!(restored.stream_id, before.stream_id);
    assert_eq!(
        restored
            .operations
            .iter()
            .find(|o| o.id == unfinished.id)
            .unwrap()
            .state,
        OperationState::Interrupted
    );
    assert_eq!(
        restored
            .operations
            .iter()
            .find(|o| o.id == operation.id)
            .unwrap()
            .state,
        OperationState::Succeeded
    );
    let bytes = std::fs::read(temp.path().join("workspace.sqlite3")).unwrap();
    assert!(!bytes.windows(6).any(|w| w == b"CANARY"));
    assert_eq!(
        reopened.cancel("unknown").unwrap_err().code,
        ErrorCode::InvalidHandle
    );
}
