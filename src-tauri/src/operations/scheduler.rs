use super::{journal, state};
use crate::{
    domain::contracts::*,
    git::{error::error, refs::safe_remote, runner::Cancellation},
    persistence::workspace::Database,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{broadcast, Mutex as AsyncMutex, Semaphore};
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LockKey {
    pub common: Vec<u8>,
    pub worktree: Option<Vec<u8>>,
}
struct Entry {
    snapshot: OperationSnapshot,
    cancel: Cancellation,
}
struct Inner {
    stream: String,
    sequence: u64,
    entries: BTreeMap<String, Entry>,
    reservations: HashMap<Vec<u8>, String>,
}
pub struct Scheduler {
    database: Arc<Database>,
    inner: Mutex<Inner>,
    locks: Mutex<BTreeMap<LockKey, Arc<AsyncMutex<()>>>>,
    network: Semaphore,
    events: broadcast::Sender<OperationEvent>,
}
pub struct OperationSpec {
    pub source_display: String,
    pub destination_display: String,
    pub reservation: Option<Vec<u8>>,
}
pub enum Completion {
    Succeeded,
    Cancelled,
}
impl Scheduler {
    pub fn new(database: Arc<Database>) -> Result<Self, AppError> {
        let (events, _) = broadcast::channel(256);
        let mut inner = Inner {
            stream: uuid::Uuid::new_v4().to_string(),
            sequence: 0,
            entries: BTreeMap::new(),
            reservations: HashMap::new(),
        };
        for mut snapshot in journal::load(&database)? {
            if !state::terminal(&snapshot.state) {
                snapshot.state = OperationState::Interrupted;
                snapshot.phase = Phase::Recovery;
                snapshot.can_cancel = false;
                snapshot.error = Some(error(ErrorCode::Interrupted, Phase::Recovery));
                snapshot.updated_at = chrono::Utc::now().to_rfc3339();
            }
            inner.sequence = inner.sequence.checked_add(1).ok_or_else(internal)?;
            if crate::clone::journal::retained(&database, &snapshot.id)? {
                snapshot.error = Some(crate::git::error::retained_error(snapshot.error.take()));
            }
            snapshot.sequence = inner.sequence.to_string();
            journal::write(&database, &snapshot)?;
            inner.entries.insert(
                snapshot.id.clone(),
                Entry {
                    snapshot,
                    cancel: Cancellation::default(),
                },
            );
        }
        Ok(Self {
            database,
            inner: Mutex::new(inner),
            locks: Mutex::new(BTreeMap::new()),
            network: Semaphore::new(2),
            events,
        })
    }
    pub fn subscribe(&self) -> broadcast::Receiver<OperationEvent> {
        self.events.subscribe()
    }
    pub fn snapshot(&self) -> Result<OperationsSnapshot, AppError> {
        let inner = self.inner.lock().map_err(|_| internal())?;
        Ok(OperationsSnapshot {
            stream_id: inner.stream.clone(),
            sequence: inner.sequence.to_string(),
            operations: inner.entries.values().map(|e| e.snapshot.clone()).collect(),
        })
    }
    /// Internal foundation hook; no IPC command creates a synthetic operation.
    pub fn enqueue(&self, spec: OperationSpec) -> Result<OperationSnapshot, AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        if spec
            .reservation
            .as_ref()
            .is_some_and(|key| inner.reservations.contains_key(key))
        {
            return Err(error(ErrorCode::DestinationBusy, Phase::Queue));
        }
        let now = chrono::Utc::now().to_rfc3339();
        let source = safe_remote(&spec.source_display).unwrap_or_else(|| "Local repository".into());
        // Destination is a backend-generated path display, never arbitrary IPC text.
        let destination = if spec.destination_display.len() <= 4096
            && !spec.destination_display.chars().any(char::is_control)
        {
            spec.destination_display
        } else {
            "Selected destination".into()
        };
        let snapshot = OperationSnapshot {
            id: uuid::Uuid::new_v4().to_string(),
            sequence: "0".into(),
            kind: OperationSnapshotKind::Clone,
            state: OperationState::Queued,
            phase: Phase::Queue,
            repo_id: None,
            worktree_id: None,
            source_display: source,
            destination_display: destination,
            progress: None,
            can_cancel: true,
            auth_prompt: None,
            error: None,
            created_at: now.clone(),
            updated_at: now,
        };
        let snapshot = self.commit(&mut inner, snapshot)?;
        if let Some(key) = spec.reservation {
            inner.reservations.insert(key, snapshot.id.clone());
        }
        Ok(snapshot)
    }
    fn commit(
        &self,
        inner: &mut Inner,
        mut snapshot: OperationSnapshot,
    ) -> Result<OperationSnapshot, AppError> {
        let next = inner.sequence.checked_add(1).ok_or_else(internal)?;
        snapshot.sequence = next.to_string();
        snapshot.updated_at = chrono::Utc::now().to_rfc3339();
        journal::write(&self.database, &snapshot)?;
        let token = inner
            .entries
            .get(&snapshot.id)
            .map(|e| e.cancel.clone())
            .unwrap_or_default();
        if state::terminal(&snapshot.state) {
            inner.reservations.retain(|_, id| id != &snapshot.id);
        }
        inner.entries.insert(
            snapshot.id.clone(),
            Entry {
                snapshot: snapshot.clone(),
                cancel: token,
            },
        );
        inner.sequence = next;
        // Send under the same mutex as the snapshot fence, after durable commit.
        let _ = self.events.send(OperationEvent {
            stream_id: inner.stream.clone(),
            sequence: next.to_string(),
            operation: snapshot.clone(),
        });
        Ok(snapshot)
    }
    pub fn transition(
        &self,
        id: &str,
        to: OperationState,
        phase: Phase,
        failure: Option<AppError>,
    ) -> Result<OperationSnapshot, AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let mut snapshot = inner.entries.get(id).ok_or_else(invalid)?.snapshot.clone();
        if state::terminal(&snapshot.state) {
            return Ok(snapshot);
        }
        // Cancel can win between lock acquisition and the Running commit.
        if to == OperationState::Running && snapshot.state == OperationState::Cancelling {
            snapshot.state = OperationState::Cancelled;
            snapshot.can_cancel = false;
            snapshot.auth_prompt = None;
            return self.commit(&mut inner, snapshot);
        }
        if !state::allowed(&snapshot.state, &to) {
            return Err(error(ErrorCode::InvalidRequest, Phase::Queue));
        }
        snapshot.state = to;
        snapshot.phase = phase;
        snapshot.can_cancel =
            !state::terminal(&snapshot.state) && snapshot.state != OperationState::Cancelling;
        snapshot.auth_prompt = None;
        snapshot.error = failure.map(|e| error(e.code, e.phase));
        if crate::clone::journal::retained(&self.database, id)? {
            snapshot.error = Some(crate::git::error::retained_error(snapshot.error.take()));
        }
        self.commit(&mut inner, snapshot)
    }
    pub fn cancel(&self, id: &str) -> Result<OperationSnapshot, AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let entry = inner.entries.get(id).ok_or_else(invalid)?;
        if state::terminal(&entry.snapshot.state)
            || entry.snapshot.state == OperationState::Cancelling
        {
            return Ok(entry.snapshot.clone());
        }
        let token = entry.cancel.clone();
        let mut snapshot = entry.snapshot.clone();
        snapshot.state = if snapshot.state == OperationState::Queued {
            OperationState::Cancelled
        } else {
            OperationState::Cancelling
        };
        snapshot.can_cancel = false;
        snapshot.auth_prompt = None;
        let result = self.commit(&mut inner, snapshot)?;
        token.cancel();
        Ok(result)
    }
    pub fn progress(
        &self,
        id: &str,
        progress: Progress,
        phase: Phase,
    ) -> Result<OperationSnapshot, AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let mut snapshot = inner.entries.get(id).ok_or_else(invalid)?.snapshot.clone();
        if state::terminal(&snapshot.state) {
            return Ok(snapshot);
        }
        if progress
            .total
            .is_some_and(|total| progress.completed > total)
        {
            return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
        }
        snapshot.progress = Some(progress);
        snapshot.phase = phase;
        self.commit(&mut inner, snapshot)
    }
    pub fn auth_prompt(&self, id: &str, prompt: AuthPrompt) -> Result<OperationSnapshot, AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let mut snapshot = inner.entries.get(id).ok_or_else(invalid)?.snapshot.clone();
        if !matches!(
            snapshot.state,
            OperationState::Running | OperationState::WaitingForUser
        ) {
            return Err(invalid());
        }
        snapshot.state = OperationState::WaitingForUser;
        snapshot.phase = Phase::Authentication;
        snapshot.auth_prompt = Some(prompt);
        self.commit(&mut inner, snapshot)
    }
    /// The durable state mutex is also the secret-release/cancellation
    /// linearization point. A later cancellation cannot revoke bytes already
    /// authorized for the owned helper; a cancellation that won releases none.
    pub fn resume_auth(&self, id: &str, is_active: impl Fn() -> bool) -> Result<bool, AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let entry = inner.entries.get(id).ok_or_else(invalid)?;
        if !matches!(
            entry.snapshot.state,
            OperationState::WaitingForUser | OperationState::Running
        ) || entry.cancel.is_cancelled()
            || !is_active()
        {
            return Ok(false);
        }
        let mut snapshot = entry.snapshot.clone();
        snapshot.state = OperationState::Running;
        snapshot.phase = Phase::Transfer;
        snapshot.auth_prompt = None;
        self.commit(&mut inner, snapshot)?;
        Ok(is_active()
            && !inner
                .entries
                .get(id)
                .ok_or_else(invalid)?
                .cancel
                .is_cancelled())
    }
    #[cfg(windows)]
    pub fn succeed_clone(
        &self,
        id: &str,
        item: &crate::repositories::discovery::Discovered,
        record: &mut crate::clone::journal::CloneRecord,
        warning: Option<AppError>,
    ) -> Result<OperationSnapshot, AppError> {
        use crate::{persistence::workspace::db_error, repositories::registry::Registry};
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let mut snapshot = inner.entries.get(id).ok_or_else(invalid)?.snapshot.clone();
        if state::terminal(&snapshot.state) {
            return Ok(snapshot);
        }
        let next = inner.sequence.checked_add(1).ok_or_else(internal)?;
        let mut connection = self.database.lock()?;
        let transaction = connection.transaction().map_err(|_| db_error())?;
        let selection = Registry::register_transaction(&transaction, item)?;
        snapshot.repo_id = Some(selection.repo_id);
        snapshot.worktree_id = selection.worktree_id;
        snapshot.state = OperationState::Succeeded;
        snapshot.phase = Phase::Registration;
        snapshot.can_cancel = false;
        snapshot.auth_prompt = None;
        snapshot.sequence = next.to_string();
        snapshot.updated_at = chrono::Utc::now().to_rfc3339();
        snapshot.error = warning;
        snapshot.progress = Some(Progress {
            completed: 4,
            total: Some(4),
            unit: ProgressUnit::Steps,
        });
        record.phase = crate::clone::journal::ClonePhase::Registered;
        journal::write_to(&transaction, &snapshot)?;
        crate::clone::journal::write_to(&transaction, record)?;
        transaction.commit().map_err(|_| db_error())?;
        let cancel = inner.entries.get(id).ok_or_else(invalid)?.cancel.clone();
        inner.entries.insert(
            id.into(),
            Entry {
                snapshot: snapshot.clone(),
                cancel,
            },
        );
        inner.reservations.retain(|_, owner| owner != id);
        inner.sequence = next;
        let _ = self.events.send(OperationEvent {
            stream_id: inner.stream.clone(),
            sequence: next.to_string(),
            operation: snapshot.clone(),
        });
        Ok(snapshot)
    }
    pub async fn execute<F, Fut>(
        &self,
        id: &str,
        keys: Vec<LockKey>,
        network: bool,
        job: F,
    ) -> Result<OperationSnapshot, AppError>
    where
        F: FnOnce(Cancellation) -> Fut,
        Fut: std::future::Future<Output = Result<Completion, AppError>>,
    {
        let cancel = {
            let inner = self.inner.lock().map_err(|_| internal())?;
            let e = inner.entries.get(id).ok_or_else(invalid)?;
            if state::terminal(&e.snapshot.state) {
                return Ok(e.snapshot.clone());
            }
            e.cancel.clone()
        };
        let mut keys = keys; // Every worktree lock also acquires its common-directory lock first.
        keys.extend(
            keys.clone()
                .into_iter()
                .filter(|key| key.worktree.is_some())
                .map(|key| LockKey {
                    common: key.common,
                    worktree: None,
                }),
        );
        keys.sort();
        keys.dedup();
        let locks = {
            let mut map = self.locks.lock().map_err(|_| internal())?;
            keys.into_iter()
                .map(|key| {
                    map.entry(key)
                        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
                        .clone()
                })
                .collect::<Vec<_>>()
        };
        let acquire = async {
            let mut guards = Vec::new();
            for lock in locks {
                guards.push(lock.lock_owned().await);
            }
            let permit = if network {
                Some(self.network.acquire().await.map_err(|_| internal())?)
            } else {
                None
            };
            Ok::<_, AppError>((guards, permit))
        };
        let cancelled = async {
            while !cancel.is_cancelled() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        let _guards = tokio::select! {value=acquire=>value?,_=cancelled=>return self.transition(id,OperationState::Cancelled,Phase::Queue,None)};
        let running = self.transition(id, OperationState::Running, Phase::Validation, None)?;
        if state::terminal(&running.state) {
            return Ok(running);
        }
        let result = job(cancel.clone()).await;
        match result {
            Ok(Completion::Succeeded) => {
                self.transition(id, OperationState::Succeeded, Phase::Registration, None)
            }
            Ok(Completion::Cancelled) => {
                self.transition(id, OperationState::Cancelled, Phase::Transfer, None)
            }
            Err(e) if e.code == ErrorCode::Interrupted && e.phase == Phase::Recovery => {
                self.transition(id, OperationState::Interrupted, Phase::Recovery, Some(e))
            }
            Err(e) if cancel.is_cancelled() && e.code == ErrorCode::Interrupted => {
                self.transition(id, OperationState::Cancelled, Phase::Transfer, None)
            }
            Err(e) if e.code == ErrorCode::Database && e.phase == Phase::Recovery => {
                self.transition(id, OperationState::Interrupted, Phase::Recovery, Some(e))
            }
            Err(e) => self.transition(id, OperationState::Failed, e.phase.clone(), Some(e)),
        }
    }
}
fn invalid() -> AppError {
    error(ErrorCode::InvalidHandle, Phase::Validation)
}
fn internal() -> AppError {
    error(ErrorCode::Internal, Phase::Queue)
}
