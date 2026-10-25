import { useState } from 'react';
import type { AppError, OperationSnapshot } from '../../contracts';
import { OperationDetails } from './OperationDetails';
import type { DisplayActionResult } from '../../stores/operations';
import { activeOperation } from '../../stores/operations';
export function TaskPanel({ operations, onCancel, onRetry, onOpen, outcomes = {}, outcomeErrors = {}, onInspect }: {
  operations: OperationSnapshot[]; onCancel: (id: string) => void; onRetry?: () => void; onOpen?: (operation: OperationSnapshot) => void;
  outcomes?: Record<string, DisplayActionResult>; outcomeErrors?: Record<string, AppError>; onInspect?: (id: string) => void;
}) {
  const [expanded, setExpanded] = useState(true);
  const active = operations.filter(activeOperation).length;
  return <section className={'task-panel ' + (expanded ? 'expanded' : '')}><button className="task-toggle" onClick={() => setExpanded(value => !value)} aria-expanded={expanded}><span>Tasks <span className="task-count">{operations.length}</span></span><span>{active ? active + ' active' : 'All quiet'} <span aria-hidden="true">{expanded ? '⌄' : '⌃'}</span></span></button>{expanded && <div className="task-body">{!operations.length ? <p className="muted">No tasks yet. Repository operations will appear here.</p> : operations.map(operation => <OperationDetails key={operation.id} operation={operation} result={outcomes[operation.id]} outcomeError={outcomeErrors[operation.id]} onInspect={onInspect} onCancel={onCancel} onRetry={onRetry} onOpen={onOpen} />)}</div>}</section>;
}
