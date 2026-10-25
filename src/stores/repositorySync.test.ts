import { act } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { createRepositorySyncStore } from './repositorySync';
import { createHistoryStore } from './history';
import { defaultRepositoryView } from './historyView';
import type { Api } from './workspace';
import type { CommitSummary, HistoryContext, HistoryScope } from '../contracts';
import wire from '../../fixtures/contracts/daily-wire.json';

const oid = 'a'.repeat(40), otherOid = 'b'.repeat(40);
const selection = { repoId: 'repo', worktreeId: 'tree' };
const ok = <T,>(value: T) => ({ ok: true as const, value });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
function service(extra?: (command: string, request: Record<string, unknown>) => unknown) {
  const api = vi.fn(async (command: string, request: Record<string, unknown>) => {
    const replacement = extra?.(command, request); if (replacement !== undefined) return replacement;
    if (command === 'get_repository_view' || command === 'save_repository_view') return ok({ scope: request.scope, viewRevision: '1', view: request.view ?? defaultRepositoryView() });
    if (command === 'open_history') {
      const scope = request.scope as HistoryScope, context = { scope, snapshotId: `snapshot-${scope.generation}` };
      return ok({ context, sessionId: `session-${scope.generation}`, head: { kind: 'branch', branch: 'main', oid }, refs: wire.catalog.branches.map(branch => ({ ...branch, displayName: branch.name, commitOid: branch.oid, hidden: false, symbolicTarget: null })), roots: [oid],
        page: { context, sessionId: `session-${scope.generation}`, commits: [oid, otherOid].map(value => commit(value, context)), nextCursor: null } });
    }
    if (command === 'get_commit_details') return ok({ context: request.context, commit: commit((request.commitHandle as string).split(':')[0], request.context as HistoryContext), base: request.base, files: [], message: 'Message', committerName: 'Author', committerEmail: 'test@example.test' });
    if (command === 'get_repository_actions') return ok({ ...wire.catalog, scope: request.scope });
    if (command === 'get_commit_containment') return ok({ ...wire.containment, context: request.context, commitHandle: request.commitHandle, oid: (request.commitHandle as string).split(':')[0] });
    if (command === 'prepare_repository_action') return ok({ ...wire.prepared, scope: request.scope, kind: 'fetch', expiresAt: '2099-01-01T00:00:00Z' });
    if (command === 'execute_repository_action') return ok({ ...wire.operation, kind: 'action', actionKind: 'fetch', repoId: selection.repoId, worktreeId: selection.worktreeId });
    throw Error(`Unexpected test command ${command}`);
  });
  const history = createHistoryStore(api as unknown as Api), store = createRepositorySyncStore(api as unknown as Api);
  const stop = store.getState().connect(history);
  return { api, history, store, stop };
}
const commit = (value: string, context: HistoryContext): CommitSummary => ({ oid: value, handle: `${value}:${context.scope.generation}`, parents: [], authorName: 'Author', authorEmail: 'test@example.test', authoredAt: '', committedAt: '', subject: value, decorations: [] });
const settle = () => act(async () => { await new Promise<void>(resolve => queueMicrotask(resolve)); });

it('AC1 uses the exact preference scope, full selected handle/context and no catalog timer or fetch', async () => {
  const timer = vi.spyOn(globalThis, 'setInterval'), { api, history, store, stop } = service();
  try {
    await history.getState().activate(selection); await settle();
    expect(store.getState().catalog?.scope).toEqual(history.getState().preferences.getState().scope);
    history.getState().selectCommit(oid); await settle();
    const selected = history.getState().selected!;
    expect(api).toHaveBeenCalledWith('get_commit_containment', { context: selected.context, commitHandle: selected.commit.handle });
    expect(store.getState().containment?.branches.map(value => value.branch.name)).toEqual(wire.containment.branches.map(value => value.branch.name));
    expect(api.mock.calls.some(([command]) => command === 'prepare_repository_action' || command === 'execute_repository_action')).toBe(false);
    const reads = api.mock.calls.length;
    history.getState().preferences.getState().patch({ detailsHeight: 450 }); await settle();
    expect(api.mock.calls).toHaveLength(reads);
    expect(timer).not.toHaveBeenCalled();
  } finally { stop(); history.getState().deactivate(); timer.mockRestore(); await settle(); }
});

