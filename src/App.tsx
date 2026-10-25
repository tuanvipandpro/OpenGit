import { useEffect, useRef, useState } from 'react';
import { useStore } from 'zustand';
import type { StoreApi } from 'zustand';
import { workspaceStore } from './stores/workspace';
import type { WorkspaceState } from './stores/workspace';
import { operationsStore, activeOperation } from './stores/operations';
import { RecentRepositories } from './features/workspaces/RecentRepositories';
import { RepositorySummary, RepositoryInfo } from './features/repository/RepositorySummary';
import { StatusList } from './features/repository/StatusList';
import { TaskPanel } from './features/tasks/TaskPanel';
import { Button } from './components/Button';
import { ErrorNotice } from './components/ErrorNotice';
import { EmptyState } from './components/EmptyState';
import { CloneDialog } from './features/clone/CloneDialog';
import { AuthDialog } from './features/tasks/AuthDialog';
import { BranchGlyph } from './components/Brand';
import { historyStore } from './stores/history';
import type { HistoryState } from './stores/history';
import { HistoryScreen } from './features/history/HistoryScreen';
import { repositorySyncStore } from './stores/repositorySync';
import type { RepositorySyncState } from './stores/repositorySync';
import { SyncPanel } from './features/sync/SyncPanel';
import { FetchDialog } from './features/sync/FetchDialog';
import type { RefreshInteractionLease } from './features/repository/refresh';
import { repositoryActionsStore } from './stores/repositoryActions';
import type { ActionLaunch, RepositoryActionsState } from './stores/repositoryActions';
import { RepositoryActionDialog } from './features/actions/RepositoryActionDialog';
import { ChangesPanel } from './features/changes/ChangesPanel';
import { repositoryChangesStore } from './stores/repositoryChanges';
import type { RepositoryChangesState } from './stores/repositoryChanges';
import { repositoryTransportStore } from './stores/repositoryTransport';
import type { RepositoryTransportState, TransportLaunch } from './stores/repositoryTransport';
import { RemoteActionDialog } from './features/sync/RemoteActionDialog';
const sameActionScopeForPublication = (a: import('./contracts').ActionScope|null,b:import('./contracts').ActionScope|null)=>!!a&&!!b&&a.repoId===b.repoId&&a.worktreeId===b.worktreeId&&a.generation===b.generation;
type FetchInteraction = { lease: RefreshInteractionLease; abort: AbortController };

