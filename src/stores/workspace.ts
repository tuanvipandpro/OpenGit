import { createStore } from 'zustand/vanilla';
import { call } from '../contracts';
import type { AppError, Environment, RepositorySnapshot, Selection, Workspace } from '../contracts';
export type Api = typeof call;
export interface WorkspaceState {
  workspace: Workspace | null; snapshot: RepositorySnapshot | null; environment: Environment | null;
  error: AppError | null; inspectionError: AppError | null; loading: boolean; inspecting: boolean; busy: boolean;
  initialize(): Promise<void>; reload(): Promise<void>; open(): Promise<void>; select(selection: Selection): Promise<void>; forget(repoId: string): Promise<void>; refresh(): Promise<void>; clearError(): void;
}
const same = (a: Selection | null | undefined, b: Selection) => a?.repoId === b.repoId && a.worktreeId === b.worktreeId;
export function createWorkspaceStore(api: Api = call) {
  let generation = 0n;
  let intent = 0, selectionPending = false;
  let selections = Promise.resolve();
  return createStore<WorkspaceState>((set, get) => {
    const apply = (workspace: Workspace) => {
      const previous = get().workspace;
      if (previous && BigInt(workspace.revision) < BigInt(previous.revision)) return false;
      const selection = workspace.activeSelection;
      if (!selection || !same(previous?.activeSelection, selection)) { generation++; set({ snapshot: null, inspectionError: null, inspecting: false }); }
      set({ workspace }); return true;
    };
    const inspect = async () => {
      const selection = get().workspace?.activeSelection;
      if (!selection) return;
      const token = (++generation).toString();
      set({ inspecting: true, inspectionError: null });
      const response = await api('inspect_repository', { ...selection, generation: token });
      if (token !== generation.toString() || !same(get().workspace?.activeSelection, selection)) return;
      if (response.ok) {
        if (response.value.generation !== token || !same(response.value, selection)) { set({ inspecting: false }); return; }
        set({ snapshot: response.value, inspecting: false });
      } else set({ inspectionError: response.error, inspecting: false });
    };
    return { workspace: null, snapshot: null, environment: null, error: null, inspectionError: null, loading: true, inspecting: false, busy: false,
      clearError: () => set({ error: null }),
      initialize: async () => {
        set({ loading: true });
        const environment = api('get_environment', {}).then(value => { if (value.ok) set({ environment: value.value }); });
        const result = await api('get_workspace', {});
        if (result.ok) { if (apply(result.value)) await inspect(); } else set({ error: result.error });
        set({ loading: false }); await environment;
      },
      reload: async () => {
        const result = await api('get_workspace', {});
        if (result.ok) { if (apply(result.value)) await inspect(); } else set({ error: result.error });
      },
      open: async () => {
        if (get().busy) return; set({ busy: true, error: null });
        const result = await api('choose_repository', {});
        if (result.ok) { if (result.value && apply(result.value)) await inspect(); } else set({ error: result.error });
        set({ busy: false });
      },
      select: selection => {
        if (!selectionPending && same(get().workspace?.activeSelection, selection)) return Promise.resolve();
        const ticket = ++intent; selectionPending = true;
        generation++; set({ snapshot: null, inspecting: true, inspectionError: null, error: null, busy: true });
        const work = selections.then(async () => {
          if (ticket !== intent) return;
          const result = await api('select_repository', selection);
          if (ticket !== intent) return;
          selectionPending = false;
          if (result.ok) { if (apply(result.value)) await inspect(); } else set({ error: result.error, inspecting: false });
          if (ticket === intent) set({ busy: false });
        });
        selections = work; return work;
      },
      forget: async repoId => {
        if (get().busy) return;
        const result = await api('forget_repository', { repoId });
        if (result.ok) { if (apply(result.value) && !get().snapshot && get().workspace?.activeSelection) await inspect(); } else set({ error: result.error });
      },
      refresh: async () => { if (!selectionPending) await inspect(); },
    };
  });
}
export const workspaceStore = createWorkspaceStore();
