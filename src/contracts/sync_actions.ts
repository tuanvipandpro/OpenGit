import type { ActionEffects, ActionScope, FetchStepPhase } from './daily';
import type { OperationSnapshot } from './models';
export type RemoteMutationKind = 'push' | 'pull' | 'remoteTagPush' | 'remoteTagDelete';
export type SyncStepKind = 'push' | 'tracking' | 'upstream' | 'fetch' | 'fastForward' | 'tagPush' | 'tagDelete';
export type SyncFailureReason = 'rejected' | 'diverged' | 'stale' | 'unsupported' | 'authentication' | 'network' | 'native' | 'recovery';
export interface PublicationEvidence { remoteName: string; role: 'fetch' | 'push'; destination: string; sourceRef: string | null; sourceOid: string | null; acknowledgedOid: string | null; acknowledgedAt: string; configDigest: string; deleted: boolean }
export interface SyncActionResult { steps: { step: SyncStepKind; phase: FetchStepPhase; effects: ActionEffects; failureReason: SyncFailureReason | null; observedAt: string | null }[]; publication: PublicationEvidence | null; trackingUpdated: boolean; upstreamUpdated: boolean; trackingRefreshRequired: boolean; fetchedOid: string | null; integratedOid: string | null }
export interface SyncActionOperationResult { operation: OperationSnapshot; result: SyncActionResult | null }
export interface SyncActionCommandMap {
  get_sync_action_outcome: { request: { operationId: string }; value: SyncActionOperationResult };
  get_publication_evidence: { request: { scope: ActionScope; sourceHandle: string }; value: { scope: ActionScope; observations: PublicationEvidence[] } };
}
