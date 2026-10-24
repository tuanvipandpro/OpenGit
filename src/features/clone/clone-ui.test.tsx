import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, it, expect, vi } from 'vitest';
import { CloneDialog } from './CloneDialog';
import { AuthDialog } from '../tasks/AuthDialog';
import { TaskPanel } from '../tasks/TaskPanel';
import { directoryError, sourceError } from './validation';
import { createOperationsStore } from '../../stores/operations';
import { createWorkspaceStore } from '../../stores/workspace';
import type { Api } from '../../stores/workspace';
import type { ApiResult, AppError, AuthPrompt, OperationSnapshot, Workspace } from '../../contracts';
import App from '../../App';
import fixture from '../../../fixtures/contracts/wire.json';
const operation = fixture.event.operation as unknown as OperationSnapshot;
const ok = <T,>(value: T): ApiResult<T> => ({ ok: true, value });
const error: AppError = { code: 'DestinationExists', message: 'The destination already exists. Choose a different folder name.', phase: 'validation', retry: 'chooseAnotherDestination' };
const source = { sourceHandle: 'opaque-source', display: 'C:\\fixture\\source' };
const destination = { destinationHandle: 'opaque-destination', parentDisplay: 'C:\\fixture' };
function props() { return { onDismiss: vi.fn(), chooseSource: vi.fn(async () => ok(source)), chooseDestination: vi.fn(async () => ok(destination)), onStart: vi.fn(async () => ok(operation)) }; }
async function fillRemote(user: ReturnType<typeof userEvent.setup>) {
  await user.type(screen.getByRole('textbox', { name: 'Repository URL' }), 'https://example.invalid/team/repo.git');
  await user.click(screen.getByRole('button', { name: 'Choose destination folder' }));
  await user.type(screen.getByRole('textbox', { name: 'Folder name' }), 'clone-fixture');
}
function prompt(kind: AuthPrompt['kind'] = 'password'): AuthPrompt { return { promptHandle: 'one-time-handle', kind, message: 'Enter the repository password or access token.', expiresAt: new Date(Date.now() + 60000).toISOString() }; }
const cancel = () => vi.fn(async () => ok({ ...operation, state: 'Cancelled' as const, canCancel: false }));

