
export type Id = string; // opaque, nonempty, backend-issued; không suy ra path từ ID
export type Counter = string; // decimal không dấu, so sánh số nguyên; không ép JS number
export type Timestamp = string; // UTC RFC3339
export type ApiResult<T> = { ok: true; value: T } | { ok: false; error: AppError };
export type ErrorCode = 'UnsupportedAction' | 'InvalidRequest' | 'InvalidHandle' | 'NotFound' | 'GitUnavailable'
  | 'UnsupportedGit' | 'NotRepository' | 'RepositoryUnavailable' | 'UnsafeRepository'
  | 'DestinationExists' | 'DestinationBusy' | 'UnsafePath' | 'InvalidSource'
  | 'AuthenticationRequired' | 'AuthenticationFailed' | 'PermissionDenied'
  | 'Network' | 'Certificate' | 'SshHostKey' | 'SshAuthentication'
  | 'Timeout' | 'OutputLimit' | 'MalformedOutput' | 'ExternalChange'
  | 'DiskFull' | 'Io' | 'Database' | 'Interrupted' | 'Internal';
export type Phase = 'validation' | 'discovery' | 'inspection' | 'queue' | 'authentication'
  | 'transfer' | 'checkout' | 'verification' | 'publish' | 'registration' | 'recovery';
export type AppError = { code: ErrorCode; message: string; phase: Phase;
  retry: 'none' | 'retry' | 'chooseAnotherDestination' | 'checkCredentials'
    | 'checkNetwork' | 'checkTrust' | 'reopenRepository'; };
export type ToolInfo = { available: boolean; version: string | null };
export type Environment = { contractVersion: '1'; appVersion: string; platform: 'windows';
  git: ToolInfo; credentialManager: ToolInfo; openSsh: ToolInfo;
  webView2: ToolInfo; identity: IdentityPresence };
export type IdentityPresence = { nameConfigured: boolean; emailConfigured: boolean };
export type Selection = { repoId: Id; worktreeId: Id | null };
export type WorktreeSummary = { id: Id; rootDisplay: string; isMain: boolean };
export type RepositorySummary = { id: Id; name: string; commonGitDirDisplay: string;
  isBare: boolean; availability: 'available' | 'missing' | 'unavailable';
  worktrees: WorktreeSummary[]; lastOpenedAt: Timestamp };
export type Workspace = { id: Id; revision: Counter; repositories: RepositorySummary[];
  activeSelection: Selection | null };
export type Head = { kind: 'branch' | 'detached' | 'unborn'; branch: string | null;
  oid: string | null }; // OID opaque, không cố định SHA-1/40 ký tự
export type GitOperation = 'merge' | 'rebase' | 'cherryPick' | 'revert' | 'bisect' | 'sequencer';
export type PathStatus = '.' | 'M' | 'T' | 'A' | 'D' | 'R' | 'C' | 'U' | '?' | '!';
export type StatusEntry = { id: Id; kind: 'ordinary' | 'rename' | 'unmerged' | 'untracked' | 'ignored';
  pathDisplay: string; previousPathDisplay: string | null; indexStatus: PathStatus;
  worktreeStatus: PathStatus; submodule: boolean };
export type RemoteSummary = { name: string; fetchUrlDisplay: string | null;
  pushUrlDisplay: string | null };
export type RepositorySnapshot = { repoId: Id; worktreeId: Id | null; generation: Counter;
  observedAt: Timestamp; isBare: boolean; head: Head;
  status: StatusEntry[]; isDirty: boolean | null; isShallow: boolean;
  remotes: RemoteSummary[]; identity: IdentityPresence; gitOperations: GitOperation[] };
export type CloneSource = { kind: 'remote'; url: string } | { kind: 'local'; sourceHandle: Id };
export type LocalCloneSource = { sourceHandle: Id; display: string };
export type CloneDestination = { destinationHandle: Id; parentDisplay: string };
export type OperationState = 'Queued' | 'Running' | 'WaitingForUser' | 'Cancelling'
  | 'Succeeded' | 'Failed' | 'Cancelled' | 'Interrupted';
export type AuthPrompt = { promptHandle: Id; kind: 'username' | 'password' | 'passphrase' | 'confirmation';
  message: string; expiresAt: Timestamp }; // message theo template, không raw Git prompt
export type Progress = { completed: number; total: number | null; unit: 'objects' | 'bytes' | 'steps' };
export type OperationSnapshot = { id: Id; sequence: Counter; kind: 'clone' | 'action'; actionKind?: import('./daily').ActionKind; outcome?: import('./daily').ActionOutcome; state: OperationState;
  phase: Phase; repoId: Id | null; worktreeId: Id | null; sourceDisplay: string;
  destinationDisplay: string; progress: Progress | null; canCancel: boolean;
  authPrompt: AuthPrompt | null; error: AppError | null;
  createdAt: Timestamp; updatedAt: Timestamp };
export type OperationsSnapshot = { streamId: Id; sequence: Counter; operations: OperationSnapshot[] };
export type OperationEvent = { streamId: Id; sequence: Counter; operation: OperationSnapshot };
