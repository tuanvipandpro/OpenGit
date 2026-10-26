import { useEffect, useRef, useState } from 'react';
import { useStore } from 'zustand';
import type { StoreApi } from 'zustand';
import type { ChangeEntry, ChangeSide } from '../../contracts';
import type { HistoryState } from '../../stores/history';
import type { WorkspaceState } from '../../stores/workspace';
import type { OperationsState } from '../../stores/operations';
import { repositoryChangesStore, sameChangeFence } from '../../stores/repositoryChanges';
import type { ChangeSelectionFence, RepositoryChangesState } from '../../stores/repositoryChanges';
import { sameActionScope } from '../../stores/repositorySync';
import type { RefreshInteractionLease } from '../repository/refresh';
import { ErrorNotice } from '../../components/ErrorNotice';
import { ChangeDiff } from './ChangeDiff';
import { CommitForm } from './CommitForm';
type Form = { kind: 'commit' } | { kind: 'file'; entryId: string; pathLabel: string; side: ChangeSide } | { kind: 'partial'; pathLabel: string; fence: ChangeSelectionFence };
const formTitle = (form: Form) => form.kind === 'commit' ? 'Commit staged changes' : form.kind === 'partial' ? form.fence.side === 'staged' ? 'Unstage selected' : 'Stage selected' : form.side === 'staged' ? 'Unstage file' : 'Stage file';
export function ChangesPanel({ history, workspace, tasks, store = repositoryChangesStore }: { history: StoreApi<HistoryState>; workspace: StoreApi<WorkspaceState>; tasks: StoreApi<OperationsState>; store?: StoreApi<RepositoryChangesState> }) {
  const state = useStore(store), owner = useRef<RefreshInteractionLease | null>(null), pending = useRef(false);
  const readAbort = useRef<AbortController | null>(null);
  const [inspection, setInspection] = useState<{ entryId: string; side: ChangeSide; pathLabel: string; waiting: boolean } | null>(null);
  const [form, setForm] = useState<Form | null>(null), [ready, setReady] = useState(false), [openingError, setOpeningError] = useState(''), [unconfirmed, setUnconfirmed] = useState(false);
  useEffect(() => store.getState().connect(history), [store, history]);
  const close = () => { const lease = owner.current; owner.current = null; readAbort.current?.abort(); readAbort.current = null; store.getState().clearDiff(); store.getState().clearPrepared(); setInspection(null); setForm(null); setReady(false); lease?.release(); };
  useEffect(() => () => { const lease = owner.current; owner.current = null; readAbort.current?.abort(); readAbort.current = null; store.getState().clearDiff(); lease?.release(); }, [store]);
  const interaction = () => {
    if (owner.current?.isCurrent()) return owner.current;
    const lease = workspace.getState().beginInteraction();
    if (!lease) { setOpeningError('Close the current action form before opening another.'); return null; }
    owner.current = lease;
    void lease.closed.then(() => { if (owner.current === lease) { owner.current = null; readAbort.current?.abort(); readAbort.current = null; setInspection(null); setForm(null); setReady(false); store.getState().clearDiff(); store.getState().clearPrepared(); } });
    return lease;
  };
  const inspect = (entryId: string, side: ChangeSide, pathLabel: string) => {
    if (form) return;
    const lease = interaction(); if (!lease) return;
    readAbort.current?.abort(); const abort = new AbortController(); readAbort.current = abort;
    store.getState().clearDiff(); setOpeningError(''); setInspection({ entryId, side, pathLabel, waiting: true });
    void (async () => {
      const ready = await lease.ready && !abort.signal.aborted && await store.getState().awaitCatalog(abort.signal);
      if (!ready || abort.signal.aborted || owner.current !== lease || !lease.isCurrent()) return;
      if (store.getState().error) { setInspection({ entryId, side, pathLabel, waiting: false }); return; }
      await store.getState().select(entryId, side);
      if (!abort.signal.aborted && owner.current === lease && lease.isCurrent()) setInspection({ entryId, side, pathLabel, waiting: false });
    })();
  };
  const open = (next: Form) => {
    if (form) return;
    const lease = interaction(); if (!lease) return;
    if (next.kind !== 'partial') { readAbort.current?.abort(); readAbort.current = null; setInspection(null); store.getState().clearDiff(); }
    setForm(next); setReady(false); setOpeningError(''); setUnconfirmed(false); store.getState().clearPrepared();
    void (async () => {
      if (!await lease.ready || !lease.isCurrent() || owner.current !== lease) return;
      // A partial selection belongs to the displayed diff. Opening its form
      // must not refresh, reselect or mint replacement source authority.
      if (next.kind !== 'partial') await store.getState().reload();
      if (owner.current === lease && lease.isCurrent()) setReady(true);
    })();
  };
  const staged = state.catalog?.entries.filter(e => e.unstageCapability !== 'unavailable') ?? [], unstaged = state.catalog?.entries.filter(e => e.stageCapability !== 'unavailable') ?? [];
  const isUnmerged = (entry: ChangeEntry) => entry.indexStatus === 'U' || entry.worktreeStatus === 'U' || entry.indexStatus === 'A' && entry.worktreeStatus === 'A' || entry.indexStatus === 'D' && entry.worktreeStatus === 'D';
  const hasUnmerged = !!state.catalog?.entries.some(isUnmerged);
  const selectedUnmerged = !!state.catalog?.entries.some(entry => entry.entryId === state.selectedChange?.entryId && isUnmerged(entry));
  const hasHead = !!state.catalog?.head.oid && state.catalog.head.kind !== 'unborn';
  const selectedPath = state.catalog?.entries.find(entry => entry.entryId === state.selectedChange?.entryId)?.pathDisplay;
  const selectedCount = state.selection ? state.selection.kind === 'hunks' ? state.selection.hunkIds.length : state.selection.lineIds.length : 0;
  const partialCurrent = form?.kind !== 'partial' || sameActionScope(state.scope, form.fence.scope) && sameChangeFence(state.diff, form.fence) && !!selectedCount;
  const list = (entries: ChangeEntry[], side: ChangeSide) => <section aria-label={side === 'staged' ? 'Staged changes' : 'Unstaged changes'}><h3>{side === 'staged' ? 'Staged' : 'Unstaged'} <span>{entries.length}</span></h3>{entries.length ? <ul>{entries.map(e => <li key={e.entryId}><button className="change-path" type="button" disabled={!!form || state.loading || state.executing} onClick={() => inspect(e.entryId, side, e.pathDisplay)}>{e.previousPathDisplay && <span>{e.previousPathDisplay} → </span>}{e.pathDisplay}</button><span className="muted">{side === 'staged' ? e.indexStatus : e.worktreeStatus}</span><button type="button" disabled={!!form || state.loading || state.catalog?.unsupported || (side === 'staged' ? e.unstageCapability : e.stageCapability) !== 'eligible'} onClick={() => open({ kind: 'file', entryId: e.entryId, pathLabel: e.pathDisplay, side })}>{side === 'staged' ? 'Unstage file' : 'Stage file'}</button></li>)}</ul> : <p className="muted">No {side} changes.</p>}</section>;
  return <div className="changes-workspace"><div className="changes-header"><div><p className="eyebrow">WORKING DIRECTORY</p><h1>Changes</h1></div><button type="button" disabled={!!form || state.loading || state.catalog?.unsupported || hasUnmerged || !staged.length && !hasHead} onClick={() => open({ kind: 'commit' })}>Commit staged changes</button></div>
    {openingError && <p role="alert">{openingError}</p>}{state.error && <ErrorNotice error={state.error} onRetry={() => { void state.reload(); }} />}
    {state.catalog?.unsupported && <p role="alert">Changes are unavailable for a bare repository, unfinished Git operation, submodule, split index or sparse index. Inspect the repository in Git.</p>}
    {inspection && !form && <div className="change-inspection-controls"><p>{inspection.side === 'staged' ? 'Staged' : 'Unstaged'} · {inspection.pathLabel}{inspection.waiting && !state.diffLoading && ' · Waiting for current changes…'}</p><button type="button" onClick={close}>Close diff</button>{!inspection.waiting && !state.diff && !state.diffLoading && state.stale && <p role="status">Repository refreshed. Select the file again explicitly to inspect current changes.</p>}</div>}
    {hasUnmerged && <p role="status">Unmerged paths remain. Resolve files externally, then use Stage file for each resolved path. Partial selection and Commit stay unavailable until all conflicts are staged.</p>}
    {state.loading && <p role="status">Loading changes…</p>}{list(unstaged, 'unstaged')}{list(staged, 'staged')}<ChangeDiff value={state.diff} loading={state.diffLoading} selection={state.selection} pathLabel={selectedPath} disabled={!!form || state.loading || state.executing || selectedUnmerged} onSelection={state.setSelection} onApplySelection={fence => open({ kind: 'partial', pathLabel: selectedPath ?? 'Selected file', fence: { scope: { ...fence.scope }, changeHandle: fence.changeHandle, diffId: fence.diffId, side: fence.side } })} />
    {form && <div className="modal-backdrop"><section className="dialog" role="dialog" aria-modal="true" aria-label={formTitle(form)}><h2>{formTitle(form)}</h2>{!ready ? <p role="status">Loading current context…</p> : <>
      {form.kind === 'commit' ? <CommitForm message={state.draft} amend={state.amend} canAmend={hasHead} disabled={state.executing} stagedCount={staged.length} onChange={state.setDraft} onAmendChange={state.setAmend} /> : <p>{form.pathLabel}<br />This action changes only the staged index. Working files remain in place.{form.kind === 'partial' && partialCurrent && <><br />{selectedCount} {state.selection?.kind === 'hunks' ? 'hunks' : 'edited lines'} selected from the displayed {form.fence.side} diff.</>}</p>}
      {!partialCurrent && <p role="alert">The selected diff changed. Close this form, select the file again, and choose changes explicitly.</p>}
      {unconfirmed && <p role="alert">Execution reply was not confirmed. Check Tasks before preparing another action. OpenGit will not retry it automatically.</p>}{state.error && <ErrorNotice error={state.error} />}{state.stale && <p role="status">Repository refreshed. Prepare again.</p>}{state.actionError && <ErrorNotice error={state.actionError} />}
      {state.prepared && <p>Prepared {state.prepared.kind}. Expires {state.prepared.expiresAt}.</p>}
      <div className="dialog-actions"><button type="button" disabled={state.executing || state.preparing || state.loading || !state.catalog || state.catalog.unsupported || !!state.error || !partialCurrent || form.kind === 'commit' && (hasUnmerged || !state.draft.trim() || !state.amend && !staged.length)} onClick={() => { void (form.kind === 'commit' ? state.prepareCommit() : form.kind === 'partial' ? state.prepareSelection(form.fence) : state.prepareFile(form.entryId, form.side)); }}>{state.preparing ? 'Preparing…' : 'Prepare'}</button><button type="button" disabled={state.executing || !state.prepared || !partialCurrent} onClick={() => { if (pending.current) return; pending.current = true; void (async () => { const lease = owner.current; try { const result = await store.getState().execute(); if (result && !result.ok && owner.current === lease) setUnconfirmed(true); if (result?.ok && lease && owner.current === lease) close(); await tasks.getState().sync(); } finally { pending.current = false; } })(); }}>{state.executing ? 'Starting…' : 'Execute'}</button><button type="button" disabled={state.executing} onClick={() => { void workspace.getState().refresh(); }}>Refresh context</button></div>
    </>}<button type="button" disabled={state.executing} onClick={close}>Close</button></section></div>}
  </div>;
}
