import type { ActionScope } from './daily';
import type { Head, Id, OperationSnapshot, PathStatus, Timestamp } from './models';
export type ChangeSide = 'staged' | 'unstaged';
export type ChangeCapability = 'eligible' | 'unsupported' | 'unavailable';
export interface ChangeEntry { entryId: Id; pathDisplay: string; previousPathDisplay: string | null; indexStatus: PathStatus; worktreeStatus: PathStatus; stageCapability: ChangeCapability; unstageCapability: ChangeCapability }
export interface RepositoryChanges { scope: ActionScope; catalogId: Id; observedAt: Timestamp; head: Head; entries: ChangeEntry[]; unsupported: boolean }
export interface ChangeTarget { entryId: Id; side: ChangeSide; changeHandle: Id }
export interface ResolvedChangeTargets { scope: ActionScope; catalogId: Id; targets: ChangeTarget[] }
export type PartialEligibility = 'eligible' | 'binary' | 'encoding' | 'lineEndings' | 'attributes' | 'rename' | 'mode' | 'tooLarge' | 'complexityLimit' | 'filePresenceOnly' | 'unsupported';
export interface ChangeDiffRow { lineId: Id | null; kind: 'context' | 'add' | 'delete'; oldLine: number | null; newLine: number | null; text: string; hasLf: boolean }
export interface ChangeHunk { hunkId: Id; rows: ChangeDiffRow[] }
export interface ChangeDiff { scope: ActionScope; changeHandle: Id; diffId: Id; side: ChangeSide; content: string; partialEligibility: PartialEligibility; hunks: ChangeHunk[] }
export type ChangeSelectionInput = { kind: 'hunks'; hunkIds: Id[] } | { kind: 'lines'; lineIds: Id[] };
export interface ResolvedChangeSelection { scope: ActionScope; changeHandle: Id; diffId: Id; selectionHandle: Id }
export type ChangeFailureReason = 'identity' | 'hookOrNative' | 'signing' | 'empty' | 'stale' | 'locked' | 'unsupported' | 'recovery';
export interface ChangeActionResult { commitOid: string | null; treeOid: string | null; failureReason: ChangeFailureReason | null; artifactsRetained: boolean }
export interface ChangeActionOperationResult { operation: OperationSnapshot; result: ChangeActionResult | null }
export interface ChangeCommandMap {
  get_repository_changes: { request: { scope: ActionScope }; value: RepositoryChanges };
  resolve_change_targets: { request: { scope: ActionScope; catalogId: Id; targets: { entryId: Id; side: ChangeSide }[] }; value: ResolvedChangeTargets };
  get_change_diff: { request: { scope: ActionScope; changeHandle: Id }; value: ChangeDiff };
  resolve_change_selection: { request: { scope: ActionScope; changeHandle: Id; diffId: Id; selection: ChangeSelectionInput }; value: ResolvedChangeSelection };
  get_change_action_outcome: { request: { operationId: Id }; value: ChangeActionOperationResult };
}
