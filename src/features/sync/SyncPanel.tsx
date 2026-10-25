import { useStore } from 'zustand';
import type { StoreApi } from 'zustand';
import type { RepositorySyncState } from '../../stores/repositorySync';
import { sameActionScope } from '../../stores/repositorySync';
import { ErrorNotice } from '../../components/ErrorNotice';

export function SyncPanel({ store, onFetch, onRefresh, opening = false, onCancelOpening }: { store: StoreApi<RepositorySyncState>; onFetch(): void; onRefresh(): void; opening?: boolean; onCancelOpening?(): void }) {
  const state = useStore(store), catalog = state.catalog;
  const current = !!catalog && sameActionScope(catalog.scope, state.scope) && !state.catalogLoading && !state.catalogError;
  return <section className="repository-sync" aria-label="Repository sync">
    <div className="repository-sync-summary">
      <strong>Sync</strong>
      {catalog ? <><span>Current branch: {catalog.sync.currentBranch ?? (catalog.head.kind === 'detached' ? 'Detached HEAD' : 'None')}</span>
        <span>Upstream: {catalog.sync.upstream ?? 'Not configured'}</span>
        <span>Ahead: {catalog.sync.ahead ?? 'unknown'} · Behind: {catalog.sync.behind ?? 'unknown'}</span></> : <span>Repository context unavailable</span>}
      <button type="button" onClick={onFetch} disabled={opening || !current || !catalog?.remotes.length}>Fetch…</button>
      <button type="button" onClick={onRefresh}>Refresh sync</button>
    </div>
    {opening && <p role="status">Waiting for repository refresh before opening Fetch… <button type="button" onClick={onCancelOpening}>Cancel opening fetch</button></p>}
    {state.catalogLoading && <p role="status">Refreshing local branch and remote information…</p>}
    {catalog && !current && <p role="status">Previously loaded repository context; actions are unavailable until refreshed.</p>}
    {catalog && <small>Local tracking snapshot · {catalog.observedAt} · ancestry {catalog.sync.completeness}. Fetch explicitly to check configured remotes.</small>}
    {state.catalogError && <ErrorNotice error={state.catalogError} onRetry={() => { void state.reload(); }} />}
  </section>;
}
