pub mod authority;
pub(crate) mod branches;
pub(crate) mod catalog;
pub(crate) mod containment;
pub(crate) mod fetch;
pub mod journal;
pub(crate) mod observations;
pub(crate) mod patch;
pub mod preflight;
pub(crate) mod publication;
pub mod recovery;
pub(crate) mod ref_catalog;
pub(crate) mod remote;
pub(crate) mod stash;
pub(crate) mod sync;
use crate::{
    domain::contracts::*,
    git::{
        error::error,
        runner::{Cancellation, GitRunner, RunOptions},
    },
    operations::scheduler::{Completion, LockKey, Scheduler},
    repositories::registry::RepositoryLocation,
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
pub struct Actions {
    #[cfg(test)]
    pub(crate) index_test: Arc<index::TestHooks>,
    #[cfg(test)]
    pub(crate) sync_test: Arc<sync::TestHooks>,
    pub authority: authority::Authority,
    pub(crate) ref_catalogs: ref_catalog::Catalogs,
    pub(crate) change_catalogs: ref_catalog::Catalogs,
    pub(crate) stash_catalogs: ref_catalog::Catalogs,
    pub(crate) remote_snapshots: Arc<remote::Snapshots>,
    pub reads: Arc<tokio::sync::Semaphore>,
    resources: Arc<AtomicUsize>,
    pub(crate) fetch_timeout: std::time::Duration,
}
impl Default for Actions {
    fn default() -> Self {
        Self {
            #[cfg(test)]
            index_test: Default::default(),
            #[cfg(test)]
            sync_test: Default::default(),
            authority: Default::default(),
            ref_catalogs: Default::default(),
            change_catalogs: Default::default(),
            stash_catalogs: Default::default(),
            remote_snapshots: Default::default(),
            reads: Arc::new(tokio::sync::Semaphore::new(4)),
            resources: Arc::new(AtomicUsize::new(0)),
            fetch_timeout: std::time::Duration::from_secs(300),
        }
    }
}
pub struct Slot {
    counter: Arc<AtomicUsize>,
}
impl Drop for Slot {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Actions {
    /// Trusted bounded policy seam for deterministic native deadline tests.
    /// No renderer setting or command can alter this value.
    pub fn with_fetch_timeout(timeout: std::time::Duration) -> Result<Self, AppError> {
        if timeout.is_zero() || timeout > std::time::Duration::from_secs(300) {
            return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
        }
        Ok(Self {
            fetch_timeout: timeout,
            ..Default::default()
        })
    }
    pub fn reserve(&self) -> Result<Slot, AppError> {
        self.resources
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                (v < 64).then_some(v + 1)
            })
            .map_err(|_| error(ErrorCode::OutputLimit, Phase::Queue))?;
        Ok(Slot {
            counter: self.resources.clone(),
        })
    }
}
pub struct Resources {
    #[cfg(test)]
    pub(crate) index_test: Arc<index::TestHooks>,
    #[cfg(test)]
    pub(crate) sync_test: Arc<sync::TestHooks>,
    pub(crate) fetch_timeout: std::time::Duration,
    pub pins: Arc<preflight::Pins>,
    pub serialization: Arc<crate::operations::scheduler::SerializationLease>,
    pub reads: Arc<tokio::sync::Semaphore>,
    pub(crate) _slot: Slot,
}
pub struct ReadScope {
    pub pins: Arc<preflight::Pins>,
    pub serialization: Arc<crate::operations::scheduler::SerializationLease>,
    pub(crate) _slot: Slot,
}
struct Lifetime {
    _resources: Arc<Resources>,
    _helper: Option<Arc<dyn Send + Sync>>,
}
struct ReadLease {
    _lifetime: Arc<dyn Send + Sync>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
pub fn read_lease(
    lifetime: Arc<dyn Send + Sync>,
    permit: tokio::sync::OwnedSemaphorePermit,
) -> Arc<dyn Send + Sync> {
    Arc::new(ReadLease {
        _lifetime: lifetime,
        _permit: permit,
    })
}
pub fn read_permit(
    reads: &Arc<tokio::sync::Semaphore>,
) -> Result<tokio::sync::OwnedSemaphorePermit, AppError> {
    reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| error(ErrorCode::OutputLimit, Phase::Inspection))
}
pub struct Execution {
    pub operation: OperationSnapshot,
    pub(crate) plan: Option<authority::Plan>,
    pub(crate) slot: Option<Slot>,
    pub(crate) guard: Option<ExecutionGuard>,
}
#[derive(Clone, Copy)]
pub(crate) enum Gate {
    Check,
    ReadStart(RemoteEndpointRole),
    Write,
    FetchStep(usize, [u8; 64]),
    RefStep(usize, [u8; 64]),
    SyncStep(usize, [u8; 64]),
}
/// The executor must use this pure lease for every owned process. Unknown BQ
/// shutdown retains directory pins and resource slot, never the user intent/DB.
pub struct ActionContext {
    pub operation_id: String,
    pub(crate) scope: ActionScope,
    pub(crate) ticket: authority::ExecutionTicket,
    pub intent: ActionIntent,
    pub targets: Vec<authority::Target>,
    pub location: RepositoryLocation,
    pub precondition: preflight::Preflight,
    pub cancel: Cancellation,
    pub resources: Arc<Resources>,
    scheduler: Arc<Scheduler>,
    runner: GitRunner,
    validate: Arc<dyn Fn(Gate) -> Result<bool, AppError> + Send + Sync>,
}
impl ActionContext {
    pub(crate) fn sync_observed(&self, proof: journal::SyncActionProof) -> Result<(), AppError> {
        self.scheduler
            .action_sync_observed(&self.operation_id, proof)
    }
    pub(crate) fn before_remote_read(&self, role: RemoteEndpointRole) -> Result<bool, AppError> {
        (self.validate)(Gate::ReadStart(role))
    }
    pub(crate) fn remote_read_observed(
        &self,
        count: Option<usize>,
        digest: Option<String>,
    ) -> Result<(), AppError> {
        self.scheduler
            .action_read_observed(&self.operation_id, count, digest)
    }
    pub(crate) async fn remote_read_output(
        &self,
        args: &[std::ffi::OsString],
        mut options: RunOptions,
    ) -> Result<crate::git::runner::GitOutput, AppError> {
        if !self.scheduler.action_read_started(&self.operation_id)? {
            return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
        }
        options.lifetime_guard = Some(Arc::new(Lifetime {
            _resources: self.resources.clone(),
            _helper: options.lifetime_guard.take(),
        }));
        options.environment.extend([
            ("GIT_NO_LAZY_FETCH".into(), "1".into()),
            ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
            ("GIT_ALLOW_PROTOCOL".into(), "file:https:ssh".into()),
        ]);
        let mut all = preflight::global_args(&self.location)?;
        all.extend_from_slice(args);
        let mut runner = self.runner.clone();
        runner.timeout = self.resources.fetch_timeout;
        runner.output_limit = runner.output_limit.min(8 * 1024 * 1024);
        runner
            .output_history(
                Some(&self.location.root),
                &all,
                &self.cancel,
                Phase::Transfer,
                options,
                None,
            )
            .await
    }

