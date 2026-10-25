import { render, screen } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import type { CommitSummary, ParentBoundary } from '../../contracts/history';
import { appendHistoryLayout, createHistoryLayout, historyEdgeGeometry, historyPointToPixels } from './layout';
import { HistoryGraph } from './HistoryGraph';
import type { HistoryGraphFrame } from './HistoryGraph';
import oracle from '../../../fixtures/history/layout-oracle.json';

const commit = (oid: string, parents: string[] = [], boundary: ParentBoundary = 'offPage'): CommitSummary => ({ oid, handle: oid, parents: parents.map(oid => ({ oid, boundary })),
  authorName: 'Author', authorEmail: 'a@example.test', authoredAt: '2000-01-01T00:00:00Z', committedAt: '2000-01-01T00:00:00Z', subject: oid, decorations: [] });
const context = { setTransform: vi.fn(), clearRect: vi.fn(), save: vi.fn(), restore: vi.fn(), beginPath: vi.fn(), rect: vi.fn(), clip: vi.fn(),
  setLineDash: vi.fn(), moveTo: vi.fn(), lineTo: vi.fn(), stroke: vi.fn(), fillRect: vi.fn(), arc: vi.fn(), fill: vi.fn() };
beforeEach(() => { vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(context as unknown as CanvasRenderingContext2D); });

it('projects only real Git parent edges and synchronizes scroll offsets, DPR and viewport resize', () => {
  const layout = appendHistoryLayout(createHistoryLayout(), oracle.commits.map(value => commit(value.oid, value.parents)));
  const frame = vi.fn<(frame: HistoryGraphFrame) => void>();
  const viewport = { width: 500, height: 1600, scrollTop: 0, scrollLeft: 0, dpr: 2 };
  const view = render(<HistoryGraph layout={layout} viewport={viewport} selectedOid={null} headOid={oracle.head.oid} onFrame={frame} />);
  const canvas = view.container.querySelector('canvas')!;
  expect([canvas.width, canvas.height]).toEqual([1000, 3200]);
  const result = frame.mock.calls.at(-1)![0];
  expect(result.edges.map(edge => `${edge.childOid}>${edge.parentOid}:${edge.parentIndex}`).sort()).toEqual(oracle.commits.flatMap(value => value.parents.map((oid, i) => `${value.oid}>${oid}:${i}`)).sort());
  expect(result.nodes).toHaveLength(45);
  for (const edge of result.edges) {
    const target = layout.nodesByOid.get(edge.parentOid)!;
    expect(edge.boundary).toBe('loaded'); expect(edge.segments.at(-1)!.to).toEqual(historyPointToPixels(target));
  }
  view.rerender(<HistoryGraph layout={layout} viewport={{ width: 300, height: 320, scrollTop: 640, scrollLeft: 8, dpr: 1.5 }} selectedOid={null} headOid={null} onFrame={frame} />);
  expect([canvas.width, canvas.height]).toEqual([450, 480]);
  const scrolled = frame.mock.calls.at(-1)![0];
  expect(scrolled.nodes.find(node => node.oid === layout.nodes[20].commit.oid)?.point).toEqual({ x: historyPointToPixels(layout.nodes[20]).x - 8, y: 16 });
  expect(context.setTransform).toHaveBeenLastCalledWith(1.5, 0, 0, 1.5, 0, 0);
});

it.each(['shallow', 'missing', 'outsideView'] as const)('draws %s boundary stubs and safely promoted actual parents', boundary => {
  const first = appendHistoryLayout(createHistoryLayout(), [commit('tip', ['base'], boundary), commit('unrelated')]);
  const frame = vi.fn<(frame: HistoryGraphFrame) => void>(), viewport = { width: 240, height: 160, scrollTop: 0, scrollLeft: 0, dpr: 2 };
  const view = render(<HistoryGraph layout={first} viewport={viewport} selectedOid="tip" headOid="tip" onFrame={frame} />);
  const stub = frame.mock.calls.at(-1)![0].edges[0];
  expect(stub.boundary).toBe(boundary); expect(stub.segments.at(-1)!.to.y).toBe(32);
  const complete = appendHistoryLayout(first, [commit('base')]);
  view.rerender(<HistoryGraph layout={complete} viewport={viewport} selectedOid="tip" headOid="tip" onFrame={frame} />);
  const promoted = frame.mock.calls.at(-1)![0].edges[0];
  expect(promoted.boundary).toBe('loaded'); expect(promoted.parentOid).toBe('base');
  expect(promoted.segments[0]).toEqual(stub.segments[0]);
  expect(promoted.segments.at(-1)!.to).toEqual(historyPointToPixels(complete.nodesByOid.get('base')!));
  expect(historyPointToPixels(complete.nodesByOid.get('unrelated')!).x).not.toBe(promoted.segments[1].from.x);
});

