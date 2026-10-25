import { useStore } from 'zustand';
import type { StoreApi } from 'zustand';
import type { RepositoryView, DiffUnavailableReason } from '../../contracts';
import type { CommitInspectorState } from '../../stores/commitInspector';
import type { HistorySelection } from '../../stores/history';
import { ErrorNotice } from '../../components/ErrorNotice';
import { MonacoDiff } from './MonacoDiff';
import type { RepositorySyncState } from '../../stores/repositorySync';
import { sameActionScope } from '../../stores/repositorySync';

const unavailable: Record<DiffUnavailableReason, string> = {
  binary: 'This file contains binary data. A text diff is unavailable.',
  gitlink: 'This entry points to a submodule commit. A text diff is unavailable.',
  tooLarge: 'This comparison exceeds the text display limit. No content was truncated.',
  unsupportedEncoding: 'This file is not supported UTF-8 text. A text diff is unavailable.',
  missingObject: 'Content is missing from the local repository. Refresh after restoring the required objects.',
};
export function CommitInspector({ store, selected, layout, onLayoutChange, syncStore }: { store: StoreApi<CommitInspectorState>; selected: HistorySelection | null; layout: RepositoryView['diffLayout']; onLayoutChange: (layout: RepositoryView['diffLayout']) => void; syncStore?: StoreApi<RepositorySyncState> }) {
  const state = useStore(store);
  const details = state.details && (!selected || selected.commit.oid === state.details.commit.oid) ? state.details : null;
  const visibleSelected = selected ?? (details ? state.selected : null);
  return <section className="commit-inspector" aria-label="Selected commit">
    {!visibleSelected ? <p>Select a commit to view its information.</p> : <>
      <header className="commit-inspector-heading"><strong>{visibleSelected.commit.subject || '(no subject)'}</strong><code>{visibleSelected.commit.oid}</code>
        <span>{visibleSelected.commit.authorName} &lt;{visibleSelected.commit.authorEmail}&gt;</span>
        {visibleSelected.outsideView && <strong className="history-outside">Outside current filters</strong>}
        <label>Compare against <select aria-label="Diff base" value={state.base.kind === 'root' ? 'root' : String(state.base.index)} disabled={state.loading && !state.refreshing}
          onChange={event => { void state.setBase(event.target.value === 'root' ? { kind: 'root' } : { kind: 'parent', index: Number(event.target.value) }); }}>
          {!visibleSelected.commit.parents.length ? <option value="root">Empty tree (root commit)</option> : visibleSelected.commit.parents.map((parent, index) => <option key={`${index}:${parent.oid}`} value={index}>Parent {index + 1} · {parent.oid.slice(0, 12)}</option>)}
        </select></label>
        <label>Diff layout <select aria-label="Diff layout" value={layout} onChange={event => onLayoutChange(event.target.value as RepositoryView['diffLayout'])}><option value="inline">Inline</option><option value="sideBySide">Side by side</option></select></label>
      </header>
      {syncStore && <CommitContainmentPanel store={syncStore} selected={visibleSelected} />}
      {state.loading && <p role="status">Loading commit details…</p>}
      {state.notice && <p role="status">{state.notice}</p>}
      {state.error && <ErrorNotice error={state.error} onRetry={() => { void state.retry(); }} />}
      {details && <>
        <details className="commit-message"><summary>Commit message and metadata</summary><p>Author: {details.commit.authorName} &lt;{details.commit.authorEmail}&gt; · {details.commit.authoredAt}</p><p>Committer: {details.committerName} &lt;{details.committerEmail}&gt; · {details.commit.committedAt}</p><pre>{details.message}</pre></details>
        {state.displayedFile && state.displayedBase && state.diff && <p className="commit-displayed-comparison">Showing {state.displayedBase.kind === 'root' ? 'empty tree' : `parent ${state.displayedBase.index + 1}`} · {state.displayedFile.pathDisplay}{state.refreshing || state.loading || state.diffLoading ? ' (previously loaded comparison)' : ''}</p>}
        <div className="commit-comparison">
          <nav className="commit-files" aria-label="Changed files">{!details.files.length && <p>No changed files for this comparison.</p>}
            {details.files.map(file => <button type="button" key={file.handle} aria-current={file.handle === state.fileHandle ? 'true' : undefined} title={file.pathDisplay}
              onClick={() => { void state.selectFile(file.handle); }}><span>{file.kind}</span> {file.previousPathDisplay && `${file.previousPathDisplay} → `}{file.pathDisplay}<small>{file.oldMode ?? '—'} → {file.newMode ?? '—'}</small></button>)}
          </nav>
          <div className="commit-diff-pane" aria-label="File comparison">
            {state.diffLoading && <p role="status">Loading file comparison…</p>}
            {state.diffError && <ErrorNotice error={state.diffError} onRetry={() => { if (state.fileHandle) void state.selectFile(state.fileHandle); }} />}
            {state.diff?.content.kind === 'unavailable' && <p role="status">{unavailable[state.diff.content.reason]}</p>}
            {state.diff?.content.kind === 'text' && <MonacoDiff key={`${state.diff.context.scope.generation}:${state.diff.context.snapshotId}:${state.diff.fileHandle}`} original={state.diff.content.original} modified={state.diff.content.modified} language={state.diff.content.language} layout={layout} />}
          </div>
        </div>
      </>}
    </>}
  </section>;
}

