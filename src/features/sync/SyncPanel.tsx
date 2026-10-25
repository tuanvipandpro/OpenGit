import { useStore } from 'zustand';
import type { StoreApi } from 'zustand';
import type { RepositorySyncState } from '../../stores/repositorySync';
import { sameActionScope } from '../../stores/repositorySync';
import { ErrorNotice } from '../../components/ErrorNotice';
import type { PublicationEvidence } from '../../contracts';

export function SyncPanel({ store, onFetch, onRefresh, opening = false, onCancelOpening, onPush, onPull, publication }: { store: StoreApi<RepositorySyncState>; onFetch(): void; onRefresh(): void; opening?: boolean; onCancelOpening?(): void; onPush?():void; onPull?():void; publication?:PublicationEvidence }) {
  const state = useStore(store), catalog = state.catalog;
  const current = !!catalog && sameActionScope(catalog.scope, state.scope) && !state.catalogLoading && !state.catalogError;
  return <section className="repository-sync" aria-label="Repository sync">
    <div className="repository-sync-summary">
      <strong>Sync</strong>
      {catalog ? <><span>Current branch: {catalog.sync.currentBranch ?? (catalog.head.kind === 'detached' ? 'Detached HEAD' : 'None')}</span>
        <span>Upstream: {catalog.sync.upstream ?? 'Not configured'}</span>
        <span>Local tracking · Ahead: {catalog.sync.ahead ?? 'unknown'} · Behind: {catalog.sync.behind ?? 'unknown'}</span></> : <span>Repository context unavailable</span>}
      <button type="button" onClick={onFetch} disabled={opening || !current || !catalog?.remotes.length}>Fetch…</button>
      <button type="button" onClick={onRefresh}>Refresh sync</button>
      {onPush && <button type="button" onClick={onPush} disabled={!current || !catalog?.remotes.length}>Push…</button>}
      {onPull && <button type="button" onClick={onPull} disabled={!current || !catalog?.sync.upstream}>Pull (FF only)…</button>}
    </div>
    {opening && <p role="status">Waiting for repository refresh before opening Fetch… <button type="button" onClick={onCancelOpening}>Cancel opening fetch</button></p>}
    {state.catalogLoading && <p role="status">Refreshing local branch and remote information…</p>}
    {catalog && !current && <p role="status">Previously loaded repository context; actions are unavailable until refreshed.</p>}
    {catalog && <small>Local tracking snapshot · {catalog.observedAt} · ancestry {catalog.sync.completeness}. Fetch explicitly to check configured remotes.</small>}
    {current && publication?.role === 'push' && !publication.deleted && publication.sourceRef === `refs/heads/${catalog?.head.branch}` && publication.sourceOid === catalog?.head.oid && <p className="publication-evidence">Dated push ACK: {publication.remoteName} · {publication.destination} · {publication.acknowledgedOid} · {publication.acknowledgedAt}. Local tracking counts require explicit Fetch; this is not a remote-current guarantee.</p>}
    {state.catalogError && <ErrorNotice error={state.catalogError} onRetry={() => { void state.reload(); }} />}
  </section>;
}
