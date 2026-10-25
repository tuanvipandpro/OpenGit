import { act, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import type * as Monaco from 'monaco-editor/editor/editor.api.js';
import type { MonacoLease } from './monacoWorker';
import { MonacoDiff } from './MonacoDiff';

const mocks = vi.hoisted(() => ({ load: vi.fn(), factory: vi.fn() }));
vi.mock('./monacoWorker', () => { mocks.factory(); return { loadMonaco: mocks.load }; });

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

it('does not load Monaco on component import, and mounts a labelled readonly pair only after loading', async () => {
  expect(mocks.factory).not.toHaveBeenCalled();
  expect(mocks.load).not.toHaveBeenCalled();
  const ready = deferred<MonacoLease>(), f = fixture(); mocks.load.mockReturnValue(ready.promise);
  const view = render(<MonacoDiff original="old" modified="new" layout="inline" />);
  expect(screen.getByRole('status')).toHaveTextContent('Loading text diff');
  await waitFor(() => expect(mocks.load).toHaveBeenCalledOnce());
  expect(f.api.editor.createModel).not.toHaveBeenCalled();
  await act(async () => ready.resolve(f.lease));
  expect(f.models.map(value => value.text)).toEqual(['old', 'new']);
  expect(f.diff.setModel).toHaveBeenCalledWith({ original: f.models[0], modified: f.models[1] });
  expect(f.api.editor.createDiffEditor).toHaveBeenCalledWith(expect.any(HTMLElement), expect.objectContaining({
    theme: 'vs-dark', readOnly: true, domReadOnly: true, originalEditable: false, ignoreTrimWhitespace: false, tabFocusMode: true, renderSideBySide: false,
    originalAriaLabel: 'Original text (read only)', modifiedAriaLabel: 'Modified text (read only)',
  }));
  expect(screen.getByRole('region', { name: 'Read-only text diff' })).toHaveAttribute('aria-busy', 'false');
  view.unmount();
  expect(f.diff.dispose).toHaveBeenCalledOnce();
  for (const value of f.models) expect(value.dispose).toHaveBeenCalledOnce();
  expect(f.release).toHaveBeenCalledOnce();
  expect(observers[0].disconnect).toHaveBeenCalledOnce();
});

it('uses the latest file and layout after rapid changes while the module is loading', async () => {
  const ready = deferred<MonacoLease>(), f = fixture(); mocks.load.mockReturnValue(ready.promise);
  const view = render(<MonacoDiff original="stale" modified="stale" layout="inline" />);
  await waitFor(() => expect(mocks.load).toHaveBeenCalledOnce());
  view.rerender(<MonacoDiff original="next" modified="next modified" language="typescript" layout="sideBySide" />);
  view.rerender(<MonacoDiff original="final" modified="final modified" language="javascript" layout="inline" />);
  await act(async () => ready.resolve(f.lease));
  expect(f.models.map(value => [value.text, value.language])).toEqual([['final', 'javascript'], ['final modified', 'javascript']]);
  expect(f.api.editor.createModel).toHaveBeenCalledTimes(2);
  expect(f.api.editor.createDiffEditor).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({ renderSideBySide: false }));
});

it('updates both texts, language and explicit layout without leaking or rebuilding models', async () => {
  const f = fixture(); mocks.load.mockResolvedValue(f.lease);
  const view = render(<MonacoDiff original="a" modified="b" language="javascript" layout="inline" />);
  await waitFor(() => expect(f.diff.setModel).toHaveBeenCalledOnce());
  view.rerender(<MonacoDiff original="c" modified="d" language="typescript" layout="sideBySide" />);
  expect(f.models.map(value => [value.text, value.language])).toEqual([['c', 'typescript'], ['d', 'typescript']]);
  expect(f.diff.updateOptions).toHaveBeenLastCalledWith({ renderSideBySide: true });
  view.rerender(<MonacoDiff original="" modified="" language={null} layout="inline" />);
  expect(f.models.map(value => [value.text, value.language])).toEqual([['', 'plaintext'], ['', 'plaintext']]);
  expect(f.diff.updateOptions).toHaveBeenLastCalledWith({ renderSideBySide: false });
  view.rerender(<MonacoDiff original="" modified="" language="unknown-language" layout="inline" />);
  expect(f.models.every(value => value.language === 'plaintext')).toBe(true);
  expect(f.api.editor.createModel).toHaveBeenCalledTimes(2);
  expect(f.api.editor.createDiffEditor).toHaveBeenCalledOnce();
  expect(f.models.every(value => value.dispose.mock.calls.length === 0)).toBe(true);
  act(() => observers[0].notify());
  expect(f.diff.layout).toHaveBeenCalledTimes(2);
  view.unmount();
  act(() => observers[0].notify());
  expect(f.diff.layout).toHaveBeenCalledTimes(2);
});

