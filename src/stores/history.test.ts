import { expect, it, vi } from 'vitest';
import { createHistoryStore } from './history';
import type { Api } from './workspace';
import type { AppError, CommitSummary, DiffBase, HistoryContext, HistoryScope, HistorySession, HistorySearchPage } from '../contracts';
import { defaultRepositoryView } from './historyView';

const ok = <T,>(value: T) => ({ ok: true as const, value });
const selection = { repoId: 'repo-a', worktreeId: 'tree-a' };
const failure: AppError = { code: 'ExternalChange', phase: 'inspection', retry: 'retry', message: 'Repository changed. Refresh history.' };
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
const commit = (oid: string, session: string, parents: string[] = []): CommitSummary => ({ oid, handle: `${session}:${oid}`, parents: parents.map(oid => ({ oid, boundary: 'offPage' })), subject: oid,
  authorName: 'Full Author', authorEmail: 'author@example.test', authoredAt: '2000-01-01T00:00:00Z', committedAt: '2000-01-01T00:00:00Z', decorations: [] });
function session(scope: HistoryScope, id = `s${scope.generation}`, oids = ['tip', 'base']): HistorySession {
  const context = { scope, snapshotId: `snapshot-${id}` };
  return { context, sessionId: id, head: { kind: 'branch', branch: 'main', oid: 'tip' }, roots: ['tip'],
    refs: ['main', 'topic'].map((name, index) => ({ name: `refs/heads/${name}`, displayName: name, kind: 'local' as const, oid: index ? 'outside' : 'tip', commitOid: index ? 'outside' : 'tip', symbolicTarget: null, hidden: false })),
    page: { context, sessionId: id, commits: oids.map((oid, i) => commit(oid, id, i + 1 < oids.length ? [oids[i + 1]] : ['root'])), nextCursor: 'page-2' } };
}
const page = (value: HistorySession, oids = ['root']) => ({ context: value.context, sessionId: value.sessionId, commits: oids.map(oid => commit(oid, value.sessionId)), nextCursor: null });
const searchPage = (context: HistoryContext, sessionId: string, query: string, oid = 'outside', outsideView = true): HistorySearchPage => ({ context, sessionId, query, commits: [{ commit: commit(oid, sessionId), outsideView }], refs: [], nextCursor: null });
function harness(extra?: (command: string, request: Record<string, unknown>) => unknown) {
  let active: HistorySession;
  const api = vi.fn(async (command: string, request: Record<string, unknown>) => {
    const result = extra?.(command, request); if (result !== undefined) return result;
    if (command === 'get_repository_view' || command === 'save_repository_view') return ok({ scope: request.scope, viewRevision: '0', view: request.view ?? defaultRepositoryView() });
    if (command === 'get_commit_details') return ok({ context: request.context, commit: commit((request.commitHandle as string).split(':').at(-1)!, active.sessionId), base: request.base, message: '', committerName: '', committerEmail: '', files: [] });
    if (command === 'open_history') { active = session(request.scope as HistoryScope); return ok(active); }
    if (command === 'load_history_page') return ok(page(active));
    if (command === 'search_history') return ok(searchPage(request.context as HistoryContext, request.sessionId as string, request.query as string));
    throw Error('Unexpected command');
  });
  return { api, store: createHistoryStore(api as unknown as Api) };
}

it('opens actual typed sessions, appends cursor pages with stable raw edges, and keeps a real selected handle', async () => {
  const { api, store } = harness(); await store.getState().activate(selection);
  const first = store.getState().layout;
  store.getState().selectCommit('tip'); expect(store.getState().selected?.commit.handle).toBe('s1:tip');
  await Promise.all([store.getState().loadMore(), store.getState().loadMore()]);
  expect(api.mock.calls.filter(([command]) => command === 'load_history_page')).toHaveLength(1);
  expect(store.getState().layout.nodes.slice(0, 2)).toEqual(first.nodes);
  expect(store.getState().layout.edges.map(edge => `${edge.childOid}>${edge.parentOid}`)).toEqual(['tip>base', 'base>root']);
  expect(store.getState().nextCursor).toBeNull(); expect(store.getState().selected?.context).toEqual(store.getState().session?.context);
});

