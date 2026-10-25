import type { Counter, Head, Id, Selection, Timestamp } from './models';

export const HISTORY_LIMITS = { pageDefault: 200, pageMax: 500, searchPageMax: 100, queryBytes: 512,
  sessionsPerWindow: 8, commitsPerSession: 100_000, filesPerSession: 20_000, cursorsPerSession: 256,
  sessionTtlSeconds: 900, diffBlobBytes: 1_048_576, diffTotalBytes: 2_097_152, responseBytes: 8_388_608,
  refsMax: 20_000, detailFilesMax: 10_000, messageBytes: 65_536 } as const;
export type HistoryScope = Selection & { generation: Counter };
export type HistoryContext = { scope: HistoryScope; snapshotId: Id };
export type HistoryFilter = { hiddenRefs: string[]; soloRefs: string[] };
export type HistoryRefKind = 'local' | 'remote' | 'tag' | 'stash';
export type HistoryRef = { name: string; displayName: string; kind: HistoryRefKind; oid: string; commitOid: string | null; symbolicTarget: string | null; hidden: boolean };
export type ParentBoundary = 'loaded' | 'offPage' | 'outsideView' | 'shallow' | 'missing';
export type HistoryParent = { oid: string; boundary: ParentBoundary };
export type CommitSummary = { handle: Id; oid: string; parents: HistoryParent[]; authorName: string; authorEmail: string; authoredAt: Timestamp; committedAt: Timestamp; subject: string; decorations: string[] };
export type HistoryPage = { context: HistoryContext; sessionId: Id; commits: CommitSummary[]; nextCursor: Id | null };
export type HistorySession = { context: HistoryContext; sessionId: Id; head: Head; refs: HistoryRef[]; roots: string[]; page: HistoryPage };
export type HistorySearchHit = { commit: CommitSummary; outsideView: boolean };
export type HistorySearchPage = { context: HistoryContext; sessionId: Id; query: string; commits: HistorySearchHit[]; refs: HistoryRef[]; nextCursor: Id | null };
export type DiffBase = { kind: 'root' } | { kind: 'parent'; index: number };
export type CommitFileKind = 'added' | 'deleted' | 'modified' | 'renamed' | 'copied' | 'typeChanged' | 'unmerged';
export type CommitFile = { handle: Id; pathDisplay: string; previousPathDisplay: string | null; kind: CommitFileKind; oldMode: string | null; newMode: string | null };
export type CommitDetails = { context: HistoryContext; commit: CommitSummary; message: string; committerName: string; committerEmail: string; base: DiffBase; files: CommitFile[] };
export type DiffUnavailableReason = 'binary' | 'gitlink' | 'tooLarge' | 'unsupportedEncoding' | 'missingObject';
export type DiffContent = { kind: 'text'; original: string; modified: string; language: string | null } | { kind: 'unavailable'; reason: DiffUnavailableReason };
export type CommitDiff = { context: HistoryContext; fileHandle: Id; content: DiffContent };
export type RepositoryView = { filter: HistoryFilter; selectedOid: string | null; anchorOid: string | null; anchorOffset: number; sidebarWidth: number; detailsHeight: number; diffLayout: 'inline' | 'sideBySide'; sidebarCollapsed: boolean };
export type RepositoryViewState = { scope: HistoryScope; viewRevision: Counter; view: RepositoryView };
export type WatchStatus = 'watching' | 'fallback';
export type RepositoryWatch = { scope: HistoryScope; subscriptionId: Id; sequence: Counter; status: WatchStatus };
export type RepositoryChangeReason = 'changed' | 'overflow' | 'unavailable';
export type RepositoryChangedEvent = { scope: HistoryScope; subscriptionId: Id; sequence: Counter; reason: RepositoryChangeReason };

export type OpenHistoryRequest = { scope: HistoryScope; filter: HistoryFilter; pageSize: number | null };
export type LoadHistoryPageRequest = { context: HistoryContext; sessionId: Id; cursor: Id; pageSize: number | null };
export type SearchHistoryRequest = { context: HistoryContext; sessionId: Id; query: string; cursor: Id | null; pageSize: number | null };
export type GetCommitDetailsRequest = { context: HistoryContext; commitHandle: Id; base: DiffBase };
export type GetCommitDiffRequest = { context: HistoryContext; fileHandle: Id };
export type GetRepositoryViewRequest = { scope: HistoryScope };
export type SaveRepositoryViewRequest = { scope: HistoryScope; expectedRevision: Counter; view: RepositoryView };
export type WatchRepositoryRequest = { scope: HistoryScope };
export type UnwatchRepositoryRequest = { subscriptionId: Id };
export interface HistoryCommandMap {
  open_history: { request: OpenHistoryRequest; value: HistorySession };
  load_history_page: { request: LoadHistoryPageRequest; value: HistoryPage };
  search_history: { request: SearchHistoryRequest; value: HistorySearchPage };
  get_commit_details: { request: GetCommitDetailsRequest; value: CommitDetails };
  get_commit_diff: { request: GetCommitDiffRequest; value: CommitDiff };
  get_repository_view: { request: GetRepositoryViewRequest; value: RepositoryViewState };
  save_repository_view: { request: SaveRepositoryViewRequest; value: RepositoryViewState };
  watch_repository: { request: WatchRepositoryRequest; value: RepositoryWatch };
  unwatch_repository: { request: UnwatchRepositoryRequest; value: null };
}
