import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { HistoryScreen } from './HistoryScreen';
import { createHistoryStore } from '../../stores/history';
import type { Api } from '../../stores/workspace';
import type { CommitSummary, HistorySession, HistoryScope, HistoryContext } from '../../contracts';
import { defaultRepositoryView } from '../../stores/historyView';

const originalScrollTo = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'scrollTo');
beforeEach(() => {
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(640);
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(320);
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
  Object.defineProperty(HTMLElement.prototype, 'scrollTo', { configurable: true, value: vi.fn() });
});
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); if (originalScrollTo) Object.defineProperty(HTMLElement.prototype, 'scrollTo', originalScrollTo); else delete (HTMLElement.prototype as unknown as { scrollTo?: unknown }).scrollTo; });
const commit = (oid: string, id: string): CommitSummary => ({ oid, handle: `${id}:${oid}`, subject: oid === 'outside' ? 'Deep history result' : 'Visible commit', parents: [], authorName: 'Full Author', authorEmail: 'author@example.test', authoredAt: '2000-01-01T00:00:00Z', committedAt: '2000-01-01T00:00:00Z', decorations: ['refs/heads/main'] });
function service() {
  let active: HistorySession;
  const api = vi.fn(async (command: string, request: Record<string, unknown>) => {
    if (command === 'get_repository_view' || command === 'save_repository_view') return { ok: true, value: { scope: request.scope, viewRevision: '0', view: request.view ?? defaultRepositoryView() } };
    if (command === 'get_commit_details') return { ok: true, value: { context: request.context, commit: commit((request.commitHandle as string).split(':').at(-1)!, active.sessionId), base: request.base, message: 'Full message', committerName: 'Committer', committerEmail: 'committer@example.test', files: [] } };
    if (command === 'open_history') {
      const scope = request.scope as HistoryScope, id = `s${scope.generation}`, context = { scope, snapshotId: `snapshot-${id}` };
      active = { context, sessionId: id, head: { kind: 'branch', branch: 'main', oid: 'visible' }, refs: [
        { name: 'refs/heads/main', displayName: 'main', oid: 'visible', commitOid: 'visible', kind: 'local', hidden: false, symbolicTarget: null },
        { name: 'refs/heads/topic', displayName: 'topic', oid: 'outside', commitOid: 'outside', kind: 'local', hidden: false, symbolicTarget: null }], roots: ['visible'],
        page: { context, sessionId: id, commits: [commit('visible', id)], nextCursor: null } };
      return { ok: true, value: active };
    }
    if (command === 'search_history') return { ok: true, value: { context: request.context as HistoryContext, sessionId: request.sessionId, query: request.query, commits: [{ commit: commit('outside', request.sessionId as string), outsideView: true }], refs: [], nextCursor: null } };
    throw Error('Unexpected command');
  });
  return { api, store: createHistoryStore(api as unknown as Api) };
}

it('integrates graph/sidebar/search with real store callbacks and keeps outside-filter selection out of graph navigation', async () => {
  const { api, store } = service(); await store.getState().activate({ repoId: 'repo', worktreeId: 'tree' });
  const view = render(<HistoryScreen store={store} selection={{ repoId: 'repo', worktreeId: 'tree' }} />);
  const option = await screen.findByRole('row'); fireEvent.click(option);
  expect(store.getState().selected?.commit.handle).toBe('s1:visible');
  fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'Full Author' } });
  fireEvent.click(screen.getByRole('button', { name: 'Search' }));
  const outside = await screen.findByRole('button', { name: /Deep history result/ }); fireEvent.click(outside);
  expect(store.getState().selected?.outsideView).toBe(true); expect(store.getState().selected?.commit.handle).toBe('s1:outside');
  expect(within(screen.getByRole('region', { name: 'Selected commit' })).getByText('Outside current filters')).toBeInTheDocument();
  expect(within(screen.getByRole('grid', { name: 'Commits' })).getAllByRole('row')).toHaveLength(1); expect(screen.getByRole('grid').getAttribute('aria-activedescendant')).toBeNull();
  const filter = store.getState().filter;
  fireEvent.click(screen.getByRole('button', { name: 'References' }));
  fireEvent.click(screen.getByRole('button', { name: 'topic' }));
  await waitFor(() => expect(store.getState().selectionLoading).toBe(false)); expect(store.getState().filter).toBe(filter);
  expect(api).toHaveBeenCalledWith('search_history', expect.objectContaining({ query: 'outside', cursor: null }));
  fireEvent.click(screen.getByRole('button', { name: 'Solo refs/heads/main' }));
  await waitFor(() => expect(store.getState().loading).toBe(false));
  expect(api.mock.calls.filter(([command]) => command === 'open_history').at(-1)![1].filter).toEqual({ hiddenRefs: [], soloRefs: ['refs/heads/main'] });
  expect(within(screen.getByRole('complementary', { name: 'History references' })).getByRole('status')).toHaveTextContent('0 hidden · 1 solo');
  view.unmount(); expect(store.getState().session).not.toBeNull(); store.getState().deactivate();
});

