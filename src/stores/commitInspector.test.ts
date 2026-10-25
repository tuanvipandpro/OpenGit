import { expect, it, vi } from 'vitest';
import { createCommitInspectorStore } from './commitInspector';
import type { Api } from './workspace';
import type { CommitDetails, CommitSummary, DiffBase, HistoryContext, CommitFile } from '../contracts';
import type { HistorySelection } from './history';
const context: HistoryContext = { scope: { repoId: 'repo', worktreeId: 'tree', generation: '1' }, snapshotId: 'snapshot' };
const commit: CommitSummary = { oid: 'a'.repeat(40), handle: 'commit-handle', subject: 'Merge', parents: [{ oid: 'b'.repeat(40), boundary: 'offPage' }, { oid: 'c'.repeat(40), boundary: 'offPage' }], authorName: 'Author', authorEmail: 'author@example.test', authoredAt: '', committedAt: '', decorations: [] };
const selected = { context, commit, outsideView: true };
const ok = <T,>(value: T) => ({ ok: true as const, value });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
function details(base: DiffBase, target = commit): CommitDetails { return { context, commit: target, base, message: 'Full message', committerName: 'Committer', committerEmail: 'committer@example.test', files: ['first', 'second'].map(handle => ({ handle, pathDisplay: `${handle}.txt`, previousPathDisplay: null, kind: 'modified', oldMode: '100644', newMode: '100644' })) }; }

it('uses real handles with explicit root/parent modes and rejects invalid merge parent indices', async () => {
  const api = vi.fn(async (command: string, request: { context: HistoryContext; base: DiffBase; fileHandle: string; commitHandle: string }) => command === 'get_commit_details' ? ok(details(request.base, request.commitHandle === 'root-handle' ? { ...commit, handle: 'root-handle', parents: [] } : commit)) : ok({ context: request.context, fileHandle: request.fileHandle, content: { kind: 'unavailable', reason: 'binary' } }));
  const store = createCommitInspectorStore(api as unknown as Api); await store.getState().select(selected);
  expect(api).toHaveBeenCalledWith('get_commit_details', { context, commitHandle: commit.handle, base: { kind: 'parent', index: 0 } });
  await store.getState().setBase({ kind: 'parent', index: 1 }); expect(store.getState().details?.base).toEqual({ kind: 'parent', index: 1 });
  const count = api.mock.calls.length; await store.getState().setBase({ kind: 'parent', index: 2 }); await store.getState().setBase({ kind: 'root' }); expect(api.mock.calls).toHaveLength(count);
  await store.getState().select({ ...selected, commit: { ...commit, handle: 'root-handle', parents: [] } });
  expect(api).toHaveBeenCalledWith('get_commit_details', { context, commitHandle: 'root-handle', base: { kind: 'root' } });
});

it('discards old commit, merge-base and file responses and clears previous text immediately', async () => {
  const requests: { command: string; request: Record<string, unknown>; ready: ReturnType<typeof deferred<unknown>> }[] = [];
  const api = vi.fn((command: string, request: Record<string, unknown>) => { const ready = deferred<unknown>(); requests.push({ command, request, ready }); return ready.promise; });
  const store = createCommitInspectorStore(api as unknown as Api);
  const first = store.getState().select(selected), second = store.getState().setBase({ kind: 'parent', index: 1 });
  requests[1].ready.resolve(ok(details({ kind: 'parent', index: 1 })));
  await vi.waitFor(() => expect(requests).toHaveLength(3));
  const choosing = store.getState().selectFile('second');
  requests[3].ready.resolve(ok({ context, fileHandle: 'second', content: { kind: 'text', original: 'old', modified: 'new', language: null } })); await choosing;
  expect(store.getState().diff?.fileHandle).toBe('second');
  requests[2].ready.resolve(ok({ context, fileHandle: 'first', content: { kind: 'text', original: 'STALE', modified: 'STALE', language: null } }));
  requests[0].ready.resolve(ok(details({ kind: 'parent', index: 0 }))); await first; await second;
  expect(store.getState().details?.base).toEqual({ kind: 'parent', index: 1 }); expect(store.getState().diff?.fileHandle).toBe('second');
  const switching = store.getState().setBase({ kind: 'parent', index: 0 }); expect(store.getState().diff).toBeNull(); expect(store.getState().details).toBeNull();
  store.getState().clear(); requests[4].ready.resolve({ ok: false, error: { message: 'STALE' } }); await switching;
  expect(store.getState().error).toBeNull(); expect(store.getState().selected).toBeNull();
});

