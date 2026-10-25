import { createStore } from 'zustand/vanilla';
import type { StoreApi } from 'zustand';
import { call, transportError } from '../contracts';
import type { ActionIntent, ActionScope, ApiResult, AppError, ChangeDiff, ChangeSide, OperationSnapshot, PreparedAction, RepositoryChanges } from '../contracts';
import type { Api } from './workspace';
import type { HistoryState } from './history';
import { sameActionScope } from './repositorySync';
const invalid = (message: string): AppError => ({ code: 'InvalidHandle', phase: 'validation', retry: 'none', message });
export interface RepositoryChangesState {
  scope: ActionScope | null; catalog: RepositoryChanges | null; loading: boolean; error: AppError | null; diff: ChangeDiff | null; diffLoading: boolean;
  draft: string; prepared: PreparedAction | null; preparing: boolean; executing: boolean; actionError: AppError | null; stale: boolean;
  connect(history: StoreApi<HistoryState>): () => void; reload(): Promise<void>; setDraft(value: string): void; clearPrepared(): void;
  select(entryId: string, side: ChangeSide): Promise<void>; prepareFile(entryId: string, side: ChangeSide): Promise<void>; prepareCommit(): Promise<void>;
  execute(): Promise<ApiResult<OperationSnapshot> | null>;
}
export function createRepositoryChangesStore(api: Api = call) {
  let owner: { history: StoreApi<HistoryState>; users: number; stop(): void } | null = null;
  let key: string | null = null, read = 0, action = 0, diffTicket = 0, scheduled = false;
  return createStore<RepositoryChangesState>((set, get) => {
    const live = (scope: ActionScope) => !!owner && sameActionScope(scope, get().scope) && sameActionScope(scope, owner.history.getState().preferences.getState().scope);
    const load = async () => {
      const scope = get().scope; if (!scope) return;
      const ticket = ++read; action++; diffTicket++;
      set({ loading: true, catalog: null, error: null, diff: null, diffLoading: false, prepared: null, preparing: false, stale: true });
      const result = await api('get_repository_changes', { scope }).catch(() => ({ ok: false as const, error: transportError() }));
      if (ticket !== read || !live(scope)) return;
      if (!result.ok) { set({ loading: false, error: result.error }); return; }
      if (!sameActionScope(result.value.scope, scope)) { set({ loading: false, error: invalid('Repository context changed. Refresh the context.') }); return; }
      set({ loading: false, catalog: result.value });
    };
    const bind = () => {
      const scope = owner?.history.getState().preferences.getState().scope ?? null, next = JSON.stringify(scope);
      if (next === key) return;
      const previous = get().scope, sameSelection = !!scope && previous?.repoId === scope.repoId && previous.worktreeId === scope.worktreeId;
      key = next; read++; action++; diffTicket++;
      set({ scope, catalog: null, loading: false, diff: null, diffLoading: false, error: null, prepared: null, preparing: false, actionError: null, stale: true, ...(!sameSelection ? { draft: '' } : {}) });
      if (scope) void load();
    };
    const schedule = () => { if (scheduled) return; scheduled = true; queueMicrotask(() => { scheduled = false; if (owner) bind(); }); };
    const resolve = async (entryId: string, side: ChangeSide) => {
      const { scope, catalog } = get(); if (!scope || !catalog || !live(scope) || get().loading) throw invalid('Refresh the changes before selecting a file.');
      const r = await api('resolve_change_targets', { scope, catalogId: catalog.catalogId, targets: [{ entryId, side }] });
      if (!live(scope) || get().catalog?.catalogId !== catalog.catalogId) throw invalid('Repository context changed. Select the file again.');
      if (!r.ok) throw r.error;
      if (!sameActionScope(r.value.scope, scope) || r.value.catalogId !== catalog.catalogId || r.value.targets.length !== 1 || r.value.targets[0].entryId !== entryId || r.value.targets[0].side !== side) throw invalid('The selected change no longer matches. Refresh the context.');
      return { scope, handle: r.value.targets[0].changeHandle };
    };
    const prepare = async (kind: 'stage' | 'unstage' | 'commit', entryId?: string, side?: ChangeSide) => {
      const state = get(), { scope, catalog } = state; if (!scope || !catalog || state.loading || state.preparing || state.executing || catalog.unsupported || !live(scope)) return;
      const ticket = ++action, catalogId = catalog.catalogId; const current = () => ticket === action && live(scope) && get().catalog?.catalogId === catalogId;
      set({ preparing: true, prepared: null, actionError: null, stale: false });
      try {
        let intent: ActionIntent;
        if (kind === 'commit') { if (!state.draft.trim()) throw invalid('Enter a commit message.'); intent = { kind: 'commit', message: state.draft, amend: false }; }
        else {
          const entry = catalog.entries.find(e => e.entryId === entryId);
          if (!entry || !side || (side === 'staged' ? entry.unstageCapability : entry.stageCapability) !== 'eligible') throw invalid('Select an available whole-file change from the current context.');
          const target = await resolve(entry.entryId, side); if (!current()) return;
          intent = kind === 'stage' ? { kind, changeHandles: [target.handle] } : { kind, changeHandles: [target.handle] };
        }
        const result = await api('prepare_repository_action', { scope, intent }); if (!current()) return;
        if (!result.ok) { set({ actionError: result.error }); return; }
        if (!sameActionScope(result.value.scope, scope) || result.value.kind !== kind || !(Date.parse(result.value.expiresAt) > Date.now())) throw invalid('Preview expired. Prepare again.');
        set({ prepared: result.value });
      } catch (e) { if (current()) set({ actionError: typeof e === 'object' && e && 'code' in e ? e as AppError : transportError() }); }
      finally { if (current()) set({ preparing: false }); }
    };
    return {
      scope: null, catalog: null, loading: false, error: null, diff: null, diffLoading: false, draft: '', prepared: null, preparing: false, executing: false, actionError: null, stale: false,
      connect: history => {
        if (owner && owner.history !== history) throw Error('Changes already have a history owner');
        if (!owner) { const a = history.subscribe(schedule), b = history.getState().preferences.subscribe(schedule); owner = { history, users: 0, stop: () => { a(); b(); } }; schedule(); }
        const connection = owner; connection.users++; let disposed = false;
        return () => { if (disposed) return; disposed = true; connection.users--; queueMicrotask(() => { if (owner !== connection || connection.users) return; owner.stop(); owner = null; key = null; read++; action++; diffTicket++; set({ scope: null, catalog: null, draft: '', prepared: null, diff: null, preparing: false, loading: false }); }); };
      },
      reload: load,
      clearPrepared: () => { if (!get().executing) action++; set({ prepared: null, preparing: false, actionError: null }); },
      setDraft: draft => { if (!get().executing) { action++; set({ draft, prepared: null, preparing: false, actionError: null }); } },
      select: async (entryId, side) => {
        const ticket = ++diffTicket; set({ diff: null, diffLoading: true, error: null });
        try { const target = await resolve(entryId, side); if (ticket !== diffTicket) return;
          const r = await api('get_change_diff', { scope: target.scope, changeHandle: target.handle }); if (ticket !== diffTicket || !live(target.scope)) return;
          if (!r.ok) set({ error: r.error }); else if (sameActionScope(r.value.scope, target.scope) && r.value.changeHandle === target.handle) set({ diff: r.value });
        } catch (e) { if (ticket === diffTicket) set({ error: typeof e === 'object' && e && 'code' in e ? e as AppError : transportError() }); }
        finally { if (ticket === diffTicket) set({ diffLoading: false }); }
      },
      prepareFile: (entryId, side) => prepare(side === 'staged' ? 'unstage' : 'stage', entryId, side), prepareCommit: () => prepare('commit'),
      execute: async () => {
        const plan = get().prepared; if (!plan || get().executing || get().preparing || !live(plan.scope)) return null;
        if (!(Date.parse(plan.expiresAt) > Date.now())) { set({ prepared: null, actionError: invalid('Preview expired. Prepare again.') }); return null; }
        set({ executing: true, prepared: null, actionError: null });
        try {
          const result = await api('execute_repository_action', { planId: plan.planId }).catch(() => ({ ok: false as const, error: transportError() }));
          if (result.ok && (result.value.kind !== 'action' || result.value.actionKind !== plan.kind || result.value.repoId !== plan.scope.repoId || result.value.worktreeId !== plan.scope.worktreeId)) { const error = invalid('Action reply could not be matched. Check Tasks before preparing another action.'); if (live(plan.scope)) set({ actionError: error }); return { ok: false as const, error }; }
          if (!result.ok && live(plan.scope)) set({ actionError: result.error });
          // Enqueue ACK does not prove a commit. Keep the memory-only draft;
          // hooks/signers or an unknown outcome can still leave no new commit.
          return result;
        } finally { set({ executing: false }); }
      },
    };
  });
}
export const repositoryChangesStore = createRepositoryChangesStore();
