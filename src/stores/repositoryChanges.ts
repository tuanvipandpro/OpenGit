import { createStore } from 'zustand/vanilla';
import type { StoreApi } from 'zustand';
import { call, transportError } from '../contracts';
import type { ActionIntent, ActionScope, ApiResult, AppError, ChangeDiff, ChangeSelectionInput, ChangeSide, OperationSnapshot, PreparedAction, RepositoryChanges } from '../contracts';
import type { Api } from './workspace';
import type { HistoryState } from './history';
import { sameActionScope } from './repositorySync';
const invalid = (message: string): AppError => ({ code: 'InvalidHandle', phase: 'validation', retry: 'none', message });
export type ChangeSelectionFence = Pick<ChangeDiff, 'scope' | 'changeHandle' | 'diffId' | 'side'>;
export const sameChangeFence = (diff: ChangeSelectionFence | null, fence: ChangeSelectionFence) => !!diff && sameActionScope(diff.scope, fence.scope) && diff.changeHandle === fence.changeHandle && diff.diffId === fence.diffId && diff.side === fence.side;
const sameSelection = (a: ActionScope | null, b: ActionScope | null) => !!a && !!b && a.repoId === b.repoId && a.worktreeId === b.worktreeId;
export interface RepositoryChangesState {
  scope: ActionScope | null; catalog: RepositoryChanges | null; loading: boolean; error: AppError | null; diff: ChangeDiff | null; diffLoading: boolean;
  selectedChange: { entryId: string; side: ChangeSide } | null; selection: ChangeSelectionInput | null;
  draft: string; amend: boolean; prepared: PreparedAction | null; preparing: boolean; executing: boolean; actionError: AppError | null; stale: boolean;
  connect(history: StoreApi<HistoryState>): () => void; reload(): Promise<void>; awaitCatalog(signal: AbortSignal): Promise<boolean>; clearDiff(): void; setDraft(value: string): void; setAmend(value: boolean): void; clearPrepared(): void;
  setSelection(fence: ChangeSelectionFence, selection: ChangeSelectionInput): void; prepareSelection(fence: ChangeSelectionFence): Promise<void>;
  select(entryId: string, side: ChangeSide): Promise<void>; prepareFile(entryId: string, side: ChangeSide): Promise<void>; prepareCommit(): Promise<void>;
  execute(): Promise<ApiResult<OperationSnapshot> | null>;
}
export function createRepositoryChangesStore(api: Api = call) {
  let owner: { history: StoreApi<HistoryState>; users: number; stop(): void } | null = null;
  let key: string | null = null, read = 0, action = 0, diffTicket = 0, execution = 0, scheduled = false;
  return createStore<RepositoryChangesState>((set, get, store) => {
    const live = (scope: ActionScope) => !!owner && sameActionScope(scope, get().scope) && sameActionScope(scope, owner.history.getState().preferences.getState().scope);
    const load = async () => {
      const scope = get().scope; if (!scope) return;
      const ticket = ++read; action++; diffTicket++;
      set({ loading: true, catalog: null, error: null, diff: null, selectedChange: null, selection: null, diffLoading: false, prepared: null, preparing: false, stale: true });
      const result = await api('get_repository_changes', { scope }).catch(() => ({ ok: false as const, error: transportError() }));
      if (ticket !== read || !live(scope)) return;
      if (!result.ok) { set({ loading: false, error: result.error }); return; }
      if (!sameActionScope(result.value.scope, scope)) { set({ loading: false, error: invalid('Repository context changed. Refresh the context.') }); return; }
      set({ loading: false, catalog: result.value });
    };
    const bind = () => {
      const scope = owner?.history.getState().preferences.getState().scope ?? null, next = JSON.stringify(scope);
      if (next === key) return;
      const retainDraft = sameSelection(scope, get().scope);
      key = next; read++; action++; diffTicket++; execution++;
      set({ scope, catalog: null, loading: false, diff: null, selectedChange: null, selection: null, diffLoading: false, executing: false, error: null, prepared: null, preparing: false, actionError: null, stale: true, ...(!retainDraft ? { draft: '', amend: false } : {}) });
      if (scope) void load();
    };
    const schedule = () => { if (scheduled) return; scheduled = true; queueMicrotask(() => { scheduled = false; if (owner) bind(); }); };
    const resolve = async (entryId: string, side: ChangeSide) => {
      const { scope, catalog } = get(); if (!scope || !catalog || !live(scope) || get().loading) throw invalid('Refresh the changes before selecting a file.');
      if (!catalog.entries.some(entry => entry.entryId === entryId && (side === 'staged' ? entry.unstageCapability : entry.stageCapability) !== 'unavailable')) throw invalid('The selected file or side is no longer available. Select a current change explicitly.');
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
        if (kind === 'commit') {
          if (!state.draft.trim()) throw invalid('Enter a commit message.');
          if (state.amend && (catalog.head.kind === 'unborn' || !catalog.head.oid)) throw invalid('Amend requires an existing HEAD commit.');
          if (!state.amend && !catalog.entries.some(entry => entry.unstageCapability !== 'unavailable')) throw invalid('Stage changes or explicitly select Amend last commit for a message-only amendment.');
          intent = { kind: 'commit', message: state.draft, amend: state.amend };
        }
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
      scope: null, catalog: null, loading: false, error: null, diff: null, selectedChange: null, selection: null, diffLoading: false, draft: '', amend: false, prepared: null, preparing: false, executing: false, actionError: null, stale: false,
      connect: history => {
        if (owner && owner.history !== history) throw Error('Changes already have a history owner');
        if (!owner) { const a = history.subscribe(schedule), b = history.getState().preferences.subscribe(() => {
          // Observe real selection changes synchronously so A→B→A cannot hide
          // a memory-owner transition behind the coalesced catalog read.
          if (!sameSelection(history.getState().preferences.getState().scope, get().scope)) {
            action++; diffTicket++; execution++; set({ draft: '', amend: false, selection: null, selectedChange: null, diff: null, prepared: null, preparing: false, executing: false });
          }
          schedule();
        }); owner = { history, users: 0, stop: () => { a(); b(); } }; schedule(); }
        const connection = owner; connection.users++; let disposed = false;
        return () => { if (disposed) return; disposed = true; connection.users--; queueMicrotask(() => { if (owner !== connection || connection.users) return; owner.stop(); owner = null; key = null; read++; action++; diffTicket++; execution++; set({ scope: null, catalog: null, draft: '', amend: false, selection: null, selectedChange: null, prepared: null, diff: null, preparing: false, executing: false, loading: false }); }); };
      },
      reload: load,
      awaitCatalog: signal => new Promise(resolveReady => {
        let done = false, unsubscribe = () => {};
        const finish = (ready: boolean) => { if (done) return; done = true; unsubscribe(); signal.removeEventListener('abort', abort); resolveReady(ready); };
        const abort = () => finish(false);
        const check = () => {
          const state = get(), expected = owner?.history.getState().preferences.getState().scope;
          if (signal.aborted || !owner || !expected) { finish(false); return; }
          if (sameActionScope(expected, state.scope) && !state.loading && (state.error || sameActionScope(expected, state.catalog?.scope))) finish(true);
        };
        unsubscribe = store.subscribe(check); signal.addEventListener('abort', abort); check();
      }),
      clearDiff: () => { diffTicket++; action++; set({ diff: null, selectedChange: null, selection: null, diffLoading: false, prepared: null, preparing: false }); },
      clearPrepared: () => { if (!get().executing) action++; set({ prepared: null, preparing: false, actionError: null }); },
      setDraft: draft => { if (!get().executing) { action++; set({ draft, prepared: null, preparing: false, actionError: null }); } },
      setAmend: amend => { if (!get().executing) { action++; set({ amend, prepared: null, preparing: false, actionError: null }); } },
      setSelection: (fence, selection) => {
        const { diff, scope } = get();
        if (get().executing || !scope || !live(scope) || !sameChangeFence(diff, fence) || diff?.partialEligibility !== 'eligible') return;
        const ids = selection.kind === 'hunks' ? selection.hunkIds : selection.lineIds;
        const available = new Set(selection.kind === 'hunks' ? diff.hunks.filter(hunk => hunk.rows.some(row => row.kind !== 'context' && row.lineId)).map(hunk => hunk.hunkId) : diff.hunks.flatMap(hunk => hunk.rows.filter(row => row.kind !== 'context' && row.lineId).map(row => row.lineId!)));
        action++;
        if (new Set(ids).size !== ids.length || ids.some(id => !available.has(id))) { set({ prepared: null, preparing: false, actionError: invalid('Choose changes from the current structured diff.') }); return; }
        set({ selection: selection.kind === 'hunks' ? { kind: 'hunks', hunkIds: [...ids] } : { kind: 'lines', lineIds: [...ids] }, prepared: null, preparing: false, actionError: null });
      },
      select: async (entryId, side) => {
        const ticket = ++diffTicket; action++; set({ diff: null, selectedChange: { entryId, side }, selection: null, prepared: null, preparing: false, diffLoading: true, error: null });
        try { const target = await resolve(entryId, side); if (ticket !== diffTicket) return;
          const r = await api('get_change_diff', { scope: target.scope, changeHandle: target.handle }); if (ticket !== diffTicket || !live(target.scope)) return;
          if (!r.ok) set({ error: r.error }); else if (sameActionScope(r.value.scope, target.scope) && r.value.changeHandle === target.handle && r.value.side === side && r.value.diffId) set({ diff: r.value });
          else set({ error: invalid('The diff no longer matches the selected file and side. Select it again explicitly.') });
        } catch (e) { if (ticket === diffTicket) set({ error: typeof e === 'object' && e && 'code' in e ? e as AppError : transportError() }); }
        finally { if (ticket === diffTicket) set({ diffLoading: false }); }
      },
      prepareFile: (entryId, side) => prepare(side === 'staged' ? 'unstage' : 'stage', entryId, side), prepareCommit: () => prepare('commit'),
      prepareSelection: async fence => {
        const { scope, catalog, diff, selection } = get();
        if (!scope || !catalog || get().loading || get().preparing || get().executing || catalog.unsupported || !live(scope)) return;
        if (!sameChangeFence(diff, fence) || diff?.partialEligibility !== 'eligible' || !selection || !(selection.kind === 'hunks' ? selection.hunkIds.length : selection.lineIds.length)) {
          set({ prepared: null, actionError: invalid('Select hunks or lines from the current file and side again explicitly.') }); return;
        }
        const ticket = ++action, catalogId = catalog.catalogId;
        const current = () => ticket === action && live(scope) && get().catalog?.catalogId === catalogId && sameChangeFence(get().diff, fence) && get().selection === selection;
        set({ preparing: true, prepared: null, actionError: null, stale: false });
        try {
          const resolved = await api('resolve_change_selection', { scope, changeHandle: fence.changeHandle, diffId: fence.diffId, selection }); if (!current()) return;
          if (!resolved.ok) throw resolved.error;
          if (!sameActionScope(resolved.value.scope, scope) || resolved.value.changeHandle !== fence.changeHandle || resolved.value.diffId !== fence.diffId) throw invalid('Selection authority changed. Select the changes again explicitly.');
          const kind = fence.side === 'unstaged' ? 'stageSelection' : 'unstageSelection';
          const result = await api('prepare_repository_action', { scope, intent: { kind, selectionHandles: [resolved.value.selectionHandle] } }); if (!current()) return;
          if (!result.ok) throw result.error;
          if (!sameActionScope(result.value.scope, scope) || result.value.kind !== kind || !(Date.parse(result.value.expiresAt) > Date.now())) throw invalid('Preview changed or expired. Select the changes again explicitly.');
          set({ prepared: result.value });
        } catch (error) { if (current()) set({ actionError: typeof error === 'object' && error && 'code' in error ? error as AppError : transportError() }); }
        finally { if (current()) set({ preparing: false }); }
      },
      execute: async () => {
        const plan = get().prepared; if (!plan || get().executing || get().preparing || !live(plan.scope)) return null;
        if (!(Date.parse(plan.expiresAt) > Date.now())) { set({ prepared: null, actionError: invalid('Preview expired. Prepare again.') }); return null; }
        const ticket = ++execution; set({ executing: true, prepared: null, actionError: null });
        try {
          const result = await api('execute_repository_action', { planId: plan.planId }).catch(() => ({ ok: false as const, error: transportError() }));
          if (result.ok && (result.value.kind !== 'action' || result.value.actionKind !== plan.kind || result.value.repoId !== plan.scope.repoId || result.value.worktreeId !== plan.scope.worktreeId)) { const error = invalid('Action reply could not be matched. Check Tasks before preparing another action.'); if (ticket === execution && live(plan.scope)) set({ actionError: error }); return { ok: false as const, error }; }
          if (!result.ok && ticket === execution && live(plan.scope)) set({ actionError: result.error });
          // Enqueue ACK does not prove a commit. Keep the memory-only draft;
          // hooks/signers or an unknown outcome can still leave no new commit.
          return result;
        } finally { if (ticket === execution) set({ executing: false }); }
      },
    };
  });
}
export const repositoryChangesStore = createRepositoryChangesStore();
