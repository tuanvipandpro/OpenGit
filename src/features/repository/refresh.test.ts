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
