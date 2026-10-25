import { listen } from '@tauri-apps/api/event';
import { call, REPOSITORY_CHANGED_EVENT } from '../../contracts';
import type { HistoryScope, RepositoryChangedEvent, RepositoryWatch } from '../../contracts';
import type { Api } from '../../stores/workspace';

export type RepositorySubscribe = (receive: (event: RepositoryChangedEvent) => void) => Promise<() => void>;
const subscribe: RepositorySubscribe = receive => listen<RepositoryChangedEvent>(REPOSITORY_CHANGED_EVENT, event => receive(event.payload));
const same = (a: HistoryScope | null, b: HistoryScope | null) => !!a && !!b && a.repoId === b.repoId && a.worktreeId === b.worktreeId && a.generation === b.generation;
const selectionKey = (scope: HistoryScope) => JSON.stringify([scope.repoId, scope.worktreeId]);
const counter = (value: unknown): bigint | null => {
  if (typeof value !== 'string' || !/^(0|[1-9]\d{0,19})$/.test(value)) return null;
  const number = BigInt(value); return number <= 18_446_744_073_709_551_615n ? number : null;
};

/** Hints only: all reads run through the workspace coordinator. No retry timer. */
export function createRepositoryWatch(refresh: () => void, api: Api = call, subscribeToEvents: RepositorySubscribe = subscribe) {
  let disposed = false, ticket = 0, scope: HistoryScope | null = null, key: string | null = null;
  let degraded = false, binding = false, active: RepositoryWatch | null = null, sequence = 0n;
  let buffer: RepositoryChangedEvent[] = [], overflow = false;
  let unlisten: (() => void) | null = null, listening: Promise<void> | null = null;
  const unwatch = (id: string) => { void api('unwatch_repository', { subscriptionId: id }).catch(() => {}); };
  const invalidate = () => {
    ticket++; binding = false; buffer = []; overflow = false;
    if (active) unwatch(active.subscriptionId);
    active = null;
  };
  const receive = (event: RepositoryChangedEvent) => {
    if (disposed || !event || !same(scope, event.scope)) return;
    if (!active) {
      if (!binding) return;
      if (buffer.length < 256) buffer.push(event); else overflow = true;
      return;
    }
    if (event.subscriptionId !== active.subscriptionId) return;
    const next = counter(event.sequence);
    if (next === null || !['changed', 'overflow', 'unavailable'].includes(event.reason)) { refresh(); return; }
    if (next <= sequence) return;
    sequence = next;
    if (event.reason === 'unavailable') {
      if (!degraded) { degraded = true; refresh(); }
    } else { if (event.reason === 'changed') degraded = false; refresh(); }
  };
  const listenFirst = (): Promise<void> => {
    if (unlisten) return Promise.resolve();
    if (listening) return listening;
    const operation = (async () => {
      const stop = await subscribeToEvents(receive);
      if (disposed) stop(); else unlisten = stop;
    })();
    listening = operation;
    void operation.finally(() => { if (listening === operation) listening = null; }).catch(() => {});
    return operation;
  };
  const retry = () => {
    if (disposed || !scope || active || binding) return;
    const version = ticket, captured = scope; binding = true;
    void (async () => {
      try {
        await listenFirst();
        if (disposed || version !== ticket) return;
        const result = await api('watch_repository', { scope: captured });
        if (!result.ok) return;
        const ack = result.value;
        if (disposed || version !== ticket) { unwatch(ack.subscriptionId); return; }
        const start = counter(ack.sequence);
        if (!same(captured, ack.scope) || !ack.subscriptionId || start === null || !['watching', 'fallback'].includes(ack.status)) { unwatch(ack.subscriptionId); return; }
        active = ack; sequence = start;
        const pending = buffer; buffer = [];
        const dropped = overflow; overflow = false;
        for (const event of pending) receive(event);
        if (dropped) refresh();
      } catch { /* Inspection/history and the existing fallback remain usable. */ }
      finally { if (version === ticket) { binding = false; buffer = []; overflow = false; } }
    })();
  };
  return {
    bind(next: HistoryScope | null) {
      if (same(scope, next) || !scope && !next) return;
      invalidate(); scope = next;
      const nextKey = next ? selectionKey(next) : null;
      if (key !== nextKey) { key = nextKey; degraded = false; }
      retry();
    },
    retry,
    dispose() { if (disposed) return; disposed = true; invalidate(); scope = null; unlisten?.(); unlisten = null; },
  };
}
