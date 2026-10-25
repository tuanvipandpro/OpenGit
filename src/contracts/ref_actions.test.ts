import { expect, it } from 'vitest';
import fixture from '../../fixtures/contracts/ref-actions-wire.json';
import type { RefActionCatalog, RefActionCommandMap, RefActionOperationResult, ResolvedRefActionTarget } from './ref_actions';
it('keeps ref authority separate from names/OIDs and frozen fetch contracts', () => {
 const catalog=fixture.catalog as RefActionCatalog; const target=fixture.resolved as ResolvedRefActionTarget; const result=fixture.result as RefActionOperationResult;
 expect(catalog.scope.generation).toBe('9007199254740993'); expect(catalog.catalogId).toHaveLength(64); expect(target.handle).toHaveLength(64);
 expect(catalog.refs[1].symbolicTarget).toBe('refs/remotes/origin/main'); expect(catalog.refs[1].canStart).toBe(false);
 expect(result.result?.steps.map(s=>s.effects)).toEqual(['applied','notApplied']);expect(result.operation.state).toBe('Failed');expect(result.operation.outcome?.effects).toBe('partial');
 const unknown=fixture.unknown as RefActionOperationResult;expect(unknown.result).toBeNull();expect(unknown.operation.state).toBe('Interrupted');expect(unknown.operation.outcome?.effects).toBe('unknown');
 const requests:{[K in keyof RefActionCommandMap]:RefActionCommandMap[K]['request']}={get_ref_actions:fixture.catalogRequest,resolve_ref_action_target:{scope:catalog.scope,catalogId:catalog.catalogId,source:{kind:'ref',entryId:catalog.refs[0].entryId}},get_ref_action_outcome:{operationId:result.operation.id}};
 expect(JSON.stringify(requests)).not.toMatch(/"(?:path|oid|url|ref|argv|refspec|environment|patch)":/);expect(target.scope).toEqual(catalog.scope);
});
