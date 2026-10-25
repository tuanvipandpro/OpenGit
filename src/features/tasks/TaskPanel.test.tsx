import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { it, expect, vi } from 'vitest';
import type { OperationSnapshot, OperationEvent, OperationsSnapshot } from '../../contracts';
import type { Api } from '../../stores/workspace';
import { createOperationsStore } from '../../stores/operations';
import { TaskPanel } from './TaskPanel';
import fixture from '../../../fixtures/contracts/wire.json';
const operation = fixture.event.operation as unknown as OperationSnapshot;
it('collapses tasks and requests cancellation only when the backend permits it', async () => {
  const user = userEvent.setup(), cancel = vi.fn(); render(<TaskPanel operations={[operation]} onCancel={cancel} />);
  await user.click(screen.getByRole('button', { name: 'Cancel task' })); expect(cancel).toHaveBeenCalledWith(operation.id);
  await user.click(screen.getByRole('button', { name: /Tasks/ })); expect(screen.queryByText('Clone repository')).not.toBeInTheDocument();
});
it('subscribes before snapshot and applies only buffered events after its fence', async () => {
  let receive!: (event: OperationEvent) => void; const order: string[] = [];
  const api = vi.fn(async () => { order.push('snapshot'); receive({ streamId: 'stream', sequence: '4', operation: { ...operation, sequence: '4', state: 'Running' } }); receive({ streamId: 'stream', sequence: '3', operation }); return { ok: true, value: { streamId: 'stream', sequence: '3', operations: [operation] } }; }) as unknown as Api;
  const store = createOperationsStore(api, async handler => { order.push('subscribe'); receive = handler; return () => {}; });
  const stop = await store.getState().start(); expect(order).toEqual(['subscribe','snapshot']); expect(store.getState().snapshot?.sequence).toBe('4'); expect(store.getState().snapshot?.operations[0].state).toBe('Running'); stop();
});
it('resynchronizes gaps, restarts and reordered events with authoritative snapshots', async () => {
  let receive!: (event: OperationEvent) => void;
  let next: OperationsSnapshot = { streamId: 'first', sequence: '1', operations: [operation] };
  const api = vi.fn(async () => ({ ok: true, value: next })) as unknown as Api;
  const store = createOperationsStore(api, async handler => { receive = handler; return () => {}; }); const stop = await store.getState().start();
  next = { streamId: 'first', sequence: '4', operations: [{ ...operation, sequence: '4', state: 'Failed' }] };
  receive({ streamId: 'first', sequence: '4', operation: { ...operation, sequence: '4', state: 'Succeeded' } }); await store.getState().sync();
  expect(store.getState().snapshot?.operations[0].state).toBe('Failed');
  next = { streamId: 'restarted', sequence: '0', operations: [] }; receive({ streamId: 'restarted', sequence: '0', operation }); await store.getState().sync();
  expect(store.getState().snapshot).toEqual(next); stop();
});
it('polls after subscription failure and stops polling after disposal', async () => {
  vi.useFakeTimers(); const api = vi.fn(async () => ({ ok: true, value: { streamId: 'stream', sequence: '0', operations: [] } })) as unknown as Api;
  const store = createOperationsStore(api, async () => { throw Error('CANARY transport'); }); const stop = await store.getState().start();
  expect(store.getState().subscriptionFailed).toBe(true); await vi.advanceTimersByTimeAsync(2000); expect(api).toHaveBeenCalledTimes(2); stop(); await vi.advanceTimersByTimeAsync(2000); expect(api).toHaveBeenCalledTimes(2); vi.useRealTimers();
});

it('AC2 renders fetch partial/unknown evidence independently from clone retry/success wording', () => {
  const action = { ...operation, id: 'fetch-op', kind: 'action' as const, actionKind: 'fetch' as const, state: 'Failed' as const,
    outcome: { effects: 'partial' as const, recovery: 'inspect' as const, cancellationRequested: true, repositoryChanged: true, observedAt: '2026-01-01' } };
  render(<TaskPanel operations={[action]} onCancel={vi.fn()} onRetry={vi.fn()} onOpen={vi.fn()} outcomes={{ [action.id]: { operation: action, fetch: { prune: true, remotes: [
    { name: 'origin', phase: 'observed', effects: 'applied', error: null, reportedRefs: [{ name: 'refs/remotes/origin/new', oldOid: null, newOid: 'abc', status: 'created' }], observedAt: '2026-01-01' },
    { name: 'upstream', phase: 'started', effects: 'unknown', error: null, reportedRefs: [], observedAt: null },
    { name: 'third', phase: 'pending', effects: 'notApplied', error: null, reportedRefs: [], observedAt: null },
  ] } } }} />);
  expect(screen.getByRole('article', { name: 'Fetch remotes task' })).toBeInTheDocument();
  expect(screen.getByText('Some changes may have been applied')).toBeInTheDocument();
  expect(screen.getByText(/Cancellation was requested/)).toBeInTheDocument();
  expect(screen.getByRole('list', { name: 'Fetch remote results' })).toHaveTextContent('refs/remotes/origin/new');
  expect(screen.getByText('The attempt started; its final effects are uncertain.')).toBeInTheDocument();
  expect(screen.getByText('This remote was not started.')).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Retry clone' })).not.toBeInTheDocument();
  expect(screen.queryByText('Repository added to workspace')).not.toBeInTheDocument();
});

