pub mod actions;
pub mod auth;
pub mod backend;
pub mod clone;
pub mod domain;
pub mod git;
pub mod history;
pub mod ipc;
pub mod operations;
pub mod persistence;
pub mod repositories;

#[cfg(feature = "desktop")]
mod desktop {
    use crate::{
        backend::Backend,
        domain::contracts::*,
        git::{environment, error::error, runner::GitRunner},
    };
    use std::{path::PathBuf, sync::Arc};
    use tauri::{Emitter, Manager, WebviewWindow};
    use tauri_plugin_dialog::DialogExt;
    type Service = Result<Arc<Backend>, AppError>;
    struct DesktopState(Service);
    fn result<T>(value: Result<T, AppError>) -> ApiResult<T> {
        match value {
            Ok(value) => ApiResult::Success(value),
            Err(error) => ApiResult::Failure(error),
        }
    }
    fn backend(app: &tauri::AppHandle) -> Result<Arc<Backend>, AppError> {
        app.try_state::<DesktopState>()
            .ok_or_else(internal)?
            .0
            .clone()
    }
    fn internal() -> AppError {
        error(ErrorCode::Internal, Phase::Validation)
    }

    #[tauri::command]
    fn get_sync_action_outcome(
        request: GetActionOutcomeRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<SyncActionOperationResult> {
        result(backend(&app).and_then(|s| s.get_sync_action_outcome(window.label(), request)))
    }
    #[tauri::command]
    async fn get_publication_evidence(
        request: GetPublicationEvidenceRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<PublicationEvidenceResult> {
        match backend(&app) {
            Ok(s) => result(s.get_publication_evidence(window.label(), request).await),
            Err(e) => ApiResult::Failure(e),
        }
    }
    #[tauri::command]
    async fn get_repository_changes(
        request: GetRepositoryChangesRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<RepositoryChanges> {
        match backend(&app) {
            Ok(s) => result(s.get_repository_changes(window.label(), request).await),
            Err(e) => ApiResult::Failure(e),
        }
    }
    #[tauri::command]
    async fn resolve_change_targets(
        request: ResolveChangeTargetsRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<ResolvedChangeTargets> {
        match backend(&app) {
            Ok(s) => result(s.resolve_change_targets(window.label(), request).await),
            Err(e) => ApiResult::Failure(e),
        }
    }
    #[tauri::command]
    async fn get_change_diff(
        request: GetChangeDiffRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<ChangeDiff> {
        match backend(&app) {
            Ok(s) => result(s.get_change_diff(window.label(), request).await),
            Err(e) => ApiResult::Failure(e),
        }
    }
    #[tauri::command]
    fn get_change_action_outcome(
        request: GetActionOutcomeRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<ChangeActionOperationResult> {
        result(backend(&app).and_then(|s| s.get_change_action_outcome(window.label(), request)))
    }
    #[tauri::command]
    async fn get_environment(
        request: EmptyRequest,
        app: tauri::AppHandle,
    ) -> ApiResult<Environment> {
        let _ = request;
        let mut value = match backend(&app) {
            Ok(service) => match service.get_environment().await {
                Ok(value) => value,
                Err(error) => return ApiResult::Failure(error),
            },
            Err(_) => {
                let runner = GitRunner::discover().ok();
                environment::environment(runner.as_ref()).await
            }
        };
        value.web_view2 = ToolInfo {
            available: true,
            version: None,
        };
        ApiResult::Success(value)
    }
    #[tauri::command]
    async fn get_remote_actions(
        request: GetRemoteActionsRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<RemoteActionCatalog> {
        match backend(&app) {
            Ok(s) => result(s.get_remote_actions(window.label(), request).await),
            Err(e) => ApiResult::Failure(e),
        }
    }
    #[tauri::command]
    async fn get_remote_action_outcome(
        request: GetActionOutcomeRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<RemoteActionOperationResult> {
        result(backend(&app).and_then(|s| s.get_remote_action_outcome(window.label(), request)))
    }
    #[tauri::command]
    async fn resolve_remote_target(
        request: ResolveRemoteTargetRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<ResolvedRemoteTarget> {
        match backend(&app) {
            Ok(s) => result(s.resolve_remote_target(window.label(), request).await),
            Err(e) => ApiResult::Failure(e),
        }
    }
    #[tauri::command]
    async fn get_ref_actions(
        request: GetRefActionsRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<RefActionCatalog> {
        match backend(&app) {
            Ok(service) => result(service.get_ref_actions(window.label(), request).await),
            Err(e) => ApiResult::Failure(e),
        }
    }
    #[tauri::command]
    async fn resolve_ref_action_target(
        request: ResolveRefActionTargetRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<ResolvedRefActionTarget> {
        match backend(&app) {
            Ok(service) => result(
                service
                    .resolve_ref_action_target(window.label(), request)
                    .await,
            ),
            Err(e) => ApiResult::Failure(e),
        }
    }
    #[tauri::command]
    async fn get_ref_action_outcome(
        request: GetActionOutcomeRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<RefActionOperationResult> {
        result(
            backend(&app)
                .and_then(|service| service.get_ref_action_outcome(window.label(), request)),
        )
    }
    #[tauri::command]
    async fn get_repository_actions(
        request: GetRepositoryActionsRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<RepositoryActionCatalog> {
        match backend(&app) {
            Ok(service) => result(
                service
                    .get_repository_actions(window.label(), request)
                    .await,
            ),
            Err(e) => ApiResult::Failure(e),
        }
    }
    #[tauri::command]
    async fn get_commit_containment(
        request: GetCommitContainmentRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<CommitContainment> {
        match backend(&app) {
            Ok(service) => result(
                service
                    .get_commit_containment(window.label(), request)
                    .await,
            ),
            Err(e) => ApiResult::Failure(e),
        }
    }
    #[tauri::command]
    async fn prepare_repository_action(
        request: PrepareRepositoryActionRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<PreparedAction> {
        match backend(&app) {
            Ok(service) => result(
                service
                    .prepare_repository_action(window.label(), request)
                    .await,
            ),
            Err(e) => ApiResult::Failure(e),
        }
    }
    #[tauri::command]
    async fn execute_repository_action(
        request: ExecuteRepositoryActionRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<OperationSnapshot> {
        result(
            backend(&app)
                .and_then(|service| service.execute_repository_action(window.label(), request)),
        )
    }
    #[tauri::command]
    async fn get_action_outcome(
        request: GetActionOutcomeRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<ActionOperationResult> {
        result(
            backend(&app).and_then(|service| service.get_action_outcome(window.label(), request)),
        )
    }
    #[tauri::command]
    async fn get_workspace(
        request: EmptyRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<Workspace> {
        let _ = request;
        result(backend(&app).and_then(|service| service.get_workspace(window.label())))
    }
    async fn pick_folder(app: tauri::AppHandle) -> Result<Option<PathBuf>, AppError> {
        #[cfg(feature = "e2e")]
        if let Some(selection) = test_folder()? {
            return Ok(selection);
        }
        tauri::async_runtime::spawn_blocking(move || {
            app.dialog()
                .file()
                .blocking_pick_folder()
                .map(|path| path.into_path().map_err(|_| internal()))
                .transpose()
        })
        .await
        .map_err(|_| internal())?
    }
    #[cfg(feature = "e2e")]
    fn test_folder() -> Result<Option<Option<PathBuf>>, AppError> {
        use std::{
            collections::VecDeque,
            sync::{Mutex, OnceLock},
        };
        type Queue = Result<Option<VecDeque<Option<PathBuf>>>, ()>;
        static QUEUE: OnceLock<Mutex<Queue>> = OnceLock::new();
        let queue = QUEUE.get_or_init(|| {
            Mutex::new(match std::env::var("OPENGIT_E2E_DIALOGS_JSON") {
                Ok(value) if value.len() <= 16 * 1024 => {
                    serde_json::from_str::<Vec<Option<PathBuf>>>(&value)
                        .ok()
                        .filter(|v| v.len() <= 128 && v.iter().flatten().all(|p| p.is_absolute()))
                        .map(|v| Some(v.into()))
                        .ok_or(())
                }
                Ok(_) => Err(()),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(_) => Err(()),
            })
        });
        match &mut *queue.lock().map_err(|_| internal())? {
            Ok(Some(queue)) => Ok(Some(queue.pop_front().flatten())),
            Ok(None) => Ok(None),
            Err(_) => Err(internal()),
        }
    }
    #[tauri::command]
    async fn choose_repository(
        request: EmptyRequest,
        app: tauri::AppHandle,
        window: WebviewWindow,
    ) -> ApiResult<Option<Workspace>> {
        let _ = request;
        let service = match backend(&app) {
            Ok(service) => service,
            Err(error) => return ApiResult::Failure(error),
        };
        match pick_folder(app).await {
            Ok(Some(path)) => result(
                service
                    .register_repository(window.label(), &path)
                    .await
                    .map(Some),
            ),
            Ok(None) => ApiResult::Success(None),
            Err(error) => ApiResult::Failure(error),
        }
    }
    #[tauri::command]
    async fn inspect_repository(
        request: InspectRepositoryRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<RepositorySnapshot> {
        let service = match backend(&app) {
            Ok(service) => service,
            Err(error) => return ApiResult::Failure(error),
        };
        result(service.inspect_repository(window.label(), request).await)
    }
    #[tauri::command]
    async fn open_history(
        request: OpenHistoryRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<HistorySession> {
        let service = match backend(&app) {
            Ok(service) => service,
            Err(error) => return ApiResult::Failure(error),
        };
        result(service.open_history(window.label(), request).await)
    }
    #[tauri::command]
    async fn load_history_page(
        request: LoadHistoryPageRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<HistoryPage> {
        let service = match backend(&app) {
            Ok(service) => service,
            Err(error) => return ApiResult::Failure(error),
        };
        result(service.load_history_page(window.label(), request).await)
    }
    #[tauri::command]
    async fn search_history(
        request: SearchHistoryRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<HistorySearchPage> {
        let service = match backend(&app) {
            Ok(service) => service,
            Err(error) => return ApiResult::Failure(error),
        };
        result(service.search_history(window.label(), request).await)
    }
    #[tauri::command]
    async fn get_commit_details(
        request: GetCommitDetailsRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<CommitDetails> {
        let service = match backend(&app) {
            Ok(service) => service,
            Err(error) => return ApiResult::Failure(error),
        };
        result(service.get_commit_details(window.label(), request).await)
    }
    #[tauri::command]
    async fn get_commit_diff(
        request: GetCommitDiffRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<CommitDiff> {
        let service = match backend(&app) {
            Ok(service) => service,
            Err(error) => return ApiResult::Failure(error),
        };
        result(service.get_commit_diff(window.label(), request).await)
    }
    #[tauri::command]
    async fn get_repository_view(
        request: GetRepositoryViewRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<RepositoryViewState> {
        result(
            backend(&app).and_then(|service| service.get_repository_view(window.label(), request)),
        )
    }
    #[tauri::command]
    async fn save_repository_view(
        request: SaveRepositoryViewRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<RepositoryViewState> {
        result(
            backend(&app).and_then(|service| service.save_repository_view(window.label(), request)),
        )
    }
    #[tauri::command]
    async fn watch_repository(
        request: WatchRepositoryRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<RepositoryWatch> {
        let service = match backend(&app) {
            Ok(service) => service,
            Err(error) => return ApiResult::Failure(error),
        };
        result(service.watch_repository(window.label(), request).await)
    }
    #[tauri::command]
    async fn unwatch_repository(
        request: UnwatchRepositoryRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<()> {
        result(
            backend(&app).and_then(|service| service.unwatch_repository(window.label(), request)),
        )
    }
    #[tauri::command]
    async fn select_repository(
        request: SelectRepositoryRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<Workspace> {
        result(backend(&app).and_then(|service| service.select_repository(window.label(), request)))
    }
    #[tauri::command]
    async fn forget_repository(
        request: ForgetRepositoryRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<Workspace> {
        result(backend(&app).and_then(|service| service.forget_repository(window.label(), request)))
    }
    #[tauri::command]
    async fn get_operations(
        request: EmptyRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<OperationsSnapshot> {
        let _ = request;
        result(backend(&app).and_then(|service| service.get_operations(window.label())))
    }
    #[tauri::command]
    async fn cancel_operation(
        request: CancelOperationRequest,
        window: WebviewWindow,
        app: tauri::AppHandle,
    ) -> ApiResult<OperationSnapshot> {
        result(backend(&app).and_then(|service| service.cancel_operation(window.label(), request)))
    }
    #[tauri::command]
    async fn choose_clone_source(
        request: EmptyRequest,
        app: tauri::AppHandle,
        window: WebviewWindow,
    ) -> ApiResult<Option<LocalCloneSource>> {
        let _ = request;
        let service = match backend(&app) {
            Ok(service) => service,
            Err(error) => return ApiResult::Failure(error),
        };
        match pick_folder(app).await {
            Ok(Some(path)) => {
                #[cfg(windows)]
                {
                    result(
                        service
                            .grant_clone_source(window.label(), &path)
                            .await
                            .map(Some),
                    )
                }
                #[cfg(not(windows))]
                {
                    let _ = (service, window, path);
                    ApiResult::Failure(internal())
                }
            }
            Ok(None) => ApiResult::Success(None),
            Err(error) => ApiResult::Failure(error),
        }
    }
    #[tauri::command]
    async fn choose_clone_destination(
        request: EmptyRequest,
        app: tauri::AppHandle,
        window: WebviewWindow,
    ) -> ApiResult<Option<CloneDestination>> {
        let _ = request;
        let service = match backend(&app) {
            Ok(service) => service,
            Err(error) => return ApiResult::Failure(error),
        };
        match pick_folder(app).await {
            Ok(Some(path)) => {
                #[cfg(windows)]
                {
                    result(
                        service
                            .grant_clone_destination(window.label(), &path)
                            .map(Some),
                    )
                }
                #[cfg(not(windows))]
                {
                    let _ = (service, window, path);
                    ApiResult::Failure(internal())
                }
            }
            Ok(None) => ApiResult::Success(None),
            Err(error) => ApiResult::Failure(error),
        }
    }
    #[tauri::command]
    async fn start_clone(
        request: StartCloneRequest,
        app: tauri::AppHandle,
        window: WebviewWindow,
    ) -> ApiResult<OperationSnapshot> {
        let service = match backend(&app) {
            Ok(service) => service,
            Err(error) => return ApiResult::Failure(error),
        };
        #[cfg(windows)]
        {
            result(service.start_clone(window.label(), request).await)
        }
        #[cfg(not(windows))]
        {
            let _ = (service, window, request);
            ApiResult::Failure(internal())
        }
    }
    #[tauri::command]
    async fn respond_auth_prompt(
        request: RespondAuthPromptRequest,
        app: tauri::AppHandle,
        window: WebviewWindow,
    ) -> ApiResult<()> {
        result(
            backend(&app).and_then(|service| service.respond_auth_prompt(window.label(), request)),
        )
    }
    pub fn run() {
        let application = tauri::Builder::default()
            .plugin(tauri_plugin_dialog::init())
            .setup(|app| {
                let directory = match std::env::var_os("OPENGIT_DATA_DIR") {
                    Some(path) => {
                        let path = PathBuf::from(path);
                        if path.is_absolute() {
                            Ok(path)
                        } else {
                            Err(error(ErrorCode::Database, Phase::Registration))
                        }
                    }
                    None => app
                        .path()
                        .app_data_dir()
                        .map_err(|_| error(ErrorCode::Database, Phase::Registration)),
                };
                let service = directory
                    .and_then(|directory| Backend::new(&directory))
                    .map(Arc::new);
                if let Ok(backend) = &service {
                    let mut changes = backend.subscribe_repository_changes();
                    let watches = Arc::downgrade(&backend.watches);
                    let event_app = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        loop {
                            match changes.recv().await {
                                Ok(change) => {
                                    let Some(watches) = watches.upgrade() else {
                                        break;
                                    };
                                    watches.forward_current(&change, || {
                                        event_app
                                            .emit_to(
                                                &change.window,
                                                "opengit://repository-changed",
                                                &change.event,
                                            )
                                            .is_ok()
                                    });
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                    if let Some(watches) = watches.upgrade() {
                                        watches.delivery_gap();
                                    } else {
                                        break;
                                    }
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                            }
                        }
                    });
                    let mut events = backend.scheduler.subscribe();
                    let app = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        loop {
                            match events.recv().await {
                                Ok(event) => {
                                    let _ = app.emit_to("main", "opengit://operation", event);
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                    continue
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                            }
                        }
                    });
                }
                app.manage(DesktopState(service));
                Ok(())
            })
            .on_window_event(|window, event| {
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    if let Some(state) = window.app_handle().try_state::<DesktopState>() {
                        if let Ok(backend) = &state.0 {
                            backend.close_window(window.label());
                        }
                    }
                }
            })
            .invoke_handler(tauri::generate_handler![
                get_sync_action_outcome,
                get_publication_evidence,
                get_repository_changes,
                resolve_change_targets,
                get_change_diff,
                get_change_action_outcome,
                get_remote_actions,
                get_remote_action_outcome,
                resolve_remote_target,
                get_ref_actions,
                resolve_ref_action_target,
                get_ref_action_outcome,
                get_environment,
                get_workspace,
                get_repository_actions,
                get_commit_containment,
                prepare_repository_action,
                execute_repository_action,
                get_action_outcome,
                choose_repository,
                inspect_repository,
                open_history,
                load_history_page,
                search_history,
                get_commit_details,
                get_commit_diff,
                get_repository_view,
                save_repository_view,
                watch_repository,
                unwatch_repository,
                select_repository,
                forget_repository,
                get_operations,
                cancel_operation,
                choose_clone_source,
                choose_clone_destination,
                start_clone,
                respond_auth_prompt
            ])
            .build(tauri::generate_context!());
        // Fatal window/runtime failures cannot be rendered; never print raw OS errors.
        if let Ok(application) = application {
            application.run(|_, _| {});
        }
    }
}
#[cfg(feature = "desktop")]
pub use desktop::run;
