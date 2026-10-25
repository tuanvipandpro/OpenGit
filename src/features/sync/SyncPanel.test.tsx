import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { createRepositorySyncStore } from '../../stores/repositorySync';
import { SyncPanel } from './SyncPanel';
import wireFixture from '../../../fixtures/contracts/daily-wire.json';
import type { RepositoryActionCatalog, PreparedAction } from '../../contracts';
const wire = wireFixture as unknown as { catalog: RepositoryActionCatalog; prepared: PreparedAction; operation: import('../../contracts').OperationSnapshot };

it('AC1 shows local current/upstream counts, unknown shallow counts and no live-server claims', () => {
  const store = createRepositorySyncStore(), onFetch = vi.fn(), onRefresh = vi.fn();
  store.setState({ scope: wire.catalog.scope, catalog: { ...wire.catalog, sync: { ...wire.catalog.sync, ahead: null, behind: null, completeness: 'shallow' } } });
  render(<SyncPanel store={store} onFetch={onFetch} onRefresh={onRefresh} />);
  expect(screen.getByText('Current branch: refs/heads/main')).toBeInTheDocument();
  expect(screen.getByText('Upstream: refs/remotes/origin/main')).toBeInTheDocument();
  expect(screen.getByText('Ahead: unknown · Behind: unknown')).toBeInTheDocument();
  expect(screen.getByText(/Local tracking snapshot/)).toHaveTextContent('ancestry shallow');
  fireEvent.click(screen.getByRole('button', { name: 'Fetch…' })); expect(onFetch).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole('button', { name: 'Refresh sync' })); expect(onRefresh).toHaveBeenCalledOnce();
});
it('AC2 disables fetch for retained context and retry does not auto-fetch', () => {
  const store = createRepositorySyncStore(), reload = vi.fn();
  store.setState({ scope: { ...wire.catalog.scope, generation: 'changed' }, catalog: wire.catalog, catalogLoading: true, reload });
  render(<SyncPanel store={store} onFetch={vi.fn()} onRefresh={vi.fn()} />);
  expect(screen.getByRole('button', { name: 'Fetch…' })).toBeDisabled();
  expect(screen.getByText(/Previously loaded repository context/)).toBeInTheDocument(); expect(reload).not.toHaveBeenCalled();
});
