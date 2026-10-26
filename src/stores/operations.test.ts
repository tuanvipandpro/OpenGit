import { expect, it, vi } from 'vitest';
import type { ActionOperationResult, AppError, OperationSnapshot, OperationsSnapshot } from '../contracts';
import type { ActionScope, StashActionResult } from '../contracts';
import type { Api } from './workspace';
import { createOperationsStore } from './operations';
import wire from '../../fixtures/contracts/daily-wire.json';
import refWire from '../../fixtures/contracts/ref-actions-wire.json';
const operation = wire.operation as OperationSnapshot;
const action = (id: string): OperationSnapshot => ({ ...operation, id, sequence: '1', kind: 'action', actionKind: 'fetch', state: 'Failed' });
const result = (value: OperationSnapshot, prune = false): ActionOperationResult => ({ operation: value, fetch: { prune, remotes: [] } });
const failure: AppError = { code: 'Internal', phase: 'inspection', retry: 'retry', message: 'Result unavailable' };
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
const stashScope: ActionScope = { repoId: 'repo', worktreeId: 'tree', generation: '1' };
const stashEvidence: StashActionResult = { stashOid: 'a'.repeat(40), stashRetention: 'present', recoveryRefRetention: 'unknown', resolution: 'unmerged', failureReason: 'conflict' };
const stashCatalog = () => ({ scope: stashScope, catalogId: 'discarded-runtime-catalog', observedAt: '', entries: [], saveAvailable: true });
it('S6 loads typed stash outcomes and Reconciles only matching current repo/worktree context without a new operation', async () => {
  const value: OperationSnapshot = { ...action('stash'), actionKind: 'stashApply', repoId: 'repo', worktreeId: 'tree' };
  let currentScope: ActionScope | null = stashScope;
  const api = vi.fn(async (command: string) => command === 'get_operations' ? { ok: true, value: { streamId: 'stash', sequence: '1', operations: [value] } } : command === 'get_repository_stashes' ? { ok: true, value: stashCatalog() } : { ok: true, value: { operation: value, result: command === 'reconcile_stash_action' ? { ...stashEvidence, resolution: 'resolvedExternally' } : stashEvidence } }) as unknown as Api;
  const store = createOperationsStore(api); await store.getState().sync(); await store.getState().loadOutcome(value.id);
  expect(api).toHaveBeenCalledWith('get_stash_action_outcome', { operationId: value.id }); expect(store.getState().outcomes[value.id].stashResult?.recoveryRefRetention).toBe('unknown');
  currentScope = { ...stashScope, repoId: 'other' }; await store.getState().reconcileStash(stashScope, value.id, () => currentScope); expect(vi.mocked(api).mock.calls.some(([command]) => command === 'reconcile_stash_action')).toBe(false);
  currentScope = stashScope; await store.getState().reconcileStash(stashScope, value.id, () => currentScope);
  const commands = vi.mocked(api).mock.calls.map(([command]) => command);
  expect(commands.indexOf('get_repository_stashes')).toBeLessThan(commands.indexOf('reconcile_stash_action'));
  expect(api).toHaveBeenCalledWith('reconcile_stash_action', { scope: stashScope, operationId: value.id }); expect(store.getState().outcomes[value.id].stashResult?.resolution).toBe('resolvedExternally'); expect(store.getState().snapshot?.operations).toEqual([value]);
  expect(vi.mocked(api).mock.calls.some(([command]) => ['prepare_repository_action', 'execute_repository_action'].includes(command))).toBe(false);
});
it('S6 Reconcile fences late responses across scope/stream changes and authoritative removal retires transient state', async () => {
  const value: OperationSnapshot = { ...action('stash'), actionKind: 'stashApply', repoId: 'repo', worktreeId: 'tree' }, pending = deferred<unknown>();
  let currentScope: ActionScope | null = stashScope, next: OperationsSnapshot = { streamId: 'stash', sequence: '1', operations: [value] };
  const api = vi.fn(async (command: string) => command === 'get_operations' ? { ok: true, value: next } : command === 'get_repository_stashes' ? { ok: true, value: stashCatalog() } : pending.promise) as unknown as Api;
  const store = createOperationsStore(api); await store.getState().sync(); const read = store.getState().reconcileStash(stashScope, value.id, () => currentScope);
  await Promise.resolve(); await Promise.resolve();
  expect(api).toHaveBeenCalledWith('reconcile_stash_action', { scope: stashScope, operationId: value.id });
  expect(store.getState().reconciling[value.id]).toBe(true); currentScope = { ...stashScope, generation: '2' }; next = { streamId: 'replacement', sequence: '2', operations: [] }; await store.getState().sync();
  pending.resolve({ ok: true, value: { operation: value, result: stashEvidence } }); await read; expect(store.getState().outcomes).toEqual({}); expect(store.getState().reconciling).toEqual({});
});
it('S6 Reconcile waits for an in-flight outcome read and submits one read-only reconciliation', async () => {
  const value: OperationSnapshot = { ...action('stash'), actionKind: 'stashApply', repoId: 'repo', worktreeId: 'tree' }, pending = deferred<unknown>();
  const api = vi.fn(async (command: string) => command === 'get_operations' ? { ok: true, value: { streamId: 'stash', sequence: '1', operations: [value] } } : command === 'get_stash_action_outcome' ? pending.promise : command === 'get_repository_stashes' ? { ok: true, value: stashCatalog() } : { ok: true, value: { operation: value, result: stashEvidence } }) as unknown as Api;
  const store = createOperationsStore(api); await store.getState().sync(); const outcome = store.getState().loadOutcome(value.id), reconcile = store.getState().reconcileStash(stashScope, value.id, () => stashScope);
  pending.resolve({ ok: true, value: { operation: value, result: stashEvidence } }); await Promise.all([outcome, reconcile]); expect(vi.mocked(api).mock.calls.filter(([command]) => command === 'reconcile_stash_action')).toHaveLength(1);
});

