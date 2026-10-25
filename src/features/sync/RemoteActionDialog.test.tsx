import { act, fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { RemoteActionDialog } from './RemoteActionDialog';
import { createRepositoryTransportStore } from '../../stores/repositoryTransport';
import type { TransportDraft, TransportLaunch } from '../../stores/repositoryTransport';
import { createRepositoryActionsStore } from '../../stores/repositoryActions';
import { createRepositorySyncStore } from '../../stores/repositorySync';
import type { OperationSnapshot, PreparedAction, RefActionCatalog, RemoteActionCatalog, RemoteRefSnapshot } from '../../contracts';
import fixture from '../../../fixtures/contracts/ref-actions-wire.json';
import daily from '../../../fixtures/contracts/daily-wire.json';
const scope = fixture.catalog.scope;
const remoteCatalog: RemoteActionCatalog = { scope, observedAt: '2026-10-25', remotes: ['origin', 'backup'].map(name => ({ name, endpoints: [{ role: 'fetch', handle: `${name}-fetch`, capability: 'eligible' }, { role: 'push', handle: `${name}-push`, capability: 'eligible' }] })) };
const snapshot: RemoteRefSnapshot = { capturedScope: scope, snapshotId: 'observation', remoteName: 'origin', role: 'push', observedAt: '2026-10-25T12:00:00Z', expiresAt: '2099-01-01', objectFormat: 'sha1', refs: [{ entryId: 'remote-main', name: 'refs/heads/main', oid: 'a'.repeat(40) }, { entryId: 'remote-tag', name: 'refs/tags/server-only', oid: 'b'.repeat(40) }, { entryId: 'remote-local-tag', name: 'refs/tags/v1', oid: 'c'.repeat(40) }] };
function setup(launch: TransportLaunch = { kind: 'push' }) {
  const store = createRepositoryTransportStore(), refs = createRepositoryActionsStore(), sync = createRepositorySyncStore();
  const begin = vi.fn(async () => {}), prepare = vi.fn(async (_draft: TransportDraft) => {}), readRemoteRefs = vi.fn(async (_name: string) => {}), execute = vi.fn(async () => ({ ok: true as const, value: fixture.result.operation as OperationSnapshot }));
  store.setState({ scope, catalog: remoteCatalog, begin, prepare, readRemoteRefs, execute });
  refs.setState({ scope, catalog: { ...fixture.catalog as RefActionCatalog, refs: [...fixture.catalog.refs as RefActionCatalog['refs'], { ...fixture.catalog.refs[0], entryId: 'dev', name: 'refs/heads/dev', kind: 'local' }] } });
  sync.setState({ scope, catalog: { ...daily.catalog as import('../../contracts').RepositoryActionCatalog, scope } });
  const onDismiss = vi.fn(), onStarted = vi.fn(async () => {}), onRefresh = vi.fn();
  const view = render(<RemoteActionDialog store={store} refs={refs} sync={sync} launch={launch} onDismiss={onDismiss} onStarted={onStarted} onRefresh={onRefresh} />);
  const observe = (value = snapshot) => act(() => store.setState({ snapshot: value }));
  const preview = (kind = launch.kind) => act(() => store.setState({ prepared: { scope, kind, planId: 'preview', expiresAt: '2099-01-01', warnings: ['externalRace'] } as PreparedAction, preview: { source: 'refs/heads/main', destination: 'origin: refs/heads/main' } }));
  return { store, refs, sync, begin, prepare, readRemoteRefs, execute, onDismiss, onStarted, onRefresh, view, observe, preview };
}
it('Sync UI requires an explicit push-role read then prepares the qualified source/destination without upstream setup', async () => {
  const t = setup(); expect(screen.getByLabelText('Source branch')).toHaveValue('refs/heads/main'); expect(screen.getByRole('button', { name: 'Prepare' })).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: 'Read remote references' })); expect(t.readRemoteRefs).toHaveBeenCalledWith('origin'); expect(t.prepare).not.toHaveBeenCalled();
  t.observe(); expect(screen.getByLabelText('Destination')).toHaveValue('ref:refs/heads/main'); expect(screen.getByText(/Remote observation:/)).toHaveTextContent(snapshot.observedAt);
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' })));
  expect(t.prepare).toHaveBeenCalledWith({ kind: 'push', sourceRef: 'refs/heads/main', remoteName: 'origin', entryId: 'remote-main', newName: 'main' });
  expect(screen.getByText(/Automatic upstream setup is unavailable/)).toBeInTheDocument(); expect(screen.queryByRole('checkbox', { name: /upstream/i })).not.toBeInTheDocument();
  t.preview(); expect(screen.getByRole('region', { name: 'Remote action preview' })).toHaveTextContent('Source: refs/heads/main'); expect(screen.getByText('external Race')).toBeInTheDocument();
});
it('Sync UI clears the old destination on source/remote changes and keeps explicit missing names after a replacement observation', async () => {
  const t = setup(); t.observe(); t.preview();
  fireEvent.change(screen.getByLabelText('Source branch'), { target: { value: 'refs/heads/dev' } });
  expect(screen.getByLabelText('Destination')).toHaveValue('new'); expect(screen.getByLabelText('New destination name')).toHaveValue('dev'); expect(screen.queryByRole('region')).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('Destination'), { target: { value: 'ref:refs/heads/main' } });
  t.observe({ ...snapshot, snapshotId: 'replaced', refs: [] });
  expect(screen.getByLabelText('Destination')).toHaveValue('ref:refs/heads/main'); expect(screen.getByText(/selected destination is missing/)).toBeInTheDocument(); expect(screen.getByRole('button', { name: 'Prepare' })).toBeDisabled();
  fireEvent.change(screen.getByLabelText('Remote'), { target: { value: 'backup' } });
  expect(t.store.getState().snapshot).toBeNull(); expect(screen.getByRole('button', { name: 'Prepare' })).toBeDisabled();
});
it('Sync UI exposes remote-only tag delete and confirms the exact observed raw OID after preparation', async () => {
  const t = setup(); act(() => t.refs.setState({ catalog: { ...t.refs.getState().catalog!, head: { kind: 'unborn', branch: 'main', oid: null }, refs: [] } }));
  fireEvent.change(screen.getByLabelText('Remote action'), { target: { value: 'remoteTagDelete' } }); t.observe();
  expect(screen.queryByLabelText('Source tag')).not.toBeInTheDocument(); expect(screen.getByRole('button', { name: 'Prepare' })).toBeDisabled();
  fireEvent.change(screen.getByLabelText('Destination'), { target: { value: 'ref:refs/tags/server-only' } });
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' })));
  expect(t.prepare).toHaveBeenCalledWith(expect.objectContaining({ kind: 'remoteTagDelete', entryId: 'remote-tag' }));
  t.preview('remoteTagDelete'); const confirmation = screen.getByLabelText(/Confirm remote tag deletion of refs\/tags\/server-only/);
  expect(confirmation.closest('label')).toHaveTextContent('b'.repeat(40)); expect(screen.getByRole('button', { name: 'Execute' })).toBeDisabled();
  fireEvent.click(confirmation); expect(screen.getByRole('button', { name: 'Execute' })).toBeEnabled();
  t.observe({ ...snapshot, snapshotId: 'replaced', refs: snapshot.refs.map(ref => ({ ...ref, oid: 'd'.repeat(40) })) });
  expect(screen.getByRole('button', { name: 'Execute' })).toBeDisabled(); expect(screen.queryByRole('region')).not.toBeInTheDocument();
});
it('Sync UI pull uses only the current configured upstream and blocks stale upstream data', async () => {
  const t = setup({ kind: 'pull' });
  expect(screen.queryByRole('button', { name: 'Read remote references' })).not.toBeInTheDocument(); expect(screen.getByText(/No autostash, rebase or merge commit/)).toBeInTheDocument();
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' })));
  expect(t.prepare).toHaveBeenCalledWith(expect.objectContaining({ kind: 'pull', sourceRef: daily.catalog.sync.upstream }));
  act(() => t.sync.setState({ scope, catalog: { ...t.sync.getState().catalog!, scope: { ...scope, generation: 'old' } } })); expect(screen.getByRole('button', { name: 'Prepare' })).toBeDisabled();
});
it('Sync UI explains unsupported endpoints, failed read, missing source and expired observation', () => {
  const t = setup(); act(() => t.store.setState({ catalog: { ...remoteCatalog, remotes: [{ name: 'origin', endpoints: [{ role: 'push', capability: 'multipleUrls', handle: null }] }] }, actionError: { code: 'AuthenticationFailed', phase: 'authentication', retry: 'checkCredentials', message: 'Remote authentication was refused.' } }));
  expect(screen.getByRole('button', { name: 'Read remote references' })).toBeDisabled(); expect(screen.getAllByRole('alert').some(alert => alert.textContent?.includes('multiple push URLs'))).toBe(true); expect(screen.getByText('Remote authentication was refused.')).toBeInTheDocument();
  act(() => t.store.setState({ catalog: remoteCatalog, actionError: null })); t.observe({ ...snapshot, expiresAt: '2000-01-01' }); expect(screen.getByText(/Remote observation expired/)).toBeInTheDocument(); expect(screen.getByRole('button', { name: 'Prepare' })).toBeDisabled();
});
it('Sync UI executes once and synchronizes Tasks on lost reply without retry or stale dismiss after unmount', async () => {
  const t = setup(); t.observe(); t.preview(); let finish!: (value: never) => void;
  t.execute.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  fireEvent.click(screen.getByRole('button', { name: 'Execute' })); fireEvent.click(screen.getByRole('button', { name: 'Execute' })); expect(t.execute).toHaveBeenCalledOnce();
  t.view.unmount(); await act(async () => finish({ ok: true, value: fixture.result.operation } as never)); expect(t.onDismiss).not.toHaveBeenCalled(); expect(t.onStarted).toHaveBeenCalledOnce();
  const next = setup(); next.observe(); next.preview(); next.execute.mockResolvedValueOnce({ ok: false, error: { code: 'Internal', phase: 'transfer', retry: 'none', message: 'Lost reply' } } as never);
  await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Execute' })));
  expect(next.onStarted).toHaveBeenCalledOnce(); expect(next.onDismiss).not.toHaveBeenCalled(); expect(next.execute).toHaveBeenCalledOnce(); expect(screen.getByText(/will not retry automatically/)).toBeInTheDocument();
});
