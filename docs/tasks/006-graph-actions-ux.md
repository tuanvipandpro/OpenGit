# 006 — Graph actions UX and refresh alignment

## Intake

- Status: DONE; user requests removing the redundant Sidebar width UI and integrating branch/Hide/Solo/actions into the graph, plus fixing misplaced Refreshing feedback. Independent review PASS, AC1–AC6 achieved and all required findings CLOSED.
- Clarification: user explicitly chose graph UX first; Git mutations such as creating/switching branches and cherry-pick are later scope. Do not add fake or disabled mutation entries.
- Target: existing VERSION0.2.0, no version bump for this follow-up. No commit/push/tag/release authorized.
- Tier: T2, coordinated graph/menu/filter interaction and responsive layout; no graph ancestry algorithm, IPC, persistence migration or Git mutation changes intended.
- Starting tree: full uncommitted0.2.0 implementation and acceptance docs present; preserve all existing work.
- Root runtime model/effort observation unavailable. Planner/reviewer Sol/high; implementation normally Sol/medium. Maximum two concurrent subagents, normally one worker.

## Acceptance

| ID | Observable result | Validation | State |
| --- | --- | --- | --- |
| AC1 | No permanent references sidebar or Sidebar width slider; graph uses available width | Component/native responsive checks | Native PASS; graph1151px@1440 and712px@960 |
| AC2 | Branch/tag actions accessible from graph decorations; Hide/Solo supports multiple branches and clear/show-all | Interaction tests and native actual Git fixture | Tests/native PASS |
| AC3 | Hidden/off-page refs remain discoverable through compact graph References control; selected commit/diff and saved filters retained | Regression tests and native | Native UI/inspector/reopen PASS; legacyfields unchanged |
| AC4 | Menus support pointer and keyboard, safe focus/close behavior, viewport bounds and no stale actions across repo/session change | Focused tests/native | PASS; independent source90/native focus; T006-F01 CLOSED |
| AC5 | Refreshing feedback aligns with view tabs at wide/narrow sizes and retains quiet/manual/slow semantics | App checks/native visibility | Tests/native center alignment<=2px PASS at1440/960 |
| AC6 | Existing history/filter/search/inspector/persistence regression passes; no mutation placeholders or version bump | Frontend/typecheck/build and independent review | PASS; author226/typecheck/build/version/native production and final independent review |

## Plan / ownership

Planner returns ordered cohesive implementation packet(s), actual source files and focused checks; no edits/dispatch. Root owns this record, documentation and tests/e2e harness updates. One worker owns agreed frontend files. Freeze persistence/IPC/backend, manifests and layout algorithm unless evidence requires root reassessment.

## Cost

No explicit cap, usage unknown. Soft allocation: one planner, one normal worker, one reviewer; reuse for repairs. No parallel workers expected. Stop/reassess after two unsuccessful rounds on the same issue. Do not repeat unchanged Rust suites for UI-only changes.

## Dispatch / validation / review

Planner v020_planner returned a single cohesive frontend packet (Sol/medium): full-width graph with a compact References drawer; real ref badges open pointer/keyboard action menus using existing visibility helpers; grid semantics permit interactive badges; stale context closes menus; aligned tab feedback. Legacy view fields remain unchanged and ignored by the new layout. No Git mutation menu placeholders. Root adapts native UI/inspector harnesses, retains selection/filter/anchor/legacy-persistence outcomes and verifies wide/narrow layout plus production build. Required independent verdict PASS before completion.

Implementation order: (1) References drawer/full-width screen, (2) graph badge menu and grid keyboard semantics, (3) tab feedback alignment, (4) focused interaction/regression checks. One writer owns HistoryScreen/CommitRows/RefsSidebar and their tests, new RefActionsMenu/ReferencesPanel and tests, history.css, App/test/styles. Stores, layout algorithm, contracts and backend are read-only. Root owns docs/e2e. Independent reviewer checks requirements, task-only delta and existing regression evidence.

Worker graph_ux_worker (worker_standard Sol/medium) source freeze14files `.tmp/task006-worker-freeze.json`, manifest SHA256A05DACE6689FF9BD314F1594A3904C12BC8D486A7ED5F41B5E85FA21EFB0F9FD. Author221 frontend tests/27files, typecheck/build/version/diff-check PASS. No source edits to stores/contracts/backend/dependencies. Task-only baseline source/harness copies `.tmp/task006-baseline-26375835d0284c79afbf050049356446`. Root syntax PASS for adapted native graph/inspector harness; native build underway. v020_reviewer independently reviews original requirements, plan and task-only delta.

## Handoff

Complete. Graph ref menus/temporary References and aligned Refreshing shipped in the local0.2.0 build. All ACs and independent review PASS. Git write features remain explicitly outside this task; no commit/push/tag/publication performed.

## First review and native probes

Worker source221 tests/27files +typecheck/build/version/diff-check PASS. Independent v020_reviewer85 focused tests,14/14freezehashes and8unchanged store/watch/search guards PASS. No ancestry/store/backend changes. Native debug e2e Tauri build PASS, unique artifact `.tmp/task006-native-2f5093b5833840aab8687cb0d3c81aac/opengit.exe`, SHA25669B1A919E828E0F795F4F4DAF436E8BDACEEE54FD363C259139869B16CD1CC7D.

