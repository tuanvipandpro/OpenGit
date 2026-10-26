# OpenGit agent instructions

## Scope and entry point

- OpenGit is a Windows-first Git desktop client with0.3.0 implementation accepted at Gate3, built from accepted0.2.0 checkpoint ce5da58. Read `README.md` and `docs/roadmap.md` before product work. The0.3.0 plan/state/evidence live in docs/plans/0.3.0.md and docs/tasks/007-implement-0.3.0.md; do not confuse earlier0.2.0 checks with new0.3.0 acceptance. Save/Apply is accepted; Pop/Drop is deferred to0.4.0. No later version is authorized by completion. Earlier EXT-01 remains in0.1.0 evidence; implementation does not imply release completion.
- The primary agent is the orchestrator unless explicitly assigned another role. This file authorizes subagent delegation for the workflow below; do not ask again to use planner, workers, or reviewer for an authorized task.
- Follow system/developer instructions and the user's current request before repository guidance. Preserve unrelated user changes. Communicate progress and results in Vietnamese unless requested otherwise; identifiers and technical instructions may be English.
- Deliver only the requested task/version. Do not interpret the roadmap as permission to implement every release.
- Follow the [versioning policy](docs/versioning.md): `VERSION` is the canonical planned target, and ordinary tasks or documentation-only edits do not bump it.

## Triage before editing

Inspect the relevant files and working tree. State the task tier and reason briefly, then act. Use risk and uncertainty, not just file count or estimated time.

| Tier | Criteria | Workflow |
| --- | --- | --- |
| T0 — quick | Obvious, reversible, isolated edit; no behavioral or policy risk | Orchestrator implements and performs a focused check. No delegation overhead. |
| T1 — bounded | Clear acceptance; one cohesive behavior; known pattern | Orchestrator writes a short checklist, implements or delegates one worker, then validates. Independent reviewer for behavior changes; simple nonbehavioral changes may be self-reviewed. |
| T2 — complex | Multiple components, unclear approach, useful parallelism, or agent workflow/configuration changes | Planner → one or more workers → independent reviewer → repairs → re-review. |
| T3 — critical | Credentials, subprocess/IPC boundaries, Git mutations/recovery, cross-repo transfer, concurrency, graph ancestry correctness, or persistent-data migration | Full T2 workflow with explicit failure scenarios, stronger worker selection, and independent high-reasoning review. |

- Escalate immediately when risk or scope exceeds the current tier. Do not downgrade a critical change because its patch is short.
- Treat documentation describing a critical subsystem according to the actual change risk; mentioning Git does not automatically make a typo fix T3.
- Use `docs/development-workflow.md` for dispatch, model routing, ownership, and review rules.

## Role and model routing

| Role | Model | Reasoning | Use |
| --- | --- | --- | --- |
| orchestrator | `gpt-6.1-sol` | `medium` | Triage, direct small tasks, dispatch, integration, final acceptance |
| planner | `gpt-6.1-sol` | `high` | T2/T3 requirement mapping and executable subtask plan |
| worker | `gpt-6-luna` | `medium` | Mechanical/repeatable subtasks with explicit patterns and checks |
| worker_standard | `gpt-6.1-sol` | `medium` | Normal feature implementation and debugging |
| worker_complex | `gpt-6.1-sol` | `high` | Ambiguous algorithms, critical operations, difficult failures |
| reviewer | `gpt-6.1-sol` | `high` | Independent requirement, correctness, regression, and evidence review |

- The role TOMLs in `.codex/agents/` are the machine-readable model defaults. Keep this table and workflow docs synchronized when changing them.
- The orchestrator selects the worker tier at dispatch. Workers reassess their fit and request a specific escalation with evidence; they cannot change their own running model or claim to have done so.
- Use the least costly sufficiently capable model; handle quick tasks directly. T3 does not automatically require Astra or xhigh. Sol/high covers known critical work; escalate immediately for unresolved serious risk when justified. Astra/xhigh requires a recorded capability gap, hard unknown, or repeated evidenced failure.
- Use the configured named role if supported. Its explicit model/effort may take precedence over dispatch overrides. For exceptional escalation, use the documented explicit-dispatch fallback without selecting a fixed named role, or report the runtime limitation. If tools lack a role selector, read that role's TOML, include its instructions and a self-contained task packet, and pass the supported model/effort explicitly.
- With tools that require a fresh context for model overrides, use a fresh-context spawn; do not pass an override with a full-history fork.
- Root model settings are defaults for new sessions, not an in-turn switch. Record runtime overrides or unavailable settings honestly. Never invent a successful model switch.
- If a model is unavailable, choose a supported alternative using the fallback rules in `docs/development-workflow.md`; preserve the independent review gate.

