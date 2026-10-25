import { afterEach, expect, it, vi } from 'vitest';
import { createHistoryViewStore, defaultRepositoryView } from './historyView';
import type { Api } from './workspace';
import type { HistoryScope, RepositoryView } from '../contracts';
const a: HistoryScope = { repoId: 'a', worktreeId: 'tree', generation: '1' };
const ok = <T,>(value: T) => ({ ok: true as const, value });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
afterEach(() => vi.useRealTimers());

it.each(['active', 'old-scope'] as const)('S5-01 retains a newer ABA edit after an equal-value %s acknowledgement', async lifetime => {
  const remote = new Map([['a', { view: defaultRepositoryView(), revision: 1 }], ['b', { view: defaultRepositoryView(), revision: 1 }]]);
  const old = deferred<unknown>(); let acknowledgement: unknown, accepted = 0;
  const api = vi.fn(async (command: string, request: { scope: HistoryScope; view: RepositoryView; expectedRevision: string }) => {
    const entry = remote.get(request.scope.repoId)!;
    if (command === 'get_repository_view') return ok({ scope: request.scope, viewRevision: String(entry.revision), view: entry.view });
    if (request.expectedRevision !== String(entry.revision)) return { ok: false, error: { code: 'ExternalChange', phase: 'inspection', retry: 'retry', message: 'Preferences changed.' } };
    entry.view = request.view; entry.revision++;
    const result = ok({ scope: request.scope, viewRevision: String(entry.revision), view: entry.view });
    if (++accepted === 1) { acknowledgement = result; return old.promise; }
    return result;
  });
  const store = createHistoryViewStore(api as unknown as Api); await store.getState().activate(a);
  store.getState().patch({ sidebarWidth: 400 }); const flushing = store.getState().flush();
  expect(accepted).toBe(1); // Backend committed revision 2; its ack is delayed.
  const entry = remote.get('a')!; entry.view = { ...entry.view, sidebarWidth: 600, sidebarCollapsed: true }; entry.revision++;
  store.getState().patch({ sidebarWidth: 500 }); store.getState().patch({ sidebarWidth: 400 });
  let opening: Promise<RepositoryView | null> | undefined;
  if (lifetime === 'old-scope') { store.getState().deactivate(); opening = store.getState().activate({ ...a, repoId: 'b', generation: '2' }); }
  old.resolve(acknowledgement); await flushing; await opening;
  if (lifetime === 'old-scope') {
    store.getState().patch({ sidebarWidth: 430 }); await store.getState().flush();
    expect(remote.get('b')!.view.sidebarWidth).toBe(430);
    expect(remote.get('b')!.view.sidebarCollapsed).toBe(false);
    await store.getState().activate({ ...a, generation: '3' }); await store.getState().flush();
  }
  expect(store.getState().error).toBeNull(); expect(entry.view.sidebarWidth).toBe(400); expect(entry.view.sidebarCollapsed).toBe(true);
  expect(store.getState().view.sidebarWidth).toBe(400); expect(store.getState().view.sidebarCollapsed).toBe(true);
  if (lifetime === 'active') {
    expect(api.mock.calls.filter(([command]) => command === 'save_repository_view')).toHaveLength(3);
    expect(api.mock.calls.filter(([command]) => command === 'get_repository_view')).toHaveLength(2);
  }
  store.getState().deactivate();
});

it('ignores late hydration and sanitizes durable hints without storing runtime tokens or stash ordinals', async () => {
  const old = deferred<unknown>(), b = { ...a, repoId: 'b', generation: '2' };
  const api = vi.fn(async (command: string, request: { scope: HistoryScope }) => request.scope.repoId === 'a' ? old.promise : ok({ scope: request.scope, viewRevision: '4', view: { ...defaultRepositoryView(), selectedOid: 'A'.repeat(64), anchorOid: 'not-an-oid', filter: { hiddenRefs: ['refs/stash/2', 'refs/heads/main'], soloRefs: [] } } }));
  const store = createHistoryViewStore(api as unknown as Api), first = store.getState().activate(a);
  await Promise.resolve(); await store.getState().activate(b);
  old.resolve(ok({ scope: a, viewRevision: '0', view: defaultRepositoryView() })); await first;
  expect(store.getState().scope).toEqual(b);
  expect(store.getState().view.selectedOid).toBe('a'.repeat(64)); expect(store.getState().view.anchorOid).toBeNull();
  expect(store.getState().view.filter.hiddenRefs).toEqual(['refs/heads/main']);
  expect(Object.keys(store.getState().view).sort()).toEqual(Object.keys(defaultRepositoryView()).sort());
  store.getState().deactivate();
});

