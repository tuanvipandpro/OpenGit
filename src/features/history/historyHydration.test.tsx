import { StrictMode, useEffect } from 'react';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createHistoryStore } from '../../stores/history';
import { defaultRepositoryView } from '../../stores/historyView';
import { createWorkspaceStore } from '../../stores/workspace';
import type { Api } from '../../stores/workspace';
import workspaceFixture from '../../../fixtures/contracts/wire.json';
import type { CommitSummary, HistoryContext, HistoryScope, RepositoryView, HistorySession } from '../../contracts';
import { HistoryScreen } from './HistoryScreen';

const tip = 'a'.repeat(40), deep = 'b'.repeat(40), missing = 'f'.repeat(40), selection = { repoId: 'repo', worktreeId: 'tree' };
const ok = <T,>(value: T) => ({ ok: true as const, value });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
const commit = (oid: string): CommitSummary => ({ oid, handle: `handle-${oid}`, parents: [], subject: oid === tip ? 'Tip commit' : 'Deep commit', authorName: 'Author', authorEmail: 'author@example.test', authoredAt: '', committedAt: '', decorations: [] });
const originalScrollTo = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'scrollTo');
beforeEach(() => {
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(640);
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(240);
  vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockImplementation(function (this: HTMLElement) { return Number.parseFloat((this.firstElementChild as HTMLElement | null)?.style.height ?? '0') || 0; });
  vi.spyOn(HTMLElement.prototype, 'scrollWidth', 'get').mockImplementation(function (this: HTMLElement) { return Number.parseFloat((this.firstElementChild as HTMLElement | null)?.style.width ?? '0') || 0; });
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
  Object.defineProperty(HTMLElement.prototype, 'scrollTo', { configurable: true, value: vi.fn(function (this: HTMLElement, options: ScrollToOptions) { this.scrollTop = options.top ?? 0; }) });
});
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); if (originalScrollTo) Object.defineProperty(HTMLElement.prototype, 'scrollTo', originalScrollTo); else delete (HTMLElement.prototype as unknown as { scrollTo?: unknown }).scrollTo; });

function service(saved: RepositoryView = defaultRepositoryView(), extra?: (command: string, request: Record<string, unknown>) => unknown) {
  let active: HistorySession;
  const api = vi.fn(async (command: string, request: Record<string, unknown>) => {
    const result = extra?.(command, request); if (result !== undefined) return result;
    if (command === 'get_workspace') return ok({ ...workspaceFixture.commands[1].result.value, activeSelection: selection });
    if (command === 'get_environment') return workspaceFixture.commands[0].result;
    if (command === 'inspect_repository') return ok({ ...workspaceFixture.snapshots[0], ...request });
    if (command === 'watch_repository') return ok({ scope: request.scope, subscriptionId: 'watch', sequence: '0', status: 'watching' });
    if (command === 'unwatch_repository') return ok(null);
    if (command === 'get_repository_view' || command === 'save_repository_view') return ok({ scope: request.scope, viewRevision: '1', view: request.view ?? saved });
    if (command === 'open_history') {
      const context = { scope: request.scope as HistoryScope, snapshotId: `snapshot-${(request.scope as HistoryScope).generation}` };
      active = { context, sessionId: 'session', refs: [{ name: 'refs/heads/main', displayName: 'main', kind: 'local', oid: tip, commitOid: tip, hidden: false, symbolicTarget: null }], head: { kind: 'branch', branch: 'main', oid: tip }, roots: [tip], page: { context, sessionId: 'session', commits: [commit(tip), ...Array.from({ length: 12 }, (_, index) => commit((index + 1).toString(16).padStart(40, '0')))], nextCursor: 'page2' } };
      return ok(active);
    }
    if (command === 'search_history') return ok({ context: request.context as HistoryContext, sessionId: request.sessionId, query: request.query, commits: request.query === missing ? [] : [{ commit: commit(request.query as string), outsideView: false }], refs: [], nextCursor: null });
    if (command === 'load_history_page') return ok({ context: active.context, sessionId: active.sessionId, commits: [commit(deep)], nextCursor: null });
    if (command === 'get_commit_details') return ok({ context: request.context, commit: commit((request.commitHandle as string).slice('handle-'.length)), base: request.base, message: 'Restored commit message', committerName: 'Committer', committerEmail: 'committer@example.test', files: [] });
    throw Error('Unexpected command');
  });
  const store = createHistoryStore(api as unknown as Api), workspace = createWorkspaceStore(api as unknown as Api, async () => () => {});
  function Owner() { useEffect(() => { const stop = workspace.getState().connectHistory(store); void workspace.getState().initialize(); return stop; }, []); return <HistoryScreen store={store} selection={selection} />; }
  return { api, store, workspace, Owner };
}

