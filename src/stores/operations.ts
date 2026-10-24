import { createStore } from 'zustand/vanilla';
import { listen } from '@tauri-apps/api/event';
import { call, OPERATION_EVENT } from '../contracts';
import type { ApiResult, CloneDestination, CommandMap, LocalCloneSource, AppError, OperationEvent, OperationSnapshot, OperationsSnapshot } from '../contracts';
import type { Api } from './workspace';
export type Subscribe = (handler: (event: OperationEvent) => void) => Promise<() => void>;
const subscribe: Subscribe = handler => listen<OperationEvent>(OPERATION_EVENT, event => handler(event.payload));
export const activeOperation = (operation: OperationSnapshot) => ['Queued','Running','WaitingForUser','Cancelling'].includes(operation.state);
interface OperationsState { snapshot: OperationsSnapshot | null; error: AppError | null; subscriptionFailed: boolean; start(): Promise<() => void>; sync(): Promise<void>; cancel(id: string): Promise<ApiResult<OperationSnapshot>>; chooseSource(): Promise<ApiResult<LocalCloneSource | null>>; chooseDestination(): Promise<ApiResult<CloneDestination | null>>; clone(request: CommandMap['start_clone']['request']): Promise<ApiResult<OperationSnapshot>>; respond(operationId: string, promptHandle: string, value: string): Promise<ApiResult<null>>; }
export function createOperationsStore(api: Api = call, subscribeToEvents: Subscribe = subscribe) {
  let inFlight: Promise<void> | null = null;
  let syncing = false, requested = false, buffer: OperationEvent[] = [], fence = 0n;
  return createStore<OperationsState>((set, get) => {
    const receive = (event: OperationEvent) => {
      const current = get().snapshot;
      if (syncing || !current) { buffer.push(event); return; }
      const sequence = BigInt(event.sequence);
      if (event.streamId === current.streamId && sequence <= fence) return;
      if (event.streamId !== current.streamId || sequence !== BigInt(current.sequence) + 1n || event.operation.sequence !== event.sequence) { void get().sync(); return; }
      set({ snapshot: { ...current, sequence: event.sequence, operations: [...current.operations.filter(item => item.id !== event.operation.id), event.operation] } });
    };
    const sync = (): Promise<void> => {
      if (inFlight) { requested = true; return inFlight; }
      inFlight = (async () => {
        do {
          requested = false; syncing = true;
          const response = await api('get_operations', {});
          if (response.ok) {
            const current = get().snapshot;
            if (!current || current.streamId !== response.value.streamId || BigInt(response.value.sequence) >= BigInt(current.sequence)) {
              fence = BigInt(response.value.sequence); set({ snapshot: response.value, error: null });
            }
          } else set({ error: response.error });
          syncing = false;
          const pending = buffer; buffer = [];
          for (const event of pending) receive(event);
        } while (requested);
      })().finally(() => { inFlight = null; });
      return inFlight;
    };
    return { snapshot: null, error: null, subscriptionFailed: false, sync,
      start: async () => {
        let disposed = false;
        let unlisten: (() => void) | undefined;
        try { unlisten = await subscribeToEvents(receive); set({ subscriptionFailed: false }); }
        catch { set({ subscriptionFailed: true }); }
        await sync();
        const timer = window.setInterval(() => { if (get().subscriptionFailed || get().snapshot?.operations.some(activeOperation)) void sync(); }, 2000);
        const focus = () => { if (!disposed) void sync(); };
        window.addEventListener('focus', focus);
        return () => { disposed = true; unlisten?.(); window.clearInterval(timer); window.removeEventListener('focus', focus); };
      },
      chooseSource: () => api('choose_clone_source', {}),
      chooseDestination: () => api('choose_clone_destination', {}),
      clone: async request => { const result = await api('start_clone', request); await sync(); return result; },
      respond: async (operationId, promptHandle, value) => {
        const operation = get().snapshot?.operations.find(operation => operation.id === operationId);
        if (operation?.state !== 'WaitingForUser' || operation.authPrompt?.promptHandle !== promptHandle) return { ok: false, error: { code: 'InvalidHandle', phase: 'authentication', retry: 'none', message: 'This authentication request is no longer active.' } };
        const result = await api('respond_auth_prompt', { operationId, promptHandle, value }); await sync(); return result;
      },
      cancel: async operationId => {
        const response = await api('cancel_operation', { operationId });
        await sync();
        if (!response.ok) set({ error: response.error });
        return response;
      },
    };
  });
}
export const operationsStore = createOperationsStore();
