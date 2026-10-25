//! Local ref-action catalogs are separate from the frozen fetch/context wire.
use super::contracts::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetRefActionsRequest {
    pub scope: ActionScope,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RefActionKind {
    Local,
    Remote,
    Tag,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefActionEntry {
    pub entry_id: Id,
    pub kind: RefActionKind,
    pub name: String,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub oid: Option<String>,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub symbolic_target: Option<String>,
    pub checked_out_here: bool,
    pub checked_out_elsewhere: bool,
    pub can_start: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefActionCatalog {
    pub scope: ActionScope,
    pub catalog_id: Id,
    pub observed_at: Timestamp,
    pub head: Head,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub head_entry_id: Option<Id>,
    pub refs: Vec<RefActionEntry>,
    pub bare: bool,
    pub dirty: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum RefActionSource {
    Ref {
        #[serde(rename = "entryId")]
        entry_id: Id,
    },
    Head,
    HistoryCommit {
        context: HistoryContext,
        #[serde(rename = "commitHandle")]
        commit_handle: Id,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveRefActionTargetRequest {
    pub scope: ActionScope,
    pub catalog_id: Id,
    pub source: RefActionSource,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RefTargetKind {
    Commit,
    Branch,
    RemoteRef,
    Tag,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedRefActionTarget {
    pub scope: ActionScope,
    pub catalog_id: Id,
    pub handle: Id,
    pub kind: RefTargetKind,
    pub display_name: String,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub oid: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefActionStepResult {
    pub step: String,
    pub phase: FetchStepPhase,
    pub effects: ActionEffects,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub failure_reason: Option<RefActionFailureReason>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RefActionFailureReason {
    Identity,
    Signing,
    Native,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefActionResult {
    pub steps: Vec<RefActionStepResult>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefActionOperationResult {
    pub operation: OperationSnapshot,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub result: Option<RefActionResult>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ref_action_wire_is_closed_and_roundtrips() {
        let f: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/contracts/ref-actions-wire.json"
        ))
        .unwrap();
        fn check<T: serde::de::DeserializeOwned + serde::Serialize>(v: &serde_json::Value) {
            assert_eq!(
                serde_json::to_value(serde_json::from_value::<T>(v.clone()).unwrap()).unwrap(),
                *v
            );
        }
        check::<GetRefActionsRequest>(&f["catalogRequest"]);
        check::<RefActionCatalog>(&f["catalog"]);
        check::<ResolvedRefActionTarget>(&f["resolved"]);
        check::<RefActionOperationResult>(&f["result"]);
        check::<RefActionOperationResult>(&f["unknown"]);
        for r in f["resolveRequests"].as_array().unwrap() {
            check::<ResolveRefActionTargetRequest>(r);
        }
        for field in ["oid", "path", "ref", "argv", "url", "environment"] {
            let mut r = f["resolveRequests"][0].clone();
            r.as_object_mut()
                .unwrap()
                .insert(field.into(), serde_json::json!("forged"));
            assert!(serde_json::from_value::<ResolveRefActionTargetRequest>(r).is_err());
        }
        assert!(serde_json::from_value::<RefActionSource>(
            serde_json::json!({"kind":"ref","entryId":"entry-0","name":"refs/heads/main"})
        )
        .is_err());
    }
}
