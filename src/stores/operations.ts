import { createStore } from 'zustand/vanilla';
import { listen } from '@tauri-apps/api/event';
import { call, OPERATION_EVENT, transportError } from '../contracts';
import type { ApiResult, CloneDestination, CommandMap, LocalCloneSource, AppError, OperationEvent, OperationSnapshot, OperationsSnapshot, ActionOperationResult, RefActionResult, ChangeActionResult, SyncActionResult, RemoteActionOperationResult } from '../contracts';
import type { Api } from './workspace';
export type Subscribe = (handler: (event: OperationEvent) => void) => Promise<() => void>;
const subscribe: Subscribe = handler => listen<OperationEvent>(OPERATION_EVENT, event => handler(event.payload));
export const activeOperation = (operation: OperationSnapshot) => ['Queued','Running','WaitingForUser','Cancelling'].includes(operation.state);
const sameOperationVersion = (a: OperationSnapshot | undefined, b: OperationSnapshot | undefined) => !!a && !!b
  && a.id === b.id && a.sequence === b.sequence && a.kind === b.kind && a.actionKind === b.actionKind
  && a.repoId === b.repoId && a.worktreeId === b.worktreeId && a.state === b.state
  && a.createdAt === b.createdAt && a.updatedAt === b.updatedAt;
export type DisplayActionResult = ActionOperationResult & { refResult?: RefActionResult | null; changeResult?: ChangeActionResult | null; syncResult?: SyncActionResult | null; remoteReadResult?: Pick<RemoteActionOperationResult, 'availability' | 'evidence'> };
const syncKinds = new Set(['push', 'pull', 'remoteTagPush', 'remoteTagDelete']);
const localRefKinds = new Set(['branchCreate', 'branchRename', 'branchSwitch', 'branchDelete', 'branchTracking', 'tagCreate', 'tagDelete']);
export interface OperationsState { outcomes: Record<string, DisplayActionResult>; outcomeErrors: Record<string, AppError>;
  loadOutcome(id: string): Promise<void>;
 snapshot: OperationsSnapshot | null; error: AppError | null; subscriptionFailed: boolean; start(): Promise<() => void>; sync(): Promise<void>; cancel(id: string): Promise<ApiResult<OperationSnapshot>>; chooseSource(): Promise<ApiResult<LocalCloneSource | null>>; chooseDestination(): Promise<ApiResult<CloneDestination | null>>; clone(request: CommandMap['start_clone']['request']): Promise<ApiResult<OperationSnapshot>>; respond(operationId: string, promptHandle: string, value: string): Promise<ApiResult<null>>; }
