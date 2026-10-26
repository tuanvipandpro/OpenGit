import { StrictMode } from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, afterEach, it, expect, vi } from 'vitest';
import App from './App';
import { createWorkspaceStore } from './stores/workspace';
import type { Api } from './stores/workspace';
import { createOperationsStore } from './stores/operations';
import { createHistoryStore } from './stores/history';
import { createRepositorySyncStore } from './stores/repositorySync';
import { createRepositoryActionsStore } from './stores/repositoryActions';
import { createRepositoryChangesStore } from './stores/repositoryChanges';
import { createRepositoryTransportStore } from './stores/repositoryTransport';
import { createRepositoryStashStore } from './stores/repositoryStash';
import changesWire from '../fixtures/contracts/changes-wire.json';
import refFixture from '../fixtures/contracts/ref-actions-wire.json';
import type { Workspace, ToolInfo, Environment } from './contracts';
import fixture from '../fixtures/contracts/wire.json';
import { defaultRepositoryView } from './stores/historyView';
beforeEach(() => { vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} }); vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null); });
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });
function makeServices(failed = false, git?: ToolInfo) {
  const workspace = fixture.commands[1].result.value as unknown as Workspace;
  const empty = { ...workspace, revision: '0', repositories: [], activeSelection: null };
  const api = vi.fn(async (command: string, request: Record<string, unknown>) => {
    if (command === 'get_repository_view' || command === 'save_repository_view') return { ok: true, value: { scope: request.scope, viewRevision: '0', view: request.view ?? defaultRepositoryView() } };
    if (command === 'get_repository_actions') return { ok: true, value: { scope: request.scope, snapshotId: 'catalog', observedAt: '', head: { kind: 'unborn', branch: 'main', oid: null }, branches: [], symbolicBranches: [], remotes: [], sync: { currentBranch: null, upstream: null, ahead: null, behind: null, completeness: 'complete' } } };
    if (command === 'get_ref_actions') return { ok: true, value: { ...refFixture.catalog, scope: request.scope, catalogId: `refs-${(request.scope as import('./contracts').ActionScope).generation}` } };
    if (command === 'get_repository_changes') return { ok:true,value:{...changesWire.catalog,scope:request.scope,catalogId:`changes-${(request.scope as import('./contracts').ActionScope).generation}`}};
    if (command === 'get_repository_stashes') return { ok: true, value: { scope: request.scope, catalogId: `stashes-${(request.scope as import('./contracts').ActionScope).generation}`, observedAt: '', saveAvailable: true, entries: [] } };
    if (command === 'resolve_ref_action_target') return { ok: true, value: { ...refFixture.resolved, scope: request.scope, catalogId: request.catalogId, kind: 'commit', displayName: 'HEAD' } };
    if (command === 'get_environment') return git ? { ok: true, value: { ...(fixture.commands[0].result.value as unknown as Environment), git } } : fixture.commands[0].result;
    if (command === 'get_workspace') return failed ? fixture.error : { ok: true, value: empty };
    if (command === 'choose_repository') return { ok: true, value: workspace };
    if (command === 'inspect_repository') return { ok: true, value: { ...fixture.snapshots[0], ...request } };
    if (command === 'forget_repository') return { ok: true, value: { ...empty, revision: '9007199254740994' } };
    if (command === 'open_history') { const context = { scope: request.scope, snapshotId: 'empty' }; return { ok: true, value: { context, sessionId: 'empty', head: { kind: 'unborn', branch: 'main', oid: null }, refs: [], roots: [], page: { context, sessionId: 'empty', commits: [], nextCursor: null } } }; }
    if (command === 'watch_repository') return { ok: true, value: { scope: request.scope, subscriptionId: 'watch', sequence: '0', status: 'watching' } };
    if (command === 'unwatch_repository') return { ok: true, value: null };
    if (command === 'get_operations') return { ok: true, value: { streamId: 'stream', sequence: '0', operations: [] } };
    throw Error('Unexpected command');
  }) as unknown as Api;
  return { stash: createRepositoryStashStore(api), transport: createRepositoryTransportStore(api), changes:createRepositoryChangesStore(api),actions: createRepositoryActionsStore(api), sync: createRepositorySyncStore(api), store: createWorkspaceStore(api, async () => () => {}), history: createHistoryStore(api), tasks: createOperationsStore(api, async () => () => {}), api };
}
function pendingReconcileServices() {
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  let operation!: import('./contracts').OperationSnapshot, finish: ((value: unknown) => void) | null = null;
  const evidence = { stashOid: 'a'.repeat(40), stashRetention: 'present', recoveryRefRetention: 'present', resolution: 'unmerged', failureReason: 'conflict' };
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'get_operations' && operation) return Promise.resolve({ ok: true, value: { streamId: 'held-stash', sequence: operation.sequence, operations: [operation] } });
    if (command === 'get_stash_action_outcome') return Promise.resolve({ ok: true, value: { operation, result: evidence } });
    if (command === 'reconcile_stash_action') return new Promise<unknown>(resolve => { finish = resolve; });
    return original(command as never, request as never);
  }) as Api);
  return { ...services,
    seed: async () => {
      await services.store.getState().open();
      const scope = services.history.getState().preferences.getState().scope!;
      operation = { ...refFixture.result.operation as import('./contracts').OperationSnapshot, repoId: scope.repoId, worktreeId: scope.worktreeId, actionKind: 'stashApply', state: 'Failed', error: null, outcome: { ...refFixture.result.operation.outcome as NonNullable<import('./contracts').OperationSnapshot['outcome']>, effects: 'conflict', repositoryChanged: false } };
      services.tasks.setState({ snapshot: { streamId: 'held-stash', sequence: operation.sequence, operations: [operation] } });
    },
    complete: () => { const reply = finish; finish = null; reply?.({ ok: true, value: { operation, result: { ...evidence, resolution: 'resolvedExternally' } } }); },
  };
}
it('S6-RECONCILE02 holds one real lease through slow read-only Reconcile and flushes passive refresh once', async () => {
  vi.useFakeTimers(); const services = pendingReconcileServices(), view = render(<App {...services} />);
  try {
    await act(async () => services.seed()); await act(async () => {});
    const reads = () => vi.mocked(services.api).mock.calls.filter(([command]) => command === 'inspect_repository').length;
    const before = reads();
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Reconcile stash' })); fireEvent.click(screen.getByRole('button', { name: 'Reconcile stash' })); });
    expect(vi.mocked(services.api).mock.calls.filter(([command]) => command === 'reconcile_stash_action')).toHaveLength(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(61_000); window.dispatchEvent(new Event('focus')); await vi.advanceTimersByTimeAsync(200); await services.store.getState().requestRefresh('watch'); });
    expect(reads()).toBe(before);
    await act(async () => services.complete()); await act(async () => {});
    expect(screen.getByText('Resolved externally')).toBeInTheDocument(); expect(reads()).toBe(before + 1);
    expect(vi.mocked(services.api).mock.calls.some(([command]) => ['prepare_repository_action', 'execute_repository_action'].includes(command))).toBe(false);
  } finally { services.complete(); view.unmount(); vi.useRealTimers(); }
});
it('S6-RECONCILE02 manual refresh and hard forget fence replies and retire the read interaction', async () => {
  const services = pendingReconcileServices(), view = render(<App {...services} />);
  try {
    await act(async () => services.seed()); await act(async () => {});
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Reconcile stash' })));
    await act(async () => services.store.getState().requestRefresh('manual'));
    await act(async () => services.complete());
    expect(screen.queryByText('Resolved externally')).not.toBeInTheDocument(); expect(screen.getByText('Repository context changed. Reconcile again when it is ready.')).toBeInTheDocument();
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Reconcile stash' })));
    await act(async () => services.store.getState().forget(services.store.getState().workspace!.activeSelection!.repoId));
    await act(async () => services.complete());
    expect(screen.queryByText('Resolved externally')).not.toBeInTheDocument(); expect(services.tasks.getState().reconciling).toEqual({});
  } finally { services.complete(); view.unmount(); }
});
it('S6-READY01 waits for published history before enabling Stash and opens once after publication', async () => {
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  let finish!: (value: unknown) => void, historyRequest!: Record<string, unknown>;
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => command === 'open_history'
    ? (historyRequest = request, new Promise<unknown>(resolve => { finish = resolve; })) : original(command as never, request as never)) as Api);
  const view = render(<App {...services} />);
  try {
    let opening: Promise<unknown> | undefined;
    await act(async () => { opening = services.store.getState().open(); for (let i = 0; i < 10; i++) await Promise.resolve(); });
    const button = await screen.findByRole('button', { name: 'Stash' });
    expect(button).toBeDisabled(); fireEvent.click(button);
    expect(vi.mocked(services.api).mock.calls.some(([command]) => command === 'get_repository_stashes')).toBe(false);
    await act(async () => { finish(await original('open_history' as never, historyRequest as never)); await opening; });
    expect(button).toBeEnabled();
    const begin = vi.spyOn(services.store.getState(), 'beginInteraction');
    await act(async () => { fireEvent.click(button); fireEvent.click(button); });
    expect(begin).toHaveBeenCalledOnce(); expect(screen.getByRole('dialog', { name: 'Stash' })).toBeInTheDocument();
  } finally { view.unmount(); }
});
it('S6-READY01 explains an unavailable interaction and opens after the existing owner releases it', async () => {
  const services = makeServices(), view = render(<App {...services} />);
  try {
    await act(async () => services.store.getState().open()); await act(async () => {});
    const held = services.store.getState().beginInteraction()!; expect(held).not.toBeNull();
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Stash' })));
    expect(screen.getByRole('alert')).toHaveTextContent('Close the current action or diff before opening Stash.');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(vi.mocked(services.api).mock.calls.some(([command]) => command === 'get_repository_stashes')).toBe(false);
    await act(async () => { held.release(); fireEvent.click(screen.getByRole('button', { name: 'Stash' })); });
    expect(screen.getByRole('dialog', { name: 'Stash' })).toBeInTheDocument(); expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  } finally { view.unmount(); }
});
it('S6 stash form holds real passive refresh, retains inputs through urgent refresh and hard-closes on forget', async () => {
  vi.useFakeTimers(); const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => command === 'prepare_repository_action' ? Promise.resolve({ ok: true, value: { scope: request.scope, kind: 'stashPush', planId: 'save', expiresAt: '2099-01-01', warnings: [] } }) : original(command as never, request as never)) as Api);
  const view = render(<StrictMode><App {...services} /></StrictMode>);
  try {
    await act(async () => services.store.getState().open()); await act(async () => {}); const begin = vi.spyOn(services.store.getState(), 'beginInteraction');
    fireEvent.click(screen.getByRole('button', { name: 'Stash' })); fireEvent.click(screen.getByRole('button', { name: 'Stash' })); await act(async () => {}); expect(begin).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole('button', { name: 'Save stash' })); fireEvent.change(screen.getByLabelText('Stash message'), { target: { value: 'preserve message' } }); fireEvent.click(screen.getByLabelText('Include untracked files'));
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' })));
    const reads = () => vi.mocked(services.api).mock.calls.filter(([command]) => command === 'inspect_repository').length, before = reads();
    await act(async () => { await vi.advanceTimersByTimeAsync(61_000); window.dispatchEvent(new Event('focus')); await vi.advanceTimersByTimeAsync(200); await services.store.getState().requestRefresh('watch'); });
    expect(reads()).toBe(before); expect(screen.getByRole('button', { name: 'Execute' })).toBeEnabled();
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Refresh context' })));
    expect(screen.getByLabelText('Stash message')).toHaveValue('preserve message'); expect(screen.getByLabelText('Include untracked files')).toBeChecked(); expect(screen.getByRole('button', { name: 'Execute' })).toBeDisabled();
    expect(vi.mocked(services.api).mock.calls.filter(([command]) => command === 'prepare_repository_action')).toHaveLength(1);
    await act(async () => services.store.getState().forget(services.store.getState().workspace!.activeSelection!.repoId)); expect(screen.queryByRole('dialog')).not.toBeInTheDocument(); expect(services.stash.getState().message).toBe('');
  } finally { view.unmount(); vi.useRealTimers(); }
});
it('S6 cancel opening and late stash catalogs cannot reopen a retired manager', async () => {
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!; let finish!: (value: unknown) => void, request!: Record<string, unknown>;
  vi.mocked(services.api).mockImplementation(((command: string, value: Record<string, unknown>) => command === 'get_repository_stashes' ? (request = value, new Promise<unknown>(resolve => { finish = resolve; })) : original(command as never, value as never)) as Api);
  const view = render(<App {...services} />);
  try {
    await act(async () => services.store.getState().open()); await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Stash' })));
    fireEvent.click(screen.getByRole('button', { name: 'Cancel opening stash' })); await act(async () => finish({ ok: true, value: { scope: request.scope, catalogId: 'late', saveAvailable: true, entries: [], observedAt: '' } }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument(); const lease = services.store.getState().beginInteraction()!; expect(lease).not.toBeNull(); lease.release();
  } finally { view.unmount(); }
});
it('S6 Tasks reconcile uses the published current scope and renders Resolved externally without queuing another action', async () => {
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  let operation!: import('./contracts').OperationSnapshot;
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => ['get_stash_action_outcome', 'reconcile_stash_action'].includes(command) ? Promise.resolve({ ok: true, value: { operation, result: { stashOid: 'a'.repeat(40), stashRetention: 'present', recoveryRefRetention: 'unknown', resolution: command === 'reconcile_stash_action' ? 'resolvedExternally' : 'unmerged', failureReason: 'conflict' } } }) : original(command as never, request as never)) as Api);
  const view = render(<App {...services} />);
  try {
    await act(async () => services.store.getState().open()); await act(async () => {}); const scope = services.history.getState().preferences.getState().scope!;
    operation = { ...refFixture.result.operation as import('./contracts').OperationSnapshot, repoId: scope.repoId, worktreeId: scope.worktreeId, actionKind: 'stashApply', error: null, outcome: { ...refFixture.result.operation.outcome as NonNullable<import('./contracts').OperationSnapshot['outcome']>, effects: 'conflict', repositoryChanged: false } };
    await act(async () => services.tasks.setState({ snapshot: { streamId: 'stash', sequence: operation.sequence, operations: [operation] } }));
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Reconcile stash' })));
    expect(services.api).toHaveBeenCalledWith('reconcile_stash_action', { scope, operationId: operation.id }); expect(screen.getByText('Resolved externally')).toBeInTheDocument();
    expect(vi.mocked(services.api).mock.calls.some(([command]) => ['prepare_repository_action', 'execute_repository_action'].includes(command))).toBe(false);
    await act(async () => services.history.getState().preferences.setState({ scope: { ...scope, generation: 'staged-refresh' } }));
    expect(screen.getByRole('button', { name: 'Reconcile stash' })).toBeDisabled();
  } finally { view.unmount(); }
});
it('S5 changes commit form reuses the real interaction lease, keeps its draft through manual refresh, and retires on selection disposal',async()=>{
    vi.useFakeTimers();const services=makeServices(),original=vi.mocked(services.api).getMockImplementation()!;
    vi.mocked(services.api).mockImplementation(((command:string,request:Record<string,unknown>)=>command==='prepare_repository_action'?Promise.resolve({ok:true,value:{scope:request.scope,kind:'commit',planId:'commit-preview',expiresAt:'2099-01-01',warnings:[]}}):original(command as never,request as never)) as Api);
    const view=render(<StrictMode><App {...services}/></StrictMode>);
    try{
        await act(async()=>services.store.getState().open());await act(async()=>fireEvent.click(screen.getByRole('button',{name:'Changes'})));
        expect(screen.getByRole('region',{name:'Staged changes'})).toBeInTheDocument();const begin=vi.spyOn(services.store.getState(),'beginInteraction');
        await act(async()=>fireEvent.click(screen.getByRole('button',{name:'Commit staged changes'})));expect(begin).toHaveBeenCalledOnce();
        fireEvent.change(screen.getByLabelText('Commit message'),{target:{value:'keep commit draft'}});await act(async()=>fireEvent.click(screen.getByRole('button',{name:'Prepare'})));
        expect(screen.getByRole('button',{name:'Execute'})).toBeEnabled();const reads=()=>vi.mocked(services.api).mock.calls.filter(([c])=>c==='inspect_repository').length,before=reads();
        await act(async()=>{await vi.advanceTimersByTimeAsync(90_000);window.dispatchEvent(new Event('focus'));});expect(reads()).toBe(before);expect(screen.getByLabelText('Commit message')).toHaveValue('keep commit draft');
        await act(async()=>fireEvent.click(screen.getByRole('button',{name:'Refresh context'})));expect(screen.getByLabelText('Commit message')).toHaveValue('keep commit draft');expect(screen.getByRole('button',{name:'Execute'})).toBeDisabled();expect(vi.mocked(services.api).mock.calls.filter(([c])=>c==='prepare_repository_action')).toHaveLength(1);
        await act(async()=>services.store.getState().forget(services.store.getState().workspace!.activeSelection!.repoId));expect(screen.queryByRole('dialog')).not.toBeInTheDocument();expect(services.changes.getState().draft).toBe('');
    }finally{view.unmount();vi.useRealTimers();}
});
it.each(['Failed','Interrupted'] as const)('S5 keeps queued commit draft and later edited text through %s outcome refresh',async terminal=>{
    const services=makeServices(),original=vi.mocked(services.api).getMockImplementation()!;let operation:import('./contracts').OperationSnapshot|null=null;let sequence='0';
    vi.mocked(services.api).mockImplementation((async(command:string,request:Record<string,unknown>)=>{
        if(command==='prepare_repository_action')return{ok:true,value:{scope:request.scope,kind:'commit',planId:'queued-commit',expiresAt:'2099-01-01',warnings:[]}};
        if(command==='execute_repository_action'){const scope=services.changes.getState().scope!;operation={...changesWire.result.operation,kind:'action',actionKind:'commit',repoId:scope.repoId,worktreeId:scope.worktreeId,state:'Queued',sequence:'1',outcome:undefined} as import('./contracts').OperationSnapshot;sequence='1';return{ok:true,value:operation};}
        if(command==='get_operations')return{ok:true,value:{streamId:'stream',sequence,operations:operation?[operation]:[]}};
        if(command==='get_change_action_outcome')return{ok:true,value:{operation,result:{commitOid:null,treeOid:null,failureReason:terminal==='Failed'?'hookOrNative':'recovery',artifactsRetained:false}}};
        return original(command as never,request as never);
    }) as Api);
    const view=render(<App {...services}/>);
    try{
        await act(async()=>services.store.getState().open());await act(async()=>fireEvent.click(screen.getByRole('button',{name:'Changes'})));await act(async()=>fireEvent.click(screen.getByRole('button',{name:'Commit staged changes'})));
        fireEvent.change(screen.getByLabelText('Commit message'),{target:{value:'original queued draft'}});await act(async()=>fireEvent.click(screen.getByRole('button',{name:'Prepare'})));await act(async()=>fireEvent.click(screen.getByRole('button',{name:'Execute'})));
        expect(services.changes.getState().draft).toBe('original queued draft');await act(async()=>fireEvent.click(screen.getByRole('button',{name:'Commit staged changes'})));fireEvent.change(screen.getByLabelText('Commit message'),{target:{value:'new draft after ACK'}});
        operation={...operation!,state:terminal,sequence:'2',outcome:{effects:terminal==='Failed'?'notApplied':'unknown',recovery:terminal==='Failed'?'none':'inspect',repositoryChanged:terminal==='Interrupted',cancellationRequested:false,observedAt:'2026-10-25T00:00:00Z'}};sequence='2';await act(async()=>services.tasks.getState().sync());await act(async()=>{});
        expect(screen.getByLabelText('Commit message')).toHaveValue('new draft after ACK');expect(services.changes.getState().draft).toBe('new draft after ACK');expect(vi.mocked(services.api)).toHaveBeenCalledWith('get_change_action_outcome',{operationId:operation.id});expect(vi.mocked(services.api).mock.calls.filter(([c])=>c==='prepare_repository_action')).toHaveLength(1);
    }finally{view.unmount();}
});

