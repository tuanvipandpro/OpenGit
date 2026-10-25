import { useEffect, useRef, useState } from 'react';
import { useStore } from 'zustand';
import type { StoreApi } from 'zustand';
import { Modal } from '../../components/Modal';
import { ErrorNotice } from '../../components/ErrorNotice';
import { sameActionScope } from '../../stores/repositorySync';
import { localActionTitles } from '../../stores/repositoryActions';
import type { ActionLaunch, LocalActionDraft, LocalActionKind, RepositoryActionsState } from '../../stores/repositoryActions';

export function RepositoryActionDialog({ store, launch, onDismiss, onStarted, onRefresh }: {
  store: StoreApi<RepositoryActionsState>; launch: ActionLaunch; onDismiss(): void; onStarted(): Promise<void>; onRefresh(): void;
}) {
  const state = useStore(store), catalog = state.catalog;
  const [draft, setDraft] = useState<LocalActionDraft>(() => ({ kind: launch.kind ?? 'branchCreate', source: launch.refName ? { kind: 'ref', name: launch.refName } : launch.selectedCommit ? { kind: 'selectedCommit' } : { kind: 'head' }, name: '', checkout: false, upstream: null, annotated: false, message: '', sign: false }));
  const [notice, setNotice] = useState<string | null>(null), [confirmed, setConfirmed] = useState(false);
  const pending = useRef(false), mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; store.getState().clearPrepared(); }; }, [store]);
  const context = JSON.stringify([state.scope, catalog?.catalogId]), previous = useRef(context);
  useEffect(() => { if (previous.current === context) return; previous.current = context; store.getState().clearPrepared(); setConfirmed(false); setNotice('Repository refreshed. Prepare again.'); }, [context, store]);
  const change = (patch: Partial<LocalActionDraft>) => { store.getState().clearPrepared(); setConfirmed(false); setNotice(null); setDraft(value => ({ ...value, ...patch })); };
  const branchTarget = ['branchRename', 'branchSwitch', 'branchDelete', 'branchTracking'].includes(draft.kind), tagTarget = draft.kind === 'tagDelete';
  const create = draft.kind === 'branchCreate' || draft.kind === 'tagCreate';
  const choices = (catalog?.refs ?? []).filter(ref => !ref.symbolicTarget && (branchTarget ? ref.kind === 'local' : tagTarget ? ref.kind === 'tag' : ref.canStart));
  const selectedName = draft.source.kind === 'ref' ? draft.source.name : null;
  const missing = selectedName !== null && !choices.some(ref => ref.name === selectedName);
  const upstreamMissing = draft.kind === 'branchTracking' && draft.upstream !== null && !catalog?.refs.some(ref => ref.name === draft.upstream && ref.kind === 'remote' && !ref.symbolicTarget);
  const available = !!catalog && sameActionScope(state.scope, catalog.scope) && !state.loading && !state.error;
  const busy = state.preparing || state.executing, preview = state.prepared;
  const destructive = draft.kind === 'branchDelete' || draft.kind === 'tagDelete';
  const sourceValue = draft.source.kind === 'head' ? 'head' : draft.source.kind === 'selectedCommit' ? 'selected' : `ref:${draft.source.name}`;
  const dismiss = () => { if (!pending.current) onDismiss(); };
  const execute = async () => {
    if (pending.current) return; pending.current = true;
    try {
      const result = await store.getState().execute();
      if (result?.ok && mounted.current) onDismiss();
      else if (result && !result.ok && mounted.current) setNotice('The action reply was not confirmed. Check Tasks before preparing another action.');
      await onStarted();
    } finally { pending.current = false; }
  };
  return <Modal title="Local repository actions" className="repository-action-dialog" onDismiss={dismiss}>
    <div className="modal-content">
      {notice && <p role="status">{notice}</p>}
      {!available && <p role="status">Waiting for current repository context.</p>}
      {state.error && <ErrorNotice error={state.error} />}
      <fieldset disabled={!available || busy}><legend>Action and target</legend>
        <label>Action<select value={draft.kind} onChange={event => change({ kind: event.target.value as LocalActionKind, source: { kind: 'head' } })}>{Object.entries(localActionTitles).map(([kind, title]) => <option key={kind} value={kind}>{title}</option>)}</select></label>
        <label>{create ? 'Start at' : 'Reference'}<select value={sourceValue} onChange={event => change({ source: event.target.value === 'head' ? { kind: 'head' } : event.target.value === 'selected' ? { kind: 'selectedCommit' } : { kind: 'ref', name: event.target.value.slice(4) } })}>
          {create ? <><option value="head">Current HEAD</option><option value="selected">Selected history commit</option></> : <option value="head">Choose a reference</option>}
          {choices.map(ref => <option key={ref.name} value={`ref:${ref.name}`}>{ref.name}{ref.checkedOutHere ? ' (current)' : ref.checkedOutElsewhere ? ' (another worktree)' : ''}</option>)}
          {missing && <option value={sourceValue}>{selectedName} (unavailable)</option>}
        </select></label>
        {missing && <p role="alert">The selected reference is missing or unavailable. Choose a current reference.</p>}
        {(create || draft.kind === 'branchRename') && <label>{draft.kind === 'tagCreate' ? 'Tag name' : 'Branch name'}<input value={draft.name} onChange={event => change({ name: event.target.value })} autoComplete="off" /></label>}
        {draft.kind === 'branchCreate' && <label><input type="checkbox" checked={draft.checkout} onChange={event => change({ checkout: event.target.checked })} />Check out the new branch</label>}
        {draft.kind === 'branchTracking' && <label>Upstream<select value={draft.upstream ?? ''} onChange={event => change({ upstream: event.target.value || null })}><option value="">Unset upstream</option>{catalog?.refs.filter(ref => ref.kind === 'remote' && !ref.symbolicTarget).map(ref => <option key={ref.name} value={ref.name}>{ref.name}</option>)}{upstreamMissing && <option value={draft.upstream!}>{draft.upstream} (unavailable)</option>}</select></label>}
        {upstreamMissing && <p role="alert">The selected upstream is missing. Choose a current remote-tracking reference.</p>}
        {draft.kind === 'tagCreate' && <><label><input type="checkbox" checked={draft.annotated} onChange={event => change({ annotated: event.target.checked })} />Annotated tag</label>{draft.annotated && <><label>Tag message<textarea value={draft.message} onChange={event => change({ message: event.target.value })} /></label><label><input type="checkbox" checked={draft.sign} onChange={event => change({ sign: event.target.checked })} />Sign annotated tag using Git configuration</label></>}</>}
      </fieldset>
      {catalog && <p>{catalog.bare ? 'Bare repository: checkout and worktree actions are unavailable.' : catalog.dirty ? 'Working tree has changes. Git will check whether this action is safe.' : 'Git will verify the current repository state before applying changes.'}</p>}
      {preview && state.preview && <section className="fetch-preview" aria-label="Local action preview"><strong>{localActionTitles[draft.kind]}</strong><p>Source: {state.preview.source}</p><p>Target: {state.preview.destination}</p>{draft.kind === 'tagCreate' && draft.annotated && <p>Annotation: {draft.message}</p>}<p>Preview expires: {preview.expiresAt}</p>{preview.warnings.map(warning => <p key={warning}>{warning.replace(/([a-z])([A-Z])/g, '$1 $2')}</p>)}{destructive && <label><input type="checkbox" checked={confirmed} onChange={event => setConfirmed(event.target.checked)} />Confirm local deletion of {state.preview.source}. No force deletion.</label>}</section>}
      {state.preparing && <p role="status">Checking references and repository state…</p>}
      {state.executing && <p role="status">Starting action. Follow the result in Tasks…</p>}
      {state.actionError && <ErrorNotice error={state.actionError} />}
    </div>
    <footer className="modal-actions"><button type="button" className="button" disabled={state.executing} onClick={dismiss}>Cancel</button><button type="button" className="button" disabled={state.executing} onClick={onRefresh}>Refresh context</button>
      {preview ? <button type="button" className="button primary" disabled={!available || busy || destructive && !confirmed} onClick={() => { void execute(); }}>Apply action</button> : <button type="button" className="button primary" disabled={!available || busy || missing || upstreamMissing || (branchTarget || tagTarget) && draft.source.kind !== 'ref' || (create || draft.kind === 'branchRename') && !draft.name.trim() || draft.kind === 'tagCreate' && draft.annotated && !draft.message.trim()} onClick={() => { setConfirmed(false); setNotice(null); void store.getState().prepare(draft); }}>Prepare action</button>}
    </footer>
  </Modal>;
}
