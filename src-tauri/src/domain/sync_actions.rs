use super::contracts::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncStepKind {
    Push,
    Tracking,
    Upstream,
    Fetch,
    FastForward,
    TagPush,
    TagDelete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncFailureReason {
    Rejected,
    Diverged,
    Stale,
    Unsupported,
    Authentication,
    Network,
    Native,
    Recovery,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncStepResult {
    pub step: SyncStepKind,
    pub phase: FetchStepPhase,
    pub effects: ActionEffects,
    pub failure_reason: Option<SyncFailureReason>,
    pub observed_at: Option<Timestamp>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicationEvidence {
    pub remote_name: String,
    pub role: RemoteEndpointRole,
    pub destination: String,
    pub source_ref: Option<String>,
    pub source_oid: Option<String>,
    pub acknowledged_oid: Option<String>,
    pub acknowledged_at: Timestamp,
    pub config_digest: String,
    pub deleted: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncActionResult {
    pub steps: Vec<SyncStepResult>,
    pub publication: Option<PublicationEvidence>,
    pub tracking_updated: bool,
    pub upstream_updated: bool,
    pub tracking_refresh_required: bool,
    pub fetched_oid: Option<String>,
    pub integrated_oid: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncActionOperationResult {
    pub operation: OperationSnapshot,
    pub result: Option<SyncActionResult>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetPublicationEvidenceRequest {
    pub scope: ActionScope,
    pub source_handle: Id,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicationEvidenceResult {
    pub scope: ActionScope,
    pub observations: Vec<PublicationEvidence>,
}
