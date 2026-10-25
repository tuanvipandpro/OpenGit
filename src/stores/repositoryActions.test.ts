import { expect, it, vi } from 'vitest';
import { createRepositoryActionsStore } from './repositoryActions';
import type { LocalActionDraft } from './repositoryActions';
import { createHistoryStore } from './history';
import type { Api } from './workspace';
import type { ActionIntent, AppError, RefActionSource } from '../contracts';
import fixture from '../../fixtures/contracts/ref-actions-wire.json';
import { defaultRepositoryView } from './historyView';
const ok = <T,>(value: T) => ({ ok: true as const, value });
const settle = async () => { await new Promise<void>(resolve => queueMicrotask(resolve)); };
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
const draft = (patch: Partial<LocalActionDraft> = {}): LocalActionDraft => ({ kind: 'branchCreate', source: { kind: 'head' }, name: 'topic', checkout: false, upstream: null, annotated: false, message: '', sign: false, ...patch });
function setup(extra?: (command: string, request: Record<string, unknown>) => unknown) {
  let serial = 0;
  const api = vi.fn(async (command: string, request: Record<string, unknown>): Promise<unknown> => {
    const replace = extra?.(command, request); if (replace !== undefined) return replace;
    if (command === 'get_repository_view' || command === 'save_repository_view') return ok({ scope: request.scope, viewRevision: '1', view: request.view ?? defaultRepositoryView() });
    if (command === 'open_history') {
      const context = { scope: request.scope, snapshotId: `snapshot-${++serial}` };
      return ok({ context, sessionId: 'session', head: fixture.catalog.head, refs: [], roots: [], page: { context, sessionId: 'session', commits: [], nextCursor: null } });
    }
    if (command === 'get_ref_actions') return ok({ ...fixture.catalog, scope: request.scope, catalogId: `catalog-${serial}`, refs: [...fixture.catalog.refs, { ...fixture.catalog.refs[0], entryId: 'remote', kind: 'remote', name: 'refs/remotes/origin/main', checkedOutHere: false }] });
    if (command === 'resolve_ref_action_target') {
      const source = request.source as RefActionSource;
      const entry = store.getState().catalog!.refs.find(ref => source.kind === 'ref' && ref.entryId === source.entryId);
      return ok({ ...fixture.resolved, scope: request.scope, catalogId: request.catalogId, kind: entry ? entry.kind === 'local' ? 'branch' : entry.kind === 'remote' ? 'remoteRef' : 'tag' : 'commit', displayName: entry?.name ?? 'HEAD', handle: entry?.entryId ?? 'head-handle' });
    }
    if (command === 'prepare_repository_action') return ok({ scope: request.scope, kind: (request.intent as ActionIntent).kind, planId: 'plan', warnings: [], expiresAt: '2099-01-01T00:00:00Z' });
    if (command === 'execute_repository_action') return ok({ ...fixture.result.operation, actionKind: store.getState().prepared?.kind ?? 'branchCreate' });
    throw Error(command);
  });
  const history = createHistoryStore(api as unknown as Api), store = createRepositoryActionsStore(api as unknown as Api);
  const stop = store.getState().connect(history);
  const start = async () => { await history.getState().activate({ repoId: 'repo', worktreeId: 'tree' }); await settle(); };
  return { api, history, store, start, stop };
}
it('S7 uses exact preference authority and resolves qualified same-OID refs lazily, preserving source identity', async () => {
  const t = setup(); await t.start();
  try {
    expect(t.store.getState().scope).toEqual(t.history.getState().preferences.getState().scope);
    expect(t.api.mock.calls.some(([command]) => command === 'resolve_ref_action_target')).toBe(false);
    await t.store.getState().prepare(draft({ source: { kind: 'ref', name: 'refs/remotes/origin/main' }, checkout: true }));
    expect(t.api).toHaveBeenCalledWith('resolve_ref_action_target', { scope: t.store.getState().scope, catalogId: t.store.getState().catalog!.catalogId, source: { kind: 'ref', entryId: 'remote' } });
    expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action', { scope: t.store.getState().scope, intent: { kind: 'branchCreate', name: 'topic', startHandle: 'remote', checkout: true } });
    expect(t.store.getState().preview).toEqual({ source: 'refs/remotes/origin/main', destination: 'topic (checkout)' });
  } finally { t.stop(); await settle(); }
});
it('S7 maps every local intent and resolves upstream by current qualified name', async () => {
  const t = setup(); await t.start();
  try {
    const cases: [Partial<LocalActionDraft>, ActionIntent][] = [
      [{ kind: 'branchRename', name: 'renamed' }, { kind: 'branchRename', branchHandle: 'entry-0', newName: 'renamed' }],
      [{ kind: 'branchSwitch' }, { kind: 'branchSwitch', branchHandle: 'entry-0' }],
      [{ kind: 'branchDelete' }, { kind: 'branchDelete', branchHandle: 'entry-0' }],
      [{ kind: 'branchTracking', upstream: 'refs/remotes/origin/main' }, { kind: 'branchTracking', branchHandle: 'entry-0', upstreamHandle: 'remote' }],
      [{ kind: 'branchTracking' }, { kind: 'branchTracking', branchHandle: 'entry-0', upstreamHandle: null }],
    ];
    for (const [patch, intent] of cases) { await t.store.getState().prepare(draft({ source: { kind: 'ref', name: 'refs/heads/main' }, ...patch })); expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action', { scope: t.store.getState().scope, intent }); }
    await t.store.getState().prepare(draft({ kind: 'tagCreate', annotated: true, message: 'Release', sign: true }));
    expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action', { scope: t.store.getState().scope, intent: { kind: 'tagCreate', name: 'topic', targetHandle: 'head-handle', annotation: { kind: 'annotated', message: 'Release', sign: true } } });
    await t.store.getState().prepare(draft({ kind: 'tagCreate' }));
    expect(t.api.mock.calls.at(-1)?.[1].intent).toMatchObject({ annotation: { kind: 'lightweight' } });
    await t.store.getState().prepare(draft({ kind: 'tagDelete', source: { kind: 'ref', name: 'refs/tags/v1' } }));
    expect(t.api.mock.calls.at(-1)?.[1].intent).toEqual({ kind: 'tagDelete', tagHandle: 'entry-2' });
  } finally { t.stop(); await settle(); }
});
it('S7 rejects symbolic, missing and wrong-kind mutation targets without preparing', async () => {
  const t = setup(); await t.start();
  try {
    for (const name of ['refs/remotes/origin/HEAD', 'refs/tags/v1', 'refs/heads/missing']) {
      await t.store.getState().prepare(draft({ kind: 'branchDelete', source: { kind: 'ref', name } }));
      expect(t.store.getState().actionError?.code).toBe('InvalidHandle');
    }
    expect(t.api.mock.calls.filter(([command]) => command === 'prepare_repository_action')).toHaveLength(0);
  } finally { t.stop(); await settle(); }
});
it.each(['scope', 'catalog', 'kind', 'name'])('S7 rejects a resolver response with mismatched %s', async mismatch => {
  const t = setup((command, request) => command === 'resolve_ref_action_target' ? ok({ ...fixture.resolved, scope: mismatch === 'scope' ? { repoId: 'other', worktreeId: 'tree', generation: '1' } : request.scope,
    catalogId: mismatch === 'catalog' ? 'stale' : request.catalogId, kind: mismatch === 'kind' ? 'tag' : 'branch', displayName: mismatch === 'name' ? 'refs/heads/wrong' : 'refs/heads/main' }) : undefined);
  await t.start();
  try {
    await t.store.getState().prepare(draft({ kind: 'branchDelete', source: { kind: 'ref', name: 'refs/heads/main' } }));
    expect(t.store.getState().actionError?.code).toBe('InvalidHandle'); expect(t.api.mock.calls.some(([command]) => command === 'prepare_repository_action')).toBe(false);
  } finally { t.stop(); await settle(); }
});
it('S7 rejects retained selected commits until their context matches the published session and preference scope', async () => {
  const t = setup(); await t.start();
  try {
    const context = t.history.getState().session!.context;
    const selected = { context, outsideView: false, commit: { handle: 'selected', oid: 'a', parents: [], subject: '', authorName: '', authorEmail: '', authoredAt: '', committedAt: '', decorations: [] } };
    t.history.setState({ selected: { ...selected, context: { ...context, snapshotId: 'old' } } }); await settle();
    await t.store.getState().prepare(draft({ source: { kind: 'selectedCommit' } })); expect(t.store.getState().actionError?.code).toBe('InvalidHandle');
    t.history.setState({ selected }); await settle();
    await t.store.getState().prepare(draft({ source: { kind: 'selectedCommit' } }));
    expect(t.api).toHaveBeenCalledWith('resolve_ref_action_target', { scope: context.scope, catalogId: t.store.getState().catalog!.catalogId, source: { kind: 'historyCommit', context, commitHandle: 'selected' } });
    t.history.setState({ selected: null }); await settle(); expect(t.store.getState().prepared).toBeNull();
  } finally { t.stop(); await settle(); }
});
it('S7 fences late resolver and prepare replies after refresh, input invalidation and disposal', async () => {
  const pending = deferred<unknown>(); let delay = true;
  const t = setup(command => command === 'resolve_ref_action_target' && delay ? pending.promise : undefined); await t.start();
  const first = t.store.getState().prepare(draft()); const old = t.store.getState();
  await t.history.getState().refresh(); await settle();
  pending.resolve(ok({ ...fixture.resolved, scope: old.scope, catalogId: old.catalog!.catalogId })); await first;
  expect(t.store.getState().prepared).toBeNull(); expect(t.api.mock.calls.filter(([command]) => command === 'prepare_repository_action')).toHaveLength(0);
  delay = false; const prepared = deferred<unknown>();
  const implementation = t.api.getMockImplementation()!; t.api.mockImplementation(async (command, request) => command === 'prepare_repository_action' ? prepared.promise : implementation(command, request));
  const second = t.store.getState().prepare(draft()); await settle(); t.store.getState().clearPrepared(); t.stop(); await settle();
  prepared.resolve(ok({ scope: t.store.getState().scope, kind: 'branchCreate', planId: 'stale', warnings: [], expiresAt: '2099-01-01' })); await second;
  expect(t.store.getState().prepared).toBeNull(); expect(t.store.getState().scope).toBeNull();
});
it.each(['Working tree has changes', 'Branch is checked out in another worktree', 'Bare repository has no worktree'])('S7 shows authoritative preflight rejection: %s', async message => {
  const error: AppError = { code: 'Internal', phase: 'inspection', retry: 'none', message };
  const t = setup(command => command === 'prepare_repository_action' ? { ok: false, error } : undefined); await t.start();
  try { await t.store.getState().prepare(draft()); expect(t.store.getState().actionError).toEqual(error); expect(t.store.getState().prepared).toBeNull(); } finally { t.stop(); await settle(); }
});
it('S7 consumes preview once and refuses expired previews without retrying execution', async () => {
  const pending = deferred<unknown>(), t = setup(command => command === 'execute_repository_action' ? pending.promise : undefined); await t.start();
  try {
    await t.store.getState().prepare(draft()); const first = t.store.getState().execute(); expect(await t.store.getState().execute()).toBeNull();
    pending.resolve(ok(fixture.result.operation)); expect((await first)?.ok).toBe(true);
    expect(t.api.mock.calls.filter(([command]) => command === 'execute_repository_action')).toHaveLength(1);
    await t.store.getState().prepare(draft()); t.store.setState({ prepared: { ...t.store.getState().prepared!, expiresAt: '2000-01-01' } });
    expect(await t.store.getState().execute()).toBeNull(); expect(t.store.getState().actionError?.message).toMatch(/expired/);
  } finally { t.stop(); await settle(); }
});