it('rejects late repository open success/error and late pages after deactivate', async () => {
  const old = deferred<ReturnType<typeof ok<HistorySession>>>();
  const { store } = harness((command, request) => command === 'open_history' && (request.scope as HistoryScope).repoId === 'repo-a' ? old.promise : undefined);
  const opening = store.getState().activate(selection);
  await store.getState().activate({ repoId: 'repo-b', worktreeId: null });
  old.resolve(ok(session({ ...selection, generation: '1' }))); await opening;
  expect(store.getState().scope?.repoId).toBe('repo-b'); expect(store.getState().session?.context.scope.repoId).toBe('repo-b');
  const delayed = deferred<unknown>(), other = harness(command => command === 'load_history_page' ? delayed.promise : undefined);
  await other.store.getState().activate(selection); const loading = other.store.getState().loadMore(); other.store.getState().deactivate();
  delayed.resolve({ ok: false, error: failure }); await loading;
  expect(other.store.getState().session).toBeNull(); expect(other.store.getState().pageError).toBeNull(); expect(other.store.getState().pageLoading).toBe(false);
});

it('lets only the latest filter open win over stale page/open responses and sends canonical positive ref sets', async () => {
  const opens: { request: Record<string, unknown>; ready: ReturnType<typeof deferred<unknown>> }[] = [], delayedPage = deferred<unknown>();
  const { api, store } = harness((command, request) => {
    if (command === 'open_history' && (request.scope as HistoryScope).generation !== '1') { const ready = deferred<unknown>(); opens.push({ request, ready }); return ready.promise; }
    if (command === 'load_history_page') return delayedPage.promise;
  });
  await store.getState().activate(selection); const loading = store.getState().loadMore();
  const first = store.getState().setFilter({ hiddenRefs: ['refs/heads/topic', 'refs/heads/topic'], soloRefs: [] });
  const latest = store.getState().setFilter({ hiddenRefs: [], soloRefs: ['refs/heads/topic', 'refs/heads/main', 'refs/heads/topic'] });
  expect(opens[1].request.filter).toEqual({ hiddenRefs: [], soloRefs: ['refs/heads/main', 'refs/heads/topic'] });
  opens[1].ready.resolve(ok(session(opens[1].request.scope as HistoryScope, 'latest'))); await latest;
  opens[0].ready.resolve({ ok: false, error: failure }); delayedPage.resolve({ ok: false, error: failure }); await Promise.all([first, loading]);
  expect(store.getState().session?.sessionId).toBe('latest'); expect(store.getState().error).toBeNull(); expect(store.getState().pageError).toBeNull();
  expect(api.mock.calls.every(([name]) => ['get_repository_view', 'open_history', 'load_history_page'].includes(name))).toBe(true);
});

it('searches beyond loaded rows, rejects stale query results and selects outside-view hits without altering filters/graph', async () => {
  const requests: { request: Record<string, unknown>; ready: ReturnType<typeof deferred<unknown>> }[] = [];
  const { store } = harness((command, request) => { if (command === 'search_history') { const ready = deferred<unknown>(); requests.push({ request, ready }); return ready.promise; } });
  await store.getState().activate(selection); const layout = store.getState().layout, filter = store.getState().filter;
  const first = store.getState().search('old message'), latest = store.getState().search('Full Author');
  const value = requests[1].request; requests[1].ready.resolve(ok(searchPage(value.context as HistoryContext, value.sessionId as string, 'Full Author', 'deep-commit'))); await latest;
  requests[0].ready.resolve({ ok: false, error: failure }); await first;
  expect(store.getState().searchResults.resultQuery).toBe('Full Author'); expect(store.getState().searchResults.error).toBeNull();
  store.getState().selectSearchHit(store.getState().searchResults.hits[0]);
  expect(store.getState().selected?.outsideView).toBe(true); expect(store.getState().selected?.commit.handle).toBe('s1:deep-commit');
  expect(store.getState().layout).toBe(layout); expect(store.getState().filter).toBe(filter);
  const typing = store.getState().search('pending'); store.getState().setQuery('new input');
  requests[2].ready.resolve(ok(searchPage(value.context as HistoryContext, value.sessionId as string, 'pending'))); await typing;
  expect(store.getState().query).toBe('new input'); expect(store.getState().searchResults.loading).toBe(false); expect(store.getState().searchResults.hits).toEqual([]);
});

