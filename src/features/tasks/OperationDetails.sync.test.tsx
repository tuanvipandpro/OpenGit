import { render, screen, cleanup } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { OperationDetails } from './OperationDetails';
import type { OperationSnapshot, SyncActionResult } from '../../contracts';
import fixture from '../../../fixtures/contracts/ref-actions-wire.json';
const operation = (kind: 'push' | 'pull' | 'remoteTagDelete' = 'push'): OperationSnapshot => ({ ...fixture.result.operation as OperationSnapshot, actionKind: kind, state: 'Succeeded', error: null, outcome: { ...fixture.result.operation.outcome as NonNullable<OperationSnapshot['outcome']>, effects: 'applied', repositoryChanged: false } });
const result: SyncActionResult = { steps: [{ step: 'push', phase: 'observed', effects: 'applied', failureReason: null, observedAt: '2026-10-25T00:00:00Z' }, { step: 'tracking', phase: 'observed', effects: 'notApplied', failureReason: null, observedAt: '2026-10-25T00:00:00Z' }], publication: { remoteName: 'origin', role: 'push', sourceRef: 'refs/heads/main', sourceOid: 'a'.repeat(40), acknowledgedOid: 'a'.repeat(40), destination: 'refs/heads/main', acknowledgedAt: '2026-10-25T00:00:00Z', configDigest: 'digest', deleted: false }, trackingUpdated: false, upstreamUpdated: false, trackingRefreshRequired: true, fetchedOid: null, integratedOid: null };
it('Sync task reports remote Applied ACK separately from unchanged local tracking counts', () => {
  const value = operation(); render(<OperationDetails operation={value} result={{ operation: value, fetch: null, syncResult: result }} onCancel={vi.fn()} />);
  expect(screen.getByText('Changes applied')).toBeInTheDocument(); expect(screen.getByText(/Dated server ACK/)).toHaveTextContent('push endpoint'); expect(screen.getByText(/Local tracking was not refreshed/)).toHaveTextContent('Fetch explicitly');
  expect(screen.queryByRole('list', { name: 'Fetch remote results' })).not.toBeInTheDocument(); expect(screen.queryByText(/not pushed|never pushed/i)).not.toBeInTheDocument();
});
it('Sync task preserves fetch-applied + integration-failed Partial pull and useful divergence evidence', () => {
  const value = { ...operation('pull'), state: 'Failed' as const, outcome: { ...operation('pull').outcome!, effects: 'partial' as const, repositoryChanged: true } };
  render(<OperationDetails operation={value} result={{ operation: value, fetch: null, syncResult: { ...result, publication: null, trackingRefreshRequired: false, fetchedOid: 'b'.repeat(40), steps: [{ step: 'fetch', phase: 'observed', effects: 'applied', failureReason: null, observedAt: '2026-10-25' }, { step: 'fastForward', phase: 'observed', effects: 'notApplied', failureReason: 'diverged', observedAt: '2026-10-25' }] } }} onCancel={vi.fn()} />);
  expect(screen.getByText('Some changes may have been applied')).toBeInTheDocument(); expect(screen.getByRole('list', { name: 'Remote action steps' })).toHaveTextContent('fetch'); expect(screen.getByText(/Fast-forward-only pull did not merge or rebase/)).toBeInTheDocument(); expect(screen.getByText(/Fetched object/)).toHaveTextContent('b'.repeat(40)); expect(screen.queryByText(/Fast-forwarded to/)).not.toBeInTheDocument();
});
it('Sync task reports Unknown/missing evidence conservatively and refuses mismatched outcome identities', () => {
  const value = { ...operation(), state: 'Interrupted' as const, outcome: { ...operation().outcome!, effects: 'unknown' as const } };
  render(<OperationDetails operation={value} result={{ operation: value, fetch: null, syncResult: null }} onCancel={vi.fn()} />);
  expect(screen.getByText(/Effects unknown/)).toBeInTheDocument(); expect(screen.getByText(/Remote action evidence is unavailable/)).toBeInTheDocument(); cleanup();
  render(<OperationDetails operation={value} result={{ operation: { ...value, id: 'another-task' }, fetch: null, syncResult: result }} onCancel={vi.fn()} />);
  expect(screen.queryByText(/Dated server ACK/)).not.toBeInTheDocument();
});
it('Sync task reports authenticated read role, dated evidence and expired availability without fetch claims', () => {
  const value: OperationSnapshot = { ...operation(), actionKind: 'readRemoteRefs' };
  render(<OperationDetails operation={value} result={{ operation: value, fetch: null, remoteReadResult: { availability: 'expired', evidence: { remoteName: 'origin', role: 'push', phase: 'observed', refCount: '5', observedAt: '2026-10-25', noGitWrites: true } } }} onCancel={vi.fn()} />);
  expect(screen.getByText(/Remote observation availability: expired/)).toBeInTheDocument(); expect(screen.getByText(/5 references/)).toHaveTextContent('push endpoint'); expect(screen.getByText(/dated reference read does not fetch objects/)).toBeInTheDocument();
});
