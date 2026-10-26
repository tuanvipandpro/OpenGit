import type { ChangeDiff as Diff, ChangeSelectionInput, PartialEligibility } from '../../contracts';
import type { ChangeSelectionFence } from '../../stores/repositoryChanges';
const fallback: Record<Exclude<PartialEligibility, 'eligible'>, string> = {
  binary: 'Binary content requires a whole-file action.', encoding: 'This encoding cannot be selected safely by line. Use a whole-file action.',
  lineEndings: 'These line endings require a whole-file action to preserve exact bytes.', attributes: 'Git attributes or content conversion require a whole-file action.',
  rename: 'Rename or copy changes require a whole-file action.', mode: 'File mode or type changes require a whole-file action.',
  tooLarge: 'This file exceeds the partial-selection size limit. Use a whole-file action.', complexityLimit: 'This diff exceeds the partial-selection complexity limit. Use a whole-file action.',
  filePresenceOnly: 'This change only affects file presence. Use a whole-file action.', unsupported: 'Partial selection is unavailable for this change. Use a whole-file action.',
};
export function ChangeDiff({ value, loading, selection = null, disabled = false, pathLabel, onSelection, onApplySelection }: {
  value: Diff | null; loading: boolean; selection?: ChangeSelectionInput | null; disabled?: boolean; pathLabel?: string;
  onSelection?(fence: ChangeSelectionFence, selection: ChangeSelectionInput): void; onApplySelection?(fence: ChangeSelectionFence): void;
}) {
  const count = selection ? selection.kind === 'hunks' ? selection.hunkIds.length : selection.lineIds.length : 0;
  const toggle = (kind: 'hunks' | 'lines', id: string, checked: boolean) => {
    if (!value || !onSelection) return;
    const ids = selection?.kind === kind ? selection.kind === 'hunks' ? selection.hunkIds : selection.lineIds : [];
    const next = checked ? [...ids, id] : ids.filter(value => value !== id);
    onSelection(value, kind === 'hunks' ? { kind, hunkIds: next } : { kind, lineIds: next });
  };
  return <section className="change-diff" aria-label="Change diff">{loading ? <p role="status">Loading diff…</p> : value ? <>
    <p>{value.side === 'staged' ? 'Staged' : 'Unstaged'}{pathLabel && ` · ${pathLabel}`}</p>
    {value.partialEligibility === 'eligible' && value.hunks.length ? <>
      <p className="muted">Select whole hunks or individual edited lines. Changing selection type clears the previous selection. Working files stay in place.</p>
      <div className="change-hunks">{value.hunks.map((hunk, index) => <section className="change-hunk" key={hunk.hunkId} aria-label={`Hunk ${index + 1}`}>
        <label className="change-hunk-select"><input type="checkbox" aria-label={`Select hunk ${index + 1}`} disabled={disabled || !onSelection} checked={selection?.kind === 'hunks' && selection.hunkIds.includes(hunk.hunkId)} onChange={event => toggle('hunks', hunk.hunkId, event.target.checked)} />Hunk {index + 1}</label>
        {hunk.rows.map((row, ordinal) => <div className={`change-diff-row change-diff-${row.kind}`} key={row.lineId ?? `context-${ordinal}`}>
          <span className="change-line-select">{row.kind !== 'context' && row.lineId && <input type="checkbox" aria-label={`Select ${row.kind === 'add' ? 'added' : 'deleted'} line ${row.kind === 'add' ? row.newLine : row.oldLine}`} disabled={disabled || !onSelection} checked={selection?.kind === 'lines' && selection.lineIds.includes(row.lineId)} onChange={event => toggle('lines', row.lineId!, event.target.checked)} />}</span>
          <span className="change-line-number" aria-hidden="true">{row.oldLine ?? ''}</span><span className="change-line-number" aria-hidden="true">{row.newLine ?? ''}</span><span aria-hidden="true">{row.kind === 'add' ? '+' : row.kind === 'delete' ? '−' : ' '}</span>
          <code>{row.text}{!row.hasLf && <small className="change-no-lf">No newline at end of file</small>}</code>
        </div>)}
      </section>)}</div>
      <p role="status">{count} {selection?.kind === 'hunks' ? 'hunk' : 'line'}{count === 1 ? '' : 's'} selected</p>
      {onApplySelection && <button type="button" disabled={disabled || !count} onClick={() => onApplySelection(value)}>{value.side === 'staged' ? 'Unstage selected' : 'Stage selected'}</button>}
    </> : <><p className="muted">{value.partialEligibility === 'eligible' ? 'No selectable text changes. Use a whole-file action.' : fallback[value.partialEligibility]}</p><pre tabIndex={0}>{value.content || 'No text diff is available.'}</pre></>}
  </> : <p className="muted">Select a staged or unstaged file to inspect its changes.</p>}</section>;
}
