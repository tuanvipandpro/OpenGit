import { StrictMode } from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { CommitSummary } from '../../contracts/history';
import { appendHistoryLayout, createHistoryLayout } from './layout';
import { CommitRows } from './CommitRows';
import type { CommitRowsProps } from './CommitRows';
import type { HistoryGraphFrame } from './HistoryGraph';

const commit = (row: number, last = false): CommitSummary => ({ oid: `c${row}`, handle: `handle-${row}`, parents: last ? [] : [{ oid: `c${row + 1}`, boundary: 'offPage' }],
  authorName: 'Author', authorEmail: 'a@example.test', authoredAt: '2000-01-01T00:00:00Z', committedAt: '2000-01-01T00:00:00Z', subject: `Commit ${row}`, decorations: row ? [] : ['refs/heads/main', 'refs/tags/v1'] });
function makeLayout(count: number) {
  let layout = createHistoryLayout();
  for (let start = 0; start < count; start += 200) layout = appendHistoryLayout(layout, Array.from({ length: Math.min(200, count - start) }, (_, i) => commit(start + i, start + i === count - 1)));
  return layout;
}
const originalScrollTo = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'scrollTo');
let width = 640, height = 320;
let observers: { notify: () => void; disconnect: ReturnType<typeof vi.fn>; unobserve: ReturnType<typeof vi.fn> }[];
beforeEach(() => {
  width = 640; height = 320; observers = [];
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(function (this: HTMLElement) { return this.classList.contains('history-scroll') ? width : 0; });
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockImplementation(function (this: HTMLElement) { return this.classList.contains('history-scroll') ? height : 0; });
  vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockImplementation(function (this: HTMLElement) { return Number.parseFloat((this.firstElementChild as HTMLElement | null)?.style.height ?? '0') || 0; });
  vi.spyOn(HTMLElement.prototype, 'scrollWidth', 'get').mockImplementation(function (this: HTMLElement) { return Number.parseFloat((this.firstElementChild as HTMLElement | null)?.style.width ?? '0') || 0; });
  Object.defineProperty(HTMLElement.prototype, 'scrollTo', { configurable: true, writable: true, value: vi.fn(function (this: HTMLElement, options: ScrollToOptions | number, y?: number) {
    if (typeof options === 'number') { this.scrollLeft = options; this.scrollTop = y ?? 0; }
    else { if (options.top !== undefined) this.scrollTop = options.top; if (options.left !== undefined) this.scrollLeft = options.left; }
    this.dispatchEvent(new Event('scroll'));
  }) });
  vi.stubGlobal('ResizeObserver', class {
    disconnect = vi.fn(); observe = vi.fn(); unobserve = vi.fn();
    constructor(notify: () => void) { observers.push({ notify: () => notify(), disconnect: this.disconnect, unobserve: this.unobserve }); }
  });
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ setTransform: vi.fn(), clearRect: vi.fn(), save: vi.fn(), restore: vi.fn(),
    beginPath: vi.fn(), rect: vi.fn(), clip: vi.fn(), setLineDash: vi.fn(), moveTo: vi.fn(), lineTo: vi.fn(), stroke: vi.fn(), fillRect: vi.fn(), arc: vi.fn(), fill: vi.fn() } as unknown as CanvasRenderingContext2D);
  vi.stubGlobal('devicePixelRatio', 2);
});
afterEach(() => {
  vi.restoreAllMocks(); vi.unstubAllGlobals();
  if (originalScrollTo) Object.defineProperty(HTMLElement.prototype, 'scrollTo', originalScrollTo);
  else delete (HTMLElement.prototype as unknown as { scrollTo?: unknown }).scrollTo;
});
const properties = (count: number): CommitRowsProps => ({ layout: makeLayout(count), selectedOid: 'c0', head: { kind: 'branch', branch: 'main', oid: 'c0' }, refs: [
  { name: 'refs/heads/main', displayName: 'main', commitOid: 'c0', oid: 'c0', kind: 'local', hidden: false, symbolicTarget: null },
  { name: 'refs/tags/v1', displayName: 'v1', commitOid: 'c0', oid: 'tag-object', kind: 'tag', hidden: false, symbolicTarget: null }],
  filter: { hiddenRefs: ['refs/tags/v1'], soloRefs: [] }, onSelect: vi.fn(), hasMore: true, loading: false, onLoadMore: vi.fn(), onAnchorChange: vi.fn() });

