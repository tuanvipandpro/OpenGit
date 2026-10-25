# 005 — Implement OpenGit 0.2.0

## Intake

- Status: COMPLETE — implementation0.2.0. Gates1/2/S6 and final integrated gate3 independently PASS; all required findings CLOSED. Integrated core138/frontend210, final inspector/watch/refresh, production launch and10k graph/performance checks PASS. No publication/commit/tag was performed.
- Original request: "ok bắt đầu implement bản 0.2.0 đi". Earlier user approved deferring quieter automatic refresh to this version and asked to control token/cost.
- Initial VERSION: 0.1.0; current/authorized target after S1: 0.2.0. Change class: minor, history/graph/filters and refresh features. Synchronize application versions during shared foundation work.
- Starting point: clean main, dded840 (0.1.0 checkpoint). No commit/tag/push/release authorized for this task.
- Tier: T3, graph ancestry/pagination, read subprocess/IPC boundaries, persisted view migration and filesystem watcher concurrency.
- Scope: R2/R6, A2/A6 and relevant F1–F5; full 0.2.0 roadmap, including quieter refresh and watchers.
- Non-goals: 0.3 Git writes, integration/conflict actions, cross-repo cherry-pick, installer/signing, external provider certification. Keep 0.1.0 EXT-01 explicitly open and separate.
- Root runtime model/effort observation unavailable; configured default Sol/medium. Planner named role requests Sol/high.

## Acceptance

| ID | Observable result | Verification | Status |
| --- | --- | --- | --- |
| AC1 | Paginated graph preserves real parent/merge relationships, boundaries, HEAD and ref decorations | Disposable DAG fixtures and independent review | PASS; S2-SH01 CLOSED; integrated core/native DAG regression PASS |
| AC2 | Sidebar branches/remotes/tags/stashes and virtualized Canvas history stay synchronized | UI and native desktop checks | PASS; final10005-node/two-stash Canvas and bounded DOM native |
| AC3 | Commit metadata/files and inline/side-by-side text diff, safe binary/large-file limits | Real Git fixtures, UI/native checks | PASS; final native inspector/API and backend failure fixtures |
| AC4 | Search SHA/message/author across full history and ref names, reveal outside current filters | Backend and UI tests beyond first page | PASS; S6-N01 CLOSED after final native outside-filter regression |
| AC5 | Multi-Hide/Solo changes visibility only, preserves shared ancestors, clear/show-all and visible active filters | Merge-heavy fixtures, source unchanged | PASS; final10k multi-Hide/Solo/search and source snapshots |
| AC6 | Per-repo filter/layout/selection persistence and stable scroll/selection across refresh/reopen | Migration and native restart checks | PASS; final fast-switch/reopen/anchor and schema/CAS checks |
| AC7 | Quiet 30s active-repo fallback, focus/post-operation refresh, hidden-window pause and manual/slow feedback | Timer/focus/visibility observable tests | Independently PASS; final native quiet/hidden/resume/manual PASS |
| AC8 | Debounced worktree/Git/common-directory watchers, linked-worktree and missed-event fallback | Disposable external-change/watch tests | Full S6 independently PASS; native lost-subscription recovery26.381s |
| AC9 | Regression safety, measured performance baseline, pinned builds, independent final review | Existing checks/native evidence and benchmark | PASS; checks/native production/performance and independent gate3 |

## Plan / ownership

Planner v020_planner completed the seven-packet plan and bounded BQ reassessment without edits or dispatch. Root owns this record and orchestration. Shared contracts/manifests/lockfiles must have one writer before dependent backend/UI work.

## Cost checkpoints

- No monetary/token cap set; usage unknown. Do not claim measured savings.
- Soft allocation: roughly 5–8 cohesive implementation handoffs and three independent review gates; revise if evidence requires.
- Normally one worker; at most two concurrent subagents. Parallel backend/UI only after frozen contracts, disjoint write ownership and recorded wall-time benefit.
- Reuse agents for repairs. Stop/reassess the same approach after two unsuccessful rounds; preserve required checks.

## Dispatch and implementation

| Packet | Agent / role | Requested settings | Status | Evidence |
| --- | --- | --- | --- | --- |
| Plan | v020_planner / planner | Sol/high named role | DONE | Seven-packet plan + bounded BQ reassessment |
| S1 | v020_backend / worker_complex | Sol/high named role | Independently PASS | Contracts/dependencies/version/quiet refresh; frontend61/selectedcore58 |
| BQ | v020_backend / worker_complex | Sol/high named role | Independently PASS / BQ-01 CLOSED | Fullcore71; process7/fault6 independent; 14 owned native clones author |
| S3a | v020_graph / worker_complex | Sol/high named role | Independently PASS / S3A-01 CLOSED | Pure layout/visibility; frontend97, pure36 independently PASS |
| S2 | v020_backend / worker_complex | Sol/high named role | Independently PASS / S2-SH01 CLOSED | Original core/native API plus repaired SHA1/SHA256 shallow raw-DAG matrix |
| S3b | v020_graph / worker_complex | Sol/high named role | Independently PASS / S3B-01 CLOSED | Canvas/virtual rows/sidebar/search components; no App/store integration |
| S5a | v020_diff_ui then v020_graph | Sol/medium then bounded Sol/high escalation | Independently PASS / S5A-01 CLOSED | Standalone readonly local Monaco and actual StrictMode lifecycle |
| S3 integration | v020_graph / worker_complex | Sol/high named role | Independently PASS / NATIVE PASS | Reviewer81/typecheck; author154/build; actual Canvas/filter/search/native10k |
| S4 | v020_backend / worker_complex | Sol/high named role | Independently PASS / NATIVE API PASS | Reviewer8/source17hashes; author63+20 overlap; native diff/view restart |
| S5 integration | v020_diff_ui, then v020_graph repair | Sol/medium, then Sol/high | Full gate2 independently PASS | Author190; reviewer69; native worker/diff/refresh/viewport/fastswitch/reopen/shallow PASS |
| S6a backend | v020_backend / worker_complex | Sol/high named role | Independently PASS / S6A-01 CLOSED | Reviewer13 unit+5 actual; bounded native watcher only, no frontend hook |
| S2-SH01 plan | v020_planner / planner | Sol/high named role | DONE | Bounded captured raw-parent induced DAG normalization |
| S2-SH01 repair | v020_backend / worker_complex | Sol/high named role | Independently CLOSED | SHA1/SHA256 depth2/5/8 and page1/2/200 matrix PASS; bounded raw-DAG normalization |
| S6 frontend | v020_backend / worker_complex | Sol/high named role | Independently PASS / S6-F01,N01 CLOSED | Author210/reviewer80; final inspector/watch/refresh native PASS,13hashes no drift |

