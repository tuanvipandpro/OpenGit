//! Window-bound capabilities. Tokens are never durable, never reusable after eviction.
use super::{journal::ActionRecord, preflight::Preflight};
use crate::{
    domain::contracts::*, git::error::error, operations::scheduler::Scheduler,
    repositories::registry::RepositoryLocation,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
pub const WINDOWS: usize = 64;
pub const GRANTS: usize = 4096;
pub const PLANS: usize = 128;
pub const PLAN_WINDOW: usize = 32;
pub const USED_WINDOW: usize = 256;
pub const BYTES: usize = 32 * 1024 * 1024;
const GRANT_WEIGHT: usize = 4096;
const PLAN_WEIGHT: usize = 65536;
const USED_WEIGHT: usize = 512;
const WINDOW_WEIGHT: usize = 4096;
// Covers both accepted-ticket copies while the queued plan enters its runner.
const EXECUTION_TICKET_WEIGHT: usize = 2048;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    Commit,
    Branch,
    Tag,
    Remote,
    RemoteRef,
    RemoteEndpoint,
    RemoteDestination,
    Change,
    Selection,
    Stash,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub kind: TargetKind,
    pub identity: Vec<u8>,
    pub proof: Option<super::journal::SafeTargetProof>,
}
#[derive(Clone)]
struct Grant {
    scope: ActionScope,
    target: Target,
    stamp: String,
    expires: Instant,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Selection {
    repo_id: String,
    worktree_id: Option<String>,
}
impl Selection {
    fn of(scope: &ActionScope) -> Self {
        Self {
            repo_id: scope.repo_id.clone(),
            worktree_id: scope.worktree_id.clone(),
        }
    }
}
fn check_execution(
    w: &Window,
    window: &str,
    scope: &ActionScope,
    ticket: &ExecutionTicket,
) -> Result<(), AppError> {
    if ticket.window != window
        || ticket.incarnation != w.incarnation
        || ticket.era == 0
        || ticket.era != w.execution_era
        || ticket.selection != Selection::of(scope)
        || w.execution_selection.as_ref() != Some(&ticket.selection)
    {
        return Err(invalid());
    }
    Ok(())
}
/// Only atomic consume can construct this runtime-only authority. It is never
/// persisted, refreshed, or minted by a background runner.
#[derive(Clone)]
pub(crate) struct ExecutionTicket {
    window: String,
    incarnation: u64,
    selection: Selection,
    era: u64,
}
impl ExecutionTicket {
    #[cfg(test)]
    pub(crate) fn fixture(scope: &ActionScope) -> Self {
        Self {
            window: "main".into(),
            incarnation: 1,
            selection: Selection::of(scope),
            era: 1,
        }
    }
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.window.capacity()
            + self.selection.repo_id.capacity()
            + self
                .selection
                .worktree_id
                .as_ref()
                .map_or(0, String::capacity)
    }
}
#[derive(Clone)]
pub struct Plan {
    pub scope: ActionScope,
    pub intent: ActionIntent,
    pub location: RepositoryLocation,
    pub preflight: Preflight,
    pub targets: Vec<Target>,
    pub epoch: u64,
    pub(crate) execution_ticket: Option<ExecutionTicket>,
    pub ref_action: Option<super::journal::RefActionProof>,
    pub(crate) stash_action: Option<super::stash::StashProof>,
}
struct LivePlan {
    value: Plan,
    expires: Instant,
}
struct Used {
    token: String,
    operation: String,
    expires: Instant,
}
#[derive(Default)]
struct Window {
    read_cancel: crate::git::runner::Cancellation,
    epoch: u64,
    incarnation: u64,
    execution_era: u64,
    execution_selection: Option<Selection>,
    scope: Option<ActionScope>,
    grants: HashMap<String, Grant>,
    plans: HashMap<String, LivePlan>,
    used: VecDeque<Used>,
}
#[derive(Default)]
struct State {
    era: u64,
    windows: HashMap<String, Window>,
}
pub struct Authority {
    state: Mutex<State>,
    clock: Arc<dyn Fn() -> Instant + Send + Sync>,
}
impl Default for Authority {
    fn default() -> Self {
        Self::with_clock(Arc::new(Instant::now))
    }
}
impl Authority {
    pub fn with_clock(clock: Arc<dyn Fn() -> Instant + Send + Sync>) -> Self {
        Self {
            state: Default::default(),
            clock,
        }
    }
}
fn invalid() -> AppError {
    error(ErrorCode::InvalidHandle, Phase::Validation)
}
fn limit() -> AppError {
    error(ErrorCode::OutputLimit, Phase::Validation)
}
fn token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
fn clean(state: &mut State, now: Instant) {
    for window in state.windows.values_mut() {
        window.grants.retain(|_, v| v.expires > now);
        window.plans.retain(|_, v| v.expires > now);
        window.used.retain(|v| v.expires > now);
    }
}
fn counts(state: &State) -> (usize, usize, usize) {
    let grants = state.windows.values().map(|w| w.grants.len()).sum();
    let plans = state.windows.values().map(|w| w.plans.len()).sum();
    let used: usize = state.windows.values().map(|w| w.used.len()).sum();
    (grants, plans, used)
}
fn room(state: &State, weight: usize) -> bool {
    let (g, p, u) = counts(state);
    saturating_authority_weight(state, g, p, u, weight) <= BYTES
}
fn saturating_authority_weight(state: &State, g: usize, p: usize, u: usize, extra: usize) -> usize {
    state
        .windows
        .len()
        .saturating_mul(WINDOW_WEIGHT)
        .saturating_add(g.saturating_mul(GRANT_WEIGHT))
        .saturating_add(p.saturating_mul(PLAN_WEIGHT))
        .saturating_add(u.saturating_mul(USED_WEIGHT))
        .saturating_add(extra)
}
impl Authority {
    pub fn open_window(&self, window: &str) -> Result<(), AppError> {
        crate::ipc::validation::scope(window)?;
        let mut s = self.state.lock().map_err(|_| invalid())?;
        if !s.windows.contains_key(window)
            && (s.windows.len() >= WINDOWS || !room(&s, WINDOW_WEIGHT))
        {
            return Err(limit());
        }
        if !s.windows.contains_key(window) {
            let era = s.era.checked_add(1).ok_or_else(invalid)?;
            s.era = era;
            s.windows.insert(
                window.into(),
                Window {
                    epoch: era,
                    incarnation: era,
                    execution_era: era,
                    ..Default::default()
                },
            );
        }
        Ok(())
    }
    pub fn begin(
        &self,
        window: &str,
        scope: &ActionScope,
        check: impl FnOnce() -> Result<(), AppError>,
    ) -> Result<u64, AppError> {
        self.begin_inner(window, scope, None, check)
    }
    /// Validate the original accepted owner and snapshot before changing read
    /// scope, under the same lock as hard revocation and scope invalidation.
    pub(crate) fn begin_execution(
        &self,
        window: &str,
        scope: &ActionScope,
        ticket: &ExecutionTicket,
        check: impl FnOnce() -> Result<(), AppError>,
    ) -> Result<u64, AppError> {
        self.begin_inner(window, scope, Some(ticket), check)
    }
    fn begin_inner(
        &self,
        window: &str,
        scope: &ActionScope,
        ticket: Option<&ExecutionTicket>,
        check: impl FnOnce() -> Result<(), AppError>,
    ) -> Result<u64, AppError> {
        crate::ipc::validation::request_size(scope)?;
        let generation = crate::ipc::validation::counter(&scope.generation)?;
        if generation.to_string() != scope.generation
            || scope.repo_id.len() > 256
            || scope.worktree_id.as_ref().is_some_and(|s| s.len() > 256)
        {
            return Err(invalid());
        }
        let mut state = self.state.lock().map_err(|_| invalid())?;
        let w = state.windows.get(window).ok_or_else(invalid)?;
        if let Some(ticket) = ticket {
            check_execution(w, window, scope, ticket)?;
        }
        check()?; // Validate registration/selection before mutating authority scope.
        if let Some(previous) = &w.scope {
            if previous != scope
                && crate::ipc::validation::counter(&previous.generation)? >= generation
            {
                return Err(invalid());
            }
        }
        if w.scope.as_ref() == Some(scope) {
            return Ok(w.epoch);
        }
        let era = state.era.checked_add(1).ok_or_else(invalid)?;
        state.era = era;
        let w = state.windows.get_mut(window).ok_or_else(invalid)?;
        w.read_cancel.cancel();
        w.read_cancel = Default::default();
        w.epoch = era;
        let selection = Selection::of(scope);
        if w.execution_selection.as_ref() != Some(&selection) {
            w.execution_era = era;
            w.execution_selection = Some(selection);
        }
        w.scope = Some(scope.clone());
        w.grants.clear();
        w.plans.clear();
        Ok(era)
    }
    pub(crate) fn read_cancel(
        &self,
        window: &str,
        scope: &ActionScope,
        epoch: u64,
    ) -> Result<crate::git::runner::Cancellation, AppError> {
        let state = self.state.lock().map_err(|_| invalid())?;
        let w = state.windows.get(window).ok_or_else(invalid)?;
        if w.epoch != epoch || w.scope.as_ref() != Some(scope) {
            return Err(invalid());
        }
        Ok(w.read_cancel.clone())
    }
    pub(crate) fn read_epoch(&self, window: &str, scope: &ActionScope) -> Result<u64, AppError> {
        let state = self.state.lock().map_err(|_| invalid())?;
        let w = state.windows.get(window).ok_or_else(invalid)?;
        if w.scope.as_ref() != Some(scope) {
            return Err(invalid());
        }
        Ok(w.epoch)
    }
    pub fn checked<T>(
        &self,
        window: &str,
        scope: &ActionScope,
        epoch: u64,
        check: impl FnOnce() -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let s = self.state.lock().map_err(|_| invalid())?;
        let w = s.windows.get(window).ok_or_else(invalid)?;
        if w.epoch != epoch || w.scope.as_ref() != Some(scope) {
            return Err(invalid());
        }
        check()
    }
    pub(crate) fn checked_execution<T>(
        &self,
        window: &str,
        scope: &ActionScope,
        ticket: &ExecutionTicket,
        check: impl FnOnce() -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let state = self.state.lock().map_err(|_| invalid())?;
        let w = state.windows.get(window).ok_or_else(invalid)?;
        check_execution(w, window, scope, ticket)?;
        check()
    }
    pub fn issue(
        &self,
        window: &str,
        scope: &ActionScope,
        epoch: u64,
        target: Target,
        stamp: String,
        check: impl FnOnce() -> Result<(), AppError>,
    ) -> Result<String, AppError> {
        self.issue_until(window, scope, epoch, target, stamp, None, check)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn issue_until(
        &self,
        window: &str,
        scope: &ActionScope,
        epoch: u64,
        target: Target,
        stamp: String,
        expires: Option<Instant>,
        check: impl FnOnce() -> Result<(), AppError>,
    ) -> Result<String, AppError> {
        let deadline = expires.unwrap_or_else(|| (self.clock)() + Duration::from_secs(300));
        if deadline <= (self.clock)() || deadline > (self.clock)() + Duration::from_secs(300) {
            return Err(invalid());
        }
        if target.identity.is_empty()
            || target.identity.len() > 2048
            || target.identity.capacity()
                + target
                    .proof
                    .as_ref()
                    .map_or(0, super::journal::SafeTargetProof::heap_bytes)
                + 1024
                > GRANT_WEIGHT
            || stamp.len() != 64
        {
            return Err(limit());
        }
        if let Some(proof) = &target.proof {
            proof.validate()?;
            if serde_json::to_vec(proof).map_err(|_| invalid())?.len()
                + target.identity.len()
                + 1024
                > GRANT_WEIGHT
            {
                return Err(limit());
            }
        }
        let mut s = self.state.lock().map_err(|_| invalid())?;
        clean(&mut s, (self.clock)());
        if counts(&s).0 >= GRANTS || !room(&s, GRANT_WEIGHT) {
            return Err(limit());
        }
        let w = s.windows.get_mut(window).ok_or_else(invalid)?;
        if w.epoch != epoch || w.scope.as_ref() != Some(scope) {
            return Err(invalid());
        }
        check()?;
        let id = token();
        w.grants.insert(
            id.clone(),
            Grant {
                scope: scope.clone(),
                target,
                stamp,
                expires: deadline,
            },
        );
        Ok(id)
    }
    /// Catalog refresh reuses only identical, still-live grants; neither expiry
    /// nor an old fingerprint is resurrected. Validate the complete batch before
    /// publishing any token so capacity failure cannot leak a partial catalog.
    pub(crate) fn issue_catalog(
        &self,
        window: &str,
        scope: &ActionScope,
        epoch: u64,
        targets: Vec<Target>,
        stamp: &str,
        check: impl FnOnce() -> Result<(), AppError>,
    ) -> Result<Vec<String>, AppError> {
        if targets.len() > 64 || stamp.len() != 64 {
            return Err(limit());
        }
        for target in &targets {
            if !matches!(target.kind, TargetKind::Remote | TargetKind::RemoteEndpoint)
                || target.identity.is_empty()
                || target.identity.len() > 2048
                || target.identity.capacity()
                    + target
                        .proof
                        .as_ref()
                        .map_or(0, super::journal::SafeTargetProof::heap_bytes)
                    + 1024
                    > GRANT_WEIGHT
            {
                return Err(limit());
            }
            if let Some(proof) = &target.proof {
                proof.validate()?;
                if serde_json::to_vec(proof).map_err(|_| invalid())?.len()
                    + target.identity.len()
                    + 1024
                    > GRANT_WEIGHT
                {
                    return Err(limit());
                }
            } else {
                return Err(invalid());
            }
        }
        let now = (self.clock)();
        let mut state = self.state.lock().map_err(|_| invalid())?;
        clean(&mut state, now);
        let w = state.windows.get(window).ok_or_else(invalid)?;
        if w.epoch != epoch || w.scope.as_ref() != Some(scope) {
            return Err(invalid());
        }
        check()?;
        let existing = targets
            .iter()
            .map(|target| {
                w.grants
                    .iter()
                    .find(|(_, g)| {
                        g.scope == *scope
                            && g.target == *target
                            && g.stamp == stamp
                            && g.expires > now
                    })
                    .map(|(id, _)| id.clone())
            })
            .collect::<Vec<_>>();
        let needed = existing.iter().filter(|v| v.is_none()).count();
        if counts(&state).0 + needed > GRANTS || !room(&state, needed * GRANT_WEIGHT) {
            return Err(limit());
        }
        let w = state.windows.get_mut(window).ok_or_else(invalid)?;
        let mut result = Vec::new();
        for (target, existing) in targets.into_iter().zip(existing) {
            let id = if let Some(id) = existing {
                id
            } else {
                let id = token();
                w.grants.insert(
                    id.clone(),
                    Grant {
                        scope: scope.clone(),
                        target,
                        stamp: stamp.into(),
                        expires: now + Duration::from_secs(300),
                    },
                );
                id
            };
            result.push(id);
        }
        Ok(result)
    }
    pub(crate) fn plan_kind(
        &self,
        window: &str,
        token: &str,
        scheduler: &Scheduler,
    ) -> Result<ActionKind, AppError> {
        let mut state = self.state.lock().map_err(|_| invalid())?;
        clean(&mut state, (self.clock)());
        let w = state.windows.get(window).ok_or_else(invalid)?;
        if let Some(used) = w.used.iter().find(|u| u.token == token) {
            return scheduler
                .get(&used.operation)?
                .action_kind
                .ok_or_else(invalid);
        }
        w.plans
            .get(token)
            .map(|p| p.value.intent.kind())
            .ok_or_else(invalid)
    }
    pub(crate) fn resolved_targets(
        &self,
        window: &str,
        scope: &ActionScope,
        epoch: u64,
        intent: &ActionIntent,
        stamp: &str,
    ) -> Result<Vec<Target>, AppError> {
        let state = self.state.lock().map_err(|_| invalid())?;
        let w = state.windows.get(window).ok_or_else(invalid)?;
        if w.epoch != epoch || w.scope.as_ref() != Some(scope) {
            return Err(invalid());
        }
        resolve(w, scope, intent, stamp, (self.clock)())
    }
    pub(crate) fn change_grant(
        &self,
        window: &str,
        scope: &ActionScope,
        epoch: u64,
        id: &str,
        stamp: &str,
    ) -> Result<(Target, Instant), AppError> {
        let state = self.state.lock().map_err(|_| invalid())?;
        let w = state.windows.get(window).ok_or_else(invalid)?;
        let g = w.grants.get(id).ok_or_else(invalid)?;
        if w.epoch != epoch
            || w.scope.as_ref() != Some(scope)
            || g.scope != *scope
            || g.target.kind != TargetKind::Change
            || g.stamp != stamp
            || g.expires <= (self.clock)()
        {
            return Err(invalid());
        }
        Ok((g.target.clone(), g.expires))
    }
    pub fn prepare(
        &self,
        window: &str,
        mut plan: Plan,
        check: impl FnOnce() -> Result<(), AppError>,
    ) -> Result<PreparedAction, AppError> {
        validate(&plan.intent)?;
        plan.execution_ticket = None;
        let mut s = self.state.lock().map_err(|_| invalid())?;
        let now = (self.clock)();
        clean(&mut s, now);
        if counts(&s).1 >= PLANS || !room(&s, PLAN_WEIGHT) {
            return Err(limit());
        }
        let w = s.windows.get_mut(window).ok_or_else(invalid)?;
        if w.epoch != plan.epoch || w.scope.as_ref() != Some(&plan.scope) {
            return Err(invalid());
        }
        check()?;
        if w.plans.len() >= PLAN_WINDOW {
            return Err(limit());
        }
        plan.targets = resolve(
            w,
            &plan.scope,
            &plan.intent,
            &plan.preflight.fingerprint,
            now,
        )?;
        // Grants and plans retain separate copies. Charge every cloned proof,
        // allocation capacity and target container, not just opaque identities.
        let proof_bytes = plan
            .targets
            .iter()
            .map(|t| {
                let wire = t
                    .proof
                    .as_ref()
                    .map(serde_json::to_vec)
                    .transpose()
                    .map_err(|_| invalid())?
                    .map_or(0, |v| v.len());
                Ok(t.identity.capacity()
                    + wire.max(
                        t.proof
                            .as_ref()
                            .map_or(0, super::journal::SafeTargetProof::heap_bytes),
                    ))
            })
            .collect::<Result<Vec<usize>, AppError>>()?
            .into_iter()
            .sum::<usize>();
        let size = serde_json::to_vec(&plan.intent)
            .map_err(|_| invalid())?
            .len()
            + plan.targets.capacity() * std::mem::size_of::<Target>()
            + proof_bytes
            + super::location_bytes(&plan.location)
            + plan.location.common.capacity() * 2
            + plan.location.root.capacity() * 2
            + plan.location.git_dir.capacity() * 2
            + plan.ref_action.as_ref().map_or(0, |p| {
                p.bytes()
                    .max(serde_json::to_vec(p).map_or(usize::MAX / 2, |v| v.len()))
            })
            + plan.stash_action.as_ref().map_or(0, |p| {
                4096usize.max(serde_json::to_vec(p).map_or(usize::MAX / 2, |v| v.len()))
            })
            + EXECUTION_TICKET_WEIGHT
            + 4096;
        if size > PLAN_WEIGHT {
            return Err(limit());
        }
        let expires = handles(&plan.intent)
            .iter()
            .filter_map(|(id, _)| w.grants.get(*id).map(|g| g.expires))
            .min()
            .unwrap_or(now + Duration::from_secs(60))
            .min(now + Duration::from_secs(60));
        let id = token();
        let reply = PreparedAction {
            scope: plan.scope.clone(),
            plan_id: id.clone(),
            kind: plan.intent.kind(),
            expires_at: (chrono::Utc::now()
                + chrono::Duration::from_std(expires.duration_since(now))
                    .map_err(|_| invalid())?)
            .to_rfc3339(),
            warnings: vec![ActionWarning::ExternalRace],
        };
        w.plans.insert(
            id,
            LivePlan {
                value: plan,
                expires,
            },
        );
        Ok(reply)
    }
    /// Enqueue and consume under one lock; a DB failure leaves the plan unused.
    pub fn consume(
        &self,
        window: &str,
        id: &str,
        scheduler: &Scheduler,
        check: impl FnOnce(&Plan) -> Result<(), AppError>,
    ) -> Result<(OperationSnapshot, Option<Plan>), AppError> {
        let mut s = self.state.lock().map_err(|_| invalid())?;
        let now = (self.clock)();
        clean(&mut s, now);
        let w = s.windows.get(window).ok_or_else(invalid)?;
        if let Some(used) = w.used.iter().find(|v| v.token == id) {
            return Ok((scheduler.get(&used.operation)?, None));
        }
        // Consuming removes a 64KiB plan and adds at most a 512B tombstone.
        let protected = s
            .windows
            .values()
            .flat_map(|w| w.used.iter().map(|u| u.operation.clone()))
            .collect::<std::collections::HashSet<_>>();
        let w = s.windows.get_mut(window).ok_or_else(invalid)?;
        let plan = &w.plans.get(id).ok_or_else(invalid)?.value;
        if w.epoch != plan.epoch || w.scope.as_ref() != Some(&plan.scope) {
            return Err(invalid());
        }
        check(plan)?;
        let record = ActionRecord {
            owner_window: window.into(),
            kind: plan.intent.kind(),
            repo_id: plan.scope.repo_id.clone(),
            worktree_id: plan.scope.worktree_id.clone(),
            common_identity: plan.location.common_identity.clone(),
            root_identity: plan.location.root_identity.clone(),
            precondition: plan.preflight.fingerprint.clone(),
            location: super::journal::RecoveryLocation {
                common: crate::persistence::workspace::encode(&plan.location.common),
                git: crate::persistence::workspace::encode(&plan.location.git_dir),
                worktree: (!plan.location.bare)
                    .then(|| crate::persistence::workspace::encode(&plan.location.root)),
                git_identity: crate::repositories::discovery::identity(&plan.location.git_dir)?,
            },
            artifacts: vec![],
            ref_action: plan.ref_action.clone(),
            remote_read: None,
            change_action: None,
            sync_action: None,
            stash_action: plan.stash_action.clone(),
            targets: plan
                .targets
                .iter()
                .filter_map(|t| t.proof.clone())
                .collect(),
            phase: super::journal::WritePhase::Prepared,
            postcondition: None,
            outcome: None,
        };
        // Reserve all bounded local-step checkpoint/final-outcome framing before
        // enqueue. A later proof update must not discover an avoidable32KiB cap.
        let reserve = if super::sync::supported(record.kind) {
            16384
        } else if super::changes::supported(record.kind) || super::stash::supported(record.kind) {
            8192
        } else {
            0
        };
        if reserve > 0
            && serde_json::to_vec(&record).map_err(|_| invalid())?.len() > 32768 - reserve
        {
            return Err(limit());
        }
        if record.ref_action.is_some() {
            let mut maximum = record.clone();
            maximum.phase = super::journal::WritePhase::Observed;
            maximum.postcondition = Some("a".repeat(64));
            maximum.outcome = Some(super::journal::outcome(ActionEffects::NotApplied, true));
            if let Some(proof) = &mut maximum.ref_action {
                for step in &mut proof.steps {
                    step.phase = FetchStepPhase::Observed;
                    step.effects = ActionEffects::NotApplied;
                    step.failure_reason = Some(RefActionFailureReason::Identity);
                    step.before_fingerprint = Some("a".repeat(64));
                    step.after_fingerprint = Some("a".repeat(64));
                }
            }
            if serde_json::to_vec(&maximum).map_err(|_| invalid())?.len() > 32768 {
                return Err(limit());
            }
        }
        let selection = Selection::of(&plan.scope);
        if w.execution_selection.as_ref() != Some(&selection) || w.execution_era == 0 {
            return Err(invalid());
        }
        let ticket = ExecutionTicket {
            window: window.into(),
            incarnation: w.incarnation,
            selection,
            era: w.execution_era,
        };
        if ticket.bytes() > EXECUTION_TICKET_WEIGHT / 2 {
            return Err(limit());
        }
        let operation = scheduler.enqueue_action_protected(record, &protected)?;
        let mut plan = w.plans.remove(id).ok_or_else(invalid)?.value;
        // This is the sole minting point, under the same authority lock as the
        // successful enqueue/registration transaction and one-use consumption.
        plan.execution_ticket = Some(ticket);
        if w.used.len() >= USED_WINDOW {
            w.used.pop_front();
        }
        w.used.push_back(Used {
            token: id.into(),
            operation: operation.id.clone(),
            expires: now + Duration::from_secs(900),
        });
        Ok((operation, Some(plan)))
    }
    pub fn revoke(&self, window: &str, close: bool) {
        if let Ok(mut state) = self.state.lock() {
            if !state.windows.contains_key(window) {
                return;
            }
            // Exhaustion must still revoke. Epoch zero is never issued, and no
            // later open/scope allocation can succeed once the global era is MAX.
            let era = state.era.checked_add(1);
            if let Some(era) = era {
                state.era = era;
            }
            if close {
                if let Some(w) = state.windows.remove(window) {
                    w.read_cancel.cancel();
                }
            } else if let Some(w) = state.windows.get_mut(window) {
                w.read_cancel.cancel();
                w.epoch = era.unwrap_or(0);
                w.execution_era = era.unwrap_or(0);
                w.execution_selection = None;
                w.scope = None;
                w.grants.clear();
                w.plans.clear();
            }
        }
    }
    pub fn revoke_repo(&self, repo: &str) {
        if let Ok(mut state) = self.state.lock() {
            let affected = state
                .windows
                .iter()
                .filter(|(_, w)| w.scope.as_ref().is_some_and(|scope| scope.repo_id == repo))
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            for name in affected {
                let era = state.era.checked_add(1);
                if let Some(era) = era {
                    state.era = era;
                }
                if let Some(w) = state.windows.get_mut(&name) {
                    w.read_cancel.cancel();
                    w.epoch = era.unwrap_or(0);
                    w.execution_era = era.unwrap_or(0);
                    w.execution_selection = None;
                    w.scope = None;
                    w.grants.clear();
                    w.plans.clear();
                }
            }
        }
    }
}
fn handles(intent: &ActionIntent) -> Vec<(&str, Vec<TargetKind>)> {
    use TargetKind::*;
    match intent {
        ActionIntent::ReadRemoteRefs { remote_handle } => {
            vec![(remote_handle, vec![RemoteEndpoint])]
        }
        ActionIntent::Fetch { remote_handles, .. } => remote_handles
            .iter()
            .map(|s| (s.as_str(), vec![Remote]))
            .collect(),
        ActionIntent::BranchCreate { start_handle, .. }
        | ActionIntent::TagCreate {
            target_handle: start_handle,
            ..
        } => vec![(start_handle, vec![Commit, Branch, Tag, RemoteRef])],
        ActionIntent::BranchRename { branch_handle, .. }
        | ActionIntent::BranchSwitch { branch_handle }
        | ActionIntent::BranchDelete { branch_handle } => vec![(branch_handle, vec![Branch])],
        ActionIntent::BranchTracking {
            branch_handle,
            upstream_handle,
        } => {
            let mut v = vec![(branch_handle.as_str(), vec![Branch])];
            if let Some(s) = upstream_handle {
                v.push((s, vec![RemoteRef]));
            }
            v
        }
        ActionIntent::TagDelete { tag_handle } => vec![(tag_handle, vec![Tag])],
        ActionIntent::Push {
            remote_handle,
            source_handle,
            destination_handle,
            ..
        } => vec![
            (remote_handle, vec![RemoteEndpoint]),
            (source_handle, vec![Branch]),
            (destination_handle, vec![RemoteDestination]),
        ],
        ActionIntent::RemoteTagPush {
            remote_handle,
            tag_handle,
            destination_handle,
        } => vec![
            (remote_handle, vec![RemoteEndpoint]),
            (tag_handle, vec![Tag]),
            (destination_handle, vec![RemoteDestination]),
        ],
        ActionIntent::RemoteTagDelete {
            remote_handle,
            tag_handle,
        } => vec![
            (remote_handle, vec![RemoteEndpoint]),
            (tag_handle, vec![RemoteDestination]),
        ],
        ActionIntent::Pull { upstream_handle } => vec![(upstream_handle, vec![RemoteRef])],
        ActionIntent::Stage { change_handles } | ActionIntent::Unstage { change_handles } => {
            change_handles
                .iter()
                .map(|s| (s.as_str(), vec![Change]))
                .collect()
        }
        ActionIntent::StageSelection { selection_handles }
        | ActionIntent::UnstageSelection { selection_handles } => selection_handles
            .iter()
            .map(|s| (s.as_str(), vec![Selection]))
            .collect(),
        ActionIntent::StashApply { stash_handle }
        | ActionIntent::StashPop { stash_handle }
        | ActionIntent::StashDrop { stash_handle } => vec![(stash_handle, vec![Stash])],
        _ => vec![],
    }
}
fn resolve(
    w: &Window,
    scope: &ActionScope,
    intent: &ActionIntent,
    stamp: &str,
    now: Instant,
) -> Result<Vec<Target>, AppError> {
    handles(intent)
        .into_iter()
        .map(|(id, kinds)| {
            let grant = w.grants.get(id).ok_or_else(invalid)?;
            if grant.expires <= now
                || &grant.scope != scope
                || grant.stamp != stamp
                || !kinds.contains(&grant.target.kind)
            {
                return Err(invalid());
            }
            Ok(grant.target.clone())
        })
        .collect()
}
pub fn validate(intent: &ActionIntent) -> Result<(), AppError> {
    crate::ipc::validation::request_size(intent)?;
    let refs = handles(intent);
    let unique = refs
        .iter()
        .map(|(s, _)| *s)
        .collect::<std::collections::HashSet<_>>();
    if unique.len() != refs.len() {
        return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
    }
    if refs.len() > 256
        || refs
            .iter()
            .any(|(s, _)| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(invalid());
    }
    if matches!(
        intent,
        ActionIntent::Fetch { .. }
            | ActionIntent::Stage { .. }
            | ActionIntent::Unstage { .. }
            | ActionIntent::StageSelection { .. }
            | ActionIntent::UnstageSelection { .. }
    ) && refs.is_empty()
    {
        return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
    }
    let text = |s: &str, empty: bool| {
        if s.len() > 8192 || s.contains('\0') || !empty && s.trim().is_empty() {
            Err(error(ErrorCode::InvalidRequest, Phase::Validation))
        } else {
            Ok(())
        }
    };
    let name = |s: &str| {
        if s.len() > 1024
            || s.starts_with('-')
            || !crate::history::refs::valid_name(&format!("refs/heads/{s}"))
        {
            Err(error(ErrorCode::InvalidRequest, Phase::Validation))
        } else {
            Ok(())
        }
    };
    match intent {
        ActionIntent::BranchCreate { name: n, .. }
        | ActionIntent::BranchRename { new_name: n, .. }
        | ActionIntent::TagCreate { name: n, .. } => name(n)?,
        _ => (),
    };
    match intent {
        ActionIntent::Commit { message, .. } => text(message, false)?,
        ActionIntent::StashPush { message, .. } => text(message, true)?,
        ActionIntent::TagCreate {
            annotation: TagAnnotation::Annotated { message, .. },
            ..
        } => text(message, false)?,
        _ => (),
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_derived_grant_cannot_outlive_original_snapshot_deadline() {
        let now = Arc::new(Mutex::new(Instant::now()));
        let clock = now.clone();
        let a = Authority::with_clock(Arc::new(move || *clock.lock().unwrap()));
        a.open_window("main").unwrap();
        let scope = scope();
        let era = a.begin("main", &scope, || Ok(())).unwrap();
        let target = Target {
            kind: TargetKind::RemoteEndpoint,
            identity: b"push:origin".to_vec(),
            proof: Some(super::super::journal::SafeTargetProof::Remote {
                name: "origin".into(),
                config_digest: "a".repeat(64),
            }),
        };
        let deadline = *now.lock().unwrap() + Duration::from_secs(60);
        let id = a
            .issue_until(
                "main",
                &scope,
                era,
                target.clone(),
                "b".repeat(64),
                Some(deadline),
                || Ok(()),
            )
            .unwrap();
        *now.lock().unwrap() += Duration::from_secs(59);
        assert!(a
            .resolved_targets(
                "main",
                &scope,
                era,
                &ActionIntent::ReadRemoteRefs {
                    remote_handle: id.clone()
                },
                &"b".repeat(64)
            )
            .is_ok());
        *now.lock().unwrap() += Duration::from_secs(1);
        assert!(a
            .resolved_targets(
                "main",
                &scope,
                era,
                &ActionIntent::ReadRemoteRefs { remote_handle: id },
                &"b".repeat(64)
            )
            .is_err());
        assert!(a
            .issue_until(
                "main",
                &scope,
                era,
                target,
                "b".repeat(64),
                Some(deadline),
                || Ok(())
            )
            .is_err());
    }
    #[test]
    fn closed_and_reopened_window_cannot_accept_a_pre_close_catalog_ticket() {
        let authority = Authority::default();
        authority.open_window("main").unwrap();
        let scope = scope();
        let old = authority.begin("main", &scope, || Ok(())).unwrap();
        authority.revoke("main", true);
        authority.open_window("main").unwrap();
        let current = authority.begin("main", &scope, || Ok(())).unwrap();
        assert_ne!(current, old);
        assert_eq!(authority.begin("main", &scope, || Ok(())).unwrap(), current);
        let temporary = tempfile::tempdir().unwrap();
        assert_eq!(
            authority
                .prepare(
                    "main",
                    plan(
                        scope.clone(),
                        old,
                        temporary.path(),
                        ActionIntent::Commit {
                            message: "fixture".into(),
                            amend: false
                        }
                    ),
                    || Ok(())
                )
                .unwrap_err()
                .code,
            ErrorCode::InvalidHandle
        );
        assert!(authority.checked("main", &scope, old, || Ok(())).is_err());
        assert!(authority
            .prepare(
                "main",
                plan(
                    scope.clone(),
                    current,
                    temporary.path(),
                    ActionIntent::Commit {
                        message: "fixture".into(),
                        amend: false
                    }
                ),
                || Ok(())
            )
            .is_ok());
        let result = authority.issue_catalog(
            "main",
            &scope,
            old,
            vec![Target {
                kind: TargetKind::Remote,
                identity: b"origin".to_vec(),
                proof: Some(super::super::journal::SafeTargetProof::Remote {
                    name: "origin".into(),
                    config_digest: "a".repeat(64),
                }),
            }],
            &"b".repeat(64),
            || Ok(()),
        );
        assert!(
            result.is_err(),
            "a pre-close read ticket must not publish fresh authority after window recreation"
        );
    }
    #[test]
    fn era_overflow_refuses_new_authority_but_close_and_revoke_never_resurrect_old_tickets() {
        let authority = Authority::default();
        authority.open_window("main").unwrap();
        let scope = scope();
        let ticket = authority.begin("main", &scope, || Ok(())).unwrap();
        authority.state.lock().unwrap().era = u64::MAX;
        assert_eq!(authority.begin("main", &scope, || Ok(())).unwrap(), ticket);
        authority.open_window("main").unwrap();
        assert!(authority.open_window("other").is_err());
        let mut newer = scope.clone();
        newer.generation = "2".into();
        assert!(authority.begin("main", &newer, || Ok(())).is_err());
        authority.revoke("main", false);
        assert!(authority
            .checked("main", &scope, ticket, || Ok(()))
            .is_err());
        assert!(authority.begin("main", &scope, || Ok(())).is_err());
        authority.revoke("main", true);
        assert!(authority.open_window("main").is_err());
        assert!(authority
            .checked("main", &scope, ticket, || Ok(()))
            .is_err());
        assert_eq!(authority.state.lock().unwrap().era, u64::MAX);
    }
    use std::sync::{Arc, Mutex};
    fn scope() -> ActionScope {
        ActionScope {
            repo_id: "repo".into(),
            worktree_id: Some("tree".into()),
            generation: "1".into(),
        }
    }
    fn plan(scope: ActionScope, epoch: u64, path: &std::path::Path, intent: ActionIntent) -> Plan {
        Plan {
            scope,
            epoch,
            intent,
            location: RepositoryLocation {
                common: path.into(),
                common_identity: vec![1],
                root: path.into(),
                root_identity: Some(vec![2]),
                git_dir: path.into(),
                bare: false,
            },
            preflight: Preflight {
                fingerprint: "a".repeat(64),
                bare: false,
                dirty: false,
                unmerged: false,
                busy: false,
            },
            targets: vec![],
            ref_action: None,
            stash_action: None,
            execution_ticket: None,
        }
    }
    #[test]
    fn ttl_caps_and_one_use_tombstone_expiry_never_allow_reexecution() {
        let temp = tempfile::tempdir().unwrap();
        let now = Arc::new(Mutex::new(Instant::now()));
        let clock = now.clone();
        let authority = Authority::with_clock(Arc::new(move || *clock.lock().unwrap()));
        authority.open_window("main").unwrap();
        let s = scope();
        let epoch = authority.begin("main", &s, || Ok(())).unwrap();
        let target = authority
            .issue(
                "main",
                &s,
                epoch,
                Target {
                    kind: TargetKind::Branch,
                    identity: b"refs/heads/main".to_vec(),
                    proof: None,
                },
                "a".repeat(64),
                || Ok(()),
            )
            .unwrap();
        *now.lock().unwrap() += Duration::from_secs(299);
        let p = authority
            .prepare(
                "main",
                plan(
                    s.clone(),
                    epoch,
                    temp.path(),
                    ActionIntent::BranchSwitch {
                        branch_handle: target.clone(),
                    },
                ),
                || Ok(()),
            )
            .unwrap();
        *now.lock().unwrap() += Duration::from_secs(2);
        let db = Arc::new(
            crate::persistence::workspace::Database::open(&temp.path().join("db")).unwrap(),
        );
        let scheduler = Scheduler::new(db).unwrap();
        assert_eq!(
            authority
                .consume("main", &p.plan_id, &scheduler, |_| Ok(()))
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidHandle
        );
        assert_eq!(
            authority
                .prepare(
                    "main",
                    plan(
                        s.clone(),
                        epoch,
                        temp.path(),
                        ActionIntent::BranchSwitch {
                            branch_handle: target
                        }
                    ),
                    || Ok(())
                )
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidHandle
        );
        let p = authority
            .prepare(
                "main",
                plan(
                    s.clone(),
                    epoch,
                    temp.path(),
                    ActionIntent::Commit {
                        message: "message".into(),
                        amend: false,
                    },
                ),
                || Ok(()),
            )
            .unwrap();
        let (op, _) = authority
            .consume("main", &p.plan_id, &scheduler, |_| Ok(()))
            .unwrap();
        assert_eq!(
            authority
                .consume("main", &p.plan_id, &scheduler, |_| panic!(
                    "replay revalidated or executed"
                ))
                .unwrap()
                .0
                .id,
            op.id
        );
        *now.lock().unwrap() += Duration::from_secs(901);
        assert_eq!(
            authority
                .consume("main", &p.plan_id, &scheduler, |_| panic!(
                    "expired replay ran"
                ))
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidHandle
        );
    }
    #[test]
    fn plan_and_payload_caps_reject_whole_requests_and_window_disposal_releases_unused_authority() {
        let temp = tempfile::tempdir().unwrap();
        let authority = Authority::default();
        let s = scope();
        for n in 0..4 {
            let window = format!("window-{n}");
            authority.open_window(&window).unwrap();
            let epoch = authority.begin(&window, &s, || Ok(())).unwrap();
            for _ in 0..PLAN_WINDOW {
                authority
                    .prepare(
                        &window,
                        plan(
                            s.clone(),
                            epoch,
                            temp.path(),
                            ActionIntent::Commit {
                                message: "message".into(),
                                amend: false,
                            },
                        ),
                        || Ok(()),
                    )
                    .unwrap();
            }
            assert_eq!(
                authority
                    .prepare(
                        &window,
                        plan(
                            s.clone(),
                            epoch,
                            temp.path(),
                            ActionIntent::Commit {
                                message: "message".into(),
                                amend: false
                            }
                        ),
                        || Ok(())
                    )
                    .unwrap_err()
                    .code,
                ErrorCode::OutputLimit
            );
        }
        authority.open_window("next").unwrap();
        let epoch = authority.begin("next", &s, || Ok(())).unwrap();
        assert_eq!(
            authority
                .prepare(
                    "next",
                    plan(
                        s.clone(),
                        epoch,
                        temp.path(),
                        ActionIntent::Commit {
                            message: "message".into(),
                            amend: false
                        }
                    ),
                    || Ok(())
                )
                .unwrap_err()
                .code,
            ErrorCode::OutputLimit
        );
        authority.revoke("window-0", true);
        authority
            .prepare(
                "next",
                plan(
                    s.clone(),
                    epoch,
                    temp.path(),
                    ActionIntent::Commit {
                        message: "message".into(),
                        amend: false,
                    },
                ),
                || Ok(()),
            )
            .unwrap();
        assert!(validate(&ActionIntent::Commit {
            message: "x".repeat(8193),
            amend: false
        })
        .is_err());
        assert!(validate(&ActionIntent::Stage {
            change_handles: vec!["a".repeat(64); 257]
        })
        .is_err());
        assert!(validate(&ActionIntent::BranchCreate {
            name: "--option".into(),
            start_handle: "a".repeat(64),
            checkout: false
        })
        .is_err());
        let a = super::super::Actions::default();
        let slots = (0..64).map(|_| a.reserve().unwrap()).collect::<Vec<_>>();
        assert!(a.reserve().is_err());
        drop(slots);
        assert!(a.reserve().is_ok());
    }
    #[test]
    fn s1_a01_large_cloned_safe_proofs_are_counted_and_rejected_atomically() {
        let temp = tempfile::tempdir().unwrap();
        let a = Authority::default();
        a.open_window("main").unwrap();
        let s = scope();
        let epoch = a.begin("main", &s, || Ok(())).unwrap();
        let mut handles = Vec::new();
        for n in 0..128 {
            handles.push(
                a.issue(
                    "main",
                    &s,
                    epoch,
                    Target {
                        kind: TargetKind::Remote,
                        identity: vec![n],
                        proof: Some(super::super::journal::SafeTargetProof::Remote {
                            name: format!("{}{:04}", "r".repeat(996), n),
                            config_digest: "b".repeat(64),
                        }),
                    },
                    "a".repeat(64),
                    || Ok(()),
                )
                .unwrap(),
            );
        }
        let intent = ActionIntent::Fetch {
            remote_handles: handles,
            prune: false,
        };
        let request = PrepareRepositoryActionRequest {
            scope: s.clone(),
            intent: intent.clone(),
        };
        assert!(serde_json::to_vec(&request).unwrap().len() <= 16384);
        assert_eq!(
            a.prepare("main", plan(s.clone(), epoch, temp.path(), intent), || Ok(
                ()
            ))
            .unwrap_err()
            .code,
            ErrorCode::OutputLimit
        );
        for _ in 0..PLAN_WINDOW {
            a.prepare(
                "main",
                plan(
                    s.clone(),
                    epoch,
                    temp.path(),
                    ActionIntent::Commit {
                        message: "message".into(),
                        amend: false,
                    },
                ),
                || Ok(()),
            )
            .unwrap();
        }
    }
    #[test]
    fn rejected_scope_begin_does_not_poison_the_next_legitimate_generation() {
        let authority = Authority::default();
        authority.open_window("main").unwrap();
        let s = scope();
        let first = authority.begin("main", &s, || Ok(())).unwrap();
        let mut forged = s.clone();
        forged.repo_id = "stale".into();
        forged.generation = "9".into();
        assert!(authority.begin("main", &forged, || Err(invalid())).is_err());
        assert_eq!(authority.begin("main", &s, || Ok(())).unwrap(), first);
        authority.revoke("main", false);
        assert!(authority.checked("main", &s, first, || Ok(())).is_err());
    }
    #[test]
    fn accepted_ticket_is_consume_only_generation_independent_and_hard_revoke_never_revives_it() {
        let directory = tempfile::tempdir().unwrap();
        let database =
            Arc::new(crate::persistence::workspace::Database::open(directory.path()).unwrap());
        let scheduler = Scheduler::new(database).unwrap();
        let time = Arc::new(Mutex::new(Instant::now()));
        let clock = time.clone();
        let authority = Authority::with_clock(Arc::new(move || *clock.lock().unwrap()));
        authority.open_window("main").unwrap();
        let mut scope = scope();
        let epoch = authority.begin("main", &scope, || Ok(())).unwrap();
        let intent = ActionIntent::Commit {
            message: "runtime-only".into(),
            amend: false,
        };
        let prepared = authority
            .prepare(
                "main",
                plan(scope.clone(), epoch, directory.path(), intent.clone()),
                || Ok(()),
            )
            .unwrap();
        assert!(
            authority.state.lock().unwrap().windows["main"].plans[&prepared.plan_id]
                .value
                .execution_ticket
                .is_none()
        );
        let (operation, accepted) = authority
            .consume("main", &prepared.plan_id, &scheduler, |_| Ok(()))
            .unwrap();
        let ticket = accepted.unwrap().execution_ticket.unwrap();
        let first_era = ticket.era;
        for generation in 2..=5 {
            scope.generation = generation.to_string();
            authority.begin("main", &scope, || Ok(())).unwrap();
            authority
                .checked_execution("main", &scope, &ticket, || Ok(()))
                .unwrap();
            assert_eq!(ticket.era, first_era);
            let replay = authority
                .consume("main", &prepared.plan_id, &scheduler, |_| {
                    panic!("replay tried to enqueue")
                })
                .unwrap();
            assert_eq!(replay.0.id, operation.id);
            assert_eq!(replay.0.created_at, operation.created_at);
            assert!(replay.1.is_none());
        }
        *time.lock().unwrap() += Duration::from_secs(61);
        // Accepted authority has no refreshed plan TTL; operation deadlines are
        // owned by the unchanged scheduler, never by view reads.
        authority
            .checked_execution("main", &scope, &ticket, || Ok(()))
            .unwrap();
        let mut other = scope.clone();
        other.worktree_id = Some("other-worktree".into());
        other.generation = "6".into();
        authority.begin("main", &other, || Ok(())).unwrap();
        scope.generation = "7".into();
        authority.begin("main", &scope, || Ok(())).unwrap();
        assert!(authority
            .checked_execution("main", &scope, &ticket, || Ok(()))
            .is_err());
        let epoch = authority.begin("main", &scope, || Ok(())).unwrap();
        let p = authority
            .prepare(
                "main",
                plan(scope.clone(), epoch, directory.path(), intent),
                || Ok(()),
            )
            .unwrap();
        let (_, accepted) = authority
            .consume("main", &p.plan_id, &scheduler, |_| Ok(()))
            .unwrap();
        let ticket = accepted.unwrap().execution_ticket.unwrap();
        authority.revoke("main", false);
        authority.begin("main", &scope, || Ok(())).unwrap();
        assert!(authority
            .checked_execution("main", &scope, &ticket, || Ok(()))
            .is_err());
        authority.revoke("main", true);
        authority.open_window("main").unwrap();
        authority.begin("main", &scope, || Ok(())).unwrap();
        assert!(authority
            .checked_execution("main", &scope, &ticket, || Ok(()))
            .is_err());
        let epoch = authority.begin("main", &scope, || Ok(())).unwrap();
        let p = authority
            .prepare(
                "main",
                plan(
                    scope.clone(),
                    epoch,
                    directory.path(),
                    ActionIntent::Commit {
                        message: "runtime-only".into(),
                        amend: false,
                    },
                ),
                || Ok(()),
            )
            .unwrap();
        let (_, accepted) = authority
            .consume("main", &p.plan_id, &scheduler, |_| Ok(()))
            .unwrap();
        let ticket = accepted.unwrap().execution_ticket.unwrap();
        authority.state.lock().unwrap().era = u64::MAX;
        authority.revoke("main", false);
        assert!(authority
            .checked_execution("main", &scope, &ticket, || Ok(()))
            .is_err());
        assert!(authority.begin("main", &scope, || Ok(())).is_err());
        authority.revoke("main", true);
        assert!(authority.open_window("main").is_err());
    }
}
