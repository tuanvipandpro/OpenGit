import { expect, it, vi } from 'vitest';
import { createRepositoryChangesStore } from './repositoryChanges';
import { createHistoryStore } from './history';
import { defaultRepositoryView } from './historyView';
import type { Api } from './workspace';
import type { ActionIntent, ChangeDiff, ChangeSelectionInput, ChangeSide, RepositoryChanges } from '../contracts';
const ok = <T,>(value: T) => ({ ok: true as const, value });
const settle = async () => { for (let i = 0; i < 5; i++) await new Promise<void>(resolve => queueMicrotask(resolve)); };
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(r => { resolve = r; }); return { promise, resolve }; }
function setup(extra?: (c: string, r: Record<string, unknown>) => unknown) {
  let serial = 0, intent: ActionIntent, selectedSide: ChangeSide = 'unstaged';
  const api = vi.fn(async (command: string, request: Record<string, unknown>): Promise<unknown> => {
    const replacement = extra?.(command, request); if (replacement !== undefined) return replacement;
    if (command === 'get_repository_view' || command === 'save_repository_view') return ok({ scope: request.scope, viewRevision: '1', view: request.view ?? defaultRepositoryView() });
    if (command === 'open_history') { const context = { scope: request.scope, snapshotId: `history-${++serial}` }; return ok({ context, sessionId: 'session', head: { kind: 'branch', branch: 'main', oid: 'a'.repeat(40) }, refs: [], roots: [], page: { context, sessionId: 'session', commits: [], nextCursor: null } }); }
    if (command === 'get_repository_changes') return ok({ scope: request.scope, catalogId: `catalog-${serial}`, observedAt: '', unsupported: false, head: { kind: 'branch', branch: 'main', oid: 'a'.repeat(40) }, entries: [{ entryId: 'entry', pathDisplay: '[literal] Ω.txt', previousPathDisplay: null, indexStatus: 'M', worktreeStatus: 'M', stageCapability: 'eligible', unstageCapability: 'eligible' }] });
    if (command === 'resolve_change_targets') { selectedSide = (request.targets as { side: ChangeSide }[])[0].side; return ok({ scope: request.scope, catalogId: request.catalogId, targets: [{ entryId: 'entry', side: selectedSide, changeHandle: 'backend-change' }] }); }
    if (command === 'get_change_diff') return ok({ scope: request.scope, changeHandle: request.changeHandle, diffId: `diff-${selectedSide}`, side: selectedSide, content: '+content', partialEligibility: 'eligible', hunks: [{ hunkId: 'hunk-1', rows: [{ lineId: null, kind: 'context', oldLine: 1, newLine: 1, text: 'same', hasLf: true }, { lineId: 'delete-2', kind: 'delete', oldLine: 2, newLine: null, text: 'old', hasLf: true }, { lineId: 'add-2', kind: 'add', oldLine: null, newLine: 2, text: 'new', hasLf: true }] }, { hunkId: 'hunk-2', rows: [{ lineId: 'add-8', kind: 'add', oldLine: null, newLine: 8, text: 'tail', hasLf: false }] }] });
    if (command === 'resolve_change_selection') return ok({ scope: request.scope, changeHandle: request.changeHandle, diffId: request.diffId, selectionHandle: 'backend-selection' });
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
it.each(['unstaged', 'staged'] as const)('S5-B prepares selected lines from the exact %s diff without reminting its source', async side => {
  const t = setup(); await t.start();
  try {
    await t.store.getState().select('entry', side); const diff = t.store.getState().diff!;
    t.store.getState().setSelection(diff, { kind: 'lines', lineIds: ['delete-2', 'add-2'] });
    const sourceReads = t.api.mock.calls.filter(([command]) => ['resolve_change_targets', 'get_change_diff'].includes(command)).length;
    await t.store.getState().prepareSelection(diff);
    expect(t.api).toHaveBeenCalledWith('resolve_change_selection', { scope: diff.scope, changeHandle: diff.changeHandle, diffId: diff.diffId, selection: { kind: 'lines', lineIds: ['delete-2', 'add-2'] } });
    expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action', { scope: diff.scope, intent: { kind: side === 'unstaged' ? 'stageSelection' : 'unstageSelection', selectionHandles: ['backend-selection'] } });
    expect(t.api.mock.calls.filter(([command]) => ['resolve_change_targets', 'get_change_diff'].includes(command))).toHaveLength(sourceReads);
    expect(JSON.stringify(t.api.mock.calls.filter(([command]) => command === 'prepare_repository_action'))).not.toMatch(/"(?:path|patch|oid|lineIds|hunkIds|environment)":/);
  } finally { t.stop(); await settle(); }
});
it('S5-B submits hunk IDs, clears opposite selection modes and invalidates selection on file/side/refresh', async () => {
  const t = setup(); await t.start();
  try {
    await t.store.getState().select('entry', 'unstaged'); let diff = t.store.getState().diff!;
    t.store.getState().setSelection(diff, { kind: 'lines', lineIds: ['add-2'] }); t.store.getState().setSelection(diff, { kind: 'hunks', hunkIds: ['hunk-1'] });
    expect(t.store.getState().selection).toEqual({ kind: 'hunks', hunkIds: ['hunk-1'] }); await t.store.getState().prepareSelection(diff);
    expect(t.api).toHaveBeenCalledWith('resolve_change_selection', { scope: diff.scope, changeHandle: diff.changeHandle, diffId: diff.diffId, selection: { kind: 'hunks', hunkIds: ['hunk-1'] } });
    await t.store.getState().select('entry', 'staged'); expect(t.store.getState().selection).toBeNull(); expect(t.store.getState().prepared).toBeNull();
    diff = t.store.getState().diff!; t.store.getState().setSelection(diff, { kind: 'lines', lineIds: ['add-2'] });
    await t.history.getState().refresh(); await settle(); expect(t.store.getState().selection).toBeNull(); expect(t.store.getState().diff).toBeNull();
  } finally { t.stop(); await settle(); }
});
it.each(['scope', 'changeHandle', 'diffId', 'side'] as const)('S5-B refuses a partial selection with mismatched %s fence', async field => {
  const t = setup(); await t.start();
  try {
    await t.store.getState().select('entry', 'unstaged'); const diff = t.store.getState().diff!;
    t.store.getState().setSelection(diff, { kind: 'lines', lineIds: ['add-2'] });
    const fence = { ...diff, ...(field === 'scope' ? { scope: { ...diff.scope, generation: 'old' } } : field === 'side' ? { side: 'staged' as const } : { [field]: 'wrong' }) };
    await t.store.getState().prepareSelection(fence); expect(t.store.getState().prepared).toBeNull(); expect(t.api.mock.calls.some(([command]) => command === 'resolve_change_selection')).toBe(false);
  } finally { t.stop(); await settle(); }
});
it.each(['scope', 'changeHandle', 'diffId'] as const)('S5-B rejects selection resolver echo mismatch: %s', async field => {
  const t = setup((command, request) => command === 'resolve_change_selection' ? ok({ scope: request.scope, changeHandle: request.changeHandle, diffId: request.diffId, selectionHandle: 'forged', ...(field === 'scope' ? { scope: { repoId: 'wrong', worktreeId: null, generation: '1' } } : { [field]: 'wrong' }) }) : undefined); await t.start();
  try {
    await t.store.getState().select('entry', 'unstaged'); const diff = t.store.getState().diff!; t.store.getState().setSelection(diff, { kind: 'hunks', hunkIds: ['hunk-1'] }); await t.store.getState().prepareSelection(diff);
    expect(t.store.getState().actionError?.message).toMatch(/authority changed/); expect(t.api.mock.calls.some(([command]) => command === 'prepare_repository_action')).toBe(false);
  } finally { t.stop(); await settle(); }
});
it('S5-B rejects foreign, duplicate and empty selections and offers no partial authority for fallback diffs', async () => {
  const t = setup(); await t.start();
  try {
    await t.store.getState().select('entry', 'unstaged'); const diff = t.store.getState().diff!;
    for (const selection of [{ kind: 'lines', lineIds: ['context-row'] }, { kind: 'hunks', hunkIds: ['missing'] }, { kind: 'lines', lineIds: ['add-2', 'add-2'] }] as ChangeSelectionInput[]) { t.store.getState().setSelection(diff, selection); expect(t.store.getState().actionError).not.toBeNull(); expect(t.store.getState().selection).toBeNull(); }
    t.store.getState().setSelection(diff, { kind: 'lines', lineIds: [] }); await t.store.getState().prepareSelection(diff); expect(t.api.mock.calls.some(([command]) => command === 'resolve_change_selection')).toBe(false);
    t.store.setState({ diff: { ...diff, partialEligibility: 'lineEndings' } }); t.store.getState().setSelection(diff, { kind: 'lines', lineIds: ['add-2'] }); expect(t.store.getState().selection).toEqual({ kind: 'lines', lineIds: [] });
  } finally { t.stop(); await settle(); }
});
it('S5-B fences delayed selection grants after edits, refresh and disposal without a replacement source read', async () => {
  const pending = deferred<unknown>(); const t = setup(command => command === 'resolve_change_selection' ? pending.promise : undefined); await t.start();
  await t.store.getState().select('entry', 'unstaged'); const diff = t.store.getState().diff!; t.store.getState().setSelection(diff, { kind: 'lines', lineIds: ['add-2'] });
  const preparing = t.store.getState().prepareSelection(diff); t.store.getState().setSelection(diff, { kind: 'lines', lineIds: ['delete-2'] });
  await t.history.getState().refresh(); await settle(); t.stop(); await settle();
  pending.resolve(ok({ scope: diff.scope, changeHandle: diff.changeHandle, diffId: diff.diffId, selectionHandle: 'old' })); await preparing;
  expect(t.store.getState().prepared).toBeNull(); expect(t.api.mock.calls.some(([command]) => command === 'prepare_repository_action')).toBe(false); expect(t.api.mock.calls.filter(([command]) => command === 'resolve_change_targets')).toHaveLength(1);
});
it('S5-B validates the returned diff side before exposing any selection controls', async () => {
  const t = setup((command, request) => command === 'get_change_diff' ? ok({ scope: request.scope, changeHandle: request.changeHandle, diffId: 'wrong-side', side: 'staged', content: 'opaque', partialEligibility: 'eligible', hunks: [] }) : undefined); await t.start();
  try { await t.store.getState().select('entry', 'unstaged'); expect(t.store.getState().diff).toBeNull(); expect(t.store.getState().error?.message).toMatch(/file and side/); } finally { t.stop(); await settle(); }
});
it('S5-B amend defaults false, supports message-only HEAD and keeps draft/toggle after enqueue ACK or failure', async () => {
  let fail = false; const t = setup(command => command === 'execute_repository_action' && fail ? { ok: false, error: { code: 'Internal', phase: 'publish', retry: 'none', message: 'Hook refused the amendment.' } } : undefined); await t.start();
  try {
    expect(t.store.getState().amend).toBe(false); const catalog = t.store.getState().catalog!; t.store.setState({ catalog: { ...catalog, entries: [] } }); t.store.getState().setDraft('message-only');
    await t.store.getState().prepareCommit(); expect(t.api.mock.calls.some(([command]) => command === 'prepare_repository_action')).toBe(false);
    t.store.getState().setAmend(true); await t.store.getState().prepareCommit(); expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action', { scope: t.store.getState().scope, intent: { kind: 'commit', message: 'message-only', amend: true } });
    await t.store.getState().execute(); expect(t.store.getState().draft).toBe('message-only'); expect(t.store.getState().amend).toBe(true);
    fail = true; await t.store.getState().prepareCommit(); await t.store.getState().execute(); expect(t.store.getState().draft).toBe('message-only'); expect(t.store.getState().amend).toBe(true); expect(t.store.getState().actionError?.message).toMatch(/Hook refused/);
    t.store.setState({ catalog: { ...catalog, head: { kind: 'unborn', branch: 'main', oid: null } } }); await t.store.getState().prepareCommit(); expect(t.store.getState().actionError?.message).toMatch(/existing HEAD/);
  } finally { t.stop(); await settle(); }
});
it('S5-B message/toggle edits invalidate preview, same-selection refresh retains memory and true ABA clears it', async () => {
  const t = setup(); await t.start();
  try {
    t.store.getState().setDraft('draft'); await t.store.getState().prepareCommit(); t.store.getState().setAmend(true); expect(t.store.getState().prepared).toBeNull();
    await t.store.getState().prepareCommit(); t.store.getState().setDraft('new draft'); expect(t.store.getState().prepared).toBeNull();
    await t.history.getState().refresh(); await settle(); expect(t.store.getState().amend).toBe(true); expect(t.store.getState().draft).toBe('new draft');
    const scope = t.store.getState().scope!; t.history.getState().preferences.setState({ scope: { ...scope, repoId: 'other' } }); t.history.getState().preferences.setState({ scope: { ...scope, generation: 'returned' } }); await settle();
    expect(t.store.getState().amend).toBe(false); expect(t.store.getState().draft).toBe('');
  } finally { t.stop(); await settle(); }
});
it('S5B-FORM01 missing stable entry after readiness reports explicit reselection without minting a different target', async () => {
  const t = setup(); await t.start();
  try {
    const catalog = t.store.getState().catalog!; t.store.setState({ catalog: { ...catalog, entries: [] } });
    await t.store.getState().select('entry', 'unstaged'); expect(t.store.getState().error?.message).toMatch(/no longer available/); expect(t.api.mock.calls.some(([command]) => command === 'resolve_change_targets')).toBe(false);
  } finally { t.stop(); await settle(); }
});
