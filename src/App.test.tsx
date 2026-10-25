import { StrictMode } from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, afterEach, it, expect, vi } from 'vitest';
import App from './App';
import { createWorkspaceStore } from './stores/workspace';
import type { Api } from './stores/workspace';
import { createOperationsStore } from './stores/operations';
import { createHistoryStore } from './stores/history';
import type { Workspace, ToolInfo, Environment } from './contracts';
import fixture from '../fixtures/contracts/wire.json';
import { defaultRepositoryView } from './stores/historyView';
beforeEach(() => { vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} }); vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null); });
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });
function makeServices(failed = false, git?: ToolInfo) {
  const workspace = fixture.commands[1].result.value as unknown as Workspace;
  const empty = { ...workspace, revision: '0', repositories: [], activeSelection: null };
  const api = vi.fn(async (command: string, request: Record<string, unknown>) => {
    if (command === 'get_repository_view' || command === 'save_repository_view') return { ok: true, value: { scope: request.scope, viewRevision: '0', view: request.view ?? defaultRepositoryView() } };
    if (command === 'get_environment') return git ? { ok: true, value: { ...(fixture.commands[0].result.value as unknown as Environment), git } } : fixture.commands[0].result;
    if (command === 'get_workspace') return failed ? fixture.error : { ok: true, value: empty };
    if (command === 'choose_repository') return { ok: true, value: workspace };
    if (command === 'inspect_repository') return { ok: true, value: { ...fixture.snapshots[0], ...request } };
    if (command === 'forget_repository') return { ok: true, value: { ...empty, revision: '9007199254740994' } };
    if (command === 'open_history') { const context = { scope: request.scope, snapshotId: 'empty' }; return { ok: true, value: { context, sessionId: 'empty', head: { kind: 'unborn', branch: 'main', oid: null }, refs: [], roots: [], page: { context, sessionId: 'empty', commits: [], nextCursor: null } } }; }
    if (command === 'watch_repository') return { ok: true, value: { scope: request.scope, subscriptionId: 'watch', sequence: '0', status: 'watching' } };
    if (command === 'unwatch_repository') return { ok: true, value: null };
    if (command === 'get_operations') return { ok: true, value: { streamId: 'stream', sequence: '0', operations: [] } };
    throw Error('Unexpected command');
  }) as unknown as Api;
  return { store: createWorkspaceStore(api, async () => () => {}), history: createHistoryStore(api), tasks: createOperationsStore(api, async () => () => {}), api };
}
it('opens a real-service workspace response from empty view and forgets without deleting files', async () => {
  const user = userEvent.setup(); render(<App {...makeServices()} />);
  const open = await screen.findByRole('button', { name: 'Open a repository' });
  expect(screen.getByRole('button', { name: 'Clone a repository' })).toBeDisabled();
  await user.click(open);
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  expect(await screen.findByRole('heading', { name: 'Fixture' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: 'main' })).toBeInTheDocument();
  expect(screen.getByRole('table', { name: 'File status' })).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Forget Fixture' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Open a repository' })).toBeInTheDocument());
  expect(screen.queryByRole('table')).not.toBeInTheDocument();
});

it.each(['Overview', 'History'] as const)('keeps %s content quiet during background refresh and aligns manual feedback', async tab => {
  vi.useFakeTimers();
  const services = makeServices();
  const implementation = vi.mocked(services.api).getMockImplementation()!;
  let finish: ((value: unknown) => void) | undefined;
  let defer = false;
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'inspect_repository' && defer) return new Promise<unknown>(resolve => { finish = () => resolve({ ok: true, value: { ...fixture.snapshots[0], ...request } }); });
    return implementation(command as never, request as never);
  }) as Api);
  const view = render(<App {...services} />);
  try {
    await act(async () => { await services.store.getState().open(); });
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: tab })); });
    expect(screen.getByRole(tab === 'Overview' ? 'table' : 'grid', { name: tab === 'Overview' ? 'File status' : 'Commits' })).toBeInTheDocument();
    defer = true;
    await act(async () => { await vi.advanceTimersByTimeAsync(30_000); });
    expect(screen.getByRole(tab === 'Overview' ? 'table' : 'grid', { name: tab === 'Overview' ? 'File status' : 'Commits' })).toBeInTheDocument();
    expect(screen.queryByText('Refreshing…')).not.toBeInTheDocument();
    expect(view.container.querySelector('.workspace-content')).toHaveAttribute('aria-busy', 'false');
    await act(async () => { await vi.advanceTimersByTimeAsync(500); });
    expect(screen.getByText('Refreshing…')).toBeInTheDocument();
    if (tab === 'History') expect(screen.getByText('Refreshing…')).toHaveClass('repository-refresh-status');
    await act(async () => { finish?.(undefined); });
    expect(screen.queryByText('Refreshing…')).not.toBeInTheDocument();
    act(() => { void services.store.getState().refresh(); });
    expect(screen.getByText('Refreshing…')).toBeInTheDocument();
    expect(screen.getByRole(tab === 'Overview' ? 'table' : 'grid', { name: tab === 'Overview' ? 'File status' : 'Commits' })).toBeInTheDocument();
    await act(async () => { finish?.(undefined); });
  } finally { view.unmount(); vi.useRealTimers(); }
});
it('keeps startup failures actionable and safe in the product UI', async () => {
  render(<App {...makeServices(true)} />);
  expect(await screen.findByRole('alert')).toHaveTextContent('Certificate verification failed.');
  expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument();
  expect(screen.queryByText(/backend|IPC|foundation/i)).not.toBeInTheDocument();
});
it('starts task synchronization when a workspace is opened after an initial loading error', async () => {
  const user = userEvent.setup(), services = makeServices(true);
  const start = vi.spyOn(services.tasks.getState(), 'start');
  render(<App {...services} />);
  await screen.findByRole('alert'); expect(start).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Open a repository' }));
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
  expect(await screen.findByRole('table', { name: 'File status' })).toBeInTheDocument();
});

