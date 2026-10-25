use super::{journal, state};
use crate::{
    domain::contracts::*,
    git::{error::error, refs::safe_remote, runner::Cancellation},
    persistence::workspace::Database,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::sync::{
    broadcast, Mutex as AsyncMutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore,
};
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
    locks: Mutex<BTreeMap<LockKey, Weak<AsyncMutex<()>>>>,
    network: Arc<Semaphore>,
    events: broadcast::Sender<OperationEvent>,
    action_records: usize,
    fetch_bytes: usize,
    clock: Arc<dyn Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync>,
}
/// Pure scope ownership. No scheduler/database/intent is reachable from this
/// lease. Existing BQ lifetime guards can retain it until process exit is proved.
pub struct SerializationLease {
    _guards: Vec<OwnedMutexGuard<()>>,
    _network: Option<OwnedSemaphorePermit>,
}
pub struct OperationSpec {
    pub source_display: String,
    pub destination_display: String,
    pub reservation: Option<Vec<u8>>,
}
pub enum Completion {
    Succeeded,
    Cancelled,
    Action {
        effects: ActionEffects,
        postcondition: Option<String>,
    },
}
pub(crate) type CompletionInvalidation = Arc<dyn Fn(ActionEffects) + Send + Sync>;
impl Scheduler {
    pub fn new(database: Arc<Database>) -> Result<Self, AppError> {
        Self::with_action_retention(database, 4096, Arc::new(chrono::Utc::now))
    }
    /// Trusted bounded test seam, never a renderer setting.
    pub fn with_action_retention(
        database: Arc<Database>,
        action_records: usize,
        clock: Arc<dyn Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync>,
    ) -> Result<Self, AppError> {
        Self::with_action_budgets(
            database,
            action_records,
            crate::actions::observations::LEDGER_BYTES,
            clock,
        )
    }
    /// Trusted small quota/clock seam, never exposed to a renderer.
    pub fn with_action_budgets(
        database: Arc<Database>,
        action_records: usize,
        fetch_bytes: usize,
        clock: Arc<dyn Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync>,
    ) -> Result<Self, AppError> {
        if !(crate::actions::observations::ACTION_BYTES
            ..=crate::actions::observations::LEDGER_BYTES)
            .contains(&fetch_bytes)
        {
            return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
        }
        if !(1..=4096).contains(&action_records) {
            return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
        }
        let (events, _) = broadcast::channel(256);
        let mut inner = Inner {
            stream: uuid::Uuid::new_v4().to_string(),
            sequence: 0,
            entries: BTreeMap::new(),
            reservations: HashMap::new(),
        };
        for mut snapshot in journal::load(&database)? {
            crate::actions::recovery::reconcile(&database, &mut snapshot)?;
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
            network: Arc::new(Semaphore::new(2)),
            events,
            action_records,
            fetch_bytes,
            clock,
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
            action_kind: None,
            outcome: None,
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
    /// Internal executor authority only; no renderer can enqueue an action.
    pub fn get(&self, id: &str) -> Result<OperationSnapshot, AppError> {
        self.inner
            .lock()
            .map_err(|_| internal())?
            .entries
            .get(id)
            .map(|e| e.snapshot.clone())
            .ok_or_else(invalid)
    }
    pub fn enqueue_action(
        &self,
        record: crate::actions::journal::ActionRecord,
    ) -> Result<OperationSnapshot, AppError> {
        self.enqueue_action_protected(record, &HashSet::new())
    }
    pub fn enqueue_action_protected(
        &self,
        record: crate::actions::journal::ActionRecord,
        protected: &HashSet<String>,
    ) -> Result<OperationSnapshot, AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        if inner
            .entries
            .values()
            .filter(|e| {
                e.snapshot.kind == OperationSnapshotKind::Action
                    && !state::terminal(&e.snapshot.state)
            })
            .count()
            >= 64
        {
            return Err(action_capacity());
        }
        let total = inner
            .entries
            .values()
            .filter(|e| e.snapshot.kind == OperationSnapshotKind::Action)
            .count();
        let needed = total.saturating_add(1).saturating_sub(self.action_records);
        let needed_bytes = if record.kind == ActionKind::Fetch {
            let connection = self.database.lock()?;
            crate::actions::observations::used(&connection)?
                .checked_add(crate::actions::observations::ACTION_BYTES)
                .ok_or_else(action_capacity)?
                .saturating_sub(self.fetch_bytes)
        } else {
            0
        };
        let mut reclaimed = 0usize;
        let mut victims = Vec::new();
        if needed > 0 || needed_bytes > 0 {
            let cutoff = (self.clock)() - chrono::Duration::seconds(900);
            let connection = self.database.lock()?;
            for entry in inner.entries.values() {
                let v = &entry.snapshot;
                if v.kind != OperationSnapshotKind::Action
                    || protected.contains(&v.id)
                    || !matches!(
                        v.state,
                        OperationState::Succeeded
                            | OperationState::Failed
                            | OperationState::Cancelled
                    )
                {
                    continue;
                }
                let Some(outcome) = v.outcome.as_ref() else {
                    continue;
                };
                if outcome.recovery != ActionRecovery::None
                    || !matches!(
                        outcome.effects,
                        ActionEffects::Applied | ActionEffects::NotApplied
                    )
                {
                    continue;
                }
                let Ok(observed) = chrono::DateTime::parse_from_rfc3339(&outcome.observed_at)
                else {
                    continue;
                };
                if observed.with_timezone(&chrono::Utc) > cutoff {
                    continue;
                }
                let r = crate::actions::journal::read_from(&connection, &v.id)?;
                if r.phase == crate::actions::journal::WritePhase::Observed
                    && r.artifacts.is_empty()
                    && r.outcome.as_ref() == Some(outcome)
                {
                    let weight = crate::actions::observations::weight(&connection, &v.id)?;
                    if victims.len() >= needed && weight == 0 {
                        continue;
                    }
                    victims.push(v.id.clone());
                    reclaimed = reclaimed.checked_add(weight).ok_or_else(action_capacity)?;
                    if victims.len() >= needed && reclaimed >= needed_bytes {
                        break;
                    }
                }
            }
            if victims.len() < needed || reclaimed < needed_bytes {
                return Err(if needed_bytes > reclaimed {
                    crate::actions::observations::capacity()
                } else {
                    action_capacity()
                });
            }
        }
        let now = chrono::Utc::now().to_rfc3339();
        let snapshot = OperationSnapshot {
            id: uuid::Uuid::new_v4().to_string(),
            sequence: "0".into(),
            kind: OperationSnapshotKind::Action,
            action_kind: Some(record.kind),
            outcome: None,
            state: OperationState::Queued,
            phase: Phase::Queue,
            repo_id: Some(record.repo_id.clone()),
            worktree_id: record.worktree_id.clone(),
            source_display: "Repository action".into(),
            destination_display: "Selected repository".into(),
            progress: None,
            can_cancel: true,
            auth_prompt: None,
            error: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.commit_action_pruning(&mut inner, snapshot, &record, &victims, |_| Ok(()))
    }
    fn commit_action(
        &self,
        inner: &mut Inner,
        snapshot: OperationSnapshot,
        record: &crate::actions::journal::ActionRecord,
    ) -> Result<OperationSnapshot, AppError> {
        self.commit_action_pruning(inner, snapshot, record, &[], |_| Ok(()))
    }
    fn commit_action_pruning(
        &self,
        inner: &mut Inner,
        mut snapshot: OperationSnapshot,
        record: &crate::actions::journal::ActionRecord,
        victims: &[String],
        validate: impl FnOnce(&rusqlite::Connection) -> Result<(), AppError>,
    ) -> Result<OperationSnapshot, AppError> {
        let reserve_fetch =
            !inner.entries.contains_key(&snapshot.id) && record.kind == ActionKind::Fetch;
        let next = inner.sequence.checked_add(1).ok_or_else(internal)?;
        snapshot.sequence = next.to_string();
        snapshot.updated_at = chrono::Utc::now().to_rfc3339();
        {
            let mut c = self.database.lock()?;
            let tx = c
                .transaction()
                .map_err(|_| crate::persistence::workspace::db_error())?;
            validate(&tx)?;
            if state::terminal(&snapshot.state) {
                crate::actions::observations::settle_to(&tx, &snapshot.id)?;
            }
            for id in victims {
                tx.execute("DELETE FROM repository_actions WHERE operation_id=?1", [id])
                    .map_err(|_| crate::persistence::workspace::db_error())?;
                tx.execute("DELETE FROM operations WHERE id=?1", [id])
                    .map_err(|_| crate::persistence::workspace::db_error())?;
            }
            journal::write_to(&tx, &snapshot)?;
            crate::actions::journal::write_to(&tx, &snapshot.id, record)?;
            if reserve_fetch {
                crate::actions::observations::reserve_to(&tx, &snapshot.id, self.fetch_bytes)?;
            }
            tx.commit()
                .map_err(|_| crate::persistence::workspace::db_error())?;
        }
        if !victims.is_empty() {
            for id in victims {
                inner.entries.remove(id);
            }
            inner.stream = uuid::Uuid::new_v4().to_string();
        }
        let cancel = inner
            .entries
            .get(&snapshot.id)
            .map(|e| e.cancel.clone())
            .unwrap_or_default();
        inner.entries.insert(
            snapshot.id.clone(),
            Entry {
                snapshot: snapshot.clone(),
                cancel,
            },
        );
        inner.sequence = next;
        let _ = self.events.send(OperationEvent {
            stream_id: inner.stream.clone(),
            sequence: next.to_string(),
            operation: snapshot.clone(),
        });
        Ok(snapshot)
    }
    pub fn action_write_started(&self, id: &str) -> Result<bool, AppError> {
        Ok(crate::actions::journal::read(&self.database, id)?.phase
            == crate::actions::journal::WritePhase::WriteStarted)
    }
    pub fn action_owner(&self, id: &str) -> Result<Option<String>, AppError> {
        let inner = self.inner.lock().map_err(|_| internal())?;
        let e = inner.entries.get(id).ok_or_else(invalid)?;
        if e.snapshot.kind != OperationSnapshotKind::Action {
            return Ok(None);
        }
        Ok(Some(
            crate::actions::journal::read(&self.database, id)?.owner_window,
        ))
    }
    pub fn cancel_owned_actions(&self, window: &str, repo: Option<&str>) {
        let ids = self
            .inner
            .lock()
            .map(|inner| {
                inner
                    .entries
                    .values()
                    .filter(|e| {
                        e.snapshot.kind == OperationSnapshotKind::Action
                            && !state::terminal(&e.snapshot.state)
                            && repo.is_none_or(|r| e.snapshot.repo_id.as_deref() == Some(r))
                    })
                    .map(|e| e.snapshot.id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for id in ids {
            if self.action_owner(&id).ok().flatten().as_deref() == Some(window) {
                let _ = self.cancel(&id);
            }
        }
    }
    pub fn cancel_repo_actions(&self, repo: &str) {
        let ids = self
            .inner
            .lock()
            .map(|inner| {
                inner
                    .entries
                    .values()
                    .filter(|e| {
                        e.snapshot.kind == OperationSnapshotKind::Action
                            && e.snapshot.repo_id.as_deref() == Some(repo)
                            && !state::terminal(&e.snapshot.state)
                    })
                    .map(|e| e.snapshot.id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for id in ids {
            let _ = self.cancel(&id);
        }
    }
    /// Linearization point: durable before-write evidence wins before native work.
    pub fn action_before_write(&self, id: &str) -> Result<bool, AppError> {
        self.action_before_write_checked(id, |_| Ok(()))
    }
    pub fn action_before_write_checked(
        &self,
        id: &str,
        validate: impl FnOnce(&rusqlite::Connection) -> Result<(), AppError>,
    ) -> Result<bool, AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let e = inner.entries.get(id).ok_or_else(invalid)?;
        if e.snapshot.kind != OperationSnapshotKind::Action {
            return Err(invalid());
        }
        if e.snapshot.action_kind == Some(ActionKind::ReadRemoteRefs) {
            return Err(invalid());
        }
        if e.cancel.is_cancelled()
            || state::terminal(&e.snapshot.state)
            || e.snapshot.state == OperationState::Cancelling
        {
            return Ok(false);
        }
        if e.snapshot.state != OperationState::Running {
            return Err(invalid());
        }
        let mut record = crate::actions::journal::read(&self.database, id)?;
        if record.phase != crate::actions::journal::WritePhase::Prepared {
            return Err(invalid());
        }
        record.phase = crate::actions::journal::WritePhase::WriteStarted;
        let mut snapshot = e.snapshot.clone();
        snapshot.phase = Phase::Publish;
        self.commit_action_pruning(&mut inner, snapshot, &record, &[], validate)?;
        Ok(true)
    }
    pub(crate) fn action_read_started(&self, id: &str) -> Result<bool, AppError> {
        Ok(crate::actions::journal::read(&self.database, id)?
            .remote_read
            .as_ref()
            .is_some_and(|p| p.phase == FetchStepPhase::Started))
    }
    pub(crate) fn action_read_start_checked(
        &self,
        id: &str,
        role: RemoteEndpointRole,
        validate: impl FnOnce(&rusqlite::Connection) -> Result<(), AppError>,
    ) -> Result<bool, AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let e = inner.entries.get(id).ok_or_else(invalid)?;
        if e.cancel.is_cancelled()
            || state::terminal(&e.snapshot.state)
            || e.snapshot.state == OperationState::Cancelling
        {
            return Ok(false);
        }
        if e.snapshot.state != OperationState::Running
            || e.snapshot.action_kind != Some(ActionKind::ReadRemoteRefs)
        {
            return Err(invalid());
        }
        let mut record = crate::actions::journal::read(&self.database, id)?;
        if record.phase != crate::actions::journal::WritePhase::Prepared
            || record.remote_read.is_some()
        {
            return Err(invalid());
        }
        let Some(crate::actions::journal::SafeTargetProof::Remote {
            name,
            config_digest,
        }) = record.targets.first()
        else {
            return Err(invalid());
        };
        record.remote_read = Some(crate::actions::journal::RemoteReadProof {
            remote_name: name.clone(),
            role,
            config_digest: config_digest.clone(),
            phase: FetchStepPhase::Started,
            ref_count: None,
            result_digest: None,
            observed_at: None,
            no_git_writes: false,
        });
        let mut snapshot = e.snapshot.clone();
        snapshot.phase = Phase::Transfer;
        self.commit_action_pruning(&mut inner, snapshot, &record, &[], validate)?;
        Ok(true)
    }
    pub(crate) fn action_read_observed(
        &self,
        id: &str,
        count: Option<usize>,
        digest: Option<String>,
    ) -> Result<(), AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let snapshot = inner.entries.get(id).ok_or_else(invalid)?.snapshot.clone();
        let mut record = crate::actions::journal::read(&self.database, id)?;
        let proof = record.remote_read.as_mut().ok_or_else(invalid)?;
        if proof.phase != FetchStepPhase::Started {
            return Err(invalid());
        }
        proof.phase = FetchStepPhase::Observed;
        proof.no_git_writes = true;
        proof.ref_count = count.map(|n| n.to_string());
        proof.result_digest = digest;
        proof.observed_at = Some(chrono::Utc::now().to_rfc3339());
        self.commit_action(&mut inner, snapshot, &record)?;
        Ok(())
    }
    pub(crate) fn action_fetch_step_checked(
        &self,
        id: &str,
        index: usize,
        refs_digest: &str,
        validate: impl FnOnce(&rusqlite::Connection) -> Result<(), AppError>,
    ) -> Result<bool, AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let entry = inner.entries.get(id).ok_or_else(invalid)?;
        if entry.cancel.is_cancelled()
            || state::terminal(&entry.snapshot.state)
            || entry.snapshot.state == OperationState::Cancelling
        {
            return Ok(false);
        }
        if entry.snapshot.state != OperationState::Running
            || entry.snapshot.action_kind != Some(ActionKind::Fetch)
        {
            return Err(invalid());
        }
        let record = crate::actions::journal::read(&self.database, id)?;
        if record.phase != crate::actions::journal::WritePhase::WriteStarted {
            return Err(invalid());
        }
        let mut snapshot = entry.snapshot.clone();
        snapshot.phase = Phase::Transfer;
        self.commit_action_pruning(&mut inner, snapshot, &record, &[], |connection| {
            validate(connection)?;
            crate::actions::observations::start_to(connection, id, index, refs_digest)
        })?;
        Ok(true)
    }
    pub(crate) fn action_ref_step_checked(
        &self,
        id: &str,
        index: usize,
        stamp: &str,
        validate: impl FnOnce(&rusqlite::Connection) -> Result<(), AppError>,
    ) -> Result<bool, AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let e = inner.entries.get(id).ok_or_else(invalid)?;
        if e.cancel.is_cancelled()
            || state::terminal(&e.snapshot.state)
            || e.snapshot.state == OperationState::Cancelling
        {
            return Ok(false);
        }
        if e.snapshot.state != OperationState::Running {
            return Err(invalid());
        }
        let mut record = crate::actions::journal::read(&self.database, id)?;
        if record.phase != crate::actions::journal::WritePhase::WriteStarted {
            return Err(invalid());
        }
        let proof = record.ref_action.as_mut().ok_or_else(invalid)?;
        if proof
            .steps
            .iter()
            .take(index)
            .any(|s| s.phase != FetchStepPhase::Observed || s.effects != ActionEffects::Applied)
        {
            return Err(invalid());
        }
        let step = proof.steps.get_mut(index).ok_or_else(invalid)?;
        if step.phase != FetchStepPhase::Pending {
            return Err(invalid());
        }
        step.phase = FetchStepPhase::Started;
        step.before_fingerprint = Some(stamp.into());
        let snapshot = e.snapshot.clone();
        self.commit_action_pruning(&mut inner, snapshot, &record, &[], validate)?;
        Ok(true)
    }
    pub(crate) fn action_ref_observed(
        &self,
        id: &str,
        index: usize,
        effects: ActionEffects,
        stamp: Option<String>,
        reason: Option<RefActionFailureReason>,
    ) -> Result<(), AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let e = inner.entries.get(id).ok_or_else(invalid)?;
        let snapshot = e.snapshot.clone();
        let mut record = crate::actions::journal::read(&self.database, id)?;
        let step = record
            .ref_action
            .as_mut()
            .and_then(|r| r.steps.get_mut(index))
            .ok_or_else(invalid)?;
        if step.phase != FetchStepPhase::Started {
            return Err(invalid());
        }
        step.phase = FetchStepPhase::Observed;
        step.effects = effects;
        step.after_fingerprint = stamp;
        step.failure_reason = reason;
        self.commit_action(&mut inner, snapshot, &record)?;
        Ok(())
    }
    pub fn finish_action(
        &self,
        id: &str,
        effects: ActionEffects,
        postcondition: Option<String>,
        failure: Option<AppError>,
    ) -> Result<OperationSnapshot, AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let entry = inner.entries.get(id).ok_or_else(invalid)?;
        if state::terminal(&entry.snapshot.state) {
            return Ok(entry.snapshot.clone());
        }
        let cancelled = entry.cancel.is_cancelled();
        let mut record = crate::actions::journal::read(&self.database, id)?;
        if record.kind == ActionKind::ReadRemoteRefs
            && !matches!(effects, ActionEffects::NotApplied | ActionEffects::Unknown)
        {
            return Err(invalid());
        }
        if effects != ActionEffects::NotApplied
            && record.phase == crate::actions::journal::WritePhase::Prepared
            && record.remote_read.is_none()
        {
            return Err(invalid());
        }
        let mut outcome = crate::actions::journal::outcome(effects, cancelled);
        if record
            .change_action
            .as_ref()
            .is_some_and(|p| p.result.artifacts_retained)
        {
            outcome.recovery = ActionRecovery::Inspect;
        }
        record.phase = crate::actions::journal::WritePhase::Observed;
        record.postcondition = postcondition;
        record.outcome = Some(outcome.clone());
        let mut snapshot = entry.snapshot.clone();
        snapshot.outcome = Some(outcome);
        snapshot.can_cancel = false;
        snapshot.auth_prompt = None;
        snapshot.state = match effects {
            ActionEffects::Applied => OperationState::Succeeded,
            ActionEffects::NotApplied if cancelled => OperationState::Cancelled,
            ActionEffects::NotApplied if failure.is_none() => OperationState::Succeeded,
            ActionEffects::Unknown => OperationState::Interrupted,
            _ => OperationState::Failed,
        };
        snapshot.phase = if effects == ActionEffects::Unknown {
            Phase::Recovery
        } else {
            Phase::Verification
        };
        snapshot.error = failure.map(|e| crate::actions::safe_error(&e));
        self.commit_action(&mut inner, snapshot, &record)
    }
    pub fn action_failure(
        &self,
        id: &str,
        failure: AppError,
    ) -> Result<OperationSnapshot, AppError> {
        let effects = self.action_failure_effects(id);
        self.finish_failure(id, effects, failure)
    }
    /// Inspect durable effects without holding the scheduler mutex. Backend
    /// invalidation uses this before terminal publication, including aborts.
    pub(crate) fn action_failure_effects(&self, id: &str) -> ActionEffects {
        let phase = crate::actions::journal::read(&self.database, id)
            .ok()
            .map(|record| record.phase);
        let read = crate::actions::journal::read(&self.database, id)
            .ok()
            .and_then(|r| r.remote_read);
        if let Some(read) = read {
            read.effects()
        } else if phase == Some(crate::actions::journal::WritePhase::Prepared) {
            ActionEffects::NotApplied
        } else {
            crate::actions::journal::read(&self.database, id)
                .ok()
                .and_then(|r| r.sync_action)
                .map(|p| p.effects())
                .or_else(|| {
                    crate::actions::journal::read(&self.database, id)
                        .ok()
                        .and_then(|r| r.change_action)
                        .map(|p| p.effects)
                })
                .or_else(|| {
                    crate::actions::journal::read(&self.database, id)
                        .ok()
                        .and_then(|r| r.ref_action)
                        .map(|p| crate::actions::branches::effects(&p))
                })
                .or_else(|| {
                    self.database
                        .lock()
                        .ok()
                        .and_then(|c| {
                            crate::actions::observations::read_from(&c, id)
                                .ok()
                                .flatten()
                        })
                        .map(|record| crate::actions::observations::effects(&record))
                })
                .unwrap_or(ActionEffects::Unknown)
        }
    }
    fn finish_failure(
        &self,
        id: &str,
        effects: ActionEffects,
        failure: AppError,
    ) -> Result<OperationSnapshot, AppError> {
        match self.finish_action(id, effects, None, Some(failure)) {
            Ok(v) => Ok(v),
            Err(_) => {
                // The durable checkpoint remains authoritative if SQLite refuses
                // final publication. Never turn a possible write into Cancelled.
                let mut inner = self.inner.lock().map_err(|_| internal())?;
                let entry = inner.entries.get_mut(id).ok_or_else(invalid)?;
                entry.snapshot.state = OperationState::Interrupted;
                entry.snapshot.phase = Phase::Recovery;
                entry.snapshot.can_cancel = false;
                entry.snapshot.auth_prompt = None;
                entry.snapshot.outcome = Some(crate::actions::journal::outcome(
                    effects,
                    entry.cancel.is_cancelled(),
                ));
                entry.snapshot.error = Some(crate::actions::safe_error(&error(
                    ErrorCode::Database,
                    Phase::Recovery,
                )));
                Ok(entry.snapshot.clone())
            }
        }
    }
    pub(crate) fn action_change_checkpoint(
        &self,
        id: &str,
        proof: crate::actions::journal::ChangeActionProof,
        artifacts: Vec<crate::actions::journal::ArtifactProof>,
    ) -> Result<(), AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let snapshot = inner.entries.get(id).ok_or_else(invalid)?.snapshot.clone();
        if state::terminal(&snapshot.state) {
            return Err(invalid());
        }
        let mut record = crate::actions::journal::read(&self.database, id)?;
        if !crate::actions::changes::supported(record.kind) {
            return Err(invalid());
        }
        record.change_action = Some(proof);
        record.artifacts = artifacts;
        self.commit_action(&mut inner, snapshot, &record)?;
        Ok(())
    }
    pub(crate) fn action_sync_begin(
        &self,
        id: &str,
        proof: crate::actions::journal::SyncActionProof,
    ) -> Result<(), AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let snapshot = inner.entries.get(id).ok_or_else(invalid)?.snapshot.clone();
        let mut record = crate::actions::journal::read(&self.database, id)?;
        if snapshot.state != OperationState::Running
            || record.phase != crate::actions::journal::WritePhase::Prepared
            || record.sync_action.is_some()
        {
            return Err(invalid());
        }
        crate::actions::sync::validate_proof(&proof, record.kind)?;
        record.sync_action = Some(proof);
        self.commit_action(&mut inner, snapshot, &record)?;
        Ok(())
    }
    pub(crate) fn action_sync_step_checked(
        &self,
        id: &str,
        index: usize,
        stamp: &str,
        validate: impl FnOnce(&rusqlite::Connection) -> Result<(), AppError>,
    ) -> Result<bool, AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let e = inner.entries.get(id).ok_or_else(invalid)?;
        if e.cancel.is_cancelled()
            || state::terminal(&e.snapshot.state)
            || e.snapshot.state == OperationState::Cancelling
        {
            return Ok(false);
        }
        if e.snapshot.state != OperationState::Running {
            return Err(invalid());
        }
        let snapshot = e.snapshot.clone();
        let mut record = crate::actions::journal::read(&self.database, id)?;
        let p = record.sync_action.as_mut().ok_or_else(invalid)?;
        let step = p.result.steps.get_mut(index).ok_or_else(invalid)?;
        if step.phase != FetchStepPhase::Pending || stamp.len() != 64 {
            return Err(invalid());
        }
        step.phase = FetchStepPhase::Started;
        p.step_stamps[index] = Some(stamp.into());
        record.phase = crate::actions::journal::WritePhase::WriteStarted;
        self.commit_action_pruning(&mut inner, snapshot, &record, &[], validate)?;
        Ok(true)
    }
    pub(crate) fn action_sync_observed(
        &self,
        id: &str,
        proof: crate::actions::journal::SyncActionProof,
    ) -> Result<(), AppError> {
        let mut inner = self.inner.lock().map_err(|_| internal())?;
        let snapshot = inner.entries.get(id).ok_or_else(invalid)?.snapshot.clone();
        let mut record = crate::actions::journal::read(&self.database, id)?;
        let old = record.sync_action.as_ref().ok_or_else(invalid)?;
        if old.remote_name != proof.remote_name
            || old.role != proof.role
            || old.config_digest != proof.config_digest
            || old.source_ref != proof.source_ref
            || old.source_oid != proof.source_oid
            || old.destination != proof.destination
            || old.preview_oid != proof.preview_oid
            || old.result.steps.len() != proof.result.steps.len()
        {
            return Err(invalid());
        }
        let publication = if old.result.publication.is_none() {
            proof.result.publication.clone()
        } else {
            if old.result.publication != proof.result.publication {
                return Err(invalid());
            }
            None
        };
        let repo = record.repo_id.clone();
        record.sync_action = Some(proof);
        self.commit_action_pruning(&mut inner, snapshot, &record, &[], |c| {
            if let Some(p) = publication {
                crate::actions::publication::write_to(c, id, &repo, &p)?;
            }
            Ok(())
        })?;
        Ok(())
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
            if snapshot.kind == OperationSnapshotKind::Action {
                let mut record = crate::actions::journal::read(&self.database, id)?;
                let outcome = crate::actions::journal::outcome(ActionEffects::NotApplied, true);
                record.phase = crate::actions::journal::WritePhase::Observed;
                record.outcome = Some(outcome.clone());
                snapshot.outcome = Some(outcome);
                return self.commit_action(&mut inner, snapshot, &record);
            }
            return self.commit(&mut inner, snapshot);
        }
        if snapshot.kind == OperationSnapshotKind::Action
            && matches!(
                to,
                OperationState::Succeeded
                    | OperationState::Failed
                    | OperationState::Interrupted
                    | OperationState::Cancelled
            )
        {
            return Err(invalid());
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
        let result = if snapshot.kind == OperationSnapshotKind::Action
            && snapshot.state == OperationState::Cancelled
        {
            let mut record = crate::actions::journal::read(&self.database, id)?;
            let outcome = crate::actions::journal::outcome(ActionEffects::NotApplied, true);
            record.phase = crate::actions::journal::WritePhase::Observed;
            record.outcome = Some(outcome.clone());
            snapshot.outcome = Some(outcome);
            self.commit_action(&mut inner, snapshot, &record)?
        } else {
            self.commit(&mut inner, snapshot)?
        };
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
    pub async fn acquire_scope(
        &self,
        keys: Vec<LockKey>,
        network: bool,
        cancel: &Cancellation,
        timeout: Option<Duration>,
    ) -> Result<Arc<SerializationLease>, AppError> {
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
            map.retain(|_, value| value.strong_count() > 0);
            keys.into_iter()
                .map(|key| {
                    if let Some(lock) = map.get(&key).and_then(Weak::upgrade) {
                        lock
                    } else {
                        let lock = Arc::new(AsyncMutex::new(()));
                        map.insert(key, Arc::downgrade(&lock));
                        lock
                    }
                })
                .collect::<Vec<_>>()
        };
        let acquire = async {
            let mut guards = Vec::new();
            for lock in locks {
                guards.push(lock.lock_owned().await);
            }
            let permit = if network {
                Some(
                    self.network
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(|_| internal())?,
                )
            } else {
                None
            };
            Ok::<_, AppError>(Arc::new(SerializationLease {
                _guards: guards,
                _network: permit,
            }))
        };
        let cancelled = async {
            while !cancel.is_cancelled() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        let deadline = async {
            if let Some(timeout) = timeout {
                tokio::time::sleep(timeout).await
            } else {
                std::future::pending::<()>().await
            }
        };
        tokio::select! {value=acquire=>value,_=cancelled=>Err(error(ErrorCode::Interrupted,Phase::Queue)),_=deadline=>Err(error(ErrorCode::Timeout,Phase::Queue))}
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
        if self.get(id)?.kind != OperationSnapshotKind::Clone {
            return Err(invalid());
        }
        self.execute_inner(id, keys, network, None, move |cancel, _lease| job(cancel))
            .await
    }
    pub async fn execute_action<F, Fut>(
        &self,
        id: &str,
        keys: Vec<LockKey>,
        network: bool,
        job: F,
    ) -> Result<OperationSnapshot, AppError>
    where
        F: FnOnce(Cancellation, Arc<SerializationLease>) -> Fut,
        Fut: std::future::Future<Output = Result<Completion, AppError>>,
    {
        if self.get(id)?.kind != OperationSnapshotKind::Action {
            return Err(invalid());
        }
        self.execute_inner(id, keys, network, None, job).await
    }
    pub(crate) async fn execute_action_with_invalidation<F, Fut>(
        &self,
        id: &str,
        keys: Vec<LockKey>,
        network: bool,
        before_finish: CompletionInvalidation,
        job: F,
    ) -> Result<OperationSnapshot, AppError>
    where
        F: FnOnce(Cancellation, Arc<SerializationLease>) -> Fut,
        Fut: std::future::Future<Output = Result<Completion, AppError>>,
    {
        if self.get(id)?.kind != OperationSnapshotKind::Action {
            return Err(invalid());
        }
        self.execute_inner(id, keys, network, Some(before_finish), job)
            .await
    }
    async fn execute_inner<F, Fut>(
        &self,
        id: &str,
        keys: Vec<LockKey>,
        network: bool,
        before_finish: Option<CompletionInvalidation>,
        job: F,
    ) -> Result<OperationSnapshot, AppError>
    where
        F: FnOnce(Cancellation, Arc<SerializationLease>) -> Fut,
        Fut: std::future::Future<Output = Result<Completion, AppError>>,
    {
        let (cancel, action) = {
            let inner = self.inner.lock().map_err(|_| internal())?;
            let e = inner.entries.get(id).ok_or_else(invalid)?;
            if state::terminal(&e.snapshot.state) {
                return Ok(e.snapshot.clone());
            }
            (
                e.cancel.clone(),
                e.snapshot.kind == OperationSnapshotKind::Action,
            )
        };
        let serialization = match self
            .acquire_scope(
                keys,
                network,
                &cancel,
                action.then_some(Duration::from_secs(30)),
            )
            .await
        {
            Ok(lease) => lease,
            Err(e) if e.code == ErrorCode::Interrupted => {
                return if action {
                    self.finish_action(id, ActionEffects::NotApplied, None, None)
                } else {
                    self.transition(id, OperationState::Cancelled, Phase::Queue, None)
                }
            }
            Err(e) if action => return self.action_failure(id, e),
            Err(e) => return Err(e),
        };
        let running = self.transition(id, OperationState::Running, Phase::Validation, None)?;
        if state::terminal(&running.state) {
            return Ok(running);
        }
        let result = if action {
            match tokio::time::timeout(
                Duration::from_secs(if network { 300 } else { 120 }),
                job(cancel.clone(), serialization.clone()),
            )
            .await
            {
                Ok(v) => v,
                Err(_) => Err(error(ErrorCode::Timeout, Phase::Recovery)),
            }
        } else {
            job(cancel.clone(), serialization.clone()).await
        };
        if action {
            // No scheduler/DB lock is held here. Invalidate while the common
            // lease is still owned and before finish_action emits a terminal.
            let effects = match &result {
                Ok(Completion::Action { effects, .. }) => *effects,
                _ => self.action_failure_effects(id),
            };
            if let Some(invalidate) = before_finish {
                invalidate(effects);
            }
            return match result {
                Ok(Completion::Action {
                    effects,
                    postcondition,
                }) => self
                    .finish_action(id, effects, postcondition, None)
                    .or_else(|e| self.action_failure(id, e)),
                Err(e) => self.action_failure(id, e),
                _ => self.action_failure(id, error(ErrorCode::InvalidRequest, Phase::Recovery)),
            };
        }
        match result {
            Ok(Completion::Action { .. }) => Err(invalid()),
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

fn action_capacity() -> AppError {
    AppError{code:ErrorCode::OutputLimit,phase:Phase::Queue,retry:AppErrorRetry::None,message:"Repository action history reached its safe retention limit. Reconcile unfinished or uncertain actions before starting another action.".into()}
}

#[cfg(test)]
mod action_retention_tests {
    use super::*;
    #[test]
    fn remote_read_checkpoint_is_not_a_write_and_registration_db_faults_prevent_native_start() {
        use crate::actions::journal::{self, SafeTargetProof, WritePhase};
        let root = tempfile::tempdir().unwrap();
        let (s, _, db) = make(root.path(), 8);
        let mut r = record(root.path());
        r.kind = ActionKind::ReadRemoteRefs;
        r.targets = vec![SafeTargetProof::Remote {
            name: "origin".into(),
            config_digest: "a".repeat(64),
        }];
        let op = s.enqueue_action(r.clone()).unwrap();
        s.transition(&op.id, OperationState::Running, Phase::Validation, None)
            .unwrap();
        assert!(s
            .action_read_start_checked(&op.id, RemoteEndpointRole::Push, |_| Err(invalid()))
            .is_err());
        assert!(s.action_before_write(&op.id).is_err());
        assert!(s
            .finish_action(&op.id, ActionEffects::Applied, None, None)
            .is_err());
        assert!(journal::read(&db, &op.id).unwrap().remote_read.is_none());
        db.lock().unwrap().execute_batch("CREATE TRIGGER read_fault BEFORE UPDATE ON repository_actions BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(s
            .action_read_start_checked(&op.id, RemoteEndpointRole::Push, |_| Ok(()))
            .is_err());
        assert!(!s.action_read_started(&op.id).unwrap());
        db.lock()
            .unwrap()
            .execute_batch("DROP TRIGGER read_fault;")
            .unwrap();
        assert!(s
            .action_read_start_checked(&op.id, RemoteEndpointRole::Push, |_| Ok(()))
            .unwrap());
        assert!(!s.action_write_started(&op.id).unwrap());
        assert!(s.action_read_started(&op.id).unwrap());
        assert_eq!(
            journal::read(&db, &op.id).unwrap().phase,
            WritePhase::Prepared
        );
        s.cancel(&op.id).unwrap();
        let unknown = s
            .action_failure(&op.id, error(ErrorCode::Interrupted, Phase::Recovery))
            .unwrap();
        assert_eq!(unknown.state, OperationState::Interrupted);
        assert_eq!(unknown.outcome.unwrap().effects, ActionEffects::Unknown);
        let second = s.enqueue_action(r).unwrap();
        s.transition(&second.id, OperationState::Running, Phase::Validation, None)
            .unwrap();
        assert!(s
            .action_read_start_checked(&second.id, RemoteEndpointRole::Fetch, |_| Ok(()))
            .unwrap());
        s.action_read_observed(&second.id, Some(0), Some("b".repeat(64)))
            .unwrap();
        s.cancel(&second.id).unwrap();
        let done = s
            .action_failure(&second.id, error(ErrorCode::Interrupted, Phase::Transfer))
            .unwrap();
        assert_eq!(done.state, OperationState::Cancelled);
        assert_eq!(done.outcome.unwrap().effects, ActionEffects::NotApplied);
        assert_eq!(
            journal::read(&db, &second.id)
                .unwrap()
                .remote_read
                .unwrap()
                .result_digest
                .as_deref(),
            Some("b".repeat(64).as_str())
        );
    }
    #[test]
    fn remote_read_restart_never_resumes_and_does_not_credit_started_as_no_effects() {
        use crate::actions::journal::{self, SafeTargetProof};
        let root = tempfile::tempdir().unwrap();
        let (s, _, db) = make(root.path(), 8);
        let mut r = record(root.path());
        r.kind = ActionKind::ReadRemoteRefs;
        r.targets = vec![SafeTargetProof::Remote {
            name: "origin".into(),
            config_digest: "a".repeat(64),
        }];
        let op = s.enqueue_action(r).unwrap();
        s.transition(&op.id, OperationState::Running, Phase::Validation, None)
            .unwrap();
        s.action_read_start_checked(&op.id, RemoteEndpointRole::Push, |_| Ok(()))
            .unwrap();
        let mut snapshot = s.get(&op.id).unwrap();
        crate::actions::recovery::reconcile(&db, &mut snapshot).unwrap();
        assert_eq!(snapshot.state, OperationState::Interrupted);
        assert_eq!(snapshot.outcome.unwrap().effects, ActionEffects::Unknown);
        assert!(
            !journal::read(&db, &op.id)
                .unwrap()
                .remote_read
                .unwrap()
                .no_git_writes
        );
    }
    #[test]
    fn fetch_step_ref_proof_and_started_phase_commit_with_the_final_registration_check() {
        use crate::actions::observations as fetch;
        let root = tempfile::tempdir().unwrap();
        let (scheduler, _, db) = make(root.path(), 4);
        let mut value = record(root.path());
        value.kind = ActionKind::Fetch;
        value
            .targets
            .push(crate::actions::journal::SafeTargetProof::Remote {
                name: "origin".into(),
                config_digest: "a".repeat(64),
            });
        let operation = scheduler.enqueue_action(value).unwrap();
        fetch::begin(
            &db,
            &operation.id,
            "repo",
            false,
            vec![fetch::RemoteProof {
                name: "origin".into(),
                config_digest: "a".repeat(64),
                refs_digest: "b".repeat(64),
            }],
        )
        .unwrap();
        scheduler
            .transition(
                &operation.id,
                OperationState::Running,
                Phase::Validation,
                None,
            )
            .unwrap();
        scheduler.action_before_write(&operation.id).unwrap();
        let before = scheduler.snapshot().unwrap();
        assert_eq!(
            scheduler
                .action_fetch_step_checked(&operation.id, 0, &"c".repeat(64), |_| Err(error(
                    ErrorCode::InvalidHandle,
                    Phase::Validation
                )))
                .unwrap_err()
                .code,
            ErrorCode::InvalidHandle
        );
        assert_eq!(scheduler.snapshot().unwrap(), before);
        {
            let c = db.lock().unwrap();
            let r = fetch::read_from(&c, &operation.id).unwrap().unwrap();
            assert_eq!(r.proofs[0].refs_digest, "b".repeat(64));
            assert_eq!(r.result.remotes[0].phase, FetchStepPhase::Pending);
        }
        assert!(scheduler
            .action_fetch_step_checked(&operation.id, 0, &"c".repeat(64), |_| Ok(()))
            .unwrap());
        let c = db.lock().unwrap();
        let r = fetch::read_from(&c, &operation.id).unwrap().unwrap();
        assert_eq!(r.proofs[0].refs_digest, "c".repeat(64));
        assert_eq!(r.result.remotes[0].phase, FetchStepPhase::Started);
    }

    use crate::actions::journal::{
        ActionRecord, ArtifactKind, ArtifactProof, ArtifactRoot, RecoveryLocation, WritePhase,
    };
    fn record(path: &std::path::Path) -> ActionRecord {
        ActionRecord {
            owner_window: "main".into(),
            kind: ActionKind::Commit,
            repo_id: "repo".into(),
            worktree_id: Some("tree".into()),
            common_identity: vec![1],
            root_identity: Some(vec![2]),
            precondition: "a".repeat(64),
            location: RecoveryLocation {
                common: crate::persistence::workspace::encode(path),
                git: crate::persistence::workspace::encode(path),
                worktree: Some(crate::persistence::workspace::encode(path)),
                git_identity: vec![3],
            },
            artifacts: vec![],
            targets: vec![],
            ref_action: None,
            change_action: None,
            sync_action: None,
            remote_read: None,
            phase: WritePhase::Prepared,
            postcondition: None,
            outcome: None,
        }
    }
    fn make(
        path: &std::path::Path,
        cap: usize,
    ) -> (
        Scheduler,
        Arc<Mutex<chrono::DateTime<chrono::Utc>>>,
        Arc<Database>,
    ) {
        let db = Arc::new(Database::open(&path.join("db")).unwrap());
        let now = Arc::new(Mutex::new(chrono::Utc::now()));
        let clock = now.clone();
        (
            Scheduler::with_action_retention(
                db.clone(),
                cap,
                Arc::new(move || *clock.lock().unwrap()),
            )
            .unwrap(),
            now,
            db,
        )
    }
    #[test]
    fn fetch_byte_pressure_compacts_only_safe_aged_unprotected_operations_and_cascades_atomically()
    {
        use crate::actions::observations as fetch;
        let directory = tempfile::tempdir().unwrap();
        let database = Arc::new(Database::open(&directory.path().join("db")).unwrap());
        let now = Arc::new(Mutex::new(chrono::Utc::now()));
        let time = now.clone();
        let scheduler = Scheduler::with_action_budgets(
            database.clone(),
            4096,
            fetch::ACTION_BYTES,
            Arc::new(move || *time.lock().unwrap()),
        )
        .unwrap();
        let mut value = record(directory.path());
        value.kind = ActionKind::Fetch;
        value
            .targets
            .push(crate::actions::journal::SafeTargetProof::Remote {
                name: "origin".into(),
                config_digest: "a".repeat(64),
            });
        let first = scheduler.enqueue_action(value.clone()).unwrap();
        fetch::begin(
            &database,
            &first.id,
            "repo",
            false,
            vec![fetch::RemoteProof {
                name: "origin".into(),
                config_digest: "a".repeat(64),
                refs_digest: "b".repeat(64),
            }],
        )
        .unwrap();
        scheduler
            .finish_action(&first.id, ActionEffects::NotApplied, None, None)
            .unwrap();
        let initial_weight = fetch::weight(&database.lock().unwrap(), &first.id).unwrap();
        assert!(initial_weight > 1024 && initial_weight < 4096);
        assert_eq!(
            scheduler.enqueue_action(value.clone()).unwrap_err().code,
            ErrorCode::OutputLimit
        );
        let clone = scheduler
            .enqueue(OperationSpec {
                source_display: "local".into(),
                destination_display: "selected".into(),
                reservation: None,
            })
            .unwrap();
        let unrelated = scheduler.enqueue_action(record(directory.path())).unwrap();
        *now.lock().unwrap() += chrono::Duration::seconds(901);
        let before = scheduler.snapshot().unwrap();
        assert_eq!(
            scheduler
                .enqueue_action_protected(value.clone(), &HashSet::from([first.id.clone()]))
                .unwrap_err()
                .code,
            ErrorCode::OutputLimit
        );
        database.lock().unwrap().execute_batch("CREATE TRIGGER block_fetch_compaction BEFORE DELETE ON operations BEGIN SELECT RAISE(ABORT,'owned'); END;").unwrap();
        assert_eq!(
            scheduler.enqueue_action(value.clone()).unwrap_err().code,
            ErrorCode::Database
        );
        assert_eq!(scheduler.snapshot().unwrap(), before);
        assert!(fetch::read(&database, &first.id).unwrap().is_some());
        assert_eq!(
            fetch::weight(&database.lock().unwrap(), &first.id).unwrap(),
            initial_weight
        );
        database
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER block_fetch_compaction;")
            .unwrap();
        let replacement = scheduler.enqueue_action(value.clone()).unwrap();
        assert_ne!(scheduler.snapshot().unwrap().stream_id, before.stream_id);
        assert!(scheduler.get(&first.id).is_err());
        assert!(fetch::read(&database, &first.id).unwrap().is_none());
        assert_eq!(
            fetch::used(&database.lock().unwrap()).unwrap(),
            fetch::ACTION_BYTES
        );
        assert!(scheduler.get(&clone.id).is_ok());
        assert!(scheduler.get(&unrelated.id).is_ok());
        scheduler
            .transition(
                &replacement.id,
                OperationState::Running,
                Phase::Validation,
                None,
            )
            .unwrap();
        scheduler.action_before_write(&replacement.id).unwrap();
        scheduler
            .action_failure(
                &replacement.id,
                error(ErrorCode::Interrupted, Phase::Recovery),
            )
            .unwrap();
        *now.lock().unwrap() += chrono::Duration::seconds(1000);
        assert_eq!(
            scheduler.enqueue_action(value).unwrap_err().code,
            ErrorCode::OutputLimit
        );
        assert_eq!(
            scheduler
                .get(&replacement.id)
                .unwrap()
                .outcome
                .unwrap()
                .effects,
            ActionEffects::Unknown
        );
        assert!(scheduler.enqueue_action(record(directory.path())).is_ok());
    }
    #[test]
    fn safe_compaction_is_atomic_rotates_stream_and_preserves_clone_and_live_replay() {
        let temp = tempfile::tempdir().unwrap();
        let (s, clock, db) = make(temp.path(), 1);
        let clone = s
            .enqueue(OperationSpec {
                source_display: "local".into(),
                destination_display: "Selected destination".into(),
                reservation: None,
            })
            .unwrap();
        let first = s.enqueue_action(record(temp.path())).unwrap();
        s.finish_action(&first.id, ActionEffects::NotApplied, None, None)
            .unwrap();
        // Age from the actual terminal observation, not fixture creation: a
        // slow setup must not consume the one-second retention-boundary margin.
        *clock.lock().unwrap() = chrono::DateTime::parse_from_rfc3339(
            &s.get(&first.id).unwrap().outcome.unwrap().observed_at,
        )
        .unwrap()
        .with_timezone(&chrono::Utc)
            + chrono::Duration::seconds(901);
        let before = s.snapshot().unwrap();
        assert_eq!(
            s.enqueue_action_protected(record(temp.path()), &HashSet::from([first.id.clone()]))
                .unwrap_err()
                .code,
            ErrorCode::OutputLimit
        );
        db.lock().unwrap().execute_batch("CREATE TRIGGER reject_history_delete BEFORE DELETE ON operations BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert_eq!(
            s.enqueue_action(record(temp.path())).unwrap_err().code,
            ErrorCode::Database
        );
        assert_eq!(s.snapshot().unwrap(), before);
        assert!(crate::actions::journal::read(&db, &first.id).is_ok());
        db.lock()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_history_delete")
            .unwrap();
        let replacement = s.enqueue_action(record(temp.path())).unwrap();
        assert_ne!(s.snapshot().unwrap().stream_id, before.stream_id);
        assert!(s.get(&first.id).is_err());
        assert!(s.get(&clone.id).is_ok());
        assert!(s.get(&replacement.id).is_ok());
        assert_eq!(
            db.lock()
                .unwrap()
                .query_row("SELECT count(*) FROM repository_actions", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn lost_journal_read_after_write_stays_unknown_even_after_cancel() {
        let temp = tempfile::tempdir().unwrap();
        let (s, _, db) = make(temp.path(), 2);
        let op = s.enqueue_action(record(temp.path())).unwrap();
        s.transition(&op.id, OperationState::Running, Phase::Validation, None)
            .unwrap();
        s.action_before_write(&op.id).unwrap();
        s.cancel(&op.id).unwrap();
        db.lock()
            .unwrap()
            .execute(
                "DELETE FROM repository_actions WHERE operation_id=?1",
                [&op.id],
            )
            .unwrap();
        let result = s
            .action_failure(&op.id, error(ErrorCode::Database, Phase::Recovery))
            .unwrap();
        assert_eq!(result.state, OperationState::Interrupted);
        assert_eq!(result.outcome.unwrap().effects, ActionEffects::Unknown);
        assert_eq!(result.error.unwrap().code, ErrorCode::Database);
    }
    #[test]
    fn rejected_registration_inside_write_transaction_never_publishes_write_started() {
        let temp = tempfile::tempdir().unwrap();
        let (s, _, db) = make(temp.path(), 2);
        let op = s.enqueue_action(record(temp.path())).unwrap();
        s.transition(&op.id, OperationState::Running, Phase::Validation, None)
            .unwrap();
        let before = s.snapshot().unwrap();
        let error = s
            .action_before_write_checked(&op.id, |_| {
                Err(crate::git::error::error(
                    ErrorCode::InvalidHandle,
                    Phase::Validation,
                ))
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidHandle);
        assert_eq!(s.snapshot().unwrap(), before);
        assert_eq!(
            crate::actions::journal::read(&db, &op.id).unwrap().phase,
            WritePhase::Prepared
        );
    }
    #[tokio::test]
    async fn s1_bq01_owned_network_permit_is_pure_and_retained_until_scope_disposal() {
        let temp = tempfile::tempdir().unwrap();
        let (s, _, _) = make(temp.path(), 2);
        let cancel = Cancellation::default();
        let keys = |n| {
            vec![LockKey {
                common: vec![n],
                worktree: None,
            }]
        };
        let first = s
            .acquire_scope(keys(1), true, &cancel, Some(Duration::from_millis(100)))
            .await
            .unwrap();
        let retained = first.clone();
        drop(first);
        let second = s
            .acquire_scope(keys(2), true, &cancel, Some(Duration::from_millis(100)))
            .await
            .unwrap();
        assert_eq!(
            s.acquire_scope(keys(3), true, &cancel, Some(Duration::from_millis(30)))
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::Timeout
        );
        let other = s
            .acquire_scope(keys(3), false, &cancel, Some(Duration::from_millis(100)))
            .await
            .unwrap();
        drop(other);
        drop(retained);
        let released = s
            .acquire_scope(keys(3), true, &cancel, Some(Duration::from_millis(100)))
            .await
            .unwrap();
        drop(released);
        drop(second);
    }
    #[test]
    fn active_conflict_partial_unknown_or_owned_artifacts_never_become_retention_victims() {
        for effects in [
            None,
            Some(ActionEffects::Conflict),
            Some(ActionEffects::Partial),
            Some(ActionEffects::Unknown),
            Some(ActionEffects::NotApplied),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let (s, clock, db) = make(temp.path(), 1);
            let first = s.enqueue_action(record(temp.path())).unwrap();
            if let Some(effects) = effects {
                if effects != ActionEffects::NotApplied {
                    s.transition(&first.id, OperationState::Running, Phase::Validation, None)
                        .unwrap();
                    s.action_before_write(&first.id).unwrap();
                }
                s.finish_action(&first.id, effects, None, None).unwrap();
                if effects == ActionEffects::NotApplied {
                    let mut r = crate::actions::journal::read(&db, &first.id).unwrap();
                    r.artifacts.push(ArtifactProof {
                        root: ArtifactRoot::Git,
                        kind: ArtifactKind::AlternateIndex,
                        name: format!("action-index-{}", "a".repeat(64)),
                        identity: vec![1],
                    });
                    crate::actions::journal::write_to(&db.lock().unwrap(), &first.id, &r).unwrap();
                }
            }
            *clock.lock().unwrap() += chrono::Duration::days(365);
            let before = s.snapshot().unwrap();
            let error = s.enqueue_action(record(temp.path())).unwrap_err();
            assert_eq!(error.code, ErrorCode::OutputLimit);
            assert!(error.message.contains("Reconcile"));
            assert_eq!(s.snapshot().unwrap(), before);
            assert!(crate::actions::journal::read(&db, &first.id).is_ok());
        }
    }
}
