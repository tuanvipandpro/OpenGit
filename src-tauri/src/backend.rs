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
pub struct Backend {
    pub registry: Arc<Registry>,
    pub scheduler: Arc<Scheduler>,
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
            runner,
            history: Default::default(),
            watches: Arc::new(Default::default()),
            #[cfg(test)]
            watch_test: Default::default(),
            windows: Mutex::new(HashSet::new()),
        })
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
        let item = discovery::discover(self.git()?, path).await?;
        let workspace = self.registry.register(&item)?;
        self.windows
            .lock()
            .map_err(|_| error(ErrorCode::Internal, Phase::Validation))?
            .insert(window.into());
        self.history.sessions.revoke_window(window, false);
        self.watches.revoke_window(window, false);
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
