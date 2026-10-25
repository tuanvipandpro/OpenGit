import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { createRepositorySyncStore } from '../../stores/repositorySync';
import { FetchDialog } from './FetchDialog';
import wireFixture from '../../../fixtures/contracts/daily-wire.json';
import type { RepositoryActionCatalog, PreparedAction } from '../../contracts';
const wire = wireFixture as unknown as { catalog: RepositoryActionCatalog; prepared: PreparedAction; operation: import('../../contracts').OperationSnapshot };
import type { OperationSnapshot } from '../../contracts';
const setup = () => {
  const store = createRepositorySyncStore(), prepareFetch = vi.fn(async () => {}), executeFetch = vi.fn(async () => ({ ok: true as const, value: wire.operation as OperationSnapshot })), onDismiss = vi.fn(), onStarted = vi.fn(async () => {});
  const catalog = { ...wire.catalog, remotes: [wire.catalog.remotes[0], { ...wire.catalog.remotes[0], name: 'upstream', handle: 'second-handle' }] };
  store.setState({ scope: catalog.scope, catalog, prepareFetch, executeFetch });
  render(<FetchDialog store={store} onDismiss={onDismiss} onStarted={onStarted} />);
  return { store, prepareFetch, executeFetch, onDismiss, onStarted, catalog };
};
it('AC2 prepares selected handles with prune off, requires preview and allows explicit all/prune', async () => {
  const test = setup();
  expect(screen.getByRole('dialog', { name: 'Fetch remotes' })).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Start fetch' })).not.toBeInTheDocument();
  expect(screen.getByRole('checkbox', { name: 'Prune deleted remote-tracking branches' })).not.toBeChecked();
  fireEvent.click(screen.getByRole('checkbox', { name: 'Fetch origin' }));
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare fetch' })));
  expect(test.prepareFetch).toHaveBeenLastCalledWith([test.catalog.remotes[0].handle], false);
  fireEvent.click(screen.getByRole('button', { name: 'Select all remotes' }));
  fireEvent.click(screen.getByRole('checkbox', { name: 'Prune deleted remote-tracking branches' }));
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare fetch' })));
  expect(test.prepareFetch).toHaveBeenLastCalledWith(test.catalog.remotes.map(remote => remote.handle), true);
  act(() => test.store.setState({ prepared: { ...wire.prepared, expiresAt: '2099-01-01T00:00:00Z' } }));
  expect(within(screen.getByRole('region', { name: 'Fetch preview' })).getByText(/origin, upstream · Prune enabled/)).toBeInTheDocument();
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Start fetch' })); fireEvent.click(screen.getByRole('button', { name: 'Start fetch' })); });
  expect(test.executeFetch).toHaveBeenCalledOnce(); expect(test.onStarted).toHaveBeenCalledOnce(); expect(test.onDismiss).toHaveBeenCalledOnce();
});
it('AC2 explains unsupported remotes, prevents misleading fetch-all and invalidates preview on option edits', () => {
  const test = setup();
  act(() => test.store.setState({ catalog: { ...test.catalog, remotes: [...test.catalog.remotes, { name: 'mirror', handle: null, fetch: 'mirror' }] } }));
  expect(screen.getByRole('checkbox', { name: 'Fetch mirror' })).toBeDisabled();
  expect(screen.getByRole('button', { name: 'Select all remotes' })).toBeDisabled();
  expect(screen.getByText('Mirror remotes are not supported.')).toBeInTheDocument();
  act(() => test.store.setState({ prepared: { ...wire.prepared, expiresAt: '2099-01-01T00:00:00Z' } }));
  fireEvent.click(screen.getByRole('checkbox', { name: 'Fetch origin' }));
  expect(screen.queryByRole('region', { name: 'Fetch preview' })).not.toBeInTheDocument();
});
it('AC2 synchronizes Tasks after an uncertain execute reply without auto retrying', async () => {
  const test = setup(); test.executeFetch.mockResolvedValueOnce({ ok: false, error: { code: 'Internal', phase: 'transfer', retry: 'none', message: 'Unknown reply' } } as never);
  act(() => test.store.setState({ prepared: { ...wire.prepared, expiresAt: '2099-01-01T00:00:00Z' } }));
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Start fetch' })));
  expect(test.onStarted).toHaveBeenCalledOnce(); expect(test.onDismiss).not.toHaveBeenCalled(); expect(test.executeFetch).toHaveBeenCalledOnce();
});

it('AC8 does not dismiss a newer surface after an old fetch dialog unmounts during execute', async () => {
  let finish!: () => void;
  const store = createRepositorySyncStore(), onDismiss = vi.fn(), onStarted = vi.fn(async () => {});
  store.setState({ scope: wire.catalog.scope, catalog: wire.catalog, prepared: { ...wire.prepared, expiresAt: '2099-01-01T00:00:00Z' }, executeFetch: async () => { await new Promise<void>(resolve => { finish = resolve; }); return { ok: true, value: wire.operation }; } });
  const view = render(<FetchDialog store={store} onDismiss={onDismiss} onStarted={onStarted} />);
  fireEvent.click(screen.getByRole('button', { name: 'Start fetch' })); view.unmount();
  await act(async () => finish());
  expect(onStarted).toHaveBeenCalledOnce(); expect(onDismiss).not.toHaveBeenCalled();
});

it('S3-FORM01 keeps remote-name/prune input through catalog changes and requires removing missing selections', async () => {
  const test = setup();
  fireEvent.click(screen.getByRole('checkbox', { name: 'Fetch origin' }));
  fireEvent.click(screen.getByRole('checkbox', { name: 'Prune deleted remote-tracking branches' }));
  act(() => test.store.setState({ scope: { ...test.catalog.scope, generation: 'new-generation' }, catalog: { ...test.catalog, scope: { ...test.catalog.scope, generation: 'new-generation' }, snapshotId: 'new-catalog', remotes: [{ ...test.catalog.remotes[0], handle: 'fresh-origin-handle' }] } }));
  expect(screen.getByText('Repository refreshed. Prepare again.')).toBeInTheDocument();
  expect(screen.getByRole('checkbox', { name: 'Fetch origin' })).toBeChecked(); expect(screen.getByRole('checkbox', { name: 'Prune deleted remote-tracking branches' })).toBeChecked();
  expect(test.prepareFetch).not.toHaveBeenCalled();
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare fetch' })));
  expect(test.prepareFetch).toHaveBeenCalledWith(['fresh-origin-handle'], true);
  act(() => test.store.setState({ catalog: { ...test.store.getState().catalog!, snapshotId: 'remote-removed', remotes: [] } }));
  expect(screen.getByRole('alert')).toHaveTextContent('Selected remote origin is missing or unavailable.');
  expect(screen.getByRole('button', { name: 'Prepare fetch' })).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: 'Remove origin selection' })); expect(screen.queryByRole('alert')).not.toBeInTheDocument();
});
