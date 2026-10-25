import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { ChangesPanel } from './ChangesPanel';
import { createRepositoryChangesStore } from '../../stores/repositoryChanges';
import { createWorkspaceStore } from '../../stores/workspace';
import { createOperationsStore } from '../../stores/operations';
import { createHistoryStore } from '../../stores/history';
import type { PreparedAction, RepositoryChanges } from '../../contracts';
import type { RefreshInteractionLease } from '../repository/refresh';
const scope = { repoId: 'repo', worktreeId: 'tree', generation: '1' };
const catalog: RepositoryChanges = { scope, catalogId: 'catalog', observedAt: '', unsupported: false, head: { kind: 'branch', branch: 'main', oid: 'a'.repeat(40) }, entries: [{ entryId: 'entry', pathDisplay: 'same Ω.txt', previousPathDisplay: null, indexStatus: 'M', worktreeStatus: 'M', stageCapability: 'eligible', unstageCapability: 'eligible' }] };
const plan: PreparedAction = { scope, kind: 'commit', planId: 'plan', warnings: [], expiresAt: '2099-01-01T00:00:00Z' };
function setup(ready = Promise.resolve(true)) {
  let finish!: () => void; const closed = new Promise<void>(r => { finish = r; }); let current = true;
  const release = vi.fn(() => { current = false; finish(); }), lease: RefreshInteractionLease = { ready, closed, isCurrent: () => current, release, cancel: release };
  const store = createRepositoryChangesStore(), workspace = createWorkspaceStore(), tasks = createOperationsStore(), history = createHistoryStore();
  const reload = vi.fn(async () => {}), connect = vi.fn(() => () => {}), select = vi.fn(async () => {}), prepareFile = vi.fn(async () => {}), prepareCommit = vi.fn(async () => {}), sync = vi.fn(async () => {});
  store.setState({ scope, catalog, reload, connect, select, prepareFile, prepareCommit }); workspace.setState({ beginInteraction: () => lease }); tasks.setState({ sync });
  const view = render(<ChangesPanel store={store} workspace={workspace} tasks={tasks} history={history} />);
  return { store, workspace, tasks, history, view, reload, select, prepareFile, prepareCommit, sync, release, finish };
}
it('shows independent staged and unstaged sides, readonly diff, and explicit file prepare/execute', async () => {
  const t = setup(); const unstaged = screen.getByRole('region', { name: 'Unstaged changes' }), staged = screen.getByRole('region', { name: 'Staged changes' });
  fireEvent.click(within(unstaged).getByRole('button', { name: 'same Ω.txt' })); expect(t.select).toHaveBeenCalledWith('entry', 'unstaged');
  fireEvent.click(within(staged).getByRole('button', { name: 'same Ω.txt' })); expect(t.select).toHaveBeenLastCalledWith('entry', 'staged');
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Stage file' })));
  expect(t.prepareFile).not.toHaveBeenCalled(); expect(screen.getByRole('button', { name: 'Execute' })).toBeDisabled();
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' }))); expect(t.prepareFile).toHaveBeenCalledWith('entry', 'unstaged');
  act(() => t.store.setState({ diff: { scope, diffId: 'd', changeHandle: 'h', content: '<script>not executable</script>', partialEligibility: 'Whole-file actions only', hunks: [] } }));
  expect(screen.getByText('<script>not executable</script>')).toBeInTheDocument(); expect(screen.queryByRole('checkbox')).not.toBeInTheDocument();
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Close' }))); expect(t.release).toHaveBeenCalledOnce();
});
it('keeps the commit draft through manual context changes and requires a new preview', async () => {
  const t = setup(); await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Commit staged changes' })));
  fireEvent.change(screen.getByLabelText('Commit message'), { target: { value: 'memory draft' } }); expect(t.prepareCommit).not.toHaveBeenCalled();
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' }))); expect(t.prepareCommit).toHaveBeenCalledOnce();
  act(() => t.store.setState({ prepared: plan })); expect(screen.getByRole('button', { name: 'Execute' })).toBeEnabled();
  act(() => t.store.setState({ scope: { ...scope, generation: '2' }, catalog: { ...catalog, scope: { ...scope, generation: '2' } }, prepared: null, stale: true }));
  expect(screen.getByLabelText('Commit message')).toHaveValue('memory draft'); expect(screen.getByRole('button', { name: 'Execute' })).toBeDisabled(); expect(screen.getByText('Repository refreshed. Prepare again.')).toBeInTheDocument();
});
it('cancels a pending interaction before readiness and never opens a late form', async () => {
  let ready!: (v: boolean) => void; const t = setup(new Promise(resolve => { ready = resolve; }));
  fireEvent.click(screen.getByRole('button', { name: 'Commit staged changes' })); expect(screen.getByText('Loading current context…')).toBeInTheDocument();
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Close' }))); await act(async () => ready(true));
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument(); expect(t.reload).not.toHaveBeenCalled(); expect(t.release).toHaveBeenCalledOnce();
});
it('syncs Tasks after a lost execution reply, preserves the draft, and never auto retries', async () => {
  const t = setup(); const execute = vi.fn(async () => ({ ok: false as const, error: { code: 'Internal' as const, phase: 'publish' as const, retry: 'none' as const, message: 'Reply lost. Check Tasks.' } })); t.store.setState({ execute });
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Commit staged changes' }))); fireEvent.change(screen.getByLabelText('Commit message'), { target: { value: 'keep draft' } }); act(() => t.store.setState({ prepared: plan }));
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Execute' }))); expect(t.sync).toHaveBeenCalledOnce(); expect(execute).toHaveBeenCalledOnce(); expect(screen.getByLabelText('Commit message')).toHaveValue('keep draft'); expect(screen.getByRole('dialog')).toBeInTheDocument();
});
it('does not clear a newer form when an old execution resolves after disposal', async () => {
  const t = setup(); let resolve!: (v: never) => void; t.store.setState({ execute: () => new Promise(done => { resolve = done; }) });
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Commit staged changes' }))); act(() => t.store.setState({ prepared: plan })); fireEvent.click(screen.getByRole('button', { name: 'Execute' })); t.view.unmount();
  t.store.setState({ prepared: { ...plan, planId: 'newer' } }); await act(async () => resolve({ ok: true, value: {} } as never));
  expect(t.sync).toHaveBeenCalledOnce(); expect(t.store.getState().prepared?.planId).toBe('newer');
});
it('carries the clicked entry ID rather than a duplicate display label through the file form',async()=>{
    const t=setup();act(()=>t.store.setState({catalog:{...catalog,entries:[{...catalog.entries[0],entryId:'first'},{...catalog.entries[0],entryId:'second'}]}}));
    const list=screen.getByRole('region',{name:'Unstaged changes'});await act(async()=>fireEvent.click(within(list).getAllByRole('button',{name:'Stage file'})[1]));
    act(()=>t.store.setState({catalog:{...catalog,catalogId:'replaced',entries:[{...catalog.entries[0],entryId:'second'},{...catalog.entries[0],entryId:'first'}]}}));
    await act(async()=>fireEvent.click(screen.getByRole('button',{name:'Prepare'})));expect(t.prepareFile).toHaveBeenCalledWith('second','unstaged');
});