## Validation and review

Baseline evidence: 0.1.0 results record frontend52/core64 and native accepted owned fixtures. These are historical, not new 0.2.0 checks. New checks and independent verdicts will be recorded here as work proceeds.

## Handoff

Implementation, owned validation and final independent integrated review are complete. EXT-01 remains an earlier external-provider compatibility limitation. No commit/push/tag or publication was performed.

## Shared dependency research (root, before implementation)

Registry metadata queried with pnpm view on2026-10-24: @tanstack/react-virtual3.14.22 declares React/ReactDOM19 compatibility; monaco-editor0.57.0. Direct lazy Monaco with local bundled worker preferred to an additional wrapper/CDN. notify8.2.0 is documented current release; its native watcher can miss events on some filesystems, so timed fallback remains required. These are proposed pins until actual install/typecheck/build succeeds.

Primary references: https://tanstack.com/virtual/latest/docs/framework/react/react-virtual ; https://github.com/microsoft/monaco-editor/blob/main/samples/browser-esm-vite-react/src/userWorker.ts ; https://docs.rs/notify/latest/notify/ ; https://git-scm.com/docs/git-rev-list . Current docs may expose APIs newer than the selected package; installed typings/build decide actual compatibility. Do not weaken CSP with remote worker/CDN sources.

## Planner handoff / initial execution

Planner completed S1→S2→review1→parallel S3/S4→S5→review2→S6→S7→review3. Full packets recorded in docs/plans/0.2.0.md. v020_backend (worker_complex, named Sol/high) owns S1 and all shared contracts/manifests; planner is completed. v020_reviewer (independent Sol/high) begins a bounded existing process-boundary diagnostic and will be reused at review gates. No product edits by root outside delegated ownership.

Fresh baseline: frontend52 PASS. Core lib20/auth4/clone12/foundation6/operations5 PASS, then existing process_boundary detached-descendant quiescence assertion failed once (WaitForSingleObject0 returned258). Serial exact-test rerun PASS; do not call fullbaseline64PASS. Independent diagnostic pending before runner extension. Remaining baseline inspection/workspace tests were not reached after failure. Root native fixture/benchmark work is independent of this diagnostic; S1 does not change runner.

## S1 author handoff and BQ-01 reassessment

S1 v020_backend READY_FOR_REVIEW: exact versions0.2.0 and dependencies locked; 9 additive history commands/types/wire fixtures/docs; quiet30s coordinator implemented. Author checks PASS: frontend61, typecheck/build/version, core selected58 (lib21/auth4/clone12/foundation6/ops5/inspection4/workspace6), coreclippy/fmt. process_boundary intentionally excluded pending BQ-01; no claim fullsuitePASS. S1 delayed-parallel-inspection test was adapted with root approval to singleflight+pending rerun, preserving separate stale-selection checks. Independent S1 review running v020_reviewer; backend now solely authors BQ packet, not S2.

BQ-01/P1 independent diagnostic CONFIRMED30/30: job ActiveProcesses0 while original preheld child handle is not signaled and child-held exclusive file remains locked. Process identity/job membership held before termination excludes PIDreuse; after waiting originalhandle~1–1.6ms, file accessible30/30. No late user-code write shown, but kernel resource cleanup race proven. Reviewer sourcefinding runner stop_and_wait, regressiontest must hold originalhandle beforetermination. Probe outsideworkspace Temp/opengit-job-review-d0a9045dd3ac4ebcb65b010f4352b1bd/probe.rs SHA2569A26F56993D75107EF71CAA95928AEAD11E9F6370DD94512009AD1A9EAA26999. The earlier serial test PASS does not close finding.

Planner bounded reassessment recommends freshprivatejob IOCP association/collector ready beforeassign/resume, duplicate directchild handle, discover+hold verified samejob descendantprocess identities. IOCP notifications can be lost/recycled: proof requires lifetime TotalProcesses == distinct held verifiedprocess handles, ActiveProcesses0, ALLhandles signaled, no trackergap/error/cap, final accounting stable. One monotonic5s deadline, checkterminationreturn, never credit vanishedPID or add arbitrarysleep. Missing short-livedchild can cause safe uncertainty; normalGit/local/HTTPS/SSH usability mustbe tested. Unknown retains pure job/process/file guards, noBackend/DB/secrets, no publish/cleanup. Collector teardown bounded/no cycles/unboundedjoin. Worker v020_backend ownsrunner/privatejobmodule/process_boundary tests; reviewer independently closes BQ beforeS2runnerextension. Primary MS documentation: TerminateProcess, TerminateJobObject, JOBOBJECT_ASSOCIATE_COMPLETION_PORT (notificationdelivery notguaranteed), JOBOBJECT_BASIC_ACCOUNTING_INFORMATION (lifetimeTotalProcesses).

