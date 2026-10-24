import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { it, expect, vi } from 'vitest';
import App from './App';
import { createWorkspaceStore } from './stores/workspace';
import type { Api } from './stores/workspace';
import { createOperationsStore } from './stores/operations';
import type { Workspace, ToolInfo, Environment } from './contracts';
import fixture from '../fixtures/contracts/wire.json';
function makeServices(failed = false, git?: ToolInfo) {
  const workspace = fixture.commands[1].result.value as unknown as Workspace;
  const empty = { ...workspace, revision: '0', repositories: [], activeSelection: null };
  const api = vi.fn(async (command: string, request: Record<string, unknown>) => {
    if (command === 'get_environment') return git ? { ok: true, value: { ...(fixture.commands[0].result.value as unknown as Environment), git } } : fixture.commands[0].result;
    if (command === 'get_workspace') return failed ? fixture.error : { ok: true, value: empty };
    if (command === 'choose_repository') return { ok: true, value: workspace };
    if (command === 'inspect_repository') return { ok: true, value: { ...fixture.snapshots[0], ...request } };
    if (command === 'forget_repository') return { ok: true, value: { ...empty, revision: '9007199254740994' } };
    if (command === 'get_operations') return { ok: true, value: { streamId: 'stream', sequence: '0', operations: [] } };
    throw Error('Unexpected command');
  }) as unknown as Api;
  return { store: createWorkspaceStore(api), tasks: createOperationsStore(api, async () => () => {}) };
}
it('opens a real-service workspace response from empty view and forgets without deleting files', async () => {
  const user = userEvent.setup(); render(<App {...makeServices()} />);
  const open = await screen.findByRole('button', { name: 'Open a repository' });
  expect(screen.getByRole('button', { name: 'Clone a repository' })).toBeDisabled();
  await user.click(open);
  expect(await screen.findByRole('heading', { name: 'Fixture' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: 'main' })).toBeInTheDocument();
  expect(screen.getByRole('table', { name: 'File status' })).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Forget Fixture' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Open a repository' })).toBeInTheDocument());
  expect(screen.queryByRole('table')).not.toBeInTheDocument();
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
