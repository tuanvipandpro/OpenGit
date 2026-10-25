import { createStore } from 'zustand/vanilla';
import { call, transportError } from '../contracts';
import type { AppError, HistoryScope, RepositoryView } from '../contracts';
import type { Api } from './workspace';

export const defaultRepositoryView = (): RepositoryView => ({ filter: { hiddenRefs: [], soloRefs: [] }, selectedOid: null, anchorOid: null, anchorOffset: 0, sidebarWidth: 280, detailsHeight: 320, sidebarCollapsed: false, diffLayout: 'sideBySide' });
type Field = keyof RepositoryView;
type Intent = Partial<RepositoryView>;
const fields = Object.keys(defaultRepositoryView()) as Field[];
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const scopeMatches = (a: HistoryScope, b: HistoryScope) => a.repoId === b.repoId && a.worktreeId === b.worktreeId && a.generation === b.generation;
const oid = (value: string | null) => value && /^(?:[a-f\d]{40}|[a-f\d]{64})$/i.test(value) ? value.toLowerCase() : null;
const clamp = (value: number, min: number, max: number) => Number.isFinite(value) ? Math.max(min, Math.min(max, value)) : min;
export function normalizeRepositoryView(view: RepositoryView): RepositoryView {
  const durableRefs = (names: string[]) => [...new Set(names.filter(name => !name.startsWith('refs/stash/')))].sort();
  return { ...view, filter: { hiddenRefs: durableRefs(view.filter.hiddenRefs), soloRefs: durableRefs(view.filter.soloRefs) },
    selectedOid: oid(view.selectedOid), anchorOid: oid(view.anchorOid), anchorOffset: clamp(view.anchorOffset, -10_000, 10_000),
    sidebarWidth: clamp(view.sidebarWidth, 160, 640), detailsHeight: clamp(view.detailsHeight, 160, 1200) };
}
export interface HistoryViewState {
  scope: HistoryScope | null; view: RepositoryView; revision: string; hydrated: boolean; loading: boolean; saving: boolean; error: AppError | null;
  activate(scope: HistoryScope): Promise<RepositoryView | null>; bindScope(scope: HistoryScope): void;
  patch(patch: Partial<RepositoryView>): void; flush(): Promise<void>; deactivate(): void;
}

/** Persist only durable field intent. CAS conflicts merge remote untouched fields
 * with the newest local edits, never replay an older whole-view snapshot. */