RootS7 support history fixture authored and proven: fast-import650+5/10000+5 commits, nonmonotonicdates, octopus/shared ancestry, unmergedoutsideSolo andunrelatedroot, annotatedtag+remoteHEAD, binary/Unicode/rename/delete/symlink blobs, linked/shallow helpers. Oracle usesGit rev-list, read-only snapshots preserve refs/status/index. First cleanup hit readonlyGitobject; guarded readonly retry fixed thencomplete fixture+cleanupPASS. Dataset10k creation2.268s; CLI-only page200 samples119–123ms/deepPage120–125ms/search125ms on i5-8400/6logical/~17.0GBRAM Windows11Pro10.0.26300x64. This is NOT application performance. Raw CLI/reference-machine measurements in ignored.tmp JSONs; app baseline pending.

## Dependency-ready parallel adjustment / S3a

Independent S1 review PASS (v020_reviewer): frontend61 independently rerun/version/DTO andrefreshsource verified; BQ remainsopen. Root freezes S1 contracts. To avoid blocking all graph work on the existing runner repair, root splits S3a purelayout/visibility from S3 service/UI integration: S3a depends only on S1 DTOs and an independentGit oracle, fullS3 still waitsS2review1. This is a cohesive independentlyverifiable algorithm packet, not arbitrary line splitting. Max2subagents maintained for backendBQ +graphworker. Rootexportsfixtures/history/layout-oracle.json45actualGit commits+8refs; UIworker owns onlyfournewlayout/visibility source/tests, noApp/store/backend/shared changes.

S3a v020_graph author READY_FOR_REVIEW: incrementalimmutablefrontier/layout and positive-root/decoration/filteractions, fullfrontend84PASS/typecheck/build. AC1/5 purefoundation only, noCanvas/service/nativeclaim. Reviewerreused toinspectpureS3a whilebackendBQfinishestests. BackendBQ sourceauthor milestones: clippyPASS/process_boundary7serialPASS, originalchild+grandchildhandles/filelocks beforetermination/immediateassert afterrunnerreturn, bystanderalive. Faultunitduplicate test stopped aftertwoinvalidhardcoded-count assumptions (threeverifiedmembers); rootapprovedactualidentity/accountingbaseline+duplicate-rootidentity assertion, not weakenedquiescence. Nativeownedtransport usability runninguniqueexe, notyetPASS.

Root authored tests/e2e/history_acceptance.py againstfrozenDTOs foractualTauri backend, currentlysyntaxPASS/NOTRUN pendingS2handlers. Existingdesktop/productionharness versionasserts nowreadVERSION. Nativehistoryscript verifies655nodeDAG parentoracle/topo/pages, positiveSolo/searchoutsideview/deepresult, crosssnapshotcursorrejection/noSourcemutation/refrewriteExternalChange; it doesnot yetclaimCanvas/diffUI coverage.


## BQ and pure graph independent closure

BQ-01 CLOSED by v020_reviewer after direct process_boundary7/fault-lifecycle6 PASS and sourceproof, including held identities/accounting completeness, signals, bounded collector, pureguard retention64/1024caps, abort/callback. Authorfullcore71/clippy/fmt and nativeclone14PASS supplied, not independentlyrerunall. S3A-01/P2 was independentlyfound: promoting terminalparent after unrelatednode reusedsamecolumn andedgecrossedwrongnodecenter. Workerfixed withreservedboundaryLanes, preservingprefix; 9newregressionsfailedbefore thenfrontend97PASS. Reviewerindependentlypure36PASS andCLOSEDfinding; fullCanvas/serviceintegration remainsnotverified. No openrequiredsourcefinding atthischeckpoint; S2nowauthorizedbackendonly.

## Native refresh and harness evidence boundaries

Root native S1/BQ e2e executable refresh_acceptance.py PASS: actual rendered inspection timestamp stays stable >3s (no old2s UI refresh), changes near30s fallback, actual document.visibilityState hidden after minimize for31s shows no UI inspection update, resume converges to externally changedREADME, manual refresh changes timestamp. This is actualUI/source/unit combined evidence, NOT a direct native IPC/process-count measurement. Two instrumentation approaches could not observe calls (JSbridgewrap, ProgramFiles forwardingcounter); stopped/reassessed and removed them ratherthanclaimtraffic counts. Exactrequest fan-out/hidden policy remainscoordinatorunit/source proof; final S6integration rerun stillrequired.

DesktopSession processenvironment nowcanonicalizesWindowscase beforeNonefilter; negativecontrols test case-insensitiveoverride andunset plus existingstartupHTTPcounter controls4PASS. Noappbridge/securitypolicychanged. NewhistorynativeAPIharness remainsNOTRUN untilS2 handlersfreeze. Roottestharness changes require independent reviewatgate3.


## Additional independent UI preparation