describe('clone form', () => {
  it('starts a remote clone with opaque destination and prevents duplicate submission', async () => {
    const user = userEvent.setup(), actions = props();
    let resolve!: (value: ApiResult<OperationSnapshot>) => void;
    actions.onStart.mockImplementationOnce(() => new Promise(done => { resolve = done; }));
    render(<CloneDialog {...actions} />); await fillRemote(user);
    const form = screen.getByRole('dialog', { name: 'Clone repository' }).querySelector('form')!;
    fireEvent.submit(form); fireEvent.submit(form);
    expect(actions.onStart).toHaveBeenCalledTimes(1);
    expect(actions.onStart).toHaveBeenCalledWith({ source: { kind: 'remote', url: 'https://example.invalid/team/repo.git' }, destinationHandle: 'opaque-destination', directoryName: 'clone-fixture' });
    await act(async () => { resolve(ok(operation)); }); expect(actions.onDismiss).toHaveBeenCalledTimes(1);
  });
  it('uses backend source handles and retains a previous chooser selection after cancellation', async () => {
    const user = userEvent.setup(), actions = props();
    actions.chooseSource.mockResolvedValueOnce(ok(source)).mockResolvedValueOnce(ok(null) as ApiResult<typeof source>);
    render(<CloneDialog {...actions} />);
    await user.click(screen.getByRole('radio', { name: 'Local folder' }));
    await user.click(screen.getByRole('button', { name: 'Choose source folder' }));
    await user.click(screen.getByRole('button', { name: 'Choose source folder' }));
    expect(screen.getByText(source.display)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Choose destination folder' }));
    await user.type(screen.getByRole('textbox', { name: 'Folder name' }), 'local-clone');
    await user.click(screen.getByRole('button', { name: 'Start clone' }));
    expect(actions.onStart).toHaveBeenCalledWith({ source: { kind: 'local', sourceHandle: 'opaque-source' }, destinationHandle: 'opaque-destination', directoryName: 'local-clone' });
    expect(JSON.stringify(actions.onStart.mock.calls)).not.toContain(source.display);
  });
  it('rejects unsafe input before the backend and supports explicit correction after destination errors', async () => {
    const user = userEvent.setup(), actions = props();
    actions.onStart.mockResolvedValueOnce({ ok: false, error }).mockResolvedValueOnce(ok(operation));
    render(<CloneDialog {...actions} />); await fillRemote(user);
    const name = screen.getByRole('textbox', { name: 'Folder name' });
    await user.clear(name); await user.type(name, 'NUL.txt'); await user.click(screen.getByRole('button', { name: 'Start clone' }));
    expect(screen.getByRole('alert')).toHaveTextContent('valid Windows folder name'); expect(actions.onStart).not.toHaveBeenCalled();
    await user.clear(name); await user.type(name, 'existing'); await user.click(screen.getByRole('button', { name: 'Start clone' }));
    expect(screen.getByRole('alert')).toHaveTextContent(error.message); expect(actions.onDismiss).not.toHaveBeenCalled();
    await user.clear(name); await user.type(name, 'another'); await user.click(screen.getByRole('button', { name: 'Start clone' }));
    expect(actions.onDismiss).toHaveBeenCalledTimes(1);
  });
  it('rejects credential URLs without echoing them into validation messages', async () => {
    const user = userEvent.setup(), actions = props(); render(<CloneDialog {...actions} />);
    await user.type(screen.getByRole('textbox', { name: 'Repository URL' }), 'https://user:CANARY_SECRET@example.invalid/repo');
    await user.click(screen.getByRole('button', { name: 'Start clone' }));
    expect(screen.getByRole('alert')).not.toHaveTextContent('CANARY_SECRET'); expect(actions.onStart).not.toHaveBeenCalled();
  });
  it('dismisses via native dialog cancel without starting a clone', () => {
    const actions = props(); render(<CloneDialog {...actions} />);
    fireEvent(screen.getByRole('dialog'), new Event('cancel', { bubbles: true, cancelable: true }));
    expect(actions.onDismiss).toHaveBeenCalledTimes(1); expect(actions.onStart).not.toHaveBeenCalled();
  });
});
it.each(['NUL.txt','COM¹.repo','..','with/child','trailing.','bad\u0085name'])('rejects unsafe Windows name %s', value => { expect(directoryError(value)).not.toBeNull(); });
it.each(['https://example.invalid/repo','ssh://git@example.invalid/team/repo','git@example.invalid:team/repo'])('accepts supported remote shape %s', value => { expect(sourceError(value)).toBeNull(); });
it.each(['ext::helper','-option','C:\\repo','https://user:canary@example.invalid/repo','https://example.invalid/repo?token=canary'])('rejects unsafe remote shape %s', value => { expect(sourceError(value)).not.toBeNull(); });

describe('transient authentication', () => {
  it.each([['password','Password'], ['passphrase','Key passphrase']] as const)('masks %s and clears the value before awaiting a response', async (kind, label) => {
    const user = userEvent.setup(), respond = vi.fn(async () => ok(null));
    render(<AuthDialog sourceDisplay="example.invalid/team/repo" prompt={prompt(kind)} onRespond={respond} onCancel={cancel()} />);
    const input = screen.getByLabelText(label); expect(input).toHaveAttribute('type', 'password'); expect(input).toHaveAttribute('autocomplete', 'off');
    await user.type(input, 'CANARY_SECRET'); await user.click(screen.getByRole('button', { name: 'Continue' }));
    expect(respond).toHaveBeenCalledExactlyOnceWith('CANARY_SECRET'); expect(input).toHaveValue(''); expect(document.body).not.toHaveTextContent('CANARY_SECRET');
    expect(screen.getByRole('button', { name: 'Continue' })).toBeDisabled();
  });
  it('clears a typed secret when the prompt changes and when cancelling the task', async () => {
    const user = userEvent.setup(), respond = vi.fn(async () => ok(null)), onCancel = cancel();
    const { rerender } = render(<AuthDialog sourceDisplay="example.invalid/team/repo" key="first" prompt={prompt()} onRespond={respond} onCancel={onCancel} />);
    await user.type(screen.getByLabelText('Password'), 'CANARY_SECRET');
    rerender(<AuthDialog sourceDisplay="example.invalid/team/repo" key="second" prompt={{ ...prompt(), promptHandle: 'second-handle', kind: 'username' }} onRespond={respond} onCancel={onCancel} />);
    expect(screen.getByLabelText('Username')).toHaveValue('');
    await user.type(screen.getByLabelText('Username'), 'transient-user'); await user.click(screen.getByRole('button', { name: 'Cancel authentication' }));
    expect(onCancel).toHaveBeenCalledTimes(1); expect(screen.getByLabelText('Username')).toHaveValue(''); expect(respond).not.toHaveBeenCalled();
  });
  it.each([['Trust host','yes'],['Reject host','no']])('requires explicit host decision: %s', async (button, response) => {
    const user = userEvent.setup(), respond = vi.fn(async () => ok(null));
    const message = 'Verify SSH host example.invalid. Fingerprint SHA256:ABCDEFGHIJKLMNOPQRSTUV. Trust this host?';
    render(<AuthDialog sourceDisplay="example.invalid/team/repo" prompt={{ ...prompt('confirmation'), message }} onRespond={respond} onCancel={cancel()} />);
    expect(screen.getByText(message)).toBeInTheDocument(); expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: button })); expect(respond).toHaveBeenCalledExactlyOnceWith(response);
  });
  it('disables expired prompts and preserves safe failures without retaining secret text', async () => {
    const respond = vi.fn(async () => ok(null)); render(<AuthDialog sourceDisplay="example.invalid/team/repo" prompt={{ ...prompt(), expiresAt: '2000-01-01T00:00:00Z' }} onRespond={respond} onCancel={cancel()} />);
    expect(screen.getByRole('alert')).toHaveTextContent('expired'); expect(screen.getByRole('button', { name: 'Continue' })).toBeDisabled(); expect(respond).not.toHaveBeenCalled();
  });
  it('shows denied authentication and clears the submitted value', async () => {
    const user = userEvent.setup(), respond = vi.fn(async (): Promise<ApiResult<null>> => ({ ok: false, error: { code: 'AuthenticationFailed', message: 'Authentication failed. Check your credentials.', phase: 'authentication', retry: 'checkCredentials' } }));
    render(<AuthDialog sourceDisplay="example.invalid/team/repo" prompt={prompt()} onRespond={respond} onCancel={cancel()} />);
    await user.type(screen.getByLabelText('Password'), 'CANARY_SECRET'); await user.click(screen.getByRole('button', { name: 'Continue' }));
    expect(screen.getByRole('alert')).toHaveTextContent('Authentication failed.'); expect(screen.getByLabelText('Password')).toHaveValue('');
  });
});
it('shows four-step progress and authoritative success after a cancellation race', async () => {
  const user = userEvent.setup(), onCancel = vi.fn();
  const { rerender } = render(<TaskPanel operations={[{ ...operation, state: 'Running', phase: 'publish', progress: { completed: 3, total: 4, unit: 'steps' } }]} onCancel={onCancel} />);
  expect(screen.getByRole('progressbar', { name: 'Clone progress' })).toHaveAttribute('max','4');
  await user.click(screen.getByRole('button', { name: 'Cancel task' })); expect(onCancel).toHaveBeenCalledWith(operation.id);
  rerender(<TaskPanel operations={[{ ...operation, state: 'Succeeded', canCancel: false }]} onCancel={onCancel} />);
  expect(screen.getByText('Succeeded')).toBeInTheDocument(); expect(screen.queryByText('Cancelled')).not.toBeInTheDocument(); expect(screen.queryByRole('button', { name: 'Cancel task' })).not.toBeInTheDocument();
});
it('binds auth responses to the active prompt and never stores the response value', async () => {
  const api = vi.fn(async (command: string) => command === 'respond_auth_prompt' ? ok(null) : ok({ streamId: 'stream', sequence: '2', operations: [{ ...operation, state: 'Running', authPrompt: null }] })) as unknown as Api;
  const store = createOperationsStore(api); store.setState({ snapshot: { streamId: 'stream', sequence: '1', operations: [{ ...operation, state: 'WaitingForUser', authPrompt: prompt() }] } });
  const stale = await store.getState().respond(operation.id, 'stale-handle', 'CANARY_SECRET'); expect(stale.ok).toBe(false); expect(api).not.toHaveBeenCalled();
  await store.getState().respond(operation.id, 'one-time-handle', 'CANARY_SECRET');
  expect(api).toHaveBeenCalledWith('respond_auth_prompt', { operationId: operation.id, promptHandle: 'one-time-handle', value: 'CANARY_SECRET' }); expect(JSON.stringify(store.getState())).not.toContain('CANARY_SECRET');
});
it('reloads the real workspace after a newly observed clone succeeds', async () => {
  const workspace = fixture.commands[1].result.value as unknown as Workspace; let cloned = false;
  const api = vi.fn(async (command: string, request: Record<string, unknown>) => {
    if (command === 'get_environment') return fixture.commands[0].result;
    if (command === 'get_workspace') return ok(cloned ? workspace : { ...workspace, revision: '0', repositories: [], activeSelection: null });
    if (command === 'inspect_repository') return ok({ ...fixture.snapshots[0], ...request });
    if (command === 'get_operations') return ok({ streamId: 'stream', sequence: '0', operations: [] });
    throw Error('Unexpected request');
  }) as unknown as Api;
  const store = createWorkspaceStore(api), tasks = createOperationsStore(api, async () => () => {});
  render(<App store={store} tasks={tasks} />); await screen.findByRole('button', { name: 'Open a repository' });
  cloned = true; act(() => tasks.setState({ snapshot: { streamId: 'stream', sequence: '2', operations: [{ ...operation, state: 'Succeeded', canCancel: false }] } }));
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Fixture' })).toBeInTheDocument());
  expect(screen.getByRole('heading', { name: 'main' })).toBeInTheDocument();
});

