import { useRef, useState } from 'react';
import type { ApiResult, AppError, CloneDestination, CommandMap, LocalCloneSource, OperationSnapshot } from '../../contracts';
import { Modal } from '../../components/Modal';
import { Button } from '../../components/Button';
import { ErrorNotice } from '../../components/ErrorNotice';
import { CloneSourceInput } from './CloneSourceInput';
import { CloneDestinationPicker } from './CloneDestinationPicker';
import { directoryError, sourceError } from './validation';
export interface CloneDialogProps {
  onDismiss(): void;
  chooseSource(): Promise<ApiResult<LocalCloneSource | null>>;
  chooseDestination(): Promise<ApiResult<CloneDestination | null>>;
  onStart(request: CommandMap['start_clone']['request']): Promise<ApiResult<OperationSnapshot>>;
}
export function CloneDialog({ onDismiss, chooseSource, chooseDestination, onStart }: CloneDialogProps) {
  const [mode, setMode] = useState<'remote' | 'local'>('remote'), [url, setUrl] = useState(''), [local, setLocal] = useState<LocalCloneSource | null>(null), [destination, setDestination] = useState<CloneDestination | null>(null), [name, setName] = useState('');
  const [error, setError] = useState<AppError | null>(null), [validation, setValidation] = useState<string | null>(null), [busy, setBusy] = useState(false);
  const pending = useRef(false);
  const dismiss = () => { if (!pending.current) { setUrl(''); onDismiss(); } };
  const choose = async (kind: 'source' | 'destination') => {
    if (pending.current) return; pending.current = true; setBusy(true); setError(null); setValidation(null);
    if (kind === 'source') { const result = await chooseSource(); if (result.ok) { if (result.value) setLocal(result.value); } else setError(result.error); }
    else { const result = await chooseDestination(); if (result.ok) { if (result.value) setDestination(result.value); } else setError(result.error); }
    pending.current = false; setBusy(false);
  };
  const submit = async () => {
    if (pending.current) return;
    const remote = url.trim();
    const invalid = (mode === 'remote' ? sourceError(remote) : local ? null : 'Choose a local source repository.') ?? (!destination ? 'Choose a destination parent folder.' : directoryError(name));
    if (invalid) { setValidation(invalid); setError(null); return; }
    pending.current = true; setBusy(true); setError(null); setValidation(null);
    const response = await onStart({ source: mode === 'remote' ? { kind: 'remote', url: remote } : { kind: 'local', sourceHandle: local!.sourceHandle }, destinationHandle: destination!.destinationHandle, directoryName: name });
    pending.current = false; setBusy(false);
    if (response.ok) { setUrl(''); onDismiss(); }
    else { setError(response.error); if (response.error.code === 'InvalidHandle') { setDestination(null); setLocal(null); } }
  };
  return <Modal title="Clone repository" onDismiss={dismiss}><form onSubmit={event => { event.preventDefault(); void submit(); }} aria-busy={busy}>
    <div className="modal-content"><p className="modal-intro">Bring a repository into your workspace.</p><CloneSourceInput mode={mode} url={url} local={local} busy={busy} onMode={value => { setMode(value); setUrl(''); setValidation(null); }} onUrl={setUrl} onChoose={() => { void choose('source'); }} /><CloneDestinationPicker destination={destination} busy={busy} name={name} onChoose={() => { void choose('destination'); }} onName={setName} />{validation && <p className="form-validation" role="alert">{validation}</p>}{error && <ErrorNotice error={error} />}</div>
    <footer className="modal-actions"><Button type="button" disabled={busy} onClick={dismiss}>Cancel</Button><Button type="submit" className="primary" disabled={busy}>{busy ? 'Preparing…' : 'Start clone'}</Button></footer>
  </form></Modal>;
}