While S2 history/query tests run, root starts S5a using worker_standard Sol/medium (v020_diff_ui): only standalone lazy readonly Monaco component/worker-loader/resource lifecycle tests under features/history, depending solely on S1 fixedDTOs and pinnedpackage. NoApp/store/style/contracts/config writes and no backendservice stubs; fullS5 integration still waitsS3/S4. This substantial disjoint UIpreparation overlaps backend criticalwork without contaminating nativeAPI checks or waiting on unimplemented services. Componentunusedbuild doesnotprove emittedworker/CSP/native capability. Reuse thisnormalUIworker laterS5 ratherthanforcingSolhighforroutineinspector. Max2agents backend+UI; reviewerwillrun afteronecompletes.


## S5a lifecycle review and bounded testability escalation

S5a standaloneMonaco authorinitialfocused10/full107/typecheck/build and realVite878-modulelocalworker compilationpassed, but native/CSPnotverified becausecomponentunused. Independent reviewerraised S5A-01/P2: keyremountdoesnotproveactualStrictMode/sharedrefs replay. StandardUIworkerstopped repeateddynamic-modulemock attempts (activeeffect receivedactualhelper withmockedemptyAPI; nosourcebugconfirmed) andrequestedSol/high capabilityescalation. Rootreusedworker_complexv020_graph withnarrowtestownership. Splittingactualhelpertest removedmockmixing; globaldynamicImportSettled timedout twice, requestackstillcouldnotexerciseactiveeditor. RootapprovednormaltypedcreateMonacoDiff(loader) dependencyinjection, samehooks/resources/defaultstaticlocal lazyloader, noarbitraryURL/IPC/testflags. Thisavoidsunstablemodulemocking, not an unprovenproductfix.

ActualStrictModeregression nowconfirms2effectsetups andbothleasecompletionorders, latestprops/noorphan/noactiveeditorprematuredispose/unmountcleanup. Workerfocused12/full109/typecheck/buildPASS. Independentreviewer directly12focused+typecheckPASS, hashmatched; S5A-01 CLOSED andstandaloneS5aPASS. BQ-01/S3A-01remainCLOSED. FullS5stillwaitsbackenddetails/migration+graphintegration/nativeMonaco.

S2 new-capabilitydecision: foundationGit2.31minunchanged; newhistory readsrequireprobed --no-lazy-fetch and --shallow-file capabilities. UnsupportedhistoryreturnsaccurateUnsupportedGit/Inspectionstaticupdateguidance. Allhistorycallsusemodernno-lazy andno-replace flags; capturednonshallow explicitnullshallowfile closesABA; capturedshallow heldNOFOLLOWread/no-write/no-delete leaseforboundedquery, comparedtofingerprint andprotectedbyBQpureguard. Tradeoff: transiently blocks externalshallowreplacementwhilequeryheld, notordinaryref/worktreeupdates; guardreleasedafterprovenstop, retainedonuncertaintywithrestartguidance. Hist-onlyGIT_GRAFT_FILEtrustednullafterinheritedenvclear closeslegacygraftABAwithoutmutatingGit; staticallyunsupportedshallow/graft/promisorstate remainsvalidated. Tests/remainingfreezeongoing.

Service timingauthorbeforefinalshallowguard: actual10005nodebackend open1678ms, deepUnicode/literalfullhistorysearch2132ms withtwoconcurrenttestthreads. Searchuses4096 sizeheaders +rawpacks<=4MiB, notper-commitprocesses; notnativeUIlatencyorfinalthreshold. Newmetadata testcasesPASS17beforelatestguards; corecountsfinalpending.

## S3b preparation and S2 gate1 snapshot

Root approved a second isolated S3 component packet while S2 ran: Canvas/virtual rows/sidebar/search, with no App/store/API edits. This depends only on frozen S1 DTOs and reviewed S3a algorithms. Independent review found S3B-01: delayed anchor/selection intent was consumed before its page loaded. Worker retained pending intents, canceled them on explicit navigation and prevented default anchor saves while awaiting restore. Reviewer reran40 focused checks and both original delayed-target reproductions; finding CLOSED and standalone packet PASS. Full S3 integration still waits S2 gate1. A normal Monaco option delta, ignoreTrimWhitespace:false, was also reviewed; actual whitespace-only native diff remains pending.

S2 captures the full bounded stash stack, including older stash entries, without adding renderer revision expressions. Files and reftable author fixtures passed, including missing older objects and cap. A current stash ref without a reflog is valid and uses the captured ref only; existing-log tail errors fail closed. Logical HEAD handles reftable compatibility stubs. Freeze manifest .tmp/s2-source-freeze-fc5e5bc97e6d41098cca0e3bd9fc19f4.json records owned source hashes; one targeted fatal-tail regression may update its test hash. Independent gate1 is running; author final core/native build evidence follows. No S4 work has started.

## S2 final author and root native evidence

Final source author full core94PASS: lib28/auth4/clone12/foundation6/historyBoundaries14/historyQueries8/operations5/process7/inspection4/workspace6; doc0. fmt/clippy-Dwarnings/diff and frontend137/typecheck/version/build PASS. Frozen manifest .tmp/s2-source-freeze-final.json SHA256D8F80477B27FC3E0B756CCA2AF8BB8E8D0A967BB763402B37E1DF36640B79597, drift0. Debug e2e Tauri build PASS; unique .tmp/s2-native-8b3baa77c413476382010c54317d640a/opengit.exe SHA25604D425881F97BF184EAB9180F007F674AD3E8F7ABFC2D0D61DA82BDB103477C1. Not final production.

