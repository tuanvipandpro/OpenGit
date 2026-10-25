// Additive v0.2 history DTOs retain the foundation import surface.
pub use super::history::*;
use serde::{Deserialize, Serialize};
pub type Id = String;
pub type Counter = String;
pub type Timestamp = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub phase: Phase,
    pub retry: AppErrorRetry,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub available: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub contract_version: EnvironmentContractVersion,
    pub app_version: String,
    pub platform: EnvironmentPlatform,
    pub git: ToolInfo,
    pub credential_manager: ToolInfo,
    pub open_ssh: ToolInfo,
    pub web_view2: ToolInfo,
    pub identity: IdentityPresence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityPresence {
    pub name_configured: bool,
    pub email_configured: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub repo_id: Id,
    pub worktree_id: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeSummary {
    pub id: Id,
    pub root_display: String,
    pub is_main: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySummary {
    pub id: Id,
    pub name: String,
    pub common_git_dir_display: String,
    pub is_bare: bool,
    pub availability: RepositorySummaryAvailability,
    pub worktrees: Vec<WorktreeSummary>,
    pub last_opened_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: Id,
    pub revision: Counter,
    pub repositories: Vec<RepositorySummary>,
    pub active_selection: Option<Selection>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Head {
    pub kind: HeadKind,
    pub branch: Option<String>,
    pub oid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEntry {
    pub id: Id,
    pub kind: StatusEntryKind,
    pub path_display: String,
    pub previous_path_display: Option<String>,
    pub index_status: PathStatus,
    pub worktree_status: PathStatus,
    pub submodule: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSummary {
    pub name: String,
    pub fetch_url_display: Option<String>,
    pub push_url_display: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySnapshot {
    pub repo_id: Id,
    pub worktree_id: Option<Id>,
    pub generation: Counter,
    pub observed_at: Timestamp,
    pub is_bare: bool,
    pub head: Head,
    pub status: Vec<StatusEntry>,
    pub is_dirty: Option<bool>,
    pub is_shallow: bool,
    pub remotes: Vec<RemoteSummary>,
    pub identity: IdentityPresence,
    pub git_operations: Vec<GitOperation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalCloneSource {
    pub source_handle: Id,
    pub display: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneDestination {
    pub destination_handle: Id,
    pub parent_display: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthPrompt {
    pub prompt_handle: Id,
    pub kind: AuthPromptKind,
    pub message: String,
    pub expires_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub completed: u64,
    pub total: Option<u64>,
    pub unit: ProgressUnit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationSnapshot {
    pub id: Id,
    pub sequence: Counter,
    pub kind: OperationSnapshotKind,
    pub state: OperationState,
    pub phase: Phase,
    pub repo_id: Option<Id>,
    pub worktree_id: Option<Id>,
    pub source_display: String,
    pub destination_display: String,
    pub progress: Option<Progress>,
    pub can_cancel: bool,
    pub auth_prompt: Option<AuthPrompt>,
    pub error: Option<AppError>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationsSnapshot {
    pub stream_id: Id,
    pub sequence: Counter,
    pub operations: Vec<OperationSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationEvent {
    pub stream_id: Id,
    pub sequence: Counter,
    pub operation: OperationSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    #[serde(rename = "InvalidRequest")]
    InvalidRequest,
    #[serde(rename = "InvalidHandle")]
    InvalidHandle,
    #[serde(rename = "NotFound")]
    NotFound,
    #[serde(rename = "GitUnavailable")]
    GitUnavailable,
    #[serde(rename = "UnsupportedGit")]
    UnsupportedGit,
    #[serde(rename = "NotRepository")]
    NotRepository,
    #[serde(rename = "RepositoryUnavailable")]
    RepositoryUnavailable,
    #[serde(rename = "UnsafeRepository")]
    UnsafeRepository,
    #[serde(rename = "DestinationExists")]
    DestinationExists,
    #[serde(rename = "DestinationBusy")]
    DestinationBusy,
    #[serde(rename = "UnsafePath")]
    UnsafePath,
    #[serde(rename = "InvalidSource")]
    InvalidSource,
    #[serde(rename = "AuthenticationRequired")]
    AuthenticationRequired,
    #[serde(rename = "AuthenticationFailed")]
    AuthenticationFailed,
    #[serde(rename = "PermissionDenied")]
    PermissionDenied,
    #[serde(rename = "Network")]
    Network,
    #[serde(rename = "Certificate")]
    Certificate,
    #[serde(rename = "SshHostKey")]
    SshHostKey,
    #[serde(rename = "SshAuthentication")]
    SshAuthentication,
    #[serde(rename = "Timeout")]
    Timeout,
    #[serde(rename = "OutputLimit")]
    OutputLimit,
    #[serde(rename = "MalformedOutput")]
    MalformedOutput,
    #[serde(rename = "ExternalChange")]
    ExternalChange,
    #[serde(rename = "DiskFull")]
    DiskFull,
    #[serde(rename = "Io")]
    Io,
    #[serde(rename = "Database")]
    Database,
    #[serde(rename = "Interrupted")]
    Interrupted,
    #[serde(rename = "Internal")]
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    #[serde(rename = "validation")]
    Validation,
    #[serde(rename = "discovery")]
    Discovery,
    #[serde(rename = "inspection")]
    Inspection,
    #[serde(rename = "queue")]
    Queue,
    #[serde(rename = "authentication")]
    Authentication,
    #[serde(rename = "transfer")]
    Transfer,
    #[serde(rename = "checkout")]
    Checkout,
    #[serde(rename = "verification")]
    Verification,
    #[serde(rename = "publish")]
    Publish,
    #[serde(rename = "registration")]
    Registration,
    #[serde(rename = "recovery")]
    Recovery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppErrorRetry {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "retry")]
    Retry,
    #[serde(rename = "chooseAnotherDestination")]
    ChooseAnotherDestination,
    #[serde(rename = "checkCredentials")]
    CheckCredentials,
    #[serde(rename = "checkNetwork")]
    CheckNetwork,
    #[serde(rename = "checkTrust")]
    CheckTrust,
    #[serde(rename = "reopenRepository")]
    ReopenRepository,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnvironmentContractVersion {
    #[serde(rename = "1")]
    Value0,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnvironmentPlatform {
    #[serde(rename = "windows")]
    Windows,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepositorySummaryAvailability {
    #[serde(rename = "available")]
    Available,
    #[serde(rename = "missing")]
    Missing,
    #[serde(rename = "unavailable")]
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeadKind {
    #[serde(rename = "branch")]
    Branch,
    #[serde(rename = "detached")]
    Detached,
    #[serde(rename = "unborn")]
    Unborn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitOperation {
    #[serde(rename = "merge")]
    Merge,
    #[serde(rename = "rebase")]
    Rebase,
    #[serde(rename = "cherryPick")]
    CherryPick,
    #[serde(rename = "revert")]
    Revert,
    #[serde(rename = "bisect")]
    Bisect,
    #[serde(rename = "sequencer")]
    Sequencer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathStatus {
    #[serde(rename = ".")]
    Value0,
    #[serde(rename = "M")]
    M,
    #[serde(rename = "T")]
    T,
    #[serde(rename = "A")]
    A,
    #[serde(rename = "D")]
    D,
    #[serde(rename = "R")]
    R,
    #[serde(rename = "C")]
    C,
    #[serde(rename = "U")]
    U,
    #[serde(rename = "?")]
    Value8,
    #[serde(rename = "!")]
    Value9,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatusEntryKind {
    #[serde(rename = "ordinary")]
    Ordinary,
    #[serde(rename = "rename")]
    Rename,
    #[serde(rename = "unmerged")]
    Unmerged,
    #[serde(rename = "untracked")]
    Untracked,
    #[serde(rename = "ignored")]
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationState {
    #[serde(rename = "Queued")]
    Queued,
    #[serde(rename = "Running")]
    Running,
    #[serde(rename = "WaitingForUser")]
    WaitingForUser,
    #[serde(rename = "Cancelling")]
    Cancelling,
    #[serde(rename = "Succeeded")]
    Succeeded,
    #[serde(rename = "Failed")]
    Failed,
    #[serde(rename = "Cancelled")]
    Cancelled,
    #[serde(rename = "Interrupted")]
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthPromptKind {
    #[serde(rename = "username")]
    Username,
    #[serde(rename = "password")]
    Password,
    #[serde(rename = "passphrase")]
    Passphrase,
    #[serde(rename = "confirmation")]
    Confirmation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgressUnit {
    #[serde(rename = "objects")]
    Objects,
    #[serde(rename = "bytes")]
    Bytes,
    #[serde(rename = "steps")]
    Steps,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationSnapshotKind {
    #[serde(rename = "clone")]
    Clone,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApiResult<T> {
    Success(T),
    Failure(AppError),
}
impl<T: Serialize> Serialize for ApiResult<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut value = serializer.serialize_struct("ApiResult", 2)?;
        match self {
            Self::Success(data) => {
                value.serialize_field("ok", &true)?;
                value.serialize_field("value", data)?;
            }
            Self::Failure(error) => {
                value.serialize_field("ok", &false)?;
                value.serialize_field("error", error)?;
            }
        }
        value.end()
    }
}
impl<'de, T: serde::de::DeserializeOwned> Deserialize<'de> for ApiResult<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut object = serde_json::Value::deserialize(deserializer)?
            .as_object()
            .cloned()
            .ok_or_else(|| serde::de::Error::custom("Invalid API result"))?;
        let ok = object
            .remove("ok")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| serde::de::Error::custom("Invalid API result"))?;
        if object.len() != 1 {
            return Err(serde::de::Error::custom("Invalid API result"));
        }
        if ok {
            let value = object
                .remove("value")
                .ok_or_else(|| serde::de::Error::custom("Missing result"))?;
            serde_json::from_value(value)
                .map(Self::Success)
                .map_err(serde::de::Error::custom)
        } else {
            let error = object
                .remove("error")
                .ok_or_else(|| serde::de::Error::custom("Missing error"))?;
            serde_json::from_value(error)
                .map(Self::Failure)
                .map_err(serde::de::Error::custom)
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum CloneSource {
    Remote {
        url: String,
    },
    Local {
        #[serde(rename = "sourceHandle")]
        source_handle: Id,
    },
}
fn required_nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(d)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EmptyRequest {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectRepositoryRequest {
    pub repo_id: Id,
    #[serde(deserialize_with = "required_nullable")]
    pub worktree_id: Option<Id>,
    pub generation: Counter,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectRepositoryRequest {
    pub repo_id: Id,
    #[serde(deserialize_with = "required_nullable")]
    pub worktree_id: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForgetRepositoryRequest {
    pub repo_id: Id,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartCloneRequest {
    pub source: CloneSource,
    pub destination_handle: Id,
    pub directory_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelOperationRequest {
    pub operation_id: Id,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RespondAuthPromptRequest {
    pub operation_id: Id,
    pub prompt_handle: Id,
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    fn roundtrip<T: serde::de::DeserializeOwned + Serialize>(v: &Value) {
        let parsed: T = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *v);
    }
    #[test]
    fn cross_language_wire_fixtures() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../fixtures/contracts/wire.json")).unwrap();
        for sample in fixture["commands"].as_array().unwrap() {
            match sample["command"].as_str().unwrap() {
                "get_environment" => {
                    roundtrip::<EmptyRequest>(&sample["request"]);
                    roundtrip::<ApiResult<Environment>>(&sample["result"]);
                }
                "get_workspace" => {
                    roundtrip::<EmptyRequest>(&sample["request"]);
                    roundtrip::<ApiResult<Workspace>>(&sample["result"]);
                }
                "choose_repository" => {
                    roundtrip::<EmptyRequest>(&sample["request"]);
                    roundtrip::<ApiResult<Option<Workspace>>>(&sample["result"]);
                }
                "inspect_repository" => {
                    roundtrip::<InspectRepositoryRequest>(&sample["request"]);
                    roundtrip::<ApiResult<RepositorySnapshot>>(&sample["result"]);
                }
                "select_repository" => {
                    roundtrip::<SelectRepositoryRequest>(&sample["request"]);
                    roundtrip::<ApiResult<Workspace>>(&sample["result"]);
                }
                "forget_repository" => {
                    roundtrip::<ForgetRepositoryRequest>(&sample["request"]);
                    roundtrip::<ApiResult<Workspace>>(&sample["result"]);
                }
                "choose_clone_source" => {
                    roundtrip::<EmptyRequest>(&sample["request"]);
                    roundtrip::<ApiResult<Option<LocalCloneSource>>>(&sample["result"]);
                }
                "choose_clone_destination" => {
                    roundtrip::<EmptyRequest>(&sample["request"]);
                    roundtrip::<ApiResult<Option<CloneDestination>>>(&sample["result"]);
                }
                "start_clone" => {
                    roundtrip::<StartCloneRequest>(&sample["request"]);
                    roundtrip::<ApiResult<OperationSnapshot>>(&sample["result"]);
                }
                "get_operations" => {
                    roundtrip::<EmptyRequest>(&sample["request"]);
                    roundtrip::<ApiResult<OperationsSnapshot>>(&sample["result"]);
                }
                "cancel_operation" => {
                    roundtrip::<CancelOperationRequest>(&sample["request"]);
                    roundtrip::<ApiResult<OperationSnapshot>>(&sample["result"]);
                }
                "respond_auth_prompt" => {
                    roundtrip::<RespondAuthPromptRequest>(&sample["request"]);
                    roundtrip::<ApiResult<Option<()>>>(&sample["result"]);
                }
                _ => panic!("Unknown fixture command"),
            }
        }
        for snapshot in fixture["snapshots"].as_array().unwrap() {
            roundtrip::<RepositorySnapshot>(snapshot);
        }
        roundtrip::<ApiResult<()>>(&fixture["error"]);
        roundtrip::<OperationEvent>(&fixture["event"]);
        for source in fixture["cloneSources"].as_array().unwrap() {
            roundtrip::<CloneSource>(source);
        }
        for value in fixture["enumValues"]["ErrorCode"].as_array().unwrap() {
            roundtrip::<ErrorCode>(value);
        }
        for value in fixture["enumValues"]["Phase"].as_array().unwrap() {
            roundtrip::<Phase>(value);
        }
        for value in fixture["enumValues"]["AppErrorRetry"].as_array().unwrap() {
            roundtrip::<AppErrorRetry>(value);
        }
        for value in fixture["enumValues"]["EnvironmentContractVersion"]
            .as_array()
            .unwrap()
        {
            roundtrip::<EnvironmentContractVersion>(value);
        }
        for value in fixture["enumValues"]["EnvironmentPlatform"]
            .as_array()
            .unwrap()
        {
            roundtrip::<EnvironmentPlatform>(value);
        }
        for value in fixture["enumValues"]["RepositorySummaryAvailability"]
            .as_array()
            .unwrap()
        {
            roundtrip::<RepositorySummaryAvailability>(value);
        }
        for value in fixture["enumValues"]["HeadKind"].as_array().unwrap() {
            roundtrip::<HeadKind>(value);
        }
        for value in fixture["enumValues"]["GitOperation"].as_array().unwrap() {
            roundtrip::<GitOperation>(value);
        }
        for value in fixture["enumValues"]["PathStatus"].as_array().unwrap() {
            roundtrip::<PathStatus>(value);
        }
        for value in fixture["enumValues"]["StatusEntryKind"].as_array().unwrap() {
            roundtrip::<StatusEntryKind>(value);
        }
        for value in fixture["enumValues"]["OperationState"].as_array().unwrap() {
            roundtrip::<OperationState>(value);
        }
        for value in fixture["enumValues"]["AuthPromptKind"].as_array().unwrap() {
            roundtrip::<AuthPromptKind>(value);
        }
        for value in fixture["enumValues"]["ProgressUnit"].as_array().unwrap() {
            roundtrip::<ProgressUnit>(value);
        }
        for value in fixture["enumValues"]["OperationSnapshotKind"]
            .as_array()
            .unwrap()
        {
            roundtrip::<OperationSnapshotKind>(value);
        }
    }
    #[test]
    fn strict_requests_reject_unscoped_and_missing_arguments() {
        assert!(serde_json::from_value::<EmptyRequest>(json!({"cwd":"C:/"})).is_err());
        assert!(serde_json::from_value::<InspectRepositoryRequest>(
            json!({"repoId":"fixture", "generation":"1"})
        )
        .is_err());
        assert!(serde_json::from_value::<SelectRepositoryRequest>(
            json!({"repoId":"fixture", "worktreeId":null, "path":"C:/"})
        )
        .is_err());
        assert!(serde_json::from_value::<StartCloneRequest>(json!({"source":{"kind":"local","sourceHandle":"fixture","url":"unexpected"},"destinationHandle":"fixture","directoryName":"new"})).is_err());
        assert!(serde_json::from_value::<RespondAuthPromptRequest>(
            json!({"operationId":"fixture","promptHandle":"fixture"})
        )
        .is_err());
        assert!(serde_json::from_value::<ApiResult<()>>(json!({"ok":true,"error":null})).is_err());
    }
}
