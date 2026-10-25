import { useRef, useState } from 'react';
import { Modal } from '../../components/Modal';
import { RefsSidebar } from './RefsSidebar';
import type { RefsSidebarProps } from './RefsSidebar';

/** Temporary discovery surface; persisted sidebar layout never opens it. */
export function ReferencesPanel(props: RefsSidebarProps) {
  const [open, setOpen] = useState(false), trigger = useRef<HTMLButtonElement>(null);
  const close = () => { setOpen(false); trigger.current?.focus({ preventScroll: true }); };
  return <div className="history-toolbar"><button type="button" ref={trigger} className="history-references-trigger" aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(true)}>References</button>
    {open && <Modal title="References" className="history-references-dialog" onDismiss={close}><RefsSidebar {...props} /></Modal>}
  </div>;
}