const membershipLabel = { contains: 'Contains selected commit', doesNotContain: 'Does not contain selected commit', unknown: 'Containment unknown' };
function CommitContainmentPanel({ store, selected }: { store: StoreApi<RepositorySyncState>; selected: HistorySelection }) {
  const state = useStore(store), candidate = state.containment;
  const value = candidate?.oid === selected.commit.oid && candidate.context.scope.repoId === selected.context.scope.repoId
    && candidate.context.scope.worktreeId === selected.context.scope.worktreeId ? candidate : null;
  const retained = state.containmentRetained || !!value && (!sameActionScope(value.context.scope, selected.context.scope) || value.context.snapshotId !== selected.context.snapshotId);
  const inclusion = (name: string | null) => name ? value?.branches.find(item => item.branch.name === name)?.membership ?? 'unknown' : null;
  return <section className="commit-containment" aria-label="Commit branch containment">
    <details open><summary>Containing branches and remote evidence</summary>
      {state.containmentLoading && <p role="status">Checking all local and remote-tracking branches…</p>}
      {retained && value && <p role="status">Previously loaded membership; awaiting the current history context.</p>}
      {!value && !state.containmentLoading && !state.containmentError && <p>Membership unavailable until this commit is bound to the current history snapshot.</p>}
      {state.containmentError && <ErrorNotice error={state.containmentError} onRetry={() => { void state.retryContainment(); }} />}
      {value && <>
        <p>Current branch: {value.currentBranch ?? 'None'} · {inclusion(value.currentBranch) ? membershipLabel[inclusion(value.currentBranch)!] : 'Not applicable'}</p>
        <p>Upstream: {value.upstream ?? 'Not configured'} · {inclusion(value.upstream) ? membershipLabel[inclusion(value.upstream)!] : 'Not applicable'}</p>
        <p>Local branch snapshot · {value.observedAt} · ancestry {value.completeness}. This is local tracking evidence; server state may have changed.</p>
        <ul aria-label="Branch membership">{value.branches.map(({ branch, membership }) => <li key={branch.name} data-ref-name={branch.name} data-membership={membership}>
          <strong>{branch.name}</strong> · {branch.kind === 'remote' ? 'Remote-tracking' : 'Local'} · {membershipLabel[membership]}
          {branch.kind === 'remote' && <small>{branch.observation?.matchesCurrent && branch.observation.observedOid === branch.oid
            ? `Last successful app fetch: ${branch.observation.observedAt} · observed ${branch.observation.observedOid}`
            : branch.observation ? `Previous app fetch: ${branch.observation.observedAt}; it does not match the current local tracking ref. Server state unknown.`
              : 'No matching successful app fetch evidence. Server state unknown.'}</small>}
        </li>)}</ul>
        {!value.branches.length && <p>No local or remote-tracking branch membership is available. This does not establish whether the commit was previously published.</p>}
      </>}
      {state.catalog && sameActionScope(state.catalog.scope, selected.context.scope) && !!state.catalog.symbolicBranches.length && <details><summary>Symbolic branch aliases</summary><ul>{state.catalog.symbolicBranches.map(branch => <li key={branch.name}>{branch.name} → {branch.target}</li>)}</ul></details>}
    </details>
  </section>;
}
