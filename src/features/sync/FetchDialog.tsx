import { useEffect, useRef, useState } from 'react';
import { useStore } from 'zustand';
import type { StoreApi } from 'zustand';
import { Modal } from '../../components/Modal';
import { ErrorNotice } from '../../components/ErrorNotice';
import { sameActionScope } from '../../stores/repositorySync';
import type { RepositorySyncState } from '../../stores/repositorySync';

const reasons = { unsupportedTransport: 'Transport is not supported for fetch.', unsupportedRefspec: 'Refspec mapping is not supported for safe fetch.', mirror: 'Mirror remotes are not supported.', noRefspec: 'No fetch refspec is configured.', eligibleProbeRequired: 'Git capability will be checked during preparation.' };
export function FetchDialog({ store, onDismiss, onStarted, onRefresh }: { store: StoreApi<RepositorySyncState>; onDismiss(): void; onStarted(): Promise<void>; onRefresh?(): void }) {
  const state = useStore(store), catalog = state.catalog;
  const [names, setNames] = useState<string[]>([]), [prune, setPrune] = useState(false);
  const [notice, setNotice] = useState<string | null>(null), [inputError, setInputError] = useState<string | null>(null);
  const [executeNotice, setExecuteNotice] = useState<string | null>(null);
  const pending = useRef(false), preview = state.prepared;
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const context = JSON.stringify([state.scope, catalog?.snapshotId]), previousContext = useRef(context);
  useEffect(() => {
    if (previousContext.current === context) return;
    previousContext.current = context; store.getState().clearPrepared(); setNotice('Repository refreshed. Prepare again.');
  }, [context, store]);
  const available = !!catalog && sameActionScope(catalog.scope, state.scope) && !state.catalogLoading && !state.catalogError;
  const remotes = catalog?.remotes ?? [], eligible = remotes.filter(remote => remote.fetch === 'eligibleProbeRequired' && remote.handle);
  const missing = names.filter(name => !eligible.some(remote => remote.name === name));
  const busy = state.preparing || state.executing;
  const change = (next: string[]) => { store.getState().clearPrepared(); setNames(next); setInputError(null); };
  const prepare = () => {
    const current = store.getState(), choices = current.catalog?.remotes ?? [];
    const handles = names.map(name => choices.find(remote => remote.name === name && remote.fetch === 'eligibleProbeRequired')?.handle);
    if (handles.some(handle => !handle)) { setInputError('A selected remote is missing or no longer supports fetch. Remove it or refresh the context.'); return; }
    setInputError(null); setNotice(null); void current.prepareFetch(handles as string[], prune);
  };
  const dismiss = () => { if (!pending.current) { store.getState().clearPrepared(); onDismiss(); } };
  const execute = async () => {
    if (pending.current) return; pending.current = true;
    try {
      const result = await store.getState().executeFetch();
      pending.current = false;
      if (result?.ok && mounted.current) onDismiss();
      else if (result && !result.ok && mounted.current) setExecuteNotice('The fetch reply was not confirmed. Check Tasks before preparing another action.');
      // Always synchronize Tasks after execution: a lost reply is not proof
      // that enqueue failed. No automatic execute retry.
      await onStarted();
    } finally { pending.current = false; }
  };
  return <Modal title="Fetch remotes" className="fetch-dialog" onDismiss={dismiss}>
    <div className="modal-content">
      <p>Update local remote-tracking branches from the selected configured remotes. This does not merge commits into your branch.</p>
      {notice && <p role="status">{notice}</p>}
      {!available && <p role="status">Waiting for current repository context. Refresh the context if it remains unavailable.</p>}
      {state.catalogError && <ErrorNotice error={state.catalogError} />}
      <fieldset disabled={busy || !available}><legend>Remotes</legend>
        <button type="button" disabled={!remotes.length || eligible.length !== remotes.length} onClick={() => change(eligible.map(remote => remote.name))}>Select all remotes</button>
        {remotes.map(remote => <label key={remote.name} className="fetch-remote"><input type="checkbox" aria-label={`Fetch ${remote.name}`} disabled={!remote.handle || remote.fetch !== 'eligibleProbeRequired'} checked={names.includes(remote.name)}
          onChange={event => change(event.target.checked ? [...names, remote.name] : names.filter(name => name !== remote.name))} />{remote.name}<small>{reasons[remote.fetch]}</small></label>)}
        {missing.map(name => <p key={name} role="alert">Selected remote {name} is missing or unavailable. <button type="button" onClick={() => change(names.filter(value => value !== name))}>Remove {name} selection</button></p>)}
        <label className="fetch-prune"><input type="checkbox" checked={prune} onChange={event => { store.getState().clearPrepared(); setPrune(event.target.checked); }} />Prune deleted remote-tracking branches</label>
        <p>Prune is opt-in. Local branches and tags are not selected for deletion.</p>
      </fieldset>
      {preview && <section className="fetch-preview" aria-label="Fetch preview"><strong>Ready to fetch {names.length} remote{names.length === 1 ? '' : 's'}</strong>
        <p>{names.join(', ')} · Prune {prune ? 'enabled' : 'off'}</p>
        <p>Preview expires: {preview.expiresAt}</p>{preview.warnings.map(warning => <p key={warning}>{warning.replace(/([a-z])([A-Z])/g, '$1 $2')}</p>)}</section>}
      {state.preparing && <p role="status">Checking fetch capability and repository state…</p>}
      {state.executing && <p role="status">Starting fetch. Follow its results in Tasks…</p>}
      {state.actionError && <ErrorNotice error={state.actionError} />}
      {inputError && <p role="alert">{inputError}</p>}{executeNotice && <p role="status">{executeNotice}</p>}
    </div>
    <footer className="modal-actions"><button type="button" className="button" onClick={dismiss} disabled={state.executing}>Cancel</button>
      {onRefresh && <button type="button" className="button" disabled={state.executing} onClick={onRefresh}>Refresh context</button>}
      {preview ? <button type="button" className="button primary" disabled={busy || !available} onClick={() => { void execute(); }}>Start fetch</button>
        : <button type="button" className="button primary" disabled={busy || !available || !names.length || !!missing.length} onClick={prepare}>Prepare fetch</button>}
    </footer>
  </Modal>;
}
