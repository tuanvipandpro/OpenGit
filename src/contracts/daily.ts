import type { HistoryContext } from './history';
import type { Counter, Id, Timestamp, Selection, OperationSnapshot } from './models';
export const DAILY_LIMITS = { windows: 64, grantsGlobal: 4096, grantsPerWindow: 4096, grantBytes: 4096, grantTtlSeconds: 300,
  plansGlobal: 128, plansPerWindow: 32, planBytes: 65536, planTtlSeconds: 60, consumedPerWindow: 256, consumedTtlSeconds: 900,
  authorityBytes: 33554432, journalRecords: 4096, artifactProofs: 32, recoveryLocatorBytes: 24576, contexts: 64, preflightReads: 4, queueSeconds: 30, localSeconds: 120, networkSeconds: 300,
  catalogRemotes:32, catalogRefs:4096, configBytes:1048576, fetchCoverage:4096, fetchActionBytes:8388608, fetchLedgerBytes:67108864, observedRefRows:16384, observedRefBytes:16777216,
  namesBytes: 1024, messageBytes: 8192, selectionHandles: 256, requestBytes: 16384 } as const;
export type ActionScope = Selection & { generation: Counter };
export type TagAnnotation = { kind: 'lightweight' } | { kind: 'annotated'; message: string; sign: boolean };
export type ActionIntent =
  | { kind: 'readRemoteRefs'; remoteHandle: Id }
  | { kind: 'fetch'; remoteHandles: Id[]; prune: boolean }
  | { kind: 'branchCreate'; name: string; startHandle: Id; checkout: boolean }
  | { kind: 'branchRename'; branchHandle: Id; newName: string }
  | { kind: 'branchSwitch'; branchHandle: Id }
  | { kind: 'branchDelete'; branchHandle: Id }
  | { kind: 'branchTracking'; branchHandle: Id; upstreamHandle: Id | null }
  | { kind: 'tagCreate'; name: string; targetHandle: Id; annotation: TagAnnotation }
  | { kind: 'tagDelete'; tagHandle: Id }
  | { kind: 'push'; remoteHandle: Id; sourceHandle: Id; destinationHandle: Id; setUpstream: boolean }
  | { kind: 'remoteTagPush'; remoteHandle: Id; tagHandle: Id; destinationHandle: Id }
  | { kind: 'remoteTagDelete'; remoteHandle: Id; tagHandle: Id }
  | { kind: 'pull'; upstreamHandle: Id }
  | { kind: 'stage'; changeHandles: Id[] }
  | { kind: 'unstage'; changeHandles: Id[] }
  | { kind: 'stageSelection'; selectionHandles: Id[] }
  | { kind: 'unstageSelection'; selectionHandles: Id[] }
  | { kind: 'commit'; message: string; amend: boolean }
  | { kind: 'stashPush'; message: string; includeUntracked: boolean }
  | { kind: 'stashApply'; stashHandle: Id }
  | { kind: 'stashPop'; stashHandle: Id }
  | { kind: 'stashDrop'; stashHandle: Id };
export type ActionKind = ActionIntent['kind'];
export type ActionEffects = 'applied' | 'notApplied' | 'conflict' | 'partial' | 'unknown';
export type ActionRecovery = 'none' | 'inspect' | 'resolveConflicts';
export type ActionOutcome = { effects: ActionEffects; recovery: ActionRecovery; cancellationRequested: boolean; repositoryChanged: boolean; observedAt: Timestamp };
export type ActionWarning = 'dirtyWorktree' | 'detachedHead' | 'unbornHead' | 'externalRace' | 'destructiveRemote' | 'incompleteHistory';
export type PreparedAction = { scope: ActionScope; planId: Id; kind: ActionKind; expiresAt: Timestamp; warnings: ActionWarning[] };
export type PrepareRepositoryActionRequest = { scope: ActionScope; intent: ActionIntent };
export type ExecuteRepositoryActionRequest = { planId: Id };
export type GetActionOutcomeRequest = { operationId: Id };
// Frozen for dependency-ready executors; S1 registers none of these commands.
export interface DailyCommandMap {
  get_repository_actions: { request: GetRepositoryActionsRequest; value: RepositoryActionCatalog };
  get_commit_containment: { request: GetCommitContainmentRequest; value: CommitContainment };
  prepare_repository_action: { request: PrepareRepositoryActionRequest; value: PreparedAction };
  execute_repository_action: { request: ExecuteRepositoryActionRequest; value: OperationSnapshot };
  get_action_outcome: { request: GetActionOutcomeRequest; value: ActionOperationResult };
}

export type GetRepositoryActionsRequest = { scope: ActionScope };
export type GetCommitContainmentRequest = { context: HistoryContext; commitHandle: Id };
export type ActionBranchKind = 'local' | 'remote';
export type Membership = 'contains' | 'doesNotContain' | 'unknown';
export type AncestryCompleteness = 'complete' | 'shallow' | 'unavailable';
export type FetchCapability = 'eligibleProbeRequired' | 'unsupportedTransport' | 'unsupportedRefspec' | 'mirror' | 'noRefspec';
export type RefObservation = { remoteName: string; observedOid: string | null; observedAt: Timestamp; matchesCurrent: boolean };
export type ActionBranch = { name: string; oid: string; kind: ActionBranchKind; observation: RefObservation | null };
export type SymbolicBranch = { name: string; target: string };
export type ActionRemote = { name: string; handle: Id | null; fetch: FetchCapability };
export type SyncStatus = { currentBranch: string | null; upstream: string | null; ahead: Counter | null; behind: Counter | null; completeness: AncestryCompleteness };
export type RepositoryActionCatalog = { scope: ActionScope; snapshotId: Id; observedAt: Timestamp; head: import('./models').Head; branches: ActionBranch[]; symbolicBranches: SymbolicBranch[]; remotes: ActionRemote[]; sync: SyncStatus };
export type ContainmentBranch = { branch: ActionBranch; membership: Membership };
export type CommitContainment = { context: HistoryContext; commitHandle: Id; oid: string; observedAt: Timestamp; branches: ContainmentBranch[]; currentBranch: string | null; upstream: string | null; completeness: AncestryCompleteness };

export type FetchStepPhase = 'pending' | 'started' | 'observed';
export type FetchRefStatus = 'created' | 'updated' | 'forced' | 'deleted' | 'upToDate';
export type FetchRefChange = { name: string; oldOid: string | null; newOid: string | null; status: FetchRefStatus };
export type FetchRemoteResult = { name: string; phase: FetchStepPhase; effects: ActionEffects; error: import('./models').AppError | null; reportedRefs: FetchRefChange[]; observedAt: Timestamp | null };
export type FetchResult = { prune: boolean; remotes: FetchRemoteResult[] };
export type ActionOperationResult = { operation: OperationSnapshot; fetch: FetchResult | null };