it('shows the waiting operation source instead of another clone or the selected workspace repository', async () => {
  const workspace = fixture.commands[1].result.value as unknown as Workspace;
  const waiting = { ...operation, id: 'waiting-beta', state: 'WaitingForUser', sourceDisplay: 'example.invalid/team/beta', authPrompt: prompt() };
  const running = { ...operation, id: 'running-alpha', state: 'Running', sourceDisplay: 'example.invalid/team/alpha', authPrompt: null };
  const api = vi.fn(async (command: string, request: Record<string, unknown>) => {
    if (command === 'get_environment') return fixture.commands[0].result;
    if (command === 'get_workspace') return ok(workspace);
    if (command === 'inspect_repository') return ok({ ...fixture.snapshots[0], ...request });
    if (command === 'get_operations') return ok({ streamId: 'stream', sequence: '3', operations: [running, waiting] });
    throw Error('Unexpected request');
  }) as unknown as Api;
  const store = createWorkspaceStore(api), tasks = createOperationsStore(api, async () => () => {});
  render(<App store={store} tasks={tasks} />);
  const dialog = await screen.findByRole('dialog', { name: 'Repository authentication' });
  expect(within(dialog).getByText('example.invalid/team/beta')).toBeInTheDocument();
  expect(within(dialog).queryByText('example.invalid/team/alpha')).not.toBeInTheDocument();
  expect(within(dialog).queryByText('Fixture')).not.toBeInTheDocument();
  expect(within(dialog).getByLabelText('Password')).toHaveAttribute('type', 'password');
});