it('hydrates before mounting rows or opening history and keeps a restored anchor ahead of a deep selected hint', async () => {
  const hydration = deferred<unknown>();
  const saved = { ...defaultRepositoryView(), selectedOid: deep, anchorOid: tip, anchorOffset: 7, filter: { hiddenRefs: ['refs/heads/deleted'], soloRefs: [] } };
  const { api, store, Owner } = service(saved, (command, request) => command === 'get_repository_view' ? hydration.promise : undefined);
  const view = render(<Owner />);
  await waitFor(() => expect(api).toHaveBeenCalledWith('get_repository_view', expect.anything()));
  expect(screen.getByRole('status')).toHaveTextContent('Restoring repository view');
  expect(screen.queryByRole('grid', { name: 'Commits' })).not.toBeInTheDocument(); expect(api.mock.calls.filter(([command]) => command === 'open_history')).toHaveLength(0);
  const scope = api.mock.calls.find(([command]) => command === 'get_repository_view')![1].scope;
  await act(async () => hydration.resolve(ok({ scope, viewRevision: '3', view: saved })));
  await waitFor(() => expect(store.getState().selected?.commit.oid).toBe(deep));
  expect(api).toHaveBeenCalledWith('open_history', { scope, filter: saved.filter, pageSize: null });
  expect(api.mock.calls.filter(([command]) => command === 'load_history_page')).toHaveLength(0);
  expect(screen.getByRole('grid', { name: 'Commits' }).scrollTop).toBe(7);
  expect(store.getState().filter.hiddenRefs).toEqual([]);
  expect(within(screen.getByRole('region', { name: 'Selected commit' })).getByText(deep)).toBeInTheDocument();
  expect(store.getState().preferences.getState().view.selectedOid).toBe(deep);
  view.unmount();
});

it('rejects out-of-format persisted OIDs without extra pages/search and falls back from a missing anchor', async () => {
  const mismatch = service({ ...defaultRepositoryView(), selectedOid: 'c'.repeat(64), anchorOid: 'c'.repeat(64) });
  await mismatch.store.getState().activate(selection);
  expect(mismatch.api.mock.calls.filter(([command]) => command === 'load_history_page' || command === 'search_history')).toHaveLength(0);
  expect(mismatch.store.getState().anchor).toBeNull(); expect(mismatch.store.getState().selected).toBeNull();
  await mismatch.store.getState().refresh(); expect(mismatch.api.mock.calls.filter(([command]) => command === 'search_history')).toHaveLength(0);
  mismatch.store.getState().deactivate();
  const lost = service({ ...defaultRepositoryView(), anchorOid: missing, selectedOid: missing });
  await lost.store.getState().activate(selection);
  expect(lost.store.getState().anchor).toBeNull(); expect(lost.store.getState().selected).toBeNull();
  expect(lost.store.getState().preferences.getState().view.anchorOid).toBeNull(); expect(lost.store.getState().preferences.getState().view.selectedOid).toBeNull();
  expect(lost.api.mock.calls.filter(([command]) => command === 'load_history_page')).toHaveLength(1);
  lost.store.getState().deactivate();
});

