import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { ChangesPanel } from './ChangesPanel';
import { createWorkspaceStore } from '../../stores/workspace';
import type { Api } from '../../stores/workspace';
import { createRepositoryChangesStore } from '../../stores/repositoryChanges';
import { createHistoryStore } from '../../stores/history';
import { createOperationsStore } from '../../stores/operations';
import { defaultRepositoryView } from '../../stores/historyView';
import type { ActionIntent, ChangeSide, Workspace } from '../../contracts';
import fixture from '../../../fixtures/contracts/wire.json';
const ok = <T,>(value: T) => ({ ok: true as const, value });
const settle = async () => { for (let i = 0; i < 5; i++) await new Promise<void>(resolve => queueMicrotask(resolve)); };
function setup(extra?: (command: string, request: Record<string, unknown>) => unknown) {
  const workspaceValue = fixture.commands[1].result.value as unknown as Workspace;
  const api = vi.fn(async (command: string, request: Record<string, unknown>): Promise<unknown> => {
    const replacement = extra?.(command, request); if (replacement !== undefined) return replacement;
    if (command === 'get_workspace') return ok(workspaceValue);
    if (command === 'select_repository') return ok({ ...workspaceValue, activeSelection: request });
    if (command === 'get_environment') return fixture.commands[0].result;
    if (command === 'inspect_repository') return ok({ ...fixture.snapshots[0], ...request });
    if (command === 'get_repository_view' || command === 'save_repository_view') return ok({ scope: request.scope, viewRevision: '1', view: request.view ?? defaultRepositoryView() });
    if (command === 'open_history') { const context = { scope: request.scope, snapshotId: 'snapshot' }; return ok({ context, sessionId: 'session', head: { kind: 'branch', branch: 'main', oid: 'a'.repeat(40) }, refs: [], roots: [], page: { context, sessionId: 'session', commits: [], nextCursor: null } }); }
    if (command === 'get_repository_changes') return ok({ scope: request.scope, catalogId: 'changes', observedAt: '', head: { kind: 'branch', branch: 'main', oid: 'a'.repeat(40) }, unsupported: false, entries: [{ entryId: 'file', pathDisplay: 'selected.txt', previousPathDisplay: null, indexStatus: 'M', worktreeStatus: 'M', stageCapability: 'eligible', unstageCapability: 'eligible' }] });
    if (command === 'resolve_change_targets') { const target = (request.targets as { entryId: string; side: ChangeSide }[])[0]; return ok({ scope: request.scope, catalogId: request.catalogId, targets: [{ ...target, changeHandle: `source:${target.side}` }] }); }
    if (command === 'get_change_diff') return ok({ scope: request.scope, changeHandle: request.changeHandle, diffId: 'displayed-diff', side: (request.changeHandle as string).endsWith('unstaged') ? 'unstaged' : 'staged', partialEligibility: 'eligible', content: 'opaque', hunks: [{ hunkId: 'hunk', rows: [{ lineId: 'deleted', kind: 'delete', oldLine: 2, newLine: null, text: 'old', hasLf: true }, { lineId: 'added', kind: 'add', oldLine: null, newLine: 2, text: 'new', hasLf: true }] }] });
    if (command === 'resolve_change_selection') return ok({ scope: request.scope, changeHandle: request.changeHandle, diffId: request.diffId, selectionHandle: 'selection-proof' });
    if (command === 'prepare_repository_action') return ok({ scope: request.scope, planId: 'partial-plan', kind: (request.intent as ActionIntent).kind, warnings: [], expiresAt: '2099-01-01' });
    if (command === 'watch_repository') return ok({ scope: request.scope, subscriptionId: 'watch', sequence: '0', status: 'watching' });
    if (command === 'unwatch_repository') return ok(null);
    throw Error(command);
  });
  const history = createHistoryStore(api as unknown as Api), workspace = createWorkspaceStore(api as unknown as Api, async () => () => {}), store = createRepositoryChangesStore(api as unknown as Api), tasks = createOperationsStore(api as unknown as Api);
  const disconnect = workspace.getState().connectHistory(history);
  const view = render(<ChangesPanel history={history} workspace={workspace} store={store} tasks={tasks} />);
  const start = async () => { await act(async () => { await workspace.getState().initialize(); await settle(); }); };
  const dispose = async () => { view.unmount(); disconnect(); await settle(); };
  return { api, history, workspace, store, view, start, dispose };
}
it('S5-B partial form holds real passive watcher/fallback reads, prepares its original selection and releases one deferred refresh', async () => {
  const t = setup(); await t.start();
  try {
    await act(async () => fireEvent.click(within(screen.getByRole('region', { name: 'Unstaged changes' })).getByRole('button', { name: 'selected.txt' })));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Select deleted line 2' })); fireEvent.click(screen.getByRole('checkbox', { name: 'Select added line 2' }));
    const original = t.store.getState().diff!, counts = () => t.api.mock.calls.filter(([command]) => ['inspect_repository', 'resolve_change_targets', 'get_change_diff', 'get_repository_changes'].includes(command)).length, before = counts();
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Stage selected' })));
    expect(counts()).toBe(before);
    await act(async () => { await t.workspace.getState().requestRefresh('watch'); await t.workspace.getState().requestRefresh('fallback'); await t.workspace.getState().requestRefresh('focus'); });
    expect(counts()).toBe(before); expect(screen.getByRole('checkbox', { name: 'Select added line 2' })).toBeChecked();
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' })));
    expect(t.api).toHaveBeenCalledWith('resolve_change_selection', { scope: original.scope, changeHandle: original.changeHandle, diffId: original.diffId, selection: { kind: 'lines', lineIds: ['deleted', 'added'] } });
    expect(t.api).toHaveBeenLastCalledWith('prepare_repository_action', { scope: original.scope, intent: { kind: 'stageSelection', selectionHandles: ['selection-proof'] } });
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Close' })); await settle(); });
    expect(t.api.mock.calls.filter(([command]) => command === 'inspect_repository')).toHaveLength(2); expect(t.store.getState().selection).toBeNull();
  } finally { await t.dispose(); }
});
it('S5-B an urgent same-selection refresh invalidates the partial form instead of silently selecting the replacement file', async () => {
  const t = setup(); await t.start();
  try {
    await act(async () => fireEvent.click(within(screen.getByRole('region', { name: 'Staged changes' })).getByRole('button', { name: 'selected.txt' })));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Select hunk 1' })); await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Unstage selected' })));
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Refresh context' })));
    expect(screen.getByRole('dialog', { name: 'Unstage selected' })).toBeInTheDocument(); expect(screen.getByText(/The selected diff changed/)).toBeInTheDocument(); expect(screen.getByRole('button', { name: 'Prepare' })).toBeDisabled();
    expect(t.api.mock.calls.filter(([command]) => command === 'resolve_change_targets')).toHaveLength(1); expect(t.api.mock.calls.some(([command]) => command === 'resolve_change_selection')).toBe(false);
  } finally { await t.dispose(); }
});
it('S5B-FORM01 explicit file click waits for an in-flight quiet refresh before resolving the stable entry', async () => {
  let delay = false, finish!: (value: unknown) => void, inspection!: Record<string, unknown>;
  const t = setup((command, request) => command === 'inspect_repository' && delay ? (inspection = request, new Promise(resolve => { finish = resolve; })) : undefined); await t.start();
  try {
    delay = true;
    let refreshing!: Promise<void>; act(() => { refreshing = t.workspace.getState().requestRefresh('watch'); });
    fireEvent.click(within(screen.getByRole('region', { name: 'Unstaged changes' })).getByRole('button', { name: 'selected.txt' })); expect(screen.getByRole('button', { name: 'Close diff' })).toBeInTheDocument(); expect(t.api.mock.calls.some(([command]) => command === 'resolve_change_targets')).toBe(false);
    await act(async () => { finish(ok({ ...fixture.snapshots[0], ...inspection })); await refreshing; await settle(); });
    expect(screen.getByRole('checkbox', { name: 'Select hunk 1' })).toBeEnabled();
    fireEvent.click(screen.getByRole('checkbox', { name: 'Select hunk 1' })); await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Stage selected' })));
    expect(screen.getByRole('button', { name: 'Prepare' })).toBeEnabled();
    expect(t.api.mock.calls.filter(([command]) => command === 'get_repository_changes')).toHaveLength(2); expect(t.api.mock.calls.filter(([command]) => command === 'resolve_change_targets')).toHaveLength(1); expect(t.api.mock.calls.filter(([command]) => command === 'get_change_diff')).toHaveLength(1);
  } finally { await t.dispose(); }
});
it('S5B-FORM01 slow diff survives real fallback ticks, watch and focus, then transfers its original capability into the partial form', async () => {
  vi.useFakeTimers(); let finish!: (value: unknown) => void, request!: Record<string, unknown>;
  const t = setup((command, value) => command === 'get_change_diff' ? (request = value, new Promise(resolve => { finish = resolve; })) : undefined); await t.start();
  try {
    const begin = vi.spyOn(t.workspace.getState(), 'beginInteraction');
    await act(async () => fireEvent.click(within(screen.getByRole('region', { name: 'Unstaged changes' })).getByRole('button', { name: 'selected.txt' })));
    expect(screen.getByText('Loading diff…')).toBeInTheDocument();
    await act(async () => { await vi.advanceTimersByTimeAsync(61_000); await t.workspace.getState().requestRefresh('watch'); window.dispatchEvent(new Event('focus')); await vi.advanceTimersByTimeAsync(200); });
    expect(t.api.mock.calls.filter(([command]) => command === 'inspect_repository')).toHaveLength(1); expect(screen.getByText('Loading diff…')).toBeInTheDocument();
    await act(async () => finish(ok({ scope: request.scope, changeHandle: request.changeHandle, diffId: 'slow-displayed', side: 'unstaged', partialEligibility: 'eligible', content: 'opaque', hunks: [{ hunkId: 'slow-hunk', rows: [{ lineId: 'slow-line', kind: 'add', oldLine: null, newLine: 2, text: 'slow text', hasLf: true }] }] })));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Select added line 2' })); await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Stage selected' })));
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Prepare' })));
    expect(begin).toHaveBeenCalledOnce(); expect(t.api).toHaveBeenCalledWith('resolve_change_selection', { scope: request.scope, changeHandle: request.changeHandle, diffId: 'slow-displayed', selection: { kind: 'lines', lineIds: ['slow-line'] } });
    expect(t.api.mock.calls.filter(([command]) => command === 'resolve_change_targets')).toHaveLength(1); expect(t.api.mock.calls.filter(([command]) => command === 'get_change_diff')).toHaveLength(1);
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Close' })); await settle(); }); expect(t.api.mock.calls.filter(([command]) => command === 'inspect_repository')).toHaveLength(2);
  } finally { await t.dispose(); vi.useRealTimers(); }
});
it('S5B-FORM01 side switch reuses the inspection lease, clears selection and Close diff releases it once', async () => {
  const t = setup(); await t.start();
  try {
    const begin = vi.spyOn(t.workspace.getState(), 'beginInteraction');
    await act(async () => fireEvent.click(within(screen.getByRole('region', { name: 'Unstaged changes' })).getByRole('button', { name: 'selected.txt' })));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Select hunk 1' }));
    await act(async () => fireEvent.click(within(screen.getByRole('region', { name: 'Staged changes' })).getByRole('button', { name: 'selected.txt' })));
    expect(begin).toHaveBeenCalledOnce(); expect(t.store.getState().diff?.side).toBe('staged'); expect(t.store.getState().selection).toBeNull();
    await act(async () => t.workspace.getState().requestRefresh('watch'));
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Close diff' })); await settle(); });
    expect(t.store.getState().diff).toBeNull(); expect(t.api.mock.calls.filter(([command]) => command === 'inspect_repository')).toHaveLength(2);
    const next = t.workspace.getState().beginInteraction()!; expect(next).not.toBeNull(); next.release();
  } finally { await t.dispose(); }
});
it('S5B-FORM01 real selection ABA hard-closes inspection and fences its late diff reply', async () => {
  let finish!: (value: unknown) => void, request!: Record<string, unknown>;
  const t = setup((command, value) => command === 'get_change_diff' ? (request = value, new Promise(resolve => { finish = resolve; })) : undefined); await t.start();
  try {
    await act(async () => fireEvent.click(within(screen.getByRole('region', { name: 'Unstaged changes' })).getByRole('button', { name: 'selected.txt' })));
    const original = t.workspace.getState().workspace!.activeSelection!;
    await act(async () => { const leaving = t.workspace.getState().select({ repoId: 'other', worktreeId: null }); const returning = t.workspace.getState().select(original); await Promise.all([leaving, returning]); await settle(); });
    await act(async () => finish(ok({ scope: request.scope, changeHandle: request.changeHandle, diffId: 'old', side: 'unstaged', partialEligibility: 'eligible', content: 'stale', hunks: [] })));
    expect(screen.queryByRole('button', { name: 'Close diff' })).not.toBeInTheDocument(); expect(t.store.getState().diff).toBeNull(); expect(t.store.getState().selection).toBeNull();
    const next = t.workspace.getState().beginInteraction()!; expect(next).not.toBeNull(); next.release();
  } finally { await t.dispose(); }
});
it('S5B-FORM01 leaving the Changes tab releases a running inspection and ignores the late diff', async () => {
  let finish!: (value: unknown) => void, request!: Record<string, unknown>;
  const t = setup((command, value) => command === 'get_change_diff' ? (request = value, new Promise(resolve => { finish = resolve; })) : undefined); await t.start();
  try {
    await act(async () => fireEvent.click(within(screen.getByRole('region', { name: 'Unstaged changes' })).getByRole('button', { name: 'selected.txt' })));
    await act(async () => t.workspace.getState().requestRefresh('watch'));
    await act(async () => { t.view.rerender(<div>Other repository tab</div>); await settle(); });
    await act(async () => finish(ok({ scope: request.scope, changeHandle: request.changeHandle, diffId: 'late', side: 'unstaged', partialEligibility: 'eligible', content: 'late', hunks: [] })));
    expect(t.store.getState().diff).toBeNull(); expect(t.api.mock.calls.filter(([command]) => command === 'inspect_repository')).toHaveLength(2);
    const next = t.workspace.getState().beginInteraction()!; expect(next).not.toBeNull(); next.release();
  } finally { await t.dispose(); }
});