it.each([
  { git: { available: false, version: '2.20.0' }, status: 'Git needs update', diagnostic: 'Git 2.20.0 is unsupported. Git 2.31 or later is required.' },
  { git: { available: false, version: null }, status: 'Git not found', diagnostic: 'Git not found' },
])('distinguishes unsupported Git from a missing installation: $status', async ({ git, status, diagnostic }) => {
  const user = userEvent.setup(); render(<App {...makeServices(false, git)} />);
  await screen.findByRole('button', { name: 'Open a repository' });
  expect(screen.getByText(status)).toBeInTheDocument();
  expect(screen.queryByText('Git ready')).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: /Settings & diagnostics/ }));
  expect(screen.getByRole('region', { name: 'Git diagnostics' })).toHaveTextContent(diagnostic);
  if (git.version) expect(screen.queryByText('Git not found')).not.toBeInTheDocument();
  else expect(screen.queryByText(/unsupported/)).not.toBeInTheDocument();
});

it('opens History by default with typed history data and keeps Overview/status available', async () => {
  const services = makeServices(), implementation = vi.mocked(services.api).getMockImplementation()!;
  let failInspection = false;
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'inspect_repository' && failInspection) return Promise.resolve(fixture.error);
    if (command === 'open_history') {
      const context = { scope: request.scope, snapshotId: 'empty-history' };
      return Promise.resolve({ ok: true, value: { context, sessionId: 'empty-session', head: { kind: 'unborn', branch: 'main', oid: null }, refs: [], roots: [], page: { context, sessionId: 'empty-session', commits: [], nextCursor: null } } });
    }
    return implementation(command as never, request as never);
  }) as Api);
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
  const canvas = vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
  const view = render(<App {...services} history={createHistoryStore(services.api)} />), user = userEvent.setup();
  try {
    await user.click(await screen.findByRole('button', { name: 'Open a repository' }));
    expect(await screen.findByText('No commits in the current view.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'History' })).toHaveAttribute('aria-current', 'page');
    expect(screen.queryByText('HEAD has no commits.')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'References' }));
    expect(screen.getByText('HEAD has no commits.')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Close References' }));
    expect(vi.mocked(services.api).mock.calls.filter(([command]) => command === 'open_history')).toHaveLength(1);
    failInspection = true;
    await user.click(screen.getByRole('button', { name: 'Refresh repository' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Certificate verification failed.');
    expect(screen.getByText('No commits in the current view.')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Overview' }));
    expect(await screen.findByRole('table', { name: 'File status' })).toBeInTheDocument();
  } finally { view.unmount(); canvas.mockRestore(); vi.unstubAllGlobals(); }
});

it('finishes pending and newer preference saves before an explicit repository selection', async () => {
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  const saves: { request: Record<string, unknown>; resolve: (result: unknown) => void }[] = [];
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'save_repository_view') return new Promise<unknown>(resolve => { saves.push({ request, resolve }); });
    return original(command as never, request as never);
  }) as Api);
  const history = createHistoryStore(services.api), selected = vi.spyOn(services.store.getState(), 'select');
  const view = render(<App {...services} history={history} />);
  fireEvent.click(await screen.findByRole('button', { name: 'Open a repository' }));
  await screen.findByRole('button', { name: 'Select Fixture' });
  await waitFor(() => expect(history.getState().preferences.getState().hydrated).toBe(true));
  act(() => history.getState().preferences.getState().patch({ sidebarWidth: 450 }));
  fireEvent.click(screen.getByRole('button', { name: 'Select Fixture' }));
  await waitFor(() => expect(saves).toHaveLength(1)); expect(selected).not.toHaveBeenCalled();
  act(() => history.getState().preferences.getState().patch({ sidebarWidth: 500 }));
  await act(async () => saves[0].resolve({ ok: true, value: { scope: saves[0].request.scope, viewRevision: '1', view: saves[0].request.view } }));
  await waitFor(() => expect(saves).toHaveLength(2)); expect(selected).not.toHaveBeenCalled();
  expect(saves[1].request.view).toMatchObject({ sidebarWidth: 500 });
  await act(async () => saves[1].resolve({ ok: true, value: { scope: saves[1].request.scope, viewRevision: '2', view: saves[1].request.view } }));
  await waitFor(() => expect(selected).toHaveBeenCalledOnce());
  view.unmount();
});

