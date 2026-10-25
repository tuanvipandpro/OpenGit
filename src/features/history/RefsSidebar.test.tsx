import { fireEvent, render, screen, within } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import type { HistoryFilter, HistoryRef } from '../../contracts/history';
import { RefsSidebar } from './RefsSidebar';
import type { RefsSidebarProps } from './RefsSidebar';
import oracle from '../../../fixtures/history/layout-oracle.json';

const refs: HistoryRef[] = oracle.refs.map(ref => ({ ...ref, kind: ref.kind as HistoryRef['kind'], displayName: ref.name, hidden: false }));
refs.push({ name: 'refs/stash', displayName: 'Latest stash', kind: 'stash', oid: 'stash-object', commitOid: 'stash-commit', symbolicTarget: null, hidden: false });
refs.push({ name: 'refs/stash/1', displayName: 'stash@{1}: Earlier work', kind: 'stash', oid: 'older-stash-object', commitOid: 'older-stash-commit', symbolicTarget: null, hidden: false });
refs.push({ name: 'refs/tags/tree', displayName: 'Tree tag', kind: 'tag', oid: 'tree-object', commitOid: null, symbolicTarget: null, hidden: false });
const setup = (filter: HistoryFilter = { hiddenRefs: [], soloRefs: [] }) => {
  const props: RefsSidebarProps = { refs, head: { ...oracle.head, kind: 'branch' }, filter, onFilterChange: vi.fn(), onSelectRef: vi.fn() };
  const view = render(<RefsSidebar {...props} />); return { props, view };
};
it('renders qualified local/remote/tag/stash groups and returns captured ref hints on selection', () => {
  const { props } = setup();
  expect(screen.getByText('Local branches')).toBeInTheDocument(); expect(screen.getByText('Remote origin')).toBeInTheDocument();
  expect(screen.getByText('Stashes')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Tree tag (not a commit)' })).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: 'Latest stash' }));
  expect(props.onSelectRef).toHaveBeenLastCalledWith(refs.find(ref => ref.name === 'refs/stash'));
  fireEvent.click(screen.getByRole('button', { name: 'stash@{1}: Earlier work' }));
  expect(props.onSelectRef).toHaveBeenLastCalledWith(refs.find(ref => ref.name === 'refs/stash/1'));
  expect(screen.queryByRole('button', { name: 'Solo Stashes' })).not.toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Hide refs/stash' })).not.toBeInTheDocument();
  expect(props.onFilterChange).not.toHaveBeenCalled();
  const tag = refs.find(ref => ref.name === 'refs/tags/v-fixture')!;
  fireEvent.click(screen.getByRole('button', { name: tag.displayName }));
  expect(props.onSelectRef).toHaveBeenLastCalledWith(tag); expect(tag.oid).not.toBe(tag.commitOid);
});
it('applies exact group Hide/Solo union, disjoint normalization, individual toggles and Show all', () => {
  const { props, view } = setup({ hiddenRefs: ['refs/heads/main'], soloRefs: ['refs/heads/topic/unmerged'] });
  fireEvent.click(screen.getByRole('button', { name: 'Solo Remote origin' }));
  const remoteNames = refs.filter(ref => ref.kind === 'remote').map(ref => ref.name);
  expect(props.onFilterChange).toHaveBeenLastCalledWith({ hiddenRefs: ['refs/heads/main'], soloRefs: ['refs/heads/topic/unmerged', ...remoteNames].sort() });
  const next: HistoryFilter = { hiddenRefs: remoteNames, soloRefs: ['refs/heads/main'] };
  view.rerender(<RefsSidebar {...props} filter={next} />);
  expect(screen.getByRole('status')).toHaveTextContent('2 hidden · 1 solo');
  fireEvent.click(screen.getByRole('button', { name: 'Solo Remote origin' }));
  expect(props.onFilterChange).toHaveBeenLastCalledWith({ hiddenRefs: [], soloRefs: ['refs/heads/main', ...remoteNames].sort() });
  fireEvent.click(screen.getByRole('button', { name: 'Hide Local branches' }));
  expect(props.onFilterChange).toHaveBeenLastCalledWith({ hiddenRefs: [...remoteNames, ...refs.filter(ref => ref.kind === 'local').map(ref => ref.name)].sort(), soloRefs: [] });
  fireEvent.click(screen.getByRole('button', { name: 'Show refs/remotes/origin/main' }));
  expect(props.onFilterChange).toHaveBeenLastCalledWith({ hiddenRefs: ['refs/remotes/origin/HEAD'], soloRefs: ['refs/heads/main'] });
  fireEvent.click(screen.getByRole('button', { name: 'Show all' }));
  expect(props.onFilterChange).toHaveBeenLastCalledWith({ hiddenRefs: [], soloRefs: [] });
});
it('announces empty/unborn, detached, loading and safe controlled errors', () => {
  const { props, view } = setup();
  view.rerender(<RefsSidebar {...props} refs={[]} head={{ kind: 'unborn', branch: 'main', oid: null }} loading error="References unavailable" />);
  expect(screen.getByText('HEAD has no commits.')).toBeInTheDocument(); expect(screen.getByRole('alert')).toHaveTextContent('References unavailable');
  expect(within(screen.getByRole('complementary')).getByText('Loading references…')).toBeInTheDocument();
  view.rerender(<RefsSidebar {...props} head={{ kind: 'detached', branch: null, oid: 'abcdef123456' }} />);
  expect(screen.getByText('HEAD detached · abcdef12')).toBeInTheDocument();
});
