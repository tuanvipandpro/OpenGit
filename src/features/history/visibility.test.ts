import { describe, expect, it } from 'vitest';
import type { HistoryFilter, HistoryRef } from '../../contracts/history';
import type { Head } from '../../contracts/models';
import { applyHistoryFilterAction, deriveHistoryVisibility, reconcileHistoryFilter, visibleHistoryDecorations } from './visibility';
import oracle from '../../../fixtures/history/layout-oracle.json';

const noFilter = (): HistoryFilter => ({ hiddenRefs: [], soloRefs: [] });
const ref = (name: string, commitOid: string | null, kind: HistoryRef['kind'] = 'local', extra: Partial<HistoryRef> = {}): HistoryRef => ({
  name, displayName: name, oid: `object-${name}`, commitOid, kind, symbolicTarget: null, hidden: false, ...extra,
});
const main = 'refs/heads/main', topic = 'refs/heads/topic', remote = 'refs/remotes/origin/main', remoteHead = 'refs/remotes/origin/HEAD';
const tag = 'refs/tags/v1', treeTag = 'refs/tags/tree', stash = 'refs/stash';
const refs = [ref(main, 'M'), ref(topic, 'T'), ref(remote, 'M', 'remote'),
  ref(remoteHead, 'M', 'remote', { symbolicTarget: remote }), ref(tag, 'M', 'tag'),
  ref(treeTag, null, 'tag'), ref(stash, 'S', 'stash')];
const head: Head = { kind: 'branch', branch: 'main', oid: 'M' };

