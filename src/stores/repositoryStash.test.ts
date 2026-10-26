import { expect, it, vi } from 'vitest';
import { createRepositoryStashStore } from './repositoryStash';
import { createHistoryStore } from './history';
import type { Api } from './workspace';
import type { ActionIntent, ActionScope, OperationSnapshot, RepositoryStashes } from '../contracts';
import wire from '../../fixtures/contracts/ref-actions-wire.json';
const scope: ActionScope = { repoId: 'repo', worktreeId: 'tree', generation: '1' };
const catalog: RepositoryStashes = { scope, catalogId: 'catalog', observedAt: '2026-10-26', saveAvailable: true, entries: ['first', 'second'].map((entryId, ordinal) => ({ entryId, ordinal: String(ordinal), oid: 'a'.repeat(40), baseOid: 'b'.repeat(40), includesUntracked: ordinal === 0, applyAvailable: true })) };
const ok = <T,>(value: T) => ({ ok: true as const, value });
const settle = async () => { for (let i = 0; i < 4; i++) await new Promise<void>(resolve => queueMicrotask(resolve)); };
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { resolve, promise }; }
function setup(extra?: (command: string, request: Record<string, unknown>) => unknown) {
  let intent: ActionIntent;
  const api = vi.fn(async (command: string, request: Record<string, unknown>): Promise<unknown> => {
    const replacement = extra?.(command, request); if (replacement !== undefined) return replacement;
    if (command === 'get_repository_stashes') return ok({ ...catalog, scope: request.scope });
    if (command === 'resolve_stash_target') return ok({ ...request, stashHandle: `handle:${request.entryId}` });
    if (command === 'prepare_repository_action') { intent = request.intent as ActionIntent; return ok({ scope: request.scope, kind: intent.kind, planId: 'plan', warnings: [], expiresAt: '2099-01-01' }); }
    if (command === 'execute_repository_action') return ok({ ...wire.result.operation as OperationSnapshot, actionKind: intent.kind });
    throw Error(command);
  });
  const history = createHistoryStore(api as unknown as Api), store = createRepositoryStashStore(api as unknown as Api);
  history.getState().preferences.setState({ scope }); const stop = store.getState().connect(history);
  const start = async () => { await settle(); await store.getState().begin(); await settle(); };
  return { api, history, store, stop, start };
}
it('S6 uses exact preference scope, reads only while open and saves tracked changes by default', async () => {
  const t = setup(); await settle(); expect(t.api).not.toHaveBeenCalled(); await t.start();
  try {
    expect(t.store.getState().scope).toEqual(scope); expect(t.store.getState().includeUntracked).toBe(false);
    t.store.getState().setMessage('memory message'); await t.store.getState().prepareSave();
    expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action', { scope, intent: { kind: 'stashPush', message: 'memory message', includeUntracked: false } });
    t.store.getState().setIncludeUntracked(true); expect(t.store.getState().prepared).toBeNull(); await t.store.getState().prepareSave();
    expect(t.api.mock.calls.at(-1)?.[1].intent).toEqual({ kind: 'stashPush', message: 'memory message', includeUntracked: true });
    t.store.getState().end(); await t.store.getState().begin(); expect(t.store.getState().includeUntracked).toBe(false); expect(t.store.getState().message).toBe('memory message');
  } finally { t.stop(); await settle(); }
});
it('S6 resolves duplicate-OID ordered entries lazily by exact entry ID and emits only the backend handle', async () => {
  const t = setup(); await t.start();
  try {
    expect(t.api.mock.calls.some(([command]) => command === 'resolve_stash_target')).toBe(false);
    await t.store.getState().prepareApply('second'); expect(t.api).toHaveBeenCalledWith('resolve_stash_target', { scope, catalogId: catalog.catalogId, entryId: 'second' });
    expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action', { scope, intent: { kind: 'stashApply', stashHandle: 'handle:second' } });
    expect(JSON.stringify(t.api.mock.calls.filter(([command]) => command === 'prepare_repository_action'))).not.toMatch(/"(?:oid|ordinal|path|argv|environment)":/);
    const before = t.api.mock.calls.length; await t.store.getState().prepareApply('missing'); expect(t.api.mock.calls).toHaveLength(before); expect(t.store.getState().actionError?.message).toMatch(/missing/);
  } finally { t.stop(); await settle(); }
});
it.each(['scope', 'catalogId', 'entryId'])('S6 refuses resolver echo mismatch: %s', async field => {
  const t = setup((command, request) => command === 'resolve_stash_target' ? ok({ ...request, stashHandle: 'forged', ...(field === 'scope' ? { scope: { ...scope, generation: 'old' } } : { [field]: 'wrong' }) }) : undefined); await t.start();
  try { await t.store.getState().prepareApply('first'); expect(t.store.getState().prepared).toBeNull(); expect(t.store.getState().actionError?.message).toMatch(/target changed/); expect(t.api.mock.calls.some(([command]) => command === 'prepare_repository_action')).toBe(false); } finally { t.stop(); await settle(); }
});
it.each(['Dirty tracked baseline', 'Ignored path collision', 'Untracked path collision'])('S6 displays authoritative refusal without retry: %s', message => {
  const t = setup(command => command === 'prepare_repository_action' ? { ok: false, error: { code: 'InvalidHandle', phase: 'inspection', retry: 'none', message } } : undefined);
  return (async () => { await t.start(); try { await t.store.getState().prepareApply('first'); expect(t.store.getState().actionError?.message).toBe(message); expect(t.store.getState().prepared).toBeNull(); } finally { t.stop(); await settle(); } })();
});
it('S6 retains options through same-selection refresh but drops previews; ABA clears memory and late resolver replies', async () => {
  const pending = deferred<unknown>(); const t = setup(command => command === 'resolve_stash_target' ? pending.promise : undefined); await t.start();
  try {
    t.store.getState().setMessage('draft'); t.store.getState().setIncludeUntracked(true); await t.store.getState().prepareSave();
    t.history.getState().preferences.setState({ scope: { ...scope, generation: '2' } }); await settle(); expect(t.store.getState().message).toBe('draft'); expect(t.store.getState().includeUntracked).toBe(true); expect(t.store.getState().prepared).toBeNull();
    const oldScope = t.store.getState().scope!, preparing = t.store.getState().prepareApply('first');
    t.history.getState().preferences.setState({ scope: { ...scope, repoId: 'other', generation: '3' } }); t.history.getState().preferences.setState({ scope: { ...scope, generation: '4' } }); await settle();
    pending.resolve(ok({ scope: oldScope, catalogId: catalog.catalogId, entryId: 'first', stashHandle: 'old' })); await preparing;
    expect(t.store.getState().message).toBe(''); expect(t.store.getState().includeUntracked).toBe(false); expect(t.store.getState().prepared).toBeNull();
  } finally { t.stop(); await settle(); }
});
it('S6 consumes preview once, preserves draft on lost ACK and refuses expired execution', async () => {
  const pending = deferred<unknown>(); const t = setup(command => command === 'execute_repository_action' ? pending.promise : undefined); await t.start();
  try {
    t.store.getState().setMessage('keep'); await t.store.getState().prepareSave(); const execution = t.store.getState().execute(); expect(await t.store.getState().execute()).toBeNull();
    pending.resolve({ ok: false, error: { code: 'Internal', phase: 'publish', retry: 'none', message: 'Lost reply' } }); await execution;
    expect(t.store.getState().message).toBe('keep'); expect(t.api.mock.calls.filter(([command]) => command === 'execute_repository_action')).toHaveLength(1);
    await t.store.getState().prepareSave(); t.store.setState({ prepared: { ...t.store.getState().prepared!, expiresAt: '2000-01-01' } }); expect(await t.store.getState().execute()).toBeNull();
  } finally { t.stop(); await settle(); }
});
it('S6 end/disposal fences pending catalog reads and execute errors from an older manager', async () => {
  const pending = deferred<unknown>(); const t = setup(command => command === 'get_repository_stashes' ? pending.promise : undefined); await settle(); const opening = t.store.getState().begin();
  t.store.getState().end(); t.stop(); await settle(); pending.resolve(ok(catalog)); await opening;
  expect(t.store.getState().catalog).toBeNull(); expect(t.store.getState().scope).toBeNull();
});
