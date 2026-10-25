import { expect, it } from 'vitest';
import fixture from '../../fixtures/contracts/daily-wire.json';
import { DAILY_LIMITS } from './daily';
import type { OperationSnapshot } from './models';
import type { ActionIntent, ActionOutcome, PreparedAction } from './daily';
import type { ActionOperationResult, RepositoryActionCatalog, CommitContainment, DailyCommandMap } from './daily';
it('freezes daily intent/outcome wire variants without renderer paths/revisions/argv', () => {
 const intents = fixture.intents as ActionIntent[]; const prepared = fixture.prepared as PreparedAction; const outcomes = fixture.outcomes as ActionOutcome[];
 expect(new Set(intents.map(v => v.kind)).size).toBe(21); expect(prepared.scope.generation).toBe('9007199254740993');
 expect(outcomes.map(v => v.effects)).toEqual(['applied','notApplied','conflict','partial','unknown']);
 expect(JSON.stringify(intents)).not.toMatch(/"(?:path|oid|argv|refspec|environment|patch)":/);
 expect((fixture.operation as OperationSnapshot).kind).toBe('action');
 expect(DAILY_LIMITS.requestBytes).toBe(16384); expect(DAILY_LIMITS.planTtlSeconds).toBeLessThan(DAILY_LIMITS.consumedTtlSeconds);
});
it('keeps catalog scope, history context, unknown membership and actual per-ref fetch evidence separate', () => {
 const catalog=fixture.catalog as RepositoryActionCatalog;
 const membership=fixture.shallowContainment as CommitContainment;
 const result=fixture.actionOperationResult as ActionOperationResult;
 const missing=fixture.unknownWithoutLedger as ActionOperationResult;
 expect(catalog.scope.generation).toBe('9007199254740993');
 expect(catalog.remotes[0].fetch).toBe('eligibleProbeRequired');
 expect(catalog.symbolicBranches[0].name).toBe('refs/remotes/origin/HEAD');
 expect(catalog.branches.some(b=>b.name==='refs/remotes/origin/HEAD')).toBe(false);
 expect(membership.context.scope).toEqual(catalog.scope);
 expect(membership.branches.some(b=>b.membership==='unknown')).toBe(true);
 expect(result.fetch?.remotes[0].reportedRefs.map(r=>r.status)).toEqual(['created','updated','forced','deleted','upToDate']);
 expect(missing.fetch).toBeNull(); expect(missing.operation.outcome?.effects).toBe('unknown');
 expect(DAILY_LIMITS.fetchLedgerBytes).toBe(64*1024*1024);
 expect(DAILY_LIMITS.catalogRefs).toBeLessThan(DAILY_LIMITS.observedRefRows);
 const requests:{[K in keyof DailyCommandMap]:DailyCommandMap[K]['request']}={
  get_repository_actions:{scope:catalog.scope},
  get_commit_containment:{context:membership.context,commitHandle:membership.commitHandle},
  prepare_repository_action:{scope:catalog.scope,intent:{kind:'fetch',remoteHandles:[catalog.remotes[0].handle!],prune:false}},
  execute_repository_action:{planId:fixture.prepared.planId},
  get_action_outcome:{operationId:result.operation.id},
 };
 expect(JSON.stringify(requests)).not.toMatch(/"(?:path|oid|url|argv|refspec|environment|patch)":/);
 expect(JSON.parse(JSON.stringify(fixture))).toEqual(fixture);
});
