import { render, screen } from '@testing-library/react';
import { it, expect } from 'vitest';
import { StatusList } from './StatusList';
import { RepositorySummary, RepositoryInfo } from './RepositorySummary';
import type { RepositorySnapshot } from '../../contracts';
import fixtures from '../../../fixtures/contracts/wire.json';
const snapshot = fixtures.snapshots[0] as unknown as RepositorySnapshot;
it('shows rename paths with separate index and working-tree statuses', () => {
  render(<StatusList snapshot={snapshot} />);
  expect(screen.getByRole('table', { name: 'File status' })).toBeInTheDocument();
  expect(screen.getByText('new name.txt')).toBeInTheDocument(); expect(screen.getByText('from old name.txt')).toBeInTheDocument();
  expect(screen.getByText('Renamed')).toBeInTheDocument(); expect(screen.getByText('Unchanged')).toBeInTheDocument();
});
it('shows unborn and bare metadata with no invented working-tree status', () => {
  render(<><RepositorySummary snapshot={fixtures.snapshots[1] as unknown as RepositorySnapshot} /><StatusList snapshot={fixtures.snapshots[2] as unknown as RepositorySnapshot} /></>);
  expect(screen.getByText('No commits yet')).toBeInTheDocument(); expect(screen.getByText(/This bare repository has no working directory/)).toBeInTheDocument(); expect(screen.queryByRole('table')).not.toBeInTheDocument();
});
it('warns about existing operations without offering unsupported recovery', () => {
  render(<><RepositorySummary snapshot={{ ...snapshot, gitOperations: ['merge'] }} /><RepositoryInfo snapshot={snapshot} /></>);
  expect(screen.getByText('Git operation in progress')).toBeInTheDocument(); expect(screen.getByText('Git identity is incomplete')).toBeInTheDocument(); expect(screen.queryByRole('button', { name: /abort|continue|stage|commit/i })).not.toBeInTheDocument();
});
