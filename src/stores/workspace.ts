import { createStore } from 'zustand/vanilla';
import { call, transportError } from '../contracts';
import type { AppError, Environment, RepositorySnapshot, Selection, Workspace } from '../contracts';
import { createRefreshCoordinator } from '../features/repository/refresh';
import type { StoreApi } from 'zustand';
import type { HistoryState } from './history';
import { createRepositoryWatch } from '../features/repository/repositoryWatch';
import type { RepositorySubscribe } from '../features/repository/repositoryWatch';
import type { RefreshReason, RefreshInteractionLease } from '../features/repository/refresh';
export type Api = typeof call;
export interface WorkspaceState {
  workspace: Workspace | null; snapshot: RepositorySnapshot | null; environment: Environment | null;
  error: AppError | null; inspectionError: AppError | null; loading: boolean; inspecting: boolean; refreshing: boolean; busy: boolean;
  requestRefresh(reason: RefreshReason): Promise<void>; startRefresh(): () => void;
  beginInteraction(): RefreshInteractionLease | null;
  connectHistory(history: StoreApi<HistoryState>): () => void;
  initialize(): Promise<void>; reload(): Promise<void>; open(): Promise<void>; select(selection: Selection): Promise<void>; forget(repoId: string): Promise<void>; refresh(): Promise<void>; clearError(): void;
}
const same = (a: Selection | null | undefined, b: Selection) => a?.repoId === b.repoId && a.worktreeId === b.worktreeId;
export function createWorkspaceStore(api: Api = call, subscribe?: RepositorySubscribe) {
  let generation = 0n, scopeEpoch = 0;
  let intent = 0, selectionPending = false;
  let selections = Promise.resolve();
  let initializing: Promise<void> | null = null;
  let connection: { history: StoreApi<HistoryState>; users: number; stop(): void; retry(): void; suspend(): void } | null = null;
  return createStore<WorkspaceState>((set, get) => {
    const apply = (workspace: Workspace) => {
      const previous = get().workspace;
      if (previous && BigInt(workspace.revision) < BigInt(previous.revision)) return false;
      const selection = workspace.activeSelection;
      if (!selection || !same(previous?.activeSelection, selection)) { refresh.invalidateInteraction(); generation++; scopeEpoch++; connection?.suspend(); set({ snapshot: null, inspectionError: null, inspecting: false, refreshing: false }); }
      set({ workspace }); return true;
    };
    const inspect = async () => {
      if (selectionPending) return;
      const selection = get().workspace?.activeSelection;
      if (!selection) return;
      const token = (++generation).toString();
      set({ inspecting: true, inspectionError: null });
      const response = await api('inspect_repository', { ...selection, generation: token }).catch(() => ({ ok: false as const, error: transportError() }));
      if (token !== generation.toString() || !same(get().workspace?.activeSelection, selection)) return;
      if (response.ok) {
        if (response.value.generation !== token || !same(response.value, selection)) { set({ inspecting: false }); return; }
        set({ snapshot: response.value, inspecting: false });
      } else set({ inspectionError: response.error, inspecting: false });
    };
    const scope = () => { const s = get().workspace?.activeSelection; return s ? JSON.stringify([s.repoId, s.worktreeId, scopeEpoch]) : null; };
    const run = async () => {
      const epoch = scopeEpoch, selection = get().workspace?.activeSelection;
      if (selectionPending || !selection) return;
      connection?.retry();
      await inspect();
      if (selectionPending || epoch !== scopeEpoch || !same(get().workspace?.activeSelection, selection)) return;
      const history = connection?.history;
      if (!history) return;
      const current = history.getState().scope;
      await (current && same(current, selection) ? history.getState().refresh() : history.getState().activate(selection));
    };
    const refresh = createRefreshCoordinator({ scope, run, feedback: (key, visible) => { if (key === scope()) set({ refreshing: visible }); } });
    return { workspace: null, snapshot: null, environment: null, error: null, inspectionError: null, loading: true, inspecting: false, refreshing: false, busy: false,
      requestRefresh: reason => selectionPending ? Promise.resolve() : refresh.request(reason),
      beginInteraction: () => selectionPending || get().busy ? null : refresh.acquireInteraction(),
      startRefresh: () => refresh.start(),
      connectHistory: history => {
        if (connection && connection.history !== history) throw Error('Workspace already has a history owner');
        if (!connection) {
          const watch = createRepositoryWatch(() => { void get().requestRefresh('watch'); }, api, subscribe);
          const preferences = history.getState().preferences;
          const bind = () => watch.bind(selectionPending ? null : preferences.getState().scope);
          const stopPreferences = preferences.subscribe(bind);
          const stopRefresh = refresh.start();
          connection = { history, users: 0, retry: watch.retry,
            suspend: () => { watch.bind(null); history.getState().deactivate(); },
            stop: () => { refresh.invalidateInteraction(); stopRefresh(); stopPreferences(); watch.dispose(); history.getState().deactivate(); } };
          bind();
        }
        const owner = connection; owner.users++;
        let disposed = false;
        return () => {
          if (disposed) return; disposed = true; owner.users--;
          // StrictMode reclaims the owner before this microtask. Native promises
          // use separate tickets, so late acknowledgements are still disposed.
          queueMicrotask(() => { if (connection === owner && !owner.users) { owner.stop(); connection = null; } });
        };
      },
      clearError: () => set({ error: null }),
      initialize: () => {
        if (initializing) return initializing;
        const work = (async () => {
          set({ loading: true });
          const environment = api('get_environment', {}).then(value => { if (value.ok) set({ environment: value.value }); });
          const result = await api('get_workspace', {});
          if (result.ok) { if (apply(result.value)) await refresh.request('operation'); } else set({ error: result.error });
          set({ loading: false }); await environment;
        })();
        initializing = work;
        void work.finally(() => { if (initializing === work) initializing = null; }).catch(() => {});
        return work;
      },
      reload: async () => {
        const result = await api('get_workspace', {});
        if (result.ok) { if (apply(result.value)) await refresh.request('operation'); } else set({ error: result.error });
      },
      open: async () => {
        if (get().busy) return; set({ busy: true, error: null });
        const result = await api('choose_repository', {});
        if (result.ok) { if (result.value && apply(result.value)) await refresh.request('operation'); } else set({ error: result.error });
        set({ busy: false });
      },
      select: selection => {
        if (!selectionPending && same(get().workspace?.activeSelection, selection)) return Promise.resolve();
        const ticket = ++intent; selectionPending = true;
        refresh.invalidateInteraction();
        generation++; scopeEpoch++; connection?.suspend(); set({ snapshot: null, inspecting: true, refreshing: false, inspectionError: null, error: null, busy: true });
        const work = selections.then(async () => {
          if (ticket !== intent) return;
          const result = await api('select_repository', selection);
          if (ticket !== intent) return;
          selectionPending = false;
          if (result.ok) { if (apply(result.value)) await refresh.request('operation'); } else { set({ error: result.error, inspecting: false }); await refresh.request('operation'); }
          if (ticket === intent) set({ busy: false });
        });
        selections = work; return work;
      },
      forget: repoId => {
        if (get().busy) return Promise.resolve();
        const active = get().workspace?.activeSelection?.repoId === repoId;
        const ticket = active ? ++intent : intent;
        if (active) {
          refresh.invalidateInteraction();
          selectionPending = true; generation++; scopeEpoch++; connection?.suspend();
          set({ busy: true, snapshot: null, inspecting: false, refreshing: false });
        }
        const work = selections.then(async () => {
          const result = await api('forget_repository', { repoId });
          if (ticket !== intent) return;
          if (active) selectionPending = false;
          if (result.ok) {
            if (apply(result.value) && !get().snapshot && get().workspace?.activeSelection) await refresh.request('operation');
          } else {
            set({ error: result.error });
            if (active) await refresh.request('operation');
          }
          if (active && ticket === intent) set({ busy: false });
        });
        selections = work; return work;
      },
      refresh: () => get().requestRefresh('manual'),
    };
  });
}
export const workspaceStore = createWorkspaceStore();