it('S7 dispatches the exact qualified same-commit ref action and preserves filter/selection/focus', async () => {
  const props = { ...properties(20), filter: { hiddenRefs: [], soloRefs: [] }, onSelectRef: vi.fn(), onFilterAction: vi.fn(), onAction: vi.fn(), onCommitAction: vi.fn() };
  const view = render(<CommitRows {...props} />);
  const tag = await screen.findByRole('button', { name: 'Actions for refs/tags/v1' }); fireEvent.click(tag);
  expect(screen.queryByRole('menuitem', { name: 'Switch branch' })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('menuitem', { name: 'Delete local tag' }));
  expect(props.onAction).toHaveBeenLastCalledWith({ kind: 'tagDelete', refName: 'refs/tags/v1' }); expect(tag).toHaveFocus();
  const branch = screen.getByRole('button', { name: 'Actions for refs/heads/main' }); fireEvent.click(branch);
  fireEvent.click(screen.getByRole('menuitem', { name: 'Set or unset upstream' }));
  expect(props.onAction).toHaveBeenLastCalledWith({ kind: 'branchTracking', refName: 'refs/heads/main' });
  expect(props.onFilterAction).not.toHaveBeenCalled(); expect(props.onSelect).not.toHaveBeenCalled();
  fireEvent.contextMenu(view.container.querySelector('[data-oid=c0]')!); expect(props.onSelect).toHaveBeenLastCalledWith('c0'); expect(props.onCommitAction).toHaveBeenLastCalledWith('c0');
});

it('uses real TanStack Virtual to bound 10k DOM rows, synchronize graph scrolling and preserve append/selection', async () => {
  const props = properties(10_000), frames = vi.fn<(frame: HistoryGraphFrame) => void>();
  const view = render(<StrictMode><CommitRows {...props} onGraphFrame={frames} /></StrictMode>);
  await waitFor(() => expect(screen.getAllByRole('row').length).toBeGreaterThan(0));
  // Ten visible rows + six overscan on each side, plus one partial row.
  expect(screen.getAllByRole('row').length).toBeLessThanOrEqual(23);
  expect(screen.getByText('main')).toBeInTheDocument(); expect(screen.queryByText('v1')).not.toBeInTheDocument();
  const scroll = screen.getByRole('grid', { name: 'Commits' });
  fireEvent.scroll(scroll, { target: { scrollTop: 5000 * 32 + 7, scrollLeft: 9 } });
  await waitFor(() => expect(view.container.querySelector('[data-oid=c5000]')).toBeInTheDocument());
  expect(screen.getAllByRole('row').length).toBeLessThanOrEqual(23);
  expect(props.onAnchorChange).toHaveBeenLastCalledWith({ oid: 'c5000', offset: 7 });
  const frame = frames.mock.calls.at(-1)![0], node = frame.nodes.find(value => value.oid === 'c5000')!;
  expect(node.point).toEqual({ x: 3, y: 9 });
  const row = view.container.querySelector('[data-oid=c5000]')!;
  expect((row as HTMLElement).style.transform).toBe('translateY(160000px)');
  fireEvent.click(row); expect(props.onSelect).toHaveBeenLastCalledWith('c5000'); expect(scroll).toHaveFocus();
  view.rerender(<StrictMode><CommitRows {...props} selectedOid="c5000" onGraphFrame={frames} /></StrictMode>);
  expect(row).toHaveAttribute('aria-selected', 'true');
  const before = scroll.scrollTop;
  const appended = appendHistoryLayout(props.layout, [{ ...commit(10_000, true), parents: [] }]);
  view.rerender(<StrictMode><CommitRows {...props} layout={appended} selectedOid="c5000" onGraphFrame={frames} /></StrictMode>);
  expect(scroll.scrollTop).toBe(before); expect(view.container.querySelector('[data-oid=c5000]')).toBe(row);
  const canvas = view.container.querySelector('canvas')!; expect([canvas.width, canvas.height]).toEqual([1280, 640]);
  width = 420; height = 192;
  act(() => { observers.filter(observer => !observer.disconnect.mock.calls.length && !observer.unobserve.mock.calls.length).forEach(observer => observer.notify()); window.dispatchEvent(new Event('resize')); });
  expect([canvas.width, canvas.height]).toEqual([840, 384]);
  expect(screen.getAllByRole('row').length).toBeLessThanOrEqual(19);
  view.unmount(); expect(observers.every(observer => observer.disconnect.mock.calls.length > 0 || observer.unobserve.mock.calls.length > 0)).toBe(true);
});

