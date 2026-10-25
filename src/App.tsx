import { useEffect, useRef, useState } from 'react';
import { useStore } from 'zustand';
import type { StoreApi } from 'zustand';
import { workspaceStore } from './stores/workspace';
import type { WorkspaceState } from './stores/workspace';
import { operationsStore } from './stores/operations';
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

export default function App({ store = workspaceStore, tasks = operationsStore, history = historyStore }: { store?: StoreApi<WorkspaceState>; tasks?: typeof operationsStore; history?: StoreApi<HistoryState> }) {
  const state = useStore(store);
  const operations = useStore(tasks);
  const [settings, setSettings] = useState(false), [cloneOpen, setCloneOpen] = useState(false);
  const [repositoryTab, setRepositoryTab] = useState<'overview' | 'history'>('history');
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
  useEffect(() => {
    let changed = false;
    for (const operation of operations.snapshot?.operations ?? []) {
      if (operation.state === 'Succeeded' && !completed.current.has(operation.id)) { completed.current.add(operation.id); changed = true; }
    }
    if (changed) void store.getState().reload();
  }, [operations.snapshot, store]);
  const active = state.workspace?.repositories.find(repo => repo.id === state.workspace?.activeSelection?.repoId);
  useEffect(() => {
    const disconnect = store.getState().connectHistory(history);
    void store.getState().initialize();
    return disconnect;
  }, [store, history]);
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
      <div className="sidebar-bottom"><button className="settings-button" onClick={() => setSettings(value => !value)} aria-expanded={settings}>⚙ <span>Settings &amp; diagnostics</span></button>{settings && <section className="diagnostics" aria-label="Git diagnostics"><strong>OpenGit {state.environment?.appVersion ?? '0.2.0'}</strong><p>Git {state.environment?.git.available ? state.environment.git.version ?? 'available' : state.environment?.git.version ? state.environment.git.version + ' is unsupported. Git 2.31 or later is required.' : 'not found'}</p><p>Credential manager {state.environment?.credentialManager.available ? state.environment.credentialManager.version ?? 'available' : 'not detected'}</p><p>OpenSSH {state.environment?.openSsh.available ? state.environment.openSsh.version ?? 'available' : 'not detected'}</p></section>}</div>
    </aside>
    <div className="main-shell" id="workspace"><header className="toolbar"><div className="current-repository"><span className="muted">Workspace /</span><strong>{active?.name ?? 'Overview'}</strong></div><div className="toolbar-actions">{active && <Button disabled={state.refreshing || state.busy} onClick={() => { void state.refresh(); }} aria-label="Refresh repository">↻ <span>Refresh</span></Button>}<Button disabled={state.busy || state.loading} onClick={() => { void leaveRepository(() => state.open()); }}>Open repository</Button><Button className="primary" disabled={cloneUnavailable} onClick={() => setCloneOpen(true)}>Clone repository</Button></div></header>
      {state.error && <div className="global-error"><ErrorNotice error={state.error} onRetry={() => { void state.initialize(); }} /></div>}
      {repositoryTab === 'history' && state.inspectionError && <div className="global-error"><ErrorNotice error={state.inspectionError} onRetry={() => { void state.refresh(); }} /></div>}
      {active && <nav className="repository-tabs" aria-label="Repository views"><button type="button" aria-current={repositoryTab === 'overview' ? 'page' : undefined} onClick={() => { void leaveRepository(async () => setRepositoryTab('overview')); }}>Overview</button><button type="button" aria-current={repositoryTab === 'history' ? 'page' : undefined} onClick={() => setRepositoryTab('history')}>History</button>{repositoryTab === 'history' && state.refreshing && <span className="repository-refresh-status" role="status">Refreshing…</span>}</nav>}
      <div className={`workspace-content${active && repositoryTab === 'history' ? ' history-workspace' : ''}`} aria-busy={state.loading || (state.inspecting && !state.snapshot)}>
        {state.loading ? <div className="loading-state" role="status">Loading your workspace…</div> : !active ? <div className="welcome"><div className="welcome-symbol"><BranchGlyph /></div><p className="eyebrow">A CLEAR VIEW OF YOUR WORK</p><h1>Your next idea<br />starts here.</h1><p>Bring your repositories together in one workspace.<br />Open a local repository to see its current branch, status and remotes.</p><div className="welcome-actions"><Button className="primary" disabled={state.busy} onClick={() => { void leaveRepository(() => state.open()); }}>Open a repository</Button><Button disabled={cloneUnavailable} onClick={() => setCloneOpen(true)}>Clone a repository</Button></div><div className="welcome-hint"><span aria-hidden="true">⌘</span><div><strong>Your repositories stay yours.</strong><p>Opening or removing a repository from the workspace keeps your files in place.</p></div></div></div> : <>
          {repositoryTab === 'history' ? <HistoryScreen store={history} selection={state.busy && !state.snapshot ? null : state.workspace?.activeSelection ?? null} /> : <><div className="repository-main"><div className="repository-title"><div><p className="eyebrow">REPOSITORY OVERVIEW</p><h1>{active.name}</h1></div>{state.refreshing && <span className="refreshing" role="status">Refreshing…</span>}</div>
          {state.inspectionError && <ErrorNotice error={state.inspectionError} onRetry={() => { void state.refresh(); }} />}{state.snapshot ? <><RepositorySummary snapshot={state.snapshot} /><StatusList snapshot={state.snapshot} /></> : !state.inspecting && <EmptyState title="Repository information is unavailable"><p>Refresh or reopen this repository to try again.</p></EmptyState>}</div>
          {state.snapshot && <RepositoryInfo snapshot={state.snapshot} />}</>}
        </>}
      </div>
      {operations.error && <div className="task-error"><ErrorNotice error={operations.error} onRetry={() => { void operations.sync(); }} /></div>}
      <TaskPanel operations={operations.snapshot?.operations ?? []} onCancel={id => { void operations.cancel(id); }} onRetry={() => setCloneOpen(true)} onOpen={operation => { if (operation.repoId) void leaveRepository(() => state.select({ repoId: operation.repoId!, worktreeId: operation.worktreeId })); }} />
      {cloneOpen && <CloneDialog onDismiss={() => setCloneOpen(false)} chooseSource={operations.chooseSource} chooseDestination={operations.chooseDestination} onStart={async request => { if (!await flushPreferences()) return { ok: false, error: history.getState().preferences.getState().error! }; return operations.clone(request); }} />}
      {!cloneOpen && authOperation?.authPrompt && <AuthDialog sourceDisplay={authOperation.sourceDisplay} key={authOperation.authPrompt.promptHandle} prompt={authOperation.authPrompt} onRespond={value => operations.respond(authOperation.id, authOperation.authPrompt!.promptHandle, value)} onCancel={() => operations.cancel(authOperation.id)} />}
      <footer className="statusbar"><span><i aria-hidden="true" />{!state.environment ? 'Checking Git…' : state.environment.git.available ? 'Git ready' : state.environment.git.version ? 'Git needs update' : 'Git not found'}</span><span>{state.workspace?.repositories.length ?? 0} repositories in workspace</span><span>OpenGit 0.2.0</span></footer>
    </div>
  </div>;
}
