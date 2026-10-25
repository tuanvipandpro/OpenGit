import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createWorkspaceStore } from './workspace';
import type { Api } from './workspace';
import { createHistoryStore } from './history';
import { defaultRepositoryView } from './historyView';
import fixture from '../../fixtures/contracts/wire.json';
import type { HistoryScope, RepositoryChangedEvent, RepositoryView, Workspace } from '../contracts';

const deferred = <T,>() => { let resolve!: (value: T) => void; const promise = new Promise<T>(yes => { resolve = yes; }); return { promise, resolve }; };
const ok = <T,>(value: T) => ({ ok: true as const, value });
const tick = async () => { for (let i = 0; i < 24; i++) await Promise.resolve(); };
let disposers: (() => void)[];
beforeEach(() => { vi.useFakeTimers(); disposers = []; });
afterEach(async () => { disposers.forEach(stop => stop()); await tick(); vi.useRealTimers(); vi.restoreAllMocks(); });
function service(selected = false) {
  const commit = { oid: "a".repeat(40), handle: "commit", parents: [], subject: "Selected", authorName: "Author", authorEmail: "author@example.test", authoredAt: "", committedAt: "", decorations: [] };
  let savedView = { ...defaultRepositoryView(), selectedOid: selected ? commit.oid : null };
  let workspace = fixture.commands[1].result.value as unknown as Workspace;
  let receive!: (event: RepositoryChangedEvent) => void;
  let inspectGate: ReturnType<typeof deferred<unknown>> | null = null, historyGate: ReturnType<typeof deferred<unknown>> | null = null, detailsGate: ReturnType<typeof deferred<unknown>> | null = null;
  let selectGate: ReturnType<typeof deferred<unknown>> | null = null, listenGate: ReturnType<typeof deferred<() => void>> | null = null;
  let failInspection = false; let forgetGate: ReturnType<typeof deferred<unknown>> | null = null;
  const stop = vi.fn(), subscribe = vi.fn(async (handler: typeof receive) => { receive = handler; return listenGate ? listenGate.promise : stop; });
  const contexts: { scope: HistoryScope; snapshotId: string }[] = [];
  const api = vi.fn(async (command: string, request: Record<string, unknown>) => {
    if (command === 'get_workspace') return ok(workspace);
    if (command === 'get_environment') return fixture.commands[0].result;
    if (command === 'get_repository_view' || command === 'save_repository_view') { if (command === 'save_repository_view') savedView = request.view as RepositoryView; return ok({ scope: request.scope, viewRevision: '9007199254740993', view: savedView }); }
    if (command === 'inspect_repository') return inspectGate ? inspectGate.promise : failInspection ? fixture.error : ok({ ...fixture.snapshots[0], ...request });
    if (command === 'select_repository') { workspace = { ...workspace, revision: String(BigInt(workspace.revision) + 1n), activeSelection: request as Workspace['activeSelection'] }; return selectGate ? selectGate.promise : ok(workspace); }
    if (command === 'forget_repository') { workspace = { ...workspace, revision: String(BigInt(workspace.revision) + 1n), activeSelection: null }; return forgetGate ? forgetGate.promise : ok(workspace); }
    if (command === 'open_history') {
      const context = { scope: request.scope as HistoryScope, snapshotId: `snapshot-${contexts.length}` }; contexts.push(context);
      return historyGate ? historyGate.promise : ok({ context, sessionId: context.snapshotId, refs: [], roots: [], head: { kind: 'unborn', oid: null, branch: 'main' }, page: { context, sessionId: context.snapshotId, commits: selected ? [commit] : [], nextCursor: null } });
    }
    if (command === 'get_commit_details') return detailsGate ? detailsGate.promise : ok({ context: request.context, commit, base: request.base, files: [], message: '', committerName: '', committerEmail: '' });
    if (command === 'watch_repository') return ok({ scope: request.scope, subscriptionId: `watch-${(request.scope as HistoryScope).generation}`, sequence: '0', status: 'fallback' });
    if (command === 'unwatch_repository') return ok(null);
    throw Error(`Unexpected ${command}`);
  }) as unknown as Api;
  const history = createHistoryStore(api), store = createWorkspaceStore(api, subscribe);
  disposers.push(store.getState().connectHistory(history));
  const count = (command: string) => vi.mocked(api).mock.calls.filter(([name]) => name === command).length;
  return { api, history, store, contexts, subscribe, stop, count,
    inspect: (gate: typeof inspectGate) => { inspectGate = gate; }, opening: (gate: typeof historyGate) => { historyGate = gate; }, details: (gate: typeof detailsGate) => { detailsGate = gate; }, selecting: (gate: typeof selectGate) => { selectGate = gate; },
    forgetting: (gate: typeof forgetGate) => { forgetGate = gate; }, listen: (gate: typeof listenGate) => { listenGate = gate; }, fail: () => { failInspection = true; },
    currentWorkspace: () => workspace,
    emit: (sequence: string, reason: RepositoryChangedEvent['reason'] = 'changed') => { const scope = history.getState().preferences.getState().scope!; receive({ scope, subscriptionId: `watch-${scope.generation}`, sequence, reason }); },
  };
}
it('awaits inspection then history for a whole run and coalesces signals into one pending rerun', async () => {
  const h = service(); await h.store.getState().initialize();
  const inspecting = deferred<unknown>(), opening = deferred<unknown>(); h.inspect(inspecting); h.opening(opening);
  const work = h.store.getState().requestRefresh('watch');
  void h.store.getState().requestRefresh('focus'); void h.store.getState().requestRefresh('watch');
  expect(h.count('inspect_repository')).toBe(2); expect(h.count('open_history')).toBe(1);
  await vi.advanceTimersByTimeAsync(499); expect(h.store.getState().refreshing).toBe(false);
  const request = vi.mocked(h.api).mock.calls.filter(([name]) => name === 'inspect_repository').at(-1)![1];
  inspecting.resolve(ok({ ...fixture.snapshots[0], ...request })); await tick();
  expect(h.count('open_history')).toBe(2); expect(h.count('inspect_repository')).toBe(2);
  await vi.advanceTimersByTimeAsync(1); expect(h.store.getState().refreshing).toBe(true);
  h.inspect(null); h.opening(null);
  opening.resolve({ ok: false, error: fixture.error.error }); await work;
  expect(h.count('inspect_repository')).toBe(3); expect(h.count('open_history')).toBe(3); expect(h.store.getState().refreshing).toBe(false);
});
it('fences the old repository before a delayed selection response, then activates only the final linked selection', async () => {
  const h = service(); await h.store.getState().initialize();
  const inspection = deferred<unknown>(), selection = deferred<unknown>(); h.inspect(inspection); h.selecting(selection);
  const old = h.store.getState().requestRefresh('watch');
  const request = vi.mocked(h.api).mock.calls.filter(([name]) => name === 'inspect_repository').at(-1)![1];
  const switching = h.store.getState().select({ repoId: 'other', worktreeId: 'linked' }); await tick();
  expect(h.history.getState().scope).toBeNull();
  inspection.resolve(ok({ ...fixture.snapshots[0], ...request })); await old;
  expect(h.count('open_history')).toBe(1);
  h.inspect(null); selection.resolve(ok(h.currentWorkspace())); await switching;
  expect(h.history.getState().scope).toMatchObject({ repoId: 'other', worktreeId: 'linked' });
  expect(h.history.getState().preferences.getState().scope).toEqual(h.history.getState().scope);
  expect(h.count('open_history')).toBe(2);
});
it('invalidates an already pending history response when selection begins', async () => {
  const h = service(); await h.store.getState().initialize(); const opening = deferred<unknown>(); h.opening(opening);
  const old = h.store.getState().refresh(); await tick();
  const switching = h.store.getState().select({ repoId: 'new', worktreeId: null }); await tick();
  expect(h.history.getState().session).toBeNull(); h.opening(null);
  opening.resolve({ ok: false, error: fixture.error.error }); await Promise.all([old, switching]);
  expect(h.history.getState().session?.context.scope.repoId).toBe('new'); expect(h.history.getState().error).toBeNull();
});
it('allows current history after failed inspection and pending listen never blocks manual/fallback refresh', async () => {
  const h = service(), listener = deferred<() => void>(); h.listen(listener); h.fail();
  await h.store.getState().initialize(); expect(h.history.getState().session).not.toBeNull(); expect(h.store.getState().inspectionError).not.toBeNull(); expect(h.count('watch_repository')).toBe(0);
  await h.store.getState().refresh(); await vi.advanceTimersByTimeAsync(30_000);
  expect(h.count('open_history')).toBe(3); expect(h.subscribe).toHaveBeenCalledOnce();
  listener.resolve(h.stop); await tick(); expect(h.count('watch_repository')).toBe(1);
});
it('marks hidden hints dirty, resumes once, and prevents fallback→unavailable rebind loops', async () => {
  const h = service(); await h.store.getState().initialize(); await tick();
  vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden'); document.dispatchEvent(new Event('visibilitychange'));
  h.emit('1'); h.emit('2'); await vi.advanceTimersByTimeAsync(60_000); expect(h.count('inspect_repository')).toBe(1);
  vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible'); document.dispatchEvent(new Event('visibilitychange')); await tick();
  expect(h.count('inspect_repository')).toBe(2);
  h.emit('1', 'unavailable'); await tick(); expect(h.count('inspect_repository')).toBe(3);
  h.emit('1', 'unavailable'); await tick(); expect(h.count('inspect_repository')).toBe(3);
  h.emit('2', 'changed'); await tick(); expect(h.count('inspect_repository')).toBe(4);
  h.emit('1', 'unavailable'); await tick(); expect(h.count('inspect_repository')).toBe(5);
});

