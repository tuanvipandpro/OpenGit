# OpenGit roadmap

Release completion is not claimed. Checkpoint0.2.0 ce5da58 has native Windows evidence;0.3.0 is being implemented under task007 and versions0.4.0 onward remain planned. The canonical current target is recorded in the root `VERSION` file; it does not imply that a release exists. This document is scope, not implementation evidence or authorization to build all versions in one task. First supported target: Windows; portable architecture for later macOS/Linux releases.

Current0.3.0 implementation/review status is tracked in [task007](tasks/007-implement-0.3.0.md) and [plan](plans/0.3.0.md); earlier [0.2.0 evidence](testing/0.2.0-acceptance.md) and external-provider EXT-01 in [0.1.0 evidence](testing/0.1.0-acceptance.md) remain separate. This roadmap does not replace living evidence.

## Requirement IDs

Tài liệu triển khai: [thiết kế ban đầu](architecture.md), [kế hoạch toàn sản phẩm](plans/implementation-plan.md), [packet0.2.0](plans/0.2.0.md). Kế hoạch mô tả scope; chỉ evidence xác nhận kiểm chứng implementation, không đồng nghĩa release đã phát hành.

| ID | Requirement | Delivery |
| --- | --- | --- |
| R1 | Open/clone public and private repositories via HTTPS, SSH, local paths using authorized access | v0.1.0; broader compatibility in v0.6.0 |
| R2 | Graph, branch/tag sidebar, commit/diff, history search, multiple-branch Hide/Solo, filters persisted per repo | v0.2.0 |
| R3 | Fetch, create/switch branch, tag, stage/commit, pull/push with explicit results/errors | v0.3.0; divergent pull strategies in v0.4.0 |
| R4 | Merge, cherry-pick, rebase, conflict resolution, interrupted-operation recovery | v0.4.0 |
| R5 | Cherry-pick across remotes and separate repositories in one workspace | v0.5.0, built on v0.4.0 |
| R6 | Responsive desktop UX inspired by GitKraken, with original assets and identity | Starts v0.1.0, measured from v0.2.0 onward |

## v0.1.0 — Repository and authentication foundation

- Application scaffold and typed IPC; choose/pin toolchain versions and record real build/test commands.
- Workspace containing multiple repositories; open local repo, recent repos, clone destination handling.
- Public/private clone via HTTPS, SSH, local path; progress and cancellation where safe.
- Reuse configured credential helpers/GCM and OpenSSH/agent. Prompt for auth appropriately; never log tokens or store them in SQLite.
- Detect Git installation/version and identity. Respect configured proxy, CA and SSH host verification.
- Task panel with structured status/errors; distinguish permission, credential, network, certificate and SSH problems.
- Basic status/current branch; detect existing unfinished Git operations without attempting unsupported recovery.
- Introduce task scheduling and operation-state model before later mutating features.

Acceptance: public and private HTTPS/SSH clone exercised in authorized test environments; failure does not overwrite existing destination data; errors remain actionable. Integration tests use isolated repositories, no production remotes.

## v0.2.0 — Graph and focused history

- Commit DAG with correct parent/merge edges, HEAD and branch/tag decorations.
- Sidebar for local branches, remotes, tags and stashes.
- Incremental loading, virtualized commit rows, Canvas drawing synchronized with scrolling.
- Commit details, file list, inline/side-by-side text diff.
- Search SHA, message and author across history, not just loaded rows; search branch/tag names.
- Hide individual/grouped refs, Solo multiple branches, clear/show-all, visible filter status.
- Persist filters/layout/selection per repo; refresh after external CLI/IDE changes.
- Refresh UX: replace the current 2-second repository polling with a 30-second fallback for the active repository, refresh on window focus and after relevant app operations, and pause automatic polling while the window is hidden. Background refresh stays quiet; manual or slow refresh gets visible feedback.
- Add filesystem watchers with debounce after the fallback policy is established. Watch the working tree and resolved Git/common directories, including linked worktrees; recover from missed/overflowed events with a full refresh. Watchers are hints, Git remains authoritative.

Hide/Solo is presentation-only. Preserve shared ancestors needed to interpret visible history; distinguish hiding decorations from excluding history. Search can reveal a result outside the current filter. Pagination boundaries and refresh must not invent or drop parent relationships.

Acceptance: merge-heavy fixture histories render correctly across filters and page boundaries; state survives reopen; selection/scroll stay stable. Establish a reference machine, representative dataset and measured performance baseline before setting release thresholds.

