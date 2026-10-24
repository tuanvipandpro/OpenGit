export type { AppError, ErrorCode, Phase } from './models';
import type { AppError } from './models';
export const transportError = (): AppError => ({ code: 'Internal', message: 'The desktop service could not complete this request.', phase: 'validation', retry: 'none' });
