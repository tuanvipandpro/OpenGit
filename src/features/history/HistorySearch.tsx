import { useId } from 'react';
import { HISTORY_LIMITS } from '../../contracts/history';
import type { HistoryRef, HistorySearchHit } from '../../contracts/history';
import './history.css';

export type HistorySearchProps = { query: string; resultQuery: string | null; hits: readonly HistorySearchHit[]; refs: readonly HistoryRef[];
  loading: boolean; retained?: boolean; error?: string | null; hasMore: boolean; onQueryChange: (query: string) => void; onSearch: (query: string) => void;
  onLoadMore: () => void; onSelectCommit: (hit: HistorySearchHit) => void; onSelectRef: (ref: HistoryRef) => void };

export function HistorySearch(props: HistorySearchProps) {
  const id = useId(), bytes = new TextEncoder().encode(props.query).length;
  const valid = bytes > 0 && bytes <= HISTORY_LIMITS.queryBytes && !props.query.includes('\0');
  return <section className="history-search" aria-label="History search">
    <form onSubmit={event => { event.preventDefault(); if (valid) props.onSearch(props.query); }}>
      <label htmlFor={`${id}-query`}>Search history</label>
      <p id={`${id}-hint`}>Search all history by SHA, message, author or reference name.</p>
      <div className="history-search-input"><input id={`${id}-query`} type="search" value={props.query} aria-describedby={`${id}-hint`} onChange={event => props.onQueryChange(event.currentTarget.value)} />
        <button type="submit" disabled={!valid}>Search</button></div>
      {props.query && !valid && <p role="alert">Search text must contain 1–512 UTF-8 bytes and no null characters.</p>}
    </form>
    {props.error && <p role="alert">{props.error}</p>}
    {props.loading && <p role="status">Searching history…{(props.hits.length > 0 || props.refs.length > 0) && " Previous search results are retained until current results are ready."}</p>}
    {props.retained && !props.loading && <p role="status">Previous search results are retained. Refresh history or search again before selecting a result.</p>}
    {props.resultQuery !== null && <div aria-label="Search results" aria-busy={props.loading}>
      <p>Results for “{props.resultQuery}”</p>
      {!props.loading && !props.error && !props.hits.length && !props.refs.length && <p>No matching commits or references.</p>}
      <ul>{props.refs.map(ref => <li key={ref.name}><button type="button" disabled={props.loading || props.retained || !ref.commitOid} onClick={() => props.onSelectRef(ref)}>Reference · {ref.displayName}{!ref.commitOid && ' (not a commit)'}</button></li>)}
        {props.hits.map(hit => <li key={hit.commit.oid}><button type="button" disabled={props.loading || props.retained} onClick={() => props.onSelectCommit(hit)}>
          <span>{hit.commit.subject || '(no subject)'}</span> · <code>{hit.commit.oid.slice(0, 8)}</code> · <span>{hit.commit.authorName}</span>
          {hit.outsideView && <strong className="history-outside">Outside current filters</strong>}</button></li>)}</ul>
      {props.hasMore && <button type="button" disabled={props.loading || props.retained} onClick={props.onLoadMore}>Load more search results</button>}
    </div>}
  </section>;
}
