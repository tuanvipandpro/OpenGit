import type { AppError } from '../contracts';
export function ErrorNotice({ error, onRetry }: { error: AppError; onRetry?: () => void }) { return <div className="error-notice" role="alert"><strong>{error.code.replace(/([a-z])([A-Z])/g, '$1 $2')}</strong><p>{error.message}</p>{onRetry && <button className="text-button" onClick={onRetry}>Try again</button>}</div>; }
