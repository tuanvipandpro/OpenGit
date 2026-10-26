//! Save/Apply only. Displayed ordinals/OIDs never authorize a stash mutation.
use super::contracts::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StashEntry {
    pub entry_id: Id,
    pub ordinal: Counter,
    pub oid: String,
    pub base_oid: Option<String>,
    pub includes_untracked: bool,
    pub apply_available: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryStashes {
    pub scope: ActionScope,
    pub catalog_id: Id,
    pub observed_at: Timestamp,
    pub entries: Vec<StashEntry>,
    pub save_available: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetRepositoryStashesRequest {
    pub scope: ActionScope,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveStashTargetRequest {
    pub scope: ActionScope,
    pub catalog_id: Id,
    pub entry_id: Id,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedStashTarget {
    pub scope: ActionScope,
    pub catalog_id: Id,
    pub entry_id: Id,
    pub stash_handle: Id,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StashFailureReason {
    Stale,
    Dirty,
    Collision,
    Conflict,
    Locked,
    Native,
    Unsupported,
    Recovery,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StashResolution {
    NotRequired,
    Unmerged,
    ResolvedExternally,
    Inspect,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StashRetention {
    Present,
    Absent,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StashActionResult {
    pub stash_oid: Option<String>,
    pub stash_retention: StashRetention,
    pub recovery_ref_retention: StashRetention,
    pub resolution: StashResolution,
    pub failure_reason: Option<StashFailureReason>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StashActionOperationResult {
    pub operation: OperationSnapshot,
    pub result: Option<StashActionResult>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetStashActionOutcomeRequest {
    pub operation_id: Id,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReconcileStashActionRequest {
    pub scope: ActionScope,
    pub operation_id: Id,
}
