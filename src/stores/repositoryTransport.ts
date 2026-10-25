import { createStore } from 'zustand/vanilla';
import type { StoreApi } from 'zustand';
import { call, transportError } from '../contracts';
import type { ActionIntent, ActionScope, ApiResult, AppError, OperationSnapshot, PreparedAction, PublicationEvidence, RemoteActionCatalog, RemoteMutationKind, RemoteRefSnapshot } from '../contracts';
import type { Api } from './workspace';
import type { HistoryState } from './history';
import type { RepositoryActionsState } from './repositoryActions';
import type { OperationsState } from './operations';
import { activeOperation } from './operations';
import { sameActionScope } from './repositorySync';
export type TransportDraft = { kind: RemoteMutationKind; sourceRef: string; remoteName: string; entryId: string | null; newName: string };
export type TransportLaunch = { kind: RemoteMutationKind; refName?: string };
const invalid = (message: string): AppError => ({ code: 'InvalidHandle', phase: 'inspection', retry: 'none', message });
const pushEndpoint = (catalog: RemoteActionCatalog | null, name: string) => {
  const endpoints = catalog?.remotes.find(remote => remote.name === name)?.endpoints.filter(endpoint => endpoint.role === 'push');
  return endpoints?.length === 1 && endpoints[0].capability === 'eligible' && endpoints[0].handle ? endpoints[0] : null;
};
export interface RepositoryTransportState {
  scope: ActionScope | null; catalog: RemoteActionCatalog | null; loading: boolean; error: AppError | null;
  readId: string | null; readRemote: string | null; readLoading: boolean; snapshot: RemoteRefSnapshot | null;
  prepared: PreparedAction | null; preview: { source: string; destination: string } | null; preparing: boolean; executing: boolean; actionError: AppError | null;
  publicationScope: ActionScope | null; publications: PublicationEvidence[];
  connect(history: StoreApi<HistoryState>, refs: StoreApi<RepositoryActionsState>, tasks: StoreApi<OperationsState>): () => void;
  begin(): Promise<void>; end(): void; reload(): Promise<void>; clearPrepared(): void; clearRead(): void;
  readRemoteRefs(remoteName: string): Promise<void>; prepare(draft: TransportDraft): Promise<void>; execute(): Promise<ApiResult<OperationSnapshot> | null>;
}
export function createRepositoryTransportStore(api: Api = call) {
  let owner: { history: StoreApi<HistoryState>; refs: StoreApi<RepositoryActionsState>; tasks: StoreApi<OperationsState>; users: number; stop(): void } | null = null;
  let opened = false, key: string | null = null, publicationKey: string | null = null, readSequence: string | null = null, readStream: string | null = null;
  let contextTicket = 0, actionTicket = 0, readTicket = 0, publicationTicket = 0, executionTicket = 0, scheduled = false, refsKey: string | null = null;
  let preparedCurrent: (() => boolean) | null = null;
  return createStore<RepositoryTransportState>((set, get) => {
    const live = (scope: ActionScope) => !!owner && sameActionScope(scope, get().scope) && sameActionScope(scope, owner.history.getState().preferences.getState().scope);
    const sourceCatalog = (scope: ActionScope) => { const refs = owner?.refs.getState(); return refs && !refs.loading && !refs.error && refs.catalog && sameActionScope(refs.catalog.scope, scope) ? refs.catalog : null; };
    const source = async (scope: ActionScope, name: string, kind: 'branch' | 'tag' | 'remoteRef') => {
      const catalog = sourceCatalog(scope); const entry = catalog?.refs.find(e => e.name === name && !e.symbolicTarget && e.kind === (kind === 'branch' ? 'local' : kind === 'tag' ? 'tag' : 'remote'));
      if (!catalog || !entry) throw invalid('Choose an available qualified reference from the current context.');
      const r = await api('resolve_ref_action_target', { scope, catalogId: catalog.catalogId, source: { kind: 'ref', entryId: entry.entryId } });
      if (!live(scope) || sourceCatalog(scope)?.catalogId !== catalog.catalogId) throw invalid('Repository context changed. Prepare again.');
      if (!r.ok) throw r.error;
      if (!sameActionScope(r.value.scope, scope) || r.value.catalogId !== catalog.catalogId || r.value.kind !== kind || r.value.displayName !== name || r.value.oid !== entry.oid) throw invalid('The resolved source changed. Prepare again.');
      return r.value.handle;
    };
    const load = async () => {
      const scope = get().scope; if (!opened || !scope) return;
      const ticket = ++contextTicket; actionTicket++; readTicket++; readSequence = null; readStream = null; preparedCurrent = null;
      set({ loading: true, error: null, catalog: null, snapshot: null, readId: null, readRemote: null, readLoading: false, prepared: null, preview: null, preparing: false });
      const r = await api('get_remote_actions', { scope }).catch(() => ({ ok: false as const, error: transportError() }));
      if (ticket !== contextTicket || !live(scope) || !opened) return;
      if (!r.ok) { set({ loading: false, error: r.error }); return; }
      if (!sameActionScope(r.value.scope, scope)) { set({ loading: false, error: invalid('Remote context changed. Refresh the context.') }); return; }
      set({ loading: false, catalog: r.value });
    };
    const loadPublication = async (scope: ActionScope, name: string, ticket: number) => {
      try {
        const handle = await source(scope, name, 'branch'); const r = await api('get_publication_evidence', { scope, sourceHandle: handle });
        if (ticket !== publicationTicket || !live(scope)) return;
        if (!r.ok || !sameActionScope(r.value.scope, scope)) { set({ publications: [], publicationScope: null }); return; }
        const entry = sourceCatalog(scope)?.refs.find(ref => ref.name === name && ref.kind === 'local' && !ref.symbolicTarget);
        const observations = r.value.observations.filter(value => value.role === 'push' && value.sourceRef === name && value.sourceOid === entry?.oid);
        set({ publications: observations, publicationScope: scope });
      } catch { if (ticket === publicationTicket) set({ publications: [], publicationScope: null }); }
    };
    const observeRead = () => {
      const { readId, readRemote, scope } = get(); if (!readId || !readRemote || !scope || !live(scope)) return;
      const taskState = owner?.tasks.getState(), streamId = taskState?.snapshot?.streamId;
      const operation = taskState?.snapshot?.operations.find(o => o.id === readId);
      if (readStream && (readStream !== streamId || !operation)) {
        readTicket++; readSequence = null; readStream = null; preparedCurrent = null;
        set({ readId: null, snapshot: null, readLoading: false, prepared: null, preview: null, actionError: invalid('Remote read task was retired or replaced. Read again explicitly.') }); return;
      }
      if (operation && streamId) readStream = streamId;
      if (!operation || activeOperation(operation) || operation.sequence === readSequence) return;
      readSequence = operation.sequence; const ticket = readTicket;
      void (async () => {
        const r = await api('get_remote_action_outcome', { operationId: readId }).catch(() => ({ ok: false as const, error: transportError() }));
        const latest = owner?.tasks.getState().snapshot;
        if (ticket !== readTicket || get().readId !== readId || !opened || !live(scope) || !latest || latest.streamId !== streamId || !latest.operations.some(value => value.id === readId && value.sequence === operation.sequence && value.actionKind === 'readRemoteRefs')) return;
        if (!r.ok) { set({ readLoading: false, actionError: r.error }); return; }
        const snapshot = r.value.snapshot;
        if (r.value.operation.id !== readId || r.value.operation.kind !== 'action' || r.value.operation.actionKind !== 'readRemoteRefs' || r.value.operation.sequence !== operation.sequence || r.value.operation.repoId !== scope.repoId || r.value.operation.worktreeId !== scope.worktreeId || r.value.operation.state !== 'Succeeded' || r.value.availability !== 'available' || !snapshot || snapshot.role !== 'push' || snapshot.remoteName !== readRemote || !sameActionScope(snapshot.capturedScope, scope) || !(Date.parse(snapshot.expiresAt) > Date.now()) || !r.value.evidence || r.value.evidence.role !== 'push' || r.value.evidence.remoteName !== readRemote || r.value.evidence.phase !== 'observed') {
          set({ readLoading: false, snapshot: null, actionError: r.value.operation.error ?? invalid('Remote references are unavailable or expired. Inspect Tasks and read again explicitly.') }); return;
        }
        set({ snapshot, readLoading: false, actionError: null });
      })();
    };
    const bind = () => {
      const scope = owner?.history.getState().preferences.getState().scope ?? null, next = JSON.stringify(scope);
      if (next !== key) {
        key = next; contextTicket++; actionTicket++; readTicket++; executionTicket++; publicationTicket++; publicationKey = null; readSequence = null; readStream = null; preparedCurrent = null;
        set({ scope, catalog: null, loading: false, prepared: null, preview: null, preparing: false, executing: false, publications: [], publicationScope: null, readId: null, readRemote: null, readLoading: false, snapshot: null });
        if (scope && opened) void load();
      }
      const currentRefsKey = scope ? sourceCatalog(scope)?.catalogId ?? null : null;
      if (currentRefsKey !== refsKey) { refsKey = currentRefsKey; actionTicket++; preparedCurrent = null; set({ prepared: null, preview: null, preparing: false }); }
      if (scope) {
        const catalog = sourceCatalog(scope), branch = catalog?.head.branch;
        const tasks = owner?.tasks.getState().snapshot;
        const pushes = tasks?.operations.filter(operation => operation.kind === 'action' && operation.actionKind === 'push' && operation.repoId === scope.repoId && !activeOperation(operation)).map(operation => [operation.id, operation.sequence]);
        const nextPublication = catalog && catalog.head.kind === 'branch' && branch && catalog.head.oid ? JSON.stringify([scope, catalog.catalogId, branch, catalog.head.oid, tasks?.streamId, pushes]) : null;
        if (nextPublication !== publicationKey) { publicationKey = nextPublication; publicationTicket++; set({ publications: [], publicationScope: null }); if (nextPublication) void loadPublication(scope, `refs/heads/${branch}`, publicationTicket); }
      }
      observeRead();
    };
    const schedule = () => { if (scheduled) return; scheduled = true; queueMicrotask(() => { scheduled = false; if (owner) bind(); }); };
    return {
      scope: null, catalog: null, loading: false, error: null, readId: null, readRemote: null, readLoading: false, snapshot: null, prepared: null, preview: null, preparing: false, executing: false, actionError: null, publicationScope: null, publications: [],
      connect: (history, refs, tasks) => {
        if (owner && (owner.history !== history || owner.refs !== refs || owner.tasks !== tasks)) throw Error('Remote actions already have an owner');
        if (!owner) { const a = history.getState().preferences.subscribe(schedule), b = refs.subscribe(schedule), c = tasks.subscribe(schedule); owner = { history, refs, tasks, users: 0, stop: () => { a(); b(); c(); } }; schedule(); }
        const connection = owner; connection.users++; let disposed = false;
        return () => { if (disposed) return; disposed = true; connection.users--; queueMicrotask(() => { if (owner !== connection || connection.users) return; owner.stop(); owner = null; opened = false; readStream = null; key = null; refsKey = null; publicationKey = null; preparedCurrent = null; contextTicket++; actionTicket++; readTicket++; publicationTicket++; executionTicket++; set({ scope: null, catalog: null, snapshot: null, prepared: null, preview: null, preparing: false, executing: false, readId: null, readRemote: null, readLoading: false, publications: [], publicationScope: null }); }); };
      },
      begin: async () => { opened = true; await load(); }, end: () => { opened = false; readStream = null; contextTicket++; actionTicket++; readTicket++; executionTicket++; preparedCurrent = null; set({ catalog: null, loading: false, snapshot: null, readId: null, readRemote: null, readLoading: false, prepared: null, preview: null, preparing: false, executing: false, actionError: null }); }, reload: load,
      clearPrepared: () => { actionTicket++; preparedCurrent = null; set({ prepared: null, preview: null, preparing: false, actionError: null }); },
      clearRead: () => { actionTicket++; readTicket++; readSequence = null; readStream = null; preparedCurrent = null; set({ readId: null, readRemote: null, readLoading: false, snapshot: null, prepared: null, preview: null, preparing: false, actionError: null }); },
      readRemoteRefs: async remoteName => {
        const { scope, catalog } = get(); if (!opened || !scope || !catalog || !sameActionScope(catalog.scope, scope) || get().loading || get().error || !live(scope) || get().readLoading || get().preparing || get().executing) return;
        const handle = pushEndpoint(catalog, remoteName)?.handle;
        if (!handle) { set({ actionError: invalid('Choose an eligible single push endpoint.') }); return; }
        const tasks = owner?.tasks;
        const ticket = ++readTicket; readSequence = null; readStream = null; actionTicket++; preparedCurrent = null; set({ readLoading: true, readId: null, readRemote: remoteName, snapshot: null, prepared: null, preview: null, actionError: null });
        try {
          const p = await api('prepare_repository_action', { scope, intent: { kind: 'readRemoteRefs', remoteHandle: handle } }); if (ticket !== readTicket || !live(scope)) return;
          if (!p.ok) { set({ actionError: p.error, readLoading: false }); return; }
          if (get().catalog !== catalog || p.value.kind !== 'readRemoteRefs' || !sameActionScope(p.value.scope, scope) || !(Date.parse(p.value.expiresAt) > Date.now())) throw invalid('Read preview changed or expired. Read again explicitly.');
          const r = await api('execute_repository_action', { planId: p.value.planId });
          if (ticket === readTicket && opened && live(scope)) {
            if (!r.ok) set({ actionError: r.error, readLoading: false });
            else if (r.value.kind !== 'action' || r.value.actionKind !== 'readRemoteRefs' || r.value.repoId !== scope.repoId || r.value.worktreeId !== scope.worktreeId) throw invalid('Read reply could not be matched. Check Tasks.');
            else { set({ readId: r.value.id }); observeRead(); }
          }
        } catch (e) { if (ticket === readTicket) set({ readLoading: false, actionError: typeof e === 'object' && e && 'code' in e ? e as AppError : transportError() }); }
        finally { await tasks?.getState().sync(); }
      },
      prepare: async draft => {
        const { scope, catalog, snapshot } = get(); if (!opened || !scope || !catalog || !sameActionScope(catalog.scope, scope) || get().loading || get().error || !live(scope) || get().preparing || get().executing || get().readLoading) return;
        const refCatalog = sourceCatalog(scope); if (!refCatalog) { set({ actionError: invalid('Wait for the current local reference catalog.') }); return; }
        const ticket = ++actionTicket;
        const contextCurrent = () => opened && live(scope) && get().catalog === catalog && sourceCatalog(scope)?.catalogId === refCatalog.catalogId && (draft.kind === 'pull' || !!snapshot && get().snapshot === snapshot && sameActionScope(snapshot.capturedScope, scope) && Date.parse(snapshot.expiresAt) > Date.now());
        const current = () => ticket === actionTicket && contextCurrent();
        set({ preparing: true, prepared: null, preview: null, actionError: null });
        try {
          let intent: ActionIntent, from = draft.sourceRef, destination = '';
          if (draft.kind === 'pull') { const handle = await source(scope, draft.sourceRef, 'remoteRef'); intent = { kind: 'pull', upstreamHandle: handle }; destination = 'Current branch (fast-forward only)'; }
          else {
            const handle = pushEndpoint(catalog, draft.remoteName)?.handle;
            if (!handle || !snapshot || !sameActionScope(snapshot.capturedScope, scope) || snapshot.remoteName !== draft.remoteName || snapshot.role !== 'push' || !(Date.parse(snapshot.expiresAt) > Date.now())) throw invalid('Read the selected remote push endpoint before preparing a destination.');
            const namespace = draft.kind === 'push' ? 'branch' as const : 'tag' as const;
            const entry = snapshot.refs.find(r => r.entryId === draft.entryId);
            if (draft.entryId !== null && (!entry || !entry.name.startsWith(namespace === 'branch' ? 'refs/heads/' : 'refs/tags/'))) throw invalid('Select a destination of the correct kind from the current remote observation.');
            if (draft.kind === 'remoteTagDelete' && !entry) throw invalid('Select the existing remote tag to delete.');
            const src = draft.entryId === null ? { kind: 'new' as const, namespace, name: draft.newName } : { kind: 'existing' as const, entryId: draft.entryId };
            const r = await api('resolve_remote_target', { scope, snapshotId: snapshot.snapshotId, source: src }); if (!current()) return;
            if (!r.ok) throw r.error;
            const expected = entry?.name ?? `${namespace === 'branch' ? 'refs/heads/' : 'refs/tags/'}${draft.newName}`;
            if (!sameActionScope(r.value.scope, scope) || r.value.snapshotId !== snapshot.snapshotId || r.value.namespace !== namespace || r.value.name !== expected || r.value.expectedOid !== (entry?.oid ?? null)) throw invalid('Resolved destination changed. Prepare again.');
            destination = `${draft.remoteName}: ${r.value.name}`;
            if (draft.kind === 'remoteTagDelete') { intent = { kind: 'remoteTagDelete', remoteHandle: handle, tagHandle: r.value.handle }; from = `Observed object ${r.value.expectedOid ?? 'unavailable'}`; }
            else { const local = await source(scope, draft.sourceRef, draft.kind === 'push' ? 'branch' : 'tag'); if (!current()) return; intent = draft.kind === 'push' ? { kind: 'push', remoteHandle: handle, sourceHandle: local, destinationHandle: r.value.handle, setUpstream: false } : { kind: 'remoteTagPush', remoteHandle: handle, tagHandle: local, destinationHandle: r.value.handle }; }
          }
          if (!current()) return;
          const r = await api('prepare_repository_action', { scope, intent }); if (!current()) return;
          if (!r.ok) throw r.error;
          if (r.value.kind !== draft.kind || !sameActionScope(r.value.scope, scope) || !(Date.parse(r.value.expiresAt) > Date.now())) throw invalid('Preview changed or expired. Prepare again.');
          preparedCurrent = contextCurrent; set({ prepared: r.value, preview: { source: from, destination } });
        } catch (e) { if (ticket === actionTicket && live(scope)) set({ actionError: typeof e === 'object' && e && 'code' in e ? e as AppError : transportError() }); }
        finally { if (ticket === actionTicket) set({ preparing: false }); }
      },
      execute: async () => {
        const p = get().prepared; if (!p || get().executing || get().preparing || !live(p.scope)) return null;
        if (!preparedCurrent?.()) { set({ prepared: null, preview: null, actionError: invalid('Repository context or remote observation changed or expired. Read and prepare again explicitly.') }); return null; }
        if (!(Date.parse(p.expiresAt) > Date.now())) { set({ prepared: null, actionError: invalid('Preview expired. Prepare again.') }); return null; }
        const execution = ++executionTicket; preparedCurrent = null; set({ executing: true, prepared: null, actionError: null });
        try { const r = await api('execute_repository_action', { planId: p.planId }).catch(() => ({ ok: false as const, error: transportError() }));
          if (r.ok && (r.value.kind !== 'action' || r.value.actionKind !== p.kind || r.value.repoId !== p.scope.repoId || r.value.worktreeId !== p.scope.worktreeId)) { const e = invalid('Execution reply could not be matched. Check Tasks before preparing another action.'); if (execution === executionTicket && live(p.scope)) set({ actionError: e }); return { ok: false as const, error: e }; }
          if (!r.ok && execution === executionTicket && live(p.scope)) set({ actionError: r.error }); return r;
        } finally { if (execution === executionTicket) set({ executing: false }); }
      },
    };
  });
}
export const repositoryTransportStore = createRepositoryTransportStore();
