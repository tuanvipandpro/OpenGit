# 003 — OpenGit application planning

## Intake

- Status: DONE
- Date: 2026-10-24, Asia/Saigon.
- Original request: "Ok, bắt đầu planning để xây dựng ứng dụng OpenGit này đi".
- Current / target version: 0.1.0 / 0.1.0; change class none, planning for the same unreleased target.
- Release/tag authorization: none.
- Scope: implementation strategy through 1.0.0, initial architecture/contracts, executable 0.1.0 work packets, dependencies, ownership, acceptance, failure scenarios and cost checkpoints.
- Non-goals: application implementation, scaffolding, dependency/tool installation, private-server access, publishing, version bump, commit or push.
- Triage: T2 planning artifact with T3 design-review concerns. Intake conservatively called this T3 because IPC, credentials, subprocesses, concurrency and recovery are being designed; future implementation packets are individually tiered. Planning validation reviews the design, not unimplemented application behavior.
- Starting state: repository contains only setup/configuration/documentation; all files are untracked and there are no application manifests.
- Root session settings: no in-turn switch requested or claimed. Native named role dispatch is available for this task.

## Acceptance

| ID | Observable planning result | Verification | Status |
| --- | --- | --- | --- |
| AC1 | R1–R6 mapped to ordered three-part releases 0.1.0 through 1.0.0 with exit gates | Requirement and roadmap coverage | PASS |
| AC2 | Architecture names module boundaries, typed IPC/domain models, authority and ownership | Design review | PASS |
| AC3 | 0.1.0 broken into coherent dependency-ready packets with steps, files, role/model, validation and completion | Executability review and DAG check | PASS |
| AC4 | Auth, cancellation, restart, destination safety, common-Git-directory coordination and persistence failures have explicit planned checks | Critical-scenario review | PASS |
| AC5 | Local prerequisites, unresolved decisions and authorized private-fixture dependencies distinguished from verified facts | Evidence/source review | PASS |
| AC6 | Cost-conscious batches/review gates and scope/version discipline retained; no code or release claimed | Inventory and independent review | PASS |

## Plan / ownership

1. Planner inspects accepted requirements and returns architectural decisions, release strategy and detailed 0.1.0 packets. Root gathers bounded local/environment and official technical-source evidence in parallel.
2. One documentation worker materializes the plan into docs/architecture.md, docs/plans/implementation-plan.md and docs/plans/0.1.0.md, then links them from README.md, docs/roadmap.md and docs/tasks/README.md.
3. Root checks links, task ID/dependency consistency, version and unchanged out-of-scope files.
4. Independent reviewer checks AC1–AC6; required findings return to the same documentation worker and reviewer.

Root alone owns this task record. Worker owns only the three planning documents and the named navigation updates. VERSION, AGENTS, .codex and historical task records are outside worker scope.

## Environment and official evidence

Read-only observations; readiness is not a completed implementation check:

| Component | Observation |
| --- | --- |
| Git | 2.56.0.windows.2 |
| Node | 24.19.0 |
| Git Credential Manager | 2.9.1; presence does not prove authentication works |
| OpenSSH | Windows 9.5p2 |
| WebView2 | Registry entry 155.0.4283.62; presence is not a Tauri launch test |
| pnpm | Bundled fallback command found; version and standalone project suitability unverified |
| Rust / Cargo / MSVC | Commands not found in PATH; default user Rustup and conventional vswhere paths absent; installation status remains unconfirmed |

Opened sources on 2026-10-24: [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/), [Tauri capabilities](https://v2.tauri.app/security/capabilities/), [Vite guide](https://vite.dev/guide/), [Git status](https://git-scm.com/docs/git-status), [Git clone](https://git-scm.com/docs/git-clone), [GCM](https://github.com/git-ecosystem/git-credential-manager/blob/main/README.md).

Exact dependency pins and compatibility are an explicit readiness/scaffold task, not invented from documentation alone. No private test fixtures/credentials have been supplied; production provider/SSO coverage cannot be declared without authorized integration evidence.

## Cost checkpoints and dispatch

- Expected current allocation: one planner Sol/high, one documentation worker Sol/medium, one reviewer Sol/high, one initial review pass. Reuse worker/reviewer for findings, reassess after two unsuccessful repairs of the same finding.
- Normally one worker; at most two concurrent subagents under policy. Root research overlaps planner inspection; parallel document writers are unnecessary because the plan has shared contracts.
- Future implementation allocations are proposals in the plan, not dispatched work or fixed monetary/token budgets.
- Usage: unknown, no billing telemetry available. No exact cost/savings claim or budget cap.

| Agent | Role / requested settings | Result |
| --- | --- | --- |
| application_planner | planner / Sol high | Plan returned: 13 coherent 0.1.0 packets, architecture, release epics/gates and failure matrix |
| application_plan_worker | worker_standard / Sol medium | Completed three planning docs and three navigation updates; focused checks passed |
| application_plan_reviewer | reviewer / Sol high | Native-role fresh-context dispatch; independent planning PASS |

## Validation and independent review

- Artifacts: [architecture](../architecture.md), [implementation strategy](../plans/implementation-plan.md), [0.1.0 packets](../plans/0.1.0.md). README, roadmap and task index link them.
- Root clarified the planner handoff before materialization: full explicit file paths, complete dependency edges (including local GUI before GUI-auth proof), no circular dependency on the later common fixture harness, bare-repo read-only behavior, and cancellation commit point around no-replace clone publication. These are proposed implementation decisions requiring contract freeze and implementation evidence later.
- Worker validated six owned documents for links/UTF-8/whitespace, 13 packet fields, exact dependency DAG and seven release gates. A missing Input field in packet 13 was fixed before delivery; all checks then passed.
- Root independently checked all 13 packet headings, required fields and at least three ordered steps per packet; parsed dependencies match the agreed acyclic graph; all seven full-version release gates and R1–R6 are present.
- Root checked 25 files for UTF-8/BOM/whitespace and all 47 local Markdown links. SHA256 verifies VERSION, AGENTS, .codex/config.toml and historical task records 000–002 unchanged.
- No package.json, src, src-tauri or node_modules exists. Product tests and builds are not applicable to this planning-only task. Future commands are explicitly labelled proposals pending scaffold and successful execution.
- Independent verdict: PASS for the planning artifacts from application_plan_reviewer, which did not author or edit the documents. All AC1–AC6 passed; no required findings or optional suggestions; zero review repair rounds.
- Reviewer independently checked actual untracked contents, all 13 packet fields/steps and the exact acyclic dependency graph using PowerShell; checked 15 Markdown files and 50 local links, seven release gates, unchanged version and absence of scaffold. A system-Python attempt failed because python was not on PATH; PowerShell completed its checks. Reviewer accepted root's environment/hash evidence without claiming to rerun those probes.
- Applicable T3 design invariants and corresponding future verification gates were reviewed, including GUI auth, scoped subprocesses, secrets, shared Git directories, no-replace clone publication and restart reconciliation. This does not constitute passing unimplemented software tests.

## Handoff

- Planning complete and independently approved: three planning/design documents, three navigation updates and this task record.
- First implementation packet is OG-010-01 (readiness and contract freeze), followed by OG-010-02 (scaffold) only after build readiness is verified. Live private-auth acceptance remains conditional on authorized fixtures and does not block planning completion.
- Application version 0.1.0 remains Planned. No application code, dependency/tool installs, release/tag/commit/push or version bump.
- Actual coordination: one planner Sol/high, one documentation worker Sol/medium, one independent reviewer Sol/high, one formal review pass. Task usage/billing remains unknown; no quantified cost or savings claimed.