it('resolves captured ref OIDs through literal search and ignores a superseded selection/forged ref', async () => {
  const ready = deferred<unknown>();
  const { api, store } = harness(command => command === 'search_history' ? ready.promise : undefined);
  await store.getState().activate(selection); const session = store.getState().session!, target = session.refs[1];
  const selecting = store.getState().selectRef(target);
  expect(api).toHaveBeenLastCalledWith('search_history', { context: session.context, sessionId: 's1', query: 'outside', cursor: null, pageSize: null });
  store.getState().selectCommit('base');
  ready.resolve(ok(searchPage(session.context, session.sessionId, 'outside'))); await selecting;
  expect(store.getState().selected?.commit.oid).toBe('base'); expect(store.getState().selectionLoading).toBe(false);
  api.mockClear(); await store.getState().selectRef({ ...target, name: 'refs/heads/forged', commitOid: '--all' }); expect(api).not.toHaveBeenCalled();
});

it('stages refresh pages atomically, preserves extent/anchor and reissues the latest selected handle', async () => {
  const refreshOpen = deferred<unknown>(), refreshPage = deferred<unknown>();
  let fresh: HistorySession;
  const { store } = harness((command, request) => {
    if (command === 'open_history' && (request.scope as HistoryScope).generation === '2') { fresh = session(request.scope as HistoryScope, 'fresh'); return refreshOpen.promise; }
    if (command === 'load_history_page' && request.sessionId === 'fresh') return refreshPage.promise;
  });
  await store.getState().activate(selection); await store.getState().loadMore();
  store.getState().selectCommit('tip'); store.getState().setAnchor({ oid: 'root', offset: 9 });
  const old = store.getState().layout, refreshing = store.getState().refresh();
  expect(store.getState().layout).toBe(old); expect(store.getState().refreshing).toBe(true);
  store.getState().selectCommit('base');
  refreshOpen.resolve(ok(fresh!)); await Promise.resolve(); await Promise.resolve();
  expect(store.getState().layout).toBe(old);
  refreshPage.resolve(ok(page(fresh!))); await refreshing;
  expect(store.getState().layout.nodes).toHaveLength(3); expect(store.getState().anchor).toEqual({ oid: 'root', offset: 9 });
  expect(store.getState().selected?.commit.handle).toBe('fresh:base'); expect(store.getState().selected?.context).toEqual(fresh!.context);
  expect(store.getState().refreshing).toBe(false);
});

it('retains old content on refresh failure and remembers filters only in the correct repository', async () => {
  let fail = false;
  const { store, api } = harness(command => command === 'open_history' && fail ? { ok: false, error: failure } : undefined);
  await store.getState().activate(selection); await store.getState().setFilter({ hiddenRefs: ['refs/heads/topic'], soloRefs: [] });
  store.getState().selectCommit('tip'); const old = store.getState().layout;
  fail = true; await store.getState().refresh(); expect(store.getState().layout).toBe(old); expect(store.getState().error).toEqual(failure);
  fail = false; await store.getState().activate({ repoId: 'repo-b', worktreeId: null });
  expect(store.getState().filter).toEqual({ hiddenRefs: [], soloRefs: [] }); expect(store.getState().selected).toBeNull();
  expect(api.mock.calls.filter(([command]) => command === 'search_history')).toHaveLength(0);
  await store.getState().activate(selection); expect(store.getState().filter.hiddenRefs).toEqual(['refs/heads/topic']);
  // The synthetic non-hex selection is not a durable OID hint.
  expect(store.getState().selected).toBeNull();
});

it('fails closed on wrong page/session echoes, preserves retry cursor, and sanitizes transport rejection', async () => {
  let mode = 'wrong';
  const { store } = harness((command, request) => {
    if (command !== 'load_history_page') return;
    if (mode === 'throw') throw Error('CANARY private filesystem path');
    if (mode === 'wrong') return ok({ context: { ...(request.context as HistoryContext), snapshotId: 'wrong' }, sessionId: request.sessionId, commits: [], nextCursor: null });
  });
  await store.getState().activate(selection); const old = store.getState().layout;
  await store.getState().loadMore(); expect(store.getState().layout).toBe(old); expect(store.getState().pageError).not.toBeNull(); expect(store.getState().nextCursor).toBe('page-2');
  mode = 'throw'; await store.getState().loadMore(); expect(store.getState().pageError?.message).not.toContain('CANARY');
  mode = 'okay'; await store.getState().loadMore(); expect(store.getState().pageError).toBeNull(); expect(store.getState().nextCursor).toBeNull();
});

