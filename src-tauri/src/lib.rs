pub mod auth;
pub mod backend;
pub mod clone;
pub mod domain;
pub mod git;
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
                get_environment,
                get_workspace,
                choose_repository,
                inspect_repository,
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