it('AC1 rejects late selection responses, wrong echo and same-OID handles from another snapshot', async () => {
  const first = deferred<unknown>(); let defer = true;
  const test = service((command, request) => command === 'get_commit_containment' && (request.commitHandle as string).startsWith(oid) && defer ? first.promise : undefined);
  try {
    await test.history.getState().activate(selection); test.history.getState().selectCommit(oid); await settle();
    test.history.getState().selectCommit(otherOid); await settle();
    expect(test.store.getState().containment?.oid).toBe(otherOid);
    first.resolve(ok({ ...wire.containment, oid, context: test.history.getState().session!.context, commitHandle: `${oid}:1` })); await settle();
    expect(test.store.getState().containment?.oid).toBe(otherOid);
    defer = false;
    test.api.mockImplementationOnce(async () => ok({ ...wire.containment, context: { ...test.history.getState().session!.context, snapshotId: 'forged' }, oid: otherOid, commitHandle: `${otherOid}:1` }));
    await test.store.getState().retryContainment();
    expect(test.store.getState().containmentError?.code).toBe('InvalidHandle');
  } finally { test.stop(); test.history.getState().deactivate(); await settle(); }
});

it('AC1 does not dispatch retained selection during scope staging and resumes only after current session publishes', async () => {
  const refresh = deferred<unknown>(); let staged = false;
  const test = service((command, request) => command === 'open_history' && staged ? refresh.promise : undefined);
  try {
    await test.history.getState().activate(selection); test.history.getState().selectCommit(oid); await settle();
    const old = test.history.getState().session!, before = test.api.mock.calls.filter(([command]) => command === 'get_commit_containment').length;
    staged = true; const pending = test.history.getState().refresh(); await settle();
    expect(test.history.getState().selected?.context).toEqual(old.context);
    expect(test.store.getState().scope).toEqual(test.history.getState().preferences.getState().scope);
    expect(test.api.mock.calls.filter(([command]) => command === 'get_commit_containment')).toHaveLength(before);
    test.history.getState().selectCommit(otherOid); await settle();
    expect(test.api.mock.calls.filter(([command]) => command === 'get_commit_containment')).toHaveLength(before);
    const scope = test.history.getState().preferences.getState().scope!, context = { scope, snapshotId: 'new-snapshot' };
    refresh.resolve(ok({ ...old, context, sessionId: 'new-session', page: { context, sessionId: 'new-session', commits: [commit(oid, context), commit(otherOid, context)], nextCursor: null } }));
    await pending; await settle();
    expect(test.store.getState().containment?.context).toEqual(context);
    expect(test.store.getState().containment?.oid).toBe(otherOid);
    expect(test.api.mock.calls.filter(([command]) => command === 'get_commit_containment')).toHaveLength(before + 1);
  } finally { test.stop(); test.history.getState().deactivate(); await settle(); }
});

it('AC1 cancels stale catalogs/reads on repo switch and disposal; StrictMode reconnect coalesces reads', async () => {
  const stale = deferred<unknown>(); let catalogReads = 0;
  const test = service((command, request) => command === 'get_repository_actions' && ++catalogReads === 1 ? stale.promise : undefined);
  await test.history.getState().activate(selection); await settle();
  const secondOwner = test.store.getState().connect(test.history); test.stop(); await settle();
  expect(catalogReads).toBe(1);
  await test.history.getState().activate({ repoId: 'other', worktreeId: null }); await settle();
  expect(test.store.getState().catalog?.scope.repoId).toBe('other');
  stale.resolve(ok({ ...wire.catalog, scope: { ...selection, generation: '1' } })); await settle();
  expect(test.store.getState().catalog?.scope.repoId).toBe('other');
  secondOwner(); await settle(); test.history.getState().deactivate();
  expect(test.store.getState().scope).toBeNull();
});