it('S7 local form holds passive refresh, retains manual-refresh input and invalidates on selection disposal', async () => {
  vi.useFakeTimers();
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'prepare_repository_action') return Promise.resolve({ ok: true, value: { scope: request.scope, kind: 'branchCreate', planId: 'local-plan', expiresAt: '2099-01-01', warnings: [] } });
    return original(command as never, request as never);
  }) as Api);
  const view = render(<StrictMode><App {...services} /></StrictMode>);
  try {
    await act(async () => services.store.getState().open()); await act(async () => {});
    const begin = vi.spyOn(services.store.getState(), 'beginInteraction');
    fireEvent.click(screen.getByRole('button', { name: 'Local actions' })); fireEvent.click(screen.getByRole('button', { name: 'Local actions' })); await act(async () => {});
    expect(begin).toHaveBeenCalledOnce(); expect(screen.getByRole('dialog', { name: 'Local repository actions' })).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('Branch name'), { target: { value: 'keep-this-name' } }); fireEvent.click(screen.getByLabelText('Check out the new branch'));
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare action' })));
    const reads = () => vi.mocked(services.api).mock.calls.filter(([command]) => command === 'inspect_repository').length, before = reads();
    await act(async () => { await vi.advanceTimersByTimeAsync(90_000); window.dispatchEvent(new Event('focus')); });
    expect(reads()).toBe(before); expect(screen.getByRole('button', { name: 'Apply action' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Refresh context' })); await act(async () => {});
    expect(reads()).toBe(before + 1); expect(screen.getByLabelText('Branch name')).toHaveValue('keep-this-name'); expect(screen.getByLabelText('Check out the new branch')).toBeChecked(); expect(screen.queryByRole('button', { name: 'Apply action' })).not.toBeInTheDocument();
    expect(vi.mocked(services.api).mock.calls.filter(([command]) => command === 'prepare_repository_action')).toHaveLength(1);
    await act(async () => services.store.getState().forget(services.store.getState().workspace!.activeSelection!.repoId));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument(); expect(services.actions.getState().scope).toBeNull();
  } finally { view.unmount(); vi.useRealTimers(); }
});
it('opens a real-service workspace response from empty view and forgets without deleting files', async () => {
  const user = userEvent.setup(); render(<App {...makeServices()} />);
  const open = await screen.findByRole('button', { name: 'Open a repository' });
  expect(screen.getByRole('button', { name: 'Clone a repository' })).toBeDisabled();
  await user.click(open);
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  expect(await screen.findByRole('heading', { name: 'Fixture' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: 'main' })).toBeInTheDocument();
  expect(screen.getByRole('table', { name: 'File status' })).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Forget Fixture' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Open a repository' })).toBeInTheDocument());
  expect(screen.queryByRole('table')).not.toBeInTheDocument();
});