it('pages an exact ref-OID lookup when message matches fill its first search page', async () => {
  const { api, store } = harness((command, request) => {
    if (command !== 'search_history') return;
    const value = searchPage(request.context as HistoryContext, request.sessionId as string, request.query as string, request.cursor ? 'outside' : 'message-match');
    return ok({ ...value, nextCursor: request.cursor ? null : 'sha-page-2' });
  });
  await store.getState().activate(selection); const ref = store.getState().session!.refs[1];
  await store.getState().selectRef(ref);
  expect(api.mock.calls.filter(([command]) => command === 'search_history').map(([, request]) => request.cursor)).toEqual([null, 'sha-page-2']);
  expect(store.getState().selected?.commit.oid).toBe('outside'); expect(store.getState().selected?.outsideView).toBe(true);
});

it('refresh rebinds a ref-selection intent made during staging and discards its old-session response', async () => {
  const ready = deferred<unknown>(), requests: { request: Record<string, unknown>; ready: ReturnType<typeof deferred<unknown>> }[] = [];
  let fresh: HistorySession;
  const { store } = harness((command, request) => {
    if (command === 'open_history' && (request.scope as HistoryScope).generation === '2') { fresh = session(request.scope as HistoryScope, 'fresh'); return ready.promise; }
    if (command === 'search_history') { const response = deferred<unknown>(); requests.push({ request, ready: response }); return response.promise; }
  });
  await store.getState().activate(selection); const old = store.getState().session!;
  const refresh = store.getState().refresh(), selecting = store.getState().selectRef(old.refs[1]);
  ready.resolve(ok(fresh!)); await vi.waitFor(() => expect(requests).toHaveLength(2));
  expect(requests[1].request.context).toEqual(fresh!.context);
  requests[0].ready.resolve(ok(searchPage(old.context, old.sessionId, 'outside'))); await selecting;
  expect(store.getState().selectionLoading).toBe(true);
  requests[1].ready.resolve(ok(searchPage(fresh!.context, fresh!.sessionId, 'outside'))); await refresh;
  expect(store.getState().selected?.context).toEqual(fresh!.context); expect(store.getState().selected?.commit.handle).toBe('fresh:outside');
  expect(store.getState().selectionLoading).toBe(false);
});

it('keeps search cursor/hits on a failed continuation and retries the exact query without mixing results', async () => {
  let fail = true;
  const { store, api } = harness((command, request) => {
    if (command !== 'search_history') return;
    if (request.cursor && fail) return { ok: false, error: failure };
    return ok({ ...searchPage(request.context as HistoryContext, request.sessionId as string, request.query as string, request.cursor ? 'second' : 'first'), nextCursor: request.cursor ? null : 'search-next' });
  });
  await store.getState().activate(selection); await store.getState().search('literal.*'); await store.getState().searchMore();
  expect(store.getState().searchResults.hits.map(hit => hit.commit.oid)).toEqual(['first']); expect(store.getState().searchResults.nextCursor).toBe('search-next');
  fail = false; await store.getState().searchMore();
  expect(store.getState().searchResults.hits.map(hit => hit.commit.oid)).toEqual(['first', 'second']); expect(store.getState().searchResults.error).toBeNull();
  expect(api.mock.calls.filter(([command]) => command === 'search_history').map(([, request]) => [request.query, request.cursor])).toEqual([['literal.*', null], ['literal.*', 'search-next'], ['literal.*', 'search-next']]);
});

it('reveals a deep in-view search hit through real pages while outside-view hits never request graph pages', async () => {
  const { store, api } = harness((command, request) => command === 'search_history' ? ok(searchPage(request.context as HistoryContext, request.sessionId as string, request.query as string, 'root', false)) : undefined);
  await store.getState().activate(selection); await store.getState().search('deep root');
  store.getState().selectSearchHit(store.getState().searchResults.hits[0]);
  await vi.waitFor(() => expect(store.getState().layout.nodesByOid.has('root')).toBe(true));
  expect(store.getState().selected?.commit.handle).toBe('s1:root'); expect(api.mock.calls.filter(([command]) => command === 'load_history_page')).toHaveLength(1);
  const outside = harness(); await outside.store.getState().activate(selection); await outside.store.getState().search('deep outside');
  outside.store.getState().selectSearchHit(outside.store.getState().searchResults.hits[0]);
  expect(outside.api.mock.calls.filter(([command]) => command === 'load_history_page')).toHaveLength(0);
});