it('supports keyboard navigation and external selected-OID changes without fabricating rows', async () => {
  const props = properties(60), view = render(<CommitRows {...props} />), list = screen.getByRole('grid');
  fireEvent.keyDown(list, { key: 'ArrowDown' }); expect(props.onSelect).toHaveBeenLastCalledWith('c1');
  view.rerender(<CommitRows {...props} selectedOid="c1" />);
  fireEvent.keyDown(list, { key: 'ArrowUp' }); expect(props.onSelect).toHaveBeenLastCalledWith('c0');
  fireEvent.keyDown(list, { key: 'End' }); expect(props.onSelect).toHaveBeenLastCalledWith('c59');
  view.rerender(<CommitRows {...props} selectedOid="c59" />);
  await waitFor(() => expect(view.container.querySelector('[data-oid=c59]')).toBeInTheDocument());
  expect(list.getAttribute('aria-activedescendant')).toBe(view.container.querySelector('[data-oid=c59]')!.id);
  fireEvent.keyDown(list, { key: 'PageUp' }); expect(props.onSelect).toHaveBeenLastCalledWith('c49');
  fireEvent.keyDown(list, { key: 'Home' }); expect(props.onSelect).toHaveBeenLastCalledWith('c0');
});

it('restores the initial OID anchor and offers controlled loading/retry/empty states', async () => {
  const props = properties(100), view = render(<CommitRows {...props} initialAnchor={{ oid: 'c40', offset: 11 }} loading />);
  const list = screen.getByRole('grid'); await waitFor(() => expect(list.scrollTop).toBe(40 * 32 + 11));
  expect(screen.getByRole('status')).toHaveTextContent('Loading history'); expect(screen.getByRole('button', { name: 'Loading…' })).toBeDisabled();
  view.rerender(<CommitRows {...props} error="History changed; retry" />);
  fireEvent.click(screen.getByRole('button', { name: 'Retry loading history' })); expect(props.onLoadMore).toHaveBeenCalledOnce();
  expect(screen.getByRole('alert')).toHaveTextContent('History changed; retry');
  view.rerender(<CommitRows {...props} layout={createHistoryLayout()} hasMore={false} selectedOid={null} />);
  expect(screen.getByText('No commits in the current view.')).toBeInTheDocument(); expect(screen.queryAllByRole('row')).toHaveLength(0);
});

const firstPage = () => appendHistoryLayout(createHistoryLayout(), Array.from({ length: 20 }, (_, i) => commit(i)));
const extend = (layout: CommitRowsProps['layout'], count: number) => appendHistoryLayout(layout,
  Array.from({ length: count - layout.nodes.length }, (_, i) => commit(layout.nodes.length + i)));