it.each(['Overview', 'History'] as const)('keeps %s content quiet during background refresh and aligns manual feedback', async tab => {
  vi.useFakeTimers();
  const services = makeServices();
  const implementation = vi.mocked(services.api).getMockImplementation()!;
  let finish: ((value: unknown) => void) | undefined;
  let defer = false;
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'inspect_repository' && defer) return new Promise<unknown>(resolve => { finish = () => resolve({ ok: true, value: { ...fixture.snapshots[0], ...request } }); });
    return implementation(command as never, request as never);
  }) as Api);
  const view = render(<App {...services} />);
  try {
    await act(async () => { await services.store.getState().open(); });
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: tab })); });
    expect(screen.getByRole(tab === 'Overview' ? 'table' : 'grid', { name: tab === 'Overview' ? 'File status' : 'Commits' })).toBeInTheDocument();
    defer = true;
    await act(async () => { await vi.advanceTimersByTimeAsync(30_000); });
    expect(screen.getByRole(tab === 'Overview' ? 'table' : 'grid', { name: tab === 'Overview' ? 'File status' : 'Commits' })).toBeInTheDocument();
    expect(screen.queryByText('Refreshing…')).not.toBeInTheDocument();
    expect(view.container.querySelector('.workspace-content')).toHaveAttribute('aria-busy', 'false');
    await act(async () => { await vi.advanceTimersByTimeAsync(500); });
    expect(screen.getByText('Refreshing…')).toBeInTheDocument();
    if (tab === 'History') expect(screen.getByText('Refreshing…')).toHaveClass('repository-refresh-status');
    await act(async () => { finish?.(undefined); });
    expect(screen.queryByText('Refreshing…')).not.toBeInTheDocument();
    act(() => { void services.store.getState().refresh(); });
    expect(screen.getByText('Refreshing…')).toBeInTheDocument();
    expect(screen.getByRole(tab === 'Overview' ? 'table' : 'grid', { name: tab === 'Overview' ? 'File status' : 'Commits' })).toBeInTheDocument();
    await act(async () => { finish?.(undefined); });
  } finally { view.unmount(); vi.useRealTimers(); }
});
it('keeps startup failures actionable and safe in the product UI', async () => {
  render(<App {...makeServices(true)} />);
  expect(await screen.findByRole('alert')).toHaveTextContent('Certificate verification failed.');
  expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument();
  expect(screen.queryByText(/backend|IPC|foundation/i)).not.toBeInTheDocument();
});
it('starts task synchronization when a workspace is opened after an initial loading error', async () => {
  const user = userEvent.setup(), services = makeServices(true);
  const start = vi.spyOn(services.tasks.getState(), 'start');
  render(<App {...services} />);
  await screen.findByRole('alert'); expect(start).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Open a repository' }));
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
  expect(await screen.findByRole('table', { name: 'File status' })).toBeInTheDocument();
});

