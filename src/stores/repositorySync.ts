import { createStore } from 'zustand/vanilla';
import type { StoreApi } from 'zustand';
import { call, transportError } from '../contracts';
import type { ActionScope, ApiResult, AppError, CommitContainment, HistoryContext, OperationSnapshot, PreparedAction, RepositoryActionCatalog } from '../contracts';
import type { Api } from './workspace';
import type { HistoryState } from './history';

export const sameActionScope = (a: ActionScope | null | undefined, b: ActionScope | null | undefined) => !!a && !!b && a.repoId === b.repoId && a.worktreeId === b.worktreeId && a.generation === b.generation;
const sameContext = (a: HistoryContext, b: HistoryContext) => sameActionScope(a.scope, b.scope) && a.snapshotId === b.snapshotId;
const unavailable = (message: string): AppError => ({ code: 'InvalidHandle', phase: 'inspection', retry: 'retry', message });
export interface RepositorySyncState {
  scope: ActionScope | null; catalog: RepositoryActionCatalog | null; catalogLoading: boolean; catalogError: AppError | null;
  containment: CommitContainment | null; containmentLoading: boolean; containmentRetained: boolean; containmentError: AppError | null;
  prepared: PreparedAction | null; preparing: boolean; executing: boolean; actionError: AppError | null;
  connect(history: StoreApi<HistoryState>): () => void;
  awaitCatalog(signal: AbortSignal): Promise<boolean>;
  reload(): Promise<void>; retryContainment(): Promise<void>; clearPrepared(): void;
  prepareFetch(handles: string[], prune: boolean): Promise<void>;
  executeFetch(): Promise<ApiResult<OperationSnapshot> | null>;
}

/** Shares history's published authority scope. Tickets only order local reads;
 * they are never sent as a separate backend generation. No polling or fetch. */