export function createHistoryViewStore(api: Api = call) {
  let epoch = 0, timer: ReturnType<typeof setTimeout> | undefined;
  let flight: Promise<void> | null = null;
  const pending = new Map<string, Intent>();
  let editVersion = 0n;
  const pendingVersions = new Map<string, Partial<Record<Field, bigint>>>();
  const cancelTimer = () => { clearTimeout(timer); timer = undefined; };
  return createStore<HistoryViewState>((set, get) => {
    const schedule = () => { cancelTimer(); timer = setTimeout(() => { void get().flush(); }, 350); };
    const fail = (): AppError => ({ code: 'Internal', phase: 'inspection', retry: 'retry', message: 'View preferences could not be synchronized. Try saving again.' });
    const merge = (remote: RepositoryView, scope: HistoryScope) => normalizeRepositoryView({ ...remote, ...pending.get(scope.repoId) });
    const save = async (version: number, conflicts = 0): Promise<void> => {
      const state = get(), scope = state.scope;
      if (!scope || !state.hydrated || version !== epoch) return;
      const intent = { ...pending.get(scope.repoId) };
      const dispatchedVersions = { ...pendingVersions.get(scope.repoId) };
      if (!Object.keys(intent).length) return;
      const sent = normalizeRepositoryView(state.view);
      set({ saving: true, error: null });
      const result = await api('save_repository_view', { scope, expectedRevision: state.revision, view: sent }).catch(() => ({ ok: false as const, error: transportError() }));
      if (result.ok) {
        if (!scopeMatches(result.value.scope, scope)) { if (version === epoch) set({ saving: false, error: fail() }); return; }
        const remaining = { ...pending.get(scope.repoId) };
        const remainingVersions = { ...pendingVersions.get(scope.repoId) };
        for (const field of fields) if (field in intent && dispatchedVersions[field] !== undefined && remainingVersions[field] === dispatchedVersions[field]) {
          delete remaining[field]; delete remainingVersions[field];
        }
        pending.set(scope.repoId, remaining);
        pendingVersions.set(scope.repoId, remainingVersions);
        // A successful old-scope save is durable even after switching away.
        // Retire only the acknowledged intent; never replay it over a later
        // window's changes when this repository is opened again.
        if (version !== epoch) return;
        set({ revision: result.value.viewRevision, view: merge(result.value.view, scope), saving: false });
        if (Object.keys(remaining).length) schedule();
        return;
      }
      if (version !== epoch) return;
      if (result.error.code !== 'ExternalChange' || conflicts >= 1) { set({ saving: false, error: result.error }); return; }
      // Refetch under the current generation: a refresh may have rebound scope
      // while the save was in flight. Newer local field intent stays pending.
      const currentScope = get().scope!;
      const latest = await api('get_repository_view', { scope: currentScope }).catch(() => ({ ok: false as const, error: transportError() }));
      if (version !== epoch) return;
      if (!latest.ok) { set({ saving: false, error: latest.error }); return; }
      if (!scopeMatches(latest.value.scope, currentScope)) { set({ saving: false, error: fail() }); return; }
      set({ revision: latest.value.viewRevision, view: merge(latest.value.view, currentScope), saving: false });
      await save(version, conflicts + 1);
    };
    return { scope: null, view: defaultRepositoryView(), revision: '0', hydrated: false, loading: false, saving: false, error: null,
      activate: async scope => {
        cancelTimer(); const version = ++epoch, previous = flight;
        set({ scope, hydrated: false, loading: true, saving: false, error: null, view: defaultRepositoryView() });
        // An already issued save is allowed to settle before reading the same
        // common-repo preferences (including a linked-worktree switch).
        await previous;
        if (version !== epoch) return null;
        const result = await api('get_repository_view', { scope }).catch(() => ({ ok: false as const, error: transportError() }));
        if (version !== epoch) return null;
        if (!result.ok) { set({ loading: false, error: result.error }); return null; }
        if (!scopeMatches(result.value.scope, scope)) { set({ loading: false, error: fail() }); return null; }
        const view = merge(result.value.view, scope);
        set({ view, revision: result.value.viewRevision, hydrated: true, loading: false });
        if (Object.keys(pending.get(scope.repoId) ?? {}).length) schedule();
        return view;
      },
      bindScope: scope => { if (get().scope?.repoId === scope.repoId && get().scope?.worktreeId === scope.worktreeId) set({ scope }); },
      patch: patch => {
        const state = get(); if (!state.scope || !state.hydrated) return;
        const view = normalizeRepositoryView({ ...state.view, ...patch }), intent = { ...pending.get(state.scope.repoId) };
        const versions = { ...pendingVersions.get(state.scope.repoId) };
        let changed = false;
        for (const field of fields) if (!same(view[field], state.view[field])) { Object.assign(intent, { [field]: view[field] }); versions[field] = ++editVersion; changed = true; }
        if (!changed) return;
        pending.set(state.scope.repoId, intent); pendingVersions.set(state.scope.repoId, versions); set({ view, error: null }); schedule();
      },
      flush: async () => {
        cancelTimer(); const version = epoch;
        for (let round = 0; round < 2; round++) {
          if (!flight) {
            const operation = save(version); flight = operation;
            void operation.finally(() => { if (flight === operation) flight = null; });
          }
          await flight;
          if (version !== epoch || get().error) return;
          cancelTimer();
          const scope = get().scope;
          if (!scope || !Object.keys(pending.get(scope.repoId) ?? {}).length) return;
        }
        // Do not let an endless stream of edits delay a repository transition.
        // The caller keeps the view open and offers an explicit retry.
        set({ error: fail(), saving: false });
      },
      deactivate: () => {
        cancelTimer(); void get().flush(); epoch++;
        set({ scope: null, hydrated: false, loading: false, saving: false, error: null });
      },
    };
  });
}
