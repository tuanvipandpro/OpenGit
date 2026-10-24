import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, it, expect, vi } from 'vitest';
import { createWorkspaceStore } from '../../stores/workspace';
import type { Api } from '../../stores/workspace';
import type { ApiResult, Workspace, RepositorySnapshot } from '../../contracts';
import fixtures from '../../../fixtures/contracts/wire.json';
import { RecentRepositories } from './RecentRepositories';
const initial = fixtures.commands[1].result.value as unknown as Workspace;
const sample = fixtures.snapshots[0] as unknown as RepositorySnapshot;
const ok = <T,>(value: T): ApiResult<T> => ({ ok: true, value });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
function mockApi(extra?: (command: string, request: Record<string, unknown>) => unknown) {
  return vi.fn(async (command: string, request: Record<string, unknown>) => {
    const custom = extra?.(command, request); if (custom !== undefined) return custom;
    if (command === 'get_workspace') return ok(initial);
    if (command === 'get_environment') return fixtures.commands[0].result;
    if (command === 'inspect_repository') return ok({ ...sample, ...request });
    if (command === 'choose_repository') return ok(null);
    throw Error('Unexpected command ' + command);
  });
}
describe('durable workspace and stale responses', () => {
  it('reopens the backend selection and inspects with a fresh generation', async () => {
    const api = mockApi(); const store = createWorkspaceStore(api as unknown as Api);
    await store.getState().initialize();
    expect(store.getState().workspace?.activeSelection).toEqual(initial.activeSelection);
    expect(store.getState().snapshot?.repoId).toBe(initial.activeSelection?.repoId);
    expect(api).toHaveBeenCalledWith('inspect_repository', { ...initial.activeSelection, generation: '2' });
    const reopened = createWorkspaceStore(api as unknown as Api); await reopened.getState().initialize();
    expect(reopened.getState().workspace?.activeSelection).toEqual(initial.activeSelection);
  });
  it('keeps selection on chooser cancellation and suppresses old workspace revisions', async () => {
    let chosen: Workspace | null = null;
    const api = mockApi(command => command === 'choose_repository' ? ok(chosen) : undefined);
    const store = createWorkspaceStore(api as unknown as Api); await store.getState().initialize();
    const snapshot = store.getState().snapshot; await store.getState().open();
    expect(store.getState().snapshot).toBe(snapshot);
    chosen = { ...initial, revision: '1', activeSelection: null }; await store.getState().open();
    expect(store.getState().workspace?.activeSelection).toEqual(initial.activeSelection);
  });
  it('prevents a delayed refresh from replacing a newer inspection', async () => {
    const requests: { request: Record<string, unknown>; pending: ReturnType<typeof deferred<ApiResult<RepositorySnapshot>>> }[] = [];
    let intercept = false;
    const api = mockApi((command, request) => {
      if (command === 'inspect_repository' && intercept) { const pending = deferred<ApiResult<RepositorySnapshot>>(); requests.push({ request, pending }); return pending.promise; }
    });
    const store = createWorkspaceStore(api as unknown as Api); await store.getState().initialize(); intercept = true;
    const old = store.getState().refresh(), latest = store.getState().refresh();
    requests[1].pending.resolve(ok({ ...sample, generation: String(requests[1].request.generation), head: { kind: 'branch', branch: 'latest', oid: 'new' } })); await latest;
    requests[0].pending.resolve(ok({ ...sample, generation: String(requests[0].request.generation), head: { kind: 'branch', branch: 'stale', oid: 'old' } })); await old;
    expect(store.getState().snapshot?.head.branch).toBe('latest');
  });
  it('serializes rapid switches so the final user choice is persisted', async () => {
    const switchResult = deferred<ApiResult<Workspace>>();
    let backend = initial; let first = true;
    const api = mockApi((command, request) => {
      if (command === 'get_workspace') return ok(backend);
      if (command === 'select_repository') {
        if (first) { first = false; return switchResult.promise; }
        backend = { ...initial, revision: '9007199254740995', activeSelection: request as unknown as Workspace['activeSelection'] }; return ok(backend);
      }
    });
    const store = createWorkspaceStore(api as unknown as Api); await store.getState().initialize();
    const firstSwitch = store.getState().select({ repoId: 'other', worktreeId: 'other-tree' }); await Promise.resolve();
    const finalSwitch = store.getState().select(initial.activeSelection!);
    switchResult.resolve(ok({ ...initial, revision: '9007199254740994', activeSelection: { repoId: 'other', worktreeId: 'other-tree' } }));
    await Promise.all([firstSwitch, finalSwitch]);
    expect(store.getState().snapshot?.repoId).toBe(initial.activeSelection?.repoId);
    const reopened = createWorkspaceStore(api as unknown as Api); await reopened.getState().initialize();
    expect(reopened.getState().workspace?.activeSelection).toEqual(initial.activeSelection);
  });
  it('shows safe service errors and handles an empty workspace', async () => {
    const api = mockApi(command => command === 'get_workspace' ? ok({ ...initial, repositories: [], activeSelection: null }) : command === 'choose_repository' ? fixtures.error : undefined);
    const store = createWorkspaceStore(api as unknown as Api); await store.getState().initialize(); await store.getState().open();
    expect(store.getState().workspace?.repositories).toEqual([]);
    expect(store.getState().snapshot).toBeNull(); expect(store.getState().error?.code).toBe('Certificate'); expect(store.getState().busy).toBe(false);
  });
});
it('filters repository names and exposes keyboard-accessible selection and forget controls', async () => {
  const user = userEvent.setup(), onSelect = vi.fn(), onForget = vi.fn();
  render(<RecentRepositories workspace={initial} onSelect={onSelect} onForget={onForget} />);
  await user.click(screen.getByRole('button', { name: 'Select Fixture' })); expect(onSelect).toHaveBeenCalledWith(initial.activeSelection);
  await user.click(screen.getByRole('button', { name: 'Forget Fixture' })); expect(onForget).toHaveBeenCalledWith('repo-fixture');
  await user.type(screen.getByRole('textbox', { name: 'Filter repositories' }), 'missing'); expect(screen.getByText('No matching repositories.')).toBeInTheDocument();
});