it('S3B01 retains c40 +11 anchor until page append and never reports the default top over pending restoration', async () => {
  const props = { ...properties(20), layout: firstPage() }, anchor = { oid: 'c40', offset: 11 };
  const view = render(<StrictMode><CommitRows {...props} initialAnchor={anchor} /></StrictMode>), list = screen.getByRole('grid');
  expect(list.scrollTop).toBe(0); expect(props.onAnchorChange).not.toHaveBeenCalled();
  const later = extend(props.layout, 60);
  view.rerender(<StrictMode><CommitRows {...props} layout={later} initialAnchor={anchor} /></StrictMode>);
  await waitFor(() => expect(list.scrollTop).toBe(1291));
  expect(props.onAnchorChange).toHaveBeenLastCalledWith(anchor);
  expect(vi.mocked(props.onAnchorChange!).mock.calls.every(([value]) => value.oid === 'c40')).toBe(true);
  vi.mocked(HTMLElement.prototype.scrollTo).mockClear();
  view.rerender(<StrictMode><CommitRows {...props} layout={extend(later, 80)} initialAnchor={anchor} /></StrictMode>);
  expect(list.scrollTop).toBe(1291); expect(HTMLElement.prototype.scrollTo).not.toHaveBeenCalled();
});

it('S3B01 keeps changed selection c0 to absent c40 pending and reveals it once after append', async () => {
  const props = { ...properties(20), layout: firstPage() }, view = render(<CommitRows {...props} />), list = screen.getByRole('grid');
  vi.mocked(props.onAnchorChange!).mockClear();
  view.rerender(<CommitRows {...props} selectedOid="c40" />);
  expect(list.scrollTop).toBe(0); expect(props.onAnchorChange).not.toHaveBeenCalled();
  const later = extend(props.layout, 60);
  view.rerender(<CommitRows {...props} layout={later} selectedOid="c40" />);
  await waitFor(() => expect(list.scrollTop).toBeGreaterThan(0));
  expect(view.container.querySelector('[data-oid=c40]')).toBeInTheDocument();
  const top = list.scrollTop;
  vi.mocked(HTMLElement.prototype.scrollTo).mockClear();
  view.rerender(<CommitRows {...props} layout={extend(later, 80)} selectedOid="c40" />);
  expect(list.scrollTop).toBe(top); expect(HTMLElement.prototype.scrollTo).not.toHaveBeenCalled();
});

it('lets the latest selection supersede a pending anchor and any earlier selected OID', async () => {
  const props = { ...properties(20), layout: firstPage() }, anchor = { oid: 'c40', offset: 11 };
  const view = render(<CommitRows {...props} initialAnchor={anchor} />), list = screen.getByRole('grid');
  view.rerender(<CommitRows {...props} selectedOid="c41" initialAnchor={anchor} />);
  view.rerender(<CommitRows {...props} selectedOid="c50" initialAnchor={anchor} />);
  const middle = extend(props.layout, 45);
  view.rerender(<CommitRows {...props} layout={middle} selectedOid="c50" initialAnchor={anchor} />);
  expect(list.scrollTop).toBe(0); expect(props.onAnchorChange).not.toHaveBeenCalled();
  view.rerender(<CommitRows {...props} layout={extend(middle, 60)} selectedOid="c50" initialAnchor={anchor} />);
  await waitFor(() => expect(list.scrollTop).toBeGreaterThan(0));
  expect(view.container.querySelector('[data-oid=c50]')).toBeInTheDocument(); expect(list.scrollTop).not.toBe(1291);
});