Root history_acceptance.py on that actual Windows/WebView2 artifact PASS exit0:655-node full paginated raw-parent DAG/octopus/orphan6.464s; deep and outside-filter search; positive Solo; cross-snapshot cursor rejection; refs/index/status preserved; external-ref invalidation; linked worktree/detached identity; shallow raw-parent boundary. Native service test uses chooser-only injection and does not establish integrated graph/diff UI. Driver artifact .tmp/desktop-cdf195e5dfed4c1a9f9c58461825fb85. Independent reviewer also directly passed bounded stdin, shallow pin/release, nested tags/fullstash Unicode search and original BQ teardown proof; final gate1 verdict pending.

Gate1 v020_reviewer PASS: no requiredfinding, independently4focused tests and15/15sourcehashes, full S2 AC backendcoverage plus author94/rootnative evidence. Root authorized S3 App/store integration (v020_graph, Sol/high) and S4 Rust details/diff/schema3 (v020_backend, Sol/high) in parallel, disjoint ownership and sharedDTOs frozen. This is the planned substantial UI/backend concurrency; max2agents maintained. S5 normalUIworker follows both; gate2 stillrequired beforewatchers. No finalversioncompletion claim.

S4 design decision: stash ordinal keys refs/stash/<index> are snapshot-only, so storing them would retarget a different stash after push/pop. Product sidebar is selection-only for stashes; persisted Hide/Solo requirement targets branches/tags. Root chose explicit InvalidRequest on saving ordinal stash filters (documented), retaining normal branch/tag/current real refs and durable selectedOID. No silent retargeting, extra persistent ordinal mapping, or Git capability preflight merely to read preferences. S2 ephemeral filtering remains unchanged. Worker tests rejection and normal persistence; reviewer gate2 must verify this decision.

S4 preference policy clarification: selected/anchor OIDs are syntactic full40-or64 hex UI hints, not a claim about current object format/existence. Preference reads/writes need no Git subprocess or history capability probe; S5 reconciles hints against actual opened history. Backend owner updates architecture and tests malformed hints/CAS/knownrepo handling. Root authored native S4 API canary/diff/CAS/restart harness and S3 UI/Canvas harness; both syntax-only until corresponding services/screens build, not accepted evidence.

## S3/S4 combined native snapshot

S3 author frontend154/19files+focused53/typecheck/build PASS; one legacy cloneUI test adapted to explicitly open Overview after clone, preserving assertions (root narrow ownership grant). Eight files frozen; independent S3review running. S4 author broad63/delta20 overlapping PASS, fmt/coreclippy/defaultdesktopcheck,17filefreeze drift0 manifestSHA4B1FD2B149A416679E1A54EF3C7FA1CC24A5481CD973524BD4168E49EDAA0964. Four actual S4 handlers; no S6. Rename/copy candidate limit1000 refuses warning/fallback rather than silently mislabeling. No runner/BQ change.

Root combined debug e2e build PASS21.38s, .tmp/s34-native-1c5bc9d90946469cbca52b26a481467a/opengit.exe SHA256A41E9CC5BDE34B927BD8F3F5E49C6B3063ADE772ED058DFE1C93E7D965F07AEA. Native history_details_acceptance.py PASS: root/Unicode/symlink/binary/whitespace/rename/delete/three merge parents and invalidfilehandle; externaldiff/textconv canary notexecuted; refs/index/statusunchanged; viewCAS/stalerejection and exactview/revisionafterapprestart. This advances its own scope and intentionally does not claim full UI acceptance.

Native history_ui_acceptance.py PASS:10,005defaultgraph+two stashes, painted Canvas and bounded17/21DOMrows at two actualviewports, scroll/multiSolo/Hide/deep/outsidefiltersearch and sourcepreservation. Screenshots .tmp/desktop-bb32bfc50a7c44a18747e706751dd6dd/history-1440.png and history-1100.png visually inspected. Debug/warm open-through-firstpaint5.831s/deepsearch-through-render4.046s, no threshold or coldcacheclaim. Canvasbitmap911x459/662x331 matches viewport×DPR1, painted2413/1759pixels. Fullinspector/Monaco/persistenceUI/watch integration and finalproductionstillpending.

S3 independent PASS v020_reviewer: direct81tests/typecheck/scopedwhitespace plus screenshots, nofinding. Root now dispatches S5 normalUI integration to reused worker_standard v020_diff_ui Sol/medium, and independent S4 backend half ofgate2 to v020_reviewer Sol/high. Disjoint frontendwrites/Rustreadonly review, max2agents. S5 updates actual inspector/viewhydration andresponsivefit; shared history generation mustcoordinate get/save preferences because sessions.begin can advance/revokecontext. Fullgate2 still waitsS5freeze/nativeMonaco+UIpersistence. S6 notauthorizedyet.

Additional rootnative workspace_acceptance.py onS3/S4 artifact PASS: workingtreeOverview, baremetadata, pickerCancel, durablereopen/selection, forgetpreservesfiles andcorruptDBstartup preservesoriginalbytes. Screenshot .tmp/desktop-5b7a747b4c424d209cba70794388bde5/workspace.png. Existing harness nowexplicitlychoosesOverview instead ofrelyingonolddefault. RawnativeS3/S4 baseline saved ignored.tmp/0.2.0-native-s34-baseline.json, accurately labeleddebug/warm/pre-S5-S6/no threshold.

## Dependency reassessment after S4 approval

