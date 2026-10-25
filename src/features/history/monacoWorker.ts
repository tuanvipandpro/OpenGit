import EditorWorker from 'monaco-editor/editor/editor.worker.js?worker';
import type * as Monaco from 'monaco-editor/editor/editor.api.js';

export type MonacoLease = { api: typeof Monaco; release: () => void };

// This module itself is loaded only when a diff is mounted. Monaco owns the
// shared editor worker; each mounted view owns only its error subscription.
const failures = new Set<() => void>();
let pending: Promise<typeof Monaco> | undefined;

export async function loadMonaco(onWorkerFailure: () => void): Promise<MonacoLease> {
  failures.add(onWorkerFailure);
  if (!pending) {
    globalThis.MonacoEnvironment = {
      getWorker() {
        try {
          const worker = new EditorWorker();
          worker.addEventListener('error', event => {
            event.preventDefault();
            for (const notify of failures) notify();
          });
          worker.addEventListener('messageerror', () => {
            for (const notify of failures) notify();
          });
          return worker;
        } catch {
          for (const notify of failures) notify();
          throw new Error('Diff worker unavailable');
        }
      },
    };
    pending = Promise.all([
      import('monaco-editor/editor/editor.api.js'),
      import('monaco-editor/basic-languages/monaco.contribution.js'),
    ]).then(([api]) => api).catch(error => {
      pending = undefined;
      throw error;
    });
  }
  try {
    const api = await pending;
    return { api, release: () => { failures.delete(onWorkerFailure); } };
  } catch (error) {
    failures.delete(onWorkerFailure);
    throw error;
  }
}