it('does not allocate editors or models when unmounted before loading finishes', async () => {
  const ready = deferred<MonacoLease>(), f = fixture(); mocks.load.mockReturnValue(ready.promise);
  const view = render(<MonacoDiff original="a" modified="b" layout="inline" />);
  await waitFor(() => expect(mocks.load).toHaveBeenCalledOnce());
  view.unmount();
  await act(async () => ready.resolve(f.lease));
  expect(f.api.editor.createModel).not.toHaveBeenCalled();
  expect(f.api.editor.createDiffEditor).not.toHaveBeenCalled();
  expect(f.release).toHaveBeenCalledOnce();
  expect(observers).toHaveLength(0);
});

it('ignores a superseded mount resolving after its replacement', async () => {
  const stale = deferred<MonacoLease>(), current = deferred<MonacoLease>();
  const old = fixture(), next = fixture();
  mocks.load.mockReturnValueOnce(stale.promise).mockReturnValueOnce(current.promise);
  const view = render(<MonacoDiff key="old" original="a" modified="b" layout="inline" />);
  await waitFor(() => expect(mocks.load).toHaveBeenCalledOnce());
  view.rerender(<MonacoDiff key="new" original="next" modified="next modified" layout="inline" />);
  await waitFor(() => expect(mocks.load).toHaveBeenCalledTimes(2));
  await act(async () => current.resolve(next.lease));
  await act(async () => stale.resolve(old.lease));
  expect(old.api.editor.createModel).not.toHaveBeenCalled();
  expect(old.release).toHaveBeenCalledOnce();
  expect(next.models).toHaveLength(2);
  expect(next.diff.dispose).not.toHaveBeenCalled();
  view.rerender(<MonacoDiff key="new" original="updated" modified="updated modified" layout="inline" />);
  expect(next.models.map(value => value.text)).toEqual(['updated', 'updated modified']);
});

it('shows a static safe error when lazy loading fails, without displaying exception text', async () => {
  mocks.load.mockRejectedValue(Error('https://secret.invalid/path?token=CANARY'));
  render(<MonacoDiff original="a" modified="b" layout="inline" />);
  expect(await screen.findByRole('alert')).toHaveTextContent('Text diff could not be loaded.');
  expect(document.body.textContent).not.toMatch(/CANARY|secret.invalid/);
  expect(screen.queryByRole('status')).not.toBeInTheDocument();
});

it('releases partial allocations when editor construction fails', async () => {
  const f = fixture(); mocks.load.mockResolvedValue(f.lease);
  f.api.editor.createDiffEditor.mockImplementation(() => { throw Error('CANARY'); });
  render(<MonacoDiff original="a" modified="b" layout="inline" />);
  await screen.findByRole('alert');
  expect(f.models).toHaveLength(2);
  for (const value of f.models) expect(value.dispose).toHaveBeenCalledOnce();
  expect(f.release).toHaveBeenCalledOnce();
  expect(document.body.textContent).not.toContain('CANARY');
});

it('handles a late worker failure by disposing all live view resources once', async () => {
  const f = fixture(); mocks.load.mockResolvedValue(f.lease);
  const view = render(<MonacoDiff original="a" modified="b" layout="sideBySide" />);
  await waitFor(() => expect(f.diff.setModel).toHaveBeenCalledOnce());
  act(() => mocks.load.mock.calls[0][0]());
  expect(screen.getByRole('alert')).toBeInTheDocument();
  expect(f.diff.dispose).toHaveBeenCalledOnce();
  for (const value of f.models) expect(value.dispose).toHaveBeenCalledOnce();
  expect(observers[0].disconnect).toHaveBeenCalledOnce();
  view.unmount();
  expect(f.diff.dispose).toHaveBeenCalledOnce();
  expect(f.release).toHaveBeenCalledOnce();
});

it('does not create orphan resources if worker failure arrives before the loader resolves', async () => {
  const ready = deferred<MonacoLease>(), f = fixture(); mocks.load.mockReturnValue(ready.promise);
  render(<MonacoDiff original="a" modified="b" layout="inline" />);
  await waitFor(() => expect(mocks.load).toHaveBeenCalledOnce());
  act(() => mocks.load.mock.calls[0][0]());
  await act(async () => ready.resolve(f.lease));
  expect(f.api.editor.createModel).not.toHaveBeenCalled();
  expect(f.release).toHaveBeenCalledOnce();
  expect(screen.getByRole('alert')).toBeInTheDocument();
});
