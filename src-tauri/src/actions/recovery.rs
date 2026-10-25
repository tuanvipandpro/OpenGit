//! Restart never resumes or retries a write. Prepared proves the write gate was
//! not crossed; a durable write-start checkpoint requires action-specific inspection.
use super::journal::{self, WritePhase};
use crate::{domain::contracts::*, persistence::workspace::Database};
pub fn reconcile(db: &Database, snapshot: &mut OperationSnapshot) -> Result<(), AppError> {
    if snapshot.kind != OperationSnapshotKind::Action {
        return Ok(());
    }
    let mut record = journal::read(db, &snapshot.id)?;
    if matches!(record.phase, WritePhase::Observed) {
        snapshot.outcome = record.outcome.clone();
        return Ok(());
    }
    let effects = if let Some(proof) = &record.sync_action {
        proof.effects()
    } else if let Some(proof) = &record.change_action {
        if matches!(
            proof.phase,
            journal::IndexPhase::Published | journal::IndexPhase::Observed
        ) {
            proof.effects
        } else {
            ActionEffects::Unknown
        }
    } else if let Some(proof) = &record.remote_read {
        proof.effects()
    } else if record.phase == WritePhase::Prepared {
        ActionEffects::NotApplied
    } else if record.kind == ActionKind::Fetch {
        let c = db.lock()?;
        super::observations::read_from(&c, &snapshot.id)?
            .map(|r| super::observations::effects(&r))
            .unwrap_or(ActionEffects::Unknown)
    } else if let Some(proof) = &record.ref_action {
        super::branches::effects(proof)
    } else {
        ActionEffects::Unknown
    };
    let mut result = journal::outcome(effects, snapshot.state == OperationState::Cancelling);
    if record
        .change_action
        .as_ref()
        .is_some_and(|p| p.result.artifacts_retained)
    {
        result.recovery = ActionRecovery::Inspect;
    }
    snapshot.outcome = Some(result.clone());
    snapshot.state = OperationState::Interrupted;
    snapshot.phase = Phase::Recovery;
    snapshot.can_cancel = false;
    snapshot.auth_prompt = None;
    snapshot.error = Some(super::safe_error(&crate::git::error::error(
        ErrorCode::Interrupted,
        Phase::Recovery,
    )));
    record.outcome = Some(result); // Preserve WriteStarted: no universal no-effects proof.
    let mut c = db.lock()?;
    let tx = c
        .transaction()
        .map_err(|_| crate::persistence::workspace::db_error())?;
    super::observations::settle_to(&tx, &snapshot.id)?;
    journal::write_to(&tx, &snapshot.id, &record)?;
    crate::operations::journal::write_to(&tx, snapshot)?;
    tx.commit()
        .map_err(|_| crate::persistence::workspace::db_error())?;
    Ok(())
}

/// Inspection aid only, never authorization to resume or clean an artifact.
/// The pure pins stay alive through any caller's owned process lifetime guard.
pub struct VerifiedLocation {
    pub location: crate::repositories::registry::RepositoryLocation,
    pub pins: std::sync::Arc<super::preflight::Pins>,
}
pub fn verify_location(record: &journal::ActionRecord) -> Result<VerifiedLocation, AppError> {
    use crate::{
        persistence::workspace::decode,
        repositories::{discovery::identity, registry::RepositoryLocation},
    };
    let common = decode(record.location.common.clone())?;
    let git_dir = decode(record.location.git.clone())?;
    let root = record
        .location
        .worktree
        .as_ref()
        .map(|v| decode(v.clone()))
        .transpose()?
        .unwrap_or_else(|| common.clone());
    let location = RepositoryLocation {
        common,
        git_dir,
        root,
        common_identity: record.common_identity.clone(),
        root_identity: record.root_identity.clone(),
        bare: record.location.worktree.is_none(),
    };
    let pins = super::preflight::Pins::open(&location)?;
    if identity(&location.common)? != record.common_identity
        || identity(&location.git_dir)? != record.location.git_identity
        || record
            .root_identity
            .as_ref()
            .is_some_and(|id| identity(&location.root).as_ref() != Ok(id))
    {
        return Err(crate::git::error::error(
            ErrorCode::ExternalChange,
            Phase::Recovery,
        ));
    }
    Ok(VerifiedLocation { location, pins })
}
