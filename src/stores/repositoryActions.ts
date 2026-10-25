import { createStore } from 'zustand/vanilla';
import type { StoreApi } from 'zustand';
import { call, transportError } from '../contracts';
import type { ActionIntent, ActionScope, ApiResult, AppError, HistoryContext, OperationSnapshot, PreparedAction, RefActionCatalog, RefActionSource, ResolvedRefActionTarget } from '../contracts';
import type { Api } from './workspace';
import type { HistoryState } from './history';
import { sameActionScope } from './repositorySync';

export type LocalActionKind = 'branchCreate' | 'branchRename' | 'branchSwitch' | 'branchDelete' | 'branchTracking' | 'tagCreate' | 'tagDelete';
export type ActionSourceChoice = { kind: 'head' } | { kind: 'ref'; name: string } | { kind: 'selectedCommit' };
export type LocalActionDraft = { kind: LocalActionKind; source: ActionSourceChoice; name: string; checkout: boolean; upstream: string | null; annotated: boolean; message: string; sign: boolean };
export type ActionLaunch = { kind?: LocalActionKind; refName?: string; selectedCommit?: boolean; remoteKind?: import('../contracts').RemoteMutationKind };
export const localActionTitles: Record<LocalActionKind, string> = { branchCreate: 'Create branch', branchRename: 'Rename branch', branchSwitch: 'Switch branch', branchDelete: 'Delete local branch', branchTracking: 'Set or unset upstream', tagCreate: 'Create tag', tagDelete: 'Delete local tag' };
const invalid = (message: string): AppError => ({ code: 'InvalidHandle', phase: 'inspection', retry: 'retry', message });
const sameContext = (a: HistoryContext, b: HistoryContext) => sameActionScope(a.scope, b.scope) && a.snapshotId === b.snapshotId;
export interface RepositoryActionsState {
  scope: ActionScope | null; catalog: RefActionCatalog | null; loading: boolean; error: AppError | null;
  prepared: PreparedAction | null; preview: { source: string; destination: string } | null; preparing: boolean; executing: boolean; actionError: AppError | null;
  connect(history: StoreApi<HistoryState>): () => void; awaitCatalog(signal: AbortSignal): Promise<boolean>;
  reload(): Promise<void>; clearPrepared(): void; prepare(draft: LocalActionDraft): Promise<void>; execute(): Promise<ApiResult<OperationSnapshot> | null>;
}
export function createRepositoryActionsStore(api: Api = call) {
  let owner: { history: StoreApi<HistoryState>; users: number; stop(): void } | null = null;
  let readTicket = 0, actionTicket = 0, key: string | null = null, selectionKey: string | null = null, scheduled = false;
  return createStore<RepositoryActionsState>((set, get, store) => {
    const live = (scope: ActionScope) => !!owner && sameActionScope(owner.history.getState().preferences.getState().scope, scope) && sameActionScope(get().scope, scope);
    const load = async () => {
      const scope = get().scope; if (!scope) return;
      const ticket = ++readTicket; actionTicket++;
      set({ loading: true, error: null, prepared: null, preview: null, preparing: false });
      const result = await api('get_ref_actions', { scope }).catch(() => ({ ok: false as const, error: transportError() }));
      if (ticket !== readTicket || !live(scope)) return;
      if (!result.ok) { set({ loading: false, error: result.error }); return; }
      if (!sameActionScope(scope, result.value.scope)) { set({ loading: false, error: invalid('Repository context changed. Refresh the context.') }); return; }
      set({ catalog: result.value, loading: false });
    };
    const bind = () => {
      const scope = owner?.history.getState().preferences.getState().scope ?? null, next = JSON.stringify(scope);
      const selected = owner?.history.getState().selected;
      const nextSelection = JSON.stringify(selected && [selected.context, selected.commit.handle]);
      if (nextSelection !== selectionKey) {
        selectionKey = nextSelection;
        if (!get().executing) { actionTicket++; set({ prepared: null, preview: null, preparing: false }); }
      }
      if (next === key) return;
      key = next; readTicket++; actionTicket++;
      set({ scope, catalog: null, loading: false, error: null, prepared: null, preview: null, preparing: false, executing: false, actionError: null });
      if (scope) void load();
    };
    const schedule = () => { if (scheduled) return; scheduled = true; queueMicrotask(() => { scheduled = false; if (owner) bind(); }); };
    return { scope: null, catalog: null, loading: false, error: null, prepared: null, preview: null, preparing: false, executing: false, actionError: null,
      connect: history => {
        if (owner && owner.history !== history) throw Error('Repository actions already have a history owner');
        if (!owner) { const a = history.subscribe(schedule), b = history.getState().preferences.subscribe(schedule); owner = { history, users: 0, stop: () => { a(); b(); } }; schedule(); }
        const connection = owner; connection.users++; let disposed = false;
        return () => { if (disposed) return; disposed = true; connection.users--; queueMicrotask(() => {
          if (owner !== connection || connection.users) return;
          owner.stop(); owner = null; key = null; selectionKey = null; readTicket++; actionTicket++;
          set({ scope: null, catalog: null, loading: false, prepared: null, preview: null, preparing: false, executing: false });
        }); };
      },
      awaitCatalog: signal => new Promise(resolve => {
        let done = false, unsubscribe = () => {};
        const finish = (value: boolean) => { if (done) return; done = true; unsubscribe(); signal.removeEventListener('abort', abort); resolve(value); };
        const abort = () => finish(false);
        const check = () => {
          const history = owner?.history.getState(), scope = history?.preferences.getState().scope, state = get();
          if (signal.aborted || !history || !scope && !history.loading) { finish(false); return; }
          if (!history.loading && sameActionScope(scope, state.scope) && !state.loading && (state.error || sameActionScope(scope, state.catalog?.scope))) finish(true);
        };
        unsubscribe = store.subscribe(check); signal.addEventListener('abort', abort); check();
      }),
      reload: load,
      clearPrepared: () => { if (!get().executing) actionTicket++; set({ prepared: null, preview: null, preparing: false, actionError: null }); },
      prepare: async draft => {
        const state = get(), { scope, catalog } = state;
        if (!scope || !catalog || state.loading || state.error || state.preparing || state.executing || !live(scope) || !sameActionScope(scope, catalog.scope)) return;
        const initialHistory = owner?.history.getState(), initialSelection = initialHistory?.selected;
        if (draft.source.kind === 'selectedCommit' && (!initialHistory || initialHistory.loading || !initialSelection || !initialHistory.session || !sameActionScope(initialSelection.context.scope, scope) || !sameContext(initialSelection.context, initialHistory.session.context))) {
          set({ prepared: null, preview: null, actionError: invalid('Select the commit again in the current history snapshot.') }); return;
        }
        const ticket = ++actionTicket, catalogId = catalog.catalogId;
        const selectedAtStart = owner?.history.getState().selected;
        const current = () => {
          const history = owner?.history.getState(), selected = history?.selected;
          return ticket === actionTicket && live(scope) && get().catalog?.catalogId === catalogId && (draft.source.kind !== 'selectedCommit' || !!selectedAtStart && !!selected && !!history?.session && !history.loading && sameContext(selected.context, selectedAtStart.context) && sameContext(history.session.context, selected.context) && selected.commit.handle === selectedAtStart.commit.handle);
        };
        set({ preparing: true, prepared: null, preview: null, actionError: null });
        try {
          const resolve = async (choice: ActionSourceChoice, expected?: 'branch' | 'remoteRef' | 'tag'): Promise<ResolvedRefActionTarget> => {
            let source: RefActionSource, resolvedKind: ResolvedRefActionTarget['kind'] = 'commit';
            if (choice.kind === 'ref') {
              const entry = catalog.refs.find(item => item.name === choice.name);
              const kind = expected === 'branch' ? 'local' : expected === 'remoteRef' ? 'remote' : expected === 'tag' ? 'tag' : null;
              if (!entry || entry.symbolicTarget || kind && entry.kind !== kind || !expected && !entry.canStart) throw invalid('Select an available qualified reference from the current catalog.');
              source = { kind: 'ref', entryId: entry.entryId };
              resolvedKind = entry.kind === 'local' ? 'branch' : entry.kind === 'remote' ? 'remoteRef' : 'tag';
            } else if (choice.kind === 'selectedCommit') {
              const history = owner?.history.getState(), selected = history?.selected;
              if (!history || history.loading || !selected || !history.session || !sameActionScope(selected.context.scope, scope) || !sameContext(selected.context, history.session.context)) throw invalid('Select the commit again in the current history snapshot.');
              source = { kind: 'historyCommit', context: selected.context, commitHandle: selected.commit.handle };
            } else source = { kind: 'head' };
            if (expected && source.kind !== 'ref') throw invalid('This action requires a reference of the correct kind.');
            const result = await api('resolve_ref_action_target', { scope, catalogId, source }).catch(() => ({ ok: false as const, error: transportError() }));
            if (!current()) throw invalid('Repository context changed. Prepare again.');
            if (!result.ok) throw result.error;
            if (!sameActionScope(result.value.scope, scope) || result.value.catalogId !== catalogId || result.value.kind !== resolvedKind || expected && result.value.kind !== expected || choice.kind === 'ref' && result.value.displayName !== choice.name) throw invalid('The resolved target changed. Prepare again.');
            if (source.kind === 'historyCommit') {
              const history = owner?.history.getState(), selected = history?.selected;
              if (!selected || !history?.session || !sameContext(selected.context, source.context) || !sameContext(history.session.context, source.context) || selected.commit.handle !== source.commitHandle) throw invalid('Selected commit changed. Prepare again.');
            }
            return result.value;
          };
          const expected = ['branchRename', 'branchSwitch', 'branchDelete', 'branchTracking'].includes(draft.kind) ? 'branch' : draft.kind === 'tagDelete' ? 'tag' : undefined;
          const target = await resolve(draft.source, expected);
          if (!current()) return;
          let intent: ActionIntent, destination = draft.name;
          switch (draft.kind) {
            case 'branchCreate': intent = { kind: draft.kind, name: draft.name, startHandle: target.handle, checkout: draft.checkout }; destination += draft.checkout ? ' (checkout)' : ' (keep current branch)'; break;
            case 'branchRename': intent = { kind: draft.kind, branchHandle: target.handle, newName: draft.name }; break;
            case 'branchSwitch': case 'branchDelete': intent = { kind: draft.kind, branchHandle: target.handle }; destination = target.displayName; break;
            case 'tagDelete': intent = { kind: draft.kind, tagHandle: target.handle }; destination = target.displayName; break;
            case 'branchTracking': {
              const upstream = draft.upstream ? await resolve({ kind: 'ref', name: draft.upstream }, 'remoteRef') : null;
              intent = { kind: draft.kind, branchHandle: target.handle, upstreamHandle: upstream?.handle ?? null }; destination = upstream?.displayName ?? 'Unset upstream'; break;
            }
            case 'tagCreate': intent = { kind: draft.kind, name: draft.name, targetHandle: target.handle, annotation: draft.annotated ? { kind: 'annotated', message: draft.message, sign: draft.sign } : { kind: 'lightweight' } }; destination += draft.annotated ? ` (annotated${draft.sign ? ', signed' : ''})` : ' (lightweight)'; break;
          }
          if (!current()) return;
          const result = await api('prepare_repository_action', { scope, intent }).catch(() => ({ ok: false as const, error: transportError() }));
          if (!current()) return;
          if (!result.ok) { set({ actionError: result.error }); return; }
          if (result.value.kind !== draft.kind || !sameActionScope(result.value.scope, scope) || !(Date.parse(result.value.expiresAt) > Date.now())) { set({ actionError: invalid('Preview expired or changed. Prepare again.') }); return; }
          set({ prepared: result.value, preview: { source: target.displayName, destination } });
        } catch (error) { if (current()) set({ actionError: error as AppError }); }
        finally { if (current()) set({ preparing: false }); }
      },
      execute: async () => {
        const state = get(), plan = state.prepared;
        if (state.executing || state.preparing || !plan || !live(plan.scope)) return null;
        if (!(Date.parse(plan.expiresAt) > Date.now())) { set({ prepared: null, preview: null, actionError: invalid('Preview expired. Prepare again.') }); return null; }
        const ticket = ++actionTicket; set({ executing: true, prepared: null, actionError: null });
        try {
          const result = await api('execute_repository_action', { planId: plan.planId }).catch(() => ({ ok: false as const, error: transportError() }));
          if (result.ok && (result.value.kind !== 'action' || result.value.actionKind !== plan.kind || result.value.repoId !== plan.scope.repoId || result.value.worktreeId !== plan.scope.worktreeId)) {
            const error = invalid('Action reply could not be matched. Check Tasks before preparing another action.');
            if (ticket === actionTicket && live(plan.scope)) set({ actionError: error });
            return { ok: false, error };
          }
          if (!result.ok && ticket === actionTicket && live(plan.scope)) set({ actionError: result.error });
          return result;
        } finally { if (ticket === actionTicket) set({ executing: false }); }
      },
    };
  });
}
export const repositoryActionsStore = createRepositoryActionsStore();
