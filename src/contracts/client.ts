import { invoke } from '@tauri-apps/api/core';
import type { ApiResult } from './models';
import type { CommandMap } from './requests';
import { transportError } from './errors';
// Commands and payloads are closed by CommandMap. Never expose transport exception text.
export async function call<C extends keyof CommandMap>(command: C, request: CommandMap[C]['request']): Promise<ApiResult<CommandMap[C]['value']>> {
  try { return await invoke<ApiResult<CommandMap[C]['value']>>(command, { request }); }
  catch { return { ok: false, error: transportError() }; }
}
