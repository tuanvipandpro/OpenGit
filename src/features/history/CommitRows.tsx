import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import type { Head } from '../../contracts/models';
import type { HistoryFilter, HistoryRef } from '../../contracts/history';
import { HISTORY_LAYOUT_METRICS } from './layout';
import type { HistoryLayout } from './layout';
import { visibleHistoryDecorations } from './visibility';
import { HistoryGraph } from './HistoryGraph';
import type { HistoryGraphFrame, HistoryViewport } from './HistoryGraph';
import { RefActionsMenu } from './RefActionsMenu';
import type { RefMenuTarget } from './RefActionsMenu';
import type { HistoryFilterAction } from './visibility';
import './history.css';

export type HistoryAnchor = { oid: string; offset: number };
type PendingScroll = ({ kind: 'anchor' } & HistoryAnchor) | { kind: 'selection'; oid: string };
export type CommitRowsProps = {
  layout: HistoryLayout; head: Head; refs: readonly HistoryRef[]; filter: HistoryFilter;
  selectedOid: string | null; onSelect: (oid: string) => void;
  hasMore: boolean; loading: boolean; error?: string | null; onLoadMore: () => void;
  initialAnchor?: HistoryAnchor | null; onAnchorChange?: (anchor: HistoryAnchor) => void;
  suppressSelectionScroll?: boolean;
  actionContext?: string; actionsLoading?: boolean;
  onSelectRef?: (ref: HistoryRef) => void; onFilterAction?: (action: HistoryFilterAction) => void;
  onGraphFrame?: (frame: HistoryGraphFrame) => void;
};

/** Scope/snapshot reset is controlled by the parent (remount with a scope key).
 * Appends retain OID keys, scroll offset and selection. Anchor offset is the
 * number of pixels scrolled past the anchored row's top. initialAnchor is a
 * mount-time restoration hint; changed selection or manual navigation cancels
 * it. An initially saved selection does not override that scroll restoration.
 */
