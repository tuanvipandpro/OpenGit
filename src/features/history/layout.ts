import { HISTORY_LIMITS } from '../../contracts/history';
import type { CommitSummary, HistoryParent, ParentBoundary } from '../../contracts/history';

// Coordinates use row/column units; integer rows are node centres. The same
// metrics can position virtual rows and project Canvas points after scrolling.
export const HISTORY_LAYOUT_METRICS = { rowHeight: 32, columnWidth: 18, paddingX: 12 } as const;
export type HistoryLayoutMetrics = { rowHeight: number; columnWidth: number; paddingX: number };
export type HistoryPoint = { readonly row: number; readonly column: number };
export type HistorySegment = { readonly from: HistoryPoint; readonly to: HistoryPoint };
type LayoutCommit = Omit<Readonly<CommitSummary>, 'parents' | 'decorations'> & {
  readonly parents: readonly Readonly<HistoryParent>[];
  readonly decorations: readonly string[];
};
export type HistoryNode = HistoryPoint & { readonly commit: LayoutCommit };
export type HistoryEdge = {
  readonly childOid: string;
  readonly parentOid: string;
  readonly parentIndex: number;
  readonly from: HistoryPoint;
  readonly column: number;
  readonly boundary: ParentBoundary;
};
export type HistoryFrontier = { readonly oid: string; readonly column: number };
export type HistoryLayout = {
  readonly nodes: readonly HistoryNode[];
  readonly nodesByOid: ReadonlyMap<string, HistoryNode>;
  readonly edges: readonly HistoryEdge[];
  readonly frontier: readonly HistoryFrontier[];
  // Terminal boundaries draw only stubs, but still reserve their actual OID's
  // corridor: a later loaded parent must not connect through an unrelated node.
  readonly boundaryLanes: readonly HistoryFrontier[];
  readonly columnCount: number;
  // Only objects with zero raw parents count as roots, never page boundaries.
  readonly rootCount: number;
};
export type HistoryEdgeGeometry = {
  readonly boundary: ParentBoundary;
  readonly segments: readonly HistorySegment[];
  readonly end: HistoryPoint;
};

export function createHistoryLayout(): HistoryLayout {
  return { nodes: [], nodesByOid: new Map(), edges: [], frontier: [], boundaryLanes: [], columnCount: 0, rootCount: 0 };
}

// Small min-heap: allocating lanes must not repeatedly scan every active fork.
function push(heap: number[], value: number) {
  let index = heap.length;
  heap.push(value);
  while (index > 0) {
    const parent = (index - 1) >>> 1;
    if (heap[parent] <= value) break;
    heap[index] = heap[parent]; index = parent;
  }
  heap[index] = value;
}
function pop(heap: number[]): number {
  const result = heap[0], tail = heap.pop()!;
  if (heap.length) {
    let index = 0;
    while (index * 2 + 1 < heap.length) {
      let child = index * 2 + 1;
      if (child + 1 < heap.length && heap[child + 1] < heap[child]) child++;
      if (heap[child] >= tail) break;
      heap[index] = heap[child]; index = child;
    }
    heap[index] = tail;
  }
  return result;
}
function sameCommit(a: LayoutCommit, b: CommitSummary) {
  return a.handle === b.handle && a.authorName === b.authorName && a.authorEmail === b.authorEmail
    && a.authoredAt === b.authoredAt && a.committedAt === b.committedAt && a.subject === b.subject
    && a.parents.length === b.parents.length && a.parents.every((parent, i) => parent.oid === b.parents[i].oid)
    && a.decorations.length === b.decorations.length && a.decorations.every((name, i) => name === b.decorations[i]);
}

/** Append one backend page from the SAME snapshot, in its supplied topological
 * order. Callers reset on scope/snapshot change. Replaying a page is idempotent;
 * contradictory metadata, backwards ancestry and limits fail atomically.
 * Existing nodes/edges keep their positions. Only continuation endpoints extend.
 */
