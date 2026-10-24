import { useEffect, useRef, useState } from 'react';
import type { ApiResult, AppError, AuthPrompt, OperationSnapshot } from '../../contracts';
import { Modal } from '../../components/Modal';
import { Button } from '../../components/Button';
import { ErrorNotice } from '../../components/ErrorNotice';
export function AuthDialog({ prompt, sourceDisplay, onRespond, onCancel }: { prompt: AuthPrompt; sourceDisplay: string; onRespond: (value: string) => Promise<ApiResult<null>>; onCancel: () => Promise<ApiResult<OperationSnapshot>> }) {
  const [value, setValue] = useState(''), [busy, setBusy] = useState(false), [answered, setAnswered] = useState(false), [error, setError] = useState<AppError | null>(null), [validation, setValidation] = useState<string | null>(null), [expired, setExpired] = useState(false);
  const pending = useRef(false), cancelling = useRef(false);
  const [cancelPending, setCancelPending] = useState(false);
  const confirmation = prompt.kind === 'confirmation';
  const label = prompt.kind === 'username' ? 'Username' : prompt.kind === 'passphrase' ? 'Key passphrase' : 'Password';
  useEffect(() => {
    const check = () => { if (!Number.isFinite(Date.parse(prompt.expiresAt)) || Date.now() >= Date.parse(prompt.expiresAt)) { setExpired(true); setValue(''); } };
    check(); const timer = setInterval(check, 1000); return () => { clearInterval(timer); };
  }, [prompt.expiresAt]);
  const cancel = () => { setValue(''); if (cancelling.current) return; cancelling.current = true; setCancelPending(true); setBusy(true); void onCancel().then(result => { if (!result.ok) setError(result.error); }).finally(() => { cancelling.current = false; setCancelPending(false); setBusy(pending.current); }); };
  const send = async (response: string) => {
    if (pending.current || cancelling.current || answered || expired || error?.code === 'InvalidHandle') return;
    if (new Blob([response]).size > 8192 || /[\0\r\n]/.test(response)) { setValue(''); setValidation('Enter a response without line breaks, up to 8192 bytes.'); return; }
    pending.current = true; setValue(''); setBusy(true); setValidation(null); setError(null);
    const result = await onRespond(response);
    pending.current = false; setBusy(cancelling.current);
    if (result.ok) setAnswered(true); else setError(result.error);
  };
  return <Modal title={confirmation ? 'Verify SSH host' : 'Repository authentication'} onDismiss={cancel} className="auth-modal"><form autoComplete="off" onSubmit={event => { event.preventDefault(); if (!confirmation) void send(value); }}>
    <div className="modal-content"><p className="field-hint auth-context">Repository: <strong>{sourceDisplay}</strong></p><p className="auth-message">{prompt.message}</p>{confirmation ? <p className="field-hint">Verify this fingerprint through a trusted channel before accepting it.</p> : <label className="field-label">{label}<input autoFocus type={prompt.kind === 'username' ? 'text' : 'password'} name="auth-response" autoComplete="off" autoCapitalize="none" spellCheck={false} value={value} disabled={busy || answered || expired || error?.code === 'InvalidHandle'} required onChange={event => setValue(event.target.value)} /></label>}{answered && <p role="status">Waiting for Git to continue…</p>}{expired && <p role="alert" className="form-validation">This authentication request has expired.</p>}{validation && <p role="alert" className="form-validation">{validation}</p>}{error && <ErrorNotice error={error} />}</div>
    <footer className="modal-actions"><Button type="button" disabled={cancelPending} onClick={cancel}>Cancel authentication</Button>{confirmation ? <><Button type="button" autoFocus disabled={busy || answered || expired} onClick={() => { void send('no'); }}>Reject host</Button><Button type="button" className="primary" disabled={busy || answered || expired} onClick={() => { void send('yes'); }}>Trust host</Button></> : <Button type="submit" className="primary" disabled={busy || answered || expired || !value}>Continue</Button>}</footer>
  </form></Modal>;
}
