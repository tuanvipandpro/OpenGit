import { useEffect, useRef, useState } from 'react';
import { useStore } from 'zustand';
import type { StoreApi } from 'zustand';
import { Modal } from '../../components/Modal';
import { ErrorNotice } from '../../components/ErrorNotice';
import type { RepositoryTransportState, TransportLaunch } from '../../stores/repositoryTransport';
import type { RepositoryActionsState } from '../../stores/repositoryActions';
import type { RepositorySyncState } from '../../stores/repositorySync';
import { sameActionScope } from '../../stores/repositorySync';
import type { RemoteMutationKind } from '../../contracts';
const endpointReasons = { eligible: 'eligible endpoint', unsupportedTransport: 'this transport is unsupported', multipleUrls: 'multiple push URLs are unsupported', mirror: 'mirror remotes are unsupported', unsupportedRefspec: 'the configured refspec is unsupported' };
type DestinationChoice = { kind: 'auto' | 'new' | 'none' } | { kind: 'existing'; name: string };
export function RemoteActionDialog({ store, refs, sync, launch, onDismiss, onStarted, onRefresh }: { store: StoreApi<RepositoryTransportState>; refs: StoreApi<RepositoryActionsState>; sync: StoreApi<RepositorySyncState>; launch: TransportLaunch; onDismiss(): void; onStarted(): Promise<void>; onRefresh(): void }) {
  const state = useStore(store), local = useStore(refs), currentSync = useStore(sync);
  const [kind, setKind] = useState<RemoteMutationKind>(launch.kind);
  const [sourceRef, setSourceRef] = useState(launch.refName ?? (local.catalog?.head.branch ? `refs/heads/${local.catalog.head.branch}` : ''));
  const [remoteName, setRemoteName] = useState(''), [destination, setDestination] = useState<DestinationChoice>({ kind: 'auto' }), [newName, setNewName] = useState((launch.refName ?? sourceRef).replace(/^refs\/(heads|tags)\//, ''));
  const [confirmation, setConfirmation] = useState<string | null>(null), [notice, setNotice] = useState<string | null>(null);
  const pending = useRef(false), mounted = useRef(true), context = JSON.stringify([state.scope, local.catalog?.catalogId]), previous = useRef(context);
  const previousObservation = useRef(state.snapshot);
  useEffect(() => { mounted.current = true; void store.getState().begin(); return () => { mounted.current = false; store.getState().end(); }; }, [store]);
  useEffect(() => { if (previous.current === context) return; previous.current = context; store.getState().clearPrepared(); setConfirmation(null); setNotice('Repository refreshed. Read and prepare again explicitly.'); }, [context, store]);
  useEffect(() => { if (previousObservation.current === state.snapshot) return; const prior = previousObservation.current; previousObservation.current = state.snapshot; store.getState().clearPrepared(); setConfirmation(null); if (prior) setNotice('Remote observation changed. Read and prepare again explicitly.'); }, [state.snapshot, store]);
  useEffect(() => { if (!remoteName && state.catalog?.remotes[0]) setRemoteName(state.catalog.remotes[0].name); }, [state.catalog, remoteName]);
  const pull = kind === 'pull', tag = kind === 'remoteTagPush' || kind === 'remoteTagDelete', deleting = kind === 'remoteTagDelete';
  useEffect(() => {
    if (!state.snapshot || destination.kind !== 'auto' || state.snapshot.remoteName !== remoteName) return;
    const name = deleting ? launch.refName : sourceRef;
    const selected = state.snapshot.refs.find(entry => entry.name === name && entry.name.startsWith(tag ? 'refs/tags/' : 'refs/heads/'));
    setDestination(selected ? { kind: 'existing', name: selected.name } : { kind: deleting ? 'none' : 'new' });
  }, [state.snapshot, destination, remoteName, sourceRef, deleting, tag, launch.refName]);
  const choices = local.catalog?.refs.filter(e => !e.symbolicTarget && e.kind === (tag ? 'tag' : 'local')) ?? [];
  const destinations = state.snapshot?.refs.filter(e => e.name.startsWith(tag ? 'refs/tags/' : 'refs/heads/')) ?? [];
  const selectedEntry = destination.kind === 'existing' ? destinations.find(entry => entry.name === destination.name) : undefined;
  const missingDestination = destination.kind === 'existing' && !selectedEntry;
  const selectedRemote = state.catalog?.remotes.find(remote => remote.name === remoteName), pushEndpoints = selectedRemote?.endpoints.filter(endpoint => endpoint.role === 'push') ?? [];
  const pushEndpoint = pushEndpoints.length === 1 ? pushEndpoints[0] : undefined;
  const available = !!state.scope && !!state.catalog && !!local.catalog && sameActionScope(state.scope, state.catalog.scope) && sameActionScope(state.scope, local.catalog.scope) && !local.loading && !local.error && !state.loading && !state.error;
  const snapshotCurrent = !!state.snapshot && sameActionScope(state.scope, state.snapshot.capturedScope) && state.snapshot.remoteName === remoteName && state.snapshot.role === 'push' && Date.parse(state.snapshot.expiresAt) > Date.now();
  const syncCurrent = !!currentSync.catalog && sameActionScope(state.scope, currentSync.catalog.scope) && !currentSync.catalogLoading && !currentSync.catalogError;
  const upstream = syncCurrent ? currentSync.catalog?.sync.upstream : null;
  const sourceMissing = !choices.some(entry => entry.name === sourceRef);
  const busy = state.preparing || state.executing || state.readLoading;
  const change = () => { store.getState().clearPrepared(); setConfirmation(null); setNotice(null); };
  const prepare = () => { setConfirmation(null); void store.getState().prepare({ kind, sourceRef: pull ? upstream ?? '' : sourceRef, remoteName, entryId: selectedEntry?.entryId ?? null, newName }); };
  const confirmationKey = JSON.stringify([state.prepared?.planId, state.snapshot?.snapshotId, selectedEntry?.name, selectedEntry?.oid]);
  const confirmed = confirmation === confirmationKey;
  const dismiss = () => { if (!pending.current) onDismiss(); };
  const execute = async () => { if (pending.current) return; pending.current = true; try { const r = await store.getState().execute(); if (r?.ok && mounted.current) onDismiss(); else if (r && !r.ok && mounted.current) setNotice('Execution reply was not confirmed. Check Tasks before preparing another action. OpenGit will not retry automatically.'); await onStarted(); } finally { pending.current = false; } };
  const canPrepare = available && !busy && (pull ? !!upstream : snapshotCurrent && !!pushEndpoint?.handle && pushEndpoint.capability === 'eligible' && !missingDestination && (deleting ? !!selectedEntry : !sourceMissing && (destination.kind === 'existing' || destination.kind === 'new' && !!newName.trim())));
  return <Modal title="Remote synchronization" className="remote-action-dialog" onDismiss={dismiss}><div className="modal-content">
    <h3>{pull ? 'Fast-forward-only pull' : deleting ? 'Delete remote tag' : tag ? 'Push tag' : 'Push branch'}</h3>
    <label>Action<select aria-label="Remote action" disabled={busy} value={kind} onChange={event => { change(); setKind(event.target.value as RemoteMutationKind); setDestination({ kind: 'auto' }); }}><option value="push">Push branch</option><option value="pull">Fast-forward-only pull</option><option value="remoteTagPush">Push tag</option><option value="remoteTagDelete">Delete remote tag</option></select></label>
    {notice && <p role="status">{notice}</p>}{state.error && <ErrorNotice error={state.error} />}{state.actionError && <ErrorNotice error={state.actionError} />}
    {local.error && <ErrorNotice error={local.error} />}{pull && currentSync.catalogError && <ErrorNotice error={currentSync.catalogError} />}
    {!available && <p role="status">Loading current repository context…</p>}
    {pull ? <p>Current branch: {syncCurrent ? currentSync.catalog?.sync.currentBranch ?? 'unavailable' : 'unavailable'}<br />Configured upstream: {upstream ?? 'not configured'}<br />Fetch the configured upstream, then fast-forward only. Dirty or diverged repositories are refused. No autostash, rebase or merge commit.</p> : <>
      {!deleting && <label>{tag ? 'Source tag' : 'Source branch'}<select aria-label={tag ? 'Source tag' : 'Source branch'} disabled={busy} value={sourceRef} onChange={e => { change(); setSourceRef(e.target.value); setDestination({ kind: 'auto' }); setNewName(e.target.value.replace(/^refs\/(heads|tags)\//, '')); }}><option value="">Select a qualified source</option>{choices.map(e => <option key={e.name} value={e.name}>{e.name}</option>)}{sourceMissing && sourceRef && <option value={sourceRef}>{sourceRef} (unavailable)</option>}</select></label>}
      {!deleting && sourceMissing && <p role="alert">The selected local source is unavailable. Choose a current qualified {tag ? 'tag' : 'branch'}.</p>}
      <label>Remote<select aria-label="Remote" disabled={busy} value={remoteName} onChange={e => { change(); store.getState().clearRead(); setRemoteName(e.target.value); setDestination({ kind: 'auto' }); }}><option value="">Select a remote</option>{state.catalog?.remotes.map(r => <option key={r.name} value={r.name}>{r.name}</option>)}{remoteName && !selectedRemote && <option value={remoteName}>{remoteName} (unavailable)</option>}</select></label>
      <p>Uses this remote's single push endpoint, which can differ from its fetch endpoint.</p>
      {remoteName && (!pushEndpoint?.handle || pushEndpoint.capability !== 'eligible') && <p role="alert">{!selectedRemote ? 'Selected remote is missing. Choose a current remote.' : `Push endpoint unavailable: ${pushEndpoint ? endpointReasons[pushEndpoint.capability] : 'a single push endpoint is required'}.`}</p>}
      <button type="button" disabled={!available || busy || !pushEndpoint?.handle || pushEndpoint.capability !== 'eligible'} onClick={() => { change(); void state.readRemoteRefs(remoteName); }}>Read remote references</button>
      {state.readLoading && <p role="status">Remote reference read is queued or running. Complete authentication in Tasks if requested.</p>}
      {state.readId && <p>Read task: {state.readId}</p>}
      {state.snapshot && <><p>Remote observation: {state.snapshot.observedAt}. This is dated evidence; the remote can change before Execute.</p>{!snapshotCurrent && <p role="alert">Remote observation expired or changed. Read again explicitly.</p>}<label>Destination<select aria-label="Destination" disabled={busy || !snapshotCurrent} value={destination.kind === 'existing' ? `ref:${destination.name}` : destination.kind} onChange={e => { change(); setDestination(e.target.value === 'new' ? { kind: 'new' } : { kind: 'existing', name: e.target.value.slice(4) }); }}>{!deleting && <option value="new">New destination</option>}{(destination.kind === 'none' || destination.kind === 'auto') && <option value={destination.kind}>Select an existing remote tag</option>}{destinations.map(e => <option key={e.entryId} value={`ref:${e.name}`}>{e.name} · {e.oid}</option>)}{destination.kind === 'existing' && missingDestination && <option value={`ref:${destination.name}`}>{destination.name} (unavailable)</option>}</select></label>{missingDestination && <p role="alert">The selected destination is missing from this observation. Choose a destination explicitly.</p>}{destination.kind === 'new' && !deleting && <label>New destination name<input aria-label="New destination name" disabled={busy} value={newName} onChange={e => { change(); setNewName(e.target.value); }} /></label>}</>}
      {!tag && <p>Git enforces fast-forward protection. Automatic upstream setup is unavailable here; Fetch, then use Local actions → Set or unset upstream. After acknowledgement, local tracking counts still require an explicit Fetch.</p>}
      {tag && !deleting && <p>Push the raw local tag object only to an absent or identical remote tag. Replacement is refused.</p>}
    </>}
    {state.preview && state.prepared && <section className="fetch-preview" aria-label="Remote action preview"><p>Source: {state.preview.source}</p><p>Destination: {state.preview.destination}</p><p>Expires: {state.prepared.expiresAt}</p>{state.prepared.warnings.map(warning => <p key={warning}>{warning.replace(/([a-z])([A-Z])/g, '$1 $2')}</p>)}{deleting && selectedEntry && <label><input type="checkbox" checked={confirmed} disabled={busy} onChange={e => setConfirmation(e.target.checked ? confirmationKey : null)} />Confirm remote tag deletion of {selectedEntry.name} at exact observed object {selectedEntry.oid} ({state.snapshot?.observedAt})</label>}</section>}
    <div className="dialog-actions"><button type="button" disabled={!canPrepare} onClick={prepare}>Prepare</button><button type="button" disabled={!state.prepared || !available || busy || !pull && !snapshotCurrent || deleting && !confirmed} onClick={() => { void execute(); }}>Execute</button><button type="button" disabled={busy} onClick={onRefresh}>Refresh context</button><button type="button" disabled={state.executing} onClick={dismiss}>Close</button></div>
  </div></Modal>;
}