it('fails closed on mismatched echoes and sanitized transport failures', async () => {
  let fail = false;
  const api = vi.fn(async () => { if (fail) throw Error('CANARY filesystem path'); return ok({ ...details({ kind: 'parent', index: 0 }), context: { ...context, snapshotId: 'wrong' } }); });
  const store = createCommitInspectorStore(api as unknown as Api); await store.getState().select(selected);
  expect(store.getState().details).toBeNull(); expect(store.getState().error?.code).toBe('Internal');
  fail = true; await store.getState().retry(); expect(store.getState().error?.message).not.toContain('CANARY');
});

const fresh: HistorySelection = { ...selected, context: { ...context, scope: { ...context.scope, generation: '2' }, snapshotId: 'fresh' }, commit: { ...commit, handle: 'fresh-commit' } };
function refreshedDetails(target: HistorySelection, base: DiffBase, names = ['README.txt', 'whitespace.txt']): CommitDetails {
  return { ...details(base, target.commit), context: target.context, files: names.map((name, index): CommitFile => ({ handle: `${target.context.snapshotId}:${base.kind === 'root' ? 'root' : base.index}:${index}`,
    pathDisplay: name, previousPathDisplay: null, kind: 'modified', oldMode: '100644', newMode: '100644' })) };
}
const textDiff = (context: HistoryContext, fileHandle: string) => ({ context, fileHandle, content: { kind: 'text' as const, original: 'Same line', modified: 'Same line  ', language: null } });

it('S5-N02 retains nonfirst file/nondefault parent and content until new server handles finish rebinding', async () => {
  const metadata = deferred<unknown>(), content = deferred<unknown>();
  const api = vi.fn(async (command: string, request: { context: HistoryContext; base: DiffBase; fileHandle: string }) => {
    if (request.context.snapshotId === 'fresh') return command === 'get_commit_details' ? metadata.promise : content.promise;
    return ok(command === 'get_commit_details' ? refreshedDetails(selected, request.base) : textDiff(request.context, request.fileHandle));
  });
  const store = createCommitInspectorStore(api as unknown as Api); await store.getState().select(selected); await store.getState().setBase({ kind: 'parent', index: 1 }); await store.getState().selectFile('snapshot:1:1');
  const old = store.getState().diff, oldDetails = store.getState().details;
  store.getState().beginRefresh(context); expect(store.getState().diff).toBe(old); expect(store.getState().details).toBe(oldDetails);
  const rebinding = store.getState().select(fresh);
  expect(store.getState().diff).toBe(old); expect(store.getState().base).toEqual({ kind: 'parent', index: 1 });
  metadata.resolve(ok(refreshedDetails(fresh, { kind: 'parent', index: 1 })));
  await vi.waitFor(() => expect(api).toHaveBeenLastCalledWith('get_commit_diff', { context: fresh.context, fileHandle: 'fresh:1:1' }));
  expect(store.getState().diff).toBe(old);
  content.resolve(ok(textDiff(fresh.context, 'fresh:1:1'))); await rebinding;
  expect(store.getState().fileHandle).toBe('fresh:1:1'); expect(store.getState().displayedFile?.pathDisplay).toBe('whitespace.txt');
  expect(store.getState().diff?.context).toEqual(fresh.context); expect(store.getState().refreshing).toBe(false);
  expect(api.mock.calls.filter(([, request]) => request.context.snapshotId === 'fresh').map(([command, request]) => [command, request.fileHandle ?? request.base])).toEqual([
    ['get_commit_details', { kind: 'parent', index: 1 }], ['get_commit_diff', 'fresh:1:1'],
  ]);
});