it.each(['wheel', 'touchStart', 'pointerDown', 'rowClick', 'keyboard', 'clearSelection'] as const)('cancels pending restoration on newer %s intent instead of yanking on a later page', async intent => {
  const props = { ...properties(20), layout: firstPage() }, anchor = { oid: 'c40', offset: 11 };
  const view = render(<CommitRows {...props} initialAnchor={anchor} />), list = screen.getByRole('grid');
  if (intent === 'wheel') fireEvent.wheel(list, { deltaY: 96 });
  if (intent === 'touchStart') fireEvent.touchStart(list);
  if (intent === 'pointerDown') fireEvent.pointerDown(list);
  if (intent === 'rowClick') fireEvent.click(view.container.querySelector('[data-oid=c0]')!);
  if (intent === 'keyboard') fireEvent.keyDown(list, { key: 'ArrowDown' });
  if (intent === 'clearSelection') view.rerender(<CommitRows {...props} selectedOid={null} initialAnchor={anchor} />);
  fireEvent.scroll(list, { target: { scrollTop: 96 } });
  await waitFor(() => expect(props.onAnchorChange).toHaveBeenLastCalledWith({ oid: 'c3', offset: 0 }));
  vi.mocked(HTMLElement.prototype.scrollTo).mockClear();
  view.rerender(<CommitRows {...props} layout={extend(props.layout, 60)} selectedOid={intent === 'clearSelection' ? null : props.selectedOid} initialAnchor={anchor} />);
  expect(list.scrollTop).toBe(96); expect(HTMLElement.prototype.scrollTo).not.toHaveBeenCalled();
});

it('does not reveal an initially saved selection in preference to the saved anchor', async () => {
  const props = { ...properties(20), layout: firstPage(), selectedOid: 'c50' }, anchor = { oid: 'c40', offset: 11 };
  const view = render(<CommitRows {...props} initialAnchor={anchor} />), list = screen.getByRole('grid');
  view.rerender(<CommitRows {...props} layout={extend(props.layout, 60)} initialAnchor={anchor} />);
  await waitFor(() => expect(list.scrollTop).toBe(1291));
});

it('preserves an already loaded initial anchor through StrictMode effect replay', async () => {
  const props = properties(60), anchor = { oid: 'c40', offset: 11 };
  render(<StrictMode><CommitRows {...props} initialAnchor={anchor} /></StrictMode>);
  await waitFor(() => expect(screen.getByRole('grid').scrollTop).toBe(1291));
  expect(vi.mocked(props.onAnchorChange!).mock.calls.every(([value]) => value.oid === 'c40' && value.offset === 11)).toBe(true);
});

it('cancels pending selection on manual scroll and can reveal a later explicit selection again', async () => {
  const props = { ...properties(20), layout: firstPage() }, view = render(<CommitRows {...props} />), list = screen.getByRole('grid');
  view.rerender(<CommitRows {...props} selectedOid="c40" />);
  fireEvent.wheel(list, { deltaY: 96 }); fireEvent.scroll(list, { target: { scrollTop: 96 } });
  const later = extend(props.layout, 60);
  view.rerender(<CommitRows {...props} layout={later} selectedOid="c40" />);
  expect(list.scrollTop).toBe(96);
  view.rerender(<CommitRows {...props} layout={later} selectedOid={null} />);
  view.rerender(<CommitRows {...props} layout={later} selectedOid="c40" />);
  await waitFor(() => expect(list.scrollTop).toBeGreaterThan(96));
  expect(view.container.querySelector('[data-oid=c40]')).toBeInTheDocument();
});

it('AC2 opens the exact qualified badge ref without selecting a row and applies actions through callbacks', async () => {
  const props = { ...properties(20), filter: { hiddenRefs: [], soloRefs: ['refs/heads/other'] }, actionContext: 'context', onSelectRef: vi.fn(), onFilterAction: vi.fn() };
  const view = render(<CommitRows {...props} />);
  const badge = await screen.findByRole('button', { name: 'Actions for refs/tags/v1' });
  fireEvent.pointerDown(badge); fireEvent.click(badge);
  expect(props.onSelect).not.toHaveBeenCalled();
  expect(screen.getByRole('menu', { name: 'Actions for refs/tags/v1' })).toBeInTheDocument();
  expect(screen.getByRole('menuitem', { name: 'View commit' })).toHaveFocus();
  fireEvent.click(screen.getByRole('menuitem', { name: 'Add to Solo' }));
  expect(props.onFilterAction).toHaveBeenLastCalledWith({ kind: 'solo', names: ['refs/tags/v1'] });
  expect(badge).toHaveFocus(); expect(props.onSelect).not.toHaveBeenCalled();
  fireEvent.contextMenu(badge); fireEvent.click(screen.getByRole('menuitem', { name: 'View commit' }));
  expect(props.onSelectRef).toHaveBeenLastCalledWith(props.refs[1]);
  fireEvent.keyDown(badge, { key: 'ArrowDown' }); expect(props.onSelect).not.toHaveBeenCalled();
  fireEvent.keyDown(badge, { key: 'F10', shiftKey: true }); expect(screen.getByRole('menu')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('menuitem', { name: 'Hide' }));
  expect(props.onFilterAction).toHaveBeenLastCalledWith({ kind: 'hide', names: ['refs/tags/v1'] });
  view.unmount();
});