it.each([
  { git: { available: false, version: '2.20.0' }, status: 'Git needs update', diagnostic: 'Git 2.20.0 is unsupported. Git 2.31 or later is required.' },
  { git: { available: false, version: null }, status: 'Git not found', diagnostic: 'Git not found' },
])('distinguishes unsupported Git from a missing installation: $status', async ({ git, status, diagnostic }) => {
  const user = userEvent.setup(); render(<App {...makeServices(false, git)} />);
  await screen.findByRole('button', { name: 'Open a repository' });
  expect(screen.getByText(status)).toBeInTheDocument();
  expect(screen.queryByText('Git ready')).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: /Settings & diagnostics/ }));
  expect(screen.getByRole('region', { name: 'Git diagnostics' })).toHaveTextContent(diagnostic);
  if (git.version) expect(screen.queryByText('Git not found')).not.toBeInTheDocument();
  else expect(screen.queryByText(/unsupported/)).not.toBeInTheDocument();
});

it('opens History by default with typed history data and keeps Overview/status available', async () => {
  const services = makeServices(), implementation = vi.mocked(services.api).getMockImplementation()!;
  let failInspection = false;
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'inspect_repository' && failInspection) return Promise.resolve(fixture.error);
    if (command === 'open_history') {
      const context = { scope: request.scope, snapshotId: 'empty-history' };
      return Promise.resolve({ ok: true, value: { context, sessionId: 'empty-session', head: { kind: 'unborn', branch: 'main', oid: null }, refs: [], roots: [], page: { context, sessionId: 'empty-session', commits: [], nextCursor: null } } });
    }
    return implementation(command as never, request as never);
  }) as Api);
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
  const canvas = vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
  const view = render(<App {...services} history={createHistoryStore(services.api)} />), user = userEvent.setup();
  try {
    await user.click(await screen.findByRole('button', { name: 'Open a repository' }));
    expect(await screen.findByText('No commits in the current view.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'History' })).toHaveAttribute('aria-current', 'page');
    expect(screen.queryByText('HEAD has no commits.')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'References' }));
    expect(screen.getByText('HEAD has no commits.')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Close References' }));
    expect(vi.mocked(services.api).mock.calls.filter(([command]) => command === 'open_history')).toHaveLength(1);
    failInspection = true;
    await user.click(screen.getByRole('button', { name: 'Refresh repository' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Certificate verification failed.');
    expect(screen.getByText('No commits in the current view.')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Overview' }));
    expect(await screen.findByRole('table', { name: 'File status' })).toBeInTheDocument();
  } finally { view.unmount(); canvas.mockRestore(); vi.unstubAllGlobals(); }
});

it('finishes pending and newer preference saves before an explicit repository selection', async () => {
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  const saves: { request: Record<string, unknown>; resolve: (result: unknown) => void }[] = [];
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'save_repository_view') return new Promise<unknown>(resolve => { saves.push({ request, resolve }); });
    return original(command as never, request as never);
  }) as Api);
  const history = createHistoryStore(services.api), selected = vi.spyOn(services.store.getState(), 'select');
  const view = render(<App {...services} history={history} />);
  fireEvent.click(await screen.findByRole('button', { name: 'Open a repository' }));
  await screen.findByRole('button', { name: 'Select Fixture' });
  await waitFor(() => expect(history.getState().preferences.getState().hydrated).toBe(true));
  act(() => history.getState().preferences.getState().patch({ sidebarWidth: 450 }));
  fireEvent.click(screen.getByRole('button', { name: 'Select Fixture' }));
  await waitFor(() => expect(saves).toHaveLength(1)); expect(selected).not.toHaveBeenCalled();
  act(() => history.getState().preferences.getState().patch({ sidebarWidth: 500 }));
  await act(async () => saves[0].resolve({ ok: true, value: { scope: saves[0].request.scope, viewRevision: '1', view: saves[0].request.view } }));
  await waitFor(() => expect(saves).toHaveLength(2)); expect(selected).not.toHaveBeenCalled();
  expect(saves[1].request.view).toMatchObject({ sidebarWidth: 500 });
  await act(async () => saves[1].resolve({ ok: true, value: { scope: saves[1].request.scope, viewRevision: '2', view: saves[1].request.view } }));
  await waitFor(() => expect(selected).toHaveBeenCalledOnce());
  view.unmount();
});