it('queues latest parent/file intents during history staging without calling old handles', async () => {
  const api = vi.fn(async (command: string, request: { context: HistoryContext; base: DiffBase; fileHandle: string }) => ok(command === 'get_commit_details'
    ? refreshedDetails(request.context.snapshotId === 'fresh' ? fresh : selected, request.base) : textDiff(request.context, request.fileHandle)));
  const store = createCommitInspectorStore(api as unknown as Api); await store.getState().select(selected);
  store.getState().beginRefresh(context); api.mockClear();
  await store.getState().setBase({ kind: 'parent', index: 1 }); await store.getState().selectFile('snapshot:0:1');
  expect(api).not.toHaveBeenCalled();
  await store.getState().select(fresh);
  expect(store.getState().base).toEqual({ kind: 'parent', index: 1 }); expect(store.getState().fileHandle).toBe('fresh:1:1');
  expect(api).toHaveBeenCalledWith('get_commit_details', { context: fresh.context, commitHandle: 'fresh-commit', base: { kind: 'parent', index: 1 } });
});

it('lets a later parent/file choice supersede an in-flight rebind and ignores its stale metadata', async () => {
  const requests: { request: { context: HistoryContext; base: DiffBase }; ready: ReturnType<typeof deferred<unknown>> }[] = [];
  const api = vi.fn(async (command: string, request: { context: HistoryContext; base: DiffBase; fileHandle: string }) => {
    if (command === 'get_commit_details' && request.context.snapshotId === 'fresh') { const ready = deferred<unknown>(); requests.push({ request, ready }); return ready.promise; }
    return ok(command === 'get_commit_details' ? refreshedDetails(selected, request.base) : textDiff(request.context, request.fileHandle));
  });
  const store = createCommitInspectorStore(api as unknown as Api); await store.getState().select(selected); await store.getState().setBase({ kind: 'parent', index: 1 }); await store.getState().selectFile('snapshot:1:1');
  store.getState().beginRefresh(context); const old = store.getState().select(fresh), latest = store.getState().setBase({ kind: 'parent', index: 0 });
  const before = api.mock.calls.length; await store.getState().selectFile('snapshot:1:0'); expect(api.mock.calls).toHaveLength(before);
  requests[1].ready.resolve(ok(refreshedDetails(fresh, { kind: 'parent', index: 0 }))); await latest;
  requests[0].ready.resolve(ok(refreshedDetails(fresh, { kind: 'parent', index: 1 }))); await old;
  expect(store.getState().base).toEqual({ kind: 'parent', index: 0 }); expect(store.getState().fileHandle).toBe('fresh:0:0');
});

it.each(['old collision', 'new collision', 'missing'] as const)('requires explicit file selection for %s instead of guessing across snapshots', async kind => {
  const api = vi.fn(async (command: string, request: { context: HistoryContext; base: DiffBase; fileHandle: string }) => {
    if (command === 'get_commit_diff') return ok(textDiff(request.context, request.fileHandle));
    const isFresh = request.context.snapshotId === 'fresh', value = refreshedDetails(isFresh ? fresh : selected, request.base);
    if (!isFresh && kind === 'old collision' || isFresh && kind === 'new collision') value.files.push({ ...value.files[1], handle: `${request.context.snapshotId}:collision`, kind: 'added', oldMode: null });
    if (isFresh && kind === 'missing') value.files = [value.files[0]];
    return ok(value);
  });
  const store = createCommitInspectorStore(api as unknown as Api); await store.getState().select(selected); await store.getState().selectFile('snapshot:0:1');
  store.getState().beginRefresh(context); api.mockClear(); await store.getState().select(fresh);
  expect(store.getState().fileHandle).toBeNull(); expect(store.getState().diff).toBeNull(); expect(store.getState().notice).toContain('unambiguously');
  expect(api.mock.calls.filter(([command]) => command === 'get_commit_diff')).toHaveLength(0);
  await store.getState().selectFile('fresh:0:0'); expect(store.getState().diff?.fileHandle).toBe('fresh:0:0'); expect(store.getState().notice).toBeNull();
});

