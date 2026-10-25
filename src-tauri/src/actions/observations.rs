//! Durable per-remote write gates and reported-ref evidence. No URLs or runtime
//! capabilities are stored here. Cache eviction loses freshness, never authority.
use crate::{
    domain::contracts::*,
    persistence::workspace::{db_error, Database},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
pub const ACTION_BYTES: usize = 8 * 1024 * 1024;
pub const LEDGER_BYTES: usize = 64 * 1024 * 1024;
const CACHE_BYTES: usize = 16 * 1024 * 1024;
const CACHE_REFS: usize = 16384;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RemoteProof {
    pub name: String,
    pub config_digest: String,
    pub refs_digest: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Record {
    pub repo_id: String,
    pub proofs: Vec<RemoteProof>,
    pub result: FetchResult,
}
fn hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|v| v.is_ascii_hexdigit())
}
fn text(record: &Record) -> Result<String, AppError> {
    if record.repo_id.len() > 256
        || record.proofs.is_empty()
        || record.proofs.len() > super::fetch::REMOTES_MAX
        || record.proofs.len() != record.result.remotes.len()
    {
        return Err(db_error());
    }
    let mut count = 0usize;
    let mut names = std::collections::HashSet::new();
    for (proof, result) in record.proofs.iter().zip(&record.result.remotes) {
        if proof.name != result.name
            || !super::fetch::remote_name(&proof.name)
            || !names.insert(&proof.name)
            || !hash(&proof.config_digest)
            || !hash(&proof.refs_digest)
        {
            return Err(db_error());
        }
        count = count
            .checked_add(result.reported_refs.len())
            .ok_or_else(db_error)?;
        if count > super::fetch::REFS_MAX {
            return Err(db_error());
        }
        let mut refs = std::collections::HashSet::new();
        for reference in &result.reported_refs {
            if !reference
                .name
                .starts_with(&format!("refs/remotes/{}/", result.name))
                || !crate::history::refs::valid_name(&reference.name)
                || !refs.insert(&reference.name)
                || reference
                    .old_oid
                    .iter()
                    .chain(&reference.new_oid)
                    .any(|v| !super::catalog::valid_oid(v, v.len()))
            {
                return Err(db_error());
            }
        }
        if result.phase != FetchStepPhase::Observed && !result.reported_refs.is_empty() {
            return Err(db_error());
        }
    }
    let result = serde_json::to_string(record).map_err(|_| db_error())?;
    if result
        .len()
        .checked_add(1024)
        .is_none_or(|v| v > ACTION_BYTES)
    {
        return Err(crate::git::error::error(
            ErrorCode::OutputLimit,
            Phase::Recovery,
        ));
    }
    Ok(result)
}
pub(crate) fn read_from(c: &Connection, id: &str) -> Result<Option<Record>, AppError> {
    let raw: Option<String> = c
        .query_row(
            "SELECT record FROM fetch_actions WHERE operation_id=?1",
            [id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|_| db_error())?
        .flatten();
    raw.map(|raw| {
        if raw.len() > ACTION_BYTES {
            return Err(db_error());
        }
        let mut record: Record = serde_json::from_str(&raw).map_err(|_| db_error())?;
        for remote in &mut record.result.remotes {
            if let Some(e) = &mut remote.error {
                *e = super::safe_error(e);
            }
        }
        text(&record)?;
        Ok(record)
    })
    .transpose()
}
pub(crate) fn read(db: &Database, id: &str) -> Result<Option<FetchResult>, AppError> {
    let connection = db.lock()?;
    Ok(read_from(&connection, id)?.map(|v| v.result))
}
fn write_to(c: &Connection, id: &str, record: &Record, reserve: bool) -> Result<(), AppError> {
    let raw = text(record)?;
    let weight = if reserve {
        ACTION_BYTES
    } else {
        raw.len() + 1024
    };
    c.execute(
        "UPDATE fetch_actions SET record=?2,reserved_bytes=?3 WHERE operation_id=?1",
        params![id, raw, weight as i64],
    )
    .map_err(|_| db_error())?;
    Ok(())
}
pub(crate) fn used(c: &Connection) -> Result<usize, AppError> {
    let value: i64 = c
        .query_row(
            "SELECT COALESCE(SUM(reserved_bytes),0) FROM fetch_actions",
            [],
            |r| r.get(0),
        )
        .map_err(|_| db_error())?;
    usize::try_from(value).map_err(|_| db_error())
}
pub(crate) fn weight(c: &Connection, id: &str) -> Result<usize, AppError> {
    let value: Option<i64> = c
        .query_row(
            "SELECT reserved_bytes FROM fetch_actions WHERE operation_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| db_error())?;
    usize::try_from(value.unwrap_or(0)).map_err(|_| db_error())
}
pub(crate) fn reserve_to(c: &Connection, id: &str, budget: usize) -> Result<(), AppError> {
    if used(c)?
        .checked_add(ACTION_BYTES)
        .is_none_or(|n| n > budget)
    {
        return Err(capacity());
    }
    c.execute(
        "INSERT INTO fetch_actions(operation_id,reserved_bytes,record) VALUES(?1,?2,NULL)",
        params![id, ACTION_BYTES as i64],
    )
    .map_err(|_| db_error())?;
    Ok(())
}
pub(crate) fn begin(
    db: &Database,
    id: &str,
    repo: &str,
    prune: bool,
    proofs: Vec<RemoteProof>,
) -> Result<(), AppError> {
    let record = Record {
        repo_id: repo.into(),
        result: FetchResult {
            prune,
            remotes: proofs
                .iter()
                .map(|p| FetchRemoteResult {
                    name: p.name.clone(),
                    phase: FetchStepPhase::Pending,
                    effects: ActionEffects::NotApplied,
                    error: None,
                    reported_refs: vec![],
                    observed_at: None,
                })
                .collect(),
        },
        proofs,
    };
    let raw = text(&record)?;
    let mut c = db.lock()?;
    let tx = c.transaction().map_err(|_| db_error())?;
    let action = super::journal::read_from(&tx, id)?;
    if action.kind != ActionKind::Fetch
        || action.repo_id != repo
        || action.phase != super::journal::WritePhase::Prepared
    {
        return Err(db_error());
    }
    for proof in &record.proofs {
        if !action.targets.iter().any(|p|matches!(p,super::journal::SafeTargetProof::Remote {name,config_digest} if name==&proof.name && config_digest==&proof.config_digest)) {return Err(db_error());}
    }
    let changed=tx.execute("UPDATE fetch_actions SET record=?2 WHERE operation_id=?1 AND record IS NULL AND reserved_bytes=?3",params![id,raw,ACTION_BYTES as i64]).map_err(|_|db_error())?;
    if changed != 1 {
        return Err(db_error());
    }
    tx.commit().map_err(|_| db_error())
}
pub(crate) fn capacity() -> AppError {
    AppError {code:ErrorCode::OutputLimit, phase:Phase::Queue, retry:AppErrorRetry::None, message:"Fetch evidence reached its safety limit. Inspect unresolved actions before starting another fetch.".into()}
}
pub(crate) fn start_to(
    c: &Connection,
    id: &str,
    index: usize,
    refs_digest: &str,
) -> Result<(), AppError> {
    if !hash(refs_digest) {
        return Err(db_error());
    }
    let mut record = read_from(c, id)?.ok_or_else(db_error)?;
    let step = record.result.remotes.get_mut(index).ok_or_else(db_error)?;
    if step.phase != FetchStepPhase::Pending {
        return Err(db_error());
    }
    step.phase = FetchStepPhase::Started;
    step.effects = ActionEffects::Unknown;
    record
        .proofs
        .get_mut(index)
        .ok_or_else(db_error)?
        .refs_digest = refs_digest.into();
    write_to(c, id, &record, true)
}
/// Successful command + exact post-state verification is the only path that
/// writes observations. Publication is atomic with the per-step result.
pub(crate) fn observe(
    db: &Database,
    id: &str,
    index: usize,
    mut result: FetchRemoteResult,
) -> Result<(), AppError> {
    if result.phase != FetchStepPhase::Observed {
        return Err(db_error());
    }
    if let Some(e) = &mut result.error {
        *e = super::safe_error(e);
    }
    let mut c = db.lock()?;
    let tx = c.transaction().map_err(|_| db_error())?;
    let mut record = read_from(&tx, id)?.ok_or_else(db_error)?;
    let step = record.result.remotes.get(index).ok_or_else(db_error)?;
    if step.phase != FetchStepPhase::Started || step.name != result.name {
        return Err(db_error());
    }
    let proof = record.proofs.get(index).ok_or_else(db_error)?;
    if matches!(
        result.effects,
        ActionEffects::Applied | ActionEffects::NotApplied
    ) && result.error.is_none()
    {
        let time = result.observed_at.as_ref().ok_or_else(db_error)?;
        for reference in &result.reported_refs {
            let weight =
                2 * (record.repo_id.len() + proof.name.len() + reference.name.len()) + 1024;
            tx.execute("INSERT INTO fetch_observations(repo_id,remote_name,ref_name,config_digest,oid,observed_at,weight) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(repo_id,remote_name,ref_name) DO UPDATE SET config_digest=excluded.config_digest,oid=excluded.oid,observed_at=excluded.observed_at,weight=excluded.weight",params![record.repo_id,proof.name,reference.name,proof.config_digest,reference.new_oid,time,weight as i64]).map_err(|_| db_error())?;
        }
    }
    record.result.remotes[index] = result;
    write_to(&tx, id, &record, true)?;
    trim_cache(&tx)?;
    tx.commit().map_err(|_| db_error())
}
fn trim_cache(c: &Connection) -> Result<(), AppError> {
    trim_cache_budget(c, CACHE_REFS, CACHE_BYTES)
}
fn trim_cache_budget(c: &Connection, row_cap: usize, budget: usize) -> Result<(), AppError> {
    let (mut count, mut bytes): (i64, i64) = c
        .query_row(
            "SELECT COUNT(*),COALESCE(SUM(weight),0) FROM fetch_observations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| db_error())?;
    if count < 0 || bytes < 0 {
        return Err(db_error());
    }
    if count as usize <= row_cap && bytes as usize <= budget {
        return Ok(());
    }
    let mut statement = c
        .prepare("SELECT rowid,weight FROM fetch_observations ORDER BY observed_at,rowid")
        .map_err(|_| db_error())?;
    let rows = statement
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
        .map_err(|_| db_error())?;
    let mut victims = Vec::new();
    for row in rows {
        if count as usize <= row_cap && bytes as usize <= budget {
            break;
        }
        let (id, weight) = row.map_err(|_| db_error())?;
        if weight <= 0 {
            return Err(db_error());
        }
        victims.push(id);
        count -= 1;
        bytes -= weight;
    }
    drop(statement);
    for id in victims {
        c.execute("DELETE FROM fetch_observations WHERE rowid=?1", [id])
            .map_err(|_| db_error())?;
    }
    Ok(())
}
pub(crate) fn effects(record: &Record) -> ActionEffects {
    let mut applied = false;
    let mut unknown = false;
    let mut pending = false;
    if record.result.remotes.iter().any(|r| {
        r.phase == FetchStepPhase::Started
            || r.error
                .as_ref()
                .is_some_and(|e| e.code == ErrorCode::Interrupted && e.phase == Phase::Recovery)
    }) {
        return ActionEffects::Unknown;
    }
    for step in &record.result.remotes {
        match step.effects {
            ActionEffects::Applied => applied = true,
            ActionEffects::NotApplied => pending = true,
            _ => unknown = true,
        }
    }
    if applied && (unknown || pending) {
        ActionEffects::Partial
    } else if unknown {
        ActionEffects::Unknown
    } else if applied {
        ActionEffects::Applied
    } else {
        ActionEffects::NotApplied
    }
}
/// Once no executor can append more evidence, release unused reservation bytes.
/// Started uncertainty is retained, never converted to cancellation or retried.
pub(crate) fn settle_to(c: &Connection, id: &str) -> Result<Option<ActionEffects>, AppError> {
    let Some(mut record) = read_from(c, id)? else {
        c.execute(
            "UPDATE fetch_actions SET reserved_bytes=1024 WHERE operation_id=?1 AND record IS NULL",
            [id],
        )
        .map_err(|_| db_error())?;
        return Ok(None);
    };
    for step in &mut record.result.remotes {
        if step.phase == FetchStepPhase::Started {
            step.effects = ActionEffects::Unknown;
            step.error = Some(super::safe_error(&crate::git::error::error(
                ErrorCode::Interrupted,
                Phase::Recovery,
            )));
        }
    }
    let result = effects(&record);
    write_to(c, id, &record, false)?;
    Ok(Some(result))
}
pub(crate) fn attach(
    db: &Database,
    repo: &str,
    remotes: &[super::catalog::Remote],
    branches: &mut [ActionBranch],
) -> Result<(), AppError> {
    let c = db.lock()?;
    for branch in branches {
        branch.observation = None;
        if branch.kind != ActionBranchKind::Remote {
            continue;
        }
        for remote in remotes {
            if !branch
                .name
                .starts_with(&format!("refs/remotes/{}/", remote.name))
            {
                continue;
            }
            let value: Option<(Option<String>,String)> = c.query_row("SELECT oid,observed_at FROM fetch_observations WHERE repo_id=?1 AND remote_name=?2 AND ref_name=?3 AND config_digest=?4", params![repo,remote.name,branch.name,remote.digest], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_| db_error())?;
            if let Some((oid, time)) = value {
                if oid
                    .as_ref()
                    .is_some_and(|v| !super::catalog::valid_oid(v, v.len()))
                    || time.len() > 64
                    || chrono::DateTime::parse_from_rfc3339(&time).is_err()
                {
                    return Err(db_error());
                }
                branch.observation = Some(RefObservation {
                    remote_name: remote.name.clone(),
                    matches_current: oid.as_ref() == Some(&branch.oid),
                    observed_oid: oid,
                    observed_at: time,
                });
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expendable_cache_eviction_and_configuration_changes_return_no_freshness_instead_of_false_evidence(
    ) {
        let root = tempfile::tempdir().unwrap();
        let db = Database::open(root.path()).unwrap();
        {
            let c = db.lock().unwrap();
            for (i, name) in ["old", "middle", "new"].iter().enumerate() {
                c.execute(
                    "INSERT INTO fetch_observations VALUES('repo','origin',?1,?2,?3,?4,1024)",
                    params![
                        format!("refs/remotes/origin/{name}"),
                        "a".repeat(64),
                        "b".repeat(40),
                        format!("2026-10-25T00:00:0{i}Z")
                    ],
                )
                .unwrap();
            }
            trim_cache_budget(&c, 2, 2048).unwrap();
            assert_eq!(
                c.query_row("SELECT COUNT(*) FROM fetch_observations", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                2
            );
        }
        let mut remote = super::super::catalog::Remote {
            name: "origin".into(),
            url: None,
            refspecs: vec![],
            digest: "a".repeat(64),
            capability: FetchCapability::NoRefspec,
            ssh: false,
        };
        let mut branches = ["old", "new"].map(|n| ActionBranch {
            name: format!("refs/remotes/origin/{n}"),
            oid: "b".repeat(40),
            kind: ActionBranchKind::Remote,
            observation: None,
        });
        attach(&db, "repo", std::slice::from_ref(&remote), &mut branches).unwrap();
        assert!(branches[0].observation.is_none());
        assert!(branches[1].observation.as_ref().unwrap().matches_current);
        remote.digest = "c".repeat(64);
        attach(&db, "repo", &[remote], &mut branches).unwrap();
        assert!(branches.iter().all(|b| b.observation.is_none()));
    }
}
