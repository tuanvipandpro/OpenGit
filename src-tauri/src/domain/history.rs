//! Additive history v1 wire types; services are implemented in subsequent packets.
use super::contracts::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryScope {
    pub repo_id: Id,
    #[serde(deserialize_with = "required_nullable")]
    pub worktree_id: Option<Id>,
    pub generation: Counter,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryContext {
    pub scope: HistoryScope,
    pub snapshot_id: Id,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryFilter {
    pub hidden_refs: Vec<String>,
    pub solo_refs: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryRefKind {
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "remote")]
    Remote,
    #[serde(rename = "tag")]
    Tag,
    #[serde(rename = "stash")]
    Stash,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryRef {
    pub name: String,
    pub display_name: String,
    pub kind: HistoryRefKind,
    pub oid: String,
    #[serde(deserialize_with = "required_nullable")]
    pub commit_oid: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub symbolic_target: Option<String>,
    pub hidden: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParentBoundary {
    #[serde(rename = "loaded")]
    Loaded,
    #[serde(rename = "offPage")]
    OffPage,
    #[serde(rename = "outsideView")]
    OutsideView,
    #[serde(rename = "shallow")]
    Shallow,
    #[serde(rename = "missing")]
    Missing,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryParent {
    pub oid: String,
    pub boundary: ParentBoundary,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommitSummary {
    pub handle: Id,
    pub oid: String,
    pub parents: Vec<HistoryParent>,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: Timestamp,
    pub committed_at: Timestamp,
    pub subject: String,
    pub decorations: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryPage {
    pub context: HistoryContext,
    pub session_id: Id,
    pub commits: Vec<CommitSummary>,
    #[serde(deserialize_with = "required_nullable")]
    pub next_cursor: Option<Id>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistorySession {
    pub context: HistoryContext,
    pub session_id: Id,
    pub head: Head,
    pub refs: Vec<HistoryRef>,
    pub roots: Vec<String>,
    pub page: HistoryPage,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistorySearchHit {
    pub commit: CommitSummary,
    pub outside_view: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistorySearchPage {
    pub context: HistoryContext,
    pub session_id: Id,
    pub query: String,
    pub commits: Vec<HistorySearchHit>,
    pub refs: Vec<HistoryRef>,
    #[serde(deserialize_with = "required_nullable")]
    pub next_cursor: Option<Id>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitFileKind {
    #[serde(rename = "added")]
    Added,
    #[serde(rename = "deleted")]
    Deleted,
    #[serde(rename = "modified")]
    Modified,
    #[serde(rename = "renamed")]
    Renamed,
    #[serde(rename = "copied")]
    Copied,
    #[serde(rename = "typeChanged")]
    TypeChanged,
    #[serde(rename = "unmerged")]
    Unmerged,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommitFile {
    pub handle: Id,
    pub path_display: String,
    #[serde(deserialize_with = "required_nullable")]
    pub previous_path_display: Option<String>,
    pub kind: CommitFileKind,
    #[serde(deserialize_with = "required_nullable")]
    pub old_mode: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub new_mode: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommitDetails {
    pub context: HistoryContext,
    pub commit: CommitSummary,
    pub message: String,
    pub committer_name: String,
    pub committer_email: String,
    pub base: DiffBase,
    pub files: Vec<CommitFile>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffUnavailableReason {
    #[serde(rename = "binary")]
    Binary,
    #[serde(rename = "gitlink")]
    Gitlink,
    #[serde(rename = "tooLarge")]
    TooLarge,
    #[serde(rename = "unsupportedEncoding")]
    UnsupportedEncoding,
    #[serde(rename = "missingObject")]
    MissingObject,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommitDiff {
    pub context: HistoryContext,
    pub file_handle: Id,
    pub content: DiffContent,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryView {
    pub filter: HistoryFilter,
    #[serde(deserialize_with = "required_nullable")]
    pub selected_oid: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub anchor_oid: Option<String>,
    pub anchor_offset: f64,
    pub sidebar_width: u32,
    pub details_height: u32,
    pub diff_layout: DiffLayout,
    pub sidebar_collapsed: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryViewState {
    pub scope: HistoryScope,
    pub view_revision: Counter,
    pub view: RepositoryView,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WatchStatus {
    #[serde(rename = "watching")]
    Watching,
    #[serde(rename = "fallback")]
    Fallback,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryWatch {
    pub scope: HistoryScope,
    pub subscription_id: Id,
    pub sequence: Counter,
    pub status: WatchStatus,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepositoryChangeReason {
    #[serde(rename = "changed")]
    Changed,
    #[serde(rename = "overflow")]
    Overflow,
    #[serde(rename = "unavailable")]
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryChangedEvent {
    pub scope: HistoryScope,
    pub subscription_id: Id,
    pub sequence: Counter,
    pub reason: RepositoryChangeReason,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenHistoryRequest {
    pub scope: HistoryScope,
    pub filter: HistoryFilter,
    #[serde(deserialize_with = "required_nullable")]
    pub page_size: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoadHistoryPageRequest {
    pub context: HistoryContext,
    pub session_id: Id,
    pub cursor: Id,
    #[serde(deserialize_with = "required_nullable")]
    pub page_size: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchHistoryRequest {
    pub context: HistoryContext,
    pub session_id: Id,
    pub query: String,
    #[serde(deserialize_with = "required_nullable")]
    pub cursor: Option<Id>,
    #[serde(deserialize_with = "required_nullable")]
    pub page_size: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetCommitDetailsRequest {
    pub context: HistoryContext,
    pub commit_handle: Id,
    pub base: DiffBase,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetCommitDiffRequest {
    pub context: HistoryContext,
    pub file_handle: Id,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetRepositoryViewRequest {
    pub scope: HistoryScope,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveRepositoryViewRequest {
    pub scope: HistoryScope,
    pub expected_revision: Counter,
    pub view: RepositoryView,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WatchRepositoryRequest {
    pub scope: HistoryScope,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnwatchRepositoryRequest {
    pub subscription_id: Id,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, tag = "kind")]
pub enum DiffBase {
    Root,
    Parent { index: u32 },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffLayout {
    #[serde(rename = "inline")]
    Inline,
    #[serde(rename = "sideBySide")]
    SideBySide,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, tag = "kind")]
pub enum DiffContent {
    Text {
        original: String,
        modified: String,
        #[serde(deserialize_with = "required_nullable")]
        language: Option<String>,
    },
    Unavailable {
        reason: DiffUnavailableReason,
    },
}
fn required_nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(d)
}
pub const HISTORY_PAGE_DEFAULT: usize = 200;
pub const HISTORY_PAGE_MAX: usize = 500;
pub const HISTORY_SEARCH_PAGE_MAX: usize = 100;
pub const HISTORY_QUERY_BYTES: usize = 512;
pub const HISTORY_SESSIONS_PER_WINDOW: usize = 8;
pub const HISTORY_COMMITS_PER_SESSION: usize = 100000;
pub const HISTORY_FILES_PER_SESSION: usize = 20000;
pub const HISTORY_CURSORS_PER_SESSION: usize = 256;
pub const HISTORY_SESSION_TTL_SECONDS: usize = 900;
pub const HISTORY_DIFF_BLOB_BYTES: usize = 1048576;
pub const HISTORY_DIFF_TOTAL_BYTES: usize = 2097152;
pub const HISTORY_RESPONSE_BYTES: usize = 8388608;
pub const HISTORY_REFS_MAX: usize = 20000;
pub const HISTORY_DETAIL_FILES_MAX: usize = 10000;
pub const HISTORY_MESSAGE_BYTES: usize = 65536;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    fn roundtrip<T: serde::de::DeserializeOwned + Serialize>(value: &Value) {
        let parsed: T = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *value);
    }
    fn request<T: serde::de::DeserializeOwned + Serialize>(value: &Value) {
        roundtrip::<T>(value);
        let object = value.as_object().unwrap();
        for key in object.keys() {
            let mut missing = object.clone();
            missing.remove(key);
            assert!(
                serde_json::from_value::<T>(Value::Object(missing)).is_err(),
                "required {key}"
            );
        }
        for key in ["path", "offset", "argv", "revision"] {
            let mut extra = object.clone();
            extra.insert(key.into(), json!("forbidden"));
            assert!(serde_json::from_value::<T>(Value::Object(extra)).is_err());
        }
    }
    #[test]
    fn additive_wire_and_strict_requests() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../fixtures/contracts/history-wire.json"
        ))
        .unwrap();
        for sample in fixture["commands"].as_array().unwrap() {
            macro_rules! check {
                ($request:ty, $result:ty) => {{
                    request::<$request>(&sample["request"]);
                    roundtrip::<ApiResult<$result>>(&sample["result"]);
                }};
            }
            match sample["command"].as_str().unwrap() {
                "open_history" => check!(OpenHistoryRequest, HistorySession),
                "load_history_page" => check!(LoadHistoryPageRequest, HistoryPage),
                "search_history" => check!(SearchHistoryRequest, HistorySearchPage),
                "get_commit_details" => check!(GetCommitDetailsRequest, CommitDetails),
                "get_commit_diff" => check!(GetCommitDiffRequest, CommitDiff),
                "get_repository_view" => check!(GetRepositoryViewRequest, RepositoryViewState),
                "save_repository_view" => check!(SaveRepositoryViewRequest, RepositoryViewState),
                "watch_repository" => check!(WatchRepositoryRequest, RepositoryWatch),
                "unwatch_repository" => check!(UnwatchRepositoryRequest, Option<()>),
                command => panic!("unexpected {command}"),
            }
        }
        roundtrip::<RepositoryChangedEvent>(&fixture["event"]);
        for v in fixture["diffVariants"].as_array().unwrap() {
            roundtrip::<CommitDiff>(v);
        }
        for v in fixture["baseVariants"].as_array().unwrap() {
            roundtrip::<DiffBase>(v);
        }
        for v in fixture["fileKinds"].as_array().unwrap() {
            roundtrip::<CommitFileKind>(v);
        }
        for v in fixture["watchVariants"].as_array().unwrap() {
            roundtrip::<RepositoryWatch>(v);
        }
        for v in fixture["changeReasons"].as_array().unwrap() {
            roundtrip::<RepositoryChangeReason>(v);
        }
        for v in fixture["boundaries"].as_array().unwrap() {
            roundtrip::<ParentBoundary>(v);
        }
        assert!(serde_json::from_value::<HistoryContext>(
            json!({"scope":{"repoId":"r","generation":"1"},"snapshotId":"s"})
        )
        .is_err());
        assert!(serde_json::from_value::<DiffBase>(json!({"kind":"parent","index":-1})).is_err());
        assert!(
            serde_json::from_value::<RepositoryView>(json!({"cursor":"runtime-token"})).is_err()
        );
    }
}
