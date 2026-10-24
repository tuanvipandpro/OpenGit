use super::discovery::Discovered;
use crate::{
    domain::contracts::*,
    git::error::error,
    persistence::workspace::{db_error, decode, encode, Database},
};
use rusqlite::{params, OptionalExtension};
use std::{path::PathBuf, sync::Arc};
pub struct Registry {
    pub database: Arc<Database>,
}
#[derive(Clone)]
pub struct RepositoryLocation {
    pub common: PathBuf,
    pub common_identity: Vec<u8>,
    pub root: PathBuf,
    pub root_identity: Option<Vec<u8>>,
    pub git_dir: PathBuf,
    pub bare: bool,
}
impl Registry {
    pub fn register(&self, item: &Discovered) -> Result<Workspace, AppError> {
        if super::discovery::identity(&item.common)? != item.common_identity
            || item
                .root
                .as_ref()
                .zip(item.root_identity.as_ref())
                .is_some_and(|(path, expected)| {
                    super::discovery::identity(path).as_ref() != Ok(expected)
                })
        {
            return Err(error(ErrorCode::ExternalChange, Phase::Registration));
        }
        let mut connection = self.database.lock()?;
        let transaction = connection.transaction().map_err(|_| db_error())?;
        Self::register_transaction(&transaction, item)?;
        transaction.commit().map_err(|_| db_error())?;
        drop(connection);
        self.workspace()
    }
    pub(crate) fn register_transaction(
        transaction: &rusqlite::Transaction<'_>,
        item: &Discovered,
    ) -> Result<Selection, AppError> {
        let existing: Option<String> = transaction
            .query_row(
                "SELECT id FROM repositories WHERE identity=?1",
                [&item.common_identity],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| db_error())?;
        let id = existing.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        transaction.execute("INSERT INTO repositories(id,identity,common_path,bare,opened) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET common_path=excluded.common_path,opened=excluded.opened", params![id,item.common_identity,encode(&item.common),item.bare,chrono::Utc::now().to_rfc3339()]).map_err(|_| db_error())?;
        let worktree_id = if let (Some(root), Some(identity)) = (&item.root, &item.root_identity) {
            let existing: Option<String> = transaction
                .query_row(
                    "SELECT id FROM worktrees WHERE identity=?1",
                    [identity],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| db_error())?;
            let worktree = existing.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            transaction.execute("INSERT INTO worktrees(id,repo_id,identity,root,git_dir,is_main) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET repo_id=excluded.repo_id,root=excluded.root,git_dir=excluded.git_dir,is_main=excluded.is_main", params![worktree,id,identity,encode(root),encode(&item.git_dir),item.common==item.git_dir]).map_err(|_| db_error())?;
            Some(worktree)
        } else {
            None
        };
        transaction
            .execute(
                "UPDATE workspace SET revision=revision+1,active_repo=?1,active_worktree=?2",
                params![id, worktree_id],
            )
            .map_err(|_| db_error())?;
        Ok(Selection {
            repo_id: id,
            worktree_id,
        })
    }
    pub fn workspace(&self) -> Result<Workspace, AppError> {
        let connection = self.database.lock()?;
        let (id, revision, repo, worktree): (String, i64, Option<String>, Option<String>) =
            connection
                .query_row(
                    "SELECT id,revision,active_repo,active_worktree FROM workspace",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .map_err(|_| db_error())?;
        let mut statement = connection
            .prepare("SELECT id,common_path,bare,opened,identity FROM repositories ORDER BY opened DESC,id")
            .map_err(|_| db_error())?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, bool>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Vec<u8>>(4)?,
                ))
            })
            .map_err(|_| db_error())?;
        let mut repositories = Vec::new();
        for row in rows {
            let (id, raw, bare, opened, expected_identity) = row.map_err(|_| db_error())?;
            let common = decode(raw)?;
            let mut trees = connection
                .prepare("SELECT id,root,is_main FROM worktrees WHERE repo_id=?1 ORDER BY id")
                .map_err(|_| db_error())?;
            let mut worktrees = Vec::new();
            for tree in trees
                .query_map([&id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, bool>(2)?,
                    ))
                })
                .map_err(|_| db_error())?
            {
                let (id, path, is_main) = tree.map_err(|_| db_error())?;
                worktrees.push(WorktreeSummary {
                    id,
                    root_display: decode(path)?.to_string_lossy().into(),
                    is_main,
                });
            }
            repositories.push(RepositorySummary {
                id,
                name: (if bare {
                    Some(common.as_path())
                } else {
                    common.parent()
                })
                .and_then(|p| p.file_name())
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
                common_git_dir_display: common.to_string_lossy().into(),
                is_bare: bare,
                availability: if super::discovery::identity(&common).as_ref()
                    == Ok(&expected_identity)
                {
                    RepositorySummaryAvailability::Available
                } else if !common.exists() {
                    RepositorySummaryAvailability::Missing
                } else {
                    RepositorySummaryAvailability::Unavailable
                },
                worktrees,
                last_opened_at: opened,
            });
        }
        Ok(Workspace {
            id,
            revision: revision.to_string(),
            repositories,
            active_selection: repo.map(|repo_id| Selection {
                repo_id,
                worktree_id: worktree,
            }),
        })
    }
    pub fn location(
        &self,
        repo: &str,
        worktree: Option<&str>,
    ) -> Result<RepositoryLocation, AppError> {
        let connection = self.database.lock()?;
        let row: Option<(Vec<u8>, Vec<u8>, bool)> = connection
            .query_row(
                "SELECT common_path,identity,bare FROM repositories WHERE id=?1",
                [repo],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|_| db_error())?;
        let (common, common_identity, bare) =
            row.ok_or_else(|| error(ErrorCode::InvalidHandle, Phase::Validation))?;
        let common = decode(common)?;
        if bare && worktree.is_none() {
            return Ok(RepositoryLocation {
                root: common.clone(),
                git_dir: common.clone(),
                common,
                common_identity,
                root_identity: None,
                bare,
            });
        }
        let worktree = worktree
            .filter(|_| !bare)
            .ok_or_else(|| error(ErrorCode::InvalidHandle, Phase::Validation))?;
        let row: Option<(Vec<u8>, Vec<u8>, Vec<u8>)> = connection
            .query_row(
                "SELECT root,identity,git_dir FROM worktrees WHERE id=?1 AND repo_id=?2",
                params![worktree, repo],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|_| db_error())?;
        let (root, identity, git_dir) =
            row.ok_or_else(|| error(ErrorCode::InvalidHandle, Phase::Validation))?;
        Ok(RepositoryLocation {
            common,
            common_identity,
            root: decode(root)?,
            root_identity: Some(identity),
            git_dir: decode(git_dir)?,
            bare,
        })
    }
    pub fn select(&self, request: &SelectRepositoryRequest) -> Result<Workspace, AppError> {
        let location = self.location(&request.repo_id, request.worktree_id.as_deref())?;
        if super::discovery::identity(&location.common)? != location.common_identity
            || location.root_identity.as_ref().is_some_and(|expected| {
                super::discovery::identity(&location.root).as_ref() != Ok(expected)
            })
        {
            return Err(error(ErrorCode::ExternalChange, Phase::Inspection));
        }
        let connection = self.database.lock()?;
        connection
            .execute(
                "UPDATE workspace SET revision=revision+1,active_repo=?1,active_worktree=?2",
                params![request.repo_id, request.worktree_id],
            )
            .map_err(|_| db_error())?;
        drop(connection);
        self.workspace()
    }
    pub fn forget(&self, repo: &str) -> Result<Workspace, AppError> {
        let mut connection = self.database.lock()?;
        let tx = connection.transaction().map_err(|_| db_error())?;
        if tx
            .execute("DELETE FROM repositories WHERE id=?1", [repo])
            .map_err(|_| db_error())?
            == 0
        {
            return Err(error(ErrorCode::InvalidHandle, Phase::Validation));
        }
        tx.execute("UPDATE workspace SET revision=revision+1,active_worktree=CASE WHEN active_repo=?1 THEN NULL ELSE active_worktree END,active_repo=CASE WHEN active_repo=?1 THEN NULL ELSE active_repo END",[repo]).map_err(|_|db_error())?;
        tx.commit().map_err(|_| db_error())?;
        drop(connection);
        self.workspace()
    }
}
