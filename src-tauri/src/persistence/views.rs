use super::workspace::{db_error, Database};
use crate::{
    domain::contracts::*,
    git::error::error,
    ipc::validation,
    repositories::{discovery, registry::RepositoryLocation},
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

fn invalid() -> AppError {
    error(ErrorCode::InvalidRequest, Phase::Validation)
}
fn unavailable() -> AppError {
    error(ErrorCode::InvalidHandle, Phase::Validation)
}
pub fn validate(view: &RepositoryView) -> Result<RepositoryView, AppError> {
    validation::request_size(view)?;
    if !view.anchor_offset.is_finite()
        || !(-10_000.0..=10_000.0).contains(&view.anchor_offset)
        || !(160..=640).contains(&view.sidebar_width)
        || !(160..=1200).contains(&view.details_height)
    {
        return Err(invalid());
    }
    if view
        .selected_oid
        .iter()
        .chain(&view.anchor_oid)
        .any(|v| !matches!(v.len(), 40 | 64) || !v.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(invalid());
    }
    for name in view.filter.hidden_refs.iter().chain(&view.filter.solo_refs) {
        if name.starts_with("refs/stash/") {
            return Err(AppError {
                code: ErrorCode::InvalidRequest,
                phase: Phase::Validation,
                retry: AppErrorRetry::None,
                message:
                    "Older stash entries cannot be saved as filters. Select a stash commit instead."
                        .into(),
            });
        }
        if !crate::history::refs::valid_name(name) {
            return Err(invalid());
        }
    }
    let mut view = view.clone();
    for names in [&mut view.filter.hidden_refs, &mut view.filter.solo_refs] {
        names.sort();
        names.dedup();
    }
    view.selected_oid = view.selected_oid.map(|s| s.to_ascii_lowercase());
    view.anchor_oid = view.anchor_oid.map(|s| s.to_ascii_lowercase());
    Ok(view)
}
fn default_view() -> RepositoryView {
    RepositoryView {
        filter: HistoryFilter {
            hidden_refs: vec![],
            solo_refs: vec![],
        },
        selected_oid: None,
        anchor_oid: None,
        anchor_offset: 0.0,
        sidebar_width: 280,
        details_height: 320,
        diff_layout: DiffLayout::SideBySide,
        sidebar_collapsed: false,
    }
}
pub fn check_scope(
    connection: &Connection,
    scope: &HistoryScope,
    location: &RepositoryLocation,
) -> Result<(), AppError> {
    validation::counter(&scope.generation)?;
    let row: Option<(Vec<u8>, bool)> = connection
        .query_row(
            "SELECT identity,bare FROM repositories WHERE id=?1",
            [&scope.repo_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|_| db_error())?;
    if row != Some((location.common_identity.clone(), location.bare)) {
        return Err(unavailable());
    }
    if let Some(tree) = &scope.worktree_id {
        let identity: Option<Vec<u8>> = connection
            .query_row(
                "SELECT identity FROM worktrees WHERE id=?1 AND repo_id=?2",
                params![tree, scope.repo_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| db_error())?;
        if identity != location.root_identity || location.bare {
            return Err(unavailable());
        }
    } else if !location.bare {
        return Err(unavailable());
    }
    let active: (Option<String>, Option<String>) = connection
        .query_row(
            "SELECT active_repo,active_worktree FROM workspace",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| db_error())?;
    if active != (Some(scope.repo_id.clone()), scope.worktree_id.clone()) {
        return Err(unavailable());
    }
    if discovery::identity(&location.common).as_ref() != Ok(&location.common_identity)
        || location
            .root_identity
            .as_ref()
            .is_some_and(|identity| discovery::identity(&location.root).as_ref() != Ok(identity))
    {
        return Err(error(ErrorCode::ExternalChange, Phase::Inspection));
    }
    Ok(())
}
fn read(connection: &Connection, scope: &HistoryScope) -> Result<RepositoryViewState, AppError> {
    let row: Option<(i64, Option<String>)> = connection
        .query_row(
            "SELECT revision,CASE WHEN length(CAST(value AS BLOB)) <= 16384 THEN value END FROM repository_views WHERE repo_id=?1",
            [&scope.repo_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|_| db_error())?;
    let (revision, view) = if let Some((revision, value)) = row {
        let value = value.ok_or_else(db_error)?;
        if revision < 0 || value.len() > 16 * 1024 {
            return Err(db_error());
        }
        let view: RepositoryView = serde_json::from_str(&value).map_err(|_| db_error())?;
        (revision, validate(&view).map_err(|_| db_error())?)
    } else {
        (0, default_view())
    };
    Ok(RepositoryViewState {
        scope: scope.clone(),
        view_revision: revision.to_string(),
        view,
    })
}
pub fn get(
    database: &Database,
    scope: &HistoryScope,
    location: &RepositoryLocation,
) -> Result<RepositoryViewState, AppError> {
    let connection = database.lock()?;
    check_scope(&connection, scope, location)?;
    read(&connection, scope)
}
pub fn save(
    database: &Database,
    request: &SaveRepositoryViewRequest,
    location: &RepositoryLocation,
) -> Result<RepositoryViewState, AppError> {
    let expected = validation::counter(&request.expected_revision)?;
    let view = validate(&request.view)?;
    let mut connection = database.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| db_error())?;
    check_scope(&transaction, &request.scope, location)?;
    let current = read(&transaction, &request.scope)?;
    if expected != validation::counter(&current.view_revision)? {
        return Err(error(ErrorCode::ExternalChange, Phase::Registration));
    }
    let next = i64::try_from(expected)
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| error(ErrorCode::OutputLimit, Phase::Registration))?;
    let value = serde_json::to_string(&view).map_err(|_| invalid())?;
    transaction.execute("INSERT INTO repository_views(repo_id,revision,value) VALUES(?1,?2,?3) ON CONFLICT(repo_id) DO UPDATE SET revision=excluded.revision,value=excluded.value", params![request.scope.repo_id,next,value]).map_err(|_| db_error())?;
    check_scope(&transaction, &request.scope, location)?;
    transaction.commit().map_err(|_| db_error())?;
    Ok(RepositoryViewState {
        scope: request.scope.clone(),
        view_revision: next.to_string(),
        view,
    })
}
