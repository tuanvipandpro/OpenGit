import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { ChangesPanel } from './ChangesPanel';
import { createRepositoryChangesStore } from '../../stores/repositoryChanges';
import { createWorkspaceStore } from '../../stores/workspace';
import { createOperationsStore } from '../../stores/operations';
import { createHistoryStore } from '../../stores/history';
import type { ChangeDiff as Diff, PreparedAction, RepositoryChanges } from '../../contracts';
import type { RefreshInteractionLease } from '../repository/refresh';
const scope = { repoId: 'repo', worktreeId: 'tree', generation: '1' };
const catalog: RepositoryChanges = { scope, catalogId: 'catalog', observedAt: '', unsupported: false, head: { kind: 'branch', branch: 'main', oid: 'a'.repeat(40) }, entries: [{ entryId: 'entry', pathDisplay: 'same Ω.txt', previousPathDisplay: null, indexStatus: 'M', worktreeStatus: 'M', stageCapability: 'eligible', unstageCapability: 'eligible' }] };
const plan: PreparedAction = { scope, kind: 'commit', planId: 'plan', warnings: [], expiresAt: '2099-01-01T00:00:00Z' };
const diff: Diff = { scope, side: 'unstaged', changeHandle: 'displayed-source', diffId: 'displayed-diff', content: 'opaque display text', partialEligibility: 'eligible', hunks: [{ hunkId: 'first-hunk', rows: [{ lineId: null, kind: 'context', oldLine: 1, newLine: 1, text: 'same', hasLf: true }, { lineId: 'delete-2', kind: 'delete', oldLine: 2, newLine: null, text: 'old text', hasLf: true }, { lineId: 'add-2', kind: 'add', oldLine: null, newLine: 2, text: '<script>visible text</script>', hasLf: true }] }, { hunkId: 'last-hunk', rows: [{ lineId: 'add-8', kind: 'add', oldLine: null, newLine: 8, text: 'tail', hasLf: false }] }] };
function setup(ready = Promise.resolve(true)) {
  let finish!: () => void; const closed = new Promise<void>(r => { finish = r; }); let current = true;
  const release = vi.fn(() => { current = false; finish(); }), lease: RefreshInteractionLease = { ready, closed, isCurrent: () => current, release, cancel: release };
  const store = createRepositoryChangesStore(), workspace = createWorkspaceStore(), tasks = createOperationsStore(), history = createHistoryStore();
  const reload = vi.fn(async () => {}), connect = vi.fn(() => () => {}), select = vi.fn(async () => {}), prepareFile = vi.fn(async () => {}), prepareCommit = vi.fn(async () => {}), sync = vi.fn(async () => {});
  const beginInteraction = vi.fn(() => lease);
  store.setState({ scope, catalog, reload, connect, select, prepareFile, prepareCommit, awaitCatalog: async () => true }); workspace.setState({ beginInteraction }); tasks.setState({ sync });
  const view = render(<ChangesPanel store={store} workspace={workspace} tasks={tasks} history={history} />);
  return { store, workspace, tasks, history, view, reload, select, prepareFile, prepareCommit, sync, release, finish, beginInteraction };
}
it('shows independent staged and unstaged sides, readonly diff, and explicit file prepare/execute', async () => {
  const t = setup(); const unstaged = screen.getByRole('region', { name: 'Unstaged changes' }), staged = screen.getByRole('region', { name: 'Staged changes' });
  await act(async () => fireEvent.click(within(unstaged).getByRole('button', { name: 'same Ω.txt' }))); expect(t.select).toHaveBeenCalledWith('entry', 'unstaged');
  await act(async () => fireEvent.click(within(staged).getByRole('button', { name: 'same Ω.txt' }))); expect(t.select).toHaveBeenLastCalledWith('entry', 'staged'); expect(t.beginInteraction).toHaveBeenCalledOnce();
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Stage file' })));
  expect(t.prepareFile).not.toHaveBeenCalled(); expect(screen.getByRole('button', { name: 'Execute' })).toBeDisabled();
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' }))); expect(t.prepareFile).toHaveBeenCalledWith('entry', 'unstaged');
  act(() => t.store.setState({ diff: { scope, diffId: 'd', changeHandle: 'h', side: 'unstaged', content: '<script>not executable</script>', partialEligibility: 'unsupported', hunks: [] } }));
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
it('S5-B shows structured edit text with accessible hunk/line choices and keeps selection modes separate', () => {
  const t = setup(), setSelection = vi.fn((_fence, selection) => t.store.setState({ selection }));
  act(() => t.store.setState({ diff, selectedChange: { entryId: 'entry', side: 'unstaged' }, setSelection }));
  expect(screen.getByText('<script>visible text</script>')).toBeInTheDocument(); expect(t.view.container.querySelector('script')).toBeNull(); expect(screen.getByText('No newline at end of file')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('checkbox', { name: 'Select deleted line 2' })); fireEvent.click(screen.getByRole('checkbox', { name: 'Select added line 2' }));
  expect(t.store.getState().selection).toEqual({ kind: 'lines', lineIds: ['delete-2', 'add-2'] }); expect(setSelection).toHaveBeenLastCalledWith(diff, { kind: 'lines', lineIds: ['delete-2', 'add-2'] });
  fireEvent.click(screen.getByRole('checkbox', { name: 'Select hunk 1' })); expect(t.store.getState().selection).toEqual({ kind: 'hunks', hunkIds: ['first-hunk'] }); expect(screen.getByRole('checkbox', { name: 'Select added line 2' })).not.toBeChecked();
  fireEvent.click(screen.getByRole('checkbox', { name: 'Select added line 8' })); expect(screen.getByRole('checkbox', { name: 'Select hunk 1' })).not.toBeChecked();
});
it.each(['unstaged', 'staged'] as const)('S5-B opens the %s partial form without reload, reselect or implicit preparation', async side => {
  const t = setup(), prepareSelection = vi.fn(async () => {}), selectedDiff = { ...diff, side };
  act(() => t.store.setState({ diff: selectedDiff, selection: { kind: 'lines', lineIds: ['delete-2', 'add-2'] }, selectedChange: { entryId: 'entry', side }, prepareSelection }));
  await act(async () => fireEvent.click(screen.getByRole('button', { name: side === 'unstaged' ? 'Stage selected' : 'Unstage selected' })));
  expect(screen.getByRole('dialog', { name: side === 'unstaged' ? 'Stage selected' : 'Unstage selected' })).toBeInTheDocument(); expect(t.reload).not.toHaveBeenCalled(); expect(t.select).not.toHaveBeenCalled(); expect(prepareSelection).not.toHaveBeenCalled();
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' })));
  expect(prepareSelection).toHaveBeenCalledWith({ scope, changeHandle: diff.changeHandle, diffId: diff.diffId, side });
});
it('S5-B waits for lease readiness and requires explicit reselection when the original diff fence changes', async () => {
  let ready!: (value: boolean) => void; const t = setup(new Promise(resolve => { ready = resolve; })), prepareSelection = vi.fn(async () => {});
  act(() => t.store.setState({ diff, selection: { kind: 'hunks', hunkIds: ['first-hunk'] }, prepareSelection }));
  fireEvent.click(screen.getByRole('button', { name: 'Stage selected' })); expect(screen.getByText('Loading current context…')).toBeInTheDocument();
  act(() => t.store.setState({ diff: { ...diff, diffId: 'replacement' }, selection: null })); await act(async () => ready(true));
  expect(screen.getByRole('alert')).toHaveTextContent('select the file again'); expect(screen.getByRole('button', { name: 'Prepare' })).toBeDisabled(); expect(t.reload).not.toHaveBeenCalled(); expect(t.select).not.toHaveBeenCalled(); expect(prepareSelection).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Close' })); expect(t.release).toHaveBeenCalledOnce();
});
it.each(['binary', 'lineEndings', 'attributes', 'complexityLimit', 'filePresenceOnly'] as const)('S5-B explains %s fallback and leaves whole-file actions available', eligibility => {
  const t = setup(); act(() => t.store.setState({ diff: { ...diff, partialEligibility: eligibility, hunks: [] } }));
  expect(screen.queryByRole('checkbox')).not.toBeInTheDocument(); expect(screen.getByRole('region', { name: 'Change diff' })).toHaveTextContent(/whole-file action/); expect(screen.getByRole('button', { name: 'Stage file' })).toBeEnabled();
});
it('S5-B allows a message-only amend form with HEAD and zero staged files, defaults off and stales preview on toggle/message edits', async () => {
  const t = setup(); act(() => t.store.setState({ catalog: { ...catalog, entries: [] } }));
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Commit staged changes' })));
  expect(screen.getByRole('checkbox', { name: 'Amend last commit' })).not.toBeChecked(); fireEvent.change(screen.getByLabelText('Commit message'), { target: { value: 'new message' } }); expect(screen.getByRole('button', { name: 'Prepare' })).toBeDisabled();
  fireEvent.click(screen.getByRole('checkbox', { name: 'Amend last commit' })); expect(screen.getByRole('button', { name: 'Prepare' })).toBeEnabled();
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' }))); expect(t.prepareCommit).toHaveBeenCalledOnce();
  act(() => t.store.setState({ prepared: plan })); fireEvent.change(screen.getByLabelText('Commit message'), { target: { value: 'edited message' } }); expect(screen.getByRole('button', { name: 'Execute' })).toBeDisabled();
  act(() => t.store.setState({ prepared: plan })); fireEvent.click(screen.getByRole('checkbox', { name: 'Amend last commit' })); expect(screen.getByRole('button', { name: 'Execute' })).toBeDisabled();
});
it('S5-B keeps amend draft/toggle after queued ACK and does not claim commit success', async () => {
  const t = setup(); t.store.setState({ execute: async () => ({ ok: true, value: { kind: 'action', actionKind: 'commit', state: 'Queued' } as never }) });
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Commit staged changes' })));
  fireEvent.change(screen.getByLabelText('Commit message'), { target: { value: 'keep after queue' } }); fireEvent.click(screen.getByRole('checkbox', { name: 'Amend last commit' })); act(() => t.store.setState({ prepared: plan }));
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Execute' })));
  expect(t.store.getState().draft).toBe('keep after queue'); expect(t.store.getState().amend).toBe(true); expect(t.sync).toHaveBeenCalledOnce(); expect(screen.queryByRole('dialog')).not.toBeInTheDocument(); expect(screen.queryByText(/commit succeeded/i)).not.toBeInTheDocument();
});
it.each(['close', 'unmount'] as const)('S5B-FORM01 %s cancels inspection before readiness without a late source read', async disposition => {
  let ready!: (value: boolean) => void; const t = setup(new Promise(resolve => { ready = resolve; }));
  fireEvent.click(within(screen.getByRole('region', { name: 'Unstaged changes' })).getByRole('button', { name: 'same Ω.txt' }));
  expect(t.beginInteraction).toHaveBeenCalledOnce(); expect(screen.getByRole('button', { name: 'Close diff' })).toBeInTheDocument();
  if (disposition === 'close') fireEvent.click(screen.getByRole('button', { name: 'Close diff' })); else t.view.unmount();
  await act(async () => ready(true)); expect(t.select).not.toHaveBeenCalled(); expect(t.reload).not.toHaveBeenCalled(); expect(t.release).toHaveBeenCalledOnce();
});