it('keeps Overview and a concurrent repository transition behind pending and newer preference acknowledgements', async () => {
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  const saves: { request: Record<string, unknown>; resolve: (result: unknown) => void }[] = [];
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'save_repository_view') return new Promise<unknown>(resolve => { saves.push({ request, resolve }); });
    return original(command as never, request as never);
  }) as Api);
  const history = createHistoryStore(services.api), selected = vi.spyOn(services.store.getState(), 'select');
  const view = render(<App {...services} history={history} />);
  fireEvent.click(await screen.findByRole('button', { name: 'Open a repository' }));
  await screen.findByRole('button', { name: 'Select Fixture' });
  await waitFor(() => expect(history.getState().preferences.getState().hydrated).toBe(true));
  act(() => history.getState().preferences.getState().patch({ sidebarWidth: 400 }));
  fireEvent.click(screen.getByRole('button', { name: 'Overview' }));
  await waitFor(() => expect(saves).toHaveLength(1));
  expect(screen.getByRole('button', { name: 'History' })).toHaveAttribute('aria-current', 'page');
  act(() => history.getState().preferences.getState().patch({ sidebarWidth: 500 }));
  fireEvent.click(screen.getByRole('button', { name: 'Select Fixture' }));
  expect(selected).not.toHaveBeenCalled();
  await act(async () => saves[0].resolve({ ok: true, value: { scope: saves[0].request.scope, viewRevision: '1', view: saves[0].request.view } }));
  await waitFor(() => expect(saves).toHaveLength(2)); expect(selected).not.toHaveBeenCalled();
  expect(screen.getByRole('button', { name: 'History' })).toHaveAttribute('aria-current', 'page');
  expect(saves[1].request.view).toMatchObject({ sidebarWidth: 500 });
  await act(async () => saves[1].resolve({ ok: true, value: { scope: saves[1].request.scope, viewRevision: '2', view: saves[1].request.view } }));
  await waitFor(() => expect(selected).toHaveBeenCalledOnce());
  expect(screen.getByRole('button', { name: 'Overview' })).toHaveAttribute('aria-current', 'page');
  expect(history.getState().preferences.getState().hydrated).toBe(true);
  view.unmount();
});

it('has one StrictMode owner across tabs, preserves preferences and disposes the watcher on unmount', async () => {
  const services = makeServices(), history = services.history;
  const view = render(<StrictMode><App {...services} /></StrictMode>), user = userEvent.setup();
  await user.click(await screen.findByRole('button', { name: 'Open a repository' }));
  await screen.findByText('No commits in the current view.');
  const scope = history.getState().scope;
  act(() => history.getState().preferences.getState().patch({ sidebarWidth: 450, detailsHeight: 420, diffLayout: 'inline' }));
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  expect(history.getState().scope).toBe(scope); expect(history.getState().preferences.getState().hydrated).toBe(true);
  await user.click(screen.getByRole('button', { name: 'History' }));
  expect(history.getState().scope).toBe(scope); expect(history.getState().preferences.getState().view).toMatchObject({ sidebarWidth: 450, detailsHeight: 420, diffLayout: 'inline' });
  const count = (name: string) => vi.mocked(services.api).mock.calls.filter(([command]) => command === name).length;
  expect(count('get_workspace')).toBe(1); expect(count('get_repository_view')).toBe(1); expect(count('open_history')).toBe(1); expect(count('watch_repository')).toBe(1);
  view.unmount(); await act(async () => {});
  expect(history.getState().scope).toBeNull(); expect(count('unwatch_repository')).toBe(1);
});

it('AC2/AC8 refreshes changed action outcomes once for the matching repo including linked worktrees', async () => {
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'get_action_outcome') return Promise.resolve({ ok: true, value: { operation: services.tasks.getState().snapshot!.operations.find(value => value.id === request.operationId)!, fetch: null } });
    return original(command as never, request as never);
  }) as Api);
  const view = render(<App {...services} />);
  await act(async () => services.store.getState().open());
  await screen.findByRole('grid', { name: 'Commits' });
  const repoId = services.store.getState().workspace!.activeSelection!.repoId;
  const refresh = vi.spyOn(services.store.getState(), 'requestRefresh').mockResolvedValue();
  const reload = vi.spyOn(services.store.getState(), 'reload').mockResolvedValue();
  const action = { ...fixture.event.operation, id: 'partial-fetch', sequence: '5', kind: 'action' as const, actionKind: 'fetch' as const, state: 'Failed' as const, repoId, worktreeId: 'different-linked-worktree',
    outcome: { effects: 'partial' as const, recovery: 'inspect' as const, cancellationRequested: true, repositoryChanged: true, observedAt: '2026-01-01' } } as unknown as import('./contracts').OperationSnapshot;
  act(() => services.tasks.setState({ snapshot: { streamId: 'stream', sequence: '5', operations: [action] } }));
  await waitFor(() => expect(refresh).toHaveBeenCalledTimes(1)); expect(refresh).toHaveBeenCalledWith('operation'); expect(reload).not.toHaveBeenCalled();
  act(() => services.tasks.setState({ snapshot: { streamId: 'stream', sequence: '5', operations: [{ ...action }] } }));
  await act(async () => {}); expect(refresh).toHaveBeenCalledTimes(1);
  const other = { ...action, id: 'other-repo-action', sequence: '6', repoId: 'different-repo', outcome: { ...action.outcome!, effects: 'unknown' as const } };
  act(() => services.tasks.setState({ snapshot: { streamId: 'stream', sequence: '6', operations: [action, other] } }));
  await act(async () => {}); expect(refresh).toHaveBeenCalledTimes(1);
  const unknown = { ...other, id: 'unknown-matching', sequence: '7', repoId };
  act(() => services.tasks.setState({ snapshot: { streamId: 'stream', sequence: '7', operations: [action, unknown] } }));
  await waitFor(() => expect(refresh).toHaveBeenCalledTimes(2));
  const clone = { ...fixture.event.operation, id: 'clone-done', sequence: '8', state: 'Succeeded' as const } as unknown as import('./contracts').OperationSnapshot;
  act(() => services.tasks.setState({ snapshot: { streamId: 'stream', sequence: '8', operations: [action, unknown, clone] } }));
  await waitFor(() => expect(reload).toHaveBeenCalledOnce()); expect(refresh).toHaveBeenCalledTimes(2);
  view.unmount();
});