it('debounces current field intent and saves under the rebound monotonic history scope', async () => {
  vi.useFakeTimers();
  const api = vi.fn(async (command: string, request: { scope: HistoryScope; view: RepositoryView }) => ok({ scope: request.scope, viewRevision: command === 'get_repository_view' ? '7' : '8', view: request.view ?? defaultRepositoryView() }));
  const store = createHistoryViewStore(api as unknown as Api); await store.getState().activate(a);
  store.getState().patch({ sidebarWidth: 300 }); await vi.advanceTimersByTimeAsync(200);
  store.getState().patch({ sidebarWidth: 450, diffLayout: 'inline' });
  store.getState().bindScope({ ...a, generation: '2' });
  await vi.advanceTimersByTimeAsync(349); expect(api.mock.calls.filter(([command]) => command === 'save_repository_view')).toHaveLength(0);
  await vi.advanceTimersByTimeAsync(1);
  const saves = api.mock.calls.filter(([command]) => command === 'save_repository_view'); expect(saves).toHaveLength(1);
  expect(saves[0][1]).toEqual({ scope: { ...a, generation: '2' }, expectedRevision: '7', view: { ...defaultRepositoryView(), sidebarWidth: 450, diffLayout: 'inline' } });
  store.getState().deactivate();
});

it('merges CAS conflicts using only latest locally dirty fields, preserving newer remote fields', async () => {
  const saving = deferred<unknown>(), resync = deferred<unknown>(); let reads = 0, saves = 0;
  const api = vi.fn(async (command: string, request: { scope: HistoryScope; view: RepositoryView }) => {
    if (command === 'get_repository_view') return ++reads === 1 ? ok({ scope: request.scope, viewRevision: '1', view: defaultRepositoryView() }) : resync.promise;
    if (++saves === 1) return saving.promise;
    return ok({ scope: request.scope, viewRevision: '9', view: request.view });
  });
  const store = createHistoryViewStore(api as unknown as Api); await store.getState().activate(a);
  store.getState().patch({ detailsHeight: 400 }); const flush = store.getState().flush();
  store.getState().patch({ detailsHeight: 500 });
  saving.resolve({ ok: false, error: { code: 'ExternalChange', phase: 'inspection', retry: 'retry', message: 'Preferences changed.' } });
  await vi.waitFor(() => expect(reads).toBe(2));
  store.getState().patch({ detailsHeight: 600 });
  resync.resolve(ok({ scope: a, viewRevision: '8', view: { ...defaultRepositoryView(), sidebarCollapsed: true, sidebarWidth: 550, diffLayout: 'inline' } }));
  await flush;
  expect(api.mock.calls.filter(([command]) => command === 'save_repository_view')[1][1]).toEqual({ scope: a, expectedRevision: '8', view: { ...defaultRepositoryView(), detailsHeight: 600, sidebarCollapsed: true, sidebarWidth: 550, diffLayout: 'inline' } });
  expect(store.getState().view.detailsHeight).toBe(600); expect(store.getState().view.sidebarWidth).toBe(550);
  expect(store.getState().error).toBeNull(); store.getState().deactivate();
});

it('bounds repeated conflicts and preserves pending local intent for an explicit retry', async () => {
  const api = vi.fn(async (command: string, request: { scope: HistoryScope }) => command === 'get_repository_view' ? ok({ scope: request.scope, viewRevision: '3', view: defaultRepositoryView() }) : { ok: false, error: { code: 'ExternalChange', phase: 'inspection', retry: 'retry', message: 'Preferences changed.' } });
  const store = createHistoryViewStore(api as unknown as Api); await store.getState().activate(a);
  store.getState().patch({ diffLayout: 'inline' }); await store.getState().flush();
  expect(api.mock.calls.filter(([command]) => command === 'save_repository_view')).toHaveLength(2);
  expect(store.getState().view.diffLayout).toBe('inline'); expect(store.getState().error?.code).toBe('ExternalChange');
  store.getState().deactivate();
});