it('keeps manual feedback and the single flight active through real inspector restoration', async () => {
  const h = service(true); await h.store.getState().initialize();
  expect(h.history.getState().selected?.commit.oid).toBe('a'.repeat(40));
  const details = deferred<unknown>(); h.details(details);
  const work = h.store.getState().refresh(); await tick();
  expect(h.store.getState().refreshing).toBe(true); expect(h.count('get_commit_details')).toBe(2);
  void h.store.getState().requestRefresh('watch'); await tick(); expect(h.count('inspect_repository')).toBe(2);
  h.details(null); details.resolve({ ok: false, error: fixture.error.error }); await work;
  expect(h.count('inspect_repository')).toBe(3); expect(h.count('get_commit_details')).toBe(3);
  expect(h.store.getState().refreshing).toBe(false);
});

it('blocks stale reads and watchers while active forget waits, and recovers the original selection after failure', async () => {
  const h = service(); await h.store.getState().initialize(); const forgetting = deferred<unknown>(); h.forgetting(forgetting);
  const work = h.store.getState().forget(h.store.getState().workspace!.activeSelection!.repoId); await tick();
  expect(h.history.getState().scope).toBeNull();
  await h.store.getState().refresh(); await vi.advanceTimersByTimeAsync(30_000);
  expect(h.count('inspect_repository')).toBe(1); expect(h.count('watch_repository')).toBe(1);
  forgetting.resolve({ ok: false, error: fixture.error.error }); await work;
  expect(h.history.getState().session).not.toBeNull(); expect(h.count('inspect_repository')).toBe(2); expect(h.store.getState().busy).toBe(false);
  h.forgetting(null); await h.store.getState().forget(h.store.getState().workspace!.activeSelection!.repoId);
  expect(h.history.getState().scope).toBeNull(); expect(h.store.getState().workspace?.activeSelection).toBeNull();
});