it('S3-CACHE01 retires App bookkeeping with authoritative IDs without repeating surviving completion on rotation', async () => {
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  let next: import('./contracts').OperationsSnapshot = { streamId: 'initial', sequence: '0', operations: [] };
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'get_operations') return Promise.resolve({ ok: true, value: next });
    if (command === 'get_action_outcome') return Promise.resolve({ ok: true, value: { operation: next.operations.find(value => value.id === request.operationId)!, fetch: null } });
    return original(command as never, request as never);
  }) as Api);
  const view = render(<App {...services} />);
  await act(async () => services.store.getState().open()); await screen.findByRole('grid', { name: 'Commits' });
  const refresh = vi.spyOn(services.store.getState(), 'requestRefresh').mockResolvedValue(), reload = vi.spyOn(services.store.getState(), 'reload').mockResolvedValue();
  const action = { ...fixture.event.operation, id: 'retired-action', sequence: '5', kind: 'action', actionKind: 'fetch', state: 'Failed', repoId: services.store.getState().workspace!.activeSelection!.repoId,
    outcome: { effects: 'unknown', recovery: 'inspect', cancellationRequested: false, repositoryChanged: true, observedAt: '2026-01-01' } } as import('./contracts').OperationSnapshot;
  const clone = { ...fixture.event.operation, id: 'surviving-clone', sequence: '6', state: 'Succeeded' } as import('./contracts').OperationSnapshot;
  const publish = async (streamId: string, sequence: string, operations: import('./contracts').OperationSnapshot[]) => {
    next = { streamId, sequence, operations }; await act(async () => services.tasks.getState().sync());
  };
  const reads = () => vi.mocked(services.api).mock.calls.filter(([command]) => command === 'get_action_outcome').length;
  await publish('first', '10', [action, clone]);
  await waitFor(() => expect(reload).toHaveBeenCalledTimes(1)); expect(refresh).not.toHaveBeenCalled(); expect(reads()).toBe(1);
  await publish('compaction-rotation', '11', [action, clone]);
  expect(reload).toHaveBeenCalledTimes(1); expect(refresh).not.toHaveBeenCalled(); expect(reads()).toBe(1);
  await publish('retire-action', '12', [clone]); expect(services.tasks.getState().outcomes).toEqual({});
  await publish('reintroduce-action', '13', [action, clone]);
  await waitFor(() => expect(refresh).toHaveBeenCalledTimes(1)); expect(reload).toHaveBeenCalledTimes(1); expect(reads()).toBe(2);
  await publish('retire-all', '14', []);
  await publish('reintroduce-both', '15', [action, clone]);
  await waitFor(() => expect(reload).toHaveBeenCalledTimes(2)); expect(reads()).toBe(3); expect(refresh).toHaveBeenCalledTimes(1);
  view.unmount();
});

it('S3-FORM01 keeps delayed prepare, remote selection and prune through ordinary fallback ticks', async () => {
  vi.useFakeTimers();
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  const daily = (await import('../fixtures/contracts/daily-wire.json')).default;
  let finishPrepare!: (value: unknown) => void;
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'get_repository_actions') return Promise.resolve({ ok: true, value: { ...daily.catalog, scope: request.scope } });
    if (command === 'prepare_repository_action') return new Promise<unknown>(resolve => { finishPrepare = resolve; });
    return original(command as never, request as never);
  }) as Api);
  const view = render(<App {...services} />);
  try {
    await act(async () => services.store.getState().open()); await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: 'Fetch…' })); await act(async () => {});
    fireEvent.click(screen.getByRole('checkbox', { name: 'Fetch origin' }));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Prune deleted remote-tracking branches' }));
    fireEvent.click(screen.getByRole('button', { name: 'Prepare fetch' }));
    const reads = vi.mocked(services.api).mock.calls.filter(([command]) => command === 'inspect_repository').length;
    await act(async () => { await vi.advanceTimersByTimeAsync(60_000); window.dispatchEvent(new Event('focus')); await vi.advanceTimersByTimeAsync(200); });
    expect(screen.getByRole('dialog', { name: 'Fetch remotes' })).toBeInTheDocument();
    expect(screen.getByRole('checkbox', { name: 'Fetch origin' })).toBeChecked();
    expect(screen.getByRole('checkbox', { name: 'Prune deleted remote-tracking branches' })).toBeChecked();
    expect(vi.mocked(services.api).mock.calls.filter(([command]) => command === 'inspect_repository')).toHaveLength(reads);
    await act(async () => finishPrepare({ ok: false, error: { code: 'ExternalChange', phase: 'inspection', retry: 'retry', message: 'Repository changed externally.' } }));
    expect(screen.getByRole('alert')).toHaveTextContent('Repository changed externally.');
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' })); await act(async () => {});
    expect(vi.mocked(services.api).mock.calls.filter(([command]) => command === 'inspect_repository')).toHaveLength(reads + 1);
  } finally { finishPrepare?.({ ok: false, error: { code: 'Internal', phase: 'inspection', retry: 'none', message: 'Stopped test' } }); view.unmount(); vi.useRealTimers(); }
});

it('S3-FORM01 opening during quiet refresh waits for refreshed history and catalog', async () => {
  vi.useFakeTimers();
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  const daily = (await import('../fixtures/contracts/daily-wire.json')).default;
  let delayInspection = false, delayCatalog = false;
  let finishInspection!: (value: unknown) => void, finishCatalog!: (value: unknown) => void;
  let inspectionRequest!: Record<string, unknown>, catalogRequest!: Record<string, unknown>;
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'inspect_repository' && delayInspection) { inspectionRequest = request; return new Promise<unknown>(resolve => { finishInspection = resolve; }); }
    if (command === 'get_repository_actions') {
      catalogRequest = request;
      if (delayCatalog) return new Promise<unknown>(resolve => { finishCatalog = resolve; });
      return Promise.resolve({ ok: true, value: { ...daily.catalog, scope: request.scope } });
    }
    return original(command as never, request as never);
  }) as Api);
  const view = render(<App {...services} />);
  try {
    await act(async () => services.store.getState().open()); await act(async () => {});
    delayInspection = true; delayCatalog = true;
    let refreshing!: Promise<void>; act(() => { refreshing = services.store.getState().requestRefresh('watch'); });
    expect(screen.queryByText('Refreshing…')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Fetch…' })); await act(async () => {});
    expect(screen.queryByRole('dialog', { name: 'Fetch remotes' })).not.toBeInTheDocument();
    await act(async () => { finishInspection({ ok: true, value: { ...fixture.snapshots[0], ...inspectionRequest } }); await refreshing; });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await act(async () => finishCatalog({ ok: true, value: { ...daily.catalog, scope: catalogRequest.scope } }));
    expect(screen.getByRole('dialog', { name: 'Fetch remotes' })).toBeInTheDocument();
  } finally { view.unmount(); vi.useRealTimers(); }
});