it('stops a superseded in-view reveal after its current page, sharing an existing page flight', async () => {
  const ready = deferred<unknown>();
  const { store, api } = harness((command, request) => {
    if (command === 'load_history_page') return ready.promise;
    if (command === 'search_history') return ok({ ...searchPage(request.context as HistoryContext, request.sessionId as string, request.query as string, 'deep', false),
      commits: [{ commit: commit('deep', 's1'), outsideView: false }, { commit: commit('outside', 's1'), outsideView: true }] });
  });
  await store.getState().activate(selection); await store.getState().search('two hits');
  const loading = store.getState().loadMore(), hits = store.getState().searchResults.hits;
  store.getState().selectSearchHit(hits[0]); store.getState().selectSearchHit(hits[1]);
  const session = store.getState().session!; ready.resolve(ok({ ...page(session), nextCursor: 'page-3' })); await loading; await Promise.resolve();
  expect(api.mock.calls.filter(([command]) => command === 'load_history_page')).toHaveLength(1);
  expect(store.getState().selected?.commit.oid).toBe('outside'); expect(store.getState().nextCursor).toBe('page-3');
});

it('preserves the loaded tail behind an anchor shifted over a page boundary by new commits', async () => {
  const { store, api } = harness((command, request) => {
    if (command === 'open_history' && (request.scope as HistoryScope).generation === '2') {
      const fresh = session(request.scope as HistoryScope, 'fresh', ['new', 'tip']);
      fresh.page.commits[1].parents = [{ oid: 'base', boundary: 'offPage' }];
      return ok(fresh);
    }
    if (command === 'load_history_page' && request.sessionId === 's1') return ok({ context: request.context, sessionId: 's1',
      commits: [commit('root', 's1', ['tail']), commit('tail', 's1')], nextCursor: null });
    if (command === 'load_history_page' && request.sessionId === 'fresh') return ok({ context: request.context, sessionId: 'fresh',
      commits: request.cursor === 'page-2' ? [commit('base', 'fresh', ['root'])] : [commit('root', 'fresh', ['tail']), commit('tail', 'fresh')],
      nextCursor: request.cursor === 'page-2' ? 'page-3' : null });
  });
  await store.getState().activate(selection); await store.getState().loadMore(); store.getState().setAnchor({ oid: 'base', offset: 11 });
  await store.getState().refresh();
  expect(store.getState().layout.nodes.map(node => node.commit.oid)).toEqual(['new', 'tip', 'base', 'root', 'tail']);
  expect(api.mock.calls.filter(([command, request]) => command === 'load_history_page' && request.sessionId === 'fresh')).toHaveLength(2);
  expect(store.getState().anchor).toEqual({ oid: 'base', offset: 11 });
});

function inspectedHistory() {
  let active: HistorySession;
  const gate = deferred<unknown>();
  const value = harness((command, request) => {
    if (command === 'open_history') {
      const scope = request.scope as HistoryScope, id = scope.generation === '1' ? 'old' : 'fresh';
      active = session(scope, id);
      active.page.commits = [commit('tip', id, ['left', 'right']), commit('left', id, ['root']), commit('right', id, ['root']), commit('root', id)]; active.page.nextCursor = null;
      return scope.generation === '1' ? ok(active) : gate.promise;
    }
    if (command === 'get_commit_details') {
      const base = request.base as DiffBase, target = active.page.commits.find(commit => commit.handle === request.commitHandle)!;
      return ok({ context: request.context, commit: target, base, message: 'Complete message', committerName: 'Committer', committerEmail: 'c@example.test',
        files: ['README.txt', 'whitespace.txt'].map((pathDisplay, index) => ({ handle: `${active.sessionId}:${base.kind === 'parent' ? base.index : 'root'}:${index}`, pathDisplay, previousPathDisplay: null, kind: 'modified', oldMode: '100644', newMode: '100644' })) });
    }
    if (command === 'get_commit_diff') return ok({ context: request.context, fileHandle: request.fileHandle, content: { kind: 'text', original: 'Same line', modified: 'Same line  ', language: null } });
  });
  return { ...value, gate, active: () => active };
}