## Planning and parallel work

- For T2/T3, maintain one task record using `docs/tasks/TEMPLATE.md`; record requirement IDs, acceptance criteria, dependencies, ownership, model choices, evidence, and review outcomes.
- Planner inspects the code, identifies unknowns, and breaks work into the smallest independently verifiable subtasks. Each needs ordered steps, inputs, outputs, owned files, dependencies, validation, and a completion criterion. Avoid splitting a coherent change into arbitrary line edits.
- Keep handoff packets compact, share existing research/evidence and relevant file paths, and batch related mechanical work into cohesive subtasks. Reuse workers/reviewers for repairs while their context remains useful.
- Plan expected dispatches, review rounds, and stop/reassess checkpoints as soft allocations. No budget is set by default; honor a later explicit cap and never mark unfinished work complete. Record runtime usage when available, otherwise unknown. Do not claim exact cost/savings without measured usage, a billing method, and current pricing; subscription quota is not an API bill.
- Planner returns a plan to the orchestrator; it does not implement or dispatch workers. The orchestrator records/updates the task record and routes every handoff.
- Only dispatch dependency-ready subtasks. Freeze shared interfaces first. Workers may run in parallel only with disjoint write ownership or separate checkouts plus a defined integration order.
- At most two concurrent subagents by operational policy, subject to a lower host limit; normally use one worker. Use two parallel workers only for substantial disjoint tasks with a documented wall-time benefit worth context and integration overhead. The configured ceiling of 10 is distinct from this instruction policy; it is not deterministic cost enforcement. Only the orchestrator spawns agents; no recursive worker/reviewer delegation.
- Shared contracts, dependency manifests, lockfiles, and task records must each have one writer. Workers request changes outside their ownership instead of overwriting another worker's work.
- Do not commit, push, publish, reset, or clean the repository unless the user's scope authorizes it. Stop only the conflicting work if unexpected edits appear; continue independent work.

## Review and completion

- Reviewer must be a separate agent that did not author the patch. It reads the original request, plan, acceptance criteria, final diff/files, and validation evidence.
- Return `PASS`, `CHANGES_REQUIRED`, or `BLOCKED`, with requirement coverage and concrete findings. Each required finding has an ID, severity, location, impact, and a verification condition.
- Orchestrator sends required findings to the responsible worker. Worker fixes them and supplies evidence; the reviewer checks the changed result again. Worker self-approval never closes a finding.
- Suggestions are nonblocking unless they affect requirements/correctness. Disputed findings require evidence and reviewer disposition, not silent dismissal.
- On repair, review the delta and impacted regressions while retaining full acceptance coverage; never omit required checks to save cost.
- After two unsuccessful attempts or repair rounds on the same issue, stop that approach and reassess the plan/model; escalate with evidence when justified. Do not loop blindly or mark incomplete work complete.
- Validate the integrated result after parallel work. Completion requires all acceptance criteria met, no unresolved required findings, and checks passing (or an explicit, accurately reported limitation; required blocked checks mean not complete).
- Final handoff states changes, validation, outstanding limits, and relevant file links. Never claim tests or a review ran when they did not.

## Product implementation constraints

- Current stack: Tauri 2, React/TypeScript/Vite, Rust/Tokio, Git CLI, SQLite, Canvas graph, TanStack Virtual, lazy readonly Monaco with local workers and notify watchers. Exact current dependency versions are pinned in manifests and lockfiles; verify compatibility before changing them.
- Git is authoritative for repository/operation state; the app database stores UI state, cache, and task context. Detect externally started operations and changes.
- Run Git via argument arrays, never shell-interpolated user input. Validate typed IPC requests, refs, paths, and repository scope.
- Reuse Git credential helpers/OpenSSH. Never put credentials in logs, task documents, fixtures, or SQLite. Do not weaken certificate or SSH host verification to fix an auth error.
- Serialize conflicting writes, including worktrees sharing a Git directory. Plan failure recovery before exposing a mutating action.
- Hide/Solo changes visibility only. Preserve correct ancestry, pagination, selection, and per-repo filter state.
- Test mutations and failure recovery on disposable repositories; do not use the developer's working repository as a destructive fixture.
- Use focused checks for low-impact edits. For behavior changes, test observable outcomes and failure paths rather than reproducing the implementation in a test.
- Never invent build/test commands. Read the current manifests, scripts and README; application checks now exist. Distinguish author checks from native acceptance, independent review and release evidence.