export function CommitRows(props: CommitRowsProps) {
  const { layout, selectedOid, onSelect } = props, rowHeight = HISTORY_LAYOUT_METRICS.rowHeight;
  const [menu, setMenu] = useState<RefMenuTarget | null>(null);
  const closedTrigger = useRef<HTMLButtonElement | null>(null);
  const latest = useRef(props); latest.current = props;
  const closeMenu = useCallback(() => {
    const trigger = menuTrigger.current;
    closedTrigger.current = trigger;
    if (trigger) (trigger.isConnected ? trigger : scroll.current)?.focus({ preventScroll: true });
    setMenu(null);
  }, []);
  const menuTrigger = useRef<HTMLButtonElement | null>(null);
  menuTrigger.current = menu?.trigger ?? null;
  const scroll = useRef<HTMLDivElement>(null), previousSelection = useRef(selectedOid);
  const focusReferences = useCallback(() => {
    scroll.current?.closest('.history-screen')?.querySelector<HTMLElement>('.history-references-trigger')?.focus({ preventScroll: true });
  }, []);
  const pendingScroll = useRef<PendingScroll | null>(props.initialAnchor ? { kind: 'anchor', ...props.initialAnchor } : null);
  const lastReportedAnchor = useRef<HistoryAnchor | null>(null);
  const cancelPendingScroll = useCallback(() => { pendingScroll.current = null; }, []);
  const id = useId(), anchorCallback = useRef(props.onAnchorChange);
  anchorCallback.current = props.onAnchorChange;
  const [viewport, setViewport] = useState<HistoryViewport>({ width: 0, height: 0, scrollTop: 0, scrollLeft: 0, dpr: 1 });
  const measure = useCallback(() => {
    const element = scroll.current;
    if (!element) return;
    const next = { width: element.clientWidth, height: element.clientHeight, scrollTop: element.scrollTop, scrollLeft: element.scrollLeft, dpr: window.devicePixelRatio || 1 };
    setViewport(old => Object.keys(next).every(key => next[key as keyof HistoryViewport] === old[key as keyof HistoryViewport]) ? old : next);
  }, []);
  const virtualizer = useVirtualizer<HTMLDivElement, HTMLDivElement>({ count: layout.nodes.length, getScrollElement: () => scroll.current,
    estimateSize: () => rowHeight, getItemKey: index => layout.nodes[index].commit.oid, overscan: 6, useFlushSync: false });
  useLayoutEffect(() => {
    const element = scroll.current;
    if (!element) return;
    const observer = new ResizeObserver(measure); observer.observe(element); window.addEventListener('resize', measure); measure();
    return () => { observer.disconnect(); window.removeEventListener('resize', measure); };
  }, [measure]);
  useLayoutEffect(() => {
    if (previousSelection.current !== selectedOid) {
      previousSelection.current = selectedOid;
      if (!props.suppressSelectionScroll) pendingScroll.current = selectedOid ? { kind: 'selection', oid: selectedOid } : null;
    }
    const pending = pendingScroll.current;
    if (!pending) return;
    const node = layout.nodesByOid.get(pending.oid);
    if (!node) return; // Keep the latest intent until a following page supplies it.
    pendingScroll.current = null; // Consume once; appending again must not scroll.
    if (pending.kind === 'anchor') virtualizer.scrollToOffset(Math.max(0, node.row * rowHeight + pending.offset));
    else virtualizer.scrollToIndex(node.row, { align: 'auto' });
    measure();
  }, [selectedOid, layout, rowHeight, virtualizer, measure, props.suppressSelectionScroll]);
  useEffect(() => {
    if (pendingScroll.current) return;
    // A layout effect may just have restored scroll before the viewport state
    // catches up. Read the element so the old default top cannot overwrite it.
    const top = scroll.current?.scrollTop ?? viewport.scrollTop, row = layout.nodes[Math.floor(top / rowHeight)];
    if (!row) return;
    const anchor = { oid: row.commit.oid, offset: top - row.row * rowHeight };
    if (anchor.oid === lastReportedAnchor.current?.oid && anchor.offset === lastReportedAnchor.current.offset) return;
    lastReportedAnchor.current = anchor; anchorCallback.current?.(anchor);
  }, [layout, rowHeight, viewport.scrollTop, selectedOid]);
  useLayoutEffect(() => { if (menu && (props.actionsLoading || menu.context !== props.actionContext || !props.refs.some(ref => ref.name === menu.ref.name && ref.commitOid === menu.ref.commitOid))) closeMenu(); }, [menu, props.actionsLoading, props.actionContext, props.refs, closeMenu]);
  useLayoutEffect(() => () => {
    // The rows remount for a new snapshot. Transfer only focus owned by this
    // graph; a newer click in search, the inspector or another tab wins.
    if (scroll.current?.contains(document.activeElement) || menuTrigger.current && document.activeElement?.closest('.history-ref-menu')) focusReferences();
  }, [focusReferences]);
  const openMenu = (ref: HistoryRef, trigger: HTMLButtonElement) => {
    if (!latest.current.actionsLoading) setMenu({ ref, trigger, context: props.actionContext });
  };
  const dispatch = (action?: HistoryFilterAction) => {
    const current = latest.current, target = menu;
    closeMenu();
    if (!target || current.actionsLoading || current.actionContext !== target.context || !current.refs.some(ref => ref.name === target.ref.name && ref.commitOid === target.ref.commitOid)) return;
    if (action) {
      // Filter refresh disables/replaces the current badge. References belongs
      // to the screen and survives both a failed request and a new snapshot.
      focusReferences(); current.onFilterAction?.(action);
    } else current.onSelectRef?.(target.ref);
  };
  const refs = useMemo(() => new Map(props.refs.map(ref => [ref.name, ref])), [props.refs]);
  const items = virtualizer.getVirtualItems(), selected = selectedOid ? layout.nodesByOid.get(selectedOid) : undefined;
  useLayoutEffect(() => {
    if (closedTrigger.current && !closedTrigger.current.isConnected) {
      if (document.activeElement === document.body) scroll.current?.focus({ preventScroll: true });
      closedTrigger.current = null;
    }
  });
  const graphWidth = layout.columnCount * HISTORY_LAYOUT_METRICS.columnWidth + HISTORY_LAYOUT_METRICS.paddingX * 2;
  const keyboard = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget) return;
    if (['ArrowDown', 'ArrowUp', 'ArrowLeft', 'ArrowRight', 'Home', 'End', 'PageDown', 'PageUp', ' '].includes(event.key)) cancelPendingScroll();
    if (!layout.nodes.length) return;
    let next = selected?.row ?? -1;
    if (event.key === 'ArrowDown') next++;
    else if (event.key === 'ArrowUp') next = next < 0 ? 0 : next - 1;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = layout.nodes.length - 1;
    else if (event.key === 'PageDown') next += Math.max(1, Math.floor(viewport.height / rowHeight));
    else if (event.key === 'PageUp') next = Math.max(0, next) - Math.max(1, Math.floor(viewport.height / rowHeight));
    else return;
    event.preventDefault(); next = Math.max(0, Math.min(layout.nodes.length - 1, next));
    onSelect(layout.nodes[next].commit.oid); virtualizer.scrollToIndex(next, { align: 'auto' });
  };
  return <section className="history-rows" aria-label="Commit history">
    <div className="history-list-frame">
      <div ref={scroll} className="history-scroll" role="grid" aria-label="Commits" tabIndex={0} aria-busy={props.loading} aria-rowcount={layout.nodes.length}
        aria-activedescendant={selected && items.some(item => item.index === selected.row) ? `${id}-row-${selected.row}` : undefined}
        onKeyDown={keyboard} onScroll={measure} onWheel={cancelPendingScroll} onTouchStart={cancelPendingScroll} onPointerDown={cancelPendingScroll}>
        <div className="history-list-content" style={{ height: virtualizer.getTotalSize(), width: Math.max(viewport.width, graphWidth + 560) }}>
          {items.map(item => {
            const node = layout.nodes[item.index], commit = node.commit;
            const decorations = visibleHistoryDecorations(commit.decorations, props.filter);
            return <div key={commit.oid} id={`${id}-row-${item.index}`} role="row" aria-selected={commit.oid === selectedOid}
              aria-rowindex={item.index + 1} data-oid={commit.oid} className="history-commit-row"
              style={{ height: rowHeight, transform: `translateY(${item.start}px)`, paddingLeft: graphWidth }} onClick={() => { cancelPendingScroll(); scroll.current?.focus({ preventScroll: true }); onSelect(commit.oid); }}>
              <span role="gridcell" className="history-subject">{commit.oid === props.head.oid && <strong className="history-decoration">HEAD</strong>}
                {decorations.map(name => {
                  const ref = refs.get(name);
                  return ref?.commitOid && props.onSelectRef ? <button type="button" className="history-decoration" key={name} title={name}
                    aria-label={`Actions for ${name}`} aria-haspopup="menu" aria-expanded={menu?.ref.name === name && menu.trigger.isConnected}
                    aria-disabled={!!props.actionsLoading} onPointerDown={event => event.stopPropagation()}
                    onClick={event => { event.stopPropagation(); openMenu(ref, event.currentTarget); }}
                    onContextMenu={event => { event.preventDefault(); event.stopPropagation(); openMenu(ref, event.currentTarget); }}
                    onKeyDown={event => { if (event.key === 'F10' && event.shiftKey || event.key === 'ContextMenu') { event.preventDefault(); event.stopPropagation(); openMenu(ref, event.currentTarget); } }}
                  >{ref.displayName}</button> : <span className="history-decoration" key={name} title={name}>{ref?.displayName ?? name}</span>;
                })}{commit.subject || '(no subject)'}</span>
              <span role="gridcell" className="history-author" title={`${commit.authorName} <${commit.authorEmail}>`}>{commit.authorName}</span><code role="gridcell" title={commit.oid}>{commit.oid.slice(0, 8)}</code>
              <span className="history-sr-only">{commit.parents.map(parent => `${parent.oid}: ${layout.nodesByOid.has(parent.oid) ? 'loaded' : parent.boundary}`).join('; ')}</span>
            </div>;
          })}
        </div>
      </div>
      <HistoryGraph layout={layout} viewport={viewport} selectedOid={selectedOid} headOid={props.head.oid} onFrame={props.onGraphFrame} />
    </div>
    {menu && !props.actionsLoading && menu.context === props.actionContext && <RefActionsMenu target={menu} filter={props.filter} onClose={closeMenu} onView={() => dispatch()} onFilter={dispatch} />}
    <div className="history-load-state">
      {props.error && <p role="alert">{props.error}</p>}
      {props.loading && <p role="status">Loading history…</p>}
      {!layout.nodes.length && !props.loading && !props.error && <p>No commits in the current view.</p>}
      {props.hasMore && <button type="button" onClick={props.onLoadMore} disabled={props.loading}>{props.loading ? 'Loading…' : props.error ? 'Retry loading history' : 'Load more commits'}</button>}
    </div>
  </section>;
}
