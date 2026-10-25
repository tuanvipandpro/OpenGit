# IPC foundation contract 0.1.0 — frozen v1

Target 0.2.0 adds the scoped history/view/watch DTOs in [history contract](history.md) without changing foundation contractVersion `1` or the twelve commands below. S1 types/fixtures do not imply those new handlers are registered yet.

OG-010-01, 2026-10-24; phạm vi R1/R6, F2–F4. Owner 02 hiện thực cùng schema trong Rust/TypeScript; thay đổi cần orchestrator điều phối. JSON UTF-8, camelCase, enum phân biệt hoa thường. Mọi field bên dưới bắt buộc; `null` biểu thị không có. Không nhận unknown fields trong request. Tauri gọi command bằng `{ request: <args> }`, kể cả `{ request: {} }`; trả `ApiResult<T>`. Không có shell/fs/argv API.

```ts
type Id = string; // opaque, nonempty, backend-issued; không suy ra path từ ID
type Counter = string; // decimal không dấu, so sánh số nguyên; không ép JS number
type Timestamp = string; // UTC RFC3339
type ApiResult<T> = { ok: true; value: T } | { ok: false; error: AppError };
type ErrorCode = 'InvalidRequest' | 'InvalidHandle' | 'NotFound' | 'GitUnavailable'
  | 'UnsupportedGit' | 'NotRepository' | 'RepositoryUnavailable' | 'UnsafeRepository'
  | 'DestinationExists' | 'DestinationBusy' | 'UnsafePath' | 'InvalidSource'
  | 'AuthenticationRequired' | 'AuthenticationFailed' | 'PermissionDenied'
  | 'Network' | 'Certificate' | 'SshHostKey' | 'SshAuthentication'
  | 'Timeout' | 'OutputLimit' | 'MalformedOutput' | 'ExternalChange'
  | 'DiskFull' | 'Io' | 'Database' | 'Interrupted' | 'Internal';
type Phase = 'validation' | 'discovery' | 'inspection' | 'queue' | 'authentication'
  | 'transfer' | 'checkout' | 'verification' | 'publish' | 'registration' | 'recovery';
type AppError = { code: ErrorCode; message: string; phase: Phase;
  retry: 'none' | 'retry' | 'chooseAnotherDestination' | 'checkCredentials'
    | 'checkNetwork' | 'checkTrust' | 'reopenRepository'; };
type ToolInfo = { available: boolean; version: string | null };
type Environment = { contractVersion: '1'; appVersion: string; platform: 'windows';
  git: ToolInfo; credentialManager: ToolInfo; openSsh: ToolInfo;
  webView2: ToolInfo; identity: IdentityPresence };
type IdentityPresence = { nameConfigured: boolean; emailConfigured: boolean };
type Selection = { repoId: Id; worktreeId: Id | null };
type WorktreeSummary = { id: Id; rootDisplay: string; isMain: boolean };
type RepositorySummary = { id: Id; name: string; commonGitDirDisplay: string;
  isBare: boolean; availability: 'available' | 'missing' | 'unavailable';
  worktrees: WorktreeSummary[]; lastOpenedAt: Timestamp };
type Workspace = { id: Id; revision: Counter; repositories: RepositorySummary[];
  activeSelection: Selection | null };
type Head = { kind: 'branch' | 'detached' | 'unborn'; branch: string | null;
  oid: string | null }; // OID opaque, không cố định SHA-1/40 ký tự
type GitOperation = 'merge' | 'rebase' | 'cherryPick' | 'revert' | 'bisect' | 'sequencer';
type PathStatus = '.' | 'M' | 'T' | 'A' | 'D' | 'R' | 'C' | 'U' | '?' | '!';
type StatusEntry = { id: Id; kind: 'ordinary' | 'rename' | 'unmerged' | 'untracked' | 'ignored';
  pathDisplay: string; previousPathDisplay: string | null; indexStatus: PathStatus;
  worktreeStatus: PathStatus; submodule: boolean };
type RemoteSummary = { name: string; fetchUrlDisplay: string | null;
  pushUrlDisplay: string | null };
type RepositorySnapshot = { repoId: Id; worktreeId: Id | null; generation: Counter;
  observedAt: Timestamp; isBare: boolean; head: Head;
  status: StatusEntry[]; isDirty: boolean | null; isShallow: boolean;
  remotes: RemoteSummary[]; identity: IdentityPresence; gitOperations: GitOperation[] };
type CloneSource = { kind: 'remote'; url: string } | { kind: 'local'; sourceHandle: Id };
type LocalCloneSource = { sourceHandle: Id; display: string };
type CloneDestination = { destinationHandle: Id; parentDisplay: string };
type OperationState = 'Queued' | 'Running' | 'WaitingForUser' | 'Cancelling'
  | 'Succeeded' | 'Failed' | 'Cancelled' | 'Interrupted';
type AuthPrompt = { promptHandle: Id; kind: 'username' | 'password' | 'passphrase' | 'confirmation';
  message: string; expiresAt: Timestamp }; // message theo template, không raw Git prompt
type Progress = { completed: number; total: number | null; unit: 'objects' | 'bytes' | 'steps' };
type OperationSnapshot = { id: Id; sequence: Counter; kind: 'clone'; state: OperationState;
  phase: Phase; repoId: Id | null; worktreeId: Id | null; sourceDisplay: string;
  destinationDisplay: string; progress: Progress | null; canCancel: boolean;
  authPrompt: AuthPrompt | null; error: AppError | null;
  createdAt: Timestamp; updatedAt: Timestamp };
type OperationsSnapshot = { streamId: Id; sequence: Counter; operations: OperationSnapshot[] };
type OperationEvent = { streamId: Id; sequence: Counter; operation: OperationSnapshot };
```