it('keeps Overview and a concurrent repository transition behind pending and newer preference acknowledgements', async () => {
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  const saves: { request: Record<string, unknown>; resolve: (result: unknown) => void }[] = [];
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'save_repository_view') return new Promise<unknown>(resolve => { saves.push({ request, resolve }); });
    return original(command as never, request as never);
  }) as Api);
  const history = createHistoryStore(services.api), selected = vi.spyOn(services.store.getState(), 'select');
  const view = render(<App {...services} history={history} />);
  fireEvent.click(await screen.findByRole('button', { name: 'Open a repository' }));
  await screen.findByRole('button', { name: 'Select Fixture' });
  await waitFor(() => expect(history.getState().preferences.getState().hydrated).toBe(true));
  act(() => history.getState().preferences.getState().patch({ sidebarWidth: 400 }));
  fireEvent.click(screen.getByRole('button', { name: 'Overview' }));
  await waitFor(() => expect(saves).toHaveLength(1));
  expect(screen.getByRole('button', { name: 'History' })).toHaveAttribute('aria-current', 'page');
  act(() => history.getState().preferences.getState().patch({ sidebarWidth: 500 }));
  fireEvent.click(screen.getByRole('button', { name: 'Select Fixture' }));
  expect(selected).not.toHaveBeenCalled();
  await act(async () => saves[0].resolve({ ok: true, value: { scope: saves[0].request.scope, viewRevision: '1', view: saves[0].request.view } }));
  await waitFor(() => expect(saves).toHaveLength(2)); expect(selected).not.toHaveBeenCalled();
  expect(screen.getByRole('button', { name: 'History' })).toHaveAttribute('aria-current', 'page');
  expect(saves[1].request.view).toMatchObject({ sidebarWidth: 500 });
  await act(async () => saves[1].resolve({ ok: true, value: { scope: saves[1].request.scope, viewRevision: '2', view: saves[1].request.view } }));
  await waitFor(() => expect(selected).toHaveBeenCalledOnce());
  expect(screen.getByRole('button', { name: 'Overview' })).toHaveAttribute('aria-current', 'page');
  expect(history.getState().preferences.getState().hydrated).toBe(true);
  view.unmount();
});

it('has one StrictMode owner across tabs, preserves preferences and disposes the watcher on unmount', async () => {
  const services = makeServices(), history = services.history;
  const view = render(<StrictMode><App {...services} /></StrictMode>), user = userEvent.setup();
  await user.click(await screen.findByRole('button', { name: 'Open a repository' }));
  await screen.findByText('No commits in the current view.');
  const scope = history.getState().scope;
  act(() => history.getState().preferences.getState().patch({ sidebarWidth: 450, detailsHeight: 420, diffLayout: 'inline' }));
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  expect(history.getState().scope).toBe(scope); expect(history.getState().preferences.getState().hydrated).toBe(true);
  await user.click(screen.getByRole('button', { name: 'History' }));
  expect(history.getState().scope).toBe(scope); expect(history.getState().preferences.getState().view).toMatchObject({ sidebarWidth: 450, detailsHeight: 420, diffLayout: 'inline' });
  const count = (name: string) => vi.mocked(services.api).mock.calls.filter(([command]) => command === name).length;
  expect(count('get_workspace')).toBe(1); expect(count('get_repository_view')).toBe(1); expect(count('open_history')).toBe(1); expect(count('watch_repository')).toBe(1);
  view.unmount(); await act(async () => {});
  expect(history.getState().scope).toBeNull(); expect(count('unwatch_repository')).toBe(1);
});
