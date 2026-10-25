import { expect, it, vi } from 'vitest';
import { createRepositoryChangesStore } from './repositoryChanges';
import { createHistoryStore } from './history';
import { defaultRepositoryView } from './historyView';
import type { Api } from './workspace';
import type { ActionIntent, RepositoryChanges } from '../contracts';
const ok = <T,>(value: T) => ({ ok: true as const, value });
const settle = async () => { for (let i = 0; i < 5; i++) await new Promise<void>(resolve => queueMicrotask(resolve)); };
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(r => { resolve = r; }); return { promise, resolve }; }
function setup(extra?: (c: string, r: Record<string, unknown>) => unknown) {
  let serial = 0, intent: ActionIntent;
  const api = vi.fn(async (command: string, request: Record<string, unknown>): Promise<unknown> => {
    const replacement = extra?.(command, request); if (replacement !== undefined) return replacement;
    if (command === 'get_repository_view' || command === 'save_repository_view') return ok({ scope: request.scope, viewRevision: '1', view: request.view ?? defaultRepositoryView() });
    if (command === 'open_history') { const context = { scope: request.scope, snapshotId: `history-${++serial}` }; return ok({ context, sessionId: 'session', head: { kind: 'branch', branch: 'main', oid: 'a'.repeat(40) }, refs: [], roots: [], page: { context, sessionId: 'session', commits: [], nextCursor: null } }); }
    if (command === 'get_repository_changes') return ok({ scope: request.scope, catalogId: `catalog-${serial}`, observedAt: '', unsupported: false, head: { kind: 'branch', branch: 'main', oid: 'a'.repeat(40) }, entries: [{ entryId: 'entry', pathDisplay: '[literal] Ω.txt', previousPathDisplay: null, indexStatus: 'M', worktreeStatus: 'M', stageCapability: 'eligible', unstageCapability: 'eligible' }] });
    if (command === 'resolve_change_targets') return ok({ scope: request.scope, catalogId: request.catalogId, targets: [{ entryId: 'entry', side: (request.targets as { side: string }[])[0].side, changeHandle: 'backend-change' }] });
    if (command === 'get_change_diff') return ok({ scope: request.scope, changeHandle: request.changeHandle, diffId: 'diff', content: '+content', partialEligibility: 'Whole file', hunks: [] });
    if (command === 'prepare_repository_action') { intent = request.intent as ActionIntent; return ok({ scope: request.scope, planId: 'plan', kind: intent.kind, expiresAt: '2099-01-01T00:00:00Z', warnings: [] }); }
    if (command === 'execute_repository_action') return ok({ id: 'operation', kind: 'action', actionKind: intent.kind, repoId: 'repo', worktreeId: 'tree' });
    throw Error(command);
  });
  const history = createHistoryStore(api as unknown as Api), store = createRepositoryChangesStore(api as unknown as Api), stop = store.getState().connect(history);
  const start = async () => { await history.getState().activate({ repoId: 'repo', worktreeId: 'tree' }); await settle(); };
  return { api, history, store, stop, start };
}
it('resolves staged and unstaged authority lazily and sends only handles to stage/unstage', async () => {
  const t = setup(); await t.start();
  try {
    expect(t.api.mock.calls.filter(([c]) => c === 'resolve_change_targets')).toHaveLength(0);
    await t.store.getState().select('entry', 'unstaged'); expect(t.store.getState().diff?.content).toBe('+content');
    for (const [side, kind] of [['unstaged', 'stage'], ['staged', 'unstage']] as const) {
      await t.store.getState().prepareFile('entry', side);
      expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action', { scope: t.store.getState().scope, intent: { kind, changeHandles: ['backend-change'] } });
    }
    expect(JSON.stringify(t.api.mock.calls.filter(([c]) => ['prepare_repository_action', 'execute_repository_action'].includes(c)))).not.toMatch(/"(?:path|patch|oid|environment)":/);
  } finally { t.stop(); await settle(); }
});
it('keeps a memory-only commit draft through same-selection refresh while invalidating previews, and clears it on selection disposal', async () => {
  const t = setup(); await t.start();
  t.store.getState().setDraft('draft without persistence'); await t.store.getState().prepareCommit();
  expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action', { scope: t.store.getState().scope, intent: { kind: 'commit', message: 'draft without persistence', amend: false } });
  await t.history.getState().refresh(); await settle(); expect(t.store.getState().prepared).toBeNull(); expect(t.store.getState().draft).toBe('draft without persistence');
  expect(t.api.mock.calls.filter(([c]) => c === 'prepare_repository_action')).toHaveLength(1);
  await t.history.getState().activate({ repoId: 'other', worktreeId: null }); await settle(); expect(t.store.getState().draft).toBe(''); t.stop(); await settle();
});
it('discards late diff/resolve/prepare replies after catalog refresh, edit and disposal', async () => {
  const pending = deferred<unknown>(); let delay = true;
  const t = setup((c, r) => c === 'resolve_change_targets' && delay ? pending.promise : undefined); await t.start();
  const selecting = t.store.getState().select('entry', 'unstaged'); const initial = t.store.getState().catalog!;
  await t.history.getState().refresh(); await settle(); pending.resolve(ok({ scope: initial.scope, catalogId: initial.catalogId, targets: [{ entryId: 'entry', side: 'unstaged', changeHandle: 'old' }] })); await selecting;
  expect(t.store.getState().diff).toBeNull(); expect(t.api.mock.calls.some(([c]) => c === 'get_change_diff')).toBe(false);
  delay = false; const preview = deferred<unknown>(); const implementation = t.api.getMockImplementation()!;
  t.api.mockImplementation(async (c, r) => c === 'prepare_repository_action' ? preview.promise : implementation(c, r));
  t.store.getState().setDraft('first'); const preparing = t.store.getState().prepareCommit(); const scope = t.store.getState().scope;
  t.store.getState().setDraft('second'); t.stop(); await settle(); preview.resolve(ok({ scope, kind: 'commit', planId: 'stale', expiresAt: '2099-01-01', warnings: [] })); await preparing; expect(t.store.getState().prepared).toBeNull();
});
it.each(['scope', 'catalog', 'side', 'entry'])('rejects mismatched %s resolver authority', async mismatch => {
  const t = setup((c, r) => c === 'resolve_change_targets' ? ok({ scope: mismatch === 'scope' ? { repoId: 'wrong', worktreeId: null, generation: '1' } : r.scope, catalogId: mismatch === 'catalog' ? 'wrong' : r.catalogId, targets: [{ entryId: mismatch === 'entry' ? 'wrong' : 'entry', side: mismatch === 'side' ? 'staged' : 'unstaged', changeHandle: 'handle' }] }) : undefined); await t.start();
  await t.store.getState().prepareFile('entry', 'unstaged'); expect(t.store.getState().actionError?.code).toBe('InvalidHandle'); expect(t.api.mock.calls.some(([c]) => c === 'prepare_repository_action')).toBe(false); t.stop(); await settle();
});
it('submits once, clears preview before await, and never retries a lost execution reply', async () => {
  const pending = deferred<unknown>(); const t = setup(c => c === 'execute_repository_action' ? pending.promise : undefined); await t.start(); t.store.getState().setDraft('message'); await t.store.getState().prepareCommit();
  const first = t.store.getState().execute(); expect(await t.store.getState().execute()).toBeNull(); expect(t.store.getState().prepared).toBeNull();
  pending.resolve({ ok: false, error: { code: 'Internal', phase: 'publish', retry: 'none', message: 'Lost execution reply. Check Tasks.' } }); await first;
  expect(t.store.getState().draft).toBe('message'); expect(t.api.mock.calls.filter(([c]) => c === 'execute_repository_action')).toHaveLength(1); expect(await t.store.getState().execute()).toBeNull(); t.stop(); await settle();
});
it('blocks empty commits and unsupported changes without preparing', async () => {
  const t = setup(); await t.start(); await t.store.getState().prepareCommit(); expect(t.store.getState().actionError?.message).toMatch(/message/);
  const c = t.store.getState().catalog as RepositoryChanges; t.store.setState({ catalog: { ...c, unsupported: true } }); await t.store.getState().prepareFile('entry', 'unstaged'); expect(t.api.mock.calls.some(([command]) => command === 'prepare_repository_action')).toBe(false); t.stop(); await settle();
});
it('keeps exact clicked entry authority through duplicate display labels and catalog reorder, and refuses a removed locator',async()=>{
  let reordered=false;
  const entries=[{entryId:'raw-a',pathDisplay:'same visible label',previousPathDisplay:null,indexStatus:'M',worktreeStatus:'M',stageCapability:'eligible',unstageCapability:'eligible'},{entryId:'raw-b',pathDisplay:'same visible label',previousPathDisplay:null,indexStatus:'M',worktreeStatus:'M',stageCapability:'eligible',unstageCapability:'eligible'}];
  const t=setup((command,r)=>{
    if(command==='get_repository_changes')return ok({scope:r.scope,catalogId:reordered?'reordered':'original',observedAt:'',unsupported:false,head:{kind:'branch',branch:'main',oid:'a'.repeat(40)},entries:reordered?[...entries].reverse():entries});
    if(command==='resolve_change_targets'){const target=(r.targets as {entryId:string;side:string}[])[0];return ok({scope:r.scope,catalogId:r.catalogId,targets:[{...target,changeHandle:`handle-${target.entryId}`} ]});}
    return undefined;
  });await t.start();
  try{
    await t.store.getState().prepareFile('raw-b','unstaged');expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action',{scope:t.store.getState().scope,intent:{kind:'stage',changeHandles:['handle-raw-b']}});
    reordered=true;await t.store.getState().reload();await t.store.getState().prepareFile('raw-b','unstaged');expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action',{scope:t.store.getState().scope,intent:{kind:'stage',changeHandles:['handle-raw-b']}});
    const before=t.api.mock.calls.filter(([c])=>c==='resolve_change_targets').length;const catalog=t.store.getState().catalog!;t.store.setState({catalog:{...catalog,entries:catalog.entries.filter(e=>e.entryId!=='raw-b')}});await t.store.getState().prepareFile('raw-b','unstaged');
    expect(t.api.mock.calls.filter(([c])=>c==='resolve_change_targets')).toHaveLength(before);expect(t.store.getState().prepared).toBeNull();expect(t.store.getState().actionError?.code).toBe('InvalidHandle');
  }finally{t.stop();await settle();}
});
