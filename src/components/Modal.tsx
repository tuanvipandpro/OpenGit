import { useEffect, useId, useRef } from 'react';
import type { ReactNode } from 'react';
export function Modal({ title, children, onDismiss, className = '' }: { title: string; children: ReactNode; onDismiss: () => void; className?: string }) {
  const dialog = useRef<HTMLDialogElement>(null), heading = useId();
  useEffect(() => { const element = dialog.current!; element.showModal(); return () => { element.close(); }; }, []);
  return <dialog className={'modal ' + className} ref={dialog} aria-labelledby={heading} onCancel={event => { event.preventDefault(); onDismiss(); }}><header className="modal-header"><h2 id={heading}>{title}</h2><button type="button" className="modal-close" aria-label={'Close ' + title} onClick={onDismiss}>×</button></header>{children}</dialog>;
}