it('S5-N02 history refresh retains the explicit merge parent/file and old content until new handles are ready', async () => {
  const { store, api, gate, active } = inspectedHistory(); await store.getState().activate(selection); store.getState().selectCommit('tip');
  const inspector = store.getState().inspector;
  await vi.waitFor(() => expect(inspector.getState().diff).not.toBeNull());
  await inspector.getState().setBase({ kind: 'parent', index: 1 }); await inspector.getState().selectFile('old:1:1');
  const previous = inspector.getState().diff, refreshing = store.getState().refresh();
  expect(inspector.getState().diff).toBe(previous); expect(inspector.getState().displayedFile?.pathDisplay).toBe('whitespace.txt');
  gate.resolve(ok(active())); await refreshing;
  expect(inspector.getState().base).toEqual({ kind: 'parent', index: 1 }); expect(inspector.getState().fileHandle).toBe('fresh:1:1');
  expect(inspector.getState().diff?.context).toEqual(active().context); expect(inspector.getState().displayedFile?.pathDisplay).toBe('whitespace.txt');
  expect(api.mock.calls.filter(([command, request]) => command === 'get_commit_diff' && (request.context as HistoryContext).snapshotId === 'snapshot-fresh').map(([, request]) => request.fileHandle)).toEqual(['fresh:1:1']);
  store.getState().deactivate();
});

it('a genuinely new commit selected during refresh clears prior comparison and wins fresh-handle rebinding', async () => {
  const { store, api, gate, active } = inspectedHistory(); await store.getState().activate(selection); store.getState().selectCommit('tip');
  const inspector = store.getState().inspector; await vi.waitFor(() => expect(inspector.getState().diff).not.toBeNull());
  await inspector.getState().selectFile('old:0:1'); const refreshing = store.getState().refresh();
  const count = api.mock.calls.length; store.getState().selectCommit('right');
  expect(inspector.getState().diff).toBeNull(); expect(inspector.getState().details).toBeNull(); expect(api.mock.calls).toHaveLength(count);
  gate.resolve(ok(active())); await refreshing;
  expect(store.getState().selected?.commit.oid).toBe('right'); expect(inspector.getState().selected?.commit.oid).toBe('right');
  expect(inspector.getState().details?.commit.handle).toBe('fresh:right'); expect(inspector.getState().fileHandle).toBe('fresh:0:0');
  store.getState().deactivate();
});

it('a failed history refresh retains the labelled comparison without issuing revoked-context requests', async () => {
  const { store, api, gate } = inspectedHistory(); await store.getState().activate(selection); store.getState().selectCommit('tip');
  const inspector = store.getState().inspector; await vi.waitFor(() => expect(inspector.getState().diff).not.toBeNull());
  await inspector.getState().selectFile('old:0:1'); const previous = inspector.getState().diff, refreshing = store.getState().refresh();
  gate.resolve({ ok: false, error: failure }); await refreshing;
  expect(inspector.getState().diff).toBe(previous); expect(inspector.getState().notice).toContain('retained'); expect(inspector.getState().refreshing).toBe(false);
  const count = api.mock.calls.length; await inspector.getState().setBase({ kind: 'parent', index: 1 }); await inspector.getState().selectFile('old:0:0');
  expect(api.mock.calls).toHaveLength(count); store.getState().deactivate();
});

