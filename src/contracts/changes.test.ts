import { expect, it } from 'vitest';
import fixture from '../../fixtures/contracts/changes-wire.json';
import type { ChangeCommandMap, RepositoryChanges, ResolvedChangeTargets, ChangeDiff, ChangeActionOperationResult } from './changes';
it('keeps descriptive paths separate from closed change authority and whole-file fallback', () => {
  const catalog = fixture.catalog as RepositoryChanges, target = fixture.resolved as ResolvedChangeTargets, diff = fixture.diff as ChangeDiff, result = fixture.result as ChangeActionOperationResult;
  expect(catalog.scope.generation).toBe('9007199254740993'); expect(catalog.entries[0]).not.toHaveProperty('changeHandle'); expect(target.targets[0].changeHandle).toHaveLength(64); expect(diff.hunks).toEqual([]); expect(result.result?.artifactsRetained).toBe(false);
  const requests: { [K in keyof ChangeCommandMap]: ChangeCommandMap[K]['request'] } = { get_repository_changes: fixture.catalogRequest, resolve_change_targets: { ...fixture.resolveRequest, targets: [{ entryId: catalog.entries[0].entryId, side: 'unstaged' }] }, get_change_diff: fixture.diffRequest, get_change_action_outcome: { operationId: result.operation.id } };
  expect(JSON.stringify(requests)).not.toMatch(/"(?:path|pathDisplay|patch|oid|argv|environment)":/);
});
