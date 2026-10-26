import type { ActionScope } from './daily';
import type { Counter, Id, OperationSnapshot, Timestamp } from './models';

/** OIDs and ordinals are labels; only a current backend capability authorizes Apply. */
export interface StashEntry { entryId: Id; ordinal: Counter; oid: string; baseOid: string | null; includesUntracked: boolean; applyAvailable: boolean }
export interface RepositoryStashes { scope: ActionScope; catalogId: Id; observedAt: Timestamp; entries: StashEntry[]; saveAvailable: boolean }
export interface ResolvedStashTarget { scope: ActionScope; catalogId: Id; entryId: Id; stashHandle: Id }
export type StashFailureReason = 'stale' | 'dirty' | 'collision' | 'conflict' | 'locked' | 'native' | 'unsupported' | 'recovery';
export type StashResolution = 'notRequired' | 'unmerged' | 'resolvedExternally' | 'inspect';
export type StashRetention = 'present' | 'absent' | 'unknown';
export interface StashActionResult { stashOid: string | null; stashRetention: StashRetention; recoveryRefRetention: StashRetention; resolution: StashResolution; failureReason: StashFailureReason | null }
export interface StashActionOperationResult { operation: OperationSnapshot; result: StashActionResult | null }
export interface StashCommandMap {
  get_repository_stashes: { request: { scope: ActionScope }; value: RepositoryStashes };
  resolve_stash_target: { request: { scope: ActionScope; catalogId: Id; entryId: Id }; value: ResolvedStashTarget };
  get_stash_action_outcome: { request: { operationId: Id }; value: StashActionOperationResult };
  reconcile_stash_action: { request: { scope: ActionScope; operationId: Id }; value: StashActionOperationResult };
}
