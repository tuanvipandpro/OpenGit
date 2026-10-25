import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRefreshCoordinator } from './refresh';
import { createWorkspaceStore } from '../../stores/workspace';
import type { Api } from '../../stores/workspace';
import type { RepositorySnapshot, Workspace } from '../../contracts';
import fixture from '../../../fixtures/contracts/wire.json';

const deferred = <T,>() => { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; };
let cleanups: (() => void)[];
beforeEach(() => { vi.useFakeTimers(); cleanups = []; });
afterEach(() => { cleanups.forEach(stop => stop()); vi.useRealTimers(); vi.restoreAllMocks(); });
function harness() {
  let visible = true, scope: string | null = 'repo-a';
  const work: ReturnType<typeof deferred<void>>[] = [], feedback = vi.fn();
  const run = vi.fn(() => { const item = deferred<void>(); work.push(item); return item.promise; });
  const refresh = createRefreshCoordinator({ scope: () => scope, run, feedback, visible: () => visible });
  cleanups.push(refresh.start());
  return { refresh, run, feedback, work, hide: () => { visible = false; document.dispatchEvent(new Event('visibilitychange')); }, show: () => { visible = true; document.dispatchEvent(new Event('visibilitychange')); }, select: (next: string | null) => { scope = next; } };
}
it('uses a quiet 30-second fallback, pauses while hidden, and refreshes once on resume', async () => {
  const h = harness();
  await vi.advanceTimersByTimeAsync(29_999); expect(h.run).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1); expect(h.run).toHaveBeenCalledTimes(1); expect(h.feedback).not.toHaveBeenCalled();
  h.work[0].resolve(); await vi.advanceTimersByTimeAsync(0);
  h.hide(); await vi.advanceTimersByTimeAsync(90_000); expect(h.run).toHaveBeenCalledTimes(1);
  h.show(); h.show(); expect(h.run).toHaveBeenCalledTimes(2);
  h.work[1].resolve(); await vi.advanceTimersByTimeAsync(0);
});
it('debounces focus bursts and coalesces operation/watch/fallback signals into one rerun', async () => {
  const h = harness();
  window.dispatchEvent(new Event('focus')); await vi.advanceTimersByTimeAsync(100);
  window.dispatchEvent(new Event('focus')); await vi.advanceTimersByTimeAsync(149); expect(h.run).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1); expect(h.run).toHaveBeenCalledTimes(1);
  void h.refresh.request('operation'); void h.refresh.request('watch'); void h.refresh.request('fallback');
  expect(h.run).toHaveBeenCalledTimes(1);
  h.work[0].resolve(); await vi.advanceTimersByTimeAsync(0); expect(h.run).toHaveBeenCalledTimes(2);
  h.work[1].resolve(); await vi.advanceTimersByTimeAsync(0); expect(h.run).toHaveBeenCalledTimes(2);
});
it('shows manual feedback immediately and slow automatic feedback only at 500ms', async () => {
  const h = harness();
  const first = h.refresh.request('watch');
  await vi.advanceTimersByTimeAsync(499); expect(h.feedback).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1); expect(h.feedback).toHaveBeenLastCalledWith('repo-a', true);
  h.work[0].resolve(); await first; expect(h.feedback).toHaveBeenLastCalledWith('repo-a', false);
  const manual = h.refresh.request('manual'); expect(h.feedback).toHaveBeenLastCalledWith('repo-a', true);
  h.work[1].resolve(); await manual; expect(h.feedback).toHaveBeenLastCalledWith('repo-a', false);
});
it('defers a queued automatic rerun while hidden and cancels focus/timers on disposal', async () => {
  const h = harness(); const first = h.refresh.request('watch'); void h.refresh.request('watch');
  h.hide(); h.work[0].resolve(); await first; expect(h.run).toHaveBeenCalledTimes(1);
  h.show(); expect(h.run).toHaveBeenCalledTimes(2); h.work[1].resolve(); await vi.advanceTimersByTimeAsync(0);
  window.dispatchEvent(new Event('focus')); cleanups.forEach(stop => stop()); cleanups = [];
  await vi.advanceTimersByTimeAsync(90_000); expect(h.run).toHaveBeenCalledTimes(2);
});
it('does not start reads without an active repository or show feedback for a stale scope', async () => {
  const h = harness(); void h.refresh.request('watch'); h.select(null);
  await vi.advanceTimersByTimeAsync(600); expect(h.feedback).not.toHaveBeenCalled();
  h.work[0].resolve(); await vi.advanceTimersByTimeAsync(0);
  await h.refresh.request('manual'); expect(h.run).toHaveBeenCalledTimes(1);
});
it('preserves content and recovers after a service failure; late old-repo responses cannot replace selection', async () => {
  const workspace = fixture.commands[1].result.value as unknown as Workspace;
  let current = workspace, delayed: ReturnType<typeof deferred<unknown>> | null = null;
  let fail = false;
  const api = vi.fn(async (command: string, request: Record<string, unknown>) => {
    if (command === 'get_environment') return fixture.commands[0].result;
    if (command === 'get_workspace') return { ok: true, value: current };
    if (command === 'select_repository') { current = { ...workspace, revision: '9007199254740994', activeSelection: request as Workspace['activeSelection'] }; return { ok: true, value: current }; }
    if (command === 'inspect_repository') {
      if (delayed) { const item = delayed; delayed = null; return item.promise; }
      if (fail) return fixture.error;
      return { ok: true, value: { ...fixture.snapshots[0], ...request } };
    }
    throw Error('unexpected command');
  }) as unknown as Api;
  const store = createWorkspaceStore(api); await store.getState().initialize(); cleanups.push(store.getState().startRefresh());
  const previous = store.getState().snapshot; fail = true;
  await store.getState().requestRefresh('watch'); expect(store.getState().snapshot).toBe(previous); expect(store.getState().inspectionError).not.toBeNull();
  fail = false; delayed = deferred<unknown>(); const old = delayed;
  const refresh = store.getState().requestRefresh('watch');
  const request = vi.mocked(api).mock.calls.at(-1)![1];
  const selecting = store.getState().select({ repoId: 'other-repo', worktreeId: 'other-tree' });
  await vi.advanceTimersByTimeAsync(0);
  old.resolve({ ok: true, value: { ...fixture.snapshots[0], ...request } });
  await Promise.all([refresh, selecting]);
  expect(store.getState().snapshot?.repoId).toBe('other-repo'); expect(store.getState().inspectionError).toBeNull();
  expect(store.getState().refreshing).toBe(false);
  expect((store.getState().snapshot as RepositorySnapshot).generation).not.toBe((request as { generation: string }).generation);
});

