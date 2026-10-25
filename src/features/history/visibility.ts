import type { HistoryFilter, HistoryRef } from '../../contracts/history';
import type { Head } from '../../contracts/models';

const sortedSet = (names: readonly string[]) => [...new Set(names)].sort();
export type HistoryFilterAction = { kind: 'hide' | 'show' | 'solo' | 'unsolo'; names: readonly string[] }
  | { kind: 'clear' };

/** Exact qualified names only. Groups use the same action with all member names.
 * User actions keep Hide/Solo disjoint: Solo unhides, Hide removes Solo.
 * Persisted overlapping filters are NOT silently rewritten; backend semantics
 * still omit hidden selected roots (including a valid empty effective view).
 */
export function applyHistoryFilterAction(filter: HistoryFilter, action: HistoryFilterAction): HistoryFilter {
  if (action.kind === 'clear') return { hiddenRefs: [], soloRefs: [] };
  const hidden = new Set(filter.hiddenRefs), solo = new Set(filter.soloRefs);
  // Normalize any pre-existing overlap with Hide precedence, matching the query.
  for (const name of hidden) solo.delete(name);
  for (const name of action.names) {
    if (action.kind === 'hide') { hidden.add(name); solo.delete(name); }
    if (action.kind === 'show') hidden.delete(name);
    if (action.kind === 'solo') { solo.add(name); hidden.delete(name); }
    if (action.kind === 'unsolo') solo.delete(name);
  }
  return { hiddenRefs: sortedSet([...hidden]), soloRefs: sortedSet([...solo]) };
}

/** Reconcile deleted saved names and ordering exactly as open_history does. */
export function reconcileHistoryFilter(filter: HistoryFilter, refs: readonly HistoryRef[]): HistoryFilter {
  const names = new Set(refs.map(ref => ref.name));
  return { hiddenRefs: sortedSet(filter.hiddenRefs.filter(name => names.has(name))),
    soloRefs: sortedSet(filter.soloRefs.filter(name => names.has(name))) };
}

export type HistoryVisibility = {
  readonly filter: HistoryFilter;
  readonly positiveRoots: readonly string[];
  readonly decorationRefs: readonly HistoryRef[];
  readonly status: { readonly active: boolean; readonly hiddenCount: number; readonly soloCount: number };
};

/** Predict the backend positive-root choice for sidebar/filter presentation.
 * This deliberately does not traverse ancestors or filter commit rows. The
 * returned backend session roots/page remain authoritative after a query.
 */
export function deriveHistoryVisibility(refs: readonly HistoryRef[], head: Head, saved: HistoryFilter): HistoryVisibility {
  const filter = reconcileHistoryFilter(saved, refs);
  const hidden = new Set(filter.hiddenRefs), solo = new Set(filter.soloRefs);
  const roots = new Set<string>();
  for (const ref of refs) {
    if (hidden.has(ref.name) || !ref.commitOid) continue;
    if (solo.size ? solo.has(ref.name) : ref.kind !== 'stash') roots.add(ref.commitOid);
  }
  if (!solo.size && head.kind === 'detached' && head.oid) roots.add(head.oid);
  return { filter, positiveRoots: [...roots].sort(), decorationRefs: refs.filter(ref => !hidden.has(ref.name)),
    status: { active: hidden.size > 0 || solo.size > 0, hiddenCount: hidden.size, soloCount: solo.size } };
}

/** Use existing decoration identities; never infer a branch from a row/OID. */
export function visibleHistoryDecorations(decorations: readonly string[], filter: HistoryFilter): string[] {
  const hidden = new Set(filter.hiddenRefs);
  return sortedSet(decorations.filter(name => !hidden.has(name)));
}
