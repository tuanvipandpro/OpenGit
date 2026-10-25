import { useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import type { HistoryFilter, HistoryRef } from '../../contracts/history';
import type { HistoryFilterAction } from './visibility';

export type RefMenuTarget = { ref: HistoryRef; trigger: HTMLButtonElement; context: string | undefined };
export function RefActionsMenu({ target, filter, onClose, onView, onFilter }: {
  target: RefMenuTarget; filter: HistoryFilter; onClose: () => void;
  onView: () => void; onFilter: (action: HistoryFilterAction) => void;
}) {
  const menu = useRef<HTMLDivElement>(null), callbacks = useRef(onClose);
  callbacks.current = onClose;
  const [position, setPosition] = useState({ left: 8, top: 8 });
  useLayoutEffect(() => {
    const element = menu.current!, bounds = target.trigger.getBoundingClientRect(), box = element.getBoundingClientRect();
    setPosition({ left: Math.max(8, Math.min(bounds.left, window.innerWidth - box.width - 8)),
      top: Math.max(8, Math.min(bounds.bottom, window.innerHeight - box.height - 8)) });
    element.querySelector<HTMLElement>('[role=menuitem]')?.focus({ preventScroll: true });
    const close = () => callbacks.current();
    const outside = (event: PointerEvent) => { if (!element.contains(event.target as Node) && !target.trigger.contains(event.target as Node)) close(); };
    const observer = new MutationObserver(() => { if (!target.trigger.isConnected) close(); });
    observer.observe(document.body, { childList: true, subtree: true });
    document.addEventListener('pointerdown', outside);
    window.addEventListener('resize', close); document.addEventListener('scroll', close, true);
    return () => { observer.disconnect(); document.removeEventListener('pointerdown', outside); window.removeEventListener('resize', close); document.removeEventListener('scroll', close, true); };
  }, [target]);
  const hidden = filter.hiddenRefs.includes(target.ref.name), solo = filter.soloRefs.includes(target.ref.name);
  const action = (kind: 'hide' | 'show' | 'solo' | 'unsolo') => onFilter({ kind, names: [target.ref.name] });
  return createPortal(<div ref={menu} role="menu" aria-label={`Actions for ${target.ref.name}`} className="history-ref-menu" style={position}
    onKeyDown={event => {
      const items = [...menu.current!.querySelectorAll<HTMLButtonElement>('[role=menuitem]')];
      const index = items.indexOf(document.activeElement as HTMLButtonElement);
      if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
        event.preventDefault(); items[event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length]?.focus();
      } else if (event.key === 'Escape') { event.preventDefault(); onClose(); }
      else if (event.key === 'Tab') onClose();
      event.stopPropagation();
    }}>
    <button type="button" role="menuitem" onClick={onView}>View commit</button>
    {target.ref.kind !== 'stash' && <><button type="button" role="menuitem" onClick={() => action(hidden ? 'show' : 'hide')}>{hidden ? 'Show' : 'Hide'}</button>
      <button type="button" role="menuitem" onClick={() => action(solo ? 'unsolo' : 'solo')}>{solo ? 'Remove from Solo' : 'Add to Solo'}</button></>}
  </div>, document.body);
}