it('uses one monotonic scope for StrictMode hydration, refresh and subsequent preference saves', async () => {
  const { api, store, workspace, Owner } = service(); const setup = vi.fn();
  function Probe() { useEffect(() => { setup(); }, []); return null; }
  const view = render(<StrictMode><Probe /><Owner /></StrictMode>);
  expect(setup).toHaveBeenCalledTimes(2);
  await screen.findByRole('grid', { name: 'Commits' });
  const opening = api.mock.calls.filter(([command]) => command === 'open_history')[0][1].scope as HistoryScope;
  expect(api.mock.calls.filter(([command]) => command === 'get_repository_view').some(([, request]) => (request.scope as HistoryScope).generation === opening.generation)).toBe(true);
  await act(async () => workspace.getState().requestRefresh('focus'));
  await waitFor(() => expect(api.mock.calls.filter(([command]) => command === 'open_history')).toHaveLength(2));
  const fresh = api.mock.calls.filter(([command]) => command === 'open_history')[1][1].scope as HistoryScope;
  expect(BigInt(fresh.generation)).toBeGreaterThan(BigInt(opening.generation));
  fireEvent.change(screen.getByRole('slider', { name: 'Details height' }), { target: { value: '400' } });
  await act(async () => store.getState().preferences.getState().flush());
  expect(api.mock.calls.filter(([command]) => command === 'save_repository_view').at(-1)![1].scope).toEqual(fresh);
  view.unmount();
});

it('preserves remote CAS filter/selection through local anchor/layout changes and quiet handle rebinds', async () => {
  let remote: RepositoryView = { ...defaultRepositoryView(), selectedOid: tip }; let revision = 1, conflict = false;
  const { api, store } = service(remote, (command, request) => {
    if (command === 'get_repository_view') return ok({ scope: request.scope, viewRevision: String(revision), view: remote });
    if (command === 'save_repository_view') {
      if (conflict) { conflict = false; remote = { ...remote, filter: { hiddenRefs: [], soloRefs: ['refs/heads/main'] }, selectedOid: deep }; revision++;
        return { ok: false, error: { code: 'ExternalChange', phase: 'inspection', retry: 'retry', message: 'Preferences changed.' } }; }
      expect(request.expectedRevision).toBe(String(revision)); remote = request.view as RepositoryView; revision++;
      return ok({ scope: request.scope, viewRevision: String(revision), view: remote });
    }
  });
  await store.getState().activate(selection);
  expect(store.getState().selected?.commit.oid).toBe(tip);
  conflict = true; store.getState().preferences.getState().patch({ detailsHeight: 450 }); await store.getState().preferences.getState().flush();
  expect(remote.selectedOid).toBe(deep); expect(remote.filter.soloRefs).toEqual(['refs/heads/main']);
  // Cross-window durable hints are merged without reopening this window's
  // graph. Local scroll/layout and automatic refresh cannot rewrite them.
  expect(store.getState().selected?.commit.oid).toBe(tip); expect(store.getState().filter.soloRefs).toEqual([]);
  store.getState().setAnchor({ oid: tip, offset: 6 }); store.getState().preferences.getState().patch({ sidebarWidth: 420 });
  await store.getState().refresh(); await store.getState().preferences.getState().flush();
  expect(remote.selectedOid).toBe(deep); expect(remote.filter.soloRefs).toEqual(['refs/heads/main']); expect(remote.anchorOffset).toBe(6); expect(remote.sidebarWidth).toBe(420);
  expect(api.mock.calls.filter(([command]) => command === 'get_commit_details')).toHaveLength(2);
  // A subsequent explicit user selection/filter is newer intent and may win.
  store.getState().selectCommit(tip); await store.getState().setFilter({ hiddenRefs: ['refs/heads/main'], soloRefs: [] }); await store.getState().preferences.getState().flush();
  expect(remote.selectedOid).toBe(tip); expect(remote.filter.hiddenRefs).toEqual(['refs/heads/main']);
  store.getState().deactivate();
});
