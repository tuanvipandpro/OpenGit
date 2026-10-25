import type { ActionScope, ActionEffects } from './daily';
import type { Id, Timestamp, Head, OperationSnapshot } from './models';
import type { HistoryContext } from './history';
export type RefActionKind = 'local' | 'remote' | 'tag';
export interface RefActionEntry { entryId: Id; kind: RefActionKind; name: string; oid: string | null; symbolicTarget: string | null; checkedOutHere: boolean; checkedOutElsewhere: boolean; canStart: boolean }
export interface RefActionCatalog { scope: ActionScope; catalogId: Id; observedAt: Timestamp; head: Head; headEntryId: Id | null; refs: RefActionEntry[]; bare: boolean; dirty: boolean }
export type RefActionSource = { kind: 'ref'; entryId: Id } | { kind: 'head' } | { kind: 'historyCommit'; context: HistoryContext; commitHandle: Id };
export type RefTargetKind = 'commit' | 'branch' | 'remoteRef' | 'tag';
export interface ResolvedRefActionTarget { scope: ActionScope; catalogId: Id; handle: Id; kind: RefTargetKind; displayName: string; oid: string | null }
export interface RefActionResult { steps: { step: string; phase: 'pending' | 'started' | 'observed'; effects: ActionEffects; failureReason: 'identity' | 'signing' | 'native' | null }[] }
export interface RefActionOperationResult { operation: OperationSnapshot; result: RefActionResult | null }
export interface RefActionCommandMap {
 get_ref_actions: { request: { scope: ActionScope }; value: RefActionCatalog };
 resolve_ref_action_target: { request: { scope: ActionScope; catalogId: Id; source: RefActionSource }; value: ResolvedRefActionTarget };
 get_ref_action_outcome: { request: { operationId: Id }; value: RefActionOperationResult };
}
