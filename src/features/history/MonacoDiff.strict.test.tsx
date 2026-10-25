import { StrictMode, useEffect } from 'react';
import { act, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import type * as Monaco from 'monaco-editor/editor/editor.api.js';
import type { MonacoLease } from './monacoWorker';
import { createMonacoDiff } from './MonacoDiff';

// Use the same component hooks with a controlled loader, independent of Vitest's
// concurrent dynamic-import mock implementation. The default lazy loader and
// actual worker helper remain covered by their separate suites.
const mocks = { load: vi.fn() };
const MonacoDiff = createMonacoDiff(mocks.load);

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function fixture() {
  const models: ReturnType<typeof model>[] = [];
  const model = (text: string, language: string) => ({
    text, language, dispose: vi.fn(),
    getValue() { return this.text; }, setValue(value: string) { this.text = value; },
    getLanguageId() { return this.language; },
  });
  const diff = { setModel: vi.fn(), updateOptions: vi.fn(), dispose: vi.fn(), layout: vi.fn() };
  const api = { languages: { getLanguages: () => [{ id: 'plaintext' }, { id: 'javascript' }, { id: 'typescript' }] }, editor: {
    createModel: vi.fn((text: string, language: string) => { const value = model(text, language); models.push(value); return value; }),
    createDiffEditor: vi.fn(() => diff),
    setModelLanguage: vi.fn((value: ReturnType<typeof model>, language: string) => { value.language = language; }),
  } };
  const release = vi.fn();
  const lease: MonacoLease = { api: api as unknown as typeof Monaco, release };
  return { api, models, diff, lease, release };
}

let observers: { disconnect: ReturnType<typeof vi.fn>; observe: ReturnType<typeof vi.fn>; notify: () => void }[];
beforeEach(() => {
  mocks.load.mockReset();
  observers = [];
  vi.stubGlobal('ResizeObserver', class {
    disconnect = vi.fn(); observe = vi.fn(); notify: () => void;
    constructor(notify: () => void) { this.notify = notify; observers.push(this); }
  });
});

it.each(['forward', 'reverse'] as const)('preserves the active editor across actual StrictMode replay with %s load completion', async order => {
  const setup = vi.fn(), cleanup = vi.fn(), requested = deferred<void>();
  function EffectProbe() {
    useEffect(() => { setup(); return cleanup; }, []);
    return null;
  }
  const loads: { ready: ReturnType<typeof deferred<MonacoLease>>; value: ReturnType<typeof fixture>; settled: boolean }[] = [];
  let finishing = false;
  mocks.load.mockImplementation(() => {
    const load = { ready: deferred<MonacoLease>(), value: fixture(), settled: false };
    loads.push(load); requested.resolve();
    // A late loader request after unmount must not leave its promise pending.
    if (finishing) { load.settled = true; load.ready.resolve(load.value.lease); }
    return load.ready.promise;
  });
  const liveLoads = () => loads.filter(load => load.value.models.length > 0);
  const resolveObserved = async () => {
    let pending = loads.filter(load => !load.settled);
    while (pending.length) {
      expect(loads.length).toBeLessThanOrEqual(2);
      if (order === 'reverse') pending.reverse();
      for (const load of pending) {
        load.settled = true;
        await act(async () => { load.ready.resolve(load.value.lease); });
        expect(screen.queryByRole('alert')).not.toBeInTheDocument();
        for (const live of liveLoads()) {
          expect(live.value.models.map(value => [value.text, value.language])).toEqual([['latest', 'typescript'], ['latest modified', 'typescript']]);
          expect(live.value.diff.dispose).not.toHaveBeenCalled();
          expect(live.value.release).not.toHaveBeenCalled();
        }
      }
      pending = loads.filter(load => !load.settled);
    }
  };
  const view = render(<StrictMode><EffectProbe /><MonacoDiff original="initial" modified="initial modified" layout="inline" /></StrictMode>);
  try {
    expect(setup).toHaveBeenCalledTimes(2);
    expect(cleanup).toHaveBeenCalledOnce();
    // Acknowledge this component's loader request, never global module state.
    // A closed effect may skip loading, so do not require two loader calls.
    await act(async () => { await requested.promise; });
    view.rerender(<StrictMode><EffectProbe /><MonacoDiff original="latest" modified="latest modified" language="typescript" layout="sideBySide" /></StrictMode>);
    // Resolve all requests observed so far; if a replay request arrives later,
    // retry only this local condition and include its controlled lease too.
    await waitFor(async () => {
      await resolveObserved();
      expect(liveLoads()).toHaveLength(1);
    });
    const active = liveLoads()[0];
    expect(active.value.api.editor.createDiffEditor).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({ renderSideBySide: true }));
    expect(screen.getByRole('region', { name: 'Read-only text diff' })).toHaveAttribute('aria-busy', 'false');
    for (const canceled of loads.filter(load => load !== active)) {
      expect(canceled.value.api.editor.createModel).not.toHaveBeenCalled();
      expect(canceled.value.api.editor.createDiffEditor).not.toHaveBeenCalled();
      expect(canceled.value.release).toHaveBeenCalledOnce();
    }
    expect(observers).toHaveLength(1);
    expect(active.value.models).toHaveLength(2);
    expect(active.value.models.every(value => value.dispose.mock.calls.length === 0)).toBe(true);
    // A stale completion must not clear the resource ref: new props still
    // update the active pair after both possible resolution orders.
    view.rerender(<StrictMode><EffectProbe /><MonacoDiff original="after replay" modified="after replay modified" language="javascript" layout="inline" /></StrictMode>);
    expect(active.value.models.map(value => [value.text, value.language])).toEqual([['after replay', 'javascript'], ['after replay modified', 'javascript']]);
    expect(active.value.diff.updateOptions).toHaveBeenLastCalledWith({ renderSideBySide: false });
    act(() => observers[0].notify());
    expect(active.value.diff.layout).toHaveBeenCalledTimes(2);
    expect(active.value.diff.dispose).not.toHaveBeenCalled();
    expect(active.value.release).not.toHaveBeenCalled();
  } finally {
    finishing = true;
    view.unmount();
    await act(async () => {
      for (const load of loads.filter(value => !value.settled)) {
        load.settled = true; load.ready.resolve(load.value.lease);
      }
    });
  }
  expect(cleanup).toHaveBeenCalledTimes(2);
  expect(liveLoads()).toHaveLength(1);
  for (const load of loads) {
    expect(load.settled).toBe(true);
    expect(load.value.release).toHaveBeenCalledOnce();
    for (const model of load.value.models) expect(model.dispose).toHaveBeenCalledOnce();
    if (load.value.models.length) expect(load.value.diff.dispose).toHaveBeenCalledOnce();
    else expect(load.value.diff.dispose).not.toHaveBeenCalled();
  }
  expect(observers[0].disconnect).toHaveBeenCalledOnce();
});
