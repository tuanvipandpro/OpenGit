import { createStore } from 'zustand/vanilla';
import { call, transportError, HISTORY_LIMITS } from '../contracts';
import type { ApiResult, AppError, Selection, HistoryScope, HistoryContext, HistorySession, HistoryFilter, CommitSummary, HistorySearchHit, HistorySearchPage, HistoryRef } from '../contracts';
import type { Api } from './workspace';
import { appendHistoryLayout, createHistoryLayout } from '../features/history/layout';
import type { HistoryLayout } from '../features/history/layout';
import type { HistoryAnchor } from '../features/history/CommitRows';
import { reconcileHistoryFilter } from '../features/history/visibility';
import { createHistoryViewStore } from './historyView';
import { createCommitInspectorStore } from './commitInspector';

export type HistorySelection = { context: HistoryContext; commit: CommitSummary; outsideView: boolean };
export type HistoryViewHint = { filter: HistoryFilter; anchor: HistoryAnchor | null; selectedOid: string | null };
export type HistorySearchState = { resultQuery: string | null; hits: HistorySearchHit[]; refs: HistoryRef[]; nextCursor: string | null; loading: boolean; retained: boolean; error: AppError | null };
export interface HistoryState {
  preferences: ReturnType<typeof createHistoryViewStore>; inspector: ReturnType<typeof createCommitInspectorStore>; restoringSelection: boolean;
  scope: HistoryScope | null; session: HistorySession | null; layout: HistoryLayout; filter: HistoryFilter; anchor: HistoryAnchor | null;
  loading: boolean; refreshing: boolean; pageLoading: boolean; error: AppError | null; pageError: AppError | null; nextCursor: string | null;
  selected: HistorySelection | null; selectionLoading: boolean; selectionError: AppError | null;
  query: string; searchResults: HistorySearchState;
  activate(selection: Selection, hint?: Partial<HistoryViewHint>): Promise<void>; deactivate(): void;
  refresh(): Promise<void>; setFilter(filter: HistoryFilter): Promise<void>; loadMore(): Promise<void>;
  setAnchor(anchor: HistoryAnchor): void; setQuery(query: string): void; search(query?: string): Promise<void>; searchMore(): Promise<void>;
  selectCommit(oid: string): void; selectSearchHit(hit: HistorySearchHit): void; selectRef(ref: HistoryRef): Promise<void>;
}
const emptyFilter = (): HistoryFilter => ({ hiddenRefs: [], soloRefs: [] });
const canonical = (filter: HistoryFilter): HistoryFilter => ({ hiddenRefs: [...new Set(filter.hiddenRefs)].sort(), soloRefs: [...new Set(filter.soloRefs)].sort() });
const emptySearch = (): HistorySearchState => ({ resultQuery: null, hits: [], refs: [], nextCursor: null, loading: false, retained: false, error: null });
const sameScope = (a: HistoryScope | null, b: HistoryScope) => a?.repoId === b.repoId && a.worktreeId === b.worktreeId && a.generation === b.generation;
const sameContext = (a: HistoryContext, b: HistoryContext) => sameScope(a.scope, b.scope) && a.snapshotId === b.snapshotId;
const displayError = (): AppError => ({ code: 'Internal', phase: 'inspection', retry: 'retry', message: 'History could not be displayed. Refresh history to try again.' });

/** Session/cursor/handle state is runtime-only. Workspace owns activation and
 * awaits refresh (including inspector restoration) in its single coordinator. */