it('leaves activation, refresh and disposal to its owner while rendering selection changes', async () => {
  const { api, store } = service(), timer = vi.spyOn(window, 'setInterval');
  await store.getState().activate({ repoId: 'repo-a', worktreeId: 'tree' });
  const view = render(<HistoryScreen store={store} selection={{ repoId: 'repo-a', worktreeId: 'tree' }} />);
  await screen.findByRole('row');
  view.rerender(<HistoryScreen store={store} selection={{ repoId: 'repo-a', worktreeId: 'tree' }} />);
  expect(api.mock.calls.filter(([command]) => command === 'open_history')).toHaveLength(1);
  await act(async () => store.getState().refresh());
  expect(api.mock.calls.filter(([command]) => command === 'open_history')).toHaveLength(2);
  // Testing Library polls at 50ms and jsdom uses an interval for RAF. Neither
  // is an application history poll; the existing 30s coordinator stays outside.
  expect(timer.mock.calls.map(([, delay]) => delay)).not.toContain(30_000);
  expect(timer.mock.calls.map(([, delay]) => delay)).not.toContain(2_000);
  view.rerender(<HistoryScreen store={store} selection={{ repoId: 'repo-b', worktreeId: null }} />);
  expect(store.getState().session?.context.scope.repoId).toBe('repo-a');
  await act(async () => store.getState().activate({ repoId: 'repo-b', worktreeId: null }));
  expect(store.getState().session?.context.scope.repoId).toBe('repo-b');
  expect(store.getState().session?.context.scope.worktreeId).toBeNull();
  view.rerender(<HistoryScreen store={store} selection={null} />);
  await act(async () => store.getState().deactivate()); expect(store.getState().session).toBeNull();
  expect(screen.getByRole('status')).toHaveTextContent('Select a repository');
});

it('AC1/AC3 ignores legacy sidebar layout and retains selected commit and preferences through drawer close and refresh', async () => {
  const { api, store } = service(), selection = { repoId: 'repo', worktreeId: 'tree' };
  await store.getState().activate(selection);
  store.getState().preferences.getState().patch({ sidebarCollapsed: false, sidebarWidth: 420 });
  const view = render(<HistoryScreen store={store} selection={selection} />);
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  expect(screen.queryByRole('slider', { name: 'Sidebar width' })).not.toBeInTheDocument();
  expect(view.container.querySelector('.history-sidebar-pane')).toBeNull();
  fireEvent.click(await screen.findByRole('row'));
  await waitFor(() => expect(store.getState().inspector.getState().details).not.toBeNull());
  const selectedOid = store.getState().selected?.commit.oid;
  fireEvent.click(screen.getByRole('button', { name: 'References' }));
  fireEvent.click(screen.getByRole('button', { name: 'Solo refs/heads/main' }));
  await waitFor(() => expect(store.getState().loading).toBe(false));
  expect(screen.getByRole('dialog')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: 'Solo refs/heads/topic' }));
  await waitFor(() => expect(store.getState().loading).toBe(false));
  expect(store.getState().filter.soloRefs).toEqual(['refs/heads/main', 'refs/heads/topic']);
  fireEvent.click(screen.getByRole('button', { name: 'Close References' }));
  expect(store.getState().selected?.commit.oid).toBe(selectedOid);
  const filter = store.getState().filter;
  await act(async () => store.getState().refresh());
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  expect(store.getState().filter).toEqual(filter); expect(store.getState().selected?.commit.oid).toBe(selectedOid);
  expect(store.getState().preferences.getState().view).toMatchObject({ sidebarWidth: 420, sidebarCollapsed: false });
  fireEvent.click(screen.getByRole('button', { name: 'References' }));
  fireEvent.click(screen.getByRole('button', { name: 'Show all' }));
  await waitFor(() => expect(store.getState().loading).toBe(false));
  expect(store.getState().filter).toEqual({ hiddenRefs: [], soloRefs: [] });
  view.unmount(); store.getState().deactivate();
});

