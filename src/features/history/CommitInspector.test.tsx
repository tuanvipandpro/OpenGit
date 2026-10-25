import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { CommitInspector } from './CommitInspector';
import { createCommitInspectorStore } from '../../stores/commitInspector';
import type { Api } from '../../stores/workspace';
import type { DiffBase, DiffUnavailableReason, HistoryContext, CommitSummary } from '../../contracts';

vi.mock('./MonacoDiff', () => ({ MonacoDiff: ({ original, modified, layout }: { original: string; modified: string; layout: string }) => <div role="region" aria-label="Rendered text diff">{original} → {modified} · {layout}</div> }));
const context: HistoryContext = { scope: { repoId: 'repo', worktreeId: null, generation: '1' }, snapshotId: 'snapshot' };
const commit: CommitSummary = { oid: 'a'.repeat(40), handle: 'commit', subject: 'Merge subject', authorName: 'Author', authorEmail: 'author@example.test', parents: [{ oid: 'b'.repeat(40), boundary: 'offPage' }, { oid: 'c'.repeat(40), boundary: 'offPage' }], decorations: [], authoredAt: '', committedAt: '' };

it('shows commit/file metadata, switches merge parent and file, and routes layout changes', async () => {
  const api = vi.fn(async (command: string, request: { base: DiffBase; fileHandle: string }) => ({ ok: true, value: command === 'get_commit_details' ? { context, commit, base: request.base, message: 'Full merge message', committerName: 'Committer', committerEmail: 'committer@example.test', files: ['one', 'two'].map(handle => ({ handle, pathDisplay: `${handle}.txt`, previousPathDisplay: handle === 'two' ? 'old.txt' : null, kind: 'modified', oldMode: '100644', newMode: '100644' })) } : { context, fileHandle: request.fileHandle, content: { kind: 'text', original: `${request.fileHandle} original`, modified: `${request.fileHandle} modified`, language: null } } }));
  const store = createCommitInspectorStore(api as unknown as Api), selected = { context, commit, outsideView: true }, onLayout = vi.fn();
  render(<CommitInspector store={store} selected={selected} layout="inline" onLayoutChange={onLayout} />);
  await act(async () => store.getState().select(selected));
  expect(screen.getByText('Outside current filters')).toBeInTheDocument(); expect(screen.getByText('Full merge message')).toBeInTheDocument();
  expect(screen.getByRole('region', { name: 'Rendered text diff' })).toHaveTextContent('one original → one modified · inline');
  fireEvent.click(screen.getByRole('button', { name: /old.txt → two.txt/ }));
  await waitFor(() => expect(screen.getByRole('region', { name: 'Rendered text diff' })).toHaveTextContent('two original'));
  fireEvent.change(screen.getByRole('combobox', { name: 'Diff base' }), { target: { value: '1' } });
  await waitFor(() => expect(store.getState().details?.base).toEqual({ kind: 'parent', index: 1 }));
  expect(api).toHaveBeenCalledWith('get_commit_details', { context, commitHandle: 'commit', base: { kind: 'parent', index: 1 } });
  fireEvent.change(screen.getByRole('combobox', { name: 'Diff layout' }), { target: { value: 'sideBySide' } }); expect(onLayout).toHaveBeenCalledWith('sideBySide');
});

it.each(['binary', 'gitlink', 'tooLarge', 'unsupportedEncoding', 'missingObject'] as DiffUnavailableReason[])('displays typed %s feedback without mounting a text editor', async reason => {
  const api = vi.fn(async (command: string, request: { base: DiffBase; fileHandle: string }) => ({ ok: true, value: command === 'get_commit_details' ? { context, commit, base: request.base, message: '', committerName: '', committerEmail: '', files: [{ handle: 'file', pathDisplay: 'file', previousPathDisplay: null, kind: 'modified', oldMode: null, newMode: null }] } : { context, fileHandle: request.fileHandle, content: { kind: 'unavailable', reason } } }));
  const store = createCommitInspectorStore(api as unknown as Api), selected = { context, commit, outsideView: false };
  render(<CommitInspector store={store} selected={selected} layout="inline" onLayoutChange={() => {}} />);
  await act(async () => store.getState().select(selected));
  expect(screen.getByRole('status')).toHaveTextContent(/binary|submodule|limit|UTF-8|missing/i);
  expect(screen.queryByRole('region', { name: 'Rendered text diff' })).not.toBeInTheDocument();
});

it('keeps a captioned previous file/base visible while rebind loads and permits the latest file intent', async () => {
  let finish!: (value: unknown) => void;
  const freshContext = { ...context, scope: { ...context.scope, generation: '2' }, snapshotId: 'fresh' }, freshCommit = { ...commit, handle: 'fresh-commit' };
  const metadata = (fresh: boolean, base: DiffBase) => ({ context: fresh ? freshContext : context, commit: fresh ? freshCommit : commit, base, message: '', committerName: '', committerEmail: '',
    files: ['one', 'two'].map(name => ({ handle: `${fresh ? 'fresh' : 'old'}-${name}`, pathDisplay: `${name}.txt`, previousPathDisplay: null, kind: 'modified', oldMode: '100644', newMode: '100644' })) });
  const api = vi.fn(async (command: string, request: { context: HistoryContext; base: DiffBase; fileHandle: string }) => {
    if (command === 'get_commit_details') return request.context.snapshotId === 'fresh' ? new Promise(resolve => { finish = resolve; }) : { ok: true, value: metadata(false, request.base) };
    return { ok: true, value: { context: request.context, fileHandle: request.fileHandle, content: { kind: 'text', original: request.fileHandle, modified: 'Same line  ', language: null } } };
  });
  const store = createCommitInspectorStore(api as unknown as Api), selected = { context, commit, outsideView: false }, freshSelected = { context: freshContext, commit: freshCommit, outsideView: false };
  const view = render(<CommitInspector store={store} selected={selected} layout="inline" onLayoutChange={() => {}} />);
  await act(async () => { await store.getState().select(selected); await store.getState().setBase({ kind: 'parent', index: 1 }); await store.getState().selectFile('old-two'); });
  act(() => store.getState().beginRefresh(context)); view.rerender(<CommitInspector store={store} selected={null} layout="inline" onLayoutChange={() => {}} />);
  expect(screen.getByRole('region', { name: 'Rendered text diff' })).toHaveTextContent('old-two'); expect(screen.getByText(/Showing parent 2 · two.txt/)).toBeInTheDocument();
  let loading!: Promise<void>;
  act(() => { loading = store.getState().select(freshSelected); });
  expect(screen.getByRole('combobox', { name: 'Diff base' })).not.toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: /one.txt/ }));
  expect(screen.getByRole('region', { name: 'Rendered text diff' })).toHaveTextContent('old-two');
  await act(async () => { finish({ ok: true, value: metadata(true, { kind: 'parent', index: 1 }) }); await loading; });
  expect(screen.getByRole('region', { name: 'Rendered text diff' })).toHaveTextContent('fresh-one');
  expect(screen.getByText('Showing parent 2 · one.txt')).toBeInTheDocument();
  expect(api).toHaveBeenLastCalledWith('get_commit_diff', { context: freshContext, fileHandle: 'fresh-one' });
});