it('AC2 null fetch ledger keeps unknown effects and offers result inspection, never blind mutation retry', async () => {
  const user = userEvent.setup(), inspect = vi.fn();
  const action = { ...operation, id: 'unknown-op', kind: 'action' as const, actionKind: 'fetch' as const, state: 'Interrupted' as const,
    outcome: { effects: 'unknown' as const, recovery: 'inspect' as const, cancellationRequested: false, repositoryChanged: true, observedAt: '2026-01-01' } };
  render(<TaskPanel operations={[action]} onCancel={vi.fn()} onRetry={vi.fn()} onInspect={inspect} outcomes={{ [action.id]: { operation: action, fetch: null } }} />);
  expect(screen.getByText('Effects unknown; inspect the repository before retrying')).toBeInTheDocument();
  expect(screen.getByText(/Per-remote fetch evidence is unavailable/)).toHaveTextContent('does not establish that no changes occurred');
  await user.click(screen.getByRole('button', { name: 'Reload action result' })); expect(inspect).toHaveBeenCalledWith(action.id);
  expect(screen.queryByText('No changes applied')).not.toBeInTheDocument();
});

it('AC2 loads terminal action ledgers once concurrently and rejects stale/wrong operation echoes', async () => {
  const action: OperationSnapshot = { ...operation, id: 'action', kind: 'action', actionKind: 'fetch', sequence: '12', state: 'Failed' };
  let finish!: (value: unknown) => void;
  const api = vi.fn(async () => new Promise(resolve => { finish = resolve; })) as unknown as Api;
  const store = createOperationsStore(api); store.setState({ snapshot: { streamId: 'stream', sequence: '12', operations: [action] } });
  const first = store.getState().loadOutcome(action.id), second = store.getState().loadOutcome(action.id);
  expect(api).toHaveBeenCalledTimes(1);
  finish({ ok: true, value: { operation: { ...action, id: 'forged' }, fetch: null } }); await Promise.all([first, second]);
  expect(store.getState().outcomes[action.id]).toBeUndefined();
  const third = store.getState().loadOutcome(action.id);
  store.setState({ snapshot: { streamId: 'stream', sequence: '13', operations: [{ ...action, sequence: '13' }] } });
  finish({ ok: true, value: { operation: action, fetch: null } }); await third;
  expect(store.getState().outcomes[action.id]).toBeUndefined();
  const fourth = store.getState().loadOutcome(action.id);
  finish({ ok: true, value: { operation: { ...action, sequence: '13' }, fetch: null } }); await fourth;
  expect(store.getState().outcomes[action.id].fetch).toBeNull();
});

it('S3-LAYOUT01 keeps three active/failed/partial task results and controls discoverable without automatic collapse', async () => {
  const user = userEvent.setup(), cancel = vi.fn();
  const active: OperationSnapshot = { ...operation, id: 'layout-active', kind: 'action', actionKind: 'fetch', state: 'Running', canCancel: true };
  const failed: OperationSnapshot = { ...active, id: 'layout-failed', state: 'Failed', canCancel: false, error: { code: 'Network', phase: 'transfer', retry: 'checkNetwork', message: 'Remote connection failed.' } };
  const partial: OperationSnapshot = { ...failed, id: 'layout-partial', error: null, outcome: { effects: 'partial', recovery: 'inspect', cancellationRequested: true, repositoryChanged: true, observedAt: '2026-01-01' } };
  const view = render(<TaskPanel operations={[active, failed, partial]} onCancel={cancel} />);
  expect(screen.getByRole('button', { name: /Tasks 3/ })).toHaveAttribute('aria-expanded', 'true');
  expect(screen.getAllByRole('article', { name: 'Fetch remotes task' })).toHaveLength(3);
  expect(screen.getByText('Remote connection failed.')).toBeInTheDocument();
  expect(screen.getByText('Some changes may have been applied')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Cancel task' })); expect(cancel).toHaveBeenCalledWith(active.id);
  await user.click(screen.getByRole('button', { name: /Tasks 3/ }));
  view.rerender(<TaskPanel operations={[{ ...active, state: 'Succeeded', canCancel: false }, failed, partial]} onCancel={cancel} />);
  expect(screen.getByRole('button', { name: /Tasks 3/ })).toHaveAttribute('aria-expanded', 'false');
  await user.click(screen.getByRole('button', { name: /Tasks 3/ }));
  expect(screen.getAllByRole('article', { name: 'Fetch remotes task' })).toHaveLength(3);
  expect(screen.getByText('Remote connection failed.')).toBeInTheDocument();
});
