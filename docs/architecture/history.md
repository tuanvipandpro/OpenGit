# History contract — additive foundation v1, target 0.2.0

S1 freezes Rust/TypeScript DTOs and wire examples. S2 implements `open_history`, `load_history_page`, `search_history`; S4 implements details/diff and repository views. S6 implements backend watch activation/deactivation, the typed desktop event bridge and frontend convergence. Implementation and acceptance status are recorded separately in `docs/testing/0.2.0-acceptance.md`. Existing twelve foundation commands keep IPC contract1. Canonical shapes: `src/contracts/history.ts`, `src-tauri/src/domain/history.rs`; examples: `fixtures/contracts/history-wire.json`.

All properties are required, including explicit nullable properties. Requests and nested request objects reject unknown fields; JSON uses camelCase and exact enum spelling. Commands keep `{request: ...}` and `ApiResult<T>`. There is no renderer offset, argv, filesystem path, blob ID or executable revision-expression API. Persisted OID hints never become Git arguments.

## Safe Git capability and query bounds

Foundation keeps its Git 2.31 minimum. New history reads probe both global `--no-lazy-fetch` and `--shallow-file` options with `--version`, once/service from the trusted executable's neutral directory, using `NUL` on Windows or `/dev/null` elsewhere. Unsupported capability returns `UnsupportedGit` / inspection with static guidance: “Installed Git does not support safe history reads. Update Git before opening history.” Do not infer a guessed minimum Git release from these options.