export default function App({ store = workspaceStore, tasks = operationsStore, history = historyStore, sync = repositorySyncStore, actions = repositoryActionsStore, changes = repositoryChangesStore, transport = repositoryTransportStore }: { store?: StoreApi<WorkspaceState>; tasks?: typeof operationsStore; history?: StoreApi<HistoryState>; sync?: StoreApi<RepositorySyncState>; actions?: StoreApi<RepositoryActionsState>; changes?: StoreApi<RepositoryChangesState>; transport?:StoreApi<RepositoryTransportState> }) {
  const state = useStore(store);
  const operations = useStore(tasks);
  const transportState=useStore(transport);
  const syncState = useStore(sync);
  const fetchOwner = useRef<FetchInteraction | null>(null);
  const actionOwner = useRef<FetchInteraction | null>(null);
  const remoteOwner=useRef<FetchInteraction|null>(null);
  const [remoteForm,setRemoteForm]=useState<{owner:FetchInteraction;ready:boolean;launch:TransportLaunch}|null>(null);
  const [actionForm, setActionForm] = useState<{ owner: FetchInteraction; ready: boolean; launch: ActionLaunch } | null>(null);
  const [fetchForm, setFetchForm] = useState<{ owner: FetchInteraction; ready: boolean } | null>(null);
  const [settings, setSettings] = useState(false), [cloneOpen, setCloneOpen] = useState(false);
  const [repositoryTab, setRepositoryTab] = useState<'overview' | 'history' | 'changes'>('history');
  const requestedOutcomes = useRef(new Map<string, string>());
  const completed = useRef(new Set<string>());
  const authOperation = operations.snapshot?.operations.find(operation => operation.state === 'WaitingForUser' && operation.authPrompt);
  const cloneUnavailable = state.loading || state.busy || !state.workspace || state.environment?.git.available === false;
  const flushPreferences = async () => {
    const preferences = history.getState().preferences;
    if (!preferences.getState().hydrated) return true;
    await preferences.getState().flush();
    return !preferences.getState().error;
  };
  const leaveRepository = async (action: () => Promise<void>) => { if (await flushPreferences()) await action(); };
  const closeFetch = (owner: FetchInteraction) => {
    if (fetchOwner.current !== owner) return;
    fetchOwner.current = null; owner.abort.abort(); sync.getState().clearPrepared(); setFetchForm(null); owner.lease.release();
  };
  const openFetch = () => {
    if (fetchOwner.current || actionOwner.current||remoteOwner.current) return;
    const lease = store.getState().beginInteraction(); if (!lease) return;
    const owner = { lease, abort: new AbortController() }; fetchOwner.current = owner;
    sync.getState().clearPrepared(); setFetchForm({ owner, ready: false });
    void lease.closed.then(() => {
      owner.abort.abort();
      if (fetchOwner.current === owner) { fetchOwner.current = null; sync.getState().clearPrepared(); setFetchForm(null); }
    });
    void (async () => {
      const ready = await lease.ready && await sync.getState().awaitCatalog(owner.abort.signal);
      if (fetchOwner.current !== owner || !lease.isCurrent()) return;
      if (ready) setFetchForm({ owner, ready: true }); else closeFetch(owner);
    })();
  };
  const closeAction = (owner: FetchInteraction) => {
    if (actionOwner.current !== owner) return;
    actionOwner.current = null; owner.abort.abort(); actions.getState().clearPrepared(); setActionForm(null); owner.lease.release();
  };
  const openAction = (launch: ActionLaunch = {}) => {
    if(launch.remoteKind){openRemote({kind:launch.remoteKind,refName:launch.refName});return;}
    if (actionOwner.current || fetchOwner.current||remoteOwner.current) return;
    const lease = store.getState().beginInteraction(); if (!lease) return;
    const owner = { lease, abort: new AbortController() }; actionOwner.current = owner;
    actions.getState().clearPrepared(); setActionForm({ owner, ready: false, launch });
    void lease.closed.then(() => {
      owner.abort.abort();
      if (actionOwner.current === owner) { actionOwner.current = null; actions.getState().clearPrepared(); setActionForm(null); }
    });
    void (async () => {
      const ready = await lease.ready && await actions.getState().awaitCatalog(owner.abort.signal);
      if (actionOwner.current !== owner || !lease.isCurrent()) return;
      if (ready) setActionForm({ owner, ready: true, launch }); else closeAction(owner);
    })();
  };
  function closeRemote(owner:FetchInteraction){if(remoteOwner.current!==owner)return;remoteOwner.current=null;owner.abort.abort();transport.getState().end();setRemoteForm(null);owner.lease.release();}
  function openRemote(launch:TransportLaunch){
    if(remoteOwner.current||actionOwner.current||fetchOwner.current)return;const lease=store.getState().beginInteraction();if(!lease)return;
    const owner={lease,abort:new AbortController()};remoteOwner.current=owner;setRemoteForm({owner,ready:false,launch});
    void lease.closed.then(()=>{owner.abort.abort();if(remoteOwner.current===owner){remoteOwner.current=null;transport.getState().end();setRemoteForm(null);}});
    void(async()=>{const ready=await lease.ready&&await actions.getState().awaitCatalog(owner.abort.signal);if(remoteOwner.current!==owner||!lease.isCurrent())return;if(ready)setRemoteForm({owner,ready:true,launch});else closeRemote(owner);})();
  }
  useEffect(() => () => {
    const action = actionOwner.current; actionOwner.current = null; action?.abort.abort(); action?.lease.cancel();
    const owner = fetchOwner.current; fetchOwner.current = null; owner?.abort.abort(); owner?.lease.cancel();
    const remote=remoteOwner.current;remoteOwner.current=null;remote?.abort.abort();remote?.lease.cancel();
  }, []);
  useEffect(() => {
    let cloneChanged = false, actionChanged = false;
    const selection = state.workspace?.activeSelection;
    const live = new Set(operations.snapshot?.operations.map(operation => operation.id));
    for (const id of requestedOutcomes.current.keys()) if (!live.has(id)) requestedOutcomes.current.delete(id);
    for (const id of completed.current) if (!live.has(id)) completed.current.delete(id);
    for (const operation of operations.snapshot?.operations ?? []) {
      if (operation.kind === 'clone' && operation.state === 'Succeeded' && !completed.current.has(operation.id)) { completed.current.add(operation.id); cloneChanged = true; }
      if (operation.kind !== 'action' || activeOperation(operation)) continue;
      const result = operations.outcomes[operation.id]?.operation;
      const requestKey = `${operations.snapshot!.streamId}:${operation.sequence}`;
      if (requestedOutcomes.current.get(operation.id) !== requestKey) {
        requestedOutcomes.current.set(operation.id, requestKey);
        if (!result || BigInt(result.sequence) < BigInt(operation.sequence)) void tasks.getState().loadOutcome(operation.id);
      }
      const latest = result && BigInt(result.sequence) >= BigInt(operation.sequence) ? result : operation;
      if (latest.outcome?.repositoryChanged && selection?.repoId === latest.repoId && !completed.current.has(operation.id)) {
        completed.current.add(operation.id); actionChanged = true;
      }
    }
    if (cloneChanged) void store.getState().reload();
    else if (actionChanged) void store.getState().requestRefresh('operation');
  }, [operations.snapshot, operations.outcomes, state.workspace?.activeSelection?.repoId, store, tasks]);
  const active = state.workspace?.repositories.find(repo => repo.id === state.workspace?.activeSelection?.repoId);
  useEffect(() => {
    const disconnect = store.getState().connectHistory(history);
    const disconnectSync = sync.getState().connect(history);
    const disconnectActions = actions.getState().connect(history);
    const disconnectTransport=transport.getState().connect(history,actions,tasks);
    void store.getState().initialize();
    return () => { disconnectTransport();disconnectActions(); disconnectSync(); disconnect(); };
  }, [store, history, sync, actions,tasks,transport]);
  const workspaceReady = state.workspace !== null;
  useEffect(() => {
    if (!workspaceReady) return;
    let alive = true, stop: (() => void) | undefined;
    void tasks.getState().start().then(dispose => { if (alive) stop = dispose; else dispose(); });
    return () => { alive = false; stop?.(); };
  }, [tasks, workspaceReady]);
  return <div className="app-shell">
    <aside className="sidebar"><a className="brand" href="#workspace" aria-label="OpenGit workspace"><BranchGlyph /><strong>OpenGit</strong><span>DESKTOP</span></a>
      <RecentRepositories workspace={state.workspace} onSelect={selection => { void leaveRepository(() => state.select(selection)); }} onForget={id => { void leaveRepository(() => state.forget(id)); }} />
      <div className="sidebar-bottom"><button className="settings-button" onClick={() => setSettings(value => !value)} aria-expanded={settings}>⚙ <span>Settings &amp; diagnostics</span></button>{settings && <section className="diagnostics" aria-label="Git diagnostics"><strong>OpenGit {state.environment?.appVersion ?? '0.3.0'}</strong><p>Git {state.environment?.git.available ? state.environment.git.version ?? 'available' : state.environment?.git.version ? state.environment.git.version + ' is unsupported. Git 2.31 or later is required.' : 'not found'}</p><p>Credential manager {state.environment?.credentialManager.available ? state.environment.credentialManager.version ?? 'available' : 'not detected'}</p><p>OpenSSH {state.environment?.openSsh.available ? state.environment.openSsh.version ?? 'available' : 'not detected'}</p></section>}</div>
    </aside>
    <div className="main-shell" id="workspace"><header className="toolbar"><div className="current-repository"><span className="muted">Workspace /</span><strong>{active?.name ?? 'Overview'}</strong></div><div className="toolbar-actions">{active && <Button disabled={!!fetchForm || !!actionForm || !!remoteForm} onClick={() => openAction()}>Local actions</Button>}{active && <Button disabled={state.refreshing || state.busy} onClick={() => { void state.refresh(); }} aria-label="Refresh repository">↻ <span>Refresh</span></Button>}<Button disabled={state.busy || state.loading} onClick={() => { void leaveRepository(() => state.open()); }}>Open repository</Button><Button className="primary" disabled={cloneUnavailable} onClick={() => setCloneOpen(true)}>Clone repository</Button></div></header>
      {state.error && <div className="global-error"><ErrorNotice error={state.error} onRetry={() => { void state.initialize(); }} /></div>}
      {repositoryTab === 'history' && state.inspectionError && <div className="global-error"><ErrorNotice error={state.inspectionError} onRetry={() => { void state.refresh(); }} /></div>}
      {active && <nav className="repository-tabs" aria-label="Repository views"><button type="button" aria-current={repositoryTab === 'overview' ? 'page' : undefined} onClick={() => { void leaveRepository(async () => setRepositoryTab('overview')); }}>Overview</button><button type="button" aria-current={repositoryTab === 'changes' ? 'page' : undefined} onClick={() => setRepositoryTab('changes')}>Changes</button><button type="button" aria-current={repositoryTab === 'history' ? 'page' : undefined} onClick={() => setRepositoryTab('history')}>History</button>{repositoryTab === 'history' && state.refreshing && <span className="repository-refresh-status" role="status">Refreshing…</span>}</nav>}
      {active && <SyncPanel store={sync} onPush={()=>openRemote({kind:'push'})} onPull={()=>openRemote({kind:'pull'})} publication={sameActionScopeForPublication(transportState.publicationScope,syncState.scope)?transportState.publications.find(p=>p.sourceRef===`refs/heads/${syncState.catalog?.head.branch}`&&p.sourceOid===syncState.catalog?.head.oid&&!p.deleted):undefined} opening={!!fetchForm && !fetchForm.ready} onCancelOpening={() => { if (fetchForm) closeFetch(fetchForm.owner); }} onFetch={openFetch} onRefresh={() => { void state.refresh(); }} />}
      <div className={`workspace-content${active && repositoryTab === 'history' ? ' history-workspace' : ''}`} aria-busy={state.loading || (state.inspecting && !state.snapshot)}>
        {state.loading ? <div className="loading-state" role="status">Loading your workspace…</div> : !active ? <div className="welcome"><div className="welcome-symbol"><BranchGlyph /></div><p className="eyebrow">A CLEAR VIEW OF YOUR WORK</p><h1>Your next idea<br />starts here.</h1><p>Bring your repositories together in one workspace.<br />Open a local repository to see its current branch, status and remotes.</p><div className="welcome-actions"><Button className="primary" disabled={state.busy} onClick={() => { void leaveRepository(() => state.open()); }}>Open a repository</Button><Button disabled={cloneUnavailable} onClick={() => setCloneOpen(true)}>Clone a repository</Button></div><div className="welcome-hint"><span aria-hidden="true">⌘</span><div><strong>Your repositories stay yours.</strong><p>Opening or removing a repository from the workspace keeps your files in place.</p></div></div></div> : <>
          {repositoryTab === 'changes' ? <ChangesPanel store={changes} history={history} workspace={store} tasks={tasks} /> : repositoryTab === 'history' ? <HistoryScreen onAction={openAction} syncStore={sync} store={history} selection={state.busy && !state.snapshot ? null : state.workspace?.activeSelection ?? null} /> : <><div className="repository-main"><div className="repository-title"><div><p className="eyebrow">REPOSITORY OVERVIEW</p><h1>{active.name}</h1></div>{state.refreshing && <span className="refreshing" role="status">Refreshing…</span>}</div>
          {state.inspectionError && <ErrorNotice error={state.inspectionError} onRetry={() => { void state.refresh(); }} />}{state.snapshot ? <><RepositorySummary snapshot={state.snapshot} /><StatusList snapshot={state.snapshot} /></> : !state.inspecting && <EmptyState title="Repository information is unavailable"><p>Refresh or reopen this repository to try again.</p></EmptyState>}</div>
          {state.snapshot && <RepositoryInfo snapshot={state.snapshot} />}</>}
        </>}
      </div>
      {operations.error && <div className="task-error"><ErrorNotice error={operations.error} onRetry={() => { void operations.sync(); }} /></div>}
      <TaskPanel outcomes={operations.outcomes} outcomeErrors={operations.outcomeErrors} onInspect={id => { void operations.loadOutcome(id); }} operations={operations.snapshot?.operations ?? []} onCancel={id => { void operations.cancel(id); }} onRetry={() => setCloneOpen(true)} onOpen={operation => { if (operation.repoId) void leaveRepository(() => state.select({ repoId: operation.repoId!, worktreeId: operation.worktreeId })); }} />
      {remoteForm&&!remoteForm.ready&&<p role="status">Waiting for current remote action context… <button type="button" onClick={()=>closeRemote(remoteForm.owner)}>Cancel opening remote action</button></p>}
      {remoteForm?.ready&&remoteForm.owner.lease.isCurrent()&&<RemoteActionDialog store={transport} refs={actions} sync={sync} launch={remoteForm.launch} onDismiss={()=>closeRemote(remoteForm.owner)} onStarted={()=>tasks.getState().sync()} onRefresh={()=>{void store.getState().refresh();}}/>}
      {actionForm && !actionForm.ready && <div role="status">Loading local actions… <button type="button" onClick={() => closeAction(actionForm.owner)}>Cancel opening local actions</button></div>}
      {actionForm?.ready && actionForm.owner.lease.isCurrent() && <RepositoryActionDialog store={actions} launch={actionForm.launch} onRefresh={() => { void store.getState().refresh(); }} onDismiss={() => closeAction(actionForm.owner)} onStarted={() => tasks.getState().sync()} />}
      {fetchForm?.ready && fetchForm.owner.lease.isCurrent() && <FetchDialog store={sync} onRefresh={() => { void store.getState().refresh(); }} onDismiss={() => closeFetch(fetchForm.owner)} onStarted={() => tasks.getState().sync()} />}
      {cloneOpen && <CloneDialog onDismiss={() => setCloneOpen(false)} chooseSource={operations.chooseSource} chooseDestination={operations.chooseDestination} onStart={async request => { if (!await flushPreferences()) return { ok: false, error: history.getState().preferences.getState().error! }; return operations.clone(request); }} />}
      {!cloneOpen && authOperation?.authPrompt && <AuthDialog sourceDisplay={authOperation.sourceDisplay} key={authOperation.authPrompt.promptHandle} prompt={authOperation.authPrompt} onRespond={value => operations.respond(authOperation.id, authOperation.authPrompt!.promptHandle, value)} onCancel={() => operations.cancel(authOperation.id)} />}
      <footer className="statusbar"><span><i aria-hidden="true" />{!state.environment ? 'Checking Git…' : state.environment.git.available ? 'Git ready' : state.environment.git.version ? 'Git needs update' : 'Git not found'}</span><span>{state.workspace?.repositories.length ?? 0} repositories in workspace</span><span>OpenGit 0.3.0</span></footer>
    </div>
  </div>;
}
