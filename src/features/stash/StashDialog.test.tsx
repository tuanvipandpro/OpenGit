import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { StashDialog } from './StashDialog';
import { createRepositoryStashStore } from '../../stores/repositoryStash';
import type { RepositoryStashes, PreparedAction, OperationSnapshot } from '../../contracts';
import fixture from '../../../fixtures/contracts/ref-actions-wire.json';
const scope = fixture.catalog.scope;
const catalog: RepositoryStashes = { scope, catalogId: 'catalog', observedAt: '', saveAvailable: true, entries: ['a', 'b'].map((entryId, ordinal) => ({ entryId, ordinal: String(ordinal), oid: 'a'.repeat(40), baseOid: null, includesUntracked: false, applyAvailable: true })) };
function setup() {
  const store = createRepositoryStashStore(), prepareSave = vi.fn(async () => {}), prepareApply = vi.fn(async (_id: string) => {}), execute = vi.fn(async () => ({ ok: true as const, value: fixture.result.operation as OperationSnapshot })), onDismiss = vi.fn(), onStarted = vi.fn(async () => {});
  store.setState({ scope, catalog, prepareSave, prepareApply, execute }); const view = render(<StashDialog store={store} onDismiss={onDismiss} onStarted={onStarted} onRefresh={vi.fn()} />);
  const preview = (kind: 'stashPush' | 'stashApply' = 'stashPush') => act(() => store.setState({ prepared: { scope, kind, planId: 'plan', warnings: [], expiresAt: '2099-01-01' } as PreparedAction }));
  return { store, prepareSave, prepareApply, execute, onDismiss, onStarted, view, preview };
}
it('S6 Save defaults untracked off, states ignored exclusion and requires Prepare before Execute', async () => {
  const t = setup(); fireEvent.click(screen.getByRole('button', { name: 'Save stash' })); expect(screen.getByLabelText('Include untracked files')).not.toBeChecked(); expect(screen.getByText(/ignored files are never included/)).toBeInTheDocument(); expect(screen.getByRole('button', { name: 'Execute' })).toBeDisabled();
  fireEvent.change(screen.getByLabelText('Stash message'), { target: { value: 'memory-only' } }); fireEvent.click(screen.getByLabelText('Include untracked files')); await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' })));
  expect(t.prepareSave).toHaveBeenCalledOnce(); expect(t.store.getState().message).toBe('memory-only'); expect(t.store.getState().includeUntracked).toBe(true); t.preview(); fireEvent.change(screen.getByLabelText('Stash message'), { target: { value: 'edited' } }); expect(screen.getByRole('button', { name: 'Execute' })).toBeDisabled();
  expect(screen.queryByRole('button', { name: /pop|drop|abort/i })).not.toBeInTheDocument();
});
it('S6 Apply dispatches exact ordered entry despite duplicate OIDs and refuses a replaced catalog', async () => {
  const t = setup(), entries = within(screen.getByRole('list', { name: 'Saved stashes' })).getAllByRole('listitem'); fireEvent.click(within(entries[1]).getByRole('button', { name: 'Apply stash' }));
  expect(screen.getByText(/original staged partition is not restored/)).toBeInTheDocument(); await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' }))); expect(t.prepareApply).toHaveBeenCalledWith('b');
  t.preview('stashApply'); act(() => t.store.setState({ catalog: { ...catalog, catalogId: 'new', entries: [...catalog.entries].reverse() } }));
  expect(screen.getByRole('alert')).toHaveTextContent('catalog changed'); expect(screen.getByRole('button', { name: 'Prepare' })).toBeDisabled(); expect(screen.getByRole('button', { name: 'Execute' })).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: 'Back to stashes' })); fireEvent.click(screen.getAllByRole('button', { name: 'Apply stash' })[0]); await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' }))); expect(t.prepareApply).toHaveBeenLastCalledWith('b');
});
it('S6 executes once, syncs Tasks on uncertain ACK and preserves the draft', async () => {
  const t = setup(); fireEvent.click(screen.getByRole('button', { name: 'Save stash' })); fireEvent.change(screen.getByLabelText('Stash message'), { target: { value: 'keep' } }); t.preview(); t.execute.mockResolvedValueOnce({ ok: false, error: { code: 'Internal', phase: 'publish', retry: 'none', message: 'Lost reply' } } as never);
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Execute' })); fireEvent.click(screen.getByRole('button', { name: 'Execute' })); });
  expect(t.execute).toHaveBeenCalledOnce(); expect(t.onStarted).toHaveBeenCalledOnce(); expect(t.onDismiss).not.toHaveBeenCalled(); expect(screen.getByLabelText('Stash message')).toHaveValue('keep'); expect(screen.getByText(/No automatic retry/)).toBeInTheDocument();
});
it('S6 an unmounted manager cannot dismiss a newer manager when an old execute reply arrives', async () => {
  const t = setup(); let finish!: (value: never) => void; t.execute.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; })); fireEvent.click(screen.getByRole('button', { name: 'Save stash' })); t.preview(); fireEvent.click(screen.getByRole('button', { name: 'Execute' })); t.view.unmount();
  await act(async () => finish({ ok: true, value: fixture.result.operation } as never)); expect(t.onDismiss).not.toHaveBeenCalled(); expect(t.onStarted).toHaveBeenCalledOnce();
});