it('S6 Reconcile discards a late scope-publication reply without sending a stale reconcile or retaining catalog authority', async () => {
  const value: OperationSnapshot = { ...action('stash'), actionKind: 'stashApply', repoId: 'repo', worktreeId: 'tree' }, pending = deferred<unknown>();
  let current: ActionScope | null = stashScope;
  const api = vi.fn(async (command: string) => command === 'get_operations' ? { ok: true, value: { streamId: 'stash', sequence: '1', operations: [value] } } : pending.promise) as unknown as Api;
  const store = createOperationsStore(api); await store.getState().sync();
  const read = store.getState().reconcileStash(stashScope, value.id, () => current);
  current = { ...stashScope, generation: '2' }; pending.resolve({ ok: true, value: stashCatalog() }); await read;
  expect(vi.mocked(api).mock.calls.some(([command]) => command === 'reconcile_stash_action')).toBe(false);
  expect(store.getState().outcomes).toEqual({}); expect(store.getState().reconciling).toEqual({});
});
it.each(['error', 'wrongScope'] as const)('S6 Reconcile keeps prior evidence and refuses a %s scope-publication reply', async reason => {
  const value: OperationSnapshot = { ...action('stash'), actionKind: 'stashApply', repoId: 'repo', worktreeId: 'tree' };
  const api = vi.fn(async (command: string) => command === 'get_operations' ? { ok: true, value: { streamId: 'stash', sequence: '1', operations: [value] } }
    : command === 'get_repository_stashes' ? reason === 'error' ? { ok: false, error: failure } : { ok: true, value: { ...stashCatalog(), scope: { ...stashScope, generation: 'other' } } }
      : { ok: true, value: { operation: value, result: stashEvidence } }) as unknown as Api;
  const store = createOperationsStore(api); await store.getState().sync(); await store.getState().loadOutcome(value.id);
  await store.getState().reconcileStash(stashScope, value.id, () => stashScope);
  expect(vi.mocked(api).mock.calls.some(([command]) => command === 'reconcile_stash_action')).toBe(false);
  expect(store.getState().outcomes[value.id].stashResult).toEqual(stashEvidence);
  expect(store.getState().outcomeErrors[value.id]).toBeDefined(); expect(store.getState().outcomes[value.id]).not.toHaveProperty('catalogId');
});

it.each(['stageSelection', 'unstageSelection'] as const)('S5-B retains %s structured change outcome and retires it on removal', async kind => {
  const value: OperationSnapshot = { ...action('partial'), actionKind: kind };
  const changeResult = { commitOid: null, treeOid: null, failureReason: 'stale', artifactsRetained: true };
  let next: OperationsSnapshot = { streamId: 'partial', sequence: '1', operations: [value] };
  const api = vi.fn(async (command: string) => command === 'get_operations' ? { ok: true, value: next } : { ok: true, value: { operation: value, result: changeResult } }) as unknown as Api;
  const store = createOperationsStore(api); await store.getState().sync(); await store.getState().loadOutcome(value.id);
  expect(api).toHaveBeenCalledWith('get_change_action_outcome', { operationId: value.id });
  expect(store.getState().outcomes[value.id].changeResult).toEqual(changeResult);
  next = { ...next, sequence: '2', operations: [] }; await store.getState().sync();
  expect(store.getState().outcomes).toEqual({});
});

