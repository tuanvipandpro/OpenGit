use super::{
    journal::{self, ClonePhase},
    path::StableDirectory,
    staging,
};
use crate::{
    domain::contracts::*,
    git::{error::error, runner::GitRunner},
    operations::{journal as operations, state},
    persistence::workspace::{db_error, decode, Database},
    repositories::{discovery, registry::Registry},
};
use std::{sync::Arc, time::Duration};
pub fn before_backend(database: Arc<Database>, runner: Option<GitRunner>) -> Result<(), AppError> {
    if journal::load(&database)?
        .iter()
        .all(|record| matches!(record.phase, ClonePhase::Registered | ClonePhase::Aborted))
    {
        return Ok(());
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| db_error())
            .and_then(|runtime| {
                runtime.block_on(async {
                    tokio::time::timeout(
                        Duration::from_secs(25),
                        reconcile(&database, runner.as_ref()),
                    )
                    .await
                    .map_err(|_| error(ErrorCode::Interrupted, Phase::Recovery))?
                })
            });
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(Duration::from_secs(30))
        .map_err(|_| error(ErrorCode::Interrupted, Phase::Recovery))?
}
pub async fn reconcile(database: &Database, runner: Option<&GitRunner>) -> Result<(), AppError> {
    let records = journal::load(database)?;
    let mut snapshots = operations::load(database)?;
    if records
        .iter()
        .filter(|record| !matches!(record.phase, ClonePhase::Registered | ClonePhase::Aborted))
        .count()
        > 64
    {
        return Err(error(ErrorCode::Interrupted, Phase::Recovery));
    }
    for mut record in records {
        if matches!(record.phase, ClonePhase::Registered | ClonePhase::Aborted) {
            continue;
        }
        super::request::directory_name(&record.leaf)?;
        super::request::directory_name(&record.container)?;
        if !record.container.starts_with(".opengit-stage-") || record.nonce.len() < 64 {
            return Err(error(ErrorCode::UnsafePath, Phase::Recovery));
        }
        let snapshot = snapshots
            .iter_mut()
            .find(|snapshot| snapshot.id == record.operation_id)
            .ok_or_else(db_error)?;
        let parent = Arc::new(StableDirectory::open(&decode(record.parent.clone())?)?);
        if parent.identity != record.parent_identity {
            return Err(error(ErrorCode::ExternalChange, Phase::Recovery));
        }
        let destination = parent.path.join(&record.leaf);
        let published = record.phase == ClonePhase::PublishIntent
            && record
                .repository_identity
                .as_ref()
                .is_some_and(|expected| discovery::identity(&destination).as_ref() == Ok(expected));
        if published {
            let runner = runner.ok_or_else(|| error(ErrorCode::GitUnavailable, Phase::Recovery))?;
            let verified = tokio::time::timeout(
                Duration::from_secs(15),
                discovery::discover(runner, &destination),
            )
            .await
            .map_err(|_| error(ErrorCode::Interrupted, Phase::Recovery))??;
            if verified.root_identity != record.repository_identity || verified.bare {
                return Err(error(ErrorCode::ExternalChange, Phase::Recovery));
            }
            let cleanup = staging::cleanup(parent.clone(), &record);
            record.cleanup = cleanup.disposition.clone();
            let warning = cleanup.warning();
            let mut connection = database.lock()?;
            let transaction = connection.transaction().map_err(|_| db_error())?;
            let selection = Registry::register_transaction(&transaction, &verified)?;
            // Explicit recovery is the only path allowed to replace an old
            // Interrupted snapshot with proven-published success.
            snapshot.state = OperationState::Succeeded;
            snapshot.phase = Phase::Registration;
            snapshot.repo_id = Some(selection.repo_id);
            snapshot.worktree_id = selection.worktree_id;
            snapshot.can_cancel = false;
            snapshot.auth_prompt = None;
            snapshot.error = warning;
            snapshot.updated_at = chrono::Utc::now().to_rfc3339();
            record.phase = ClonePhase::Registered;
            operations::write_to(&transaction, snapshot)?;
            journal::write_to(&transaction, &record)?;
            transaction.commit().map_err(|_| db_error())?;
        } else {
            let container = parent.path.join(&record.container);
            if record.phase == ClonePhase::PublishIntent && !container.join("repository").exists() {
                // The source vanished but the exact destination was not
                // proven. Never clean ambiguous ownership evidence or treat
                // API success alone as an aborted pre-publish clone.
                return Err(error(ErrorCode::Interrupted, Phase::Recovery));
            }
            let cleanup = staging::cleanup(parent.clone(), &record);
            record.cleanup = cleanup.disposition;
            if !state::terminal(&snapshot.state) {
                snapshot.state = OperationState::Interrupted;
                snapshot.phase = Phase::Recovery;
                snapshot.error = Some(error(ErrorCode::Interrupted, Phase::Recovery));
                snapshot.can_cancel = false;
                snapshot.auth_prompt = None;
                snapshot.updated_at = chrono::Utc::now().to_rfc3339();
            }
            if record.cleanup.retained() {
                snapshot.error = Some(crate::git::error::retained_error(snapshot.error.take()));
            }
            record.phase = ClonePhase::Aborted;
            operations::write(database, snapshot)?;
            journal::write(database, &record)?;
        }
    }
    Ok(())
}
