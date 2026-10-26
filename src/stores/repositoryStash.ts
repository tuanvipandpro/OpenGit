import { createStore } from 'zustand/vanilla';
import type { StoreApi } from 'zustand';
import { call, transportError } from '../contracts';
import type { ActionIntent, ActionScope, ApiResult, AppError, OperationSnapshot, PreparedAction, RepositoryStashes } from '../contracts';
import type { Api } from './workspace';
import type { HistoryState } from './history';
import { sameActionScope } from './repositorySync';
const invalid = (message: string): AppError => ({ code: 'InvalidHandle', phase: 'inspection', retry: 'none', message });
const sameSelection = (a: ActionScope | null, b: ActionScope | null) => !!a && !!b && a.repoId === b.repoId && a.worktreeId === b.worktreeId;
export interface RepositoryStashState {
  scope: ActionScope | null; catalog: RepositoryStashes | null; loading: boolean; error: AppError | null;
  message: string; includeUntracked: boolean; prepared: PreparedAction | null; preparing: boolean; executing: boolean; actionError: AppError | null;
  connect(history: StoreApi<HistoryState>): () => void; begin(): Promise<void>; end(): void; reload(): Promise<void>; awaitCatalog(signal: AbortSignal): Promise<boolean>;
  setMessage(value: string): void; setIncludeUntracked(value: boolean): void; clearPrepared(): void;
  prepareSave(): Promise<void>; prepareApply(entryId: string): Promise<void>; execute(): Promise<ApiResult<OperationSnapshot> | null>;
}
export function createRepositoryStashStore(api: Api = call) {
  let owner: { history: StoreApi<HistoryState>; users: number; stop(): void } | null = null;
  let active = false, key: string | null = null, read = 0, action = 0, execution = 0, scheduled = false;
  return createStore<RepositoryStashState>((set, get, store) => {
    const live = (scope: ActionScope) => !!owner && sameActionScope(scope, get().scope) && sameActionScope(scope, owner.history.getState().preferences.getState().scope);
    const load = async () => {
      const scope = get().scope; if (!active || !scope) return;
      const ticket = ++read; action++; set({ loading: true, catalog: null, error: null, prepared: null, preparing: false, actionError: null });
      const result = await api('get_repository_stashes', { scope }).catch(() => ({ ok: false as const, error: transportError() }));
      if (ticket !== read || !active || !live(scope)) return;
      if (!result.ok) { set({ loading: false, error: result.error }); return; }
      if (!sameActionScope(result.value.scope, scope)) { set({ loading: false, error: invalid('Stash context changed. Refresh before choosing an action.') }); return; }
      set({ loading: false, catalog: result.value });
    };
    const bind = () => {
      const scope = owner?.history.getState().preferences.getState().scope ?? null, next = JSON.stringify(scope);
      if (next === key) return;
      const retain = sameSelection(scope, get().scope); key = next; read++; action++; execution++;
      set({ scope, catalog: null, loading: false, error: null, prepared: null, preparing: false, executing: false, actionError: null, ...(!retain ? { message: '', includeUntracked: false } : {}) });
      if (scope && active) void load();
    };
    const schedule = () => { if (scheduled) return; scheduled = true; queueMicrotask(() => { scheduled = false; if (owner) bind(); }); };
    const prepare = async (entryId?: string) => {
      const { scope, catalog } = get(); if (!active || !scope || !catalog || !live(scope) || get().loading || get().error || get().preparing || get().executing) return;
      const ticket = ++action, catalogId = catalog.catalogId;
      const current = () => ticket === action && active && live(scope) && get().catalog?.catalogId === catalogId;
      set({ preparing: true, prepared: null, actionError: null });
      try {
        let intent: ActionIntent;
        if (entryId === undefined) {
          if (!catalog.saveAvailable) throw invalid('Saving a stash is unavailable in the current repository state.');
          intent = { kind: 'stashPush', message: get().message, includeUntracked: get().includeUntracked };
        } else {
          const entry = catalog.entries.find(value => value.entryId === entryId);
          if (!entry || !entry.applyAvailable) throw invalid('This stash entry is missing or unavailable. Choose a current entry explicitly.');
          const result = await api('resolve_stash_target', { scope, catalogId, entryId }).catch(() => ({ ok: false as const, error: transportError() }));
          if (!current()) return;
          if (!result.ok) throw result.error;
          if (!sameActionScope(result.value.scope, scope) || result.value.catalogId !== catalogId || result.value.entryId !== entryId) throw invalid('The stash target changed. Choose an entry from the current catalog.');
          intent = { kind: 'stashApply', stashHandle: result.value.stashHandle };
        }
        const result = await api('prepare_repository_action', { scope, intent }).catch(() => ({ ok: false as const, error: transportError() }));
        if (!current()) return;
        if (!result.ok) throw result.error;
        if (!sameActionScope(result.value.scope, scope) || result.value.kind !== intent.kind || !(Date.parse(result.value.expiresAt) > Date.now())) throw invalid('Preview changed or expired. Prepare again explicitly.');
        set({ prepared: result.value });
      } catch (error) { if (current()) set({ actionError: typeof error === 'object' && error && 'code' in error ? error as AppError : transportError() }); }
      finally { if (current()) set({ preparing: false }); }
    };
    return { scope: null, catalog: null, loading: false, error: null, message: '', includeUntracked: false, prepared: null, preparing: false, executing: false, actionError: null,
      connect: history => {
        if (owner && owner.history !== history) throw Error('Stash already has a history owner');
        if (!owner) {
          const a = history.subscribe(schedule), b = history.getState().preferences.subscribe(() => {
            if (!sameSelection(history.getState().preferences.getState().scope, get().scope)) { action++; execution++; set({ message: '', includeUntracked: false, prepared: null, preparing: false, executing: false }); }
            schedule();
          }); owner = { history, users: 0, stop: () => { a(); b(); } }; schedule();
        }
        const connection = owner; connection.users++; let disposed = false;
        return () => { if (disposed) return; disposed = true; connection.users--; queueMicrotask(() => { if (owner !== connection || connection.users) return; owner.stop(); owner = null; active = false; key = null; read++; action++; execution++; set({ scope: null, catalog: null, loading: false, message: '', includeUntracked: false, prepared: null, preparing: false, executing: false }); }); };
      },
      begin: async () => { active = true; set({ includeUntracked: false }); const before = key; bind(); if (before === key) await load(); },
      end: () => { active = false; read++; action++; execution++; set({ catalog: null, loading: false, prepared: null, preparing: false, executing: false }); }, reload: load,
      awaitCatalog: signal => new Promise(resolve => {
        let done = false, unsubscribe = () => {};
        const finish = (value: boolean) => { if (done) return; done = true; unsubscribe(); signal.removeEventListener('abort', abort); resolve(value); };
        const abort = () => finish(false);
        const check = () => { const state = get(); if (signal.aborted || !active || !owner || !state.scope) { finish(false); return; } if (live(state.scope) && !state.loading && (state.error || sameActionScope(state.scope, state.catalog?.scope))) finish(true); };
        unsubscribe = store.subscribe(check); signal.addEventListener('abort', abort); check();
      }),
      clearPrepared: () => { action++; set({ prepared: null, preparing: false, actionError: null }); },
      setMessage: message => { if (!get().executing) { action++; set({ message, prepared: null, preparing: false, actionError: null }); } },
      setIncludeUntracked: includeUntracked => { if (!get().executing) { action++; set({ includeUntracked, prepared: null, preparing: false, actionError: null }); } },
      prepareSave: () => prepare(), prepareApply: entryId => prepare(entryId),
      execute: async () => {
        const plan = get().prepared; if (!plan || !active || !live(plan.scope) || get().preparing || get().executing) return null;
        if (!(Date.parse(plan.expiresAt) > Date.now())) { set({ prepared: null, actionError: invalid('Preview expired. Prepare again explicitly.') }); return null; }
        const ticket = ++execution; set({ prepared: null, executing: true, actionError: null });
        try {
          const result = await api('execute_repository_action', { planId: plan.planId }).catch(() => ({ ok: false as const, error: transportError() }));
          if (result.ok && (result.value.kind !== 'action' || result.value.actionKind !== plan.kind || result.value.repoId !== plan.scope.repoId || result.value.worktreeId !== plan.scope.worktreeId)) { const error = invalid('Action reply could not be matched. Check Tasks before preparing another stash action.'); if (ticket === execution && active && live(plan.scope)) set({ actionError: error }); return { ok: false as const, error }; }
          if (!result.ok && ticket === execution && active && live(plan.scope)) set({ actionError: result.error });
          return result;
        } finally { if (ticket === execution) set({ executing: false }); }
      },
    };
  });
}
export const repositoryStashStore = createRepositoryStashStore();