it('AC4 closes ref menus on refresh, context change, scroll, resize, outside pointer and ref removal', async () => {
  const props = { ...properties(20), actionContext: 'first', onSelectRef: vi.fn(), onFilterAction: vi.fn() };
  const view = render(<CommitRows {...props} />);
  const open = () => fireEvent.click(screen.getByRole('button', { name: 'Actions for refs/heads/main' }));
  await screen.findByRole('button', { name: 'Actions for refs/heads/main' });
  open(); view.rerender(<CommitRows {...props} actionsLoading />); expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Actions for refs/heads/main' })).toHaveAttribute('aria-disabled', 'true');
  fireEvent.click(screen.getByRole('button', { name: 'Actions for refs/heads/main' }));
  fireEvent.contextMenu(screen.getByRole('button', { name: 'Actions for refs/heads/main' }));
  fireEvent.keyDown(screen.getByRole('button', { name: 'Actions for refs/heads/main' }), { key: 'F10', shiftKey: true });
  expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  view.rerender(<CommitRows {...props} />); open();
  view.rerender(<CommitRows {...props} actionContext="second" />); expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  view.rerender(<CommitRows {...props} />); open(); fireEvent.scroll(screen.getByRole('grid')); expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  open(); fireEvent.resize(window); expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  open(); fireEvent.pointerDown(document.body); expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  open(); view.rerender(<CommitRows {...props} refs={[]} />); expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  expect(props.onFilterAction).not.toHaveBeenCalled(); expect(props.onSelectRef).not.toHaveBeenCalled();
});

it('AC4 restores graph focus if virtualization evicts the badge trigger', async () => {
  const props = { ...properties(100), onSelectRef: vi.fn() };
  render(<CommitRows {...props} />);
  fireEvent.click(await screen.findByRole('button', { name: 'Actions for refs/heads/main' }));
  fireEvent.scroll(screen.getByRole('grid'), { target: { scrollTop: 80 * 32 } });
  await waitFor(() => expect(screen.queryByRole('button', { name: 'Actions for refs/heads/main' })).not.toBeInTheDocument());
  expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  expect(screen.getByRole('grid')).toHaveFocus();
});

it('AC2 leaves HEAD and unresolvable decorations as text and exposes only View for stash', async () => {
  const props = { ...properties(20), onSelectRef: vi.fn(), filter: { hiddenRefs: [], soloRefs: [] } };
  const layout = appendHistoryLayout(createHistoryLayout(), [{ ...commit(0, true), decorations: ['missing', 'refs/stash'] }]);
  render(<CommitRows {...props} layout={layout} refs={[{ ...props.refs[0], name: 'refs/stash', kind: 'stash', displayName: 'stash@{0}' }]} />);
  expect(await screen.findByText('HEAD')).not.toHaveAttribute('role', 'button');
  expect(screen.getByText('missing').tagName).toBe('SPAN');
  fireEvent.click(screen.getByRole('button', { name: 'Actions for refs/stash' }));
  expect(screen.getAllByRole('menuitem')).toHaveLength(1);
  fireEvent.click(screen.getByRole('menuitem', { name: 'View commit' })); expect(props.onSelectRef).toHaveBeenCalledOnce();
});