describe('positive-root visibility and filters', () => {
  it('deduplicates branch/remote/symbolic/peeled-tag roots and leaves stash inspectable outside default history', () => {
    const view = deriveHistoryVisibility(refs, head, noFilter());
    expect(view.positiveRoots).toEqual(['M', 'T']);
    expect(view.decorationRefs.map(item => item.name)).toEqual([main, topic, remote, remoteHead, tag, treeTag, stash]);
    expect(view.status).toEqual({ active: false, hiddenCount: 0, soloCount: 0 });
    expect(deriveHistoryVisibility(refs, head, { hiddenRefs: [], soloRefs: [stash] }).positiveRoots).toEqual(['S']);
    // A noncommit annotated tag is a decoration/ref, never its tag-object root.
    expect(deriveHistoryVisibility(refs, head, { hiddenRefs: [], soloRefs: [treeTag] }).positiveRoots).toEqual([]);
    expect(deriveHistoryVisibility(refs, head, { hiddenRefs: [], soloRefs: [tag] }).positiveRoots).toEqual(['M']);
  });

  it('removes only the selected name root/decoration, retaining other refs and all shared ancestry for the backend', () => {
    const filter = { hiddenRefs: [main], soloRefs: [] };
    expect(deriveHistoryVisibility(refs, head, filter).positiveRoots).toEqual(['M', 'T']);
    expect(visibleHistoryDecorations([main, remote, tag, main], filter)).toEqual([remote, tag].sort());
    const onlyBranches = refs.slice(0, 2);
    expect(deriveHistoryVisibility(onlyBranches, head, filter).positiveRoots).toEqual(['T']);
    expect(deriveHistoryVisibility(onlyBranches, head, { hiddenRefs: [main, topic], soloRefs: [] }).positiveRoots).toEqual([]);
    // This helper returns positive OIDs only. It cannot subtract the M/T shared
    // ancestors or rewrite backend commit rows, because it has no ancestry input.
    expect(refs[0].hidden).toBe(false);
  });

  it('uses union Solo roots and retains backend hidden-overlap semantics', () => {
    expect(deriveHistoryVisibility(refs, head, { hiddenRefs: [], soloRefs: [main, topic, main] }).positiveRoots).toEqual(['M', 'T']);
    const overlap = { hiddenRefs: [main], soloRefs: [main] };
    const empty = deriveHistoryVisibility(refs, head, overlap);
    expect(empty.filter).toEqual(overlap); expect(empty.positiveRoots).toEqual([]);
    expect(empty.status).toEqual({ active: true, hiddenCount: 1, soloCount: 1 });
    expect(deriveHistoryVisibility(refs, head, { hiddenRefs: [main], soloRefs: [main, topic] }).positiveRoots).toEqual(['T']);
  });

  it('includes detached HEAD only among default roots; unborn/bare history may be empty', () => {
    const detached: Head = { kind: 'detached', branch: null, oid: 'DETACHED' };
    expect(deriveHistoryVisibility(refs, detached, noFilter()).positiveRoots).toEqual(['DETACHED', 'M', 'T']);
    expect(deriveHistoryVisibility(refs, detached, { hiddenRefs: [], soloRefs: [topic] }).positiveRoots).toEqual(['T']);
    expect(deriveHistoryVisibility([], detached, noFilter()).positiveRoots).toEqual(['DETACHED']);
    expect(deriveHistoryVisibility([], { kind: 'unborn', branch: 'main', oid: null }, noFilter()).positiveRoots).toEqual([]);
    expect(deriveHistoryVisibility([], { kind: 'detached', branch: null, oid: null }, noFilter()).positiveRoots).toEqual([]);
  });

  it('reconciles deleted refs as exact names and canonical sets without mutating saved preferences', () => {
    const saved = { hiddenRefs: [topic, main, topic, 'deleted'], soloRefs: [tag, tag, 'refs/heads/ma'] };
    const before = structuredClone(saved);
    expect(reconcileHistoryFilter(saved, refs)).toEqual({ hiddenRefs: [main, topic], soloRefs: [tag] });
    expect(saved).toEqual(before);
    expect(deriveHistoryVisibility(refs, head, { hiddenRefs: [], soloRefs: ['deleted'] }).positiveRoots).toEqual(['M', 'T']);
  });

  it('supports grouped Hide/Solo, unhide/unsolo and clear with visible status', () => {
    const group = [remote, remoteHead];
    let filter = applyHistoryFilterAction(noFilter(), { kind: 'hide', names: group });
    expect(deriveHistoryVisibility(refs, head, filter).status).toEqual({ active: true, hiddenCount: 2, soloCount: 0 });
    filter = applyHistoryFilterAction(filter, { kind: 'solo', names: [...group, topic, remote] });
    expect(filter).toEqual({ hiddenRefs: [], soloRefs: [topic, remoteHead, remote].sort() });
    filter = applyHistoryFilterAction(filter, { kind: 'hide', names: [topic, remote] });
    expect(filter).toEqual({ hiddenRefs: [topic, remote].sort(), soloRefs: [remoteHead] });
    filter = applyHistoryFilterAction(filter, { kind: 'show', names: [topic] });
    filter = applyHistoryFilterAction(filter, { kind: 'unsolo', names: [remoteHead] });
    expect(filter).toEqual({ hiddenRefs: [remote], soloRefs: [] });
    filter = applyHistoryFilterAction(filter, { kind: 'clear' });
    expect(filter).toEqual(noFilter()); expect(deriveHistoryVisibility(refs, head, filter).status.active).toBe(false);
  });

  it('normalizes action results to disjoint sets, including persisted overlaps, without changing inputs', () => {
    const saved = { hiddenRefs: [main, topic], soloRefs: [main, remote] };
    const before = structuredClone(saved);
    const solo = applyHistoryFilterAction(saved, { kind: 'solo', names: [main] });
    expect(solo).toEqual({ hiddenRefs: [topic], soloRefs: [main, remote].sort() });
    const hide = applyHistoryFilterAction(saved, { kind: 'hide', names: [remote] });
    expect(hide).toEqual({ hiddenRefs: [main, topic, remote].sort(), soloRefs: [] });
    expect(saved).toEqual(before);
  });

  it('preserves the actual Git shared ancestors when Solo unions selected positive roots', () => {
    const gitRefs: HistoryRef[] = oracle.refs.map(item => ({ ...item, kind: item.kind as HistoryRef['kind'], displayName: item.name, hidden: false }));
    const gitHead: Head = { ...oracle.head, kind: 'branch' };
    const byOid = new Map(oracle.commits.map(item => [item.oid, item.parents]));
    // Test-only oracle reachability, never a second presentation traversal.
    const ancestors = (roots: readonly string[]) => {
      const seen = new Set<string>(), pending = [...roots];
      while (pending.length) {
        const oid = pending.pop()!;
        if (seen.has(oid)) continue;
        seen.add(oid); pending.push(...(byOid.get(oid) ?? []));
      }
      return seen;
    };
    const alpha = gitRefs.find(item => item.name === 'refs/heads/feature/alpha')!;
    const unmerged = gitRefs.find(item => item.name === 'refs/heads/topic/unmerged')!;
    const soloMain = deriveHistoryVisibility(gitRefs, gitHead, { hiddenRefs: [alpha.name], soloRefs: [main] });
    expect(soloMain.positiveRoots).toEqual([gitHead.oid]);
    const mainHistory = ancestors(soloMain.positiveRoots), alphaHistory = ancestors([alpha.commitOid!]);
    expect([...alphaHistory].every(oid => mainHistory.has(oid))).toBe(true);
    expect(mainHistory.has(unmerged.commitOid!)).toBe(false);
    const both = deriveHistoryVisibility(gitRefs, gitHead, { hiddenRefs: [], soloRefs: [main, unmerged.name] });
    expect(ancestors(both.positiveRoots)).toEqual(new Set([...mainHistory, ...ancestors([unmerged.commitOid!])]));
    expect(deriveHistoryVisibility(gitRefs, gitHead, noFilter()).positiveRoots).not.toContain(gitRefs.find(item => item.kind === 'tag')!.oid);
  });
});