    pub async fn before_write(&self) -> Result<bool, AppError> {
        let lease = read_lease(self.resources.clone(), read_permit(&self.resources.reads)?);
        let now = preflight::capture(&self.runner, &self.location, lease, &self.cancel).await?;
        if now.fingerprint != self.precondition.fingerprint {
            return Err(error(ErrorCode::ExternalChange, Phase::Validation));
        }
        preflight::policy(&self.intent, &now)?;
        (self.validate)(Gate::Write)
    }
    pub(crate) fn before_fetch_step(
        &self,
        index: usize,
        refs_digest: &str,
    ) -> Result<bool, AppError> {
        let digest: [u8; 64] = refs_digest
            .as_bytes()
            .try_into()
            .map_err(|_| error(ErrorCode::InvalidRequest, Phase::Validation))?;
        if !digest.iter().all(u8::is_ascii_hexdigit) {
            return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
        }
        (self.validate)(Gate::FetchStep(index, digest))
    }
    pub(crate) fn before_ref_step(&self, index: usize, stamp: &str) -> Result<bool, AppError> {
        let digest: [u8; 64] = stamp
            .as_bytes()
            .try_into()
            .map_err(|_| error(ErrorCode::InvalidRequest, Phase::Validation))?;
        (self.validate)(Gate::RefStep(index, digest))
    }
    pub(crate) fn observe_ref_step(
        &self,
        index: usize,
        effects: ActionEffects,
        stamp: Option<String>,
        reason: Option<RefActionFailureReason>,
    ) -> Result<(), AppError> {
        self.scheduler
            .action_ref_observed(&self.operation_id, index, effects, stamp, reason)
    }
    pub(crate) async fn local_output(
        &self,
        args: &[std::ffi::OsString],
        input: Option<Vec<u8>>,
    ) -> Result<crate::git::runner::GitOutput, AppError> {
        let mut all = preflight::global_args(&self.location)?;
        all.extend_from_slice(args);
        let mut options = self.run_options(RunOptions::default())?;
        options.environment.extend([
            ("GIT_ALLOW_PROTOCOL".into(), "".into()),
            ("GIT_NO_LAZY_FETCH".into(), "1".into()),
            ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
        ]);
        let mut runner = self.runner.clone();
        runner.timeout = std::time::Duration::from_secs(120);
        runner
            .output_history(
                Some(&self.location.root),
                &all,
                &self.cancel,
                Phase::Publish,
                options,
                input.as_deref(),
            )
            .await
    }
    pub(crate) fn check_owner(&self) -> Result<(), AppError> {
        (self.validate)(Gate::Check).map(|_| ())
    }
    pub(crate) fn reader(&self) -> Result<catalog::ReadQuery<'_>, AppError> {
        let lifetime = read_lease(self.resources.clone(), read_permit(&self.resources.reads)?);
        Ok(catalog::ReadQuery::new(
            &self.runner,
            &self.location,
            lifetime,
            self.cancel.clone(),
        ))
    }
    pub(crate) async fn output(
        &self,
        args: &[std::ffi::OsString],
        phase: Phase,
        options: RunOptions,
    ) -> Result<crate::git::runner::GitOutput, AppError> {
        let mut options = self.run_options(options)?;
        options.environment.extend([
            ("GIT_NO_LAZY_FETCH".into(), "1".into()),
            ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
            ("GIT_ALLOW_PROTOCOL".into(), "file:https:ssh".into()),
        ]);
        let mut all = preflight::global_args(&self.location)?;
        all.extend_from_slice(args);
        let mut runner = self.runner.clone();
        runner.timeout = self.resources.fetch_timeout;
        runner
            .output_history(
                Some(&self.location.root),
                &all,
                &self.cancel,
                phase,
                options,
                None,
            )
            .await
    }
    /// Trusted executor option composer, also reused by future structured-output
    /// calls. The returned pure lifetime guard must be passed to GitRunner.
    pub fn run_options(&self, mut options: RunOptions) -> Result<RunOptions, AppError> {
        if !self.scheduler.action_write_started(&self.operation_id)? {
            return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
        }
        // AuthSession supplies only its pure pinned helper-path lease here.
        options.lifetime_guard = Some(Arc::new(Lifetime {
            _resources: self.resources.clone(),
            _helper: options.lifetime_guard.take(),
        }));
        Ok(options)
    }
    pub async fn run(
        &self,
        args: &[std::ffi::OsString],
        phase: Phase,
        options: RunOptions,
    ) -> Result<Vec<u8>, AppError> {
        let options = self.run_options(options)?;
        let mut all = preflight::global_args(&self.location)?;
        all.extend_from_slice(args);
        self.runner
            .run_with(
                Some(&self.location.root),
                &all,
                &self.cancel,
                phase,
                options,
            )
            .await
    }
}
pub(crate) struct ExecutionGuard {
    pub scheduler: Arc<Scheduler>,
    pub id: String,
    pub armed: bool,
    pub before_finish: Option<crate::operations::scheduler::CompletionInvalidation>,
}
impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        if self.armed
            && self
                .scheduler
                .get(&self.id)
                .ok()
                .is_none_or(|o| !crate::operations::state::terminal(&o.state))
        {
            if let Some(invalidate) = &self.before_finish {
                invalidate(self.scheduler.action_failure_effects(&self.id));
            }
            let _ = self
                .scheduler
                .action_failure(&self.id, error(ErrorCode::Interrupted, Phase::Recovery));
        }
    }
}
pub fn location_bytes(location: &RepositoryLocation) -> usize {
    crate::persistence::workspace::encode(&location.common).len()
        + crate::persistence::workspace::encode(&location.root).len()
        + crate::persistence::workspace::encode(&location.git_dir).len()
        + location.common_identity.len()
        + location.root_identity.as_ref().map_or(0, Vec::len)
}
pub fn path_text(path: &Path) -> Result<std::ffi::OsString, AppError> {
    #[cfg(windows)]
    {
        crate::clone::path::git_path(path)
    }
    #[cfg(not(windows))]
    {
        Ok(path.as_os_str().to_owned())
    }
}
pub(crate) fn context(
    operation_id: String,
    plan: authority::Plan,
    cancel: Cancellation,
    resources: Arc<Resources>,
    scheduler: Arc<Scheduler>,
    runner: GitRunner,
    validate: Arc<dyn Fn(Gate) -> Result<bool, AppError> + Send + Sync>,
) -> ActionContext {
    ActionContext {
        operation_id,
        scope: plan.scope,
        ticket: plan.execution_ticket.expect("consumed action ticket"),
        intent: plan.intent,
        targets: plan.targets,
        location: plan.location,
        precondition: plan.preflight,
        cancel,
        resources,
        scheduler,
        runner,
        validate,
    }
}
pub fn lock_keys(location: &RepositoryLocation) -> Vec<LockKey> {
    vec![LockKey {
        common: location.common_identity.clone(),
        worktree: location.root_identity.clone(),
    }]
}
pub fn network(intent: &ActionIntent) -> bool {
    matches!(
        intent,
        ActionIntent::ReadRemoteRefs { .. }
            | ActionIntent::Fetch { .. }
            | ActionIntent::Push { .. }
            | ActionIntent::RemoteTagPush { .. }
            | ActionIntent::RemoteTagDelete { .. }
            | ActionIntent::Pull { .. }
    )
}
pub fn completed(effects: ActionEffects, postcondition: Option<String>) -> Completion {
    Completion::Action {
        effects,
        postcondition,
    }
}

/// Contextual static templates: clone templates remain unchanged and raw text
/// never survives the operation journal sanitizer.
pub fn safe_error(value: &AppError) -> AppError {
    let mut safe = crate::git::error::sanitized_error(value);
    if value.code == ErrorCode::UnsupportedGit {
        safe.message="Installed Git does not support safe repository actions. Update Git before running repository actions.".into();
        safe.retry = AppErrorRetry::None;
    } else if value.code == ErrorCode::Database && value.phase == Phase::Recovery {
        safe.message="Repository action effects could not be saved or verified. Restart OpenGit and inspect the repository before retrying.".into();
        safe.retry = AppErrorRetry::None;
    } else if value.code == ErrorCode::Interrupted && value.phase == Phase::Recovery {
        safe.message="Repository action effects or process shutdown could not be confirmed. Restart OpenGit and inspect the repository before retrying.".into();
        safe.retry = AppErrorRetry::None;
    }
    safe
}
pub(crate) mod changes;
pub(crate) mod commit;
pub(crate) mod index;