export function createOperationsStore(api: Api = call, subscribeToEvents: Subscribe = subscribe) {
  let inFlight: Promise<void> | null = null;
  const outcomeRequests = new Map<string, { promise: Promise<void> }>();
  let syncing = false, requested = false, buffer: OperationEvent[] = [], fence = 0n;
  return createStore<OperationsState>((set, get) => {
    const publish = (snapshot: OperationsSnapshot) => {
      const previous = get().snapshot, live = new Map(snapshot.operations.map(operation => [operation.id, operation]));
      const prior = new Map(previous?.operations.map(operation => [operation.id, operation]));
      // Authoritative removal retires both data and request ownership. An old
      // promise may finish, but cannot republish even if the same ID reappears.
      for (const id of outcomeRequests.keys()) {
        if (previous?.streamId !== snapshot.streamId || !sameOperationVersion(prior.get(id), live.get(id))) outcomeRequests.delete(id);
      }
      const outcomes = Object.fromEntries(Object.entries(get().outcomes).filter(([id, result]) => sameOperationVersion(result.operation, live.get(id))));
      const outcomeErrors = Object.fromEntries(Object.entries(get().outcomeErrors).filter(([id]) => sameOperationVersion(prior.get(id), live.get(id))));
      set({ snapshot, outcomes, outcomeErrors, error: null });
    };
    const receive = (event: OperationEvent) => {
      const current = get().snapshot;
      if (syncing || !current) { buffer.push(event); return; }
      const sequence = BigInt(event.sequence);
      if (event.streamId === current.streamId && sequence <= fence) return;
      if (event.streamId !== current.streamId || sequence !== BigInt(current.sequence) + 1n || event.operation.sequence !== event.sequence) { void get().sync(); return; }
      publish({ ...current, sequence: event.sequence, operations: [...current.operations.filter(item => item.id !== event.operation.id), event.operation] });
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
              fence = BigInt(response.value.sequence); publish(response.value);
            }
          } else set({ error: response.error });
          syncing = false;
          const pending = buffer; buffer = [];
          for (const event of pending) receive(event);
        } while (requested);
      })().finally(() => { inFlight = null; });
      return inFlight;
    };
    return { outcomes: {}, outcomeErrors: {},
      loadOutcome: id => {
        const operation = get().snapshot?.operations.find(item => item.id === id);
        if (!operation || operation.kind !== 'action' || activeOperation(operation)) return Promise.resolve();
        const existing = outcomeRequests.get(id); if (existing) return existing.promise;
        const streamId = get().snapshot!.streamId;
        const request = { promise: Promise.resolve() };
        request.promise = (async () => {
          const result = operation.actionKind === 'readRemoteRefs'
            ? await api('get_remote_action_outcome', { operationId: id }).then(response => response.ok ? { ok: true as const, value: { operation: response.value.operation, fetch: null, remoteReadResult: { availability: response.value.availability, evidence: response.value.evidence } } } : response).catch(() => ({ ok: false as const, error: transportError() }))
            : syncKinds.has(operation.actionKind ?? '')
            ? await api('get_sync_action_outcome',{operationId:id}).then(response=>response.ok?{ok:true as const,value:{operation:response.value.operation,fetch:null,syncResult:response.value.result}}:response).catch(()=>({ok:false as const,error:transportError()}))
            : ['stage','unstage','stageSelection','unstageSelection','commit'].includes(operation.actionKind ?? '')
            ? await api('get_change_action_outcome', { operationId: id }).then(response => response.ok ? { ok: true as const, value: { operation: response.value.operation, fetch: null, changeResult: response.value.result } } : response).catch(() => ({ ok: false as const, error: transportError() }))
            : localRefKinds.has(operation.actionKind ?? '')
            ? await api('get_ref_action_outcome', { operationId: id }).then(response => response.ok ? { ok: true as const, value: { operation: response.value.operation, fetch: null, refResult: response.value.result } } : response).catch(() => ({ ok: false as const, error: transportError() }))
            : await api('get_action_outcome', { operationId: id }).catch(() => ({ ok: false as const, error: transportError() }));
          if (outcomeRequests.get(id) !== request || get().snapshot?.streamId !== streamId) return;
          const current = get().snapshot?.operations.find(item => item.id === id);
          if (!current || current.kind !== 'action') return;
          if (!result.ok) { set({ outcomeErrors: { ...get().outcomeErrors, [id]: result.error } }); return; }
          if (result.value.operation.id !== id || result.value.operation.kind !== 'action' || result.value.operation.repoId !== current.repoId
            || result.value.operation.worktreeId !== current.worktreeId || result.value.operation.actionKind !== current.actionKind
            || BigInt(result.value.operation.sequence) < BigInt(current.sequence)) return;
          const errors = { ...get().outcomeErrors }; delete errors[id];
          set({ outcomes: { ...get().outcomes, [id]: result.value }, outcomeErrors: errors });
        })().finally(() => { if (outcomeRequests.get(id) === request) outcomeRequests.delete(id); });
        outcomeRequests.set(id, request); return request.promise;
      }, snapshot: null, error: null, subscriptionFailed: false, sync,
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