Implementation packets and validation for this scope are recorded in the0.2.0 plan/evidence: quiet refresh, paginated history/ref queries, graph/sidebar, details/diff, full search, multi-Hide/Solo/persisted views and watcher integration. External provider acceptance left open by0.1.0 stays tracked separately and is not closed by history work. No later-version implementation is authorized by this roadmap.

## v0.3.0 — Daily Git actions

- Selected-commit context: list local and remote-tracking branches containing the commit, current branch/upstream inclusion, independent of graph visibility and pagination. Tip decorations are not containment.
- Remote publication evidence is a local tracking snapshot with remote-specific fetch freshness/scope. Absence is not proof of never pushed; narrowed refspecs, retained deleted refs and shallow/incomplete ancestry must not produce a false definitive answer. Unknown/error states stay explicit.
- Fetch one/all remotes, optional prune; create/rename/switch/delete branches and tracking setup.
- Create lightweight/annotated tags; explicit local versus remote push/delete targets.
- Stage/unstage file, hunk, eligible text lines; commit/amend and hook/signing error visibility.
- Stash/apply/pop with conflict detection and a minimal resolve/abort path; full resolver in v0.4.0.
- Push, upstream setup and ahead/behind state; fast-forward-only pull initially.
- Explain divergence instead of silently choosing merge/rebase; full strategies arrive in v0.4.0.
- Preflight dirty state and existing operation; report actual results and failures.

Acceptance: commit containment matches real ancestry in full histories and reports conservative unknowns for incomplete data; fetch freshness is truthful for the relevant remote/scope. Branch → stage → commit → push → tag workflow works with isolated test remotes; rejected pushes and hook failures are correctly represented. No destructive mutation is tested on the development checkout.

## v0.4.0 — Integration and recovery

- Fast-forward/normal/squash merge and pull merge/rebase strategies.
- Single/multiple commit cherry-pick with explicit order; merge-commit mainline selection.
- Rebase and interactive reorder/reword/squash/fixup/drop.
- Conflict workflow showing base, both sides and editable result; handle rename/delete/binary cases explicitly; external merge tool fallback.
- Continue/abort and skip only where Git supports them.
- Recover UI after restarting during an operation; inspect Git state rather than trusting only app logs.
- Reflog inspection, recovery branch creation and revert; do not promise universal undo.
- If force pushing is exposed, explicit intent and force-with-lease semantics.

Acceptance: clean and conflicting paths pass disposable-repo tests; restart during merge/rebase/cherry-pick preserves accurate state and permits supported continuation/abort; existing work is not silently discarded.

## v0.5.0 — Cross-repository cherry-pick

- One repository with named remotes such as origin/upstream; multiple workspace repositories each with their own origin.
- Source repo/ref/commit selection → destination repo/branch → order/diff preview → execute → conflict/recovery → source-to-result mapping.
- Import objects via fetch from remote or local repository into app-managed refs; pin source SHA and recheck destination HEAD.
- Inspect dirty destination, in-progress operations and branch checked out in another worktree.
- Optional isolated-worktree trial; explicit cleanup/lifecycle for temporary refs and worktrees.
- Explain shallow-history/missing-object, incompatible-object-format and empty-commit cases.
- No silent path remapping or guarantee that logically dependent commits are detected.

Acceptance: same-repo and separate-repo transfers work, including conflict and interruption. Source repo is unchanged. Destination result/recovery is verified. A clean application is not claimed to prove application-level correctness.

## v0.6.0 — Compatibility, ergonomics and performance

- LFS and submodules, including independent private submodule auth.
- Worktree management and shallow-history deepening.
- Signing configuration UI; multiple-account and enterprise-network test coverage.
- Saved filter views, favorites, branch-prefix groups, shortcuts and command palette.
- Drag/drop opens an action preview for merge/rebase/cherry-pick.
- Large text, binary and image diff behavior with explicit display limits.
- Profile graph, watchers, caching, process scheduling and IPC on representative large histories/ref counts.

Acceptance: compatibility matrix and measured performance thresholds pass; no broad claim that all hosts or repository shapes have been tested.

## v1.0.0 — Stable release

- Complete R1–R6 with integrated evidence; no unresolved release-blocking findings.
- Windows packaging, signing/release process and controlled updates.
- Settings/cache migrations, diagnostics and redacted log export.
- Authentication/recovery documentation and explicit unsupported cases.
- End-to-end private clone → commit/push → conflict → restart/recovery → cross-repo cherry-pick.

Do not defer correctness, secret handling or recovery tests until v1.0.0; each owning version includes them. macOS/Linux release commitments are separate future scope.
