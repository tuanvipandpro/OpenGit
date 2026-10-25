import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { HistorySearch } from './HistorySearch';
import type { HistorySearchProps } from './HistorySearch';

const props = (): HistorySearchProps => ({ query: 'message', resultQuery: 'message', loading: false, hasMore: true,
  hits: [{ outsideView: true, commit: { oid: 'abcdef123456', handle: 'opaque-handle', parents: [], subject: 'A commit message', authorName: 'Full Author', authorEmail: 'author@example.test', authoredAt: '2000-01-01T00:00:00Z', committedAt: '2000-01-01T00:00:00Z', decorations: [] } }],
  refs: [{ name: 'refs/heads/main', displayName: 'main', oid: 'tip-oid', commitOid: 'commit-oid', kind: 'local', symbolicTarget: null, hidden: false }],
  onSearch: vi.fn(), onQueryChange: vi.fn(), onLoadMore: vi.fn(), onSelectCommit: vi.fn(), onSelectRef: vi.fn() });
it('submits a literal full-history query and reveals outside-filter results through callbacks only', () => {
  const value = props(); const view = render(<HistorySearch {...value} />);
  expect(screen.getByText('Search all history by SHA, message, author or reference name.')).toBeInTheDocument();
  fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'author literal.*' } });
  expect(value.onQueryChange).toHaveBeenCalledWith('author literal.*');
  view.rerender(<HistorySearch {...value} query="author literal.*" />);
  fireEvent.click(screen.getByRole('button', { name: 'Search' })); expect(value.onSearch).toHaveBeenCalledWith('author literal.*');
  fireEvent.click(screen.getByRole('button', { name: /A commit message/ }));
  expect(value.onSelectCommit).toHaveBeenCalledWith(value.hits[0]); expect(value.hits[0].outsideView).toBe(true);
  expect(screen.getByText('Outside current filters')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: 'Reference · main' })); expect(value.onSelectRef).toHaveBeenCalledWith(value.refs[0]);
  fireEvent.click(screen.getByRole('button', { name: 'Load more search results' })); expect(value.onLoadMore).toHaveBeenCalledOnce();
  expect(value.onQueryChange).toHaveBeenCalledOnce(); expect(screen.getByText('Results for “message”')).toBeInTheDocument();
});
it('handles loading, empty, errors and query byte limits without pretending to search locally', () => {
  const value = props(); const view = render(<HistorySearch {...value} hits={[]} refs={[]} loading />);
  expect(screen.getByRole('status')).toHaveTextContent('Searching history'); expect(screen.getByRole('button', { name: 'Load more search results' })).toBeDisabled();
  view.rerender(<HistorySearch {...value} hits={[]} refs={[]} hasMore={false} />);
  expect(screen.getByText('No matching commits or references.')).toBeInTheDocument();
  view.rerender(<HistorySearch {...value} error="History changed; search again" query={'é'.repeat(257)} />);
  expect(screen.getAllByRole('alert')).toHaveLength(2); expect(screen.getByRole('button', { name: 'Search' })).toBeDisabled();
  fireEvent.submit(screen.getByRole('searchbox').closest('form')!); expect(value.onSearch).not.toHaveBeenCalled();
  view.rerender(<HistorySearch {...value} query="" resultQuery={null} />);
  expect(screen.queryByLabelText('Search results')).not.toBeInTheDocument();
});

it('labels retained search results and disables their old handles during replay or failed refresh', () => {
  const value = props(), view = render(<HistorySearch {...value} loading retained />);
  expect(screen.getByRole('status')).toHaveTextContent('Previous search results are retained');
  fireEvent.click(screen.getByRole('button', { name: /A commit message/ })); fireEvent.click(screen.getByRole('button', { name: 'Reference · main' }));
  expect(value.onSelectCommit).not.toHaveBeenCalled(); expect(value.onSelectRef).not.toHaveBeenCalled();
  view.rerender(<HistorySearch {...value} retained />);
  expect(screen.getByRole('status')).toHaveTextContent('Refresh history or search again');
  expect(screen.getByRole('button', { name: /A commit message/ })).toBeDisabled(); expect(screen.getByRole('button', { name: 'Load more search results' })).toBeDisabled();
  view.rerender(<HistorySearch {...value} />);
  fireEvent.click(screen.getByRole('button', { name: /A commit message/ })); expect(value.onSelectCommit).toHaveBeenCalledOnce();
});