function pauseHistory(api: ReturnType<typeof service>['api']) {
  const original = api.getMockImplementation()!;
  let resolve!: () => void, reject!: (error: Error) => void;
  const pending = new Promise<void>((done, fail) => { resolve = done; reject = fail; });
  api.mockImplementation(async (...args) => { if (args[0] === 'open_history') await pending; return original(...args); });
  return { resolve, reject };
}

it.each(['new snapshot', 'failed request'] as const)('T006-F01 retains stable focus after graph Solo and deferred %s', async outcome => {
  const { api, store } = service(), selection = { repoId: 'repo', worktreeId: 'tree' };
  await store.getState().activate(selection);
  const view = render(<HistoryScreen store={store} selection={selection} />);
  const oldSession = store.getState().session;
  const oldBadge = await screen.findByRole('button', { name: 'Actions for refs/heads/main' });
  const references = screen.getByRole('button', { name: 'References' });
  const request = pauseHistory(api);
  fireEvent.click(oldBadge);
  fireEvent.click(screen.getByRole('menuitem', { name: 'Add to Solo' }));
  expect(store.getState().loading).toBe(true);
  expect(references).toHaveFocus();
  expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  await act(async () => { if (outcome === 'new snapshot') request.resolve(); else request.reject(new Error('History request failed')); });
  await waitFor(() => expect(store.getState().loading).toBe(false));
  expect(references).toHaveFocus();
  expect(references.isConnected).toBe(true);
  if (outcome === 'new snapshot') { expect(store.getState().session).not.toBe(oldSession); expect(oldBadge.isConnected).toBe(false); }
  else { expect(store.getState().session).toBe(oldSession); expect(screen.getByRole('alert')).toBeInTheDocument(); }
  view.unmount(); store.getState().deactivate();
});

it.each(['stay on badge', 'move to search'] as const)('T006-F01 preserves focus intent during background refresh: %s', async intent => {
  const { api, store } = service(), selection = { repoId: 'repo', worktreeId: 'tree' };
  await store.getState().activate(selection);
  const view = render(<HistoryScreen store={store} selection={selection} />);
  const oldBadge = await screen.findByRole('button', { name: 'Actions for refs/heads/main' });
  oldBadge.focus();
  const request = pauseHistory(api);
  let refreshing!: Promise<void>;
  act(() => { refreshing = store.getState().refresh(); });
  expect(oldBadge).toHaveFocus();
  expect(oldBadge).toHaveAttribute('aria-disabled', 'true');
  fireEvent.click(oldBadge); expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  const search = screen.getByRole('searchbox');
  if (intent === 'move to search') search.focus();
  await act(async () => { request.resolve(); await refreshing; });
  expect(oldBadge.isConnected).toBe(false);
  expect(intent === 'stay on badge' ? screen.getByRole('button', { name: 'References' }) : search).toHaveFocus();
  view.unmount(); store.getState().deactivate();
});

it('T006-F01 does not move focus back when the user leaves history during a pending filter request', async () => {
  const { api, store } = service(), selection = { repoId: 'repo', worktreeId: 'tree' };
  await store.getState().activate(selection);
  const view = render(<><button>Overview destination</button><HistoryScreen store={store} selection={selection} /></>);
  const request = pauseHistory(api);
  fireEvent.click(await screen.findByRole('button', { name: 'Actions for refs/heads/main' }));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Add to Solo' }));
  const destination = screen.getByRole('button', { name: 'Overview destination' }); destination.focus();
  view.rerender(<button>Overview destination</button>);
  expect(destination).toHaveFocus();
  await act(async () => request.resolve());
  await waitFor(() => expect(store.getState().loading).toBe(false));
  expect(destination).toHaveFocus();
  view.unmount(); store.getState().deactivate();
});
