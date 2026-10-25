//! Remote endpoint descriptions and runtime-only, owner-bound advertisements.
use super::contracts::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteEndpointRole {
    Fetch,
    Push,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteEndpointCapability {
    Eligible,
    UnsupportedTransport,
    MultipleUrls,
    Mirror,
    UnsupportedRefspec,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteEndpoint {
    pub role: RemoteEndpointRole,
    pub handle: Option<Id>,
    pub capability: RemoteEndpointCapability,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteActionEntry {
    pub name: String,
    pub endpoints: Vec<RemoteEndpoint>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteActionCatalog {
    pub scope: ActionScope,
    pub observed_at: Timestamp,
    pub remotes: Vec<RemoteActionEntry>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetRemoteActionsRequest {
    pub scope: ActionScope,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteRefEntry {
    pub entry_id: Id,
    pub name: String,
    pub oid: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteRefSnapshot {
    pub captured_scope: ActionScope,
    pub snapshot_id: Id,
    pub remote_name: String,
    pub role: RemoteEndpointRole,
    pub observed_at: Timestamp,
    pub expires_at: Timestamp,
    pub object_format: Option<String>,
    pub refs: Vec<RemoteRefEntry>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteSnapshotAvailability {
    Pending,
    Available,
    Expired,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteReadEvidence {
    pub remote_name: String,
    pub role: RemoteEndpointRole,
    pub phase: FetchStepPhase,
    pub ref_count: Option<Counter>,
    pub observed_at: Option<Timestamp>,
    pub no_git_writes: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteActionOperationResult {
    pub operation: OperationSnapshot,
    pub availability: RemoteSnapshotAvailability,
    pub evidence: Option<RemoteReadEvidence>,
    pub snapshot: Option<RemoteRefSnapshot>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteRefNamespace {
    Branch,
    Tag,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum RemoteTargetSource {
    Existing {
        #[serde(rename = "entryId")]
        entry_id: Id,
    },
    New {
        namespace: RemoteRefNamespace,
        name: String,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveRemoteTargetRequest {
    pub scope: ActionScope,
    pub snapshot_id: Id,
    pub source: RemoteTargetSource,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedRemoteTarget {
    pub scope: ActionScope,
    pub snapshot_id: Id,
    pub handle: Id,
    pub name: String,
    pub expected_oid: Option<String>,
    pub namespace: RemoteRefNamespace,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_wire_roundtrips_and_rejects_renderer_authority() {
        let f: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/contracts/remote-actions-wire.json"
        ))
        .unwrap();
        fn rt<T: serde::de::DeserializeOwned + serde::Serialize>(v: &serde_json::Value) {
            let x: T = serde_json::from_value(v.clone()).unwrap();
            assert_eq!(serde_json::to_value(x).unwrap(), *v);
        }
        rt::<GetRemoteActionsRequest>(&f["catalogRequest"]);
        rt::<RemoteActionCatalog>(&f["catalog"]);
        rt::<ActionIntent>(&f["intent"]);
        rt::<RemoteActionOperationResult>(&f["result"]);
        rt::<RemoteActionOperationResult>(&f["unavailable"]);
        rt::<ResolveRemoteTargetRequest>(&f["resolveRequest"]);
        rt::<ResolvedRemoteTarget>(&f["resolved"]);
        for key in ["url", "oid", "refspec", "argv", "environment", "path"] {
            let mut v = f["resolveRequest"].clone();
            v[key] = serde_json::json!("CANARY");
            assert!(serde_json::from_value::<ResolveRemoteTargetRequest>(v).is_err());
        }
        assert!(serde_json::from_value::<ActionIntent>(serde_json::json!({"kind":"readRemoteRefs","remoteHandle":"a".repeat(64),"url":"https://example.invalid"})).is_err());
    }
}
