mod details;
mod diff;
mod query;
pub(crate) use query::Query as ActionHistoryQuery;
pub(crate) mod refs;
mod search;
pub(crate) mod sessions;
pub mod watch;

use crate::{
    domain::contracts::*,
    git::{error::error, runner::GitRunner},
    ipc::validation,
    repositories::registry::RepositoryLocation,
};
use serde::Serialize;
use std::sync::atomic::Ordering;
use tokio::sync::{OnceCell, Semaphore};

pub struct History {
    pub sessions: sessions::Sessions,
    capability: OnceCell<bool>,
    reads: Semaphore,
}
impl Default for History {
    fn default() -> Self {
        Self {
            sessions: Default::default(),
            capability: OnceCell::new(),
            reads: Semaphore::new(8),
        }
    }
}
fn invalid_handle() -> AppError {
    error(ErrorCode::InvalidHandle, Phase::Validation)
}
fn invalid() -> AppError {
    error(ErrorCode::InvalidRequest, Phase::Validation)
}
fn internal() -> AppError {
    error(ErrorCode::Internal, Phase::Inspection)
}
fn limit() -> AppError {
    error(ErrorCode::OutputLimit, Phase::Inspection)
}
fn external() -> AppError {
    error(ErrorCode::ExternalChange, Phase::Inspection)
}
fn changed_error(entry: &sessions::Entry, value: AppError) -> AppError {
    entry.revoked.store(true, Ordering::SeqCst);
    if matches!(
        value.code,
        ErrorCode::InvalidRequest
            | ErrorCode::NotFound
            | ErrorCode::MalformedOutput
            | ErrorCode::ExternalChange
            | ErrorCode::RepositoryUnavailable
            | ErrorCode::UnsafePath
    ) {
        external()
    } else {
        value
    }
}
fn page_size(value: Option<u32>, default: usize, max: usize) -> Result<usize, AppError> {
    let value = value.map(|n| n as usize).unwrap_or(default);
    if value == 0 || value > max {
        Err(invalid())
    } else {
        Ok(value)
    }
}
fn bounded_response<T: Serialize>(value: &T) -> Result<(), AppError> {
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            if self.0 > HISTORY_RESPONSE_BYTES {
                Err(std::io::Error::other("History response limit"))
            } else {
                Ok(bytes.len())
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[derive(Serialize)]
    struct Success<'a, T> {
        ok: bool,
        value: &'a T,
    }
    serde_json::to_writer(Budget(0), &Success { ok: true, value }).map_err(|_| limit())
}
impl History {
    pub(crate) async fn validate_context(
        &self,
        window: &str,
        runner: &GitRunner,
        context: &HistoryContext,
        lifetime: std::sync::Arc<dyn Send + Sync>,
    ) -> Result<(), AppError> {
        let entry = self.sessions.get_context(window, context)?;
        let session = entry.session.lock().await;
        entry.valid()?;
        let query =
            query::Query::new_action(runner, &session.location, session.cancel.clone(), lifetime)
                .await
                .map_err(|e| changed_error(&entry, e))?;
        if query
            .fingerprint()
            .await
            .map_err(|e| changed_error(&entry, e))?
            != session.fingerprint
        {
            return Err(changed_error(&entry, external()));
        }
        entry.valid()
    }
    pub(crate) async fn resolve_action_commit(
        &self,
        window: &str,
        runner: &GitRunner,
        context: &HistoryContext,
        handle: &str,
        lifetime: std::sync::Arc<dyn Send + Sync>,
    ) -> Result<String, AppError> {
        let entry = self.sessions.get_context(window, context)?;
        let session = entry.session.lock().await;
        entry.valid()?;
        let oid = session
            .handle_oids
            .get(handle)
            .cloned()
            .ok_or_else(invalid_handle)?;
        let query =
            query::Query::new_action(runner, &session.location, session.cancel.clone(), lifetime)
                .await?;
        if query.fingerprint().await? != session.fingerprint {
            return Err(changed_error(&entry, external()));
        }
        entry.valid()?;
        Ok(oid)
    }
    pub(crate) async fn containment(
        &self,
        window: &str,
        runner: &GitRunner,
        request: GetCommitContainmentRequest,
        lifetime: std::sync::Arc<dyn Send + Sync>,
    ) -> Result<CommitContainment, AppError> {
        validation::request_size(&request)?;
        validation::counter(&request.context.scope.generation)?;
        let entry = self.sessions.get_context(window, &request.context)?;
        let _permit = self.reads.acquire().await.map_err(|_| internal())?;
        let session = entry.session.lock().await;
        entry.valid()?;
        let oid = session
            .handle_oids
            .get(&request.commit_handle)
            .ok_or_else(invalid_handle)?;
        let query =
            query::Query::new_action(runner, &session.location, session.cancel.clone(), lifetime)
                .await
                .map_err(|e| changed_error(&entry, e))?;
        if query
            .fingerprint()
            .await
            .map_err(|e| changed_error(&entry, e))?
            != session.fingerprint
        {
            return Err(changed_error(&entry, external()));
        }
        let result =
            crate::actions::containment::read(&query, &session, &request.commit_handle, oid)
                .await?;
        if query
            .fingerprint()
            .await
            .map_err(|e| changed_error(&entry, e))?
            != session.fingerprint
        {
            return Err(changed_error(&entry, external()));
        }
        entry.valid()?;
        bounded_response(&result)?;
        Ok(result)
    }
    pub async fn details(
        &self,
        window: &str,
        runner: &GitRunner,
        request: GetCommitDetailsRequest,
    ) -> Result<CommitDetails, AppError> {
        validation::request_size(&request)?;
        validation::counter(&request.context.scope.generation)?;
        let entry = self.sessions.get_context(window, &request.context)?;
        let _permit = self.reads.acquire().await.map_err(|_| internal())?;
        let mut session = entry.session.lock().await;
        entry.valid()?;
        let oid = session
            .handle_oids
            .get(&request.commit_handle)
            .cloned()
            .ok_or_else(invalid_handle)?;
        let query = query::Query::new(runner, &session.location, session.cancel.clone())
            .await
            .map_err(|e| changed_error(&entry, e))?;
        if query
            .fingerprint()
            .await
            .map_err(|e| changed_error(&entry, e))?
            != session.fingerprint
        {
            entry.revoked.store(true, Ordering::SeqCst);
            return Err(external());
        }
        let (result, pending) = details::read(&query, &mut session, &oid, request.base).await?;
        if query
            .fingerprint()
            .await
            .map_err(|e| changed_error(&entry, e))?
            != session.fingerprint
        {
            entry.revoked.store(true, Ordering::SeqCst);
            return Err(external());
        }
        entry.valid()?;
        bounded_response(&result)?;
        session.files.extend(pending);
        Ok(result)
    }
    pub async fn diff(
        &self,
        window: &str,
        runner: &GitRunner,
        request: GetCommitDiffRequest,
    ) -> Result<CommitDiff, AppError> {
        validation::request_size(&request)?;
        validation::counter(&request.context.scope.generation)?;
        let entry = self.sessions.get_context(window, &request.context)?;
        let _permit = self.reads.acquire().await.map_err(|_| internal())?;
        let session = entry.session.lock().await;
        entry.valid()?;
        let record = session
            .files
            .get(&request.file_handle)
            .ok_or_else(invalid_handle)?;
        let query = query::Query::new(runner, &session.location, session.cancel.clone())
            .await
            .map_err(|e| changed_error(&entry, e))?;
        if query
            .fingerprint()
            .await
            .map_err(|e| changed_error(&entry, e))?
            != session.fingerprint
        {
            entry.revoked.store(true, Ordering::SeqCst);
            return Err(external());
        }
        let result = CommitDiff {
            context: session.context.clone(),
            file_handle: request.file_handle,
            content: diff::read(&query, record).await?,
        };
        if query
            .fingerprint()
            .await
            .map_err(|e| changed_error(&entry, e))?
            != session.fingerprint
        {
            entry.revoked.store(true, Ordering::SeqCst);
            return Err(external());
        }
        entry.valid()?;
        bounded_response(&result)?;
        Ok(result)
    }
    async fn capability(&self, runner: &GitRunner) -> Result<(), AppError> {
        let supported = self
            .capability
            .get_or_try_init(|| async {
                let result = runner
                    .output_history(
                        None,
                        &crate::git::runner::argv(&[
                            "--no-lazy-fetch",
                            "--shallow-file",
                            if cfg!(windows) { "NUL" } else { "/dev/null" },
                            "--version",
                        ]),
                        &Default::default(),
                        Phase::Inspection,
                        Default::default(),
                        None,
                    )
                    .await?;
                Ok::<_, AppError>(result.success)
            })
            .await?;
        if *supported {
            Ok(())
        } else {
            Err(error(ErrorCode::UnsupportedGit, Phase::Inspection))
        }
    }
    pub async fn open(
        &self,
        window: &str,
        runner: &GitRunner,
        location: RepositoryLocation,
        request: OpenHistoryRequest,
    ) -> Result<HistorySession, AppError> {
        validation::request_size(&request)?;
        let size = page_size(request.page_size, HISTORY_PAGE_DEFAULT, HISTORY_PAGE_MAX)?;
        refs::validate_filter(&request.filter)?;
        let ticket = self.sessions.begin(window, &request.scope)?;
        let _permit = self.reads.acquire().await.map_err(|_| internal())?;
        self.capability(runner).await?;
        let query = query::Query::new(runner, &location, ticket.cancel.clone()).await?;
        let fingerprint = query.fingerprint().await?;
        let (refs, roots, all_roots) = refs::roots(&fingerprint, &request.filter);
        let order = query.order(&roots).await?;
        let all_order = if roots == all_roots {
            order.clone()
        } else {
            query.order(&all_roots).await?
        };
        let (order, all_order) = query
            .normalize_orders(
                order,
                all_order,
                &fingerprint.shallow,
                fingerprint.oid_length,
            )
            .await?;
        let mut session = sessions::Session {
            cancel: ticket.cancel.clone(),
            context: HistoryContext {
                scope: request.scope,
                snapshot_id: sessions::token(),
            },
            id: sessions::token(),
            location,
            fingerprint: fingerprint.clone(),
            refs,
            head: fingerprint.head.clone(),
            roots,
            positions: order
                .iter()
                .enumerate()
                .map(|(i, oid)| (oid.clone(), i))
                .collect(),
            all: all_order.iter().cloned().collect(),
            shallow: fingerprint.shallow.iter().cloned().collect(),
            order,
            all_order,
            graph_fence: 0,
            handles: Default::default(),
            handle_oids: Default::default(),
            cursors: Default::default(),
            files: Default::default(),
        };
        let page = query.page(&mut session, 0, size).await?;
        if query.fingerprint().await? != fingerprint {
            return Err(external());
        }
        let result = HistorySession {
            context: session.context.clone(),
            session_id: session.id.clone(),
            head: session.head.clone(),
            refs: session.refs.clone(),
            roots: session.roots.clone(),
            page,
        };
        bounded_response(&result)?;
        session.graph_fence = result.page.commits.len();
        self.sessions.publish(&ticket, session)?;
        Ok(result)
    }
    pub async fn page(
        &self,
        window: &str,
        runner: &GitRunner,
        request: LoadHistoryPageRequest,
    ) -> Result<HistoryPage, AppError> {
        validation::request_size(&request)?;
        validation::counter(&request.context.scope.generation)?;
        let size = page_size(request.page_size, HISTORY_PAGE_DEFAULT, HISTORY_PAGE_MAX)?;
        let entry = self
            .sessions
            .get(window, &request.context, &request.session_id)?;
        let _permit = self.reads.acquire().await.map_err(|_| internal())?;
        let mut session = entry.session.lock().await;
        entry.valid()?;
        let (start, expected) = match session.cursors.get(&request.cursor) {
            Some(sessions::Cursor::Page { start, size }) => (*start, *size),
            _ => return Err(invalid_handle()),
        };
        if expected != size {
            return Err(invalid_handle());
        }
        let query = query::Query::new(runner, &session.location, session.cancel.clone())
            .await
            .map_err(|e| changed_error(&entry, e))?;
        if query
            .fingerprint()
            .await
            .map_err(|e| changed_error(&entry, e))?
            != session.fingerprint
        {
            entry.revoked.store(true, Ordering::SeqCst);
            return Err(external());
        }
        let page = query.page(&mut session, start, size).await?;
        if query
            .fingerprint()
            .await
            .map_err(|e| changed_error(&entry, e))?
            != session.fingerprint
        {
            entry.revoked.store(true, Ordering::SeqCst);
            return Err(external());
        }
        entry.valid()?;
        bounded_response(&page)?;
        session.graph_fence = session.graph_fence.max(start + page.commits.len());
        Ok(page)
    }
    pub async fn search(
        &self,
        window: &str,
        runner: &GitRunner,
        request: SearchHistoryRequest,
    ) -> Result<HistorySearchPage, AppError> {
        validation::request_size(&request)?;
        validation::counter(&request.context.scope.generation)?;
        if request.query.is_empty()
            || request.query.len() > HISTORY_QUERY_BYTES
            || request.query.contains('\0')
        {
            return Err(invalid());
        }
        let size = page_size(
            request.page_size,
            HISTORY_SEARCH_PAGE_MAX,
            HISTORY_SEARCH_PAGE_MAX,
        )?;
        let entry = self
            .sessions
            .get(window, &request.context, &request.session_id)?;
        let _permit = self.reads.acquire().await.map_err(|_| internal())?;
        let mut session = entry.session.lock().await;
        entry.valid()?;
        let (start, graph_fence) = match &request.cursor {
            None => (0, session.graph_fence),
            Some(cursor) => match session.cursors.get(cursor) {
                Some(sessions::Cursor::Search {
                    start,
                    size: expected,
                    query,
                    graph_fence,
                }) if *expected == size && *query == request.query => (*start, *graph_fence),
                _ => return Err(invalid_handle()),
            },
        };
        let query = query::Query::new(runner, &session.location, session.cancel.clone())
            .await
            .map_err(|e| changed_error(&entry, e))?;
        if query
            .fingerprint()
            .await
            .map_err(|e| changed_error(&entry, e))?
            != session.fingerprint
        {
            entry.revoked.store(true, Ordering::SeqCst);
            return Err(external());
        }
        let result = search::search(
            &query,
            &mut session,
            &request.query,
            start,
            size,
            graph_fence,
            request.cursor.is_none(),
        )
        .await?;
        if query
            .fingerprint()
            .await
            .map_err(|e| changed_error(&entry, e))?
            != session.fingerprint
        {
            entry.revoked.store(true, Ordering::SeqCst);
            return Err(external());
        }
        entry.valid()?;
        bounded_response(&result)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::Command,
        time::{Duration, Instant},
    };
    #[tokio::test]
    async fn real_sessions_expire_and_runtime_cursor_handle_caps_return_errors_without_partial_pages(
    ) {
        let root = tempfile::tempdir().unwrap();
        let repository = root.path().join("repo");
        std::fs::create_dir(&repository).unwrap();
        std::fs::write(repository.join("file.txt"), b"content").unwrap();
        let runner = GitRunner::discover().unwrap();
        for arguments in [
            vec!["init", "-b", "main"],
            vec!["commit", "--allow-empty", "-m", "one"],
            vec!["commit", "--allow-empty", "-m", "two"],
            vec!["add", "--", "file.txt"],
            vec!["commit", "--allow-empty", "-m", "three"],
        ] {
            let mut command = Command::new(runner.executable());
            command
                .env(
                    "GIT_CONFIG_GLOBAL",
                    if cfg!(windows) { "NUL" } else { "/dev/null" },
                )
                .env(
                    "GIT_CONFIG_SYSTEM",
                    if cfg!(windows) { "NUL" } else { "/dev/null" },
                )
                .env("GIT_CONFIG_COUNT", "0")
                .env_remove("GIT_CONFIG_PARAMETERS");
            command
                .arg("-C")
                .arg(&repository)
                .args([
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "commit.gpgSign=false",
                ])
                .args(arguments);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }
            assert!(command.output().unwrap().status.success());
        }
        let backend =
            crate::backend::Backend::with_git(&root.path().join("database"), runner.clone())
                .unwrap();
        let selection = backend
            .register_repository("main", &repository)
            .await
            .unwrap()
            .active_selection
            .unwrap();
        let scope = HistoryScope {
            repo_id: selection.repo_id,
            worktree_id: selection.worktree_id,
            generation: "1".into(),
        };
        let location = backend
            .registry
            .location(&scope.repo_id, scope.worktree_id.as_deref())
            .unwrap();
        let service = History::default();
        service.sessions.open_window("main");
        let opened = service
            .open(
                "main",
                &runner,
                location.clone(),
                OpenHistoryRequest {
                    scope: scope.clone(),
                    filter: HistoryFilter {
                        hidden_refs: vec![],
                        solo_refs: vec![],
                    },
                    page_size: Some(1),
                },
            )
            .await
            .unwrap();
        let request = LoadHistoryPageRequest {
            context: opened.context.clone(),
            session_id: opened.session_id.clone(),
            cursor: opened.page.next_cursor.clone().unwrap(),
            page_size: Some(1),
        };
        let entry = service
            .sessions
            .get("main", &opened.context, &opened.session_id)
            .unwrap();
        {
            let mut session = entry.session.lock().await;
            for i in 0..HISTORY_CURSORS_PER_SESSION - 1 {
                session.cursors.insert(
                    sessions::token(),
                    sessions::Cursor::Page {
                        start: i + 1000,
                        size: 1,
                    },
                );
            }
        }
        assert_eq!(
            service
                .page("main", &runner, request.clone())
                .await
                .unwrap_err()
                .code,
            ErrorCode::OutputLimit
        );
        {
            let mut session = entry.session.lock().await;
            session.cursors.retain(|id, _| *id == request.cursor);
            for i in session.handles.len()..HISTORY_COMMITS_PER_SESSION {
                let oid = format!("{i:040x}");
                let handle = sessions::token();
                session.handles.insert(oid.clone(), handle.clone());
                session.handle_oids.insert(handle, oid);
            }
        }
        // The previous failed page minted its handle before hitting cursor cap;
        // request a later new commit to exercise handle exhaustion independently.
        let next = service.page("main", &runner, request).await.unwrap();
        assert_eq!(
            service
                .page(
                    "main",
                    &runner,
                    LoadHistoryPageRequest {
                        context: opened.context.clone(),
                        session_id: opened.session_id.clone(),
                        cursor: next.next_cursor.unwrap(),
                        page_size: Some(1)
                    }
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::OutputLimit
        );
        let known = service
            .search(
                "main",
                &runner,
                SearchHistoryRequest {
                    context: opened.context.clone(),
                    session_id: opened.session_id.clone(),
                    query: opened.page.commits[0].oid.clone(),
                    cursor: None,
                    page_size: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            known.commits[0].commit.handle,
            opened.page.commits[0].handle
        );
        {
            let mut session = entry.session.lock().await;
            for i in 0..HISTORY_FILES_PER_SESSION {
                session.files.insert(
                    sessions::token(),
                    details::FileRecord {
                        commit: "f".repeat(40),
                        base: DiffBase::Root,
                        old_oid: None,
                        new_oid: Some("f".repeat(40)),
                        old_mode: None,
                        new_mode: Some("100644".into()),
                        old_path: format!("cap-{i}").into_bytes(),
                        new_path: format!("cap-{i}").into_bytes(),
                    },
                );
            }
        }
        assert_eq!(
            service
                .details(
                    "main",
                    &runner,
                    GetCommitDetailsRequest {
                        context: opened.context.clone(),
                        commit_handle: opened.page.commits[0].handle.clone(),
                        base: DiffBase::Parent { index: 0 }
                    }
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::OutputLimit
        );
        assert_eq!(
            entry.session.lock().await.files.len(),
            HISTORY_FILES_PER_SESSION
        );
        drop(entry);
        let expired = service
            .open(
                "main",
                &runner,
                location,
                OpenHistoryRequest {
                    scope,
                    filter: HistoryFilter {
                        hidden_refs: vec![],
                        solo_refs: vec![],
                    },
                    page_size: Some(1),
                },
            )
            .await
            .unwrap();
        let expired_details = service
            .details(
                "main",
                &runner,
                GetCommitDetailsRequest {
                    context: expired.context.clone(),
                    commit_handle: expired.page.commits[0].handle.clone(),
                    base: DiffBase::Parent { index: 0 },
                },
            )
            .await
            .unwrap();
        service.sessions.expire_for_test(
            &expired.session_id,
            Instant::now() - Duration::from_secs(HISTORY_SESSION_TTL_SECONDS as u64 + 1),
        );
        assert_eq!(
            service
                .diff(
                    "main",
                    &runner,
                    GetCommitDiffRequest {
                        context: expired.context.clone(),
                        file_handle: expired_details.files[0].handle.clone()
                    }
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidHandle
        );
        assert_eq!(
            service
                .page(
                    "main",
                    &runner,
                    LoadHistoryPageRequest {
                        context: expired.context,
                        session_id: expired.session_id,
                        cursor: expired.page.next_cursor.unwrap(),
                        page_size: Some(1)
                    }
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidHandle
        );
    }
}