it('keeps the previous comparison on failure and retries only new-context handles', async () => {
  let fail = true;
  const api = vi.fn(async (command: string, request: { context: HistoryContext; base: DiffBase; fileHandle: string }) => {
    if (request.context.snapshotId === 'fresh' && fail) return { ok: false, error: { code: 'ExternalChange', phase: 'inspection', retry: 'retry', message: 'Refresh failed' } };
    return ok(command === 'get_commit_details' ? refreshedDetails(request.context.snapshotId === 'fresh' ? fresh : selected, request.base) : textDiff(request.context, request.fileHandle));
  });
  const store = createCommitInspectorStore(api as unknown as Api); await store.getState().select(selected); await store.getState().selectFile('snapshot:0:1'); const old = store.getState().diff;
  store.getState().beginRefresh(context); store.getState().refreshFailed(); expect(store.getState().diff).toBe(old); expect(store.getState().notice).toContain('retained');
  await store.getState().select(fresh); expect(store.getState().diff).toBe(old); expect(store.getState().error?.message).toBe('Refresh failed');
  fail = false; await store.getState().retry(); expect(store.getState().fileHandle).toBe('fresh:0:1'); expect(store.getState().diff?.context).toEqual(fresh.context);
});

it('preserves typed unavailable content until the refreshed typed result replaces it', async () => {
  const ready = deferred<unknown>();
  const api = vi.fn(async (command: string, request: { context: HistoryContext; base: DiffBase; fileHandle: string }) => {
    if (command === 'get_commit_details') return ok(refreshedDetails(request.context.snapshotId === 'fresh' ? fresh : selected, request.base));
    return request.context.snapshotId === 'fresh' ? ready.promise : ok({ context, fileHandle: request.fileHandle, content: { kind: 'unavailable', reason: 'binary' } });
  });
  const store = createCommitInspectorStore(api as unknown as Api); await store.getState().select(selected); const old = store.getState().diff;
  store.getState().beginRefresh(context); const loading = store.getState().select(fresh); await vi.waitFor(() => expect(store.getState().diffLoading).toBe(true));
  expect(store.getState().diff).toBe(old);
  ready.resolve(ok({ context: fresh.context, fileHandle: 'fresh:0:0', content: { kind: 'unavailable', reason: 'tooLarge' } })); await loading;
  expect(store.getState().diff?.content).toEqual({ kind: 'unavailable', reason: 'tooLarge' });
});

it('rejects a late old-file response after refreshed comparison has completed', async () => {
  const late = deferred<unknown>();
  const api = vi.fn(async (command: string, request: { context: HistoryContext; base: DiffBase; fileHandle: string }) => {
    if (command === 'get_commit_details') return ok(refreshedDetails(request.context.snapshotId === 'fresh' ? fresh : selected, request.base));
    if (request.fileHandle === 'snapshot:0:1') return late.promise;
    return ok(textDiff(request.context, request.fileHandle));
  });
  const store = createCommitInspectorStore(api as unknown as Api); await store.getState().select(selected);
  const old = store.getState().selectFile('snapshot:0:1'); store.getState().beginRefresh(context); await store.getState().select(fresh);
  expect(store.getState().fileHandle).toBe('fresh:0:1');
  late.resolve(ok({ context, fileHandle: 'snapshot:0:1', content: { kind: 'text', original: 'STALE', modified: 'STALE', language: null } })); await old;
  expect(store.getState().diff?.fileHandle).toBe('fresh:0:1'); expect(JSON.stringify(store.getState().diff)).not.toContain('STALE');
});