S4 independent PASS: directviews6+positiveexternaldrivercanary1+rawambiguouspath/symlink1;17/17freezehashes, nofindings. Gate2frontend stillpending. Root splits S6a backend watcher service from fullS6UI/coordinator integration: S6a depends only on frozenS1watchDTOs and independentlyreviewedS2/S4 backend lifecycles, so it can overlapS5UI with disjoint Rust/frontend ownership. This substantial independent block is expected to reduce idle walltime without newagentcontext; exactsavings/usage unknown. v020_backend reusedSol/high; v020_diff_ui remainsSol/medium, max2agents. Nofrontendwatchhook or refreshcoordinatorchange beforefullgate2; independentS6abackendreviewstillrequired beforefullS6hook/finalgate3. Coordinate a temporary backendproductionfreeze forrootnativeS5build ifbackendtestscontinue.

## S5 review findings and native observer reassessment

S5 author initially174frontendPASS/typecheck/build/version; independentreview55focusedPASS but neworiginal ABArepro FAIL. RequiredS5-01/P2: value-onlyackretirement dropsnew same-valuedfieldintent (400sentrev2; remote600+collapsetruerev3; local500→400; delayedack2). Stdworkerrepair uses perfieldmonotonicBigInt editIDs, active+oldscope exactrepros, Overviewboundedflushwrapper androotapprovedvs-darktheme. Authorrepair26focused/full177/typecheck/build/diffPASS; independentclosurepending.

S5native checkpoint artifact .tmp/s5-native-50f4748d24394e10b7c19e00698a09bd/opengit.exe SHACCB4200FE6FA3DCB204EFA8E77B1247FE29556BC657716EA6CC7B7F0610F26A9 (pre-CASrepair/theme; backendS6acheckpointunusedUI). FirsttwoactualWorkerprobes timedout because initialadd hasemptyoriginal; pinnedMonaco diffProviderFactoryService.js67returnsfastpath withnoWorker. Aftertwofailuresrootreassessedusinginstalledprimarysource andtestedbothnonemptymerge-parentREADME325→650. LocalactualWorker/messages/CSP/readOnly/inline PASS: URLs2/messages10/errors0, nofakeworkers. Thisispartialruntimeevidence, notfullS5acceptance.

Fullinspectorharness nexttimedout onwhitespacefile; tightenedselecteddetailoracleandisolatedprobe proved whitespacecorrect before refresh. .tmp/probe_s5_whitespace.py thenmanualrefresh reproduciblyresetselectedwhitespace.txt toREADME.txt. RootrequiredS5-N02/P2 candidate: history.open clearsinspector thenselectdefaultsbase/firstfile, alsoinvokedby30sfallback. Screenshot .tmp/desktop-938551975ea843f5b40ac82c45a49c7e/refresh-file-reset.png. Independentconfirmation/repairpending; no timerdisabled/workaroundacceptance. Fullnativeinspector/reopen/fastswitch/viewport stillnotPASS.

S6a actualwatch5/unit9authorPASS afterisolatedWindowsnotifyprobe proved redundantrecursiveparent+childhandlesblockancestorrename despiteShareDelete; minimalrecursiveancestorcover repaired whilelogicalroot/git/commonidentitychecksremain. Fullreviewpending. Backendownedhistoryarchitecture doc wasaccidentallytruncatedbycp1252writefailure; scopedbackupsearchfoundnone. WorkerreconstructsUTF8fromauthoritativesource/contracts/priorapproveddecisions andmustflagfulldocdeltaforindependentreview, notclaimbyteexactrecovery. No user/rootedits wereinthatownednewdoc.

S5 repair review: S5-01 CLOSED independently;42focused and originalABA repro PASS. Overview concurrent transitions/newacks covered. Gate2 remainsCHANGES_REQUIRED for independentlyconfirmed S5-N02/P2 (refresh clearscurrentcomparison anddefaultsparent0/firstfile). Root dispatches reusedv020_graph worker_complex Sol/high for this cross-store/context repair; v020_diff_ui stops after successfulCAS/theme/Overview repair. BackendS6a remains disjointRustowner, max2agents. Requirement: retaincontent/base/file throughrefresh with newserverhandles, latestintentwins, no ambiguousdisplaynameguess, typedunavailable/failure coverage, then independentre-review/native. FullS6UIhook remainsgated.

S6a author final:116 unique corechecks PASS (lib37/auth4/remaining75), unchangedquery8 intentionallyexcluded; watchunit9/actualFS5included. Authhelper now derives native Windows importlib search fromtrustedCargooutputs; originalassertionspreserved. clippy/fmt/defaultdesktopcheck/diff PASS;Cargoprocesses0. Freeze8files .tmp/s6a-source-freeze-final.json SHA256C966FC6EEF5C60A03EDA39A97B1096046B9317F8B5925B98457CA86D32B92110, drift0. Reviewer v020_reviewer receives independent backend packet whilev020_graph repairsS5-N02frontend (disjointownership,2agents). ReviewmustdisposeprovenOSancestorlimit againstAC8, fullrebuiltarchitecture doc, andauthfixturelinkerchange. Rootprovisionalpolicy retainsminimalroots, no drive-widewatchscope, accuratelydocumentsclose-app-beforeancestorstorage-moves; no universalrename or immediateunwatchresourcequiescence claim.

## Shallow integration finding / bounded replan

Native after CSS128 passed both1440/960 graph+inspector viewport checks and saved A's view, then failed opening second shallow repository with rendererInternal. Independent actualGit source confirmed13-node depth5octopus captured order contains shallow child746cb3c... at8 to rawparent782d8ac... at2. Git traversal topo omits cut edges; raw-parent UI contract cannot trust that order. Required S2-SH01/P1 reopens gate1/AC1. Root reuses v020_planner Sol/high for a compact bounded repair plan while backend finishes S6A-01; no implementation/dispatch by planner. Preserve captured set/roots/raw parents/boundaries, bounded induced stable topo sort visible+all_order before first page/cursor; no network/deepen/mutation/renderer relaxation. Actual SHA1/256/depth/page/filter/cursor regressions required. S5-01/N02 remain independently CLOSED; full native fastswitch/reopen still blocked by shallow graph.

