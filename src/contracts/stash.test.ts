import { expect, it } from 'vitest';
import fixture from '../../fixtures/contracts/stash-wire.json';
import type { RepositoryStashes, ResolvedStashTarget, StashActionOperationResult, StashCommandMap } from './stash';

it('keeps stash counters exact, resolves authority separately and excludes runtime capabilities from durable outcomes', () => {
  const catalog: RepositoryStashes = fixture.catalog;
  const resolved: ResolvedStashTarget = fixture.resolved;
  const result = fixture.result as StashActionOperationResult;
  const requests: { [K in keyof StashCommandMap]: StashCommandMap[K]['request'] } = {
    get_repository_stashes: fixture.catalogRequest,
    resolve_stash_target: fixture.resolveRequest,
    get_stash_action_outcome: fixture.outcomeRequest,
    reconcile_stash_action: fixture.reconcileRequest,
  };
  expect(catalog.scope.generation).toBe('9007199254740993');
  expect(catalog.entries[0].ordinal).toBe('9007199254740993');
  expect(catalog.entries[0]).not.toHaveProperty('stashHandle');
  expect(resolved.stashHandle).toHaveLength(64);
  expect(result.result?.recoveryRefRetention).toBe('unknown');
  expect(JSON.stringify(result)).not.toContain(resolved.stashHandle);
  expect(JSON.stringify(requests)).not.toMatch(/"(?:path|patch|oid|ordinal|message|argv|environment)":/);
});
