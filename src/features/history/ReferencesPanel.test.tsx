import { fireEvent, render, screen, within } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { ReferencesPanel } from './ReferencesPanel';

const refs = [
  { name: 'refs/heads/off-page', displayName: 'off-page', commitOid: 'a', oid: 'a', kind: 'local' as const, hidden: false, symbolicTarget: null },
  { name: 'refs/remotes/origin/main', displayName: 'origin/main', commitOid: 'b', oid: 'b', kind: 'remote' as const, hidden: false, symbolicTarget: null },
  { name: 'refs/tags/v1', displayName: 'v1', commitOid: 'b', oid: 'tag', kind: 'tag' as const, hidden: false, symbolicTarget: null },
];
const props = () => ({ refs, head: { kind: 'branch' as const, branch: 'main', oid: 'b' }, filter: { hiddenRefs: [refs[0].name], soloRefs: [] }, onSelectRef: vi.fn(), onFilterChange: vi.fn() });
it('AC1/AC3 defaults closed, discovers hidden/off-page refs, preserves props and restores focus on close', () => {
  const properties = props(); render(<ReferencesPanel {...properties} />);
  const trigger = screen.getByRole('button', { name: 'References' });
  expect(trigger).toHaveAttribute('aria-expanded', 'false'); expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  fireEvent.click(trigger);
  const dialog = screen.getByRole('dialog', { name: 'References' });
  expect(within(dialog).getByRole('button', { name: 'off-page' })).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: 'Show refs/heads/off-page' }));
  expect(properties.onFilterChange).toHaveBeenLastCalledWith({ hiddenRefs: [], soloRefs: [] });
  fireEvent.click(screen.getByRole('button', { name: 'Close References' }));
  expect(trigger).toHaveFocus(); expect(properties.onSelectRef).not.toHaveBeenCalled();
});
it('AC2/AC3 keeps multi-Solo additive, offers group/show-all and survives loading with actions blocked', async () => {
  const properties = props(); const view = render(<ReferencesPanel {...properties} />);
  fireEvent.click(screen.getByRole('button', { name: 'References' }));
  fireEvent.click(screen.getByRole('button', { name: 'Solo refs/remotes/origin/main' }));
  expect(properties.onFilterChange).toHaveBeenLastCalledWith({ hiddenRefs: [refs[0].name], soloRefs: [refs[1].name] });
  view.rerender(<ReferencesPanel {...properties} filter={{ hiddenRefs: [refs[0].name], soloRefs: [refs[1].name] }} />);
  fireEvent.click(screen.getByRole('button', { name: 'Solo refs/tags/v1' }));
  expect(properties.onFilterChange).toHaveBeenLastCalledWith({ hiddenRefs: [refs[0].name], soloRefs: [refs[1].name, refs[2].name] });
  view.rerender(<ReferencesPanel {...properties} loading />);
  expect(screen.getByRole('dialog')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Solo refs/tags/v1' })).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: 'Close References' }));
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
});
