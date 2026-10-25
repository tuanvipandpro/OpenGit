import { describe, expect, it } from 'vitest';
import { HISTORY_LIMITS } from '../../contracts/history';
import type { CommitSummary, HistoryParent } from '../../contracts/history';
import { appendHistoryLayout, createHistoryLayout, historyEdgeGeometry, historyPointToPixels, HISTORY_LAYOUT_METRICS } from './layout';
import type { HistoryLayout, HistoryPoint, HistorySegment } from './layout';
import oracle from '../../../fixtures/history/layout-oracle.json';

const commit = (oid: string, parents: string[] = [], extra: Partial<CommitSummary> = {}): CommitSummary => ({
  oid, handle: `handle-${oid}`, parents: parents.map(oid => ({ oid, boundary: 'offPage' })),
  authorName: 'Author', authorEmail: 'author@example.test', authoredAt: '2001-01-01T00:00:00Z',
  committedAt: '2001-01-01T00:00:00Z', subject: oid, decorations: [], ...extra,
});
const parent = (oid: string, boundary: HistoryParent['boundary']): HistoryParent => ({ oid, boundary });
function page(commits: readonly CommitSummary[], start: number, size: number) {
  const loaded = new Set(commits.slice(0, start + size).map(item => item.oid));
  return commits.slice(start, start + size).map(item => ({ ...item,
    parents: item.parents.map(p => ({ ...p, boundary: loaded.has(p.oid) ? 'loaded' as const : p.boundary })),
  }));
}
const edgeIds = (layout: HistoryLayout) => layout.edges.map(edge => `${edge.childOid}>${edge.parentOid}:${edge.parentIndex}`).sort();
const positions = (layout: HistoryLayout) => layout.nodes.map(node => ({ oid: node.commit.oid, row: node.row, column: node.column }));
// Independent geometric check: sharing an actual endpoint is valid; passing
// through any other node centre visually asserts a relationship that is false.
const onSegment = (point: HistoryPoint, segment: HistorySegment) => {
  const { from, to } = segment;
  const cross = (point.column - from.column) * (to.row - from.row) - (point.row - from.row) * (to.column - from.column);
  return cross === 0 && point.row >= Math.min(from.row, to.row) && point.row <= Math.max(from.row, to.row)
    && point.column >= Math.min(from.column, to.column) && point.column <= Math.max(from.column, to.column);
};
function expectNoNodeCollisions(layout: HistoryLayout) {
  const collisions: string[] = [];
  for (const edge of layout.edges) {
    const segments = historyEdgeGeometry(layout, edge).segments;
    for (const node of layout.nodes) {
      if (node.commit.oid === edge.childOid || node.commit.oid === edge.parentOid) continue;
      if (segments.some(segment => onSegment(node, segment))) collisions.push(`${edge.childOid}>${edge.parentOid} through ${node.commit.oid}`);
    }
  }
  expect(collisions).toEqual([]);
}

