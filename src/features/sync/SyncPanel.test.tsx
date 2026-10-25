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
  expect(screen.getByText('Local tracking · Ahead: unknown · Behind: unknown')).toBeInTheDocument();
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
it('Sync exposes Push/FF-only Pull and separates dated ACK from local tracking counts', () => {
  const store = createRepositorySyncStore(), onPush = vi.fn(), onPull = vi.fn();
  const catalog = { ...wire.catalog, head: { ...wire.catalog.head, kind: 'branch' as const, branch: 'main' } };
  store.setState({ scope: catalog.scope, catalog });
  render(<SyncPanel store={store} onFetch={vi.fn()} onRefresh={vi.fn()} onPush={onPush} onPull={onPull} publication={{ remoteName: 'origin', role: 'push', destination: 'refs/heads/main', sourceRef: 'refs/heads/main', sourceOid: catalog.head.oid, acknowledgedOid: catalog.head.oid, acknowledgedAt: '2026-10-25', configDigest: 'digest', deleted: false }} />);
  fireEvent.click(screen.getByRole('button', { name: 'Push…' })); fireEvent.click(screen.getByRole('button', { name: 'Pull (FF only)…' })); expect(onPush).toHaveBeenCalledOnce(); expect(onPull).toHaveBeenCalledOnce();
  expect(screen.getByText(/Dated push ACK/)).toHaveTextContent('Local tracking counts require explicit Fetch'); expect(screen.getByText(/Local tracking · Ahead/)).toBeInTheDocument(); expect(screen.queryByText(/not pushed|never pushed/i)).not.toBeInTheDocument();
});
it('Sync permits opening the action selector for remote-only tag deletion even with unborn HEAD', () => {
  const store = createRepositorySyncStore(), onPush = vi.fn();
  store.setState({ scope: wire.catalog.scope, catalog: { ...wire.catalog, head: { kind: 'unborn', branch: 'main', oid: null }, sync: { ...wire.catalog.sync, currentBranch: null, upstream: null } } });
  render(<SyncPanel store={store} onFetch={vi.fn()} onRefresh={vi.fn()} onPush={onPush} onPull={vi.fn()} />);
  expect(screen.getByRole('button', { name: 'Push…' })).toBeEnabled(); fireEvent.click(screen.getByRole('button', { name: 'Push…' })); expect(onPush).toHaveBeenCalledOnce(); expect(screen.getByRole('button', { name: 'Pull (FF only)…' })).toBeDisabled();
});