Root T1 CSS delta100 to128 minimum graph height fixed min960x640 usability; build11.92s PASS, artifact .tmp/s5-viewport-53b81527698e4e008c4181a517c6e12d/opengit.exe SHA D7536A194523A3F96DB005F57159F93C0ACEB6424241DB8FBFB8D8DAD2AFE5A4. Native screenshots .tmp/desktop-3539eab5a0b94846b71d0f42666b2366/inspector-1440.png and inspector-960.png passed viewport assertions. Same harness found its new SQLite observer context did not close readonly connections; root fixed contextlib.closing. Windows oneoff100 queries+immediate database unlink PASS. This was an owned harness handle leak, not an app database bug; failed fixture cleanup recorded accurately.

S6A-01/P2 activation precheck then switch/forget then begin race independently confirmed; backend repair authorized with registered active identity revalidation under valid ticket start/finalization, no native wait locks. Follow-on rejected-request scope poisoning reproduced; approved narrow Sessions.begin_checked validates before scope mutation. Independent watcher18 checks otherwise PASS; OS ancestor limitation and full reconstructed history doc accepted within AC8. No full S6 UI hook yet.

## Approved S2-SH01 implementation packet

Planner returned stable min-rank Kahn on the raw-parent induced captured set before positions/page/cursor publication, independently for visible/all traversal ranks. Nonshallow fast path unchanged. Capture parent metadata once for all_order via bounded full-OID rev-list --no-walk=unsorted --parents batches; replace cut-node parents from raw cat-file headers. Original positive roots/captured set remain exact, outside-set parent objects are not read.

Approved bounds: nodes100k, final parent occurrences300k including outside-set parents; parent metadata32MiB cumulative (SHA256 estimate26MB), raw cut transfer128MiB cumulative, each batch target4MiB/<=4096OID and each command/input hard8MiB. Root clarified a separate32MiB cumulative size-preflight framing budget; valid two-pass100k records fit~18.6MB. Shared Query30s deadline/cancellation, periodic yield, checked arithmetic, O(N)/O(E) structures; no runner/BQ/DTO changes. Worker v020_backend Sol/high owns query/helper/tests and released narrow mod.rs wiring after S6A-01 independently CLOSED (reviewer13unit+5actual PASS,9hashes). Root owns architecture bounds wording; new S2 review must check exact implementation and budgets. No full S6 UI hook before reopened gate1/fullgate2.

## S6 frontend lifecycle plan (read-only while SH01 repair runs)

Planner inspected current App/HistoryScreen/workspace/history/preferences and returned one App-owned lifecycle connection across tabs. Connect before initialize, start the existing coordinator once, make HistoryScreen presentational, and await inspection plus history/inspector restoration in one coordinator run. Scope comes from coordinated preferences/history, never a new counter. Small watch adapter listens before activation, buffers<=256 until ACK, validates scope/token/BigInt sequence, handles gap/overflow and late ACK cleanup, and keeps degradation latch across token generations to avoid unavailable/rebind loops. Hidden events mark dirty; Overview keeps active repo status/watch lifecycle. No new timer, dependencies or runtime framework.

Implementation remains gated by SH01/reopened gate1 and full gate2. Later backend worker Sol/high owns frontend hookup only; preserve CAS edit-version logic and N02 comparison semantics. Tests cover delayed both phases, one pending rerun, selection fencing, listener failure, preACK events, large sequence, StrictMode/tab transitions, pending preferences, hidden/focus/fallback and native linked/common changes. Planner did not implement/run new checks; usage unknown.

## S6 source freeze and integrated core checkpoint

S6 frontend author205 tests/25files, typecheck/version/build/diff-check PASS. Ten-file freeze manifest .tmp/s6-frontend-source-freeze.json SHA25637F0BA08218B26B0D895BA46099470560B227AA85A73B3E7658632A69B3BE6CE; no Rust/DTO/dependency writes. Root debug e2e Tauri build PASS, unique artifact .tmp/s6-native-3e48b71c651a4befaa390dbca3302312/opengit.exe SHA256D1084AC71607D234D279E3AC5BA844068B190B842D77193284782925F18969E6. Independent S6 review running.

Root integrated cargo test --no-default-features --locked --offline PASS138 tests plus doc0, normal desktop token; log .tmp/0.2.0-integrated-core.log. Includes lib46/auth4/clone12/foundation6/history-boundaries17/details10/queries10/watch5/operations5/process7/inspection4/views6/workspace6. Slow history query group393.67s; no repeated full Rust suite planned for frontend-only repairs.

First S6 native inspector run PASS real Monaco worker/messages/CSP/readonly, merge/inline/binary/whitespace; then outside-filter selection timed out while refresh was running and search results disappeared. Not accepted: worker diagnosis requested; log .tmp/0.2.0-s6-native-inspector.log and owned desktop-bae4bc10cb8a4ea4b211ed0c09809e94 failure evidence.

Independent harness findings S6H-01/02/03 require no guessed generation activation, new-tail scoped event checks, measured periodic timing. Root repaired harness to observe exact real watch ACK through installed SDK callback-map forwarding, unwatch only that token, match fresh event scope/token, and assert20..40s recovery after a known quiet periodic run. SyntaxPASS; independent re-review and native control pending. No fabricated backend response, generated scope or production test seam.

