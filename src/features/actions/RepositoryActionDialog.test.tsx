import { act, fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { RepositoryActionDialog } from './RepositoryActionDialog';
import { createRepositoryActionsStore } from '../../stores/repositoryActions';
import type { ActionLaunch, LocalActionDraft } from '../../stores/repositoryActions';
import type { PreparedAction, RefActionCatalog } from '../../contracts';
import fixture from '../../../fixtures/contracts/ref-actions-wire.json';
const catalog = fixture.catalog as RefActionCatalog;
const plan: PreparedAction = { scope: catalog.scope, kind: 'branchCreate', planId: 'plan', expiresAt: '2099-01-01T00:00:00Z', warnings: ['externalRace'] };
function setup(launch: ActionLaunch = {}) {
  const store = createRepositoryActionsStore(), prepare = vi.fn(async (_draft: LocalActionDraft) => {}), execute = vi.fn(async () => ({ ok: true as const, value: fixture.result.operation as import('../../contracts').OperationSnapshot })), onDismiss = vi.fn(), onStarted = vi.fn(async () => {}), onRefresh = vi.fn();
  store.setState({ scope: catalog.scope, catalog, prepare, execute });
  const view = render(<RepositoryActionDialog store={store} launch={launch} onDismiss={onDismiss} onStarted={onStarted} onRefresh={onRefresh} />);
  const preview = () => act(() => store.setState({ prepared: plan, preview: { source: launch.refName ?? 'HEAD', destination: 'topic' } }));
  return { store, prepare, execute, onDismiss, onStarted, onRefresh, view, preview };
}
it('S7 starts from HEAD, makes checkout explicit and prepares only on request', async () => {
  const t = setup(); expect(screen.getByLabelText('Start at')).toHaveValue('head'); expect(screen.getByLabelText('Check out the new branch')).not.toBeChecked();
  expect(screen.queryByRole('button', { name: 'Apply action' })).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('Branch name'), { target: { value: 'topic' } }); fireEvent.click(screen.getByLabelText('Check out the new branch'));
  expect(t.prepare).not.toHaveBeenCalled();
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare action' })));
  expect(t.prepare).toHaveBeenCalledWith(expect.objectContaining({ kind: 'branchCreate', source: { kind: 'head' }, name: 'topic', checkout: true }));
  t.preview(); expect(screen.getByRole('region', { name: 'Local action preview' })).toHaveTextContent('Source: HEAD');
  fireEvent.change(screen.getByLabelText('Branch name'), { target: { value: 'another' } }); expect(screen.queryByRole('region')).not.toBeInTheDocument();
});
it('S7 requires a fresh explicit confirmation for local deletion and submits once', async () => {
  const t = setup({ kind: 'tagDelete', refName: 'refs/tags/v1' });
  expect(screen.getByLabelText('Reference')).toHaveValue('ref:refs/tags/v1'); t.preview();
  expect(screen.getByRole('button', { name: 'Apply action' })).toBeDisabled();
  fireEvent.click(screen.getByLabelText(/Confirm local deletion/));
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Apply action' })); fireEvent.click(screen.getByRole('button', { name: 'Apply action' })); });
  expect(t.execute).toHaveBeenCalledOnce(); expect(t.onStarted).toHaveBeenCalledOnce(); expect(t.onDismiss).toHaveBeenCalledOnce();
});
it('S7 retains names, annotation and source through manual refresh while staling preview', async () => {
  const t = setup({ kind: 'tagCreate', refName: 'refs/tags/v1' });
  fireEvent.change(screen.getByLabelText('Tag name'), { target: { value: 'v2' } }); fireEvent.click(screen.getByLabelText('Annotated tag')); fireEvent.change(screen.getByLabelText('Tag message'), { target: { value: 'Release two' } });
  t.preview(); fireEvent.click(screen.getByRole('button', { name: 'Refresh context' })); expect(t.onRefresh).toHaveBeenCalledOnce();
  act(() => t.store.setState({ scope: { ...catalog.scope, generation: 'new' }, catalog: { ...catalog, scope: { ...catalog.scope, generation: 'new' }, catalogId: 'fresh' } }));
  expect(screen.getByLabelText('Tag name')).toHaveValue('v2'); expect(screen.getByLabelText('Tag message')).toHaveValue('Release two'); expect(screen.getByLabelText('Start at')).toHaveValue('ref:refs/tags/v1');
  expect(screen.queryByRole('region')).not.toBeInTheDocument(); expect(t.prepare).not.toHaveBeenCalled(); expect(screen.getByText('Repository refreshed. Prepare again.')).toBeInTheDocument();
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare action' })));
  expect(t.prepare).toHaveBeenCalledWith(expect.objectContaining({ kind: 'tagCreate', name: 'v2', annotated: true, message: 'Release two', source: { kind: 'ref', name: 'refs/tags/v1' } }));
  act(() => t.store.setState({ catalog: { ...t.store.getState().catalog!, refs: [] } }));
  expect(screen.getByRole('alert')).toHaveTextContent('missing or unavailable'); expect(screen.getByRole('button', { name: 'Prepare action' })).toBeDisabled();
});
it('S7 shows dirty, bare, linked-worktree and authoritative action errors', () => {
  const t = setup({ kind: 'branchSwitch', refName: 'refs/heads/main' });
  act(() => t.store.setState({ catalog: { ...catalog, dirty: true, refs: [{ ...catalog.refs[0], checkedOutHere: false, checkedOutElsewhere: true }] }, actionError: { code: 'InvalidHandle', phase: 'inspection', retry: 'none', message: 'Branch is checked out in another worktree.' } }));
  expect(screen.getByText(/Working tree has changes/)).toBeInTheDocument(); expect(screen.getByRole('option', { name: /another worktree/ })).toBeInTheDocument(); expect(screen.getByRole('alert')).toHaveTextContent('another worktree');
  act(() => t.store.setState({ catalog: { ...catalog, bare: true } })); expect(screen.getByText(/Bare repository/)).toBeInTheDocument();
});
it('S7 synchronizes Tasks on lost replies and disposal without blind retries or dismissing another dialog', async () => {
  const t = setup(); t.execute.mockResolvedValueOnce({ ok: false, error: { code: 'Internal', phase: 'inspection', retry: 'none', message: 'Lost reply' } } as never); t.preview();
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Apply action' })));
  expect(t.onStarted).toHaveBeenCalledOnce(); expect(t.onDismiss).not.toHaveBeenCalled(); expect(t.execute).toHaveBeenCalledOnce(); expect(screen.getByText(/reply was not confirmed/)).toBeInTheDocument();
  let finish!: (value: never) => void; t.execute.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  t.preview(); fireEvent.click(screen.getByRole('button', { name: 'Apply action' })); t.view.unmount();
  await act(async () => finish({ ok: true, value: fixture.result.operation } as never));
  expect(t.onDismiss).not.toHaveBeenCalled(); expect(t.onStarted).toHaveBeenCalledTimes(2);
});
