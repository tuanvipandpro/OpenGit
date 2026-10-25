import { expect, it, vi } from 'vitest';
import { createRepositoryWatch } from './repositoryWatch';
import type { RepositorySubscribe } from './repositoryWatch';
import type { Api } from '../../stores/workspace';
import type { HistoryScope, RepositoryChangedEvent, RepositoryWatch } from '../../contracts';

const scope: HistoryScope = { repoId: 'repo', worktreeId: 'tree', generation: '1' };
const deferred = <T,>() => { let resolve!: (value: T) => void; let reject!: (error?: unknown) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const tick = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
function harness() {
  let receive!: (event: RepositoryChangedEvent) => void;
  const listener = deferred<() => void>(), stop = vi.fn(), refresh = vi.fn();
  const activations: { scope: HistoryScope; result: ReturnType<typeof deferred<unknown>> }[] = [];
  const api = vi.fn(async (command: string, request: Record<string, unknown>) => {
    if (command === 'unwatch_repository') return { ok: true, value: null };
    const result = deferred<unknown>(); activations.push({ scope: request.scope as HistoryScope, result }); return result.promise;
  }) as unknown as Api;
  const subscribe = vi.fn<RepositorySubscribe>(handler => { receive = handler; return listener.promise; });
  const watch = createRepositoryWatch(refresh, api, subscribe);
  const ack = (index = 0, sequence = '0', status: RepositoryWatch['status'] = 'watching') => activations[index].result.resolve({ ok: true, value: { scope: activations[index].scope, sequence, subscriptionId: `token-${index}`, status } });
  return { watch, api, refresh, listener, stop, subscribe, activations, ack,
    emit: (sequence = '1', reason: RepositoryChangedEvent['reason'] = 'changed', subscriptionId = 'token-0', eventScope = scope) => receive({ scope: eventScope, sequence, reason, subscriptionId }) };
}
it('listens before activation, drains pre-ACK hints and compares large counters without precision loss', async () => {
  const h = harness(); h.watch.bind(scope); await tick(); expect(h.activations).toHaveLength(0);
  h.listener.resolve(h.stop); await tick(); expect(h.activations).toHaveLength(1);
  h.emit('9007199254740993'); h.emit('9007199254740993'); h.emit('9007199254740994', 'changed', 'wrong');
  h.ack(0, '9007199254740992'); await tick(); expect(h.refresh).toHaveBeenCalledTimes(1);
  h.emit('9007199254740992'); h.emit('9007199254740994'); expect(h.refresh).toHaveBeenCalledTimes(2);
  h.emit('9007199254740995', 'changed', 'token-0', { ...scope, generation: '2' }); expect(h.refresh).toHaveBeenCalledTimes(2);
  h.watch.dispose(); expect(h.stop).toHaveBeenCalledOnce(); expect(h.api).toHaveBeenCalledWith('unwatch_repository', { subscriptionId: 'token-0' });
});
it('refreshes authoritatively for sequence gaps, malformed matching events and overflow', async () => {
  const h = harness(); h.watch.bind(scope); h.listener.resolve(h.stop); await tick(); h.ack(); await tick();
  h.emit('3'); h.emit('3'); expect(h.refresh).toHaveBeenCalledTimes(1);
  h.emit('03'); h.emit('18446744073709551616'); h.emit('4', 'overflow'); expect(h.refresh).toHaveBeenCalledTimes(4);
  h.watch.dispose();
});
it('bounds the pre-ACK queue and requests recovery when hints were dropped', async () => {
  const h = harness(); h.watch.bind(scope); h.listener.resolve(h.stop); await tick();
  for (let i = 0; i < 300; i++) h.emit(String(i + 1));
  h.ack(0, '300'); await tick(); expect(h.refresh).toHaveBeenCalledOnce(); h.watch.dispose();
});
it('keeps fallback ACK active and suppresses unavailable rebind loops until changed recovery or selection change', async () => {
  const h = harness(); h.watch.bind(scope); h.listener.resolve(h.stop); await tick(); h.ack(0, '0', 'fallback'); await tick();
  h.emit('1', 'unavailable'); expect(h.refresh).toHaveBeenCalledOnce();
  const next = { ...scope, generation: '2' }; h.watch.bind(next); await tick(); h.ack(1, '0', 'fallback'); await tick();
  h.emit('1', 'unavailable', 'token-1', next); h.watch.retry(); await tick();
  expect(h.refresh).toHaveBeenCalledOnce(); expect(h.activations).toHaveLength(2);
  h.emit('2', 'changed', 'token-1', next); h.emit('3', 'unavailable', 'token-1', next); expect(h.refresh).toHaveBeenCalledTimes(3);
  const linked = { ...next, worktreeId: 'linked' }; h.watch.bind(linked); await tick(); h.ack(2, '0', 'fallback'); await tick();
  h.emit('1', 'unavailable', 'token-2', linked); expect(h.refresh).toHaveBeenCalledTimes(4); h.watch.dispose();
});
it('cleans up exact late ACKs after rebinding and disposal without exposing old events', async () => {
  const h = harness(); h.watch.bind(scope); h.listener.resolve(h.stop); await tick();
  const next = { ...scope, repoId: 'next' }; h.watch.bind(next); await tick(); h.ack(); await tick();
  expect(h.api).toHaveBeenCalledWith('unwatch_repository', { subscriptionId: 'token-0' });
  h.emit(); expect(h.refresh).not.toHaveBeenCalled(); h.watch.dispose(); h.ack(1); await tick();
  expect(h.api).toHaveBeenCalledWith('unwatch_repository', { subscriptionId: 'token-1' }); expect(h.stop).toHaveBeenCalledOnce();
});
it('cleans up a late listener and never activates after disposal', async () => {
  const h = harness(); h.watch.bind(scope); h.watch.dispose(); h.listener.resolve(h.stop); await tick();
  expect(h.stop).toHaveBeenCalledOnce(); expect(h.activations).toHaveLength(0);
});
it('does not spin on listen/activation failure and retries only when requested', async () => {
  const refresh = vi.fn(), receive = vi.fn(), stop = vi.fn();
  const subscribe = vi.fn<RepositorySubscribe>().mockRejectedValueOnce(Error('unavailable')).mockResolvedValue(stop);
  const api = vi.fn().mockResolvedValueOnce({ ok: false, error: {} }).mockResolvedValueOnce({ ok: true, value: { scope, subscriptionId: 'recovered', sequence: '0', status: 'watching' } }).mockResolvedValue({ ok: true, value: null }) as unknown as Api;
  const watch = createRepositoryWatch(refresh, api, subscribe); watch.bind(scope); await tick();
  expect(api).not.toHaveBeenCalled(); watch.retry(); await tick(); expect(api).toHaveBeenCalledOnce();
  expect(subscribe).toHaveBeenCalledTimes(2); await tick(); expect(api).toHaveBeenCalledOnce();
  watch.retry(); await tick(); expect(api).toHaveBeenCalledTimes(2); expect(refresh).not.toHaveBeenCalled();
  expect(receive).not.toHaveBeenCalled(); watch.dispose();
});
