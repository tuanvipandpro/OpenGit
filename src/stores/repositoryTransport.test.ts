import { expect, it, vi } from 'vitest';
import { createRepositoryTransportStore } from './repositoryTransport';
import type { TransportDraft } from './repositoryTransport';
import { createRepositoryActionsStore } from './repositoryActions';
import { createHistoryStore } from './history';
import { createOperationsStore } from './operations';
import type { Api } from './workspace';
import type { ActionIntent, ActionScope, OperationSnapshot, RemoteActionCatalog, RemoteRefSnapshot, RefActionCatalog, RefActionSource, RemoteTargetSource } from '../contracts';
import refWire from '../../fixtures/contracts/ref-actions-wire.json';
const ok = <T,>(value: T) => ({ ok: true as const, value });
const scope: ActionScope = { repoId: 'repo', worktreeId: 'tree', generation: '1' };
const oid = 'a'.repeat(40), tagOid = 'b'.repeat(40);
const local: RefActionCatalog = { ...refWire.catalog as RefActionCatalog, scope, catalogId: 'local', refs: [
  { ...refWire.catalog.refs[0], name: 'refs/heads/main', kind: 'local' },
  { ...refWire.catalog.refs[0], entryId: 'dev', name: 'refs/heads/dev', kind: 'local', checkedOutHere: false },
  { ...refWire.catalog.refs[2], entryId: 'tag', name: 'refs/tags/v1', kind: 'tag' },
  { ...refWire.catalog.refs[0], entryId: 'upstream', name: 'refs/remotes/origin/main', kind: 'remote', checkedOutHere: false },
] };
const remoteCatalog: RemoteActionCatalog = { scope, observedAt: '2026-10-25T00:00:00Z', remotes: ['origin', 'backup'].map(name => ({ name, endpoints: [{ role: 'fetch', handle: `${name}-fetch`, capability: 'eligible' }, { role: 'push', handle: `${name}-push`, capability: 'eligible' }] })) };
const snapshot: RemoteRefSnapshot = { capturedScope: scope, snapshotId: 'remote-snapshot', remoteName: 'origin', role: 'push', observedAt: '2026-10-25T00:00:00Z', expiresAt: '2099-01-01', objectFormat: 'sha1', refs: [{ entryId: 'remote-main', name: 'refs/heads/main', oid }, { entryId: 'remote-tag', name: 'refs/tags/v1', oid: tagOid }] };
const operation = (kind: ActionIntent['kind'], state: OperationSnapshot['state'] = 'Succeeded', id = kind): OperationSnapshot => ({ ...refWire.result.operation as OperationSnapshot, id, actionKind: kind, state, sequence: state === 'Queued' ? '1' : '3', error: null });
const draft = (patch: Partial<TransportDraft> = {}): TransportDraft => ({ kind: 'push', sourceRef: 'refs/heads/main', remoteName: 'origin', entryId: 'remote-main', newName: 'main', ...patch });
const settle = async () => { for (let i = 0; i < 4; i++) await new Promise<void>(resolve => queueMicrotask(resolve)); };
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
function setup(extra?: (command: string, request: Record<string, unknown>) => unknown) {
  let lastKind: ActionIntent['kind'] = 'push';
  const api = vi.fn(async (command: string, request: Record<string, unknown>): Promise<unknown> => {
    const replacement = extra?.(command, request); if (replacement !== undefined) return replacement;
    if (command === 'get_remote_actions') return ok({ ...remoteCatalog, scope: request.scope });
    if (command === 'resolve_ref_action_target') {
      const source = request.source as RefActionSource, entry = refs.getState().catalog!.refs.find(ref => source.kind === 'ref' && ref.entryId === source.entryId)!;
      return ok({ scope: request.scope, catalogId: request.catalogId, kind: entry.kind === 'local' ? 'branch' : entry.kind === 'tag' ? 'tag' : 'remoteRef', displayName: entry.name, oid: entry.oid, handle: `local:${entry.entryId}` });
    }
    if (command === 'get_publication_evidence') return ok({ scope: request.scope, observations: [] });
    if (command === 'prepare_repository_action') { lastKind = (request.intent as ActionIntent).kind; return ok({ scope: request.scope, kind: lastKind, planId: `plan:${lastKind}`, warnings: [], expiresAt: '2099-01-01' }); }
    if (command === 'execute_repository_action') return ok(operation(lastKind, 'Queued'));
    if (command === 'get_remote_action_outcome') return ok({ operation: operation('readRemoteRefs'), availability: 'available', evidence: { remoteName: 'origin', role: 'push', phase: 'observed', refCount: '2', observedAt: snapshot.observedAt, noGitWrites: true }, snapshot });
    if (command === 'resolve_remote_target') {
      const choice = request.source as RemoteTargetSource, entry = choice.kind === 'existing' ? snapshot.refs.find(ref => ref.entryId === choice.entryId)! : null;
      const namespace = entry ? entry.name.startsWith('refs/heads/') ? 'branch' : 'tag' : choice.kind === 'new' ? choice.namespace : 'branch';
      return ok({ scope: request.scope, snapshotId: request.snapshotId, handle: 'remote-target', namespace, name: entry?.name ?? `refs/${namespace === 'branch' ? 'heads' : 'tags'}/${choice.kind === 'new' ? choice.name : ''}`, expectedOid: entry?.oid ?? null });
    }
    throw Error(command);
  });
  const history = createHistoryStore(api as unknown as Api), refs = createRepositoryActionsStore(api as unknown as Api), tasks = createOperationsStore(api as unknown as Api), store = createRepositoryTransportStore(api as unknown as Api);
  history.getState().preferences.setState({ scope }); refs.setState({ scope, catalog: local });
  const sync = vi.fn(async () => {}); tasks.setState({ snapshot: { streamId: 'tasks', sequence: '0', operations: [] }, sync });
  const stop = store.getState().connect(history, refs, tasks);
  const start = async () => { await settle(); await store.getState().begin(); await settle(); };
  const publish = (value: OperationSnapshot, streamId = 'tasks') => tasks.setState({ snapshot: { streamId, sequence: value.sequence, operations: [value] } });
  const read = async () => { await store.getState().readRemoteRefs('origin'); publish(operation('readRemoteRefs')); await settle(); };
  return { api, history, refs, tasks, store, sync, stop, start, publish, read };
}
it('Sync reads the PUSH role through a queued authenticated task and consumes only its dated terminal observation', async () => {
  const t = setup(); await t.start();
  try {
    await t.store.getState().readRemoteRefs('origin');
    expect(t.api).toHaveBeenCalledWith('prepare_repository_action', { scope, intent: { kind: 'readRemoteRefs', remoteHandle: 'origin-push' } });
    expect(t.store.getState().readLoading).toBe(true); expect(t.store.getState().snapshot).toBeNull();
    t.publish(operation('readRemoteRefs', 'WaitingForUser')); await settle(); expect(t.api.mock.calls.some(([command]) => command === 'get_remote_action_outcome')).toBe(false);
    t.publish(operation('readRemoteRefs')); await settle(); expect(t.store.getState().snapshot).toEqual(snapshot); expect(t.sync).toHaveBeenCalledOnce();
    await t.store.getState().prepare(draft());
    expect(t.api).toHaveBeenCalledWith('resolve_remote_target', { scope, snapshotId: snapshot.snapshotId, source: { kind: 'existing', entryId: 'remote-main' } });
    expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action', { scope, intent: { kind: 'push', remoteHandle: 'origin-push', sourceHandle: 'local:entry-0', destinationHandle: 'remote-target', setUpstream: false } });
  } finally { t.stop(); await settle(); }
});
it('Sync resolves source by qualified current shared catalog, new destination by name and pull by configured upstream handle', async () => {
  const t = setup(); await t.start();
  try {
    await t.read(); await t.store.getState().prepare(draft({ sourceRef: 'refs/heads/dev', entryId: null, newName: 'topic' }));
    expect(t.api).toHaveBeenCalledWith('resolve_remote_target', { scope, snapshotId: snapshot.snapshotId, source: { kind: 'new', namespace: 'branch', name: 'topic' } });
    expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action', { scope, intent: { kind: 'push', remoteHandle: 'origin-push', sourceHandle: 'local:dev', destinationHandle: 'remote-target', setUpstream: false } });
    await t.store.getState().prepare(draft({ kind: 'pull', sourceRef: 'refs/remotes/origin/main' }));
    expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action', { scope, intent: { kind: 'pull', upstreamHandle: 'local:upstream' } });
  } finally { t.stop(); await settle(); }
});
it('Sync tag push uses raw local tag handle and remote-only delete uses exact observed tag target', async () => {
  const t = setup(); await t.start();
  try {
    await t.read(); await t.store.getState().prepare(draft({ kind: 'remoteTagPush', sourceRef: 'refs/tags/v1', entryId: 'remote-tag' }));
    expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action', { scope, intent: { kind: 'remoteTagPush', remoteHandle: 'origin-push', tagHandle: 'local:tag', destinationHandle: 'remote-target' } });
    t.refs.setState({ catalog: { ...local, catalogId: 'no-local-tag', refs: local.refs.filter(ref => ref.kind !== 'tag') } }); await settle();
    await t.store.getState().prepare(draft({ kind: 'remoteTagDelete', sourceRef: '', entryId: 'remote-tag' }));
    expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action', { scope, intent: { kind: 'remoteTagDelete', remoteHandle: 'origin-push', tagHandle: 'remote-target' } });
    expect(t.store.getState().preview?.source).toContain(tagOid);
  } finally { t.stop(); await settle(); }
});
it.each(['id', 'kind', 'scope', 'role', 'remote', 'expired', 'failed'])('Sync rejects forged, failed or stale read response: %s', async mismatch => {
  const t = setup(command => command === 'get_remote_action_outcome' ? ok({ operation: { ...operation('readRemoteRefs'), ...(mismatch === 'id' ? { id: 'wrong' } : mismatch === 'kind' ? { actionKind: 'fetch' } : mismatch === 'failed' ? { state: 'Failed' } : {}) }, availability: 'available', evidence: { remoteName: 'origin', role: 'push', phase: 'observed' }, snapshot: { ...snapshot, ...(mismatch === 'scope' ? { capturedScope: { ...scope, generation: 'old' } } : mismatch === 'role' ? { role: 'fetch' } : mismatch === 'remote' ? { remoteName: 'backup' } : mismatch === 'expired' ? { expiresAt: '2000-01-01' } : {}) } }) : undefined);
  await t.start();
  try { await t.read(); expect(t.store.getState().snapshot).toBeNull(); expect(t.store.getState().actionError).not.toBeNull(); expect(t.store.getState().readLoading).toBe(false); } finally { t.stop(); await settle(); }
});
it.each(['scope', 'snapshot', 'kind', 'name', 'oid'])('Sync rejects remote resolver identity mismatch: %s', async mismatch => {
  const t = setup((command, request) => command === 'resolve_remote_target' ? ok({ scope: mismatch === 'scope' ? { ...scope, generation: 'old' } : request.scope, snapshotId: mismatch === 'snapshot' ? 'old' : request.snapshotId, handle: 'forged', namespace: mismatch === 'kind' ? 'tag' : 'branch', name: mismatch === 'name' ? 'refs/heads/other' : 'refs/heads/main', expectedOid: mismatch === 'oid' ? tagOid : oid }) : undefined);
  await t.start();
  try { await t.read(); await t.store.getState().prepare(draft()); expect(t.store.getState().prepared).toBeNull(); expect(t.store.getState().actionError?.message).toMatch(/destination changed/); } finally { t.stop(); await settle(); }
});
it('Sync refuses wrong namespace and absent targets instead of silently choosing another destination', async () => {
  const t = setup(); await t.start();
  try {
    await t.read();
    for (const entryId of ['remote-tag', 'missing']) { await t.store.getState().prepare(draft({ entryId })); expect(t.store.getState().prepared).toBeNull(); }
    expect(t.api.mock.calls.some(([command]) => command === 'resolve_remote_target')).toBe(false);
  } finally { t.stop(); await settle(); }
});
it('Sync rejects expired read plans and unsupported/multiple push endpoints before enqueue', async () => {
  const t = setup(command => command === 'prepare_repository_action' ? ok({ scope, kind: 'readRemoteRefs', planId: 'expired', expiresAt: '2000-01-01', warnings: [] }) : undefined); await t.start();
  try {
    await t.store.getState().readRemoteRefs('origin'); expect(t.api.mock.calls.some(([command]) => command === 'execute_repository_action')).toBe(false);
    t.store.setState({ catalog: { ...remoteCatalog, remotes: [{ name: 'origin', endpoints: [{ role: 'fetch', capability: 'eligible', handle: 'fetch-only' }] }] } });
    await t.store.getState().readRemoteRefs('origin'); expect(t.store.getState().actionError?.message).toMatch(/single push/);
  } finally { t.stop(); await settle(); }
});
it('Sync fences late destination replies on input edits, shared-catalog replacement and selection ABA', async () => {
  const pending = deferred<unknown>(); const t = setup(command => command === 'resolve_remote_target' ? pending.promise : undefined); await t.start();
  try {
    await t.read(); const preparing = t.store.getState().prepare(draft());
    t.store.getState().clearPrepared(); t.refs.setState({ catalog: { ...local, catalogId: 'replaced' } });
    t.history.getState().preferences.setState({ scope: { ...scope, repoId: 'other', generation: '2' } }); await settle();
    t.history.getState().preferences.setState({ scope: { ...scope, generation: '3' } }); await settle();
    pending.resolve(ok({ scope, snapshotId: snapshot.snapshotId, handle: 'old', namespace: 'branch', name: 'refs/heads/main', expectedOid: oid })); await preparing;
    expect(t.store.getState().prepared).toBeNull(); expect(t.store.getState().snapshot).toBeNull(); expect(t.store.getState().scope?.generation).toBe('3');
  } finally { t.stop(); await settle(); }
});
it('Sync stales previews on shared catalog replacement, observation expiration and task retirement', async () => {
  const t = setup(); await t.start();
  try {
    await t.read(); await t.store.getState().prepare(draft()); expect(t.store.getState().prepared).not.toBeNull();
    t.refs.setState({ catalog: { ...local, catalogId: 'replaced' } }); await settle(); expect(t.store.getState().prepared).toBeNull();
    await t.store.getState().prepare(draft()); t.store.setState({ snapshot: { ...snapshot, expiresAt: '2000-01-01' } });
    expect(await t.store.getState().execute()).toBeNull(); expect(t.store.getState().actionError?.message).toMatch(/changed or expired/);
    t.tasks.setState({ snapshot: { streamId: 'tasks', sequence: '4', operations: [] } }); await settle(); expect(t.store.getState().snapshot).toBeNull();
  } finally { t.stop(); await settle(); }
});
it('Sync consumes execution once, synchronizes lost read replies and fences a disposed execution', async () => {
  const pending = deferred<unknown>(); let deferExecution = false;
  const t = setup(command => command === 'execute_repository_action' && deferExecution ? pending.promise : undefined); await t.start();
  try {
    await t.read(); await t.store.getState().prepare(draft()); deferExecution = true;
    const first = t.store.getState().execute(); expect(await t.store.getState().execute()).toBeNull();
    t.store.getState().end(); await t.store.getState().begin();
    pending.resolve({ ok: false, error: { code: 'Internal', phase: 'transfer', retry: 'none', message: 'Lost reply' } }); await first;
    expect(t.store.getState().actionError).toBeNull(); expect(t.store.getState().executing).toBe(false);
    const mutationCalls = t.api.mock.calls.filter(([command]) => command === 'execute_repository_action'); expect(mutationCalls).toHaveLength(2);
  } finally { t.stop(); await settle(); }
});
it('Sync synchronizes Tasks after a lost read enqueue reply without auto retrying', async () => {
  const t = setup(command => command === 'execute_repository_action' ? { ok: false, error: { code: 'Internal', phase: 'transfer', retry: 'none', message: 'Reply unavailable' } } : undefined); await t.start();
  try {
    await t.store.getState().readRemoteRefs('origin'); expect(t.sync).toHaveBeenCalledOnce(); expect(t.store.getState().readLoading).toBe(false); expect(t.store.getState().readId).toBeNull();
    expect(t.api.mock.calls.filter(([command]) => command === 'execute_repository_action')).toHaveLength(1);
  } finally { t.stop(); await settle(); }
});
it('Sync fences a late read proof after its task is retired/reintroduced on a different stream', async () => {
  const pending = deferred<unknown>(); const t = setup(command => command === 'get_remote_action_outcome' ? pending.promise : undefined); await t.start();
  try {
    await t.store.getState().readRemoteRefs('origin'); t.publish(operation('readRemoteRefs')); await settle();
    t.publish(operation('readRemoteRefs'), 'replacement-stream'); await settle();
    pending.resolve(ok({ operation: operation('readRemoteRefs'), availability: 'available', evidence: { remoteName: 'origin', role: 'push', phase: 'observed' }, snapshot })); await settle();
    expect(t.store.getState().snapshot).toBeNull(); expect(t.store.getState().actionError?.message).toMatch(/retired or replaced/);
  } finally { t.stop(); await settle(); }
});
it.each(['kind', 'scope', 'expiry'])('Sync rejects a mutation preparation with mismatched %s', async mismatch => {
  const t = setup((command, request) => command === 'prepare_repository_action' && (request.intent as ActionIntent).kind === 'push' ? ok({ scope: mismatch === 'scope' ? { ...scope, generation: 'old' } : scope, kind: mismatch === 'kind' ? 'pull' : 'push', planId: 'forged', expiresAt: mismatch === 'expiry' ? '2000-01-01' : '2099-01-01', warnings: [] }) : undefined); await t.start();
  try { await t.read(); await t.store.getState().prepare(draft()); expect(t.store.getState().prepared).toBeNull(); expect(t.store.getState().actionError?.message).toMatch(/changed or expired/); } finally { t.stop(); await settle(); }
});
it('Sync reloads dated matching publication ACK from existing terminal push Tasks without republishing RefCatalog', async () => {
  let acknowledged = false;
  const ack = { remoteName: 'origin', role: 'push', destination: 'refs/heads/main', sourceRef: 'refs/heads/main', sourceOid: oid, acknowledgedOid: oid, acknowledgedAt: '2026-10-25', configDigest: 'digest', deleted: false };
  const t = setup((command, request) => command === 'get_publication_evidence' ? ok({ scope: request.scope, observations: acknowledged ? [ack, { ...ack, role: 'fetch' }, { ...ack, sourceRef: 'refs/heads/other' }] : [] }) : undefined); await t.start();
  try {
    expect(t.store.getState().publications).toEqual([]); acknowledged = true; t.publish(operation('push')); await settle();
    expect(t.store.getState().publicationScope).toEqual(scope); expect(t.store.getState().publications).toEqual([ack]);
    expect(t.api.mock.calls.some(([command]) => command === 'get_ref_actions')).toBe(false);
  } finally { t.stop(); await settle(); }
});