it.each(['push', 'pull', 'remoteTagPush', 'remoteTagDelete'] as const)('Sync routes %s outcomes and retires cached evidence on compaction', async kind => {
  const value: OperationSnapshot = { ...action('sync'), actionKind: kind }, syncResult = { steps: [], publication: null, trackingUpdated: false, upstreamUpdated: false, trackingRefreshRequired: true, fetchedOid: null, integratedOid: null };
  let next: OperationsSnapshot = { streamId: 'sync', sequence: '1', operations: [value] };
  const api = vi.fn(async (command: string) => command === 'get_operations' ? { ok: true, value: next } : { ok: true, value: { operation: value, result: syncResult } }) as unknown as Api;
  const store = createOperationsStore(api); await store.getState().sync(); await store.getState().loadOutcome(value.id);
  expect(api).toHaveBeenCalledWith('get_sync_action_outcome', { operationId: value.id }); expect(store.getState().outcomes[value.id].syncResult).toEqual(syncResult);
  next = { ...next, sequence: '2', operations: [] }; await store.getState().sync(); expect(store.getState().outcomes).toEqual({});
});
it('Sync fences pending outcome replies across stream replacement and accepts null evidence conservatively', async () => {
  const value: OperationSnapshot = { ...action('sync'), actionKind: 'pull' }, pending = deferred<unknown>(); let count = 0;
  let next: OperationsSnapshot = { streamId: 'sync', sequence: '1', operations: [value] };
  const api = vi.fn(async (command: string) => command === 'get_operations' ? { ok: true, value: next } : ++count === 1 ? pending.promise : { ok: true, value: { operation: value, result: null } }) as unknown as Api;
  const store = createOperationsStore(api); await store.getState().sync(); const old = store.getState().loadOutcome(value.id);
  next = { ...next, streamId: 'replacement' }; await store.getState().sync(); await store.getState().loadOutcome(value.id);
  pending.resolve({ ok: true, value: { operation: value, result: { steps: [] } } }); await old;
  expect(store.getState().outcomes[value.id].syncResult).toBeNull();
});
it('Sync routes authenticated reference-read evidence without retaining destination handles in Tasks', async () => {
  const value: OperationSnapshot = { ...action('read'), actionKind: 'readRemoteRefs' }, evidence = { remoteName: 'origin', role: 'push', phase: 'observed', refCount: '3', observedAt: '2026-10-25', noGitWrites: true };
  const api = vi.fn(async (command: string) => command === 'get_operations' ? { ok: true, value: { streamId: 'read', sequence: '1', operations: [value] } } : { ok: true, value: { operation: value, availability: 'available', evidence, snapshot: { snapshotId: 'ephemeral-destination-authority' } } }) as unknown as Api;
  const store = createOperationsStore(api); await store.getState().sync(); await store.getState().loadOutcome(value.id);
  expect(api).toHaveBeenCalledWith('get_remote_action_outcome', { operationId: value.id });
  expect(store.getState().outcomes[value.id].remoteReadResult).toEqual({ availability: 'available', evidence }); expect(store.getState().outcomes[value.id]).not.toHaveProperty('snapshot');
});

it('S7 routes local-ref evidence through ref outcomes and retires it with authoritative removal', async () => {
  const local = refWire.result.operation as OperationSnapshot;
  let next: OperationsSnapshot = { streamId: 'refs', sequence: local.sequence, operations: [local] };
  const api = vi.fn(async (command: string) => command === 'get_operations' ? { ok: true, value: next } : { ok: true, value: refWire.result }) as unknown as Api;
  const store = createOperationsStore(api); await store.getState().sync(); await store.getState().loadOutcome(local.id);
  expect(api).toHaveBeenCalledWith('get_ref_action_outcome', { operationId: local.id });
  expect(store.getState().outcomes[local.id].refResult).toEqual(refWire.result.result); expect(store.getState().outcomes[local.id].fetch).toBeNull();
  next = { ...next, sequence: '4', operations: [] }; await store.getState().sync(); expect(store.getState().outcomes).toEqual({});
});

it('S7 fences local-ref replies after stream replacement and handles unavailable evidence', async () => {
  const local = refWire.result.operation as OperationSnapshot, old = deferred<unknown>();
  let next: OperationsSnapshot = { streamId: 'refs', sequence: local.sequence, operations: [local] }, count = 0;
  const api = vi.fn(async (command: string) => command === 'get_operations' ? { ok: true, value: next } : ++count === 1 ? old.promise : { ok: true, value: { operation: local, result: null } }) as unknown as Api;
  const store = createOperationsStore(api); await store.getState().sync(); const pending = store.getState().loadOutcome(local.id);
  next = { ...next, streamId: 'replacement' }; await store.getState().sync(); await store.getState().loadOutcome(local.id);
  old.resolve({ ok: true, value: refWire.result }); await pending;
  expect(store.getState().outcomes[local.id].refResult).toBeNull();
});

