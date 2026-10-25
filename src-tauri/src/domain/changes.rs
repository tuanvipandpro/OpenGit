use super::contracts::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeSide {
    Staged,
    Unstaged,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeCapability {
    Eligible,
    Unsupported,
    Unavailable,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeEntry {
    pub entry_id: Id,
    pub path_display: String,
    pub previous_path_display: Option<String>,
    pub index_status: PathStatus,
    pub worktree_status: PathStatus,
    pub stage_capability: ChangeCapability,
    pub unstage_capability: ChangeCapability,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryChanges {
    pub scope: ActionScope,
    pub catalog_id: Id,
    pub observed_at: Timestamp,
    pub head: Head,
    pub entries: Vec<ChangeEntry>,
    pub unsupported: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetRepositoryChangesRequest {
    pub scope: ActionScope,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeSelection {
    pub entry_id: Id,
    pub side: ChangeSide,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveChangeTargetsRequest {
    pub scope: ActionScope,
    pub catalog_id: Id,
    pub targets: Vec<ChangeSelection>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeTarget {
    pub entry_id: Id,
    pub side: ChangeSide,
    pub change_handle: Id,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedChangeTargets {
    pub scope: ActionScope,
    pub catalog_id: Id,
    pub targets: Vec<ChangeTarget>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetChangeDiffRequest {
    pub scope: ActionScope,
    pub change_handle: Id,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeDiff {
    pub scope: ActionScope,
    pub change_handle: Id,
    pub diff_id: Id,
    pub content: String,
    pub partial_eligibility: String,
    pub hunks: Vec<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeFailureReason {
    Identity,
    HookOrNative,
    Signing,
    Empty,
    Stale,
    Locked,
    Unsupported,
    Recovery,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeActionResult {
    pub commit_oid: Option<String>,
    pub tree_oid: Option<String>,
    pub failure_reason: Option<ChangeFailureReason>,
    pub artifacts_retained: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeActionOperationResult {
    pub operation: OperationSnapshot,
    pub result: Option<ChangeActionResult>,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_changes_are_closed_capabilities_and_roundtrip_without_path_authority() {
        let fixture: serde_json::Value = serde_json::from_str(
            include_str!("../../../fixtures/contracts/changes-wire.json")
                .trim_start_matches('\u{feff}'),
        )
        .unwrap();
        let catalog: RepositoryChanges =
            serde_json::from_value(fixture["catalog"].clone()).unwrap();
        assert_eq!(catalog.scope.generation, "9007199254740993");
        assert_eq!(
            catalog.entries[0].stage_capability,
            ChangeCapability::Eligible
        );
        let resolved: ResolvedChangeTargets =
            serde_json::from_value(fixture["resolved"].clone()).unwrap();
        assert_eq!(resolved.targets[0].side, ChangeSide::Unstaged);
        assert_eq!(resolved.targets[0].change_handle.len(), 64);
        let _: ChangeDiff = serde_json::from_value(fixture["diff"].clone()).unwrap();
        let _: ChangeActionOperationResult =
            serde_json::from_value(fixture["result"].clone()).unwrap();
        for key in ["path", "oid", "patch", "environment", "argv"] {
            let mut request = fixture["resolveRequest"].clone();
            request[key] = serde_json::json!("forged");
            assert!(serde_json::from_value::<ResolveChangeTargetsRequest>(request).is_err());
        }
        assert!(serde_json::from_value::<ChangeSelection>(
            serde_json::json!({"entryId":"descriptive","side":"wholeRepository"})
        )
        .is_err());
        let mut request = fixture["diffRequest"].clone();
        request["pathDisplay"] = serde_json::json!("../secret");
        assert!(serde_json::from_value::<GetChangeDiffRequest>(request).is_err());
    }
}
