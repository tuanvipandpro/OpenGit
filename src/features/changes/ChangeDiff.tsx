import type { ChangeDiff as Diff } from '../../contracts';
export function ChangeDiff({ value, loading }: { value: Diff | null; loading: boolean }) {
  return <section className="change-diff" aria-label="Change diff">{loading ? <p role="status">Loading diff…</p> : value ? <><p className="muted">{value.partialEligibility}</p><pre tabIndex={0}>{value.content || 'No text diff is available.'}</pre></> : <p className="muted">Select a staged or unstaged file to inspect its changes.</p>}</section>;
}