describe('incremental history layout uses raw ancestry', () => {
  // Octopus plus shared ancestry and an unrelated long-lived fork. Dates are
  // equal/backdated deliberately; they do not select adjacency or row order.
  const dag = [
    commit('M', ['A', 'B', 'C'], { committedAt: '1990-01-01T00:00:00Z', decorations: ['refs/heads/main', 'refs/tags/release'] }),
    commit('U', ['V']), commit('A', ['D']), commit('B', ['D']), commit('C', ['E']),
    commit('D', ['R']), commit('E', ['R']), commit('R'), commit('V'),
  ];
  const expectedEdges = ['M>A:0', 'M>B:1', 'M>C:2', 'U>V:0', 'A>D:0', 'B>D:0', 'C>E:0', 'D>R:0', 'E>R:0'].sort();

  it.each([1, 2, 200])('keeps all octopus/shared/unrelated edges and old row positions with page size %i', size => {
    let layout = createHistoryLayout();
    for (let start = 0; start < dag.length; start += size) {
      const old = layout, oldPositions = positions(old);
      layout = appendHistoryLayout(old, page(dag, start, size));
      expect(positions(layout).slice(0, old.nodes.length)).toEqual(oldPositions);
      expect(positions(old)).toEqual(oldPositions);
      expect(layout.nodesByOid.size).toBe(layout.nodes.length);
      expectNoNodeCollisions(layout);
      for (const edge of old.edges) {
        const before = historyEdgeGeometry(old, edge), after = historyEdgeGeometry(layout, edge);
        expect(after.segments[0]).toEqual(before.segments[0]);
      }
    }
    expect(edgeIds(layout)).toEqual(expectedEdges);
    expect(layout.nodes.map(node => node.commit.oid)).toEqual(['M', 'U', 'A', 'B', 'C', 'D', 'E', 'R', 'V']);
    expect(layout.nodes).toHaveLength(9); expect(layout.rootCount).toBe(2); expect(layout.frontier).toEqual([]);
    expect(layout.nodes[0].commit.decorations).toEqual(['refs/heads/main', 'refs/tags/release']);
    for (const edge of layout.edges) {
      const geometry = historyEdgeGeometry(layout, edge), target = layout.nodesByOid.get(edge.parentOid)!;
      expect(geometry.boundary).toBe('loaded'); expect(geometry.end).toEqual({ row: target.row, column: target.column });
      expect(target.row).toBeGreaterThan(edge.from.row);
      expect(geometry.segments.length).toBeLessThanOrEqual(3);
      geometry.segments.slice(1).forEach((segment, i) => expect(segment.from).toEqual(geometry.segments[i].to));
    }
  });

  it('does not change final lane choices when the same topology is split differently', () => {
    const run = (size: number) => {
      let layout = createHistoryLayout();
      for (let start = 0; start < dag.length; start += size) layout = appendHistoryLayout(layout, page(dag, start, size));
      return layout;
    };
    expect(positions(run(1))).toEqual(positions(run(2)));
    expect(positions(run(2))).toEqual(positions(run(200)));
  });

  it.each([1, 2, 200])('matches the actual Git parent oracle across page size %i', size => {
    const commits = oracle.commits.map(item => commit(item.oid, item.parents, {
      decorations: oracle.refs.filter(ref => ref.commitOid === item.oid).map(ref => ref.name),
    }));
    let layout = createHistoryLayout();
    for (let start = 0; start < commits.length; start += size) {
      const old = positions(layout);
      layout = appendHistoryLayout(layout, page(commits, start, size));
      expect(positions(layout).slice(0, old.length)).toEqual(old);
      expectNoNodeCollisions(layout);
    }
    // Expected relationships come from Git, independently of lane assignment.
    expect(edgeIds(layout)).toEqual(oracle.commits.flatMap(item => item.parents.map((oid, i) => `${item.oid}>${oid}:${i}`)).sort());
    expect(layout.nodes).toHaveLength(45);
    expect(layout.rootCount).toBe(oracle.commits.filter(item => !item.parents.length).length);
    expect(layout.frontier).toEqual([]);
    expect(layout.nodes.some(node => node.commit.parents.length === 3)).toBe(true);
    expect(layout.nodes.some(node => node.commit.decorations.length > 1)).toBe(true);
    expect(layout.edges.every(edge => historyEdgeGeometry(layout, edge).boundary === 'loaded')).toBe(true);
  });

  it('reconciles stale offPage edges by OID without inventing a parent at the next row', () => {
    const first = appendHistoryLayout(createHistoryLayout(), [commit('tip', ['base'])]);
    expect(first.rootCount).toBe(0);
    expect(first.frontier.map(item => item.oid)).toEqual(['base']);
    expect(historyEdgeGeometry(first, first.edges[0]).boundary).toBe('offPage');
    const middle = appendHistoryLayout(first, [commit('unrelated')]);
    expect(edgeIds(middle)).toEqual(['tip>base:0']);
    const continuation = historyEdgeGeometry(middle, middle.edges[0]);
    expect(continuation.boundary).toBe('offPage'); expect(continuation.end.row).toBe(1.5);
    const complete = appendHistoryLayout(middle, [commit('base')]);
    expect(historyEdgeGeometry(complete, complete.edges[0]).boundary).toBe('loaded');
    expect(historyEdgeGeometry(complete, complete.edges[0]).end.row).toBe(2);
    expect(complete.edges[0]).toBe(first.edges[0]);
    expect(first.edges[0].boundary).toBe('offPage'); expect(first.nodes).toHaveLength(1);
    expectNoNodeCollisions(complete);
  });

  describe.each(['shallow', 'missing', 'outsideView'] as const)('S3A-01: promoting %s boundaries', boundary => {
    it.each([1, 2, 200])('connects the real parent without crossing unrelated nodes, page size %i', size => {
      const commits = [commit('cutoff', [], { parents: [parent('base', boundary)] }), commit('unrelated'), commit('base')];
      let layout = createHistoryLayout();
      for (let start = 0; start < commits.length; start += size) {
        const before = layout;
        // Keep the old terminal classification even if the OID is in this page:
        // promotion must reconcile by OID rather than depend on page partition.
        layout = appendHistoryLayout(layout, commits.slice(start, start + size));
        expect(positions(layout).slice(0, before.nodes.length)).toEqual(positions(before));
        for (const edge of before.edges) expect(historyEdgeGeometry(layout, edge).segments[0]).toEqual(historyEdgeGeometry(before, edge).segments[0]);
        expectNoNodeCollisions(layout);
      }
      const edge = layout.edges[0], geometry = historyEdgeGeometry(layout, edge), base = layout.nodesByOid.get('base')!;
      expect(edgeIds(layout)).toEqual(['cutoff>base:0']);
      expect(geometry.boundary).toBe('loaded'); expect(geometry.end).toEqual({ row: base.row, column: base.column });
      expect(layout.nodes).toHaveLength(3); expect(layout.rootCount).toBe(2); expect(layout.frontier).toEqual([]);
      expect(layout.boundaryLanes).toEqual([]);
      expect(layout.nodesByOid.get('unrelated')!.column).not.toBe(edge.column);
    });
  });

  it.each([1, 2, 200])('keeps terminal promotion and offPage merge corridors free of unrelated nodes, page size %i', size => {
    const commits = [commit('merge', ['left', 'right', 'cutoff']), commit('left', ['base']),
      commit('right', [], { parents: [parent('base', 'missing')] }),
      commit('cutoff', [], { parents: [parent('outside', 'outsideView')] }),
      commit('unrelated'), commit('base', ['root']), commit('outside', ['root']), commit('root')];
    let layout = createHistoryLayout();
    for (let start = 0; start < commits.length; start += size) {
      const before = positions(layout);
      layout = appendHistoryLayout(layout, commits.slice(start, start + size));
      expect(positions(layout).slice(0, before.length)).toEqual(before);
      expectNoNodeCollisions(layout);
    }
    expect(edgeIds(layout)).toEqual(['merge>left:0', 'merge>right:1', 'merge>cutoff:2', 'left>base:0', 'right>base:0', 'cutoff>outside:0', 'base>root:0', 'outside>root:0'].sort());
    expect(layout.edges.every(edge => historyEdgeGeometry(layout, edge).boundary === 'loaded')).toBe(true);
  });

  it('retains terminal corridors after a rejected page and shares them with later offPage references', () => {
    const first = appendHistoryLayout(createHistoryLayout(), [commit('cutoff', [], { parents: [parent('base', 'missing')] })]);
    const reservation = first.boundaryLanes;
    expect(reservation.map(item => item.oid)).toEqual(['base']); expect(first.frontier).toEqual([]);
    expect(() => appendHistoryLayout(first, [commit('unrelated'), commit('invalid', ['invalid'])])).toThrow(/child before/);
    expect(first.nodes).toHaveLength(1); expect(first.boundaryLanes).toBe(reservation);
    const shared = appendHistoryLayout(first, [commit('second-child', ['base'])]);
    expect(shared.frontier.map(item => item.oid)).toEqual(['base']); expect(shared.boundaryLanes).toEqual([]);
    expect(historyEdgeGeometry(shared, shared.edges[0]).boundary).toBe('missing');
    const complete = appendHistoryLayout(shared, [commit('unrelated'), commit('base')]);
    expect(edgeIds(complete)).toEqual(['cutoff>base:0', 'second-child>base:0']);
    expect(complete.edges[0].column).toBe(complete.edges[1].column);
    expectNoNodeCollisions(complete); expect(complete.frontier).toEqual([]); expect(complete.boundaryLanes).toEqual([]);
  });

  it('retains shallow/missing/outsideView truth as explicit stubs and zero fake roots', () => {
    const layout = appendHistoryLayout(createHistoryLayout(), [commit('cutoff', [], { parents: [
      parent('shallow-parent', 'shallow'), parent('missing-parent', 'missing'), parent('filtered-parent', 'outsideView'),
    ] }), commit('other-root')]);
    expect(layout.nodes).toHaveLength(2); expect(layout.rootCount).toBe(1); expect(layout.frontier).toEqual([]);
    expect(layout.boundaryLanes.map(item => item.oid).sort()).toEqual(['filtered-parent', 'missing-parent', 'shallow-parent']);
    expect(edgeIds(layout)).toEqual(['cutoff>filtered-parent:2', 'cutoff>missing-parent:1', 'cutoff>shallow-parent:0']);
    expect(layout.edges.map(edge => historyEdgeGeometry(layout, edge).boundary)).toEqual(['shallow', 'missing', 'outsideView']);
    layout.edges.forEach(edge => expect(historyEdgeGeometry(layout, edge).end.row).toBe(0.5));
    // If an OID is explicitly supplied later, OID truth wins over old metadata.
    const later = appendHistoryLayout(layout, [commit('missing-parent')]);
    expect(historyEdgeGeometry(later, later.edges[1]).boundary).toBe('loaded');
    expect(later.boundaryLanes.map(item => item.oid).sort()).toEqual(['filtered-parent', 'shallow-parent']);
    expectNoNodeCollisions(later);
  });

  it('preserves raw duplicate parent indices and deduplicates only commit rows on replay', () => {
    const tip = commit('tip', ['base', 'base']), base = commit('base');
    const first = appendHistoryLayout(createHistoryLayout(), [tip]);
    const replay = appendHistoryLayout(first, [tip]); expect(replay).toBe(first);
    const complete = appendHistoryLayout(replay, [base]);
    expect(complete.nodes).toHaveLength(2); expect(edgeIds(complete)).toEqual(['tip>base:0', 'tip>base:1']);
    expect(appendHistoryLayout(complete, [{ ...tip, parents: [parent('base', 'loaded'), parent('base', 'loaded')] }, base])).toBe(complete);
  });

  it('clones DTO arrays and never writes to frozen inputs', () => {
    const input = commit('tip', ['base'], { decorations: ['refs/heads/topic'] });
    Object.freeze(input.parents[0]); Object.freeze(input.parents); Object.freeze(input.decorations); Object.freeze(input);
    const layout = appendHistoryLayout(createHistoryLayout(), Object.freeze([input]));
    expect(layout.nodes[0].commit).toEqual(input);
    expect(layout.nodes[0].commit.parents).not.toBe(input.parents);
    expect(layout.nodes[0].commit.decorations).not.toBe(input.decorations);
  });

  it('fails atomically on cycles, backwards pages, contradictory replay, and absent loaded parents', () => {
    const first = appendHistoryLayout(createHistoryLayout(), [commit('tip', ['base'])]);
    const before = positions(first);
    expect(() => appendHistoryLayout(first, [commit('base', ['tip'])])).toThrow(/child before/);
    expect(() => appendHistoryLayout(first, [commit('self', ['self'])])).toThrow(/child before/);
    expect(() => appendHistoryLayout(first, [commit('tip', ['different'])])).toThrow(/Conflicting/);
    expect(() => appendHistoryLayout(first, [commit('x', [], { parents: [parent('absent', 'loaded')] })])).toThrow(/absent/);
    const parentFirst = appendHistoryLayout(createHistoryLayout(), [commit('root')]);
    expect(() => appendHistoryLayout(parentFirst, [commit('child', ['root'])])).toThrow(/child before/);
    expect(positions(first)).toEqual(before); expect(first.frontier.map(item => item.oid)).toEqual(['base']);
    expect(appendHistoryLayout(first, [commit('base')]).rootCount).toBe(1);
  });

  it('bounds page and unique session rows, while an empty append preserves identity', () => {
    const empty = createHistoryLayout(); expect(appendHistoryLayout(empty, [])).toBe(empty);
    expect(() => appendHistoryLayout(empty, Array.from({ length: HISTORY_LIMITS.pageMax + 1 }, (_, i) => commit(String(i))))).toThrow(RangeError);
    // At the contract cap, reject before allocating another node (a sparse array
    // models the length guard without making the suite allocate 100k DTOs).
    const capped = { ...empty, nodes: Array(HISTORY_LIMITS.commitsPerSession) };
    expect(() => appendHistoryLayout(capped, [commit('overflow')])).toThrow(RangeError);
    expect(empty.nodes).toHaveLength(0);
  });

  it('shares row metrics with a future virtual list/Canvas projection', () => {
    const layout = appendHistoryLayout(createHistoryLayout(), [commit('a', ['b']), commit('b')]);
    expect(historyPointToPixels(layout.nodes[0]).y).toBe(HISTORY_LAYOUT_METRICS.rowHeight / 2);
    expect(historyPointToPixels(layout.nodes[1]).y - historyPointToPixels(layout.nodes[0]).y).toBe(HISTORY_LAYOUT_METRICS.rowHeight);
    expect(historyPointToPixels({ row: 2, column: 3 }, { rowHeight: 40, columnWidth: 20, paddingX: 10 })).toEqual({ x: 70, y: 100 });
  });

  it('lays out 10k commits with a fork spanning the history using bounded-size edge paths', () => {
    const count = 10_000;
    const commits = [commit('fork', ['last']), ...Array.from({ length: count - 2 }, (_, i) => commit(`c${i}`, [i === count - 3 ? 'last' : `c${i + 1}`])), commit('last')];
    let layout = createHistoryLayout();
    for (let start = 0; start < commits.length; start += 200) layout = appendHistoryLayout(layout, page(commits, start, 200));
    expect(layout.nodes).toHaveLength(count); expect(layout.edges).toHaveLength(count - 1);
    expect(layout.rootCount).toBe(1); expect(layout.columnCount).toBe(2); expect(layout.frontier).toEqual([]);
    expect(historyEdgeGeometry(layout, layout.edges[0]).end.row).toBe(count - 1);
    expect(layout.edges.reduce((total, edge) => total + historyEdgeGeometry(layout, edge).segments.length, 0)).toBeLessThanOrEqual(3 * (count - 1));
  });
});