export function appendHistoryLayout(layout: HistoryLayout, commits: readonly CommitSummary[]): HistoryLayout {
  if (commits.length > HISTORY_LIMITS.pageMax) throw new RangeError('History page exceeds the contract limit');
  if (!commits.length) return layout;
  const nodes = [...layout.nodes], nodesByOid = new Map(layout.nodesByOid), edges = [...layout.edges];
  const pending = new Map([...layout.frontier, ...layout.boundaryLanes].map(item => [item.oid, item.column]));
  const continuations = new Set(layout.frontier.map(item => item.oid));
  const occupied = new Set(pending.values()), free: number[] = [];
  let columnCount = layout.columnCount, rootCount = layout.rootCount;
  for (let column = 0; column < columnCount; column++) if (!occupied.has(column)) free.push(column);
  const take = (preferred?: number): number => {
    if (preferred !== undefined && !occupied.has(preferred)) { occupied.add(preferred); return preferred; }
    while (free.length) {
      const column = pop(free);
      if (!occupied.has(column)) { occupied.add(column); return column; }
    }
    const column = columnCount++; occupied.add(column); return column;
  };
  const release = (column: number) => { occupied.delete(column); push(free, column); };
  const added: HistoryNode[] = [];
  for (const commit of commits) {
    const previous = nodesByOid.get(commit.oid);
    if (previous) {
      if (!sameCommit(previous.commit, commit)) throw new Error('Conflicting history commit; reset the snapshot');
      continue;
    }
    if (nodes.length >= HISTORY_LIMITS.commitsPerSession) throw new RangeError('History session exceeds the contract limit');
    // Reject cycles/backwards pages instead of drawing an invented adjacency.
    if (commit.parents.some(parent => parent.oid === commit.oid || nodesByOid.has(parent.oid))) {
      throw new Error('History must put every child before its parents');
    }
    const column = pending.get(commit.oid) ?? take();
    pending.delete(commit.oid);
    continuations.delete(commit.oid);
    release(column);
    const node: HistoryNode = { row: nodes.length, column, commit: {
      ...commit, parents: commit.parents.map(parent => ({ ...parent })), decorations: [...commit.decorations],
    } };
    nodes.push(node); nodesByOid.set(commit.oid, node); added.push(node);
    if (!commit.parents.length) rootCount++;
    commit.parents.forEach((parent, parentIndex) => {
      let parentColumn = pending.get(parent.oid);
      if (parentColumn === undefined) {
        parentColumn = take(parentIndex === 0 ? column : undefined);
        // Reserve every unresolved actual parent until it is loaded. Releasing
        // a terminal lane early lets unrelated nodes occupy a future edge.
        pending.set(parent.oid, parentColumn);
      }
      if (parent.boundary === 'offPage' || parent.boundary === 'loaded') continuations.add(parent.oid);
      edges.push({ childOid: commit.oid, parentOid: parent.oid, parentIndex,
        from: { row: node.row, column }, column: parentColumn, boundary: parent.boundary });
    });
  }
  if (!added.length) return layout;
  for (const node of added) {
    if (node.commit.parents.some(parent => parent.boundary === 'loaded' && !nodesByOid.has(parent.oid))) {
      throw new Error('Loaded history parent is absent; reset the snapshot');
    }
  }
  const lanes = [...pending].map(([oid, column]) => ({ oid, column })).sort((a, b) => a.column - b.column);
  return { nodes, nodesByOid, edges, columnCount, rootCount,
    frontier: lanes.filter(item => continuations.has(item.oid)),
    boundaryLanes: lanes.filter(item => !continuations.has(item.oid)) };
}

/** Constant-size paths even for a fork spanning thousands of rows. The raw
 * edge identity is retained, and loaded OIDs supersede stale offPage metadata.
 * An external/shallow/missing end is an explicit half-row stub, never a node.
 */
export function historyEdgeGeometry(layout: HistoryLayout, edge: HistoryEdge): HistoryEdgeGeometry {
  const parent = layout.nodesByOid.get(edge.parentOid);
  const boundary = parent ? 'loaded' : edge.boundary;
  const departure = { row: edge.from.row + 0.5, column: edge.column };
  const end = parent ? { row: parent.row, column: parent.column }
    : { row: boundary === 'offPage' ? layout.nodes.length - 0.5 : departure.row, column: edge.column };
  const points = [edge.from, departure];
  if (parent) points.push({ row: parent.row - 0.5, column: edge.column });
  points.push(end);
  const segments: HistorySegment[] = [];
  for (let i = 1; i < points.length; i++) {
    if (points[i - 1].row !== points[i].row || points[i - 1].column !== points[i].column) {
      segments.push({ from: points[i - 1], to: points[i] });
    }
  }
  return { boundary, segments, end };
}

export function historyPointToPixels(point: HistoryPoint, metrics: HistoryLayoutMetrics = HISTORY_LAYOUT_METRICS) {
  return { x: metrics.paddingX + point.column * metrics.columnWidth, y: (point.row + 0.5) * metrics.rowHeight };
}
