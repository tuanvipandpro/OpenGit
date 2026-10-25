//! Safe recovery evidence only: backend locators are not cleanup authority.
//! No message/file content, URL or runtime capability is persisted.
use crate::{
    domain::contracts::*,
    persistence::workspace::{db_error, Database},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WritePhase {
    Prepared,
    WriteStarted,
    Observed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactRoot {
    Common,
    Git,
    Worktree,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactKind {
    AlternateIndex,
    OwnedLock,
    RecoveryRef,
    IndexLock,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IndexPhase {
    Reserved,
    AlternateOwned,
    CandidateReady,
    LockReserved,
    LockOwned,
    PublicationStarted,
    Published,
    CommitStarted,
    Observed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeActionProof {
    pub phase: IndexPhase,
    pub alternate_name: Option<String>,
    pub before_index: String,
    pub candidate_digest: Option<String>,
    pub candidate_tree: Option<String>,
    pub before_head: Option<String>,
    pub effects: ActionEffects,
    pub result: ChangeActionResult,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncActionProof {
    pub remote_name: String,
    pub role: RemoteEndpointRole,
    pub config_digest: String,
    pub source_ref: Option<String>,
    pub source_oid: Option<String>,
    pub destination: String,
    pub preview_oid: Option<String>,
    pub result: SyncActionResult,
    pub before_fingerprint: String,
    pub step_stamps: Vec<Option<String>>,
}
impl SyncActionProof {
    pub(crate) fn effects(&self) -> ActionEffects {
        if self.result.steps.iter().any(|s| {
            s.phase == FetchStepPhase::Started
                || s.phase == FetchStepPhase::Observed && s.effects == ActionEffects::Unknown
        }) {
            return ActionEffects::Unknown;
        }
        let applied =
            self.result.steps.iter().any(|s| {
                s.phase == FetchStepPhase::Observed && s.effects == ActionEffects::Applied
            });
        if self
            .result
            .steps
            .iter()
            .all(|s| s.phase == FetchStepPhase::Observed && s.failure_reason.is_none())
        {
            if applied {
                ActionEffects::Applied
            } else {
                ActionEffects::NotApplied
            }
        } else if applied || self.result.publication.is_some() {
            ActionEffects::Partial
        } else {
            ActionEffects::NotApplied
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactProof {
    pub root: ArtifactRoot,
    pub kind: ArtifactKind,
    pub name: String,
    pub identity: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryLocation {
    pub common: Vec<u8>,
    pub git: Vec<u8>,
    pub worktree: Option<Vec<u8>>,
    pub git_identity: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObjectFormat {
    Sha1,
    Sha256,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum SafeTargetProof {
    RemoteDestination {
        #[serde(rename = "remoteName")]
        remote_name: String,
        role: RemoteEndpointRole,
        #[serde(rename = "refName")]
        ref_name: String,
        #[serde(rename = "expectedOid")]
        expected_oid: Option<String>,
        #[serde(rename = "objectFormat")]
        object_format: Option<ObjectFormat>,
        #[serde(rename = "configDigest")]
        config_digest: String,
    },
    Ref {
        name: String,
        #[serde(
            rename = "expectedOid",
            deserialize_with = "crate::domain::contracts::required_nullable"
        )]
        expected_oid: Option<String>,
        #[serde(rename = "objectFormat")]
        object_format: ObjectFormat,
    },
    Remote {
        name: String,
        #[serde(rename = "configDigest")]
        config_digest: String,
    },
    Commit {
        oid: String,
        #[serde(rename = "objectFormat")]
        object_format: ObjectFormat,
    },
    Change {
        #[serde(rename = "pathDigest")]
        path_digest: String,
        #[serde(rename = "indexDigest")]
        index_digest: String,
    },
    Stash {
        oid: String,
        #[serde(rename = "reflogDigest")]
        reflog_digest: String,
        #[serde(rename = "objectFormat")]
        object_format: ObjectFormat,
    },
}
impl SafeTargetProof {
    pub fn heap_bytes(&self) -> usize {
        match self {
            Self::RemoteDestination {
                remote_name,
                ref_name,
                expected_oid,
                config_digest,
                ..
            } => {
                remote_name.capacity()
                    + ref_name.capacity()
                    + expected_oid.as_ref().map_or(0, String::capacity)
                    + config_digest.capacity()
            }
            Self::Ref {
                name, expected_oid, ..
            } => name.capacity() + expected_oid.as_ref().map_or(0, String::capacity),
            Self::Remote {
                name,
                config_digest,
            } => name.capacity() + config_digest.capacity(),
            Self::Commit { oid, .. } => oid.capacity(),
            Self::Change {
                path_digest,
                index_digest,
            } => path_digest.capacity() + index_digest.capacity(),
            Self::Stash {
                oid, reflog_digest, ..
            } => oid.capacity() + reflog_digest.capacity(),
        }
    }
    pub fn validate(&self) -> Result<(), AppError> {
        let oid = |v: &str, f: ObjectFormat| {
            v.len()
                == match f {
                    ObjectFormat::Sha1 => 40,
                    ObjectFormat::Sha256 => 64,
                }
                && v.bytes().all(|b| b.is_ascii_hexdigit())
        };
        let valid = match self {
            Self::RemoteDestination {
                remote_name,
                ref_name,
                expected_oid,
                object_format,
                config_digest,
                ..
            } => {
                super::fetch::remote_name(remote_name)
                    && remote_name.len() <= 1024
                    && ref_name.len() <= 1024
                    && ["refs/heads/", "refs/tags/"]
                        .iter()
                        .any(|p| ref_name.starts_with(p))
                    && crate::history::refs::valid_name(ref_name)
                    && fingerprint(config_digest)
                    && expected_oid
                        .as_ref()
                        .is_none_or(|v| object_format.is_some_and(|f| oid(v, f)))
            }
            Self::Ref {
                name,
                expected_oid,
                object_format,
            } => {
                name.len() <= 1024
                    && ["refs/heads/", "refs/remotes/", "refs/tags/"]
                        .iter()
                        .any(|p| name.starts_with(p))
                    && crate::history::refs::valid_name(name)
                    && expected_oid.as_ref().is_none_or(|v| oid(v, *object_format))
            }
            Self::Remote {
                name,
                config_digest,
            } => {
                name.len() <= 1024
                    && !name.starts_with('-')
                    && crate::history::refs::valid_name(&format!("refs/remotes/{name}/target"))
                    && fingerprint(config_digest)
            }
            Self::Commit {
                oid: v,
                object_format,
            } => oid(v, *object_format),
            Self::Change {
                path_digest,
                index_digest,
            } => fingerprint(path_digest) && fingerprint(index_digest),
            Self::Stash {
                oid: v,
                reflog_digest,
                object_format,
            } => oid(v, *object_format) && fingerprint(reflog_digest),
        };
        if valid {
            Ok(())
        } else {
            Err(db_error())
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RefStepKind {
    CreateBranch,
    RenameBranch,
    SwitchBranch,
    DeleteBranch,
    RemoveBranchConfig,
    Tracking,
    CreateTag,
    DeleteTag,
}
impl RefStepKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::CreateBranch => "createBranch",
            Self::RenameBranch => "renameBranch",
            Self::SwitchBranch => "switchBranch",
            Self::DeleteBranch => "deleteBranch",
            Self::RemoveBranchConfig => "removeBranchConfig",
            Self::Tracking => "tracking",
            Self::CreateTag => "createTag",
            Self::DeleteTag => "deleteTag",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefStepProof {
    pub kind: RefStepKind,
    pub name: String,
    pub old_name: Option<String>,
    pub old_oid: Option<String>,
    pub desired_oid: Option<String>,
    pub target_oid: Option<String>,
    pub upstream: Option<String>,
    #[serde(default)]
    pub failure_reason: Option<RefActionFailureReason>,
    pub phase: FetchStepPhase,
    pub effects: ActionEffects,
    pub before_fingerprint: Option<String>,
    pub after_fingerprint: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefActionProof {
    pub object_format: ObjectFormat,
    pub steps: Vec<RefStepProof>,
}
impl RefActionProof {
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.steps.capacity() * std::mem::size_of::<RefStepProof>()
            + self
                .steps
                .iter()
                .map(|s| {
                    s.name.capacity()
                        + [
                            &s.old_name,
                            &s.old_oid,
                            &s.desired_oid,
                            &s.target_oid,
                            &s.upstream,
                            &s.before_fingerprint,
                            &s.after_fingerprint,
                        ]
                        .into_iter()
                        .map(|v| v.as_ref().map_or(0, String::capacity))
                        .sum::<usize>()
                })
                .sum::<usize>()
    }
    pub fn validate(&self) -> Result<(), AppError> {
        if self.steps.is_empty() || self.steps.len() > 2 {
            return Err(db_error());
        }
        for s in &self.steps {
            for name in std::iter::once(&s.name)
                .chain(s.old_name.iter())
                .chain(s.upstream.iter())
            {
                if name.len() > 1024
                    || !["refs/heads/", "refs/remotes/", "refs/tags/"]
                        .iter()
                        .any(|p| name.starts_with(p))
                    || !crate::history::refs::valid_name(name)
                {
                    return Err(db_error());
                }
            }
            for oid in s
                .old_oid
                .iter()
                .chain(s.desired_oid.iter())
                .chain(s.target_oid.iter())
            {
                SafeTargetProof::Commit {
                    oid: oid.clone(),
                    object_format: self.object_format,
                }
                .validate()?;
            }
            for fp in s
                .before_fingerprint
                .iter()
                .chain(s.after_fingerprint.iter())
            {
                if !fingerprint(fp) {
                    return Err(db_error());
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteReadProof {
    pub remote_name: String,
    pub role: RemoteEndpointRole,
    pub config_digest: String,
    pub phase: FetchStepPhase,
    pub ref_count: Option<String>,
    pub result_digest: Option<String>,
    pub observed_at: Option<String>,
    pub no_git_writes: bool,
}
impl RemoteReadProof {
    pub fn effects(&self) -> ActionEffects {
        if self.phase == FetchStepPhase::Observed && self.no_git_writes {
            ActionEffects::NotApplied
        } else {
            ActionEffects::Unknown
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionRecord {
    pub owner_window: String,
    pub kind: ActionKind,
    pub repo_id: String,
    pub worktree_id: Option<String>,
    pub common_identity: Vec<u8>,
    pub root_identity: Option<Vec<u8>>,
    pub precondition: String,
    pub location: RecoveryLocation,
    pub artifacts: Vec<ArtifactProof>,
    #[serde(default)]
    pub targets: Vec<SafeTargetProof>,
    #[serde(default)]
    pub ref_action: Option<RefActionProof>,
    #[serde(default)]
    pub remote_read: Option<RemoteReadProof>,
    #[serde(default)]
    pub change_action: Option<ChangeActionProof>,
    #[serde(default)]
    pub sync_action: Option<SyncActionProof>,
    pub phase: WritePhase,
    pub postcondition: Option<String>,
    pub outcome: Option<ActionOutcome>,
}
pub fn read(db: &Database, id: &str) -> Result<ActionRecord, AppError> {
    let c = db.lock()?;
    read_from(&c, id)
}
pub(crate) fn read_from(c: &rusqlite::Connection, id: &str) -> Result<ActionRecord, AppError> {
    let text: String = c
        .query_row(
            "SELECT record FROM repository_actions WHERE operation_id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| db_error())?;
    if text.len() > 32768 {
        return Err(db_error());
    }
    serde_json::from_str(&text).map_err(|_| db_error())
}
pub(crate) fn write_to(
    c: &rusqlite::Connection,
    id: &str,
    r: &ActionRecord,
) -> Result<(), AppError> {
    if r.owner_window.len() > 256
        || r.repo_id.len() > 256
        || r.worktree_id.as_ref().is_some_and(|s| s.len() > 256)
        || r.common_identity.len() > 64
        || r.root_identity.as_ref().is_some_and(|s| s.len() > 64)
        || !fingerprint(&r.precondition)
        || r.postcondition.as_ref().is_some_and(|s| !fingerprint(s))
    {
        return Err(db_error());
    }
    if r.location.common.len()
        + r.location.git.len()
        + r.location.worktree.as_ref().map_or(0, Vec::len)
        > 24576
        || r.location.git_identity.len() > 64
        || r.artifacts.len() > 32
    {
        return Err(db_error());
    }
    if let Some(proof) = &r.ref_action {
        proof.validate()?;
    }
    if let Some(p) = &r.change_action {
        let oid = |s: &str| matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit());
        if !super::changes::supported(r.kind)
            || !fingerprint(&p.before_index)
            || p.candidate_digest.as_ref().is_some_and(|s| !fingerprint(s))
            || p.candidate_tree.as_ref().is_some_and(|s| !oid(s))
            || p.before_head.as_ref().is_some_and(|s| !oid(s))
            || p.result.commit_oid.as_ref().is_some_and(|s| !oid(s))
            || p.result.tree_oid.as_ref().is_some_and(|s| !oid(s))
            || p.alternate_name.as_ref().is_some_and(|s| {
                s.strip_prefix("action-index-")
                    .is_none_or(|v| v.len() != 64 || !v.bytes().all(|b| b.is_ascii_hexdigit()))
            })
        {
            return Err(db_error());
        }
    }
    if let Some(p) = &r.sync_action {
        super::sync::validate_proof(p, r.kind)?;
    }
    if let Some(p) = &r.remote_read {
        SafeTargetProof::Remote {
            name: p.remote_name.clone(),
            config_digest: p.config_digest.clone(),
        }
        .validate()?;
        if r.kind != ActionKind::ReadRemoteRefs
            || p.ref_count.as_ref().is_some_and(|s| {
                s.parse::<usize>()
                    .ok()
                    .filter(|n| *n <= 4096 && n.to_string() == *s)
                    .is_none()
            })
            || p.result_digest.as_ref().is_some_and(|s| !fingerprint(s))
            || p.observed_at.as_ref().is_some_and(|s| s.len() > 64)
        {
            return Err(db_error());
        }
    }
    if r.targets.len() > 256 {
        return Err(db_error());
    }
    for target in &r.targets {
        target.validate()?;
    }
    for a in &r.artifacts {
        if a.kind == ArtifactKind::IndexLock {
            if a.name != "index.lock"
                || a.root != ArtifactRoot::Git
                || a.identity.is_empty()
                || a.identity.len() > 64
            {
                return Err(db_error());
            }
            continue;
        }
        let leaf = match a.kind {
            ArtifactKind::RecoveryRef => a.name.strip_prefix("refs/opengit/recovery/"),
            ArtifactKind::AlternateIndex => a.name.strip_prefix("action-index-"),
            ArtifactKind::OwnedLock => a.name.strip_prefix("action-lock-"),
            ArtifactKind::IndexLock => unreachable!(),
        };
        if leaf.is_none_or(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
            || a.identity.is_empty()
            || a.identity.len() > 64
        {
            return Err(db_error());
        }
    }
    let text = serde_json::to_string(r).map_err(|_| db_error())?;
    if text.len() > 32768 {
        return Err(db_error());
    }
    c.execute("INSERT INTO repository_actions(operation_id,record) VALUES(?1,?2) ON CONFLICT(operation_id) DO UPDATE SET record=excluded.record",rusqlite::params![id,text]).map_err(|_|db_error())?;
    Ok(())
}
fn fingerprint(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
pub fn outcome(effects: ActionEffects, cancelled: bool) -> ActionOutcome {
    ActionOutcome {
        effects,
        recovery: match effects {
            ActionEffects::Applied | ActionEffects::NotApplied => ActionRecovery::None,
            ActionEffects::Conflict => ActionRecovery::ResolveConflicts,
            _ => ActionRecovery::Inspect,
        },
        cancellation_requested: cancelled,
        repository_changed: effects != ActionEffects::NotApplied,
        observed_at: chrono::Utc::now().to_rfc3339(),
    }
}

#[cfg(test)]
mod target_proof_tests {
    use super::*;
    #[test]
    fn closed_proofs_reject_urls_raw_authority_wrong_oids_and_unknown_metadata() {
        assert!(SafeTargetProof::Remote {
            name: "https://secret@example.invalid/repo".into(),
            config_digest: "a".repeat(64)
        }
        .validate()
        .is_err());
        assert!(SafeTargetProof::Ref {
            name: "refs/heads/main".into(),
            expected_oid: Some("a".repeat(40)),
            object_format: ObjectFormat::Sha256
        }
        .validate()
        .is_err());
        assert!(serde_json::from_value::<SafeTargetProof>(serde_json::json!({"kind":"remote","name":"origin","configDigest":"a".repeat(64),"url":"https://secret@example.invalid"})).is_err());
        assert!(serde_json::from_value::<SafeTargetProof>(
            serde_json::json!({"kind":"ref","name":"refs/heads/main","objectFormat":"sha1"})
        )
        .is_err());
        SafeTargetProof::Ref {
            name: "refs/heads/main".into(),
            expected_oid: Some("a".repeat(40)),
            object_format: ObjectFormat::Sha1,
        }
        .validate()
        .unwrap();
    }
}
