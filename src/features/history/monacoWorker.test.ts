import { expect, it, vi } from 'vitest';
import { loadMonaco } from './monacoWorker';

// Exercise the real helper with local worker/API dependencies replaced. Keeping
// this separate prevents importActual from competing with a mocked helper used
// by component effects that import it concurrently during StrictMode replay.
const mocks = vi.hoisted(() => ({ worker: vi.fn() }));
vi.mock('monaco-editor/editor/editor.worker.js?worker', () => ({ default: mocks.worker }));
vi.mock('monaco-editor/editor/editor.api.js', () => ({ editor: {}, languages: {} }));
vi.mock('monaco-editor/basic-languages/monaco.contribution.js', () => ({}));

it('creates only local bundled workers and unsubscribes failure callbacks when a view releases its lease', async () => {
  const worker = new EventTarget(); mocks.worker.mockImplementation(function () { return worker; });
  const firstFailure = vi.fn(), secondFailure = vi.fn();
  const first = await loadMonaco(firstFailure), second = await loadMonaco(secondFailure);
  expect(mocks.worker).not.toHaveBeenCalled();
  expect(globalThis.MonacoEnvironment?.getWorker?.('', 'editorWorkerService')).toBe(worker);
  expect(mocks.worker).toHaveBeenCalledWith();
  const event = new Event('error', { cancelable: true });
  worker.dispatchEvent(event);
  expect(event.defaultPrevented).toBe(true);
  expect(firstFailure).toHaveBeenCalledOnce(); expect(secondFailure).toHaveBeenCalledOnce();
  first.release();
  worker.dispatchEvent(new Event('messageerror'));
  expect(firstFailure).toHaveBeenCalledOnce(); expect(secondFailure).toHaveBeenCalledTimes(2);
  second.release();
  worker.dispatchEvent(new Event('error', { cancelable: true }));
  expect(secondFailure).toHaveBeenCalledTimes(2);
  const constructorFailure = vi.fn(), third = await loadMonaco(constructorFailure);
  mocks.worker.mockImplementation(() => { throw Error('secret URL CANARY'); });
  expect(() => globalThis.MonacoEnvironment?.getWorker?.('', 'editorWorkerService')).toThrow('Diff worker unavailable');
  expect(constructorFailure).toHaveBeenCalledOnce(); third.release();
});
