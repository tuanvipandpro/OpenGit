import { createStore } from 'zustand/vanilla';
import { call, transportError } from '../contracts';
import type { AppError, CommitDetails, CommitDiff, CommitFile, DiffBase, HistoryContext } from '../contracts';
import type { Api } from './workspace';
import type { HistorySelection } from './history';

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const sameCommit = (a: HistorySelection | null, b: HistorySelection) => !!a && a.commit.oid === b.commit.oid
  && a.context.scope.repoId === b.context.scope.repoId && a.context.scope.worktreeId === b.context.scope.worktreeId
  && a.commit.parents.length === b.commit.parents.length && a.commit.parents.every((parent, index) => parent.oid === b.commit.parents[index].oid);
const initialBase = (selected: HistorySelection): DiffBase => selected.commit.parents.length ? { kind: 'parent', index: 0 } : { kind: 'root' };
const badResponse = (): AppError => ({ code: 'Internal', phase: 'inspection', retry: 'retry', message: 'Commit information could not be displayed. Select the commit again.' });
// Display metadata is an intent hint, never a path argument or object identity.
const fileKey = (file: CommitFile) => JSON.stringify([file.pathDisplay, file.previousPathDisplay, file.kind, file.oldMode, file.newMode]);
type FileIntent = { key: string; pathDisplay: string; unique: boolean };
export interface CommitInspectorState {
  selected: HistorySelection | null; base: DiffBase; details: CommitDetails | null; fileHandle: string | null; diff: CommitDiff | null; displayedFile: CommitFile | null; displayedBase: DiffBase | null;
  loading: boolean; diffLoading: boolean; refreshing: boolean; notice: string | null; error: AppError | null; diffError: AppError | null;
  select(selected: HistorySelection): Promise<void>; setBase(base: DiffBase): Promise<void>; selectFile(handle: string): Promise<void>; retry(): Promise<void>; clear(): void;
  beginRefresh(context: HistoryContext): void; refreshFailed(): void;
}
export function createCommitInspectorStore(api: Api = call) {
  let version = 0, fileVersion = 0;
  let suspended: HistoryContext | null = null, intent: FileIntent | null = null;
  return createStore<CommitInspectorState>((set, get) => {
    const details = async (selected: HistorySelection, base: DiffBase, preserve: boolean) => {
      const ticket = ++version; fileVersion++;
      set({ selected, base, loading: true, refreshing: preserve, diffLoading: false, error: null, diffError: null,
        ...(!preserve ? { details: null, fileHandle: null, diff: null, displayedFile: null, displayedBase: null, notice: null } : {}) });
      const result = await api('get_commit_details', { context: selected.context, commitHandle: selected.commit.handle, base }).catch(() => ({ ok: false as const, error: transportError() }));
      if (ticket !== version) return;
      if (!result.ok) { set({ loading: false, refreshing: false, error: result.error }); return; }
      if (!same(result.value.context, selected.context) || result.value.commit.handle !== selected.commit.handle || result.value.commit.oid !== selected.commit.oid || !same(result.value.base, base)) {
        set({ loading: false, refreshing: false, error: badResponse() }); return;
      }
      const files = result.value.files;
      set({ details: result.value, loading: false });
      let file = files[0];
      // Consult the current intent after awaiting: a later click on the old
      // displayed list must win over the choice made when refresh started.
      if (preserve && intent) {
        const matches = files.filter(file => fileKey(file) === intent!.key);
        const uniquePath = files.filter(file => file.pathDisplay === intent!.pathDisplay).length === 1;
        if (!intent.unique || !uniquePath || matches.length !== 1) {
          set({ fileHandle: null, diff: null, displayedFile: null, displayedBase: null, refreshing: false, diffLoading: false,
            notice: 'The previous file cannot be restored unambiguously. Choose a file from the refreshed list.' });
          return;
        }
        file = matches[0];
      }
      if (file) await get().selectFile(file.handle);
      else set({ fileHandle: null, diff: null, displayedFile: null, displayedBase: null, refreshing: false });
    };
    return { selected: null, base: { kind: 'root' }, details: null, fileHandle: null, diff: null, displayedFile: null, displayedBase: null,
      loading: false, diffLoading: false, refreshing: false, notice: null, error: null, diffError: null,
      select: selected => {
        const state = get(), preserve = sameCommit(state.selected, selected);
        const base = preserve ? state.base : initialBase(selected);
        if (!preserve) intent = null;
        // During history staging only capture the new user intent; issuing old
        // handles here can race backend revocation. Rebind after the new scope.
        if (suspended && same(suspended, selected.context)) {
          version++; fileVersion++;
          set({ selected, base, refreshing: true, loading: false, diffLoading: false, error: null, diffError: null,
            ...(!preserve ? { details: null, diff: null, displayedFile: null, displayedBase: null, fileHandle: null, notice: null } : {}) });
          return Promise.resolve();
        }
        suspended = null;
        return details(selected, base, preserve);
      },
      setBase: base => {
        const state = get(), selected = state.selected;
        if (!selected || base.kind === 'root' && selected.commit.parents.length || base.kind === 'parent' && (!Number.isInteger(base.index) || base.index < 0 || base.index >= selected.commit.parents.length)) return Promise.resolve();
        if (suspended) { version++; fileVersion++; set({ base, error: null, diffError: null }); return Promise.resolve(); }
        const preserve = state.refreshing;
        if (!preserve) intent = null;
        return details(selected, base, preserve);
      },
      selectFile: async handle => {
        const state = get(), value = state.details, file = value?.files.find(file => file.handle === handle);
        if (!value || !file) return;
        intent = { key: fileKey(file), pathDisplay: file.pathDisplay, unique: value.files.filter(candidate => candidate.pathDisplay === file.pathDisplay).length === 1 };
        const ticket = ++fileVersion, parent = version;
        if (suspended || !state.selected || !same(value.context, state.selected.context) || !same(value.base, state.base)) {
          set({ fileHandle: handle, notice: null }); return;
        }
        const preserve = state.refreshing || !!state.diffError;
        set({ fileHandle: handle, diffLoading: true, diffError: null, notice: null, ...(!preserve ? { diff: null, displayedFile: null, displayedBase: null } : {}) });
        const result = await api('get_commit_diff', { context: value.context, fileHandle: handle }).catch(() => ({ ok: false as const, error: transportError() }));
        if (ticket !== fileVersion || parent !== version) return;
        if (!result.ok) { set({ diffLoading: false, refreshing: false, diffError: result.error }); return; }
        if (!same(result.value.context, value.context) || result.value.fileHandle !== handle) { set({ diffLoading: false, refreshing: false, diffError: badResponse() }); return; }
        set({ diff: result.value, displayedFile: file, displayedBase: value.base, diffLoading: false, refreshing: false });
      },
      retry: () => { const state = get(); return !suspended && state.selected ? details(state.selected, state.base, !!state.details) : Promise.resolve(); },
      beginRefresh: context => { version++; fileVersion++; suspended = context; set({ refreshing: true, loading: false, diffLoading: false, error: null, diffError: null, notice: null }); },
      refreshFailed: () => { set({ refreshing: false, loading: false, diffLoading: false, notice: get().details ? 'History refresh failed. The previous comparison is retained; refresh history to retry.' : null }); },
      clear: () => { version++; fileVersion++; suspended = null; intent = null; set({ selected: null, details: null, fileHandle: null, diff: null, displayedFile: null, displayedBase: null, loading: false, diffLoading: false, refreshing: false, notice: null, error: null, diffError: null }); },
    };
  });
}