it('AC2 prepares explicit current handles/prune, invalidates preview on scope change and prevents double execute', async () => {
  const execution = deferred<unknown>(); let deferExecute = false;
  const test = service((command) => command === 'execute_repository_action' && deferExecute ? execution.promise : undefined);
  try {
    await test.history.getState().activate(selection); await settle();
    const handle = test.store.getState().catalog!.remotes[0].handle!;
    await test.store.getState().prepareFetch([handle, handle], false);
    expect(test.api).toHaveBeenCalledWith('prepare_repository_action', { scope: test.history.getState().preferences.getState().scope, intent: { kind: 'fetch', remoteHandles: [handle], prune: false } });
    await test.history.getState().refresh(); await settle(); expect(test.store.getState().prepared).toBeNull();
    await test.store.getState().prepareFetch([handle], true);
    deferExecute = true;
    const first = test.store.getState().executeFetch(), second = await test.store.getState().executeFetch();
    expect(second).toBeNull(); expect(test.api.mock.calls.filter(([command]) => command === 'execute_repository_action')).toHaveLength(1);
    execution.resolve(ok({ ...wire.operation, kind: 'action', actionKind: 'fetch', repoId: 'repo', worktreeId: 'tree' })); await first;
    expect(test.store.getState().prepared).toBeNull();
  } finally { test.stop(); test.history.getState().deactivate(); await settle(); }
});

it('AC2 rejects expired, unavailable and immediately stale scope actions before dispatch', async () => {
  const test = service();
  try {
    await test.history.getState().activate(selection); await settle();
    await test.store.getState().prepareFetch(['forged-handle'], false);
    expect(test.api.mock.calls.some(([command]) => command === 'prepare_repository_action')).toBe(false);
    const handle = test.store.getState().catalog!.remotes[0].handle!;
    await test.store.getState().prepareFetch([handle], false);
    test.store.setState({ prepared: { ...test.store.getState().prepared!, expiresAt: '2000-01-01T00:00:00Z' } });
    expect(await test.store.getState().executeFetch()).toBeNull();
    expect(test.api.mock.calls.some(([command]) => command === 'execute_repository_action')).toBe(false);
    await test.store.getState().prepareFetch([handle], false);
    const refreshing = test.history.getState().refresh();
    // Preferences bind synchronously before the coalesced sync subscription.
    expect(await test.store.getState().executeFetch()).toBeNull();
    expect(test.api.mock.calls.some(([command]) => command === 'execute_repository_action')).toBe(false);
    await refreshing; await settle();
  } finally { test.stop(); test.history.getState().deactivate(); await settle(); }
});

it('AC2 drops a pending prepare on disposal and rejects forged execute echoes without replay', async () => {
  const prepared = deferred<unknown>(); let pause = false, forged = false;
  const test = service((command, request) => command === 'prepare_repository_action' && pause ? prepared.promise
    : command === 'execute_repository_action' && forged ? ok({ ...wire.operation, kind: 'action', actionKind: 'fetch', repoId: 'wrong-repo', worktreeId: 'tree' }) : undefined);
  await test.history.getState().activate(selection); await settle();
  const handle = test.store.getState().catalog!.remotes[0].handle!;
  await test.store.getState().prepareFetch([handle], false); forged = true;
  const result = await test.store.getState().executeFetch();
  expect(result?.ok).toBe(false); expect(test.store.getState().actionError?.code).toBe('InvalidHandle');
  expect(await test.store.getState().executeFetch()).toBeNull();
  expect(test.api.mock.calls.filter(([command]) => command === 'execute_repository_action')).toHaveLength(1);
  pause = true;
  const pending = test.store.getState().prepareFetch([handle], false);
  test.stop(); await settle();
  prepared.resolve(ok({ ...wire.prepared, expiresAt: '2099-01-01T00:00:00Z' })); await pending;
  expect(test.store.getState().prepared).toBeNull(); expect(test.store.getState().scope).toBeNull(); test.history.getState().deactivate();
});

it('S3-FORM01 catalog readiness waits for the current request and supports cancellation without polling', async () => {
  const catalog = deferred<unknown>();
  const test = service(command => command === 'get_repository_actions' ? catalog.promise : undefined);
  try {
    await test.history.getState().activate(selection); await settle();
    const abort = new AbortController(), ready = test.store.getState().awaitCatalog(abort.signal);
    let settled = false; void ready.then(() => { settled = true; }); await settle(); expect(settled).toBe(false);
    const cancelled = new AbortController(), cancelledReady = test.store.getState().awaitCatalog(cancelled.signal);
    cancelled.abort(); expect(await cancelledReady).toBe(false);
    catalog.resolve(ok({ ...wire.catalog, scope: test.history.getState().preferences.getState().scope }));
    expect(await ready).toBe(true); expect(test.api.mock.calls.filter(([command]) => command === 'get_repository_actions')).toHaveLength(1);
  } finally { test.stop(); test.history.getState().deactivate(); await settle(); }
});
