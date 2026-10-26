import { useEffect, useRef, useState } from 'react';
import { useStore } from 'zustand';
import type { StoreApi } from 'zustand';
import { Modal } from '../../components/Modal';
import { ErrorNotice } from '../../components/ErrorNotice';
import type { RepositoryStashState } from '../../stores/repositoryStash';
import { sameActionScope } from '../../stores/repositorySync';
import type { ActionScope } from '../../contracts';
type Mode = { kind: 'list' | 'save' } | { kind: 'apply'; entryId: string; catalogId: string; scope: ActionScope };
export function StashDialog({ store, onDismiss, onStarted, onRefresh }: { store: StoreApi<RepositoryStashState>; onDismiss(): void; onStarted(): Promise<void>; onRefresh(): void }) {
  const state = useStore(store), [mode, setMode] = useState<Mode>({ kind: 'list' }), [notice, setNotice] = useState<string | null>(null);
  const pending = useRef(false), mounted = useRef(true), context = JSON.stringify([state.scope, state.catalog?.catalogId]), previous = useRef(context);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; store.getState().clearPrepared(); }; }, [store]);
  useEffect(() => { if (previous.current === context) return; previous.current = context; store.getState().clearPrepared(); setNotice('Repository refreshed. Choose the current stash entry and prepare again explicitly.'); }, [context, store]);
  const available = !!state.catalog && sameActionScope(state.scope, state.catalog.scope) && !state.loading && !state.error;
  const entry = mode.kind === 'apply' ? state.catalog?.entries.find(entry => entry.entryId === mode.entryId) : null;
  const targetCurrent = mode.kind !== 'apply' || !!entry && sameActionScope(state.scope, mode.scope) && mode.catalogId === state.catalog?.catalogId && entry.applyAvailable;
  const busy = state.preparing || state.executing;
  const choose = (next: Mode) => { store.getState().clearPrepared(); setNotice(null); setMode(next); };
  const dismiss = () => { if (!pending.current) onDismiss(); };
  const execute = async () => {
    if (pending.current) return; pending.current = true;
    try { const result = await store.getState().execute(); if (result?.ok && mounted.current) onDismiss(); else if (result && !result.ok && mounted.current) setNotice('Execution reply was not confirmed. Check Tasks before preparing another action. No automatic retry.'); await onStarted(); }
    finally { pending.current = false; }
  };
  return <Modal title="Stash" className="stash-dialog" onDismiss={dismiss}><div className="modal-content">
    {!available && <p role="status">Loading current stash context…</p>}{notice && <p role="status">{notice}</p>}{state.error && <ErrorNotice error={state.error} />}
    {mode.kind === 'list' ? <><p>Save local changes or apply a saved stash. Apply keeps the stash entry.</p><button type="button" disabled={!available || busy || !state.catalog?.saveAvailable} onClick={() => choose({ kind: 'save' })}>Save stash</button>
      <ol className="stash-entries" aria-label="Saved stashes">{state.catalog?.entries.map(entry => <li key={entry.entryId}><strong>{`stash@{${entry.ordinal}}`}</strong> · <code>{entry.oid}</code><p>Base: {entry.baseOid ?? 'unknown'} · Untracked files: {entry.includesUntracked ? 'included' : 'not included'}</p><button type="button" disabled={!available || busy || !entry.applyAvailable} onClick={() => choose({ kind: 'apply', entryId: entry.entryId, catalogId: state.catalog!.catalogId, scope: { ...state.scope! } })}>Apply stash</button>{!entry.applyAvailable && <p>Apply is unavailable in the current repository state.</p>}</li>)}</ol>{available && !state.catalog?.entries.length && <p>No stash entries in this snapshot.</p>}{available && !state.catalog?.saveAvailable && <p>Saving a stash is unavailable in the current repository state.</p>}</> : <>
      <h3>{mode.kind === 'save' ? 'Save stash' : 'Apply stash'}</h3>
      {mode.kind === 'save' ? <><p>Save tracked changes. Git performs stash cleanup as part of Save; ignored files are never included.</p><label>Stash message<textarea aria-label="Stash message" value={state.message} disabled={busy} maxLength={8192} onChange={event => state.setMessage(event.target.value)} /></label><label><input type="checkbox" aria-label="Include untracked files" checked={state.includeUntracked} disabled={busy} onChange={event => state.setIncludeUntracked(event.target.checked)} />Include untracked files</label></> : <><p>Selected {entry ? `stash@{${entry.ordinal}} · ${entry.oid}` : 'stash entry unavailable'}.</p><p>Apply the pinned stash object to working files. The original staged partition is not restored. The stash is kept. A clean tracked baseline is required; untracked or ignored path collisions are refused.</p>{!targetCurrent && <p role="alert">The selected stash catalog changed or the entry is missing. Return to the list and choose a current entry explicitly.</p>}</>}
      {state.prepared && <section className="fetch-preview" aria-label="Stash preview"><strong>Prepared {state.prepared.kind}.</strong><p>{mode.kind === 'save' ? `Untracked files: ${state.includeUntracked ? 'included' : 'excluded'}` : `Pinned entry: ${entry?.oid ?? 'unavailable'} · stash retained`}</p><p>Expires: {state.prepared.expiresAt}</p>{state.prepared.warnings.map(warning => <p key={warning}>{warning.replace(/([a-z])([A-Z])/g, '$1 $2')}</p>)}</section>}
      <div className="dialog-actions"><button type="button" disabled={!available || !targetCurrent || busy || mode.kind === 'save' && !state.catalog?.saveAvailable} onClick={() => { void (mode.kind === 'save' ? state.prepareSave() : mode.kind === 'apply' ? state.prepareApply(mode.entryId) : undefined); }}>Prepare</button><button type="button" disabled={!available || !targetCurrent || busy || !state.prepared} onClick={() => { void execute(); }}>Execute</button><button type="button" disabled={busy} onClick={() => choose({ kind: 'list' })}>Back to stashes</button></div>
    </>}
    {state.preparing && <p role="status">Checking stash and repository state…</p>}{state.executing && <p role="status">Starting stash action. Inspect its results in Tasks…</p>}{state.actionError && <ErrorNotice error={state.actionError} />}
    <div className="dialog-actions"><button type="button" disabled={busy} onClick={onRefresh}>Refresh context</button><button type="button" disabled={state.executing} onClick={dismiss}>Close</button></div>
  </div></Modal>;
}
