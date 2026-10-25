import {expect,it} from 'vitest';
import fixture from '../../fixtures/contracts/remote-actions-wire.json';
import type {RemoteActionCatalog,RemoteActionOperationResult,ResolvedRemoteTarget,RemoteActionCommandMap} from './remote_actions';
import type {ActionIntent} from './daily';
it('keeps remote advertisements separate from local tracking and runtime handles out of durable evidence',()=>{
 const catalog=fixture.catalog as RemoteActionCatalog,result=fixture.result as RemoteActionOperationResult,target=fixture.resolved as ResolvedRemoteTarget,intent=fixture.intent as ActionIntent;
 expect(intent.kind).toBe('readRemoteRefs');expect(catalog.remotes[0].endpoints.map(e=>e.role)).toEqual(['fetch','push']);expect(result.operation.actionKind).toBe('readRemoteRefs');expect(result.evidence?.noGitWrites).toBe(true);expect(result.operation.outcome?.repositoryChanged).toBe(false);
 expect(result.snapshot?.refs.map(r=>r.name)).toEqual(['refs/heads/main','refs/tags/v1']);expect(target.handle).toHaveLength(64);expect(target.expectedOid).toBe(result.snapshot?.refs[0].oid);
 const requests:{[K in keyof RemoteActionCommandMap]:RemoteActionCommandMap[K]['request']}={get_remote_actions:fixture.catalogRequest,get_remote_action_outcome:{operationId:result.operation.id},resolve_remote_target:{scope:catalog.scope,snapshotId:result.snapshot!.snapshotId,source:{kind:'existing',entryId:'entry-0'}}};
 expect(JSON.stringify(requests)).not.toMatch(/"(?:url|oid|path|argv|refspec|environment)":/);expect(JSON.stringify(result.evidence)).not.toMatch(/snapshotId|entryId|handle|https?:/);expect((fixture.unavailable as RemoteActionOperationResult).snapshot).toBeNull();
});
