import { useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { ParentBoundary } from '../../contracts/history';
import { HISTORY_LAYOUT_METRICS, historyEdgeGeometry, historyPointToPixels } from './layout';
import type { HistoryEdge, HistoryLayout } from './layout';
import './history.css';

export type HistoryViewport = { width: number; height: number; scrollTop: number; scrollLeft: number; dpr: number };
type PixelPoint = { x: number; y: number };
export type HistoryGraphFrame = {
  edges: { childOid: string; parentOid: string; parentIndex: number; boundary: ParentBoundary; segments: { from: PixelPoint; to: PixelPoint }[] }[];
  nodes: { oid: string; point: PixelPoint }[];
  metrics: { visitedIntervals: number; candidateEdges: number; drawnEdges: number; drawnNodes: number; backingWidth: number; backingHeight: number };
};
export type HistoryGraphProps = { layout: HistoryLayout; viewport: HistoryViewport; selectedOid: string | null; headOid: string | null; onFrame?: (frame: HistoryGraphFrame) => void };
type Interval = { edge: HistoryEdge; start: number; end: number };
type IntervalTree = { item: Interval; minStart: number; maxEnd: number; left: IntervalTree | null; right: IntervalTree | null };
function indexIntervals(items: Interval[], start = 0, end = items.length): IntervalTree | null {
  if (start === end) return null;
  const middle = (start + end) >>> 1, item = items[middle];
  const left = indexIntervals(items, start, middle), right = indexIntervals(items, middle + 1, end);
  return { item, left, right, minStart: items[start].start, maxEnd: Math.max(item.end, left?.maxEnd ?? -Infinity, right?.maxEnd ?? -Infinity) };
}
function visibleIntervals(tree: IntervalTree | null, start: number, end: number, output: Interval[], visits: { count: number }) {
  if (!tree || tree.maxEnd < start || tree.minStart > end) return;
  visits.count++;
  if (tree.item.start <= end && tree.item.end >= start) output.push(tree.item);
  visibleIntervals(tree.left, start, end, output, visits); visibleIntervals(tree.right, start, end, output, visits);
}

/** Decorative Canvas; the adjacent virtual list owns accessible selection. */
export function HistoryGraph({ layout, viewport, selectedOid, headOid, onFrame }: HistoryGraphProps) {
  const canvas = useRef<HTMLCanvasElement>(null), callback = useRef(onFrame);
  const [unavailable, setUnavailable] = useState(false);
  callback.current = onFrame;
  useLayoutEffect(() => {
    const element = canvas.current;
    return () => { if (element) { element.width = 0; element.height = 0; } };
  }, []);
  const index = useMemo(() => indexIntervals(layout.edges.map(edge => ({ edge, start: edge.from.row,
    end: historyEdgeGeometry(layout, edge).end.row })).sort((a, b) => a.start - b.start)), [layout]);
  useLayoutEffect(() => {
    const element = canvas.current;
    if (!element) return;
    const { width, height, scrollTop, scrollLeft } = viewport;
    const dpr = Number.isFinite(viewport.dpr) && viewport.dpr > 0 ? viewport.dpr : 1;
    // Bitmap dimensions never depend on history length or logical lane width.
    const backingWidth = Math.ceil(Math.max(0, width) * dpr), backingHeight = Math.ceil(Math.max(0, height) * dpr);
    if (element.width !== backingWidth) element.width = backingWidth;
    if (element.height !== backingHeight) element.height = backingHeight;
    let context: CanvasRenderingContext2D | null;
    try { context = element.getContext('2d'); } catch { setUnavailable(true); return; }
    if (!context) { setUnavailable(true); return; }
    setUnavailable(false);
    context.setTransform(dpr, 0, 0, dpr, 0, 0); context.clearRect(0, 0, width, height);
    const graphWidth = layout.columnCount * HISTORY_LAYOUT_METRICS.columnWidth + HISTORY_LAYOUT_METRICS.paddingX * 2;
    context.save(); context.beginPath(); context.rect(0, 0, Math.max(0, Math.min(width, graphWidth - scrollLeft)), height); context.clip();
    const project = (point: { row: number; column: number }) => {
      const pixel = historyPointToPixels(point); return { x: pixel.x - scrollLeft, y: pixel.y - scrollTop };
    };
    const top = scrollTop / HISTORY_LAYOUT_METRICS.rowHeight - 0.5, bottom = (scrollTop + height) / HISTORY_LAYOUT_METRICS.rowHeight - 0.5;
    const candidates: Interval[] = [], visits = { count: 0 };
    visibleIntervals(index, top, bottom, candidates, visits);
    const edges: HistoryGraphFrame['edges'] = [], nodes: HistoryGraphFrame['nodes'] = [];
    for (const { edge } of candidates) {
      const geometry = historyEdgeGeometry(layout, edge);
      const segments = geometry.segments.map(segment => ({ from: project(segment.from), to: project(segment.to) }));
      if (!segments.some(segment => Math.max(segment.from.x, segment.to.x) >= -4 && Math.min(segment.from.x, segment.to.x) <= width + 4
        && Math.max(segment.from.y, segment.to.y) >= -4 && Math.min(segment.from.y, segment.to.y) <= height + 4)) continue;
      context.strokeStyle = '#61bfae'; context.lineWidth = 1.5;
      context.setLineDash(geometry.boundary === 'loaded' ? [] : [3, 3]);
      context.beginPath();
      for (const segment of segments) { context.moveTo(segment.from.x, segment.from.y); context.lineTo(segment.to.x, segment.to.y); }
      context.stroke(); context.setLineDash([]);
      const end = project(geometry.end);
      if (geometry.boundary !== 'loaded' && end.y >= 0 && end.y <= height) {
        context.fillStyle = '#d4b478'; context.fillRect(end.x - 2, end.y - 2, 4, 4);
      }
      edges.push({ childOid: edge.childOid, parentOid: edge.parentOid, parentIndex: edge.parentIndex, boundary: geometry.boundary, segments });
    }
    const first = Math.max(0, Math.ceil(top - 0.2)), last = Math.min(layout.nodes.length - 1, Math.floor(bottom + 0.2));
    for (let row = first; row <= last; row++) {
      const node = layout.nodes[row], point = project(node);
      if (point.x < -6 || point.x > width + 6) continue;
      context.beginPath(); context.arc(point.x, point.y, node.commit.oid === headOid ? 5 : 3.5, 0, Math.PI * 2);
      context.fillStyle = node.commit.oid === selectedOid ? '#f0d48c' : '#68cfbc'; context.fill();
      nodes.push({ oid: node.commit.oid, point });
    }
    context.restore();
    callback.current?.({ edges, nodes, metrics: { visitedIntervals: visits.count, candidateEdges: candidates.length, drawnEdges: edges.length,
      drawnNodes: nodes.length, backingWidth: element.width, backingHeight: element.height } });
  }, [layout, index, viewport, selectedOid, headOid]);
  return <><canvas ref={canvas} className="history-graph" aria-hidden="true" style={{ width: viewport.width, height: viewport.height }} />
    {unavailable && <p role="status" className="history-canvas-fallback">Graph drawing unavailable. Commit list remains available.</p>}</>;
}