it('retains an offPage continuation and bounds Canvas memory/culling for 10k history and very wide lanes', () => {
  let layout = createHistoryLayout();
  for (let start = 0; start < 10_000; start += 200) layout = appendHistoryLayout(layout, Array.from({ length: 200 }, (_, offset) => {
    const row = start + offset; return commit(`c${row}`, row === 9999 ? ['off-page-parent'] : [`c${row + 1}`]);
  }));
  const frame = vi.fn<(frame: HistoryGraphFrame) => void>();
  const view = render(<HistoryGraph layout={{ ...layout, columnCount: 100_000 }} viewport={{ width: 320, height: 320, scrollTop: 5000 * 32, scrollLeft: 0, dpr: 2 }} selectedOid={null} headOid={null} onFrame={frame} />);
  const canvas = view.container.querySelector('canvas')!, result = frame.mock.calls.at(-1)![0];
  expect([canvas.width, canvas.height]).toEqual([640, 640]);
  // A 10-row viewport of a linear chain intersects at most 11 actual edges.
  expect(result.metrics.candidateEdges).toBeLessThanOrEqual(11);
  expect(result.nodes).toHaveLength(10); expect(result.metrics.visitedIntervals).toBeLessThan(layout.edges.length);
  view.rerender(<HistoryGraph layout={layout} viewport={{ width: 320, height: 320, scrollTop: 9990 * 32, scrollLeft: 0, dpr: 2 }} selectedOid={null} headOid={null} onFrame={frame} />);
  const continuation = frame.mock.calls.at(-1)![0].edges.find(edge => edge.parentOid === 'off-page-parent')!;
  expect(continuation.boundary).toBe('offPage'); expect(continuation.segments.at(-1)!.to.y).toBe(320);
  expect(historyEdgeGeometry(layout, layout.edges.at(-1)!).end.row).toBe(9999.5);
});

it('keeps an accessible fallback when Canvas cannot be created', () => {
  vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue(null);
  render(<HistoryGraph layout={createHistoryLayout()} viewport={{ width: 320, height: 160, scrollTop: 0, scrollLeft: 0, dpr: 1 }} selectedOid={null} headOid={null} />);
  expect(screen.getByRole('status')).toHaveTextContent('Commit list remains available');
});

it('draws a long-lived edge across the viewport when both real endpoints are offscreen and releases the bitmap on unmount', () => {
  const layout = appendHistoryLayout(createHistoryLayout(), [commit('tip', ['base']), commit('unrelated'), commit('base')]);
  const frame = vi.fn<(frame: HistoryGraphFrame) => void>();
  const view = render(<HistoryGraph layout={layout} viewport={{ width: 120, height: 32, scrollTop: 32, scrollLeft: 0, dpr: 2 }} selectedOid={null} headOid={null} onFrame={frame} />);
  const canvas = view.container.querySelector('canvas')!, result = frame.mock.calls.at(-1)![0];
  expect(result.nodes.map(node => node.oid)).toEqual(['unrelated']);
  expect(result.edges.map(edge => [edge.childOid, edge.parentOid])).toEqual([['tip', 'base']]);
  expect(result.edges[0].segments[0].from.y).toBe(-16); expect(result.edges[0].segments.at(-1)!.to.y).toBe(48);
  expect(context.rect).toHaveBeenLastCalledWith(0, 0, 60, 32);
  view.unmount(); expect([canvas.width, canvas.height]).toEqual([0, 0]);
});

it('retains the list fallback if the platform throws during Canvas creation', () => {
  vi.mocked(HTMLCanvasElement.prototype.getContext).mockImplementation(() => { throw Error('Context unavailable'); });
  render(<HistoryGraph layout={createHistoryLayout()} viewport={{ width: 320, height: 160, scrollTop: 0, scrollLeft: 0, dpr: 1 }} selectedOid={null} headOid={null} />);
  expect(screen.getByRole('status')).toHaveTextContent('Commit list remains available');
});
