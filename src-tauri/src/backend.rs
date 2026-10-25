use crate::{
    domain::contracts::*,
    git::{
        environment,
        error::error,
        operation_detection, refs,
        runner::{argv, Cancellation, GitRunner},
        status,
    },
    ipc::validation,
    operations::scheduler::Scheduler,
    persistence::workspace::Database,
    repositories::{discovery, registry::Registry},
};
use std::{
    collections::HashSet,
    path::Path,
    sync::{Arc, Mutex},
};
struct ActionReadAccess {
    location: crate::repositories::registry::RepositoryLocation,
    epoch: u64,
    cancel: Cancellation,
    lifetime: Arc<dyn Send + Sync>,
}
#[path = "actions/change_backend.rs"]
mod change_backend;
#[path = "actions/sync_backend.rs"]
mod sync_backend;
#[cfg(test)]
#[derive(Default)]
struct ActionTestHooks {
    prepare_pause: Mutex<Option<Arc<PreparePause>>>,
    scope_begun: Mutex<Option<Arc<tokio::sync::Notify>>>,
    remote_resolve_pause: Mutex<Option<Arc<PreparePause>>>,
    after_terminal_pause: Mutex<Option<Arc<PreparePause>>>,
    completion_done: Mutex<Option<Arc<tokio::sync::Notify>>>,
}
#[cfg(test)]
#[derive(Default)]
struct PreparePause {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[cfg(test)]
#[path = "actions/lifecycle_tests.rs"]
pub(crate) mod action_lifecycle_tests;
pub struct Backend {
    pub registry: Arc<Registry>,
    pub scheduler: Arc<Scheduler>,
    pub actions: crate::actions::Actions,
    #[cfg(test)]
    action_test: ActionTestHooks,
    #[cfg(windows)]
    pub auth: Arc<crate::auth::AuthBroker>,
    #[cfg(windows)]
    pub clones: crate::clone::grants::CloneState,
    runner: Option<GitRunner>,
    history: crate::history::History,
    pub(crate) watches: Arc<crate::history::watch::Watches>,
    #[cfg(test)]
    pub(crate) watch_test: crate::history::watch::TestHooks,
    windows: Mutex<HashSet<String>>,
}
impl Backend {
    pub fn new(data_dir: &Path) -> Result<Self, AppError> {
        Self::build(data_dir, GitRunner::discover().ok())
    }
    pub fn with_git(data_dir: &Path, runner: GitRunner) -> Result<Self, AppError> {
        Self::build(data_dir, Some(runner))
    }
    fn build(data_dir: &Path, runner: Option<GitRunner>) -> Result<Self, AppError> {
        let database = Arc::new(Database::open(data_dir)?);
        #[cfg(windows)]
        crate::clone::recovery::before_backend(database.clone(), runner.clone())?;
        let scheduler = Arc::new(Scheduler::new(database.clone())?);
        Ok(Self {
            registry: Arc::new(Registry {
                database: database.clone(),
            }),
            #[cfg(windows)]
            auth: crate::auth::AuthBroker::new(scheduler.clone()),
            #[cfg(windows)]
            clones: Default::default(),
            scheduler,
            actions: Default::default(),
            #[cfg(test)]
            action_test: Default::default(),
            runner,
            history: Default::default(),
            watches: Arc::new(Default::default()),
            #[cfg(test)]
            watch_test: Default::default(),
            windows: Mutex::new(HashSet::new()),
        })
    }
    fn action_registration(
        &self,
        scope: &ActionScope,
        location: &crate::repositories::registry::RepositoryLocation,
    ) -> Result<(), AppError> {
        let connection = self.registry.database.lock()?;
        Self::check_watch_registration(
            &connection,
            &HistoryScope {
                repo_id: scope.repo_id.clone(),
                worktree_id: scope.worktree_id.clone(),
                generation: scope.generation.clone(),
            },
            location,
        )
    }
    async fn action_read_scope(
        &self,
        window: &str,
        scope: &ActionScope,
    ) -> Result<ActionReadAccess, AppError> {
        self.window(window)?;
        validation::request_size(scope)?;
        let location = self
            .registry
            .location(&scope.repo_id, scope.worktree_id.as_deref())?;
        let epoch = self
            .actions
            .authority
            .begin(window, scope, || self.action_registration(scope, &location))?;
        #[cfg(test)]
        if let Some(notify) = self.action_test.scope_begun.lock().unwrap().take() {
            notify.notify_one();
        }
        self.action_read_scope_existing(window, scope, location, epoch)
            .await
    }
    async fn action_read_scope_existing(
        &self,
        window: &str,
        scope: &ActionScope,
        location: crate::repositories::registry::RepositoryLocation,
        epoch: u64,
    ) -> Result<ActionReadAccess, AppError> {
        self.actions.authority.checked(window, scope, epoch, || {
            self.window(window)?;
            self.action_registration(scope, &location)
        })?;
        let cancel = self.actions.authority.read_cancel(window, scope, epoch)?;
        let slot = self.actions.reserve()?;
        let serialization = self
            .scheduler
            .acquire_scope(
                crate::actions::lock_keys(&location),
                false,
                &cancel,
                Some(std::time::Duration::from_secs(30)),
            )
            .await?;
        self.actions.authority.checked(window, scope, epoch, || {
            self.action_registration(scope, &location)
        })?;
        let permit = crate::actions::read_permit(&self.actions.reads)?;
        let pins = crate::actions::preflight::Pins::open(&location)?;
        let lifetime = crate::actions::read_lease(
            Arc::new(crate::actions::ReadScope {
                pins,
                serialization,
                _slot: slot,
            }),
            permit,
        );
        Ok(ActionReadAccess {
            location,
            epoch,
            cancel,
            lifetime,
        })
    }
    pub async fn get_ref_actions(
        &self,
        window: &str,
        request: GetRefActionsRequest,
    ) -> Result<RefActionCatalog, AppError> {
        validation::request_size(&request)?;
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            Box::pin(async {
                let access = self.action_read_scope(window, &request.scope).await?;
                let before = crate::actions::preflight::capture(
                    self.git()?,
                    &access.location,
                    access.lifetime.clone(),
                    &access.cancel,
                )
                .await?;
                let q = crate::actions::catalog::ReadQuery::new(
                    self.git()?,
                    &access.location,
                    access.lifetime.clone(),
                    access.cancel.clone(),
                );
                let capture = crate::actions::ref_catalog::capture(&q).await?;
                let after = crate::actions::preflight::capture(
                    self.git()?,
                    &access.location,
                    access.lifetime,
                    &access.cancel,
                )
                .await?;
                if before.fingerprint != after.fingerprint {
                    return Err(error(ErrorCode::ExternalChange, Phase::Inspection));
                }
                let ticket =
                    self.actions
                        .authority
                        .checked(window, &request.scope, access.epoch, || {
                            self.action_registration(&request.scope, &access.location)?;
                            self.actions.ref_catalogs.publish(
                                window,
                                request.scope.clone(),
                                access.epoch,
                                before.fingerprint,
                            )
                        })?;
                let head_entry_id = capture
                    .head
                    .branch
                    .as_ref()
                    .and_then(|b| {
                        capture
                            .entries
                            .iter()
                            .find(|r| r.name == format!("refs/heads/{b}"))
                    })
                    .map(|r| r.entry_id.clone());
                let result = RefActionCatalog {
                    scope: request.scope,
                    catalog_id: ticket.id,
                    observed_at: chrono::Utc::now().to_rfc3339(),
                    head: capture.head,
                    head_entry_id,
                    refs: capture.entries,
                    bare: before.bare,
                    dirty: before.dirty,
                };
                crate::actions::catalog::bounded_response(&result)?;
                Ok(result)
            }),
        )
        .await
        .map_err(|_| error(ErrorCode::Timeout, Phase::Inspection))?
    }
    pub async fn resolve_ref_action_target(
        &self,
        window: &str,
        request: ResolveRefActionTargetRequest,
    ) -> Result<ResolvedRefActionTarget, AppError> {
        use crate::actions::{
            authority::{Target, TargetKind},
            journal::SafeTargetProof,
            ref_catalog,
        };
        validation::request_size(&request)?;
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            Box::pin(async {
                let ticket =
                    self.actions
                        .ref_catalogs
                        .get(window, &request.scope, &request.catalog_id)?;
                let location = self
                    .registry
                    .location(&request.scope.repo_id, request.scope.worktree_id.as_deref())?;
                let access = self
                    .action_read_scope_existing(window, &request.scope, location, ticket.epoch)
                    .await?;
                let before = crate::actions::preflight::capture(
                    self.git()?,
                    &access.location,
                    access.lifetime.clone(),
                    &access.cancel,
                )
                .await?;
                if before.fingerprint != ticket.stamp {
                    return Err(error(ErrorCode::ExternalChange, Phase::Inspection));
                }
                let q = crate::actions::catalog::ReadQuery::new(
                    self.git()?,
                    &access.location,
                    access.lifetime.clone(),
                    access.cancel.clone(),
                );
                let capture = ref_catalog::capture(&q).await?;
                let (target, kind, display_name, oid) = match &request.source {
                    RefActionSource::Ref { entry_id } => {
                        let e = capture
                            .entries
                            .iter()
                            .find(|e| &e.entry_id == entry_id)
                            .ok_or_else(ref_catalog::invalid)?;
                        let (t, k) = ref_catalog::target(e, capture.length)?;
                        (t, k, e.name.clone(), e.oid.clone())
                    }
                    RefActionSource::Head => {
                        let oid = capture.head.oid.clone().ok_or_else(ref_catalog::invalid)?;
                        (
                            Target {
                                kind: TargetKind::Commit,
                                identity: oid.as_bytes().to_vec(),
                                proof: Some(SafeTargetProof::Commit {
                                    oid: oid.clone(),
                                    object_format: ref_catalog::object_format(capture.length),
                                }),
                            },
                            RefTargetKind::Commit,
                            "HEAD".into(),
                            Some(oid),
                        )
                    }
                    RefActionSource::HistoryCommit {
                        context,
                        commit_handle,
                    } => {
                        if context.scope.repo_id != request.scope.repo_id
                            || context.scope.worktree_id != request.scope.worktree_id
                            || context.scope.generation != request.scope.generation
                        {
                            return Err(ref_catalog::invalid());
                        }
                        let oid = self
                            .history
                            .resolve_action_commit(
                                window,
                                self.git()?,
                                context,
                                commit_handle,
                                access.lifetime.clone(),
                            )
                            .await?;
                        (
                            Target {
                                kind: TargetKind::Commit,
                                identity: oid.as_bytes().to_vec(),
                                proof: Some(SafeTargetProof::Commit {
                                    oid: oid.clone(),
                                    object_format: ref_catalog::object_format(capture.length),
                                }),
                            },
                            RefTargetKind::Commit,
                            oid.clone(),
                            Some(oid),
                        )
                    }
                };
                let after = crate::actions::preflight::capture(
                    self.git()?,
                    &access.location,
                    access.lifetime,
                    &access.cancel,
                )
                .await?;
                if before.fingerprint != after.fingerprint {
                    return Err(error(ErrorCode::ExternalChange, Phase::Inspection));
                }
                self.actions
                    .ref_catalogs
                    .get(window, &request.scope, &request.catalog_id)?;
                let handle = self.actions.authority.issue(
                    window,
                    &request.scope,
                    ticket.epoch,
                    target,
                    ticket.stamp,
                    || {
                        self.actions.ref_catalogs.get(
                            window,
                            &request.scope,
                            &request.catalog_id,
                        )?;
                        self.action_registration(&request.scope, &access.location)
                    },
                )?;
                Ok(ResolvedRefActionTarget {
                    scope: request.scope,
                    catalog_id: request.catalog_id,
                    handle,
                    kind,
                    display_name,
                    oid,
                })
            }),
        )
        .await
        .map_err(|_| error(ErrorCode::Timeout, Phase::Inspection))?
    }
    pub fn get_ref_action_outcome(
        &self,
        window: &str,
        request: GetActionOutcomeRequest,
    ) -> Result<RefActionOperationResult, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        if self
            .scheduler
            .action_owner(&request.operation_id)?
            .as_deref()
            != Some(window)
        {
            return Err(error(ErrorCode::InvalidHandle, Phase::Validation));
        }
        let record = crate::actions::journal::read(&self.registry.database, &request.operation_id)?;
        let result = RefActionOperationResult {
            operation: self.scheduler.get(&request.operation_id)?,
            result: record.ref_action.map(|r| RefActionResult {
                steps: r
                    .steps
                    .into_iter()
                    .map(|s| RefActionStepResult {
                        step: s.kind.label().into(),
                        phase: s.phase,
                        effects: s.effects,
                        failure_reason: s.failure_reason,
                    })
                    .collect(),
            }),
        };
        crate::actions::catalog::bounded_response(&result)?;
        Ok(result)
    }
    pub async fn get_remote_actions(
        &self,
        window: &str,
        request: GetRemoteActionsRequest,
    ) -> Result<RemoteActionCatalog, AppError> {
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            Box::pin(async {
                let scope = request.scope;
                let access = self.action_read_scope(window, &scope).await?;
                let before = crate::actions::preflight::capture(
                    self.git()?,
                    &access.location,
                    access.lifetime.clone(),
                    &access.cancel,
                )
                .await?;
                let q = crate::actions::catalog::ReadQuery::new(
                    self.git()?,
                    &access.location,
                    access.lifetime.clone(),
                    access.cancel.clone(),
                );
                let endpoints = crate::actions::remote::endpoints(&q).await?;
                let after = crate::actions::preflight::capture(
                    self.git()?,
                    &access.location,
                    access.lifetime.clone(),
                    &access.cancel,
                )
                .await?;
                if before.fingerprint != after.fingerprint {
                    return Err(error(ErrorCode::ExternalChange, Phase::Inspection));
                }
                let targets = endpoints
                    .iter()
                    .filter(|e| e.capability == RemoteEndpointCapability::Eligible)
                    .map(|e| e.target())
                    .collect();
                let handles = self.actions.authority.issue_catalog(
                    window,
                    &scope,
                    access.epoch,
                    targets,
                    &before.fingerprint,
                    || self.action_registration(&scope, &access.location),
                )?;
                let mut handles = handles.into_iter();
                let mut remotes: Vec<RemoteActionEntry> = Vec::new();
                for endpoint in endpoints {
                    let handle = if endpoint.capability == RemoteEndpointCapability::Eligible {
                        Some(
                            handles
                                .next()
                                .ok_or_else(|| error(ErrorCode::Internal, Phase::Inspection))?,
                        )
                    } else {
                        None
                    };
                    if remotes.last().is_none_or(|r| r.name != endpoint.name) {
                        remotes.push(RemoteActionEntry {
                            name: endpoint.name.clone(),
                            endpoints: Vec::new(),
                        });
                    }
                    remotes.last_mut().unwrap().endpoints.push(RemoteEndpoint {
                        role: endpoint.role,
                        handle,
                        capability: endpoint.capability,
                    });
                }
                let result = RemoteActionCatalog {
                    scope,
                    observed_at: chrono::Utc::now().to_rfc3339(),
                    remotes,
                };
                crate::actions::catalog::bounded_response(&result)?;
                Ok(result)
            }),
        )
        .await
        .map_err(|_| error(ErrorCode::Timeout, Phase::Inspection))?
    }
    fn publish_remote_snapshot(
        &self,
        location: &crate::repositories::registry::RepositoryLocation,
        value: crate::actions::remote::Snapshot,
    ) -> Result<(), AppError> {
        let owner = value.owner.clone();
        let scope = value.wire.captured_scope.clone();
        let ticket = value.ticket.clone();
        // Publication and hard revocation share the authority lock. A delayed old
        // process cannot overwrite a newly reopened window's valid snapshot.
        self.actions
            .authority
            .checked_execution(&owner, &scope, &ticket, || {
                self.window(&owner)?;
                self.action_registration(&scope, location)?;
                self.actions.remote_snapshots.publish(value)
            })
    }
    pub fn get_remote_action_outcome(
        &self,
        window: &str,
        request: GetActionOutcomeRequest,
    ) -> Result<RemoteActionOperationResult, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        if self
            .scheduler
            .action_owner(&request.operation_id)?
            .as_deref()
            != Some(window)
        {
            return Err(error(ErrorCode::InvalidHandle, Phase::Validation));
        }
        let operation = self.scheduler.get(&request.operation_id)?;
        if operation.action_kind != Some(ActionKind::ReadRemoteRefs) {
            return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
        }
        let record = crate::actions::journal::read(&self.registry.database, &request.operation_id)?;
        let mut availability = if crate::operations::state::terminal(&operation.state) {
            RemoteSnapshotAvailability::Unavailable
        } else {
            RemoteSnapshotAvailability::Pending
        };
        let mut snapshot = None;
        if let Some(value) = self
            .actions
            .remote_snapshots
            .outcome(window, &request.operation_id)
        {
            if self
                .actions
                .authority
                .checked_execution(window, &value.wire.captured_scope, &value.ticket, || Ok(()))
                .is_ok()
            {
                availability = RemoteSnapshotAvailability::Available;
                snapshot = Some(value.wire);
            }
        }
        let evidence = record.remote_read.map(|p| {
            if availability == RemoteSnapshotAvailability::Unavailable
                && p.observed_at
                    .as_ref()
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                    .is_some_and(|t| {
                        chrono::Utc::now().signed_duration_since(t).num_seconds() >= 60
                    })
            {
                availability = RemoteSnapshotAvailability::Expired;
            }
            RemoteReadEvidence {
                remote_name: p.remote_name,
                role: p.role,
                phase: p.phase,
                ref_count: p.ref_count,
                observed_at: p.observed_at,
                no_git_writes: p.no_git_writes,
            }
        });
        let result = RemoteActionOperationResult {
            operation,
            availability,
            evidence,
            snapshot,
        };
        crate::actions::catalog::bounded_response(&result)?;
        Ok(result)
    }
    pub async fn resolve_remote_target(
        &self,
        window: &str,
        request: ResolveRemoteTargetRequest,
    ) -> Result<ResolvedRemoteTarget, AppError> {
        validation::request_size(&request)?;
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            Box::pin(async {
                self.window(window)?;
                let ticket = self
                    .actions
                    .remote_snapshots
                    .ticket(window, &request.snapshot_id)?;
                #[cfg(test)]
                {
                    let pause = self.action_test.remote_resolve_pause.lock().unwrap().take();
                    if let Some(pause) = pause {
                        pause.entered.notify_one();
                        pause.release.notified().await;
                    }
                }
                let location = self
                    .registry
                    .location(&request.scope.repo_id, request.scope.worktree_id.as_deref())?;
                let epoch = self.actions.authority.begin_execution(
                    window,
                    &request.scope,
                    &ticket,
                    || {
                        self.window(window)?;
                        self.action_registration(&request.scope, &location)?;
                        self.actions
                            .remote_snapshots
                            .validate(window, &request.snapshot_id)
                    },
                )?;
                let access = self
                    .action_read_scope_existing(window, &request.scope, location, epoch)
                    .await?;
                let snapshot = self
                    .actions
                    .remote_snapshots
                    .get(window, &request.snapshot_id)?;
                let state = crate::actions::preflight::capture(
                    self.git()?,
                    &access.location,
                    access.lifetime.clone(),
                    &access.cancel,
                )
                .await?;
                let q = crate::actions::catalog::ReadQuery::new(
                    self.git()?,
                    &access.location,
                    access.lifetime.clone(),
                    access.cancel.clone(),
                );
                if state.fingerprint != snapshot.stamp
                    || crate::actions::catalog::configuration(&q).await?.digest
                        != snapshot.config_digest
                {
                    return Err(error(ErrorCode::ExternalChange, Phase::Validation));
                }
                let (name, expected_oid, namespace) = match &request.source {
                    RemoteTargetSource::Existing { entry_id } => {
                        let r = snapshot
                            .wire
                            .refs
                            .iter()
                            .find(|r| r.entry_id == *entry_id)
                            .ok_or_else(crate::actions::remote::invalid)?;
                        (
                            r.name.clone(),
                            Some(r.oid.clone()),
                            if r.name.starts_with("refs/heads/") {
                                RemoteRefNamespace::Branch
                            } else {
                                RemoteRefNamespace::Tag
                            },
                        )
                    }
                    RemoteTargetSource::New { namespace, name } => {
                        let prefix = if *namespace == RemoteRefNamespace::Branch {
                            "refs/heads/"
                        } else {
                            "refs/tags/"
                        };
                        let full = format!("{prefix}{name}");
                        if name.starts_with('-')
                            || full.len() > 1024
                            || !crate::history::refs::valid_name(&full)
                            || snapshot.wire.refs.iter().any(|r| r.name == full)
                        {
                            return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
                        }
                        (full, None, *namespace)
                    }
                };
                let proof = crate::actions::journal::SafeTargetProof::RemoteDestination {
                    remote_name: snapshot.wire.remote_name.clone(),
                    role: snapshot.wire.role,
                    ref_name: name.clone(),
                    expected_oid: expected_oid.clone(),
                    object_format: snapshot.wire.object_format.as_ref().map(|f| {
                        if f == "sha1" {
                            crate::actions::journal::ObjectFormat::Sha1
                        } else {
                            crate::actions::journal::ObjectFormat::Sha256
                        }
                    }),
                    config_digest: snapshot.config_digest.clone(),
                };
                let target = crate::actions::authority::Target {
                    kind: crate::actions::authority::TargetKind::RemoteDestination,
                    identity: serde_json::to_vec(&proof)
                        .map_err(|_| error(ErrorCode::InvalidRequest, Phase::Validation))?,
                    proof: Some(proof),
                };
                let handle = self.actions.authority.issue_until(
                    window,
                    &request.scope,
                    access.epoch,
                    target,
                    state.fingerprint,
                    Some(snapshot.expires),
                    || {
                        self.action_registration(&request.scope, &access.location)?;
                        self.actions
                            .remote_snapshots
                            .validate(window, &request.snapshot_id)
                    },
                )?;
                Ok(ResolvedRemoteTarget {
                    scope: request.scope,
                    snapshot_id: request.snapshot_id,
                    handle,
                    name,
                    expected_oid,
                    namespace,
                })
            }),
        )
        .await
        .map_err(|_| error(ErrorCode::Timeout, Phase::Inspection))?
    }
    pub async fn get_repository_actions(
        &self,
        window: &str,
        request: GetRepositoryActionsRequest,
    ) -> Result<RepositoryActionCatalog, AppError> {
        use crate::actions::{
            authority::{Target, TargetKind},
            catalog,
            journal::SafeTargetProof,
        };
        validation::request_size(&request)?;
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let ActionReadAccess {
                location,
                epoch,
                cancel,
                lifetime,
            } = self.action_read_scope(window, &request.scope).await?;
            let runner = self.git()?;
            let before =
                crate::actions::preflight::capture(runner, &location, lifetime.clone(), &cancel)
                    .await?;
            let query =
                catalog::ReadQuery::new(runner, &location, lifetime.clone(), cancel.clone());
            let format = query
                .run(&argv(&["rev-parse", "--show-object-format"]))
                .await?;
            let length = match format.as_slice() {
                b"sha1\n" => 40,
                b"sha256\n" => 64,
                _ => return Err(error(ErrorCode::MalformedOutput, Phase::Inspection)),
            };
            crate::actions::fetch::read_refs(&query, length).await?; // Whole catalog limit, including non-branch refs.
            let mut branches = catalog::branches(&query, length).await?;
            let (head, _) = catalog::head(&query, &branches).await?;
            let config = catalog::configuration(&query).await?;
            let sync = catalog::sync(&query, &head, &branches, config.partial).await?;
            crate::actions::observations::attach(
                &self.registry.database,
                &request.scope.repo_id,
                &config.remotes,
                &mut branches.branches,
            )?;
            let after =
                crate::actions::preflight::capture(runner, &location, lifetime, &cancel).await?;
            if before.fingerprint != after.fingerprint {
                return Err(error(ErrorCode::ExternalChange, Phase::Inspection));
            }
            let targets = config
                .remotes
                .iter()
                .filter(|r| r.capability == FetchCapability::EligibleProbeRequired)
                .map(|r| Target {
                    kind: TargetKind::Remote,
                    identity: r.name.as_bytes().to_vec(),
                    proof: Some(SafeTargetProof::Remote {
                        name: r.name.clone(),
                        config_digest: r.digest.clone(),
                    }),
                })
                .collect();
            let handles = self.actions.authority.issue_catalog(
                window,
                &request.scope,
                epoch,
                targets,
                &before.fingerprint,
                || self.action_registration(&request.scope, &location),
            )?;
            let mut handles = handles.into_iter();
            let result = RepositoryActionCatalog {
                scope: request.scope.clone(),
                snapshot_id: crate::history::sessions::token(),
                observed_at: chrono::Utc::now().to_rfc3339(),
                head,
                branches: branches.branches,
                symbolic_branches: branches.symbolic,
                remotes: config
                    .remotes
                    .into_iter()
                    .map(|r| {
                        let handle = if r.capability == FetchCapability::EligibleProbeRequired {
                            handles.next()
                        } else {
                            None
                        };
                        ActionRemote {
                            name: r.name,
                            handle,
                            fetch: r.capability,
                        }
                    })
                    .collect(),
                sync,
            };
            crate::actions::catalog::bounded_response(&result)?;
            Ok(result)
        })
        .await
        .map_err(|_| error(ErrorCode::Timeout, Phase::Inspection))?
    }
    pub async fn get_commit_containment(
        &self,
        window: &str,
        request: GetCommitContainmentRequest,
    ) -> Result<CommitContainment, AppError> {
        validation::request_size(&request)?;
        let scope = ActionScope {
            repo_id: request.context.scope.repo_id.clone(),
            worktree_id: request.context.scope.worktree_id.clone(),
            generation: request.context.scope.generation.clone(),
        };
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let ActionReadAccess {
                location,
                epoch,
                cancel,
                lifetime,
            } = self.action_read_scope(window, &scope).await?;
            let before = crate::actions::preflight::capture(
                self.git()?,
                &location,
                lifetime.clone(),
                &cancel,
            )
            .await?;
            let mut result = self
                .history
                .containment(window, self.git()?, request, lifetime.clone())
                .await?;
            let query =
                crate::actions::catalog::ReadQuery::new(self.git()?, &location, lifetime, cancel);
            let config = crate::actions::catalog::configuration(&query).await?;
            let mut branches = result
                .branches
                .iter()
                .map(|b| b.branch.clone())
                .collect::<Vec<_>>();
            crate::actions::observations::attach(
                &self.registry.database,
                &scope.repo_id,
                &config.remotes,
                &mut branches,
            )?;
            // The history guard must still cover config/evidence reads too.
            self.history
                .validate_context(window, self.git()?, &result.context, query.lifetime.clone())
                .await?;
            for (target, branch) in result.branches.iter_mut().zip(branches) {
                target.branch.observation = branch.observation;
            }
            let after = crate::actions::preflight::capture(
                self.git()?,
                &location,
                query.lifetime.clone(),
                &query.cancel,
            )
            .await?;
            if before.fingerprint != after.fingerprint {
                return Err(error(ErrorCode::ExternalChange, Phase::Inspection));
            }
            self.actions.authority.checked(window, &scope, epoch, || {
                self.action_registration(&scope, &location)
            })?;
            crate::actions::catalog::bounded_response(&result)?;
            Ok(result)
        })
        .await
        .map_err(|_| error(ErrorCode::Timeout, Phase::Inspection))?
    }
    pub async fn prepare_repository_action(
        &self,
        window: &str,
        request: PrepareRepositoryActionRequest,
    ) -> Result<PreparedAction, AppError> {
        #[cfg(windows)]
        self.clones.check_ready()?;
        validation::request_size(&request)?;
        crate::actions::authority::validate(&request.intent)?;
        match &request.intent {
            ActionIntent::Fetch { remote_handles, .. }
                if remote_handles.len() <= crate::actions::fetch::REMOTES_MAX => {}
            ActionIntent::Fetch { .. } => {
                return Err(error(ErrorCode::OutputLimit, Phase::Validation))
            }
            ActionIntent::ReadRemoteRefs { .. } => {}
            ActionIntent::Stage { .. }
            | ActionIntent::Unstage { .. }
            | ActionIntent::Commit { amend: false, .. } => {}
            intent if crate::actions::sync::supported(intent.kind()) => {}
            intent if crate::actions::ref_catalog::supported(intent.kind()) => {}
            _ => return Err(error(ErrorCode::UnsupportedAction, Phase::Validation)),
        }
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            Box::pin(async {
                let access = if crate::actions::changes::supported(request.intent.kind())
                    || crate::actions::sync::supported(request.intent.kind())
                {
                    self.window(window)?;
                    let epoch = self.actions.authority.read_epoch(window, &request.scope)?;
                    let location = self
                        .registry
                        .location(&request.scope.repo_id, request.scope.worktree_id.as_deref())?;
                    self.action_read_scope_existing(window, &request.scope, location, epoch)
                        .await?
                } else {
                    self.action_read_scope(window, &request.scope).await?
                };
                {
                    let query = crate::actions::catalog::ReadQuery::new(
                        self.git()?,
                        &access.location,
                        access.lifetime.clone(),
                        access.cancel.clone(),
                    );
                    let format = query
                        .run(&argv(&["rev-parse", "--show-object-format"]))
                        .await?;
                    let length = match format.as_slice() {
                        b"sha1\n" => 40,
                        b"sha256\n" => 64,
                        _ => return Err(error(ErrorCode::MalformedOutput, Phase::Inspection)),
                    };
                    if matches!(&request.intent, ActionIntent::Fetch { .. }) {
                        crate::actions::fetch::capability(&query, length).await?;
                    }
                    self.actions
                        .authority
                        .checked(window, &request.scope, access.epoch, || {
                            self.action_registration(&request.scope, &access.location)
                        })?;
                }
                #[cfg(test)]
                {
                    let pause = { self.action_test.prepare_pause.lock().unwrap().take() };
                    if let Some(pause) = pause {
                        pause.entered.notify_one();
                        pause.release.notified().await;
                    }
                }
                // Keep the original era and pure scope lease. Never re-begin a
                // request after capability work or reacquire its own common mutex.
                self.prepare_action_scoped(window, request, access, true)
                    .await
            }),
        )
        .await
        .map_err(|_| error(ErrorCode::Timeout, Phase::Inspection))?
    }
    #[cfg(windows)]
    pub fn execute_repository_action(
        self: &Arc<Self>,
        window: &str,
        request: ExecuteRepositoryActionRequest,
    ) -> Result<OperationSnapshot, AppError> {
        self.clones.check_ready()?;
        self.window(window)?;
        validation::request_size(&request)?;
        let kind = self
            .actions
            .authority
            .plan_kind(window, &request.plan_id, &self.scheduler)?;
        if !matches!(kind, ActionKind::Fetch | ActionKind::ReadRemoteRefs)
            && !crate::actions::ref_catalog::supported(kind)
            && !crate::actions::changes::supported(kind)
            && !crate::actions::sync::supported(kind)
        {
            return Err(error(ErrorCode::UnsupportedAction, Phase::Validation));
        }
        let execution = self.consume_action(window, request)?;
        let operation = execution.operation.clone();
        if execution.plan.is_none() {
            return Ok(operation);
        }
        let backend = self.clone();
        let window = window.to_owned();
        tokio::spawn(async move {
            let database = backend.registry.database.clone();
            let auth = backend.auth.clone();
            let publication_location = execution
                .plan
                .as_ref()
                .expect("consumed action")
                .location
                .clone();
            let publisher = Arc::downgrade(&backend);
            let publish: crate::actions::remote::Publisher = Arc::new(move |value| {
                publisher
                    .upgrade()
                    .ok_or_else(crate::actions::remote::invalid)?
                    .publish_remote_snapshot(&publication_location, value)
            });
            let owner = window.clone();
            let _result = backend
                .run_action(&window, execution, move |context| async move {
                    if context.intent.kind() == ActionKind::ReadRemoteRefs {
                        Box::pin(crate::actions::remote::execute(
                            context, auth, owner, publish,
                        ))
                        .await
                    } else if context.intent.kind() == ActionKind::Fetch {
                        crate::actions::fetch::execute(context, database, auth, owner).await
                    } else if matches!(
                        context.intent,
                        ActionIntent::Stage { .. } | ActionIntent::Unstage { .. }
                    ) {
                        Box::pin(crate::actions::index::execute(context)).await
                    } else if context.intent.kind() == ActionKind::Commit {
                        Box::pin(crate::actions::commit::execute(context)).await
                    } else if crate::actions::sync::supported(context.intent.kind()) {
                        Box::pin(crate::actions::sync::execute(
                            context, database, auth, owner,
                        ))
                        .await
                    } else {
                        let proof =
                            crate::actions::journal::read(&database, &context.operation_id)?
                                .ref_action
                                .ok_or_else(|| {
                                    error(ErrorCode::InvalidHandle, Phase::Validation)
                                })?;
                        Box::pin(crate::actions::branches::execute(context, proof)).await
                    }
                })
                .await;
            #[cfg(test)]
            {
                let pause = backend
                    .action_test
                    .after_terminal_pause
                    .lock()
                    .unwrap()
                    .take();
                if let Some(pause) = pause {
                    pause.entered.notify_one();
                    pause.release.notified().await;
                }
            }
            #[cfg(test)]
            if let Some(done) = backend.action_test.completion_done.lock().unwrap().take() {
                done.notify_one();
            }
        });
        Ok(operation)
    }
    pub fn get_action_outcome(
        &self,
        window: &str,
        request: GetActionOutcomeRequest,
    ) -> Result<ActionOperationResult, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        if self
            .scheduler
            .action_owner(&request.operation_id)?
            .as_deref()
            != Some(window)
        {
            return Err(error(ErrorCode::InvalidHandle, Phase::Validation));
        }
        let result = ActionOperationResult {
            operation: self.scheduler.get(&request.operation_id)?,
            fetch: crate::actions::observations::read(
                &self.registry.database,
                &request.operation_id,
            )?,
        };
        crate::actions::catalog::bounded_response(&result)?;
        Ok(result)
    }
    /// Trusted catalogs issue typed targets. There is deliberately no IPC grant command.
    pub async fn grant_action_target(
        &self,
        window: &str,
        scope: &ActionScope,
        target: crate::actions::authority::Target,
    ) -> Result<String, AppError> {
        self.window(window)?;
        let location = self
            .registry
            .location(&scope.repo_id, scope.worktree_id.as_deref())?;
        let epoch = self
            .actions
            .authority
            .begin(window, scope, || self.action_registration(scope, &location))?;
        let slot = self.actions.reserve()?;
        let cancel = Cancellation::default();
        let serialization = self
            .scheduler
            .acquire_scope(
                crate::actions::lock_keys(&location),
                false,
                &cancel,
                Some(std::time::Duration::from_secs(30)),
            )
            .await?;
        self.actions.authority.checked(window, scope, epoch, || {
            self.action_registration(scope, &location)
        })?;
        let permit = crate::actions::read_permit(&self.actions.reads)?;
        let pins = crate::actions::preflight::Pins::open(&location)?;
        let scope_guard = Arc::new(crate::actions::ReadScope {
            pins,
            serialization,
            _slot: slot,
        });
        let lease = crate::actions::read_lease(scope_guard, permit);
        let state = crate::actions::preflight::capture(
            self.git()?,
            &location,
            lease,
            &Cancellation::default(),
        )
        .await?;
        self.actions
            .authority
            .issue(window, scope, epoch, target, state.fingerprint, || {
                self.action_registration(scope, &location)
            })
    }
    pub async fn prepare_action(
        &self,
        window: &str,
        request: PrepareRepositoryActionRequest,
    ) -> Result<PreparedAction, AppError> {
        validation::request_size(&request)?;
        crate::actions::authority::validate(&request.intent)?;
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let access = self.action_read_scope(window, &request.scope).await?;
            // Trusted kernel seam retained for dependency executors/tests.
            // The registered IPC command and real executor enforce S5 eligibility.
            self.prepare_action_scoped(window, request, access, false)
                .await
        })
        .await
        .map_err(|_| error(ErrorCode::Timeout, Phase::Inspection))?
    }
    async fn prepare_action_scoped(
        &self,
        window: &str,
        request: PrepareRepositoryActionRequest,
        access: ActionReadAccess,
        inspect_changes: bool,
    ) -> Result<PreparedAction, AppError> {
        let ActionReadAccess {
            location,
            epoch,
            cancel,
            lifetime,
        } = access;
        self.actions
            .authority
            .checked(window, &request.scope, epoch, || {
                self.window(window)?;
                #[cfg(windows)]
                self.clones.check_ready()?;
                self.action_registration(&request.scope, &location)
            })?;
        let before =
            crate::actions::preflight::capture(self.git()?, &location, lifetime.clone(), &cancel)
                .await?;
        crate::actions::preflight::policy(&request.intent, &before)?;
        if matches!(request.intent, ActionIntent::ReadRemoteRefs { .. }) {
            let targets = self.actions.authority.resolved_targets(
                window,
                &request.scope,
                epoch,
                &request.intent,
                &before.fingerprint,
            )?;
            let q = crate::actions::catalog::ReadQuery::new(
                self.git()?,
                &location,
                lifetime.clone(),
                cancel.clone(),
            );
            let endpoints = crate::actions::remote::endpoints(&q).await?;
            crate::actions::remote::select(
                &endpoints,
                targets
                    .first()
                    .ok_or_else(crate::actions::remote::invalid)?,
            )?;
        }
        let ref_action = if crate::actions::ref_catalog::supported(request.intent.kind()) {
            let targets = self.actions.authority.resolved_targets(
                window,
                &request.scope,
                epoch,
                &request.intent,
                &before.fingerprint,
            )?;
            let query = crate::actions::catalog::ReadQuery::new(
                self.git()?,
                &location,
                lifetime.clone(),
                cancel.clone(),
            );
            Some(crate::actions::branches::prepare(&query, &request.intent, &targets).await?)
        } else {
            None
        };
        if inspect_changes && crate::actions::changes::supported(request.intent.kind()) {
            let targets = self.actions.authority.resolved_targets(
                window,
                &request.scope,
                epoch,
                &request.intent,
                &before.fingerprint,
            )?;
            crate::actions::changes::validate_targets(&location, &request.intent, &targets)?;
            let query = crate::actions::catalog::ReadQuery::new(
                self.git()?,
                &location,
                lifetime.clone(),
                cancel.clone(),
            );
            crate::actions::commit::prepare(&query, &request.intent).await?;
            if before.unmerged {
                return Err(error(ErrorCode::UnsupportedAction, Phase::Validation));
            }
        }
        if inspect_changes && crate::actions::sync::supported(request.intent.kind()) {
            let targets = self.actions.authority.resolved_targets(
                window,
                &request.scope,
                epoch,
                &request.intent,
                &before.fingerprint,
            )?;
            let q = crate::actions::catalog::ReadQuery::new(
                self.git()?,
                &location,
                lifetime.clone(),
                cancel.clone(),
            );
            crate::actions::sync::prepare(&q, &request.intent, &targets, &before.fingerprint)
                .await?;
        }
        let after =
            crate::actions::preflight::capture(self.git()?, &location, lifetime, &cancel).await?;
        if before.fingerprint != after.fingerprint {
            return Err(error(ErrorCode::ExternalChange, Phase::Validation));
        }
        crate::actions::preflight::policy(&request.intent, &before)?;
        self.actions.authority.prepare(
            window,
            crate::actions::authority::Plan {
                scope: request.scope.clone(),
                intent: request.intent,
                location: location.clone(),
                preflight: before,
                targets: vec![],
                ref_action,
                execution_ticket: None,
                epoch,
            },
            || {
                self.window(window)?;
                #[cfg(windows)]
                self.clones.check_ready()?;
                self.action_registration(&request.scope, &location)
            },
        )
    }
    pub fn consume_action(
        &self,
        window: &str,
        request: ExecuteRepositoryActionRequest,
    ) -> Result<crate::actions::Execution, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        let mut slot = None;
        let (operation, plan) =
            self.actions
                .authority
                .consume(window, &request.plan_id, &self.scheduler, |plan| {
                    self.action_registration(&plan.scope, &plan.location)?;
                    slot = Some(self.actions.reserve()?);
                    Ok(())
                })?;
        let guard = plan.as_ref().map(|_| crate::actions::ExecutionGuard {
            scheduler: self.scheduler.clone(),
            id: operation.id.clone(),
            armed: true,
            before_finish: None,
        });
        Ok(crate::actions::Execution {
            operation,
            plan,
            slot,
            guard,
        })
    }
    /// Internal dependency seam; no action executor/IPC is registered by S1.
    pub async fn run_action<F, Fut>(
        self: &Arc<Self>,
        window: &str,
        mut execution: crate::actions::Execution,
        job: F,
    ) -> Result<OperationSnapshot, AppError>
    where
        F: FnOnce(crate::actions::ActionContext) -> Fut,
        Fut: std::future::Future<
            Output = Result<crate::operations::scheduler::Completion, AppError>,
        >,
    {
        self.window(window)?;
        if self
            .scheduler
            .action_owner(&execution.operation.id)?
            .as_deref()
            != Some(window)
        {
            return Err(error(ErrorCode::InvalidHandle, Phase::Validation));
        }
        let Some(plan) = execution.plan.take() else {
            return self.scheduler.get(&execution.operation.id);
        };
        let mut guard = execution
            .guard
            .take()
            .ok_or_else(|| error(ErrorCode::InvalidHandle, Phase::Validation))?;
        let slot = execution
            .slot
            .take()
            .ok_or_else(|| error(ErrorCode::InvalidHandle, Phase::Validation))?;
        let pins = crate::actions::preflight::Pins::open(&plan.location)?;
        let reads = self.actions.reads.clone();
        let fetch_timeout = self.actions.fetch_timeout;
        let keys = crate::actions::lock_keys(&plan.location);
        let network = crate::actions::network(&plan.intent);
        let service = Arc::downgrade(self);
        let scope = plan.scope.clone();
        let location = plan.location.clone();
        let ticket = plan
            .execution_ticket
            .clone()
            .ok_or_else(|| error(ErrorCode::InvalidHandle, Phase::Validation))?;
        let window = window.to_owned();
        let write_scheduler = self.scheduler.clone();
        let write_id = execution.operation.id.clone();
        let validate = Arc::new(move |gate: crate::actions::Gate| {
            let backend = service
                .upgrade()
                .ok_or_else(|| error(ErrorCode::InvalidHandle, Phase::Validation))?;
            backend
                .actions
                .authority
                .checked_execution(&window, &scope, &ticket, || {
                    backend.window(&window)?;
                    #[cfg(windows)]
                    backend.clones.check_ready()?;
                    backend.action_registration(&scope, &location)?;
                    let registration = |connection: &rusqlite::Connection| {
                        Self::check_watch_registration(
                            connection,
                            &HistoryScope {
                                repo_id: scope.repo_id.clone(),
                                worktree_id: scope.worktree_id.clone(),
                                generation: scope.generation.clone(),
                            },
                            &location,
                        )
                    };
                    match gate {
                        crate::actions::Gate::ReadStart(role) => {
                            write_scheduler.action_read_start_checked(&write_id, role, registration)
                        }
                        crate::actions::Gate::Write => {
                            write_scheduler.action_before_write_checked(&write_id, registration)
                        }
                        crate::actions::Gate::FetchStep(index, digest) => write_scheduler
                            .action_fetch_step_checked(
                                &write_id,
                                index,
                                std::str::from_utf8(&digest).map_err(|_| {
                                    error(ErrorCode::InvalidRequest, Phase::Validation)
                                })?,
                                registration,
                            ),
                        crate::actions::Gate::RefStep(index, digest) => write_scheduler
                            .action_ref_step_checked(
                                &write_id,
                                index,
                                std::str::from_utf8(&digest).map_err(|_| {
                                    error(ErrorCode::InvalidRequest, Phase::Validation)
                                })?,
                                registration,
                            ),
                        crate::actions::Gate::Check => Ok(true),
                        crate::actions::Gate::SyncStep(index, digest) => write_scheduler
                            .action_sync_step_checked(
                                &write_id,
                                index,
                                std::str::from_utf8(&digest).map_err(|_| {
                                    error(ErrorCode::InvalidRequest, Phase::Validation)
                                })?,
                                registration,
                            ),
                    }
                })
        });
        let scheduler = self.scheduler.clone();
        let runner = self.git()?.clone();
        let id = execution.operation.id.clone();
        let weak = Arc::downgrade(self);
        let repo = plan.scope.repo_id.clone();
        let invalidate: crate::operations::scheduler::CompletionInvalidation =
            Arc::new(move |effects| {
                if effects != ActionEffects::NotApplied {
                    if let Some(backend) = weak.upgrade() {
                        backend.actions.authority.revoke_repo(&repo);
                        backend.history.sessions.revoke_repo(&repo);
                        backend.actions.ref_catalogs.revoke_repo(&repo);
                        backend.actions.change_catalogs.revoke_repo(&repo);
                        backend.actions.remote_snapshots.revoke_repo(&repo);
                    }
                }
            });
        guard.before_finish = Some(invalidate.clone());
        #[cfg(test)]
        let index_test = self.actions.index_test.clone();
        #[cfg(test)]
        let sync_test = self.actions.sync_test.clone();
        let result = self
            .scheduler
            .execute_action_with_invalidation(
                &execution.operation.id,
                keys,
                network,
                invalidate,
                move |cancel, serialization| async move {
                    let resources = Arc::new(crate::actions::Resources {
                        #[cfg(test)]
                        index_test,
                        #[cfg(test)]
                        sync_test,
                        fetch_timeout,
                        pins,
                        serialization,
                        reads,
                        _slot: slot,
                    });
                    validate(crate::actions::Gate::Check)?;
                    let lease = crate::actions::read_lease(
                        resources.clone(),
                        crate::actions::read_permit(&resources.reads)?,
                    );
                    let state =
                        crate::actions::preflight::capture(&runner, &plan.location, lease, &cancel)
                            .await?;
                    if state.fingerprint != plan.preflight.fingerprint {
                        return Err(error(ErrorCode::ExternalChange, Phase::Validation));
                    }
                    crate::actions::preflight::policy(&plan.intent, &state)?;
                    validate(crate::actions::Gate::Check)?;
                    job(crate::actions::context(
                        id, plan, cancel, resources, scheduler, runner, validate,
                    ))
                    .await
                },
            )
            .await;
        guard.armed = result.is_err();
        result
    }
    pub async fn get_environment(&self) -> Result<Environment, AppError> {
        Ok(environment::environment(self.runner.as_ref()).await)
    }
    pub fn get_workspace(&self, window: &str) -> Result<Workspace, AppError> {
        validation::scope(window)?;
        self.windows
            .lock()
            .map_err(|_| error(ErrorCode::Internal, Phase::Validation))?
            .insert(window.into());
        let _ = self.actions.authority.open_window(window);
        self.history.sessions.open_window(window);
        self.registry.workspace()
    }
    fn window(&self, window: &str) -> Result<(), AppError> {
        validation::scope(window)?;
        if self
            .windows
            .lock()
            .map_err(|_| error(ErrorCode::Internal, Phase::Validation))?
            .contains(window)
        {
            Ok(())
        } else {
            Err(error(ErrorCode::InvalidHandle, Phase::Validation))
        }
    }
    pub fn close_window(&self, window: &str) {
        self.actions.authority.revoke(window, true);
        self.actions.ref_catalogs.revoke(window);
        self.actions.change_catalogs.revoke(window);
        self.actions.remote_snapshots.revoke(window);
        self.scheduler.cancel_owned_actions(window, None);
        self.history.sessions.revoke_window(window, true);
        self.watches.revoke_window(window, true);
        #[cfg(windows)]
        self.auth.close_window(window);
        #[cfg(windows)]
        {
            self.clones.close_window(window);
            let operations = self
                .clones
                .controls
                .lock()
                .map(|controls| {
                    controls
                        .iter()
                        .filter(|(_, control)| control.window == window)
                        .map(|(id, control)| (id.clone(), control.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for (id, control) in operations {
                if let Ok(_gate) = control.gate.lock() {
                    if !control.published.load(std::sync::atomic::Ordering::SeqCst) {
                        let _ = self.scheduler.cancel(&id);
                    }
                }
            }
        }
        if let Ok(mut windows) = self.windows.lock() {
            windows.remove(window);
        }
    }
    pub fn git(&self) -> Result<&GitRunner, AppError> {
        self.runner
            .as_ref()
            .ok_or_else(|| error(ErrorCode::GitUnavailable, Phase::Discovery))
    }
    /// `path` comes only from the desktop OS dialog, never a renderer path string.
    pub async fn register_repository(
        &self,
        window: &str,
        path: &Path,
    ) -> Result<Workspace, AppError> {
        validation::scope(window)?;
        self.actions.authority.revoke(window, false);
        self.actions.ref_catalogs.revoke(window);
        self.actions.change_catalogs.revoke(window);
        self.actions.remote_snapshots.revoke(window);
        self.scheduler.cancel_owned_actions(window, None);
        let item = discovery::discover(self.git()?, path).await?;
        let workspace = self.registry.register(&item)?;
        self.windows
            .lock()
            .map_err(|_| error(ErrorCode::Internal, Phase::Validation))?
            .insert(window.into());
        self.actions.authority.revoke(window, false);
        self.actions.ref_catalogs.revoke(window);
        self.actions.change_catalogs.revoke(window);
        self.actions.remote_snapshots.revoke(window);
        self.scheduler.cancel_owned_actions(window, None);
        self.history.sessions.revoke_window(window, false);
        self.watches.revoke_window(window, false);
        let _ = self.actions.authority.open_window(window);
        self.history.sessions.open_window(window);
        Ok(workspace)
    }
    pub fn select_repository(
        &self,
        window: &str,
        request: SelectRepositoryRequest,
    ) -> Result<Workspace, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        self.actions.authority.revoke(window, false);
        self.actions.ref_catalogs.revoke(window);
        self.actions.change_catalogs.revoke(window);
        self.actions.remote_snapshots.revoke(window);
        self.scheduler.cancel_owned_actions(window, None);
        let result = self.registry.select(&request)?;
        self.history.sessions.revoke_window(window, false);
        self.watches.revoke_window(window, false);
        Ok(result)
    }
    pub fn forget_repository(
        &self,
        window: &str,
        request: ForgetRepositoryRequest,
    ) -> Result<Workspace, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        self.actions.authority.revoke_repo(&request.repo_id);
        self.actions.ref_catalogs.revoke_repo(&request.repo_id);
        self.actions.change_catalogs.revoke_repo(&request.repo_id);
        self.actions.remote_snapshots.revoke_repo(&request.repo_id);
        self.scheduler.cancel_repo_actions(&request.repo_id);
        let result = self.registry.forget(&request.repo_id)?;
        self.history.sessions.revoke_repo(&request.repo_id);
        self.watches.revoke_repo(&request.repo_id);
        Ok(result)
    }
    pub fn get_operations(&self, window: &str) -> Result<OperationsSnapshot, AppError> {
        #[cfg(windows)]
        self.clones.check_ready()?;
        // Snapshot subscription may precede workspace loading at application startup.
        validation::scope(window)?;
        self.windows
            .lock()
            .map_err(|_| error(ErrorCode::Internal, Phase::Validation))?
            .insert(window.into());
        let _ = self.actions.authority.open_window(window);
        self.history.sessions.open_window(window);
        self.scheduler.snapshot()
    }
    pub fn cancel_operation(
        &self,
        window: &str,
        request: CancelOperationRequest,
    ) -> Result<OperationSnapshot, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        if self
            .scheduler
            .action_owner(&request.operation_id)?
            .is_some_and(|owner| owner != window)
        {
            return Err(error(ErrorCode::InvalidHandle, Phase::Validation));
        }
        #[cfg(windows)]
        let control = self
            .clones
            .controls
            .lock()
            .map_err(|_| error(ErrorCode::Internal, Phase::Queue))?
            .get(&request.operation_id)
            .cloned();
        #[cfg(windows)]
        let _gate = control
            .as_ref()
            .map(|control| {
                control
                    .gate
                    .lock()
                    .map_err(|_| error(ErrorCode::Internal, Phase::Queue))
            })
            .transpose()?;
        #[cfg(windows)]
        if let Some(control) = &control {
            if control.window != window {
                return Err(error(ErrorCode::InvalidHandle, Phase::Validation));
            }
            if control.published.load(std::sync::atomic::Ordering::SeqCst) {
                self.clones.check_ready()?;
                return self
                    .scheduler
                    .snapshot()?
                    .operations
                    .into_iter()
                    .find(|operation| operation.id == request.operation_id)
                    .ok_or_else(|| error(ErrorCode::InvalidHandle, Phase::Validation));
            }
        }
        #[cfg(windows)]
        self.auth.invalidate(&request.operation_id);
        self.scheduler.cancel(&request.operation_id)
    }
    pub fn respond_auth_prompt(
        &self,
        window: &str,
        request: RespondAuthPromptRequest,
    ) -> Result<(), AppError> {
        self.window(window)?;
        if let Err(error) = validation::request_size(&request) {
            use zeroize::Zeroize;
            let mut request = request;
            request.value.zeroize();
            return Err(error);
        }
        let owner_check = match self.scheduler.action_owner(&request.operation_id) {
            Ok(Some(owner)) if owner != window => {
                Err(error(ErrorCode::InvalidHandle, Phase::Validation))
            }
            Err(e) => Err(e),
            _ => Ok(()),
        };
        if let Err(error) = owner_check {
            use zeroize::Zeroize;
            let mut request = request;
            request.value.zeroize();
            return Err(error);
        }
        #[cfg(windows)]
        {
            self.auth.respond(window, request)
        }
        #[cfg(not(windows))]
        {
            let _ = request;
            Err(error(ErrorCode::Internal, Phase::Authentication))
        }
    }
    #[cfg(windows)]
    pub async fn grant_clone_source(
        &self,
        window: &str,
        path: &Path,
    ) -> Result<LocalCloneSource, AppError> {
        self.window(window)?;
        let item = discovery::discover(self.git()?, path).await?;
        let root = item.root.unwrap_or(item.common.clone());
        self.clones
            .source(window, root, item.common, item.common_identity)
    }
    #[cfg(windows)]
    pub fn grant_clone_destination(
        &self,
        window: &str,
        path: &Path,
    ) -> Result<CloneDestination, AppError> {
        self.window(window)?;
        self.clones.destination(window, path)
    }
    #[cfg(windows)]
    pub async fn start_clone(
        self: &Arc<Self>,
        window: &str,
        request: StartCloneRequest,
    ) -> Result<OperationSnapshot, AppError> {
        use crate::{
            clone::{execute, grants::Control, journal, request as validate},
            operations::scheduler::{LockKey, OperationSpec},
        };
        self.window(window)?;
        self.clones.check_ready()?;
        validation::request_size(&request)?;
        validate::directory_name(&request.directory_name)?;
        crate::git::version::ensure_supported(self.git()?).await?;
        let parent = self
            .clones
            .get_destination(window, &request.destination_handle)?;
        let _ = crate::clone::path::git_path(&parent.path)?;
        if std::fs::symlink_metadata(parent.path.join(&request.directory_name)).is_ok() {
            return Err(error(ErrorCode::DestinationExists, Phase::Validation));
        }
        let (source, display, local) = match request.source {
            CloneSource::Remote { url } => {
                let display = validate::remote(&url)?;
                (std::ffi::OsString::from(url), display, None)
            }
            CloneSource::Local { source_handle } => {
                let source = self.clones.get_source(window, &source_handle)?;
                (
                    crate::clone::path::git_path(&source.path)?,
                    "Local repository".into(),
                    Some(source),
                )
            }
        };
        let mut reservation = parent.identity.clone();
        reservation.extend_from_slice(request.directory_name.to_lowercase().as_bytes());
        let mut controls = self
            .clones
            .controls
            .lock()
            .map_err(|_| error(ErrorCode::Internal, Phase::Queue))?;
        let active = self
            .scheduler
            .snapshot()?
            .operations
            .into_iter()
            .filter(|operation| !crate::operations::state::terminal(&operation.state))
            .map(|operation| operation.id)
            .collect::<std::collections::HashSet<_>>();
        if controls.iter().any(|(id, control)| {
            active.contains(id)
                && control.parent_identity == parent.identity
                && crate::clone::grants::same_leaf(&control.leaf, &request.directory_name)
        }) {
            return Err(error(ErrorCode::DestinationBusy, Phase::Queue));
        }
        let operation = self.scheduler.enqueue(OperationSpec {
            source_display: display,
            destination_display: parent
                .path
                .join(&request.directory_name)
                .to_string_lossy()
                .into(),
            reservation: Some(reservation),
        })?;
        let record = execute::record(&parent, &operation.id, &request.directory_name);
        if journal::write(&self.registry.database, &record).is_err() {
            self.clones.fail_closed();
            return Err(error(ErrorCode::Database, Phase::Recovery));
        }
        let control = Arc::new(Control {
            window: window.into(),
            parent_identity: parent.identity.clone(),
            leaf: request.directory_name.clone(),
            gate: std::sync::Mutex::new(()),
            published: std::sync::atomic::AtomicBool::new(false),
        });
        controls.insert(operation.id.clone(), control.clone());
        drop(controls);
        let keys = local
            .as_ref()
            .map(|source| {
                vec![LockKey {
                    common: source.common_identity.clone(),
                    worktree: None,
                }]
            })
            .unwrap_or_default();
        let interactive = local.is_none();
        let prepared = execute::Prepared {
            parent,
            source,
            source_guard: local,
            control,
            record,
            interactive,
        };
        let backend = self.clone();
        let id = operation.id.clone();
        tokio::spawn(async move {
            let service = backend.clone();
            let result = backend
                .scheduler
                .execute(&id, keys, interactive, move |cancel| {
                    execute::run(service, prepared, cancel)
                })
                .await;
            if result.is_err() {
                backend.clones.fail_closed();
            }
        });
        Ok(operation)
    }
    pub async fn open_history(
        &self,
        window: &str,
        request: OpenHistoryRequest,
    ) -> Result<HistorySession, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        validation::counter(&request.scope.generation)?;
        let location = self
            .registry
            .location(&request.scope.repo_id, request.scope.worktree_id.as_deref())?;
        self.history
            .open(window, self.git()?, location, request)
            .await
    }
    pub fn subscribe_repository_changes(
        &self,
    ) -> tokio::sync::broadcast::Receiver<crate::history::watch::RoutedChange> {
        self.watches.subscribe()
    }
    fn check_watch_registration(
        connection: &rusqlite::Connection,
        scope: &HistoryScope,
        location: &crate::repositories::registry::RepositoryLocation,
    ) -> Result<(), AppError> {
        use crate::persistence::workspace::{db_error, encode};
        use rusqlite::OptionalExtension;
        let invalid = || error(ErrorCode::InvalidHandle, Phase::Validation);
        // Registered identities and paths must still match the captured scope.
        // Physical availability is deliberately left to the watcher: missing
        // or inaccessible directories retain the reviewed fallback behavior.
        let repository: Option<(Vec<u8>, Vec<u8>, bool)> = connection
            .query_row(
                "SELECT identity,common_path,bare FROM repositories WHERE id=?1",
                [&scope.repo_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|_| db_error())?;
        if repository
            != Some((
                location.common_identity.clone(),
                encode(&location.common),
                location.bare,
            ))
        {
            return Err(invalid());
        }
        match &scope.worktree_id {
            Some(id) if !location.bare => {
                let tree: Option<(Vec<u8>, Vec<u8>, Vec<u8>)> = connection
                    .query_row(
                        "SELECT identity,root,git_dir FROM worktrees WHERE id=?1 AND repo_id=?2",
                        rusqlite::params![id, scope.repo_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()
                    .map_err(|_| db_error())?;
                let identity = location.root_identity.as_ref().ok_or_else(invalid)?;
                if tree
                    != Some((
                        identity.clone(),
                        encode(&location.root),
                        encode(&location.git_dir),
                    ))
                {
                    return Err(invalid());
                }
            }
            None if location.bare => (),
            _ => return Err(invalid()),
        }
        let active: (Option<String>, Option<String>) = connection
            .query_row(
                "SELECT active_repo,active_worktree FROM workspace",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| db_error())?;
        if active != (Some(scope.repo_id.clone()), scope.worktree_id.clone()) {
            return Err(invalid());
        }
        Ok(())
    }
    pub async fn watch_repository(
        &self,
        window: &str,
        request: WatchRepositoryRequest,
    ) -> Result<RepositoryWatch, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        validation::counter(&request.scope.generation)?;
        let location = self
            .registry
            .location(&request.scope.repo_id, request.scope.worktree_id.as_deref())?;
        let selection = self.registry.workspace()?.active_selection;
        if selection.as_ref().is_none_or(|s| {
            s.repo_id != request.scope.repo_id || s.worktree_id != request.scope.worktree_id
        }) {
            return Err(error(ErrorCode::InvalidHandle, Phase::Validation));
        }
        #[cfg(test)]
        self.watch_test.pause(true).await;
        let ticket = self
            .history
            .sessions
            .begin_checked(window, &request.scope, || {
                let connection = self.registry.database.lock()?;
                Self::check_watch_registration(&connection, &request.scope, &location)
            })?;
        let (token, ready) = self.history.sessions.with_ticket(&ticket, || {
            let connection = self.registry.database.lock()?;
            Self::check_watch_registration(&connection, &request.scope, &location)?;
            self.watches
                .start(window, request.scope.clone(), location.clone())
        })?;
        #[cfg(test)]
        self.watch_test.pause(false).await;
        let result = match self.watches.finish(window, token.clone(), ready).await {
            Ok(value) => self.history.sessions.with_ticket(&ticket, || {
                let connection = self.registry.database.lock()?;
                Self::check_watch_registration(&connection, &request.scope, &location)?;
                Ok(value)
            }),
            Err(error) => Err(error),
        };
        if result.is_err() {
            let _ = self.watches.unwatch(window, &token);
        }
        result
    }
    pub fn unwatch_repository(
        &self,
        window: &str,
        request: UnwatchRepositoryRequest,
    ) -> Result<(), AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        self.watches.unwatch(window, &request.subscription_id)
    }
    pub async fn get_commit_details(
        &self,
        window: &str,
        request: GetCommitDetailsRequest,
    ) -> Result<CommitDetails, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        self.registry.location(
            &request.context.scope.repo_id,
            request.context.scope.worktree_id.as_deref(),
        )?;
        self.history.details(window, self.git()?, request).await
    }
    pub async fn get_commit_diff(
        &self,
        window: &str,
        request: GetCommitDiffRequest,
    ) -> Result<CommitDiff, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        self.registry.location(
            &request.context.scope.repo_id,
            request.context.scope.worktree_id.as_deref(),
        )?;
        self.history.diff(window, self.git()?, request).await
    }
    pub fn get_repository_view(
        &self,
        window: &str,
        request: GetRepositoryViewRequest,
    ) -> Result<RepositoryViewState, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        let location = self
            .registry
            .location(&request.scope.repo_id, request.scope.worktree_id.as_deref())?;
        {
            let connection = self.registry.database.lock()?;
            crate::persistence::views::check_scope(&connection, &request.scope, &location)?;
        }
        let ticket = self.history.sessions.begin(window, &request.scope)?;
        self.history.sessions.with_ticket(&ticket, || {
            crate::persistence::views::get(&self.registry.database, &request.scope, &location)
        })
    }
    pub fn save_repository_view(
        &self,
        window: &str,
        request: SaveRepositoryViewRequest,
    ) -> Result<RepositoryViewState, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        validation::counter(&request.expected_revision)?;
        let location = self
            .registry
            .location(&request.scope.repo_id, request.scope.worktree_id.as_deref())?;
        {
            let connection = self.registry.database.lock()?;
            crate::persistence::views::check_scope(&connection, &request.scope, &location)?;
        }
        crate::persistence::views::validate(&request.view)?;
        let ticket = self.history.sessions.begin(window, &request.scope)?;
        self.history.sessions.with_ticket(&ticket, || {
            crate::persistence::views::save(&self.registry.database, &request, &location)
        })
    }
    pub async fn load_history_page(
        &self,
        window: &str,
        request: LoadHistoryPageRequest,
    ) -> Result<HistoryPage, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        validation::counter(&request.context.scope.generation)?;
        self.registry.location(
            &request.context.scope.repo_id,
            request.context.scope.worktree_id.as_deref(),
        )?;
        self.history.page(window, self.git()?, request).await
    }
    pub async fn search_history(
        &self,
        window: &str,
        request: SearchHistoryRequest,
    ) -> Result<HistorySearchPage, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        validation::counter(&request.context.scope.generation)?;
        self.registry.location(
            &request.context.scope.repo_id,
            request.context.scope.worktree_id.as_deref(),
        )?;
        self.history.search(window, self.git()?, request).await
    }
    pub async fn inspect_repository(
        &self,
        window: &str,
        request: InspectRepositoryRequest,
    ) -> Result<RepositorySnapshot, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        validation::counter(&request.generation)?;
        let location = self
            .registry
            .location(&request.repo_id, request.worktree_id.as_deref())?;
        let runner = self.git()?;
        let current = discovery::discover(runner, &location.root).await?;
        if current.common_identity != location.common_identity
            || current.root_identity != location.root_identity
            || current.git_dir != location.git_dir
        {
            return Err(error(ErrorCode::ExternalChange, Phase::Inspection));
        }
        let (head, status) = if location.bare {
            let branch = runner
                .run(
                    Some(&location.root),
                    &argv(&["symbolic-ref", "--quiet", "--short", "HEAD"]),
                    &Cancellation::default(),
                    Phase::Inspection,
                )
                .await
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .map(|s| s.trim_end().to_string());
            let oid = runner
                .run(
                    Some(&location.root),
                    &argv(&["rev-parse", "--verify", "HEAD"]),
                    &Cancellation::default(),
                    Phase::Inspection,
                )
                .await
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .map(|s| s.trim_end().to_string());
            (
                Head {
                    kind: if oid.is_none() {
                        HeadKind::Unborn
                    } else if branch.is_some() {
                        HeadKind::Branch
                    } else {
                        HeadKind::Detached
                    },
                    branch,
                    oid,
                },
                Vec::new(),
            )
        } else {
            let bytes = runner
                .run(
                    Some(&location.root),
                    &argv(&[
                        "status",
                        "--porcelain=v2",
                        "-z",
                        "--branch",
                        "--untracked-files=all",
                    ]),
                    &Cancellation::default(),
                    Phase::Inspection,
                )
                .await?;
            let parsed = status::parse(&bytes)?;
            (parsed.head, parsed.entries)
        };
        let shallow = discovery::query(
            runner,
            &location.root,
            &["rev-parse", "--is-shallow-repository"],
        )
        .await?
            == "true";
        let remotes = refs::remotes(runner, &location.root).await?;
        let identity = environment::identity(runner, Some(&location.root)).await;
        let operations = if location.bare {
            Vec::new()
        } else {
            operation_detection::detect(&location.git_dir)
        };
        if discovery::identity(&location.common)? != location.common_identity
            || location.root_identity.as_ref().is_some_and(|expected| {
                discovery::identity(&location.root).as_ref() != Ok(expected)
            })
        {
            return Err(error(ErrorCode::ExternalChange, Phase::Inspection));
        }
        Ok(RepositorySnapshot {
            repo_id: request.repo_id,
            worktree_id: request.worktree_id,
            generation: request.generation,
            observed_at: chrono::Utc::now().to_rfc3339(),
            is_bare: location.bare,
            head,
            is_dirty: if location.bare {
                None
            } else {
                Some(!status.is_empty())
            },
            status,
            is_shallow: shallow,
            remotes,
            identity,
            git_operations: operations,
        })
    }
}