it('S6-F01 restores current repository preferences/history/watch immediately after a failed selection', async () => {
  const h = service(); await h.store.getState().initialize();
  h.history.getState().preferences.getState().patch({ sidebarWidth: 450 }); await h.history.getState().preferences.getState().flush();
  const before = h.store.getState().workspace!.activeSelection!, selection = deferred<unknown>(); h.selecting(selection);
  const switching = h.store.getState().select({ repoId: 'rejected', worktreeId: null }); await tick();
  expect(h.history.getState().scope).toBeNull();
  selection.resolve({ ok: false, error: fixture.error.error }); await switching;
  expect(h.history.getState().session?.context.scope.repoId).toBe(before.repoId);
  expect(h.history.getState().preferences.getState().view.sidebarWidth).toBe(450);
  expect(h.store.getState().busy).toBe(false); expect(h.store.getState().error).toEqual(fixture.error.error);
  expect(h.count('watch_repository')).toBe(2);
  expect(vi.mocked(h.api).mock.calls.filter(([name]) => name === 'watch_repository').map(([, request]) => ((request as { scope: HistoryScope }).scope).repoId)).toEqual([before.repoId, before.repoId]);
});

it('S3-FORM01 selection-owned lease invalidates immediately for worktree switches, ABA, forget and disposal', async () => {
  const h = service(); await h.store.getState().initialize();
  const initial = h.currentWorkspace().activeSelection!, first = h.store.getState().beginInteraction()!;
  expect(await first.ready).toBe(true); const initialReads = h.count('inspect_repository');
  await h.store.getState().requestRefresh('watch'); expect(h.count('inspect_repository')).toBe(initialReads);
  h.emit('1'); await vi.advanceTimersByTimeAsync(250); expect(h.count('inspect_repository')).toBe(initialReads);
  const selecting = h.store.getState().select({ repoId: initial.repoId, worktreeId: 'different-linked-tree' });
  expect(first.isCurrent()).toBe(false); await first.closed; await selecting;
  await h.store.getState().select(initial); const second = h.store.getState().beginInteraction()!;
  expect(await second.ready).toBe(true); first.release(); expect(second.isCurrent()).toBe(true);
  const reads = h.count('inspect_repository'); await h.store.getState().requestRefresh('watch');
  const forget = h.store.getState().forget(initial.repoId); expect(second.isCurrent()).toBe(false); await forget;
  second.release(); expect(h.count('inspect_repository')).toBe(reads);
});
it('S3-FORM01 last workspace owner disposal cancels its hold without flushing old-repo reads', async () => {
  const h = service(); await h.store.getState().initialize();
  const lease = h.store.getState().beginInteraction()!; await lease.ready;
  const reads = h.count('inspect_repository'); await h.store.getState().requestRefresh('watch');
  disposers[0](); await tick(); expect(lease.isCurrent()).toBe(false); await lease.closed;
  lease.release(); expect(h.count('inspect_repository')).toBe(reads);
});
