import { render, screen, cleanup } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { OperationDetails } from './OperationDetails';
import type { DisplayActionResult } from '../../stores/operations';
import type { OperationSnapshot } from '../../contracts';
import fixture from '../../../fixtures/contracts/ref-actions-wire.json';
it('S7 renders partial local effects and each step, including identity and signing failures', () => {
  const operation = fixture.result.operation as OperationSnapshot;
  const result: DisplayActionResult = { operation, fetch: null, refResult: { steps: [...fixture.result.result.steps as NonNullable<DisplayActionResult['refResult']>['steps'], { step: 'annotateTag', phase: 'started', effects: 'unknown', failureReason: 'identity' }, { step: 'signTag', phase: 'pending', effects: 'notApplied', failureReason: 'signing' }] } };
  render(<OperationDetails operation={operation} result={result} onCancel={vi.fn()} />);
  expect(screen.getByText('Some changes may have been applied')).toBeInTheDocument(); expect(screen.getByRole('list', { name: 'Local action steps' })).toHaveTextContent('createBranch');
  expect(screen.getByText(/Git identity is missing/)).toBeInTheDocument(); expect(screen.getByText(/Git signing failed/)).toBeInTheDocument(); expect(screen.getByText(/final effects are uncertain/)).toBeInTheDocument(); expect(screen.queryByRole('list', { name: 'Fetch remote results' })).not.toBeInTheDocument();
});
it('S7 describes missing local evidence conservatively and ignores an older outcome version', () => {
  const operation = fixture.unknown.operation as OperationSnapshot;
  render(<OperationDetails operation={operation} result={{ operation, fetch: null, refResult: null }} onCancel={vi.fn()} />);
  expect(screen.getByText(/Per-step local action evidence is unavailable/)).toBeInTheDocument(); expect(screen.getByText(/Effects unknown/)).toBeInTheDocument();
  cleanup();
  render(<OperationDetails operation={operation} result={{ operation: { ...operation, sequence: '2' }, fetch: null, refResult: fixture.result.result as DisplayActionResult['refResult'] }} onCancel={vi.fn()} />);
  expect(screen.queryByRole('list', { name: 'Local action steps' })).not.toBeInTheDocument();
});