export function createHistoryStore(api: Api = call) {
  const preferences = createHistoryViewStore(api), inspector = createCommitInspectorStore(api);
  let generation = 0n, epoch = 0, searchTicket = 0, selectionTicket = 0;
  let submittedSearch: string | null = null;
  let desiredSelection: { repoId: string; oid: string } | null = null;
  let opening: { ticket: number; promise: Promise<void> } | null = null;
  let paging: { ticket: number; promise: Promise<void> } | null = null;
  const hints = new Map<string, HistoryViewHint>();
  return createStore<HistoryState>((set, get) => {
    const remember = () => {
      const state = get();
      if (state.scope) hints.set(state.scope.repoId, { filter: canonical(state.filter), anchor: state.anchor && { ...state.anchor }, selectedOid: (desiredSelection?.repoId === state.scope.repoId ? desiredSelection.oid : null) ?? state.selected?.commit.oid ?? hints.get(state.scope.repoId)?.selectedOid ?? null });
    };
    const current = (ticket: number, context?: HistoryContext) => ticket === epoch && (!context || !!get().session && sameContext(get().session!.context, context));
    const reveal = async (oid: string, version: number, selectedVersion: number, context: HistoryContext) => {
      while (current(version, context) && selectionTicket === selectedVersion && !get().layout.nodesByOid.has(oid)) {
        if (get().loading || get().pageError || !get().nextCursor) return;
        const cursor = get().nextCursor;
        await get().loadMore();
        if (!current(version, context) || selectionTicket !== selectedVersion) return;
        if (get().nextCursor === cursor) return; // Error or no progress: require an explicit retry.
      }
    };
    const choose = (commit: CommitSummary, outsideView: boolean, restoring = false): Promise<void> => {
      const state = get(); if (!state.session || !sameScope(state.scope, state.session.context.scope)) return Promise.resolve();
      desiredSelection = { repoId: state.session.context.scope.repoId, oid: commit.oid };
      const ticket = ++selectionTicket;
      const selected = { context: state.session.context, commit, outsideView };
      set({ selected, selectionLoading: false, selectionError: null, restoringSelection: restoring }); remember();
      if (!restoring) preferences.getState().patch({ selectedOid: commit.oid });
      const inspecting = inspector.getState().select(selected);
      return restoring ? inspecting : outsideView ? Promise.resolve() : reveal(commit.oid, epoch, ticket, state.session.context);
    };
    const resolveOid = async (oid: string, restoring = false) => {
      const state = get(), session = state.session;
      if (!session || !sameScope(state.scope, session.context.scope)) return;
      desiredSelection = { repoId: session.context.scope.repoId, oid };
      const node = state.layout.nodesByOid.get(oid);
      if (node) { await choose(node.commit as CommitSummary, false, restoring); return; }
      const ticket = ++selectionTicket, version = epoch;
      if (!restoring) inspector.getState().clear();
      set({ selected: null, selectionLoading: true, selectionError: null });
      let cursor: string | null = null;
      const visited = new Set<string>();
      do {
        const result: ApiResult<HistorySearchPage> = await api('search_history', { context: session.context, sessionId: session.sessionId, query: oid, cursor, pageSize: null }).catch(() => ({ ok: false as const, error: transportError() }));
        if (!current(version, session.context) || ticket !== selectionTicket) return;
        if (!result.ok) { if (restoring) inspector.getState().refreshFailed(); set({ selectionLoading: false, selectionError: result.error }); return; }
        if (!sameContext(result.value.context, session.context) || result.value.sessionId !== session.sessionId || result.value.query !== oid) { if (restoring) inspector.getState().refreshFailed(); set({ selectionLoading: false, selectionError: displayError() }); return; }
        const hit = result.value.commits.find(hit => hit.commit.oid === oid);
        if (hit) { await choose(hit.commit, hit.outsideView, restoring); return; }
        cursor = result.value.nextCursor;
        if (cursor && (visited.has(cursor) || visited.size >= HISTORY_LIMITS.cursorsPerSession)) { if (restoring) inspector.getState().refreshFailed(); set({ selectionLoading: false, selectionError: displayError() }); return; }
        if (cursor) visited.add(cursor);
      } while (cursor);
      set({ selectionLoading: false, selectionError: { code: 'NotFound', phase: 'inspection', retry: 'retry', message: 'This reference commit is unavailable. Refresh history and try again.' } });
      if (restoring) inspector.getState().refreshFailed();
      if (restoring) { desiredSelection = null; hints.set(session.context.scope.repoId, { filter: get().filter, anchor: get().anchor, selectedOid: null }); if (preferences.getState().view.selectedOid === oid) preferences.getState().patch({ selectedOid: null }); }
    };
    const open = (selection: Selection, filter: HistoryFilter, preserve: boolean, sameRepository: boolean, preparedScope?: HistoryScope): Promise<void> => {
      const before = get(), ticket = ++epoch, scope: HistoryScope = preparedScope ?? { ...selection, generation: (++generation).toString() };
      preferences.getState().bindScope(scope);
      if (preserve && before.session) inspector.getState().beginRefresh(before.session.context); else inspector.getState().clear();
      searchTicket++; selectionTicket++;
      let anchor = preserve ? before.anchor : sameRepository ? null : hints.get(selection.repoId)?.anchor ?? null;
      let selectedOid = (sameRepository ? before.selected?.commit.oid : null) ?? hints.get(selection.repoId)?.selectedOid ?? null;
      const originalSelectedHint = selectedOid;
      const targetCount = preserve ? before.layout.nodes.length : 0;
      set({ ...(!sameRepository ? { scope } : {}), filter: canonical(filter), anchor, loading: true, refreshing: preserve, pageLoading: false, error: null, pageError: null,
        searchResults: sameRepository && submittedSearch !== null ? { ...before.searchResults, loading: true, retained: true, nextCursor: null } : emptySearch(), selectionLoading: false, selectionError: null,
        ...(!sameRepository ? { session: null, layout: createHistoryLayout(), selected: null, nextCursor: null, query: '' } : {}) });
      const promise = (async () => {
        const result = await api('open_history', { scope, filter: canonical(filter), pageSize: null }).catch(() => ({ ok: false as const, error: transportError() }));
        if (!current(ticket)) return;
        if (!result.ok) { if (preserve) inspector.getState().refreshFailed(); set({ searchResults: { ...get().searchResults, loading: false }, loading: false, refreshing: false, error: result.error }); return; }
        const session = result.value;
        if (!sameScope(session.context.scope, scope) || !sameContext(session.page.context, session.context) || session.page.sessionId !== session.sessionId) { if (preserve) inspector.getState().refreshFailed(); set({ searchResults: { ...get().searchResults, loading: false }, loading: false, refreshing: false, error: displayError() }); return; }
        try {
          const objectOid = [session.head.oid, ...session.roots, ...session.refs.map(ref => ref.commitOid), ...session.page.commits.map(commit => commit.oid)].find(value => value && /^(?:[a-f\d]{40}|[a-f\d]{64})$/i.test(value));
          if (objectOid) {
            if (anchor && anchor.oid.length !== objectOid.length) anchor = null;
            if (selectedOid && selectedOid.length !== objectOid.length) selectedOid = null;
            if (desiredSelection?.repoId === selection.repoId && desiredSelection.oid.length !== objectOid.length) desiredSelection = null;
          } else if (!session.page.commits.length && !session.refs.some(ref => ref.commitOid)) { anchor = null; selectedOid = null; desiredSelection = null; }
          let layout = appendHistoryLayout(createHistoryLayout(), session.page.commits), cursor = session.page.nextCursor;
          const visited = new Set<string>();
          const needsRestorePage = () => {
            const restore = preserve ? get().anchor : anchor;
            if (!restore) return layout.nodes.length < targetCount;
            const node = layout.nodesByOid.get(restore.oid), previous = before.layout.nodesByOid.get(restore.oid);
            // Keep the already loaded tail behind the viewport anchor even if
            // new commits shifted it across a page boundary during refresh.
            const tail = preserve && previous ? before.layout.nodes.length - previous.row : 1;
            return layout.nodes.length < targetCount || !node || layout.nodes.length < node.row + tail;
          };
          while (cursor && needsRestorePage()) {
            if (visited.has(cursor) || visited.size >= HISTORY_LIMITS.cursorsPerSession) throw Error('Repeated or excessive cursors'); visited.add(cursor);
            const page = await api('load_history_page', { context: session.context, sessionId: session.sessionId, cursor, pageSize: null }).catch(() => ({ ok: false as const, error: transportError() }));
            if (!current(ticket)) return;
            if (!page.ok) { if (preserve) inspector.getState().refreshFailed(); set({ searchResults: { ...get().searchResults, loading: false }, loading: false, refreshing: false, error: page.error }); return; }
            if (!sameContext(page.value.context, session.context) || page.value.sessionId !== session.sessionId) throw Error('Mismatched page');
            layout = appendHistoryLayout(layout, page.value.commits); cursor = page.value.nextCursor;
          }
          if (!current(ticket)) return;
          const restoreOid = (desiredSelection?.repoId === selection.repoId ? desiredSelection.oid : null) ?? selectedOid;
          const restoredAnchor = (preserve ? get().anchor : anchor);
          set({ scope, session, layout, nextCursor: cursor, loading: false, refreshing: false, filter: reconcileHistoryFilter(filter, session.refs), selected: null, selectionLoading: false, searchResults: submittedSearch !== null ? { ...get().searchResults, loading: true, retained: true, nextCursor: null } : emptySearch(),
            anchor: restoredAnchor && layout.nodesByOid.has(restoredAnchor.oid) ? restoredAnchor : null, restoringSelection: !!restoreOid });
          hints.set(selection.repoId, { filter: get().filter, anchor: get().anchor, selectedOid: restoreOid ?? null });
          // Quiet refresh rebinds this window's runtime selection/filter. It
          // must not replay them over unrelated preferences merged from a CAS
          // conflict with another window. Reconcile only unchanged hints.
          const persisted = preferences.getState().view;
          preferences.getState().patch({
            ...(JSON.stringify(persisted.filter) === JSON.stringify(canonical(filter)) ? { filter: get().filter } : {}),
            ...(persisted.selectedOid === originalSelectedHint ? { selectedOid: restoreOid ?? null } : {}),
            anchorOid: get().anchor?.oid ?? null, anchorOffset: get().anchor?.offset ?? 0,
          });
          remember();
          // Search uses only the new snapshot. Do not put its actionable results
          // behind restoration of the previous inspector comparison.
          const restoring = restoreOid ? resolveOid(restoreOid, true) : Promise.resolve(inspector.getState().clear());
          const replay = submittedSearch !== null ? query(false, submittedSearch, true) : Promise.resolve();
          await Promise.all([restoring, replay]);
        } catch { if (current(ticket)) { if (preserve) inspector.getState().refreshFailed(); set({ searchResults: { ...get().searchResults, loading: false }, loading: false, refreshing: false, error: displayError() }); } }
      })();
      opening = { ticket, promise };
      void promise.finally(() => { if (opening?.ticket === ticket) opening = null; });
      return promise;
    };
    const query = async (more: boolean, requested?: string, replay = false) => {
      const state = get(), session = state.session, text = more ? state.searchResults.resultQuery : requested ?? state.query;
      if (!session || !sameScope(state.scope, session.context.scope) || text === null) return;
      const bytes = new TextEncoder().encode(text).length;
      if (!bytes || bytes > HISTORY_LIMITS.queryBytes || text.includes('\0')) return;
      if (more && (state.searchResults.loading || !state.searchResults.nextCursor)) return;
      if (!more && !replay) submittedSearch = text;
      const version = epoch, ticket = ++searchTicket, previous = more || replay ? state.searchResults : emptySearch();
      set({ query: more || replay ? state.query : text, searchResults: { ...previous, resultQuery: text, loading: true, error: null } });
      // A submitted query during staging is intent for the next snapshot;
      // the old backend session is already revoked. Publication replays it.
      if (!sameScope(preferences.getState().scope, session.context.scope)) { if (!get().loading) set({ searchResults: { ...get().searchResults, loading: false, retained: true } }); return; }
      const result = await api('search_history', { context: session.context, sessionId: session.sessionId, query: text, cursor: more ? previous.nextCursor : null, pageSize: null }).catch(() => ({ ok: false as const, error: transportError() }));
      if (!current(version, session.context) || ticket !== searchTicket) return;
      if (!result.ok) { set({ searchResults: { ...get().searchResults, loading: false, error: result.error } }); return; }
      if (!sameContext(result.value.context, session.context) || result.value.sessionId !== session.sessionId || result.value.query !== text) { set({ searchResults: { ...get().searchResults, loading: false, error: displayError() } }); return; }
      const hits = new Map((more ? previous.hits : []).map(hit => [hit.commit.oid, hit]));
      for (const hit of result.value.commits) hits.set(hit.commit.oid, hit);
      set({ searchResults: { resultQuery: text, hits: [...hits.values()], refs: more ? previous.refs : result.value.refs, nextCursor: result.value.nextCursor, loading: false, retained: false, error: null } });
    };
    return { preferences, inspector, restoringSelection: false, scope: null, session: null, layout: createHistoryLayout(), filter: emptyFilter(), anchor: null, loading: false, refreshing: false, pageLoading: false, error: null, pageError: null, nextCursor: null,
      selected: null, selectionLoading: false, selectionError: null, query: '', searchResults: emptySearch(),
      activate: async (selection, hint) => {
        remember(); submittedSearch = null;
        inspector.getState().clear();
        const ticket = ++epoch, scope = { ...selection, generation: (++generation).toString() };
        set({ scope, session: null, layout: createHistoryLayout(), selected: null, loading: true, error: null, anchor: null, restoringSelection: false });
        const durable = await preferences.getState().activate(scope);
        if (ticket !== epoch) return;
        if (!durable) { set({ loading: false }); return; }
        const saved: HistoryViewHint = { filter: durable.filter, anchor: durable.anchorOid ? { oid: durable.anchorOid, offset: durable.anchorOffset } : null, selectedOid: durable.selectedOid };
        const restored = { ...saved, ...hint, filter: canonical(hint?.filter ?? saved.filter) };
        hints.set(selection.repoId, restored);
        desiredSelection = restored.selectedOid ? { repoId: selection.repoId, oid: restored.selectedOid } : null;
        return open(selection, restored.filter, false, false, scope);
      },
      deactivate: () => { remember(); submittedSearch = null; preferences.getState().deactivate(); inspector.getState().clear(); epoch++; searchTicket++; selectionTicket++; desiredSelection = null; set({ scope: null, session: null, layout: createHistoryLayout(), selected: null, loading: false, refreshing: false, pageLoading: false, nextCursor: null, error: null, pageError: null, selectionLoading: false, selectionError: null, query: '', searchResults: emptySearch(), anchor: null, restoringSelection: false }); },
      refresh: () => {
        const state = get(); if (!state.scope) return Promise.resolve();
        if (!preferences.getState().hydrated) return get().activate(state.scope);
        if (opening?.ticket === epoch) return opening.promise;
        return open(state.scope, state.filter, true, true);
      },
      setFilter: filter => { const state = get(); if (!state.scope || !state.session) return Promise.resolve(); preferences.getState().patch({ filter: canonical(filter) }); return open(state.scope, canonical(filter), false, true); },
      setAnchor: anchor => { set({ anchor: { ...anchor } }); remember(); preferences.getState().patch({ anchorOid: anchor.oid, anchorOffset: anchor.offset }); },
      loadMore: () => {
        const state = get(), session = state.session, cursor = state.nextCursor, version = epoch;
        if (paging?.ticket === version) return paging.promise;
        if (!session || !cursor || state.loading || !sameScope(state.scope, session.context.scope)) return Promise.resolve();
        set({ pageLoading: true, pageError: null });
        const promise = (async () => {
          const result = await api('load_history_page', { context: session.context, sessionId: session.sessionId, cursor, pageSize: null }).catch(() => ({ ok: false as const, error: transportError() }));
          if (!current(version, session.context)) return;
          if (!result.ok) { set({ pageLoading: false, pageError: result.error }); return; }
          try {
            if (!sameContext(result.value.context, session.context) || result.value.sessionId !== session.sessionId || result.value.nextCursor === cursor) throw Error('Mismatched or repeated page');
            set({ layout: appendHistoryLayout(get().layout, result.value.commits), nextCursor: result.value.nextCursor, pageLoading: false });
          } catch { set({ pageLoading: false, pageError: displayError() }); }
        })();
        paging = { ticket: version, promise };
        void promise.finally(() => { if (paging?.promise === promise) paging = null; });
        return promise;
      },
      setQuery: query => { searchTicket++; set({ query, searchResults: { ...get().searchResults, loading: false } }); },
      search: text => query(false, text), searchMore: () => query(true),
      selectCommit: oid => { const node = get().layout.nodesByOid.get(oid); if (node) void choose(node.commit as CommitSummary, false); },
      selectSearchHit: hit => { if (get().searchResults.loading || get().searchResults.retained) return; const currentHit = get().searchResults.hits.find(value => value.commit.handle === hit.commit.handle && value.commit.oid === hit.commit.oid); if (currentHit) void choose(currentHit.commit, currentHit.outsideView); },
      selectRef: async ref => { const captured = get().session?.refs.find(value => value.name === ref.name && value.commitOid === ref.commitOid); if (captured?.commitOid) await resolveOid(captured.commitOid); },
    };
  });
}
export const historyStore = createHistoryStore();
