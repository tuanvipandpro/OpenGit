import { useMemo } from 'react';
import type { Head } from '../../contracts/models';
import type { HistoryFilter, HistoryRef } from '../../contracts/history';
import { applyHistoryFilterAction, deriveHistoryVisibility } from './visibility';
import type { HistoryFilterAction } from './visibility';
import './history.css';

export type RefsSidebarProps = { refs: readonly HistoryRef[]; head: Head; filter: HistoryFilter; onFilterChange: (filter: HistoryFilter) => void;
  onSelectRef: (ref: HistoryRef) => void; loading?: boolean; error?: string | null };

export function RefsSidebar({ refs, head, filter, onFilterChange, onSelectRef, loading = false, error }: RefsSidebarProps) {
  const visibility = deriveHistoryVisibility(refs, head, filter), current = visibility.filter;
  const hiddenNames = new Set(current.hiddenRefs), soloNames = new Set(current.soloRefs);
  const groups = useMemo(() => {
    const remotes = new Map<string, HistoryRef[]>();
    for (const ref of refs.filter(item => item.kind === 'remote')) {
      const name = ref.name.slice('refs/remotes/'.length).split('/')[0];
      const group = remotes.get(name);
      if (group) group.push(ref); else remotes.set(name, [ref]);
    }
    return [{ label: 'Local branches', refs: refs.filter(ref => ref.kind === 'local') },
      ...[...remotes].sort(([a], [b]) => a.localeCompare(b)).map(([name, refs]) => ({ label: `Remote ${name}`, refs })),
      { label: 'Tags', refs: refs.filter(ref => ref.kind === 'tag') },
      { label: 'Stashes', refs: refs.filter(ref => ref.kind === 'stash') }];
  }, [refs]);
  const apply = (action: HistoryFilterAction) => onFilterChange(applyHistoryFilterAction(current, action));
  return <aside className="history-refs" aria-label="History references" aria-busy={loading}>
    <fieldset disabled={loading}><header><h2>References</h2><button type="button" onClick={() => apply({ kind: 'clear' })} disabled={!visibility.status.active}>Show all</button></header>
    <p role="status">{visibility.status.active ? `${visibility.status.hiddenCount} hidden · ${visibility.status.soloCount} solo` : 'All default roots visible'}</p>
    {head.kind === 'detached' && <p>HEAD detached · {head.oid?.slice(0, 8)}</p>}
    {head.kind === 'unborn' && <p>HEAD has no commits.</p>}
    {loading && <p>Loading references…</p>}{error && <p role="alert">{error}</p>}
    {groups.map(group => {
      const names = group.refs.map(ref => ref.name), allHidden = names.length > 0 && names.every(name => hiddenNames.has(name));
      const allSolo = names.length > 0 && names.every(name => soloNames.has(name));
      return <details key={group.label} open className="history-ref-group"><summary>{group.label} <span>({names.length})</span></summary>
        {group.label !== 'Stashes' && <div className="history-ref-actions"><button type="button" disabled={!names.length} aria-label={`${allHidden ? 'Show' : 'Hide'} ${group.label}`} onClick={() => apply({ kind: allHidden ? 'show' : 'hide', names })}>{allHidden ? 'Show group' : 'Hide group'}</button>
          <button type="button" disabled={!names.length} aria-label={`Solo ${group.label}`} aria-pressed={allSolo} onClick={() => apply({ kind: allSolo ? 'unsolo' : 'solo', names })}>Solo group</button></div>}
        {!names.length && <p className="history-ref-empty">None</p>}
        {group.refs.map(ref => {
          const hidden = hiddenNames.has(ref.name), solo = soloNames.has(ref.name);
          const isHead = ref.name === `refs/heads/${head.branch}` && head.kind === 'branch';
          return <div className="history-ref-row" key={ref.name} data-hidden={hidden}>
            <button type="button" className="history-ref-select" title={ref.name} disabled={!ref.commitOid} onClick={() => onSelectRef(ref)}
              aria-label={`${isHead ? 'HEAD · ' : ''}${ref.displayName}${!ref.commitOid ? ' (not a commit)' : ''}${ref.symbolicTarget ? ` · symbolic reference to ${ref.symbolicTarget}` : ''}`}>
              {isHead && <strong>HEAD · </strong>}{ref.displayName}
              {ref.symbolicTarget && <span className="history-sr-only"> symbolic reference to {ref.symbolicTarget}</span>}
              {!ref.commitOid && <span> (not a commit)</span>}</button>
            {ref.kind !== 'stash' && <><button type="button" aria-label={`${hidden ? 'Show' : 'Hide'} ${ref.name}`} aria-pressed={hidden} onClick={() => apply({ kind: hidden ? 'show' : 'hide', names: [ref.name] })}>{hidden ? 'Show' : 'Hide'}</button>
              <button type="button" aria-label={`Solo ${ref.name}`} aria-pressed={solo} onClick={() => apply({ kind: solo ? 'unsolo' : 'solo', names: [ref.name] })}>Solo</button></>}
          </div>;
        })}
      </details>;
    })}
  </fieldset></aside>;
}