it('S3-CACHE01 retires outcomes/errors over 50 authoritative compaction rounds and retains a visible result', async () => {
  const survivor = action('survivor');
  let next: OperationsSnapshot = { streamId: 'initial', sequence: '1', operations: [survivor] };
  const errors = new Set<string>();
  const api = vi.fn(async (command: string, request: { operationId?: string }) => command === 'get_operations' ? { ok: true, value: next }
    : errors.has(request.operationId!) ? { ok: false, error: failure }
      : { ok: true, value: result(next.operations.find(value => value.id === request.operationId)!) }) as unknown as Api;
  const store = createOperationsStore(api); await store.getState().sync(); await store.getState().loadOutcome(survivor.id);
  const retained = store.getState().outcomes[survivor.id];
  for (let round = 0; round < 50; round++) {
    const ephemeral = action(`compacted-${round}`), errored = action(`errored-${round}`); errors.add(errored.id);
    next = { streamId: `append-${round}`, sequence: '2', operations: [survivor, ephemeral, errored] };
    await store.getState().sync(); await store.getState().loadOutcome(ephemeral.id); await store.getState().loadOutcome(errored.id);
    expect(Object.keys(store.getState().outcomes)).toHaveLength(2); expect(Object.keys(store.getState().outcomeErrors)).toEqual([errored.id]);
    next = { streamId: `compacted-stream-${round}`, sequence: '3', operations: [survivor] }; await store.getState().sync();
    expect(Object.keys(store.getState().outcomes)).toEqual([survivor.id]); expect(store.getState().outcomeErrors).toEqual({});
    expect(store.getState().outcomes[survivor.id]).toBe(retained);
  }
  next = { streamId: 'empty', sequence: '4', operations: [] }; await store.getState().sync();
  expect(store.getState().snapshot!.operations).toEqual([]); expect(store.getState().outcomes).toEqual({}); expect(store.getState().outcomeErrors).toEqual({});
});

it('S3-CACHE01 fences removed/reintroduced equivalent IDs and stream-retired pending success/error responses', async () => {
  const value = action('reintroduced');
  let next: OperationsSnapshot = { streamId: 'first', sequence: '1', operations: [value] };
  const requests: ReturnType<typeof deferred<unknown>>[] = [];
  const api = vi.fn(async (command: string) => {
    if (command === 'get_operations') return { ok: true, value: next };
    const request = deferred<unknown>(); requests.push(request); return request.promise;
  }) as unknown as Api;
  const store = createOperationsStore(api); await store.getState().sync();
  const oldRead = store.getState().loadOutcome(value.id);
  next = { ...next, sequence: '2', operations: [] }; await store.getState().sync();
  next = { ...next, sequence: '3', operations: [value] }; await store.getState().sync();
  const freshRead = store.getState().loadOutcome(value.id); expect(requests).toHaveLength(2);
  requests[0].resolve({ ok: true, value: result(value, false) }); await oldRead;
  expect(store.getState().outcomes[value.id]).toBeUndefined();
  requests[1].resolve({ ok: true, value: result(value, true) }); await freshRead;
  expect(store.getState().outcomes[value.id].fetch!.prune).toBe(true);
  const oldStreamRead = store.getState().loadOutcome(value.id);
  next = { ...next, streamId: 'rotated' }; await store.getState().sync();
  const currentStreamRead = store.getState().loadOutcome(value.id); expect(requests).toHaveLength(4);
  requests[2].resolve({ ok: false, error: failure }); await oldStreamRead;
  expect(store.getState().outcomeErrors).toEqual({}); expect(store.getState().outcomes[value.id].fetch!.prune).toBe(true);
  requests[3].resolve({ ok: true, value: result(value, false) }); await currentStreamRead;
  expect(store.getState().outcomes[value.id].fetch!.prune).toBe(false);
});

it('S3-CACHE01 prunes only accepted snapshots and invalidates stale cache/request versions on accepted events', async () => {
  let receive!: (event: import('../contracts').OperationEvent) => void;
  const value = action('visible'); let next: OperationsSnapshot = { streamId: 'stream', sequence: '5', operations: [value] };
  const api = vi.fn(async (command: string) => command === 'get_operations' ? { ok: true, value: next } : { ok: true, value: result(value) }) as unknown as Api;
  const store = createOperationsStore(api, async handler => { receive = handler; return () => {}; }); const stop = await store.getState().start();
  try {
    await store.getState().loadOutcome(value.id); const retained = store.getState().outcomes[value.id];
    next = { ...next, sequence: '4', operations: [] }; await store.getState().sync();
    expect(store.getState().outcomes[value.id]).toBe(retained); expect(store.getState().snapshot!.operations).toHaveLength(1);
    receive({ streamId: 'stream', sequence: '6', operation: { ...value, sequence: '6', updatedAt: '2099-01-01', state: 'Interrupted' } });
    expect(store.getState().outcomes).toEqual({});
  } finally { stop(); }
});