Root S6 native watcher acceptance EXIT0 on D1084AC artifact: real worktree notifications update Overview; external commit updates graph; linked common-directory changes and20ref atomic burst converge. Actual SDK callback-map observer captured real ACK; exact-token unwatch produced no notifications/earlyUI for3s, then recovery satisfied measured20..40s periodic window. Path-free event shape checked. Log .tmp/0.2.0-s6-native-watch.log. Print now includes actual seconds for subsequent final rerun; this run did assert bounds but did not print sample. Independent harness delta disposition pending. Root integrated fmt and clippy all-targets -Dwarnings PASS; .tmp/0.2.0-integrated-clippy.log.

Worker isolated RED reproduction confirms S6 search gap: fresh-scope search call remains0 while inspector restore diff is pending. Authorized bounded parallel fresh-search/inspector replay, latest query wins and retained result actions disabled until fresh handles. Narrow ownership adds HistorySearch.tsx/test. Reviewer source-derived S6-F01/P2 additionally requires selection failure recovery: suspend cleared current A before failed B selection, but error branch never restored A immediately; responsible worker requested focused regression and repair, reviewer retains closure authority.

S6 independent baseline CHANGES_REQUIRED: S6-F01/P2 select failure restoration (source-derived) and S6-N01/P2 native search/refresh overlap. Root earlier F02 alias refers to same S6-N01, not an additional finding. Reviewer73 focused frontend checks PASS; harness S6H-01/02/03 CLOSED after source/AST/SDK forwarding review plus actual native watcher PASS. Delta re-review required.

S6 frozen-artifact native API regression PASS: history655-node raw-parent DAG/pages/filters/search/linked/detached/shallow/source preservation; details/root/rename/merge/binary/Unicode/whitespace/canary and view CAS/stale/SQLite restart. Logs .tmp/0.2.0-s6-history_acceptance.log and -history_details_acceptance.log. Workspace first launch/status/bare/cancel passed, then harness invoked bridge before restart initialization; exact WebDriver error Cannot read properties of undefined (reading invoke). Root added explicit native-bridge readiness on reopen and reruns workspace/clone; this is not an app failure claim. First batch stopped at workspace, so clone had not run.

S6 repair source independently PASS80 focused tests and13/13 hashes; S6-F01 CLOSED. Root repaired native inspector EXIT0 on new unique artifact .tmp/s6-repair-native-81d10140c5984ace8c168f6ea758df83/opengit.exe SHA256240E7C6415531D11E6A313B698C94C34E080689D88BD2B23CE99B3EF1F15EA9C. Full outside-filter/refresh/switch/restart plus shallow source-preservation passed; reviewer verified log/artifact/screenshots and CLOSED S6-N01. S6H findings remain CLOSED; fullS6 verdict waits final watch/refresh sameartifact.

Root no-e2e production Tauri build PASS (locked/offline, release optimized1m26s), src-tauri/target/release/opengit.exe SHA256959063503C4B4FE1E5391EC27145AE1FDA1DA276A1E0743203D071B58E6C5D28; production launch verification pending. Actual Monaco local lazy2.7MB chunk yields expected Vite size warning; no new CDN/CSP weakening. Root harness guards4, syntax4 and diff-check PASS.

Workspace retry PASS all registration/status/bare/cancel/reopen/forget/corruptDB; clone native PASS owned local/privateHTTPS verifiedTLS/privateSSH verifiedhost/password/agent/passphrase/public transports. External provider EXT-01 remains unverified. No application source changes after repair freeze; no commit/push/tag.

## Final owned validation / gate3 handoff

FullS6 independently PASS, S6-F01/N01/H01/H02/H03 all CLOSED. Final repaired artifact inspector/watch/refresh batch EXIT0; observed missed-event recovery26.381s, visible fallback/hidden31s/resume/manual checks PASS. No source drift after13-file repair freeze.

Root production acceptance EXIT0 on no-e2e release artifact959063...C5D28: chooser literal absent, persisted registry, typedIPC, independent panes at1280x820 and960x640 actual WebView. Final UI10005commits/twostashes/virtualCanvas/scroll/multiHideSolo/deep+outside search/source preservation EXIT0; firstpaint6.1352724s and deepsearch3.7715669s, single warm debug sample.13/11 rendered rows, backingCanvas exactviewportxDPR1. Performance run after production build finished, no Rust suite/compiler competing; not coldcache/productionlatency/memory or release threshold.

Root updated README/roadmap/AGENTS product status only (no role/model/workflow policy changes), architecture history consumer description, acceptance matrix/measurements/known limits. Finalreview must read current docs and root harness changes including production bridge readiness. All task/version scope remains0.2.0; no next-version features or repository publication. Runtime token/cost usage remains unknown; no exact savings claimed. Final gate3 requested from independent reviewer with completeAC1–AC9 coverage, source/validation/artifact evidence and known limits.

## Final independent acceptance

v020_reviewer returns PASS final integrated gate3, AC1–AC9 covered, all prior findings CLOSED. Reviewer retained prior coverage, verified36 source hashes without drift, runner/BQ and two executables, four final graph/production screenshots, diff-check, syntax6 harness and guards4/4; independent80 frontend regression tests were run at the final repair round. Root138core/210frontend/native/build/performance evidence reconciled; unchanged long suites not repeated. Known bounds and EXT-01 remain accurately disclosed. Root only updates completion/verdict bookkeeping after this verdict; application source/executables unchanged. Implementation0.2.0 complete, no release/publication or Git commit/tag/push. Runtime usage/cost unknown.
