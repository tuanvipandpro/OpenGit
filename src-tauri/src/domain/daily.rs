//! Closed daily-action DTOs. Raw paths, revisions, argv and environment are not IPC authority.
use super::contracts::{Counter, Id, Timestamp};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionScope {
    pub repo_id: Id,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub worktree_id: Option<Id>,
    pub generation: Counter,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum TagAnnotation {
    Lightweight,
    Annotated { message: String, sign: bool },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ActionIntent {
    ReadRemoteRefs {
        #[serde(rename = "remoteHandle")]
        remote_handle: Id,
    },
    Fetch {
        #[serde(rename = "remoteHandles")]
        remote_handles: Vec<Id>,
        #[serde(rename = "prune")]
        prune: bool,
    },
    BranchCreate {
        #[serde(rename = "name")]
        name: String,
        #[serde(rename = "startHandle")]
        start_handle: Id,
        #[serde(rename = "checkout")]
        checkout: bool,
    },
    BranchRename {
        #[serde(rename = "branchHandle")]
        branch_handle: Id,
        #[serde(rename = "newName")]
        new_name: String,
    },
    BranchSwitch {
        #[serde(rename = "branchHandle")]
        branch_handle: Id,
    },
    BranchDelete {
        #[serde(rename = "branchHandle")]
        branch_handle: Id,
    },
    BranchTracking {
        #[serde(rename = "branchHandle")]
        branch_handle: Id,
        #[serde(
            rename = "upstreamHandle",
            deserialize_with = "super::contracts::required_nullable"
        )]
        upstream_handle: Option<Id>,
    },
    TagCreate {
        #[serde(rename = "name")]
        name: String,
        #[serde(rename = "targetHandle")]
        target_handle: Id,
        #[serde(rename = "annotation")]
        annotation: TagAnnotation,
    },
    TagDelete {
        #[serde(rename = "tagHandle")]
        tag_handle: Id,
    },
    Push {
        #[serde(rename = "remoteHandle")]
        remote_handle: Id,
        #[serde(rename = "sourceHandle")]
        source_handle: Id,
        #[serde(rename = "destinationHandle")]
        destination_handle: Id,
        #[serde(rename = "setUpstream")]
        set_upstream: bool,
    },
    RemoteTagPush {
        #[serde(rename = "remoteHandle")]
        remote_handle: Id,
        #[serde(rename = "tagHandle")]
        tag_handle: Id,
        #[serde(rename = "destinationHandle")]
        destination_handle: Id,
    },
    RemoteTagDelete {
        #[serde(rename = "remoteHandle")]
        remote_handle: Id,
        #[serde(rename = "tagHandle")]
        tag_handle: Id,
    },
    Pull {
        #[serde(rename = "upstreamHandle")]
        upstream_handle: Id,
    },
    Stage {
        #[serde(rename = "changeHandles")]
        change_handles: Vec<Id>,
    },
    Unstage {
        #[serde(rename = "changeHandles")]
        change_handles: Vec<Id>,
    },
    StageSelection {
        #[serde(rename = "selectionHandles")]
        selection_handles: Vec<Id>,
    },
    UnstageSelection {
        #[serde(rename = "selectionHandles")]
        selection_handles: Vec<Id>,
    },
    Commit {
        #[serde(rename = "message")]
        message: String,
        #[serde(rename = "amend")]
        amend: bool,
    },
    StashPush {
        #[serde(rename = "message")]
        message: String,
        #[serde(rename = "includeUntracked")]
        include_untracked: bool,
    },
    StashApply {
        #[serde(rename = "stashHandle")]
        stash_handle: Id,
    },
    StashPop {
        #[serde(rename = "stashHandle")]
        stash_handle: Id,
    },
    StashDrop {
        #[serde(rename = "stashHandle")]
        stash_handle: Id,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionKind {
    ReadRemoteRefs,
    Fetch,
    BranchCreate,
    BranchRename,
    BranchSwitch,
    BranchDelete,
    BranchTracking,
    TagCreate,
    TagDelete,
    Push,
    RemoteTagPush,
    RemoteTagDelete,
    Pull,
    Stage,
    Unstage,
    StageSelection,
    UnstageSelection,
    Commit,
    StashPush,
    StashApply,
    StashPop,
    StashDrop,
}
impl ActionIntent {
    pub fn kind(&self) -> ActionKind {
        match self {
            Self::ReadRemoteRefs { .. } => ActionKind::ReadRemoteRefs,
            Self::Fetch { .. } => ActionKind::Fetch,
            Self::BranchCreate { .. } => ActionKind::BranchCreate,
            Self::BranchRename { .. } => ActionKind::BranchRename,
            Self::BranchSwitch { .. } => ActionKind::BranchSwitch,
            Self::BranchDelete { .. } => ActionKind::BranchDelete,
            Self::BranchTracking { .. } => ActionKind::BranchTracking,
            Self::TagCreate { .. } => ActionKind::TagCreate,
            Self::TagDelete { .. } => ActionKind::TagDelete,
            Self::Push { .. } => ActionKind::Push,
            Self::RemoteTagPush { .. } => ActionKind::RemoteTagPush,
            Self::RemoteTagDelete { .. } => ActionKind::RemoteTagDelete,
            Self::Pull { .. } => ActionKind::Pull,
            Self::Stage { .. } => ActionKind::Stage,
            Self::Unstage { .. } => ActionKind::Unstage,
            Self::StageSelection { .. } => ActionKind::StageSelection,
            Self::UnstageSelection { .. } => ActionKind::UnstageSelection,
            Self::Commit { .. } => ActionKind::Commit,
            Self::StashPush { .. } => ActionKind::StashPush,
            Self::StashApply { .. } => ActionKind::StashApply,
            Self::StashPop { .. } => ActionKind::StashPop,
            Self::StashDrop { .. } => ActionKind::StashDrop,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionEffects {
    Applied,
    NotApplied,
    Conflict,
    Partial,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionRecovery {
    None,
    Inspect,
    ResolveConflicts,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionOutcome {
    pub effects: ActionEffects,
    pub recovery: ActionRecovery,
    pub cancellation_requested: bool,
    pub repository_changed: bool,
    pub observed_at: Timestamp,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionWarning {
    DirtyWorktree,
    DetachedHead,
    UnbornHead,
    ExternalRace,
    DestructiveRemote,
    IncompleteHistory,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedAction {
    pub scope: ActionScope,
    pub plan_id: Id,
    pub kind: ActionKind,
    pub expires_at: Timestamp,
    pub warnings: Vec<ActionWarning>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareRepositoryActionRequest {
    pub scope: ActionScope,
    pub intent: ActionIntent,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecuteRepositoryActionRequest {
    pub plan_id: Id,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetActionOutcomeRequest {
    pub operation_id: Id,
}

/// Read catalogs contain descriptive names/OIDs; only issued handles authorize actions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetRepositoryActionsRequest {
    pub scope: ActionScope,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetCommitContainmentRequest {
    pub context: super::contracts::HistoryContext,
    pub commit_handle: Id,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionBranchKind {
    Local,
    Remote,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Membership {
    Contains,
    DoesNotContain,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AncestryCompleteness {
    Complete,
    Shallow,
    Unavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FetchCapability {
    EligibleProbeRequired,
    UnsupportedTransport,
    UnsupportedRefspec,
    Mirror,
    NoRefspec,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefObservation {
    pub remote_name: String,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub observed_oid: Option<String>,
    pub observed_at: Timestamp,
    pub matches_current: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionBranch {
    pub name: String,
    pub oid: String,
    pub kind: ActionBranchKind,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub observation: Option<RefObservation>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SymbolicBranch {
    pub name: String,
    pub target: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionRemote {
    pub name: String,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub handle: Option<Id>,
    pub fetch: FetchCapability,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncStatus {
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub current_branch: Option<String>,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub upstream: Option<String>,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub ahead: Option<Counter>,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub behind: Option<Counter>,
    pub completeness: AncestryCompleteness,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryActionCatalog {
    pub scope: ActionScope,
    pub snapshot_id: Id,
    pub observed_at: Timestamp,
    pub head: super::contracts::Head,
    pub branches: Vec<ActionBranch>,
    pub symbolic_branches: Vec<SymbolicBranch>,
    pub remotes: Vec<ActionRemote>,
    pub sync: SyncStatus,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContainmentBranch {
    pub branch: ActionBranch,
    pub membership: Membership,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommitContainment {
    pub context: super::contracts::HistoryContext,
    pub commit_handle: Id,
    pub oid: String,
    pub observed_at: Timestamp,
    pub branches: Vec<ContainmentBranch>,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub current_branch: Option<String>,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub upstream: Option<String>,
    pub completeness: AncestryCompleteness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FetchStepPhase {
    Pending,
    Started,
    Observed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FetchRefStatus {
    Created,
    Updated,
    Forced,
    Deleted,
    UpToDate,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FetchRefChange {
    pub name: String,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub old_oid: Option<String>,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub new_oid: Option<String>,
    pub status: FetchRefStatus,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FetchRemoteResult {
    pub name: String,
    pub phase: FetchStepPhase,
    pub effects: ActionEffects,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub error: Option<super::contracts::AppError>,
    pub reported_refs: Vec<FetchRefChange>,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub observed_at: Option<Timestamp>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FetchResult {
    pub prune: bool,
    pub remotes: Vec<FetchRemoteResult>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionOperationResult {
    pub operation: super::contracts::OperationSnapshot,
    #[serde(deserialize_with = "super::contracts::required_nullable")]
    pub fetch: Option<FetchResult>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_intents_roundtrip_and_reject_authority_fields() {
        let f: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/contracts/daily-wire.json"))
                .unwrap();
        fn roundtrip<T: serde::de::DeserializeOwned + serde::Serialize>(value: &serde_json::Value) {
            let parsed: T = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), *value);
        }
        roundtrip::<GetRepositoryActionsRequest>(&f["catalogRequest"]);
        roundtrip::<RepositoryActionCatalog>(&f["catalog"]);
        roundtrip::<GetCommitContainmentRequest>(&f["containmentRequest"]);
        roundtrip::<CommitContainment>(&f["containment"]);
        roundtrip::<CommitContainment>(&f["shallowContainment"]);
        roundtrip::<ActionOperationResult>(&f["actionOperationResult"]);
        roundtrip::<ActionOperationResult>(&f["unknownWithoutLedger"]);
        for value in f["fetchCapabilities"].as_array().unwrap() {
            roundtrip::<FetchCapability>(value);
        }
        assert!(serde_json::from_value::<GetCommitContainmentRequest>(
            serde_json::json!({"context":f["containmentRequest"]["context"],"oid":"a".repeat(40)})
        )
        .is_err());
        assert!(serde_json::from_value::<PrepareRepositoryActionRequest>(serde_json::json!({"scope":f["prepared"]["scope"],"intent":{"kind":"fetch","remoteHandles":[],"prune":false},"url":"https://untrusted.invalid"})).is_err());
        let operation: super::super::contracts::OperationSnapshot =
            serde_json::from_value(f["operation"].clone()).unwrap();
        assert_eq!(serde_json::to_value(operation).unwrap(), f["operation"]);
        for value in f["intents"].as_array().unwrap() {
            let v: ActionIntent = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(v).unwrap(), *value);
        }
        assert!(serde_json::from_value::<ActionIntent>(
            serde_json::json!({"kind":"branchSwitch","branchHandle":"token","path":"C:/"})
        )
        .is_err());
        assert!(serde_json::from_value::<ActionIntent>(
            serde_json::json!({"kind":"run","argv":["reset","--hard"]})
        )
        .is_err());
        assert!(serde_json::from_value::<ActionScope>(
            serde_json::json!({"repoId":"repo","generation":"1"})
        )
        .is_err());
    }
}
