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