T006-H01/P2 root harness adaptation: first native attempt incorrectly expected stash names visible while References closed, and explicit [role=dialog] did not match native dialog implicit role. Root fixed stash discovery via actualdialog, dialog[open]selectors, secondrepo defaultfilter assertion, gridaria-rowindex anchor and preserved legacyfields snapshot. SyntaxPASS. Native retry passed initialdrawer/stash, fullwidthgraph1151px at1440 and712px at960, Canvas/DPR/boundedDOM, refreshverticalalignment, ShiftF10/HomeEnd/Escape focus/menu bounds beforefilter.

T006-F01/P2 confirmed by native retry: graph AddtoSolo completes new snapshot but focus becomes body/html; immediate menu-close/sameinstance unit tests missed async disabled/remount path. Reviewer predicted sourcecause; root observed actual activeElement assertion failure in `.tmp/task006-history_ui_acceptance-retry.log` line158 and sent requiredrepair to worker. Responsibleworker must restore durable focus acrossfilter/snapshot replacement without stealinglater userfocus, and add integrated deferredsession regression. Inspector did not run after firstcasefailed. No nativePASS/completion claim yet; independent repair review required.

## T006-F01 repair round1

Worker graph_ux_worker implements bounded four-file repair: focus stable References before filter dispatch; loading badges use aria-disabled and guarded handlers so backgroundrefresh does not blur before controlled cleanup; transfer focus on graph disposal only if oldgraph still owns it. Integrated real-store delayed response tests cover new snapshot, retainedfailedsession, externalrefresh, movedSearchfocus and leavingHistory. Author focused27 then full226tests/27files, typecheck/build/version/diff-check PASS. Freeze `.tmp/task006-worker-freeze-repair1.json`, SHA25659860DA30D4A01C7981FECC8E078B6A15E401A5551E1E32D5ECB72464A4E22BE. Independent re-review/native rebuild pending; no finding self-closure. No shared store/backend/dependency changes or escalation.

## Repair1 integrated native evidence

Independent source repair90tests and14hashes PASS, no newfinding. Root repair e2e Tauri build PASS; exact artifact `.tmp/task006-repair1-native-fdc8fe1b1a094c2db97ad8d1f968069d/opengit.exe` SHA256C03B8ABC22F07C9477F48E8B7AA2B460001C6594584FAF13F09454189CDDD47E. Full UI+inspector batch EXIT0: graph action focus survives actualnewsession; keyboard/menuclamp/noaccidentalrowselect/multiSolo/Referencesstash+hiddenrestore/fullsearch/sourcepreservation PASS. Graph1151px at1440x900,712px at960x640,12/10virtualrows; manualRefreshfeedback center delta<=2px and nooverflow. Graph paints before laterproductionbuild; telemetry samples are functional-run debug observations, not a new releaseperformance threshold/comparison.

Inspector realMonaco/merge/binary/whitespace/refresh/latestfile, outsidefilter, two independentrepo filters/fastswitch, durablelayout/detailsheight/selection/anchor/reopen and main+shallow sourcepreservation PASS. Legacy sidebarWidth/sidebarCollapsed captured from acknowledgedSQLiteview remain unchanged; no oldcontrols or permanentpane, References defaultsclosed onrestart. Logs `.tmp/task006-repair1-history_ui_acceptance.log` and `-history_inspector_acceptance.log`. Screenshots graphdesktop-6e9389b0a3af441bae0755de391740fd and inspectordesktop-2b08084bd13b4737beb143b8acfc5880; root visually inspected inspector960.

No-e2e production Tauri build PASS1m18, `src-tauri/target/release/opengit.exe` SHA2564972EC7218A3D7DFEA39FFD1CB3C6E6A310DDF192CB2336C961F3AB25702AB3C. Productionnative launch underway. README now explains actualgraph/refmenus and226frontendtests; historyarchitecture documents legacylayout compatibility; initial0.2acceptance is marked historicalsnapshot with currentUX task006link. Source unchanged sincefreeze; no Gitmutations/schema/dependency/version changes and no commit/pub.

## Final independent acceptance

v020_reviewer final PASS AC1–AC6, T006-F01/H01 CLOSED, no new findings. Independent90focused tests and14/14freezehashes retained; reviewer inspected native logs/artifact hashes and graph/inspector screenshots. Full native UI/inspector and no-e2e production launch EXIT0, production4972EC...02AB3C chooser literal absent, persistedworkspace/typedIPC/panes1280/960 PASS. Final rootversion0.2.0/syntax2/diffcheck PASS. Product source unchanged since repair1freeze. No repeated unchangedRust suite; earlier0.2backend evidence remains historical and valid for unchangedsource. Runtimeusage/cost unknown. Root only records completion after reviewer verdict.

Current executable: `src-tauri/target/release/opengit.exe`. Native logs `.tmp/task006-repair1-history_ui_acceptance.log`, `-history_inspector_acceptance.log`, `.tmp/task006-production-acceptance.log`; productbuild `.tmp/task006-production-build.log`. All root tool sessions ended. New UX available by restarting the app with this build. VERSION remains0.2.0; cherry-pick/create-switchbranch implementation deferred as explicitly requested, without placeholders. No commit, push, tag or release publication.
