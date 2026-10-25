import { useEffect, useRef, useState } from 'react';
import { useStore } from 'zustand';
import type { StoreApi } from 'zustand';
import type { ChangeEntry, ChangeSide } from '../../contracts';
import type { HistoryState } from '../../stores/history';
import type { WorkspaceState } from '../../stores/workspace';
import type { OperationsState } from '../../stores/operations';
import { repositoryChangesStore } from '../../stores/repositoryChanges';
import type { RepositoryChangesState } from '../../stores/repositoryChanges';
import type { RefreshInteractionLease } from '../repository/refresh';
import { ErrorNotice } from '../../components/ErrorNotice';
import { ChangeDiff } from './ChangeDiff';
import { CommitForm } from './CommitForm';
type Form = { kind: 'commit' } | { kind: 'file'; entryId: string; pathLabel: string; side: ChangeSide };
export function ChangesPanel({ history, workspace, tasks, store = repositoryChangesStore }: { history: StoreApi<HistoryState>; workspace: StoreApi<WorkspaceState>; tasks: StoreApi<OperationsState>; store?: StoreApi<RepositoryChangesState> }) {
  const state = useStore(store), owner = useRef<RefreshInteractionLease | null>(null);
  const [form, setForm] = useState<Form | null>(null), [ready, setReady] = useState(false), [openingError, setOpeningError] = useState(''), [unconfirmed, setUnconfirmed] = useState(false);
  useEffect(() => store.getState().connect(history), [store, history]);
  const close = () => { const lease = owner.current; owner.current = null; store.getState().clearPrepared(); setForm(null); setReady(false); lease?.release(); };
  useEffect(() => () => { owner.current?.release(); owner.current = null; }, []);
  const open = (next: Form) => {
    if (owner.current) return;
    const lease = workspace.getState().beginInteraction(); if (!lease) { setOpeningError('Close the current action form before opening another.'); return; }
    owner.current = lease; setForm(next); setReady(false); setOpeningError(''); setUnconfirmed(false); store.getState().clearPrepared();
    void lease.closed.then(() => { if (owner.current === lease) { owner.current = null; setForm(null); setReady(false); store.getState().clearPrepared(); } });
    void (async () => { if (!await lease.ready || !lease.isCurrent()) return; await store.getState().reload(); if (owner.current === lease && lease.isCurrent()) setReady(true); })();
  };
  const staged = state.catalog?.entries.filter(e => e.unstageCapability !== 'unavailable') ?? [], unstaged = state.catalog?.entries.filter(e => e.stageCapability !== 'unavailable') ?? [];
  const list = (entries: ChangeEntry[], side: ChangeSide) => <section aria-label={side === 'staged' ? 'Staged changes' : 'Unstaged changes'}><h3>{side === 'staged' ? 'Staged' : 'Unstaged'} <span>{entries.length}</span></h3>{entries.length ? <ul>{entries.map(e => <li key={e.entryId}><button className="change-path" type="button" onClick={() => { void state.select(e.entryId, side); }}>{e.previousPathDisplay && <span>{e.previousPathDisplay} → </span>}{e.pathDisplay}</button><span className="muted">{side === 'staged' ? e.indexStatus : e.worktreeStatus}</span><button type="button" disabled={!!form || state.loading || state.catalog?.unsupported || (side === 'staged' ? e.unstageCapability : e.stageCapability) !== 'eligible'} onClick={() => open({ kind: 'file', entryId: e.entryId, pathLabel: e.pathDisplay, side })}>{side === 'staged' ? 'Unstage file' : 'Stage file'}</button></li>)}</ul> : <p className="muted">No {side} changes.</p>}</section>;
  return <div className="changes-workspace"><div className="changes-header"><div><p className="eyebrow">WORKING DIRECTORY</p><h1>Changes</h1></div><button type="button" disabled={!!form || state.loading || state.catalog?.unsupported || !staged.length} onClick={() => open({ kind: 'commit' })}>Commit staged changes</button></div>
    {openingError && <p role="alert">{openingError}</p>}{state.error && <ErrorNotice error={state.error} onRetry={() => { void state.reload(); }} />}
    {state.catalog?.unsupported && <p role="alert">Changes are unavailable for a bare repository, unfinished Git operation, unmerged index, submodule, split index or sparse index. Inspect the repository in Git.</p>}
    {state.loading && <p role="status">Loading changes…</p>}{list(unstaged, 'unstaged')}{list(staged, 'staged')}<ChangeDiff value={state.diff} loading={state.diffLoading} />
    {form && <div className="modal-backdrop"><section className="dialog" role="dialog" aria-modal="true" aria-label={form.kind === 'commit' ? 'Commit staged changes' : form.side === 'staged' ? 'Unstage file' : 'Stage file'}><h2>{form.kind === 'commit' ? 'Commit staged changes' : form.side === 'staged' ? 'Unstage file' : 'Stage file'}</h2>{!ready ? <p role="status">Loading current context…</p> : <>
      {form.kind === 'commit' ? <CommitForm message={state.draft} disabled={state.executing} stagedCount={staged.length} onChange={state.setDraft} /> : <p>{form.pathLabel}<br />This action changes only the staged index. Working files remain in place.</p>}
      {unconfirmed && <p role="alert">Execution reply was not confirmed. Check Tasks before preparing another action. OpenGit will not retry it automatically.</p>}{state.error && <ErrorNotice error={state.error} />}{state.stale && <p role="status">Repository refreshed. Prepare again.</p>}{state.actionError && <ErrorNotice error={state.actionError} />}
      {state.prepared && <p>Prepared {state.prepared.kind}. Expires {state.prepared.expiresAt}.</p>}
      <div className="dialog-actions"><button type="button" disabled={state.executing || state.preparing || state.loading || !state.catalog || state.catalog.unsupported || !!state.error} onClick={() => { void (form.kind === 'commit' ? state.prepareCommit() : state.prepareFile(form.entryId, form.side)); }}>{state.preparing ? 'Preparing…' : 'Prepare'}</button><button type="button" disabled={state.executing || !state.prepared} onClick={() => { void (async () => { const lease = owner.current; const result = await store.getState().execute(); await tasks.getState().sync(); if (result && !result.ok && owner.current === lease) setUnconfirmed(true); if (result?.ok && lease && owner.current === lease) close(); })(); }}>{state.executing ? 'Starting…' : 'Execute'}</button><button type="button" disabled={state.executing} onClick={() => { void workspace.getState().refresh(); }}>Refresh context</button></div>
    </>}<button type="button" disabled={state.executing} onClick={close}>Close</button></section></div>}
  </div>;
}