export function createRepositorySyncStore(api: Api = call) {
  let owner: { history: StoreApi<HistoryState>; users: number; stop(): void } | null = null;
  let catalogTicket = 0, membershipTicket = 0, actionTicket = 0;
  let catalogKey: string | null = null, membershipKey: string | null = null, scheduled = false;
  return createStore<RepositorySyncState>((set, get, store) => {
    const currentSelection = () => {
      const history = owner?.history.getState(), scope = history?.preferences.getState().scope;
      const selected = history?.selected, session = history?.session;
      return history && !history.loading && scope && selected && session && sameActionScope(scope, selected.context.scope)
        && sameContext(session.context, selected.context) ? selected : null;
    };
    const loadCatalog = async () => {
      const scope = get().scope; if (!scope) return;
      const ticket = ++catalogTicket;
      set({ catalogLoading: true, catalogError: null, prepared: null, preparing: false });
      if (!get().executing) actionTicket++;
      const result = await api('get_repository_actions', { scope }).catch(() => ({ ok: false as const, error: transportError() }));
      if (ticket !== catalogTicket || !sameActionScope(get().scope, scope) || !sameActionScope(owner?.history.getState().preferences.getState().scope, scope)) return;
      if (!result.ok) { set({ catalogLoading: false, catalogError: result.error }); return; }
      if (!sameActionScope(result.value.scope, scope)) { set({ catalogLoading: false, catalogError: unavailable('Repository context changed. Refresh to reload remote information.') }); return; }
      set({ catalog: result.value, catalogLoading: false, catalogError: null });
    };
    const loadContainment = async () => {
      const selected = currentSelection(); if (!selected) return;
      const ticket = ++membershipTicket, key = membershipKey;
      const retained = get().containment?.oid === selected.commit.oid && get().containment?.context.scope.repoId === selected.context.scope.repoId;
      set({ containmentLoading: true, containmentError: null, containmentRetained: retained, ...(!retained ? { containment: null } : {}) });
      const result = await api('get_commit_containment', { context: selected.context, commitHandle: selected.commit.handle }).catch(() => ({ ok: false as const, error: transportError() }));
      const latest = currentSelection();
      if (ticket !== membershipTicket || key !== membershipKey || !latest || !sameContext(latest.context, selected.context) || latest.commit.handle !== selected.commit.handle) return;
      if (!result.ok) { set({ containmentLoading: false, containmentError: result.error }); return; }
      if (!sameContext(result.value.context, selected.context) || result.value.commitHandle !== selected.commit.handle || result.value.oid !== selected.commit.oid) {
        set({ containmentLoading: false, containmentError: unavailable('Commit context changed. Select the commit again.') }); return;
      }
      set({ containment: result.value, containmentLoading: false, containmentRetained: false, containmentError: null });
    };
    const bind = () => {
      const history = owner?.history.getState(), scope = history?.preferences.getState().scope ?? null;
      const nextKey = scope ? JSON.stringify([scope.repoId, scope.worktreeId, scope.generation]) : null;
      if (nextKey !== catalogKey) {
        const sameRepo = scope?.repoId === get().scope?.repoId;
        catalogKey = nextKey; catalogTicket++; membershipTicket++; actionTicket++; membershipKey = null;
        set({ scope, catalogLoading: false, catalogError: null, prepared: null, preparing: false, executing: false, actionError: null,
          ...(!sameRepo || !scope ? { catalog: null, containment: null, containmentError: null, containmentRetained: false } : { containmentRetained: !!get().containment }) });
        if (scope) void loadCatalog();
      }
      const selected = currentSelection();
      const nextMembership = selected ? JSON.stringify([selected.context, selected.commit.handle, selected.commit.oid]) : null;
      if (nextMembership !== membershipKey) {
        membershipKey = nextMembership; membershipTicket++;
        if (selected) void loadContainment();
        else {
          const retained = !!history?.selected && history.selected.commit.oid === get().containment?.oid && history.selected.context.scope.repoId === get().scope?.repoId;
          set({ containmentLoading: false, containmentRetained: retained, ...(!retained ? { containment: null, containmentError: null } : {}) });
        }
      }
    };
    const schedule = () => { if (scheduled) return; scheduled = true; queueMicrotask(() => { scheduled = false; if (owner) bind(); }); };
    return { scope: null, catalog: null, catalogLoading: false, catalogError: null, containment: null, containmentLoading: false, containmentRetained: false, containmentError: null,
      prepared: null, preparing: false, executing: false, actionError: null,
      connect: history => {
        if (owner && owner.history !== history) throw Error('Repository sync already has a history owner');
        if (!owner) {
          const stopHistory = history.subscribe(schedule), stopPreferences = history.getState().preferences.subscribe(schedule);
          owner = { history, users: 0, stop: () => { stopHistory(); stopPreferences(); } }; schedule();
        }
        const connection = owner; connection.users++; let disposed = false;
        return () => { if (disposed) return; disposed = true; connection.users--; queueMicrotask(() => {
          if (owner !== connection || connection.users) return;
          connection.stop(); owner = null; catalogKey = null; membershipKey = null; catalogTicket++; membershipTicket++; actionTicket++;
          set({ scope: null, catalog: null, containment: null, catalogLoading: false, containmentLoading: false, containmentRetained: false, prepared: null, preparing: false, executing: false });
        }); };
      },
      reload: loadCatalog, retryContainment: loadContainment,
      awaitCatalog: signal => new Promise(resolve => {
        let done = false, unsubscribe = () => {};
        const finish = (ready: boolean) => { if (done) return; done = true; unsubscribe(); signal.removeEventListener('abort', cancel); resolve(ready); };
        const cancel = () => finish(false);
        const check = () => {
          const history = owner?.history.getState(), scope = history?.preferences.getState().scope, state = get();
          if (!history || signal.aborted || !scope && !history.loading) { finish(false); return; }
          if (!history.loading && sameActionScope(scope, state.scope) && !state.catalogLoading && (state.catalogError || sameActionScope(scope, state.catalog?.scope))) finish(true);
        };
        unsubscribe = store.subscribe(check); signal.addEventListener('abort', cancel); check();
      }),
      clearPrepared: () => { if (!get().executing) actionTicket++; set({ prepared: null, actionError: null, preparing: false }); },
      prepareFetch: async (handles, prune) => {
        const state = get(), scope = state.scope, catalog = state.catalog;
        if (state.preparing || state.executing || !sameActionScope(owner?.history.getState().preferences.getState().scope, scope) || !scope || !catalog || state.catalogLoading || state.catalogError || !sameActionScope(catalog.scope, scope)) return;
        const unique = [...new Set(handles)];
        if (!unique.length || unique.some(handle => !catalog.remotes.some(remote => remote.handle === handle && remote.fetch === 'eligibleProbeRequired'))) { set({ actionError: unavailable('Select available remotes from the current repository.') }); return; }
        const ticket = ++actionTicket; set({ preparing: true, prepared: null, actionError: null });
        try {
          const result = await api('prepare_repository_action', { scope, intent: { kind: 'fetch', remoteHandles: unique, prune } }).catch(() => ({ ok: false as const, error: transportError() }));
          if (ticket !== actionTicket || !sameActionScope(get().scope, scope) || !sameActionScope(owner?.history.getState().preferences.getState().scope, scope)) return;
          if (!result.ok) { set({ actionError: result.error }); return; }
          if (result.value.kind !== 'fetch' || !sameActionScope(result.value.scope, scope) || !(Date.parse(result.value.expiresAt) > Date.now())) { set({ actionError: unavailable('The fetch preview expired or changed. Prepare it again.') }); return; }
          set({ prepared: result.value });
        } finally { if (ticket === actionTicket) set({ preparing: false }); }
      },
      executeFetch: async () => {
        const state = get(), prepared = state.prepared;
        if (state.executing || state.preparing || !sameActionScope(owner?.history.getState().preferences.getState().scope, state.scope) || !prepared || !sameActionScope(state.scope, prepared.scope)) return null;
        if (!(Date.parse(prepared.expiresAt) > Date.now())) { set({ prepared: null, actionError: unavailable('The fetch preview expired. Prepare it again.') }); return null; }
        const ticket = ++actionTicket; set({ executing: true, prepared: null, actionError: null });
        try {
          const result = await api('execute_repository_action', { planId: prepared.planId }).catch(() => ({ ok: false as const, error: transportError() }));
          if (ticket === actionTicket && sameActionScope(get().scope, prepared.scope)) {
            if (!result.ok) set({ actionError: result.error });
            else if (result.value.kind !== 'action' || result.value.actionKind !== 'fetch' || result.value.repoId !== prepared.scope.repoId || result.value.worktreeId !== prepared.scope.worktreeId) {
              const error = unavailable('Fetch status could not be matched. Check Tasks before starting another fetch.');
              set({ actionError: error }); return { ok: false, error };
            }
          }
          return result;
        } finally { if (ticket === actionTicket) set({ executing: false }); }
      },
    };
  });
}
export const repositorySyncStore = createRepositorySyncStore();
