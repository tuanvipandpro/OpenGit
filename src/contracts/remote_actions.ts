import type { ActionScope, FetchStepPhase } from './daily';
import type { Id, Timestamp, Counter, OperationSnapshot } from './models';
export type RemoteEndpointRole = 'fetch' | 'push';
export type RemoteEndpointCapability = 'eligible' | 'unsupportedTransport' | 'multipleUrls' | 'mirror' | 'unsupportedRefspec';
export interface RemoteEndpoint { role: RemoteEndpointRole; handle: Id | null; capability: RemoteEndpointCapability }
export interface RemoteActionCatalog { scope: ActionScope; observedAt: Timestamp; remotes: { name: string; endpoints: RemoteEndpoint[] }[] }
export interface RemoteRefEntry { entryId: Id; name: string; oid: string }
export interface RemoteRefSnapshot { capturedScope: ActionScope; snapshotId: Id; remoteName: string; role: RemoteEndpointRole; observedAt: Timestamp; expiresAt: Timestamp; objectFormat: 'sha1' | 'sha256' | null; refs: RemoteRefEntry[] }
export interface RemoteReadEvidence { remoteName: string; role: RemoteEndpointRole; phase: FetchStepPhase; refCount: Counter | null; observedAt: Timestamp | null; noGitWrites: boolean }
export interface RemoteActionOperationResult { operation: OperationSnapshot; availability: 'pending' | 'available' | 'expired' | 'unavailable'; evidence: RemoteReadEvidence | null; snapshot: RemoteRefSnapshot | null }
export type RemoteRefNamespace = 'branch' | 'tag';
export type RemoteTargetSource = { kind: 'existing'; entryId: Id } | { kind: 'new'; namespace: RemoteRefNamespace; name: string };
export interface ResolvedRemoteTarget { scope: ActionScope; snapshotId: Id; handle: Id; name: string; expectedOid: string | null; namespace: RemoteRefNamespace }
export interface RemoteActionCommandMap {
 get_remote_actions: { request: { scope: ActionScope }; value: RemoteActionCatalog };
 get_remote_action_outcome: { request: { operationId: Id }; value: RemoteActionOperationResult };
 resolve_remote_target: { request: { scope: ActionScope; snapshotId: Id; source: RemoteTargetSource }; value: ResolvedRemoteTarget };
}