it.each(['manual', 'operation'] as const)('S3-FORM01 %s invalidation keeps form/input, stales preview and resolves fresh handles only on explicit prepare', async reason => {
  vi.useFakeTimers();
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  const daily = (await import('../fixtures/contracts/daily-wire.json')).default;
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    const scope = request.scope as import('./contracts').HistoryScope;
    if (command === 'get_repository_actions') return Promise.resolve({ ok: true, value: { ...daily.catalog, scope, snapshotId: `catalog-${scope.generation}`, remotes: daily.catalog.remotes.map(remote => ({ ...remote, handle: `remote-${scope.generation}-${remote.name}` })) } });
    if (command === 'prepare_repository_action') return Promise.resolve({ ok: true, value: { ...daily.prepared, kind: 'fetch', scope, expiresAt: '2099-01-01T00:00:00Z' } });
    return original(command as never, request as never);
  }) as Api);
  const view = render(<App {...services} />);
  try {
    await act(async () => services.store.getState().open()); await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: 'Fetch…' })); await act(async () => {});
    fireEvent.click(screen.getByRole('checkbox', { name: 'Fetch origin' })); fireEvent.click(screen.getByRole('checkbox', { name: 'Prune deleted remote-tracking branches' }));
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare fetch' })));
    expect(screen.getByRole('button', { name: 'Start fetch' })).toBeInTheDocument();
    const priorScope = services.sync.getState().scope;
    if (reason === 'manual') { fireEvent.click(screen.getByRole('button', { name: 'Refresh context' })); await act(async () => {}); }
    else await act(async () => services.store.getState().requestRefresh('operation'));
    expect(services.sync.getState().scope).not.toEqual(priorScope);
    expect(screen.getByRole('dialog', { name: 'Fetch remotes' })).toBeInTheDocument();
    expect(screen.getByText('Repository refreshed. Prepare again.')).toBeInTheDocument();
    expect(screen.getByRole('checkbox', { name: 'Fetch origin' })).toBeChecked(); expect(screen.getByRole('checkbox', { name: 'Prune deleted remote-tracking branches' })).toBeChecked();
    expect(screen.queryByRole('button', { name: 'Start fetch' })).not.toBeInTheDocument();
    const preparations = () => vi.mocked(services.api).mock.calls.filter(([command]) => command === 'prepare_repository_action');
    expect(preparations()).toHaveLength(1);
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare fetch' })));
    expect(preparations()).toHaveLength(2);
    expect(preparations()[1][1]).toEqual({ scope: services.sync.getState().scope, intent: { kind: 'fetch', remoteHandles: [`remote-${services.sync.getState().scope!.generation}-origin`], prune: true } });
    expect(vi.mocked(services.api).mock.calls.some(([command]) => command === 'execute_repository_action')).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
  } finally { view.unmount(); vi.useRealTimers(); }
});

it('S3-FORM01 double-open/cancel/unmount cannot reopen a form from late readiness or leak the hold', async () => {
  vi.useFakeTimers();
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  const daily = (await import('../fixtures/contracts/daily-wire.json')).default;
  let gate = false, resolveInspection!: (value: unknown) => void, inspection!: Record<string, unknown>;
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'get_repository_actions') return Promise.resolve({ ok: true, value: { ...daily.catalog, scope: request.scope } });
    if (command === 'inspect_repository' && gate) { inspection = request; return new Promise<unknown>(resolve => { resolveInspection = resolve; }); }
    return original(command as never, request as never);
  }) as Api);
  const view = render(<StrictMode><App {...services} /></StrictMode>);
  try {
    await act(async () => services.store.getState().open()); await act(async () => {});
    gate = true; let work!: Promise<void>; act(() => { work = services.store.getState().requestRefresh('watch'); });
    const begin = vi.spyOn(services.store.getState(), 'beginInteraction');
    fireEvent.click(screen.getByRole('button', { name: 'Fetch…' })); fireEvent.click(screen.getByRole('button', { name: 'Fetch…' }));
    expect(begin).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button', { name: 'Cancel opening fetch' })); await act(async () => {});
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await act(async () => { resolveInspection({ ok: true, value: { ...fixture.snapshots[0], ...inspection } }); await work; });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    const lease = services.store.getState().beginInteraction()!; expect(await lease.ready).toBe(true); lease.release();
    gate = false; fireEvent.click(screen.getByRole('button', { name: 'Fetch…' })); await act(async () => {});
    expect(screen.getByRole('dialog')).toBeInTheDocument(); view.unmount(); await act(async () => {});
    expect(services.history.getState().scope).toBeNull();
  } finally { view.unmount(); vi.useRealTimers(); }
});

it.each(['accepted', 'lost'] as const)('S3-FORM01 releases deferred reads on %s execute reply or explicit close without retry', async reply => {
  vi.useFakeTimers();
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  const daily = (await import('../fixtures/contracts/daily-wire.json')).default;
  let finishExecute!: (value: unknown) => void;
  vi.mocked(services.api).mockImplementation(((command: string, request: Record<string, unknown>) => {
    if (command === 'get_repository_actions') return Promise.resolve({ ok: true, value: { ...daily.catalog, scope: request.scope } });
    if (command === 'prepare_repository_action') return Promise.resolve({ ok: true, value: { ...daily.prepared, kind: 'fetch', scope: request.scope, expiresAt: '2099-01-01T00:00:00Z' } });
    if (command === 'execute_repository_action') return new Promise<unknown>(resolve => { finishExecute = resolve; });
    return original(command as never, request as never);
  }) as Api);
  const view = render(<App {...services} />);
  try {
    await act(async () => services.store.getState().open()); await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: 'Fetch…' })); await act(async () => {});
    fireEvent.click(screen.getByRole('checkbox', { name: 'Fetch origin' }));
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare fetch' })));
    const scope = services.sync.getState().scope!, reads = vi.mocked(services.api).mock.calls.filter(([command]) => command === 'inspect_repository').length;
    fireEvent.click(screen.getByRole('button', { name: 'Start fetch' }));
    await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
    expect(vi.mocked(services.api).mock.calls.filter(([command]) => command === 'inspect_repository')).toHaveLength(reads);
    await act(async () => finishExecute(reply === 'accepted' ? { ok: true, value: { ...daily.operation, kind: 'action', actionKind: 'fetch', repoId: scope.repoId, worktreeId: scope.worktreeId } }
      : { ok: false, error: { code: 'Internal', phase: 'transfer', retry: 'none', message: 'Execute reply unavailable.' } }));
    if (reply === 'lost') {
      expect(screen.getByRole('dialog', { name: 'Fetch remotes' })).toBeInTheDocument();
      expect(screen.getByText(/The fetch reply was not confirmed/)).toBeInTheDocument();
      expect(vi.mocked(services.api).mock.calls.filter(([command]) => command === 'inspect_repository')).toHaveLength(reads);
      fireEvent.click(screen.getByRole('button', { name: 'Cancel' })); await act(async () => {});
    }
    expect(screen.queryByRole('dialog', { name: 'Fetch remotes' })).not.toBeInTheDocument();
    expect(vi.mocked(services.api).mock.calls.filter(([command]) => command === 'inspect_repository')).toHaveLength(reads + 1);
    expect(vi.mocked(services.api).mock.calls.filter(([command]) => command === 'execute_repository_action')).toHaveLength(1);
    expect(vi.mocked(services.api).mock.calls.filter(([command]) => command === 'prepare_repository_action')).toHaveLength(1);
  } finally { view.unmount(); vi.useRealTimers(); }
});