Every history subprocess uses `--no-lazy-fetch`, `--no-replace-objects`, explicit captured shallow input, and a scoped empty `GIT_ALLOW_PROTOCOL` allowlist. The private history runner policy sets `GIT_GRAFT_FILE` to the trusted null file after clearing inherited scope overrides. Foundation runner behavior otherwise remains separate. Replace refs, nonempty legacy graft files, partial-clone/promisor configuration and `.promisor` pack markers are explicitly unsupported and checked around queries. Reads never fetch, mutate Git, run hooks, external diff or textconv. See [Git command documentation](https://git-scm.com/docs/git#Documentation/git.txt---no-lazy-fetch).

Capture shallow metadata before traversal. A captured nonshallow query always uses the explicit null shallow file; a transient new repository shallow file cannot change the captured traversal. Windows existing shallow files are opened no-follow, with read sharing only, and held through the BQ lifetime guard. Captured bytes and file identity are rechecked. This temporarily blocks external shallow-file write/replacement during a bounded query, while ordinary ref/worktree writes remain possible. The lease ends after confirmed process stop, not session expiry. Platforms lacking the proven shallow pin reject shallow history safely; no temporary shallow snapshot is written into Git or application state.

Traverse immutable positive OIDs into bounded topological vectors: at most 100,000 commits, otherwise `OutputLimit`, never silent truncation. Graph pages load raw commit metadata on demand. Full-history literal search scans metadata checks in chunks of at most 4096 OIDs, then packs raw reads by actual object size to 4 MiB (one individually bounded larger object may use the 8 MiB process cap). It does not launch one process/commit or preload every message at session creation. Preserve Unicode lowercase and newline literal semantics; no regex/grep approximation or mailmap rewriting.

For captured shallow histories, Git traversal order is only an initial rank: Git omits cut edges when sorting, while the API preserves raw parents. Before publishing any page/cursor, collect bounded parent metadata for the captured set and replace captured shallow-cut parents from raw commit headers. Sort the induced child-to-parent graph separately for visible and all-history sets, using original Git rank as a stable tie-break. Only edges whose endpoints are already captured affect ordering; roots, captured sets, raw parent arrays and boundary truth remain unchanged. Nonshallow histories retain their existing fast path. The renderer keeps its parent-after-child guard.

Shallow normalization uses at most100,000 nodes and300,000 final parent occurrences, including parents outside the captured set. Internal parent metadata has a cumulative32MiB transfer budget: the SHA-256 upper estimate `(100000+300000)*65` is26,000,000 bytes. Size-preflight framing has a separate cumulative32MiB budget; raw shallow-object frames/bodies have a cumulative128MiB budget. Batches contain at most4096 full OIDs and target4MiB by object-size preflight, with the existing hard8MiB process/input cap for every command. Discard raw bodies after extracting headers; adjacency/heap storage is bounded by node/edge counts. These are transfer/structure bounds, not an exact RSS claim. All phases share the existing query deadline and cancellation; overflow or malformed/incomplete graph data fails before session publication. This does not fetch, deepen or inspect parent objects outside the captured child set.

Each query has a30-second monotonic subprocess budget; BQ may additionally need up to5 seconds to prove shutdown. Stdin is bounded to8MiB and written concurrently with both output drains; Windows writes are explicitly flushed before EOF so background write failures are observed. Directory arguments use the existing verified Windows path conversion. Pure directory/shallow leases follow BQ until confirmed stop, or remain quarantined until process exit on uncertainty, without Backend/SQLite/scheduler/secret retention. Git job admission is separately bounded at64 jobs and1024 held process identities/job. Inherited directory/graft/shallow/replacement-base overrides are cleared.

## Scope, capabilities and lifecycle

`HistoryScope={repoId,worktreeId,generation}` uses registered repository/worktree identities; `worktreeId=null` is valid only for bare repositories. `HistoryContext={scope,snapshotId}`. Window identity comes from trusted Tauri context, never a renderer window ID. History responses echo context; view responses echo scope/viewRevision; watch activation/events echo scope and subscription token/sequence.

Session/snapshot/cursor/commit/file/watch tokens are random opaque capabilities with at least128 random bits. The implementation combines two UUIDv4 values into64 hex characters (a single UUIDv4 has122 random bits). Tokens bind backend process, issuing window, repository/worktree and exact generation; history capabilities additionally bind snapshot/session. Cross-window/scope/snapshot/restart/expiry mismatch returns `InvalidHandle`. Fingerprint changes revoke the session and return `ExternalChange`. Display paths never confer filesystem authority.

Session TTL is900 seconds from runtime entry creation, never extended by reads. Maximum8 sessions/window, evicting least recently accessed entries on a ninth publication. Maximum100,000 commit handles,20,000 file handles and256 cursors/session; overflow returns `OutputLimit` without partial results. Commit handles deduplicate by OID/session. File handles retain commit, selected diff base, raw paths, modes and blob OIDs. Cursors are reusable/idempotent until invalidation, bound to traversal/query/page size; search cursors also capture the graph-loaded frontier. They cannot be mixed between page/search or changed queries/page sizes. No runtime capabilities are persisted. Close/forget/switch revokes the corresponding state; watch replacement revokes the previous watch token.

## Commands

| Command | Request | Success |
| --- | --- | --- |
| `open_history` | `{scope,filter:{hiddenRefs,soloRefs},pageSize:number|null}` | `HistorySession {context,sessionId,head,refs,roots,page}` |
| `load_history_page` | `{context,sessionId,cursor,pageSize:number|null}` | `HistoryPage {context,sessionId,commits,nextCursor}` |
| `search_history` | `{context,sessionId,query,cursor:Id|null,pageSize:number|null}` | `HistorySearchPage {context,sessionId,query,commits:[{commit,outsideView}],refs,nextCursor}` |
| `get_commit_details` | `{context,commitHandle,base}` | `CommitDetails {context,commit,message,committerName,committerEmail,base,files}` |
| `get_commit_diff` | `{context,fileHandle}` | `CommitDiff {context,fileHandle,content}` |
| `get_repository_view` | `{scope}` | `RepositoryViewState {scope,viewRevision,view}` |
| `save_repository_view` | `{scope,expectedRevision,view}` | same; transactional compare-and-swap |
| `watch_repository` | `{scope}` | `RepositoryWatch {scope,subscriptionId,sequence,status}` |
| `unwatch_repository` | `{subscriptionId}` | `null`, idempotent while the issuing window's stopped-token cache retains it |

`nextCursor=null` means complete, never an estimate. History pageSize defaults200, max500; search defaults/max100. Positive integers only; omitted properties are invalid. Search query is literal UTF-8,1–512 bytes, no NUL. Search checks SHA prefix, full message, author name/email/full `Name <email>` and qualified ref names across the captured all-root universe, independently of visible filters and loaded pages. Cursor continuation requires the exact query. Ref matches are on the initial search page only. `outsideView=true` allows scoped details without changing filters or inventing graph edges.

## DAG truth and filters

Default roots are resolved positive commit OIDs from local/remote branches and recursively peeled commit tags, plus detached HEAD. Normal HEAD is already included by its branch; unborn HEAD has no OID. Noncommit tags have `commitOid=null`; annotated tag `oid` remains the tag object. Symbolic refs retain `symbolicTarget` and decorations without duplicate roots. Logical HEAD is resolved through Git rather than treating reftable's loose compatibility stub as authoritative. Detached symbolic-ref failure is accepted only as exit1 with empty output, followed by a validated resolved OID.

All bounded stash reflog entries are inspectable/searchable but excluded from default graph roots. Current uses `refs/stash`; older entries use captured virtual allowlist keys `refs/stash/<index>`, `kind=stash`, `displayName=stash@{n}: <summary>`. These are snapshot identifiers, not actual refs or renderer revision expressions, and are never passed to Git. Git reads files/reftable stack storage. Contiguous selectors plus a bounded tail lookup detect silently omitted missing objects. Existing-log EOF requires exit128 with empty stdout/stderr; unrelated fatal stderr is rejected. A current ref without a reflog is explicitly recognized by `reflog exists` exit1/empty and remains inspectable as the sole current entry. Entry0 must match captured `refs/stash`; the full stack's indices/OIDs/summaries participates in the fingerprint. Stack/combined ref overflow fails, never truncates.

Qualified names are bounded to1024 UTF-8 bytes and valid Git ref-name syntax. Filters are sets: sort/deduplicate; reconcile unknown/deleted names out when opening history. Hidden names omit roots/decorations. Solo is the union ancestry of selected captured roots, then hidden selected roots are omitted. Never subtract hidden ancestry using negative roots; shared ancestors remain. Empty effective roots produce a valid empty page. `roots` reports the deduplicated resolved OIDs.

Use topological order, never timestamp-only order. `CommitSummary.parents` preserves every raw parent in object order, including merge/octopus/shallow parents. Boundaries are `loaded`, `offPage`, `outsideView`, `shallow`, `missing`; page boundaries do not invent roots. Raw shallow parents must come from commit objects, not traversal output that suppresses them. Frontend reconciles loaded edges by OID as pages arrive.

Fingerprint registered common/worktree/Git identities, mappings, all refs/symbolic targets, full stash stack, logical/raw HEAD and shallow metadata before/after reads. Discard changed results rather than mix snapshots. Missing objects produce explicit boundaries or structured errors, never false roots. Failed response budgets do not advance the graph-loaded frontier.

## Details, diff and limits

`DiffBase={kind:'root'}` is valid only for zero raw parents; `{kind:'parent',index}` selects a zero-based raw parent. Invalid root/index combinations return `InvalidRequest`. Details return complete bounded message/committer/file metadata and opaque handles. Root/add/delete compare with absence/empty content as appropriate. Raw NUL-delimited diff-tree paths stay in backend records; lossy display names never roundtrip to argv. Rename/copy handles retain both sides, including raw modes. Symlink blobs are read as text, never dereferenced. Gitlinks are typed unavailable.

Diff-tree uses `--no-ext-diff`, `--no-textconv`, explicit submodule inclusion/short mode and no-relative output; user config cannot hide gitlinks. Rename/copy detection has a1000-candidate exhaustive limit. Successful file-list stderr refuses the entire comparison with `OutputLimit` and an accurate static file-list-warning message, without claiming a particular warning or exposing diagnostics. No partially classified A/D list or file-handle publication is returned.

`DiffContent={kind:'text',original,modified,language:string|null}` or `{kind:'unavailable',reason}` where reason is `binary`, `gitlink`, `tooLarge`, `unsupportedEncoding`, `missingObject`. Complete UTF-8 only; NUL means binary. Check blob types/sizes before allocation; no partial text/truncation. Raw bytes may fit while encoded JSON exceeds its separate cap and returns `OutputLimit`.

| Bound | Maximum / behavior |
| --- | --- |
| Request encoded JSON |16KiB |
| Graph page default/max |200/500 |
| Search page/query |100 /512 UTF-8 bytes |
| Session TTL/count per window |900 seconds /8 |
| Commit/file handles; cursors per session |100,000 /20,000 /256 |
| Traversal vector; refs including stash |100,000 commits /20,000 refs |
| Complete message; qualified ref name |65,536 /1024 UTF-8 bytes |
| Complete file list |10,000 files |
| Blob/combined diff raw bytes |1MiB each /2MiB total |
| Runner stdin/output; entire encoded success response |8MiB each /8MiB |

Metadata/JSON limits include escaping and envelopes; overflow is explicit, not silent truncation. These are safety maximums, not latency targets. Monaco is direct lazy local monaco-editor0.57.0, readonly, with local Vite ES workers, CSP self/blob workers and no wrapper/CDN/wildcard/unsafe-eval. Frontend lifecycle/CSP/native behavior has separate acceptance.

## Durable schema3 view

`RepositoryView` contains only filter, selectedOid, anchorOid, anchorOffset, sidebarWidth, detailsHeight, diffLayout and sidebarCollapsed. Store per registered common repository identity; linked worktrees share preferences while runtime sessions remain worktree-bound. Defaults: empty filter/selection/anchor, anchorOffset0, sidebarWidth280, detailsHeight320, sideBySide, not collapsed. Bounds: width160–640, height160–1200, finite signed anchorOffset−10,000–10,000.

Selected/anchor OIDs are UI restoration hints: full40- or64-digit hexadecimal syntax, normalized lowercase, or null. Preference reads/writes do not invoke Git or promise current object format/existence. History hydration/search reconciles these hints against the current format and available snapshot before obtaining a commit handle. Persisted Hide/Solo accepts durable branch/tag/remote names and real current `refs/stash`; snapshot-only `refs/stash/<index>` filters are rejected with static `InvalidRequest` guidance. Older stash sidebar entries are selection-only, with durable selectedOid. S2 ephemeral stash filtering remains snapshot-scoped.

Scope must be an open window/current generation and the active registered repo/worktree. Runtime ticket validation serializes synchronous view operations with close/switch; database transactions recheck registration/identity/selection. Initial revision is decimal0; saves compare expectedRevision and increment atomically. Stale saves return `ExternalChange`; refetch/reconcile current intent rather than overwrite newer state. Revision overflow fails instead of wrapping.

The complete schema0/1/2→3 upgrade is one transaction preserving workspace/operation/clone-intent rows, rolling the entire upgrade back on failure. repository_views has a cascading repository FK, nonnegative integer revision and bounded JSON value. Corrupt/oversized/unknown-field values fail without reset; readonly/migration failures preserve prior data. Never persist runtime cursors/handles/subscriptions or credentials.

## Watch and refresh seam

`opengit://repository-changed` carries only `{scope,subscriptionId,sequence,reason}`. No paths, filenames, raw diagnostics or file content. Scope/window come from trusted backend registration/context. Status is `watching` or `fallback`; event reason is `changed`, `overflow` or `unavailable`. Activation baseline is decimal sequence0; each event increases it. The bridge revalidates the current token before delivery. Sequence exhaustion emits a final unavailable hint then stops the worker, never wraps.

One active subscription/window; a successful replacement cancels the old token. Maximum64 active/retiring native callback/actor resource slots and three logical roots. Each native callback does constant work: atomic reason bits and wakeup, discarding paths/errors. Resource slots release only after actor and native callback owners drop. Re-arm waits for the previous callback lifetime to end before creating another native watcher. The actor wakes at most every50ms while idle, without Git polling. No Backend/database/scheduler/auth state, no ownership cycle, no unbounded join and no long-lived no-delete directory lease is retained.

Activation validates current registration IDs, stored identities/canonical path blobs and active repo/worktree before changing the session scope. The checked begin, installation and final response recheck run with session-state then database lock ordering; both locks are released before waiting for native readiness. A switch/forget or registration change rejects the stale request with InvalidHandle and disposes its pending token without affecting a newer subscription. Rejected activation cannot poison the next legitimate scope. Physical missing/inaccessible roots remain the worker's fallback concern; registered membership checks do not turn unavailable watching into a false success.

Use a minimal recursive ancestor cover of the worktree, resolved Git and common directories. This covers linked worktrees while avoiding redundant native child handles: on Windows those child watches prevent ancestor rename despite FILE_SHARE_DELETE. Check registered identities and .git/commondir mappings every second; a replacement with a different identity stays in fallback until valid reactivation. Native setup waits at most2 seconds for the caller, then returns fallback. The isolated worker can finish/recover later; a stalled native ACK cannot block backend/window disposal. Cap exhaustion leaves periodic refresh usable.

This does not promise universal directory moves. The covered main-repository/root probes permit moving the watched root itself; Windows can still return AccessDenied when moving an unwatched ancestor containing a required descendant native watch. The main repository directory above a linked worktree's watched common `.git` directory is one example. This is a proven native watcher limitation, not an extra no-delete pin. Close the application before these storage/ancestor moves; an unwatch response does not prove immediate native-thread/handle release. No ancestor- or drive-wide watch is added. Independent S6a review accepted this documented boundary; it is not a universal rename guarantee.

Debounce150ms with a1-second maximum burst window while responsive. Retry unavailable native watches/identities each second. Known rescan/drop/emission failures and identity loss publish overflow/unavailable hints before attempting native re-registration, whose upstream ACK may block. Atomic file replacement is observed through directory watches. Event broadcast is bounded256; lag and failed desktop emission mark overflow. Native recovery emits a changed hint. Close/forget/switch suppresses late old-token delivery.

Unwatch is idempotent for the issuing window while its stopped-token cache retains the token: maximum256/window, TTL900 seconds. Expired/evicted or other-window tokens return `InvalidHandle`. No watcher failure disables the independent30-second Git fallback. Silent event loss remains recoverable through that fallback; Git stays authoritative.

### Frontend integration

1. Listen before activating. Call `watch_repository` with the exact active history scope. Buffer events until the activation response supplies the token; baseline remains0 even if buffered events arrived first.
2. Match repo/worktree/generation and subscription token. Parse sequence as BigInt, expecting first1; ignore old/duplicates. A forward gap or overflow requires full authoritative refresh. The first unavailable hint refreshes; a degradation latch survives generation/token replacement for the same selection, preventing a refresh/reactivation loop. A changed recovery hint clears the latch.
3. Send accepted hints through the existing coordinator's watch reason, preserving one active request/one pending rerun. Hidden hints mark dirty only; resume converges once. Keep the30-second active-visible fallback regardless of watch status. Avoid an unavailable/immediate-reactivation loop; backend already retries.
4. Rebind on active scope/generation changes. Fallback activation remains a valid token; a later changed recovery hint can refresh/rebind and obtain explicit watching status. Replacement changes token and resets sequence0.
5. Dispose/switch/forget deactivates the issued token. Ignore late responses/events; if activation resolves after disposal, unwatch its returned token. Expired cleanup InvalidHandle is harmless. Do not create another repository polling timer or bypass the coordinator.

App owns one `workspace.connectHistory(history)` connection across Overview/History tabs. Watch binding follows `preferences.scope`, established before history publication, and shares the history generation counter. Pending listener setup never blocks history reads; failures retry only at existing coordinator triggers. Pre-ACK buffering is capped256 with an overflow resync flag. Selection epochs fence both inspection and history phases; failed selection restores the still-current repository through the coordinator.

The graph UX follow-up uses interactive qualified-ref badges and a temporary References dialog instead of a permanent ref sidebar. Drawer state is local and defaults closed. Legacy `sidebarWidth` and `sidebarCollapsed` remain in the validated persisted view for compatibility, unchanged by this UI; they no longer control graph width or drawer opening. No schema/IPC change is required. Filter actions still use the authoritative current store filter; menu context is invalidated on loading/session/ref changes. See task006 for independent/native verification.

The coordinator awaits inspection and history, including fresh search replay and inspector restoration. Search and inspector restore concurrently; the newest submitted query wins. Retained search results are labelled and cannot dispatch stale handles. Comparison content keeps its label until fresh handles are rebound. S1 focus debounce150ms, post-related-operation refresh, quiet background content, manual immediate feedback and slow feedback after500ms remain. Operation-journal polling stays separate. Native convergence evidence is required in addition to backend checks; see the acceptance record.