it('S6-N01 replays search before inspector restore and keeps the newest submitted query over an older replay', async () => {
  const diff = deferred<unknown>(), oldReplay = deferred<unknown>();
  const { store, api } = harness((command, request) => {
    const context = request.context as HistoryContext;
    if (command === 'search_history' && context.scope.generation === '2' && request.query === 'outside intent') return oldReplay.promise;
    if (command === 'get_commit_details') return ok({ context, commit: commit((request.commitHandle as string).split(':').at(-1)!, `s${context.scope.generation}`), base: request.base, message: '', committerName: '', committerEmail: '',
      files: [{ handle: `file-${context.scope.generation}`, pathDisplay: 'README.txt', previousPathDisplay: null, kind: 'modified', oldMode: '100644', newMode: '100644' }] });
    if (command === 'get_commit_diff') return context.scope.generation === '2' ? diff.promise : ok({ context, fileHandle: request.fileHandle, content: { kind: 'text', original: 'old', modified: 'current', language: null } });
  });
  await store.getState().activate(selection); store.getState().selectCommit('tip');
  await vi.waitFor(() => expect(store.getState().inspector.getState().diff).not.toBeNull());
  await store.getState().search('outside intent'); const old = store.getState().searchResults.hits[0];
  const refreshing = store.getState().refresh();
  await vi.waitFor(() => expect(store.getState().inspector.getState().diffLoading).toBe(true));
  const replay = api.mock.calls.filter(([command, request]) => command === 'search_history' && (request.context as HistoryContext).scope.generation === '2');
  expect(replay).toHaveLength(1);
  expect(store.getState().searchResults.retained).toBe(true);
  store.getState().selectSearchHit(old); expect(store.getState().selected?.commit.oid).toBe('tip');
  await store.getState().search('latest during restore');
  expect(store.getState().searchResults.hits[0].commit.handle).toBe('s2:outside');
  oldReplay.resolve(ok(searchPage(store.getState().session!.context, 's2', 'outside intent', 'obsolete')));
  await Promise.resolve(); await Promise.resolve();
  expect(store.getState().searchResults.resultQuery).toBe('latest during restore');
  expect(store.getState().searchResults.hits.map(hit => hit.commit.oid)).toEqual(['outside']);
  store.getState().selectSearchHit(store.getState().searchResults.hits[0]); expect(store.getState().selected?.commit.oid).toBe('outside');
  diff.resolve(ok({ context: store.getState().session!.context, fileHandle: 'file-2', content: { kind: 'text', original: 'old', modified: 'current', language: null } }));
  await refreshing; expect(store.getState().selected?.commit.oid).toBe('outside'); store.getState().deactivate();
});

it('S6-N01 queues the latest search during filter staging and publishes current-filter fresh handles only', async () => {
  const opening = deferred<unknown>(); let fresh!: HistorySession;
  const { store, api } = harness((command, request) => {
    if (command === 'open_history' && (request.scope as HistoryScope).generation === '2') { fresh = session(request.scope as HistoryScope); return opening.promise; }
    if (command === 'search_history') return ok(searchPage(request.context as HistoryContext, request.sessionId as string, request.query as string, 'outside', (request.context as HistoryContext).scope.generation === '2'));
  });
  await store.getState().activate(selection); await store.getState().search('first');
  const old = store.getState().searchResults.hits[0], openingFilter = store.getState().setFilter({ hiddenRefs: ['refs/heads/topic'], soloRefs: ['refs/heads/main'] });
  await store.getState().search('middle'); await store.getState().search('newest staging');
  expect(api.mock.calls.filter(([command]) => command === 'search_history')).toHaveLength(1);
  store.getState().selectSearchHit(old); expect(store.getState().selected).toBeNull();
  opening.resolve(ok(fresh)); await openingFilter;
  expect(store.getState().searchResults.resultQuery).toBe('newest staging'); expect(store.getState().searchResults.retained).toBe(false);
  expect(store.getState().searchResults.hits[0]).toMatchObject({ outsideView: true, commit: { handle: 's2:outside' } });
  expect(api.mock.calls.filter(([command]) => command === 'search_history').map(([, request]) => request.query)).toEqual(['first', 'newest staging']);
  expect(store.getState().filter).toEqual({ hiddenRefs: ['refs/heads/topic'], soloRefs: ['refs/heads/main'] }); store.getState().deactivate();
});

it('S6-N01 keeps failed-refresh retained results inert without a permanent search spinner and retries the latest intent', async () => {
  let failed = true;
  const { store, api } = harness((command, request) => command === 'open_history' && (request.scope as HistoryScope).generation !== '1' && failed ? { ok: false, error: failure } : undefined);
  await store.getState().activate(selection); await store.getState().search('retained'); const hit = store.getState().searchResults.hits[0];
  await store.getState().refresh(); expect(store.getState().searchResults).toMatchObject({ loading: false, retained: true });
  store.getState().selectSearchHit(hit); expect(store.getState().selected).toBeNull();
  await store.getState().search('retry intent'); expect(store.getState().searchResults.loading).toBe(false);
  expect(api.mock.calls.filter(([command]) => command === 'search_history')).toHaveLength(1);
  failed = false; await store.getState().refresh(); expect(store.getState().searchResults.resultQuery).toBe('retry intent'); expect(store.getState().searchResults.retained).toBe(false);
  expect(store.getState().searchResults.hits[0].commit.handle).toBe('s3:outside'); store.getState().deactivate();
});