| Command | Args (`request`) | `value` khi thành công |
| --- | --- | --- |
| `get_environment` | `{}` | `Environment` |
| `get_workspace` | `{}` | `Workspace` |
| `choose_repository` | `{}` | `Workspace \| null` (OS dialog cancel = null; chọn thành công đăng ký + chọn) |
| `inspect_repository` | `{repoId: Id, worktreeId: Id \| null, generation: Counter}` | `RepositorySnapshot` |
| `select_repository` | `{repoId: Id, worktreeId: Id \| null}` | `Workspace` |
| `forget_repository` | `{repoId: Id}` | `Workspace` (chỉ registry/selection, không Git/filesystem) |
| `choose_clone_source` | `{}` | `LocalCloneSource \| null` (OS local folder chooser) |
| `choose_clone_destination` | `{}` | `CloneDestination \| null` (OS parent-folder chooser) |
| `start_clone` | `{source: CloneSource, destinationHandle: Id, directoryName: string}` | `OperationSnapshot` (đã journal/reserve trước trả Queued) |
| `get_operations` | `{}` | `OperationsSnapshot` (full snapshot, không delta) |
| `cancel_operation` | `{operationId: Id}` | `OperationSnapshot` (idempotent, terminal giữ nguyên) |
| `respond_auth_prompt` | `{operationId: Id, promptHandle: Id, value: string}` | `null` |

`worktreeId: null` chỉ hợp lệ cho bare repo; repo thường phải dùng worktree thuộc repo đó. Bare snapshot: `status=[]`, `isDirty=null`, `gitOperations=[]`; HEAD vẫn đọc metadata. Branch có tên/oid; unborn có tên, oid null; detached branch null. Worktree IDs tồn tại bền vững, status-entry IDs chỉ có hiệu lực trong snapshot và không là API mutation. Repository summary không phải cấp quyền dùng display path; backend giữ canonical/raw identity (common Git dir + worktree). Recent list là repositories sắp `lastOpenedAt` giảm dần; forget active repo đặt selection null. `revision` tăng trong cùng transaction khi workspace đổi; frontend bỏ response revision cũ.

Frontend tăng `generation` cho mỗi inspection/switch/refresh, chỉ áp dụng snapshot khớp selection và generation mới nhất. Backend echo token, xác thực scope lại; external-change refresh ít nhất khi focus và polling lúc repo đang chọn. Không có graph, ref mutation, checkout hay recovery action ở v1.

Event `opengit://operation` mang `OperationEvent`; counter toàn stream tăng đúng một cho mỗi transition/progress update, `operation.sequence = event.sequence`. Subscribe trước `get_operations`; buffer event rồi chỉ áp dụng sequence lớn hơn snapshot fence. Gap, reorder hoặc `streamId` đổi ⇒ lấy full snapshot; event cũ không ghi đè. Poll full snapshot mỗi 2 giây khi task active, khi focus và khi event subscription lỗi. Stream ID mới sau restart, không so counter xuyên stream. Snapshot và fence lấy atomically. Không prune history trong phiên 0.1.0 để không cần remove event.

Backend validate size/type/scope trước mọi spawn: request ≤16 KiB, URL ≤4096 UTF-8 bytes, auth value ≤8192 bytes và không NUL/newline. `confirmation` chỉ `yes`/`no`; secret field không echo. Handles ngẫu nhiên ≥128 bit, gắn cửa sổ/phiên/backend; clone handles hết hạn sau 15 phút, không persist, revalidate OS identity khi dùng. Auth prompt chỉ một lần, gắn operation/process/nonce, tối đa 120 giây; cancel/terminal/restart vô hiệu ngay. `InvalidHandle` khi stale/replay/sai scope.

`directoryName` là một Windows component 1–200 UTF-16 code units: cấm `.`, `..`, separators, colon, controls, NUL, trailing dot/space, reserved device names và absolute paths. Chỉ backend ghép với granted parent. Remote chỉ HTTPS, SSH URL hoặc SCP (`user@host:path`); cấm URL password/token/userinfo HTTPS, query/fragment, custom helper/`ext::`, leading option, CR/LF/NUL. SSH username được phép; drive/UNC local dùng chooser handle để không nhầm SCP. Backend không persist input URL; display bỏ userinfo/query/fragment và chỉ lưu mẫu an toàn. Remotes từ Git được sanitize tương tự; không parse được trả null.

Error message chỉ từ template/code an toàn, không chứa raw stderr, stdout, URL, prompt hoặc secret; lỗi không phân loại chắc ⇒ `Internal` với thông báo chung và retry none. Git output không tin cậy phải bound/redact trước mọi sink. Tauri reject/transport failure cũng được frontend chuyển thành template `Internal`; không render raw exception. Phân loại cert/host key không dẫn tới bỏ verification.

Owner 02 dùng serde camelCase + tagged unions tương ứng, TS types thủ công; fixture JSON round-trip cả hai chiều cho success/error, nullable/bare/unborn và mọi enum. Request unknown/missing fields bị reject; không dùng generator mới. Fixture không chứa credential thật. Contract compile/round-trip chưa chạy ở bước tài liệu này.