it('S3-FORM01 interaction holds repeated passive ticks, flushes once and does not block manual/operation reads', async () => {
  const h = harness();
  const lease = h.refresh.acquireInteraction()!; expect(await lease.ready).toBe(true);
  await vi.advanceTimersByTimeAsync(60_000); window.dispatchEvent(new Event('focus')); await vi.advanceTimersByTimeAsync(150);
  void h.refresh.request('watch'); expect(h.run).not.toHaveBeenCalled();
  const manual = h.refresh.request('manual'); expect(h.run).toHaveBeenCalledTimes(1); h.work[0].resolve(); await manual;
  const operation = h.refresh.request('operation'); expect(h.run).toHaveBeenCalledTimes(2); h.work[1].resolve(); await operation;
  lease.release(); lease.release(); expect(h.run).toHaveBeenCalledTimes(3); h.work[2].resolve(); await vi.advanceTimersByTimeAsync(0);
  expect(h.run).toHaveBeenCalledTimes(3);
});
it('S3-FORM01 readiness waits for an existing quiet run and holds its queued passive rerun', async () => {
  const h = harness(), running = h.refresh.request('watch'); void h.refresh.request('focus');
  const lease = h.refresh.acquireInteraction()!; let ready = false; void lease.ready.then(value => { ready = value; });
  await vi.advanceTimersByTimeAsync(0); expect(ready).toBe(false); expect(h.feedback).not.toHaveBeenCalled();
  h.work[0].resolve(); await running; expect(await lease.ready).toBe(true); expect(h.run).toHaveBeenCalledTimes(1);
  lease.release(); expect(h.run).toHaveBeenCalledTimes(2); h.work[1].resolve(); await vi.advanceTimersByTimeAsync(0);
});

it('S3-FORM01 release while hidden defers once until resume and cannot release a newer owner', async () => {
  const h = harness(), first = h.refresh.acquireInteraction()!; expect(await first.ready).toBe(true);
  void h.refresh.request('watch'); h.hide(); first.release(); expect(h.run).not.toHaveBeenCalled();
  h.show(); expect(h.run).toHaveBeenCalledTimes(1); h.work[0].resolve(); await vi.advanceTimersByTimeAsync(0);
  const current = h.refresh.acquireInteraction()!; first.release(); expect(current.isCurrent()).toBe(true);
  void h.refresh.request('fallback'); current.release(); expect(h.run).toHaveBeenCalledTimes(2); h.work[1].resolve(); await vi.advanceTimersByTimeAsync(0);
});
it('S3-FORM01 cancels readiness immediately across selection/ABA/disposal without reviving an old lease', async () => {
  const h = harness(), oldRun = h.refresh.request('watch'), old = h.refresh.acquireInteraction()!;
  h.refresh.invalidateInteraction(); expect(await old.ready).toBe(false);
  h.select('repo-b'); void h.refresh.request('operation'); h.select('repo-a');
  const current = h.refresh.acquireInteraction()!; old.release(); expect(current.isCurrent()).toBe(true);
  h.work[0].resolve(); await vi.advanceTimersByTimeAsync(0); expect(h.run).toHaveBeenCalledTimes(2);
  h.work[1].resolve(); await oldRun; expect(await current.ready).toBe(true);
  cleanups.forEach(stop => stop()); cleanups = []; expect(current.isCurrent()).toBe(false);
  void h.refresh.request('watch'); expect(h.run).toHaveBeenCalledTimes(3); h.work[2].resolve();
});
it('S3-FORM01 preserves an urgent queued rerun while acquiring during a quiet refresh', async () => {
  const h = harness(), running = h.refresh.request('watch'); void h.refresh.request('operation');
  const lease = h.refresh.acquireInteraction()!; h.work[0].resolve(); await vi.advanceTimersByTimeAsync(0);
  expect(h.run).toHaveBeenCalledTimes(2); let ready = false; void lease.ready.then(value => { ready = value; });
  expect(ready).toBe(false); h.work[1].resolve(); await running; expect(await lease.ready).toBe(true); lease.release();
});

it('S3-FORM01 preserves hidden operation intent through resume while the interaction remains open', async () => {
  const h = harness(), lease = h.refresh.acquireInteraction()!; await lease.ready;
  h.hide(); await h.refresh.request('operation'); expect(h.run).not.toHaveBeenCalled();
  h.show(); expect(h.run).toHaveBeenCalledTimes(1); expect(lease.isCurrent()).toBe(true);
  h.work[0].resolve(); await vi.advanceTimersByTimeAsync(0); lease.release(); expect(h.run).toHaveBeenCalledTimes(1);
});