function remoteServices() {
  const services = makeServices(), original = vi.mocked(services.api).getMockImplementation()!;
  let operations: import('./contracts').OperationSnapshot[] = [], sequence = 0, lastKind: import('./contracts').ActionKind = 'readRemoteRefs';
  let captured: import('./contracts').ActionScope | null = null, losePushReply = false;
  vi.mocked(services.api).mockImplementation((async (command: string, request: Record<string, unknown>) => {
    if (command === 'get_repository_actions') return { ok: true, value: { scope: request.scope, snapshotId: 'sync', observedAt: '', head: { kind: 'branch', branch: 'main', oid: 'a'.repeat(40) }, branches: [], symbolicBranches: [], remotes: [{ name: 'origin', handle: 'fetch-handle', fetch: 'eligibleProbeRequired' }], sync: { currentBranch: 'refs/heads/main', upstream: 'refs/remotes/origin/main', ahead: '1', behind: '0', completeness: 'complete' } } };
    if (command === 'get_remote_actions') return { ok: true, value: { scope: request.scope, observedAt: '', remotes: [{ name: 'origin', endpoints: [{ role: 'push', handle: 'push-handle', capability: 'eligible' }, { role: 'fetch', handle: 'fetch-handle', capability: 'eligible' }] }] } };
    if (command === 'get_publication_evidence') return { ok: true, value: { scope: request.scope, observations: [] } };
    if (command === 'resolve_ref_action_target') return { ok: true, value: { ...refFixture.resolved, scope: request.scope, catalogId: request.catalogId } };
    if (command === 'get_operations') return { ok: true, value: { streamId: 'remote-tasks', sequence: String(sequence), operations } };
    if (command === 'prepare_repository_action') { captured = request.scope as import('./contracts').ActionScope; lastKind = (request.intent as import('./contracts').ActionIntent).kind; return { ok: true, value: { scope: captured, kind: lastKind, planId: lastKind, expiresAt: '2099-01-01', warnings: [] } }; }
    if (command === 'execute_repository_action') {
      if (lastKind === 'push' && losePushReply) return { ok: false, error: { code: 'Internal', phase: 'transfer', retry: 'none', message: 'Push reply unavailable.' } };
      const value: import('./contracts').OperationSnapshot = { ...refFixture.result.operation as import('./contracts').OperationSnapshot, id: lastKind, sequence: String(++sequence), actionKind: lastKind, state: 'Queued', repoId: captured!.repoId, worktreeId: captured!.worktreeId, error: null, authPrompt: null, outcome: undefined };
      operations = [...operations.filter(operation => operation.id !== value.id), value]; return { ok: true, value };
    }
    if (command === 'get_remote_action_outcome') return { ok: true, value: { operation: operations.find(operation => operation.id === request.operationId), availability: 'available', evidence: { remoteName: 'origin', role: 'push', phase: 'observed', refCount: '1', observedAt: '2026-10-25', noGitWrites: true }, snapshot: { capturedScope: captured, snapshotId: 'read-observation', remoteName: 'origin', role: 'push', observedAt: '2026-10-25', expiresAt: '2099-01-01', objectFormat: 'sha1', refs: [{ entryId: 'destination-main', name: 'refs/heads/main', oid: 'a'.repeat(40) }] } } };
    if (command === 'resolve_remote_target') return { ok: true, value: { scope: request.scope, snapshotId: request.snapshotId, handle: 'observed-main', namespace: 'branch', name: 'refs/heads/main', expectedOid: 'a'.repeat(40) } };
    return original(command as never, request as never);
  }) as Api);
  const readState = async (state: 'WaitingForUser' | 'Succeeded') => {
    operations = operations.map(operation => operation.id !== 'readRemoteRefs' ? operation : { ...operation, state, sequence: String(++sequence), authPrompt: state === 'WaitingForUser' ? { promptHandle: 'read-prompt', kind: 'password', message: 'Fixture authentication required.', expiresAt: '2099-01-01' } : null, outcome: state === 'Succeeded' ? { effects: 'notApplied', recovery: 'none', cancellationRequested: false, repositoryChanged: false, observedAt: '2026-10-25' } : undefined });
    await services.tasks.getState().sync();
  };
  return { ...services, readState, loseReply: () => { losePushReply = true; } };
}
it.each(['accepted', 'lost'] as const)('Sync App uses authenticated read Tasks and keeps its lease through %s push response without retries', async reply => {
  vi.useFakeTimers(); const services = remoteServices(), view = render(<StrictMode><App {...services} /></StrictMode>);
  try {
    await act(async () => services.store.getState().open()); await act(async () => {});
    const begin = vi.spyOn(services.store.getState(), 'beginInteraction');
    fireEvent.click(screen.getByRole('button', { name: 'Push…' })); fireEvent.click(screen.getByRole('button', { name: 'Push…' })); await act(async () => {});
    expect(begin).toHaveBeenCalledOnce(); expect(screen.getByRole('dialog', { name: 'Remote synchronization' })).toBeInTheDocument();
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Read remote references' })));
    await act(async () => services.readState('WaitingForUser'));
    expect(screen.getByRole('dialog', { name: 'Repository authentication' })).toBeInTheDocument(); expect(screen.getByRole('button', { name: 'Prepare' })).toBeDisabled();
    const reads = () => vi.mocked(services.api).mock.calls.filter(([command]) => command === 'inspect_repository').length, before = reads();
    await act(async () => { await vi.advanceTimersByTimeAsync(60_000); window.dispatchEvent(new Event('focus')); }); expect(reads()).toBe(before);
    await act(async () => services.readState('Succeeded'));
    expect(screen.queryByRole('dialog', { name: 'Repository authentication' })).not.toBeInTheDocument(); expect(screen.getByLabelText('Destination')).toHaveValue('ref:refs/heads/main');
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' })));
    expect(screen.getByRole('region', { name: 'Remote action preview' })).toBeInTheDocument(); if (reply === 'lost') services.loseReply();
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Execute' })); fireEvent.click(screen.getByRole('button', { name: 'Execute' })); });
    if (reply === 'lost') { expect(screen.getByRole('dialog', { name: 'Remote synchronization' })).toBeInTheDocument(); expect(screen.getByText(/Execution reply was not confirmed/)).toBeInTheDocument(); fireEvent.click(screen.getByRole('button', { name: 'Close' })); await act(async () => {}); }
    expect(screen.queryByRole('dialog', { name: 'Remote synchronization' })).not.toBeInTheDocument(); expect(reads()).toBe(before + 1);
    expect(vi.mocked(services.api).mock.calls.filter(([command]) => command === 'execute_repository_action')).toHaveLength(2);
  } finally { view.unmount(); vi.useRealTimers(); }
});
it('Sync App keeps source/remote/name across manual refresh, drops destination authority and retires on forget', async () => {
  const services = remoteServices(), view = render(<App {...services} />);
  try {
    await act(async () => services.store.getState().open()); await act(async () => {});
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Push…' })));
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Read remote references' }))); await act(async () => services.readState('Succeeded'));
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' })));
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Refresh context' })));
    expect(screen.getByRole('dialog', { name: 'Remote synchronization' })).toBeInTheDocument(); expect(screen.getByLabelText('Source branch')).toHaveValue('refs/heads/main'); expect(screen.getByLabelText('Remote')).toHaveValue('origin');
    expect(services.transport.getState().snapshot).toBeNull(); expect(screen.getByRole('button', { name: 'Execute' })).toBeDisabled(); expect(screen.getByRole('button', { name: 'Prepare' })).toBeDisabled();
    await act(async () => services.store.getState().forget(services.store.getState().workspace!.activeSelection!.repoId));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument(); expect(services.transport.getState().scope).toBeNull();
  } finally { view.unmount(); }
});
