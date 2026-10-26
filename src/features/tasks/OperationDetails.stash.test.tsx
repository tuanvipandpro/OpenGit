import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { OperationDetails } from './OperationDetails';
import type { OperationSnapshot, StashActionResult, StashRetention } from '../../contracts';
import fixture from '../../../fixtures/contracts/ref-actions-wire.json';
const operation: OperationSnapshot = { ...fixture.result.operation as OperationSnapshot, actionKind: 'stashApply', state: 'Failed', outcome: { ...fixture.result.operation.outcome as NonNullable<OperationSnapshot['outcome']>, effects: 'conflict' }, error: null };
it.each(['present', 'absent', 'unknown'] as StashRetention[])('S6 displays %s retention literally, conflict guidance and a scoped Reconcile control', retention => {
  const onReconcile = vi.fn(), result: StashActionResult = { stashOid: 'a'.repeat(40), stashRetention: retention, recoveryRefRetention: retention, resolution: 'unmerged', failureReason: 'conflict' };
  render(<OperationDetails operation={operation} onCancel={vi.fn()} result={{ operation, fetch: null, stashResult: result }} onReconcile={onReconcile} canReconcile />);
  expect(screen.getByText(`Stash object in stack: ${retention}. Recovery pin retention: ${retention}.`)).toBeInTheDocument(); expect(screen.getByText(/Unmerged paths remain/)).toHaveTextContent('Stage each resolved file');
  fireEvent.click(screen.getByRole('button', { name: 'Reconcile stash' })); expect(onReconcile).toHaveBeenCalledWith(operation.id); expect(screen.queryByRole('button', { name: /abort|drop|pop|reset/i })).not.toBeInTheDocument();
});
it('S6 preserves historical conflict effects while reporting Resolved externally and disables reconciliation outside current context', () => {
  render(<OperationDetails operation={operation} onCancel={vi.fn()} result={{ operation, fetch: null, stashResult: { stashOid: 'a'.repeat(40), stashRetention: 'present', recoveryRefRetention: 'unknown', resolution: 'resolvedExternally', failureReason: 'conflict' } }} onReconcile={vi.fn()} canReconcile={false} />);
  expect(screen.getByText('Resolved externally')).toBeInTheDocument(); expect(screen.getByText('Conflicts require attention')).toBeInTheDocument(); expect(screen.getByRole('button', { name: 'Reconcile stash' })).toBeDisabled();
});
it('S6 missing stash evidence is unknown rather than inferred absent', () => {
  render(<OperationDetails operation={operation} onCancel={vi.fn()} result={{ operation, fetch: null, stashResult: null }} />);
  expect(screen.getByText(/Stash effects and retention are unknown/)).toBeInTheDocument(); expect(screen.queryByText('Stash object in stack: absent.')).not.toBeInTheDocument();
});
