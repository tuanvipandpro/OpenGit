import { useStore } from 'zustand';
import type { StoreApi } from 'zustand';
import type { Selection } from '../../contracts';
import type { HistoryState } from '../../stores/history';
import { ErrorNotice } from '../../components/ErrorNotice';
import { CommitRows } from './CommitRows';
import { ReferencesPanel } from './ReferencesPanel';
import { applyHistoryFilterAction } from './visibility';
import { HistorySearch } from './HistorySearch';
import { CommitInspector } from './CommitInspector';

export type HistoryScreenProps = { store: StoreApi<HistoryState>; selection: Selection | null };
export function HistoryScreen({ store, selection }: HistoryScreenProps) {
  const state = useStore(store);
  const preferences = useStore(state.preferences);
  if (!selection) return <div className="history-start-state" role="status">Select a repository to view history.</div>;
  if (state.scope && (state.scope.repoId !== selection.repoId || state.scope.worktreeId !== selection.worktreeId)) return <div className="history-start-state" role="status">Loading history…</div>;
  return <section className="history-screen" aria-label="Repository history">
    {preferences.error && <ErrorNotice error={preferences.error} onRetry={() => { void (preferences.hydrated ? preferences.flush() : state.refresh()); }} />}
    {state.error && <ErrorNotice error={state.error} onRetry={() => { void state.refresh(); }} />}
    {!state.session ? state.loading ? <p role="status">{preferences.loading ? 'Restoring repository view…' : 'Loading history…'}</p> : <p>History is unavailable. Refresh to try again.</p> : <div className="history-panes">
      <div className="history-main">
        <ReferencesPanel key={JSON.stringify(selection)} refs={state.session.refs} head={state.session.head} filter={state.filter} loading={state.loading}
          onFilterChange={filter => { const current = store.getState(); if (!current.loading && current.session === state.session) void current.setFilter(filter); }}
          onSelectRef={ref => { const current = store.getState(); if (!current.loading && current.session === state.session && current.session?.refs.some(item => item.name === ref.name && item.commitOid === ref.commitOid)) void current.selectRef(ref); }} />
        {(state.filter.hiddenRefs.length > 0 || state.filter.soloRefs.length > 0) && <div className="history-active-filters" role="status">{state.filter.hiddenRefs.length} hidden · {state.filter.soloRefs.length} solo <button type="button" onClick={() => { void state.setFilter({ hiddenRefs: [], soloRefs: [] }); }}>Clear history filters</button></div>}
        <HistorySearch query={state.query} resultQuery={state.searchResults.resultQuery} hits={state.searchResults.hits} refs={state.searchResults.refs}
          loading={state.searchResults.loading} retained={state.searchResults.retained} error={state.searchResults.error?.message} hasMore={!!state.searchResults.nextCursor}
          onQueryChange={state.setQuery} onSearch={text => { void state.search(text); }} onLoadMore={() => { void state.searchMore(); }}
          onSelectCommit={state.selectSearchHit} onSelectRef={ref => { void state.selectRef(ref); }} />
        <div className="history-graph-container"><CommitRows key={`${state.session.context.scope.generation}:${state.session.context.snapshotId}`}
          actionContext={JSON.stringify([state.session.context, state.session.sessionId])}
          actionsLoading={state.loading} onSelectRef={ref => { const current = store.getState(); if (!current.loading && current.session === state.session) void current.selectRef(ref); }}
          onFilterAction={action => { const current = store.getState(); if (!current.loading && current.session === state.session) void current.setFilter(applyHistoryFilterAction(current.filter, action)); }}
          layout={state.layout} refs={state.session.refs} head={state.session.head} filter={state.filter}
          selectedOid={state.selected?.outsideView ? null : state.selected?.commit.oid ?? null} onSelect={state.selectCommit}
          suppressSelectionScroll={state.restoringSelection}
          loading={state.pageLoading || state.loading && !state.refreshing} hasMore={!!state.nextCursor} error={state.pageError?.message}
          onLoadMore={() => { void (state.pageError?.code === 'ExternalChange' || state.pageError?.code === 'InvalidHandle' ? state.refresh() : state.loadMore()); }} initialAnchor={state.anchor} onAnchorChange={state.setAnchor} /></div>
        <div className="history-inspector-pane" style={{ height: `min(${preferences.view.detailsHeight}px, 48%)` }}>
          <label className="history-size-control">Details height<input aria-label="Details height" type="range" min="160" max="1200" value={preferences.view.detailsHeight} onChange={event => preferences.patch({ detailsHeight: Number(event.target.value) })} /></label>
          {state.selectionLoading && <p role="status">Selecting commit…</p>}
          {state.selectionError && <ErrorNotice error={state.selectionError} onRetry={() => { void state.refresh(); }} />}
          <CommitInspector store={state.inspector} selected={state.selected} layout={preferences.view.diffLayout} onLayoutChange={diffLayout => preferences.patch({ diffLayout })} />
        </div>
      </div>
    </div>}
  </section>;
}