it('waits for an issued save across linked worktree activation without replaying acknowledged old intent', async () => {
  const saving = deferred<unknown>();
  const api = vi.fn(async (command: string, request: { scope: HistoryScope }) => command === 'save_repository_view' ? saving.promise : ok({ scope: request.scope, viewRevision: request.scope.generation, view: defaultRepositoryView() }));
  const store = createHistoryViewStore(api as unknown as Api); await store.getState().activate(a);
  store.getState().patch({ sidebarCollapsed: true }); const old = store.getState().flush();
  const next = { ...a, worktreeId: 'linked', generation: '2' }, opening = store.getState().activate(next);
  await Promise.resolve(); expect(api.mock.calls.filter(([command]) => command === 'get_repository_view')).toHaveLength(1);
  saving.resolve(ok({ scope: a, viewRevision: '10', view: { ...defaultRepositoryView(), sidebarCollapsed: true } }));
  await old; await opening;
  // A newer remote/window edit after the acknowledged old save wins because
  // the old local collapse intent is no longer pending.
  expect(store.getState().scope).toEqual(next); expect(store.getState().revision).toBe('2'); expect(store.getState().view.sidebarCollapsed).toBe(false);
  store.getState().deactivate();
});

it('retains unsaved A intent after background B activation, rebases on returning to A and survives a later restart', async () => {
  let active = 'a';
  const saved = new Map<string, { view: RepositoryView; revision: number }>([['a', { view: defaultRepositoryView(), revision: 0 }], ['b', { view: defaultRepositoryView(), revision: 0 }]]);
  const api = vi.fn(async (command: string, request: { scope: HistoryScope; view: RepositoryView; expectedRevision: string }) => {
    if (request.scope.repoId !== active) return { ok: false, error: { code: 'InvalidHandle', phase: 'validation', retry: 'retry', message: 'Repository scope changed.' } };
    const entry = saved.get(active)!;
    if (command === 'save_repository_view') {
      expect(request.expectedRevision).toBe(String(entry.revision)); entry.view = request.view; entry.revision++;
    }
    return ok({ scope: request.scope, viewRevision: String(entry.revision), view: entry.view });
  });
  const store = createHistoryViewStore(api as unknown as Api); await store.getState().activate(a);
  const intent = { filter: { hiddenRefs: ['refs/heads/topic'], soloRefs: [] }, anchorOid: 'a'.repeat(40), anchorOffset: 9 };
  store.getState().patch(intent);
  // Clone/background activation can precede effect cleanup. This old-scope
  // flush must fail safely and keep pending intent in the live window only.
  active = 'b'; store.getState().deactivate(); await store.getState().activate({ ...a, repoId: 'b', worktreeId: null, generation: '2' });
  expect(store.getState().view).toEqual(defaultRepositoryView());
  expect(saved.get('a')!.view).toEqual(defaultRepositoryView());
  store.getState().patch({ sidebarWidth: 410 }); await store.getState().flush();
  active = 'a'; await store.getState().activate({ ...a, generation: '3' });
  expect(store.getState().view).toMatchObject(intent); await store.getState().flush(); store.getState().deactivate();
  const reopened = createHistoryViewStore(api as unknown as Api); await reopened.getState().activate({ ...a, generation: '4' });
  expect(reopened.getState().view).toMatchObject(intent);
  expect(saved.get('b')!.view.filter).toEqual(defaultRepositoryView().filter); expect(saved.get('b')!.view.sidebarWidth).toBe(410);
  for (const [, request] of api.mock.calls.filter(([command]) => command === 'save_repository_view')) if (request.scope.repoId === 'b') expect(request.view.filter).toEqual(defaultRepositoryView().filter);
  reopened.getState().deactivate();
});
