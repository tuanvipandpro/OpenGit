# Development workflow

This is the operational workflow for the roles declared in `.codex/agents/`. For release numbering and task version selection, follow [the versioning policy](versioning.md). `AGENTS.md` is the root instruction entry point. The task record captures evidence; role TOMLs set model defaults. These are Codex configuration and agent instructions, not a separate deterministic scheduler or a CI-enforced approval system.

## 1. Orchestrator triage

Read the request, relevant code/instructions, and working-tree state. Identify acceptance criteria and classify risk using AGENTS.md:

| Tier | Route | Evidence |
| --- | --- | --- |
| T0 | Root implements directly | Focused inspection/check and short result |
| T1 | Root checklist → root or one worker → validation; independent reviewer for behavior | Checklist, relevant checks, review if applicable |
| T2 | Planner → dependency-ready workers → integrated validation → reviewer | Task record, acceptance mapping, worker evidence, review |
| T3 | T2 plus explicit failure/recovery design and stronger workers | T2 evidence plus critical-path/failure-path verification |

Classify risk independently of estimated duration. Unknown approach or interacting changes push a task upward. A docs typo stays T0 even if it mentions authentication. Changes to these orchestration policies are T2 because they affect future work.

Root is already the orchestrator: do not spawn a second orchestrator for routine coordination. A configured `orchestrator` role is available for an explicitly bounded coordination assignment, not a prerequisite for every task.

## 2. Model routing and escalation

| Role | Model | Effort | Selection rule |
| --- | --- | --- | --- |
| orchestrator | gpt-6.1-sol | medium | Default primary-session setting |
| planner | gpt-6.1-sol | high | T2/T3 inspection, requirements, task graph and executable plan |
| worker | gpt-6-luna | medium | Known pattern, narrow scope, mechanical repetition, explicit check |
| worker_standard | gpt-6.1-sol | medium | Normal implementation, debugging and integration |
| worker_complex | gpt-6.1-sol | high | Unclear algorithms, auth, recovery, concurrency and other critical behavior |
| reviewer | gpt-6.1-sol | high | Independent review against original request and acceptance |

Use the least costly sufficiently capable model per subtask. Handle quick tasks directly and use a planner for T2/T3. A critical parent may contain mechanical work for Luna; known critical work normally uses Sol/high. T3 does not automatically require Astra or xhigh. Keep independent high-reasoning review and all planning, failure/recovery and repair gates.

Escalate with evidence for ambiguity, missing contracts, ownership collisions or insufficient capability. Immediate justified escalation is allowed for unresolved serious risk; do not force cheap failures first. Astra/xhigh requires a recorded specific capability gap, hard unknown, or repeated evidenced failure. Stop the same approach after two unsuccessful attempts or repair rounds on the same issue; reassess the plan/model and use a stronger dispatch only when justified. Workers recommend settings; they cannot hot-switch their running model.

If Luna is unavailable, use Sol/medium for mechanical work. If a required Sol or exceptional Astra setting is unavailable, inspect the supported catalog and choose a capable alternative, recording the capability and cost tradeoff. Preserve high reasoning and independence for review; never silently choose a cheaper inadequate critical reviewer. Report a limitation if no suitable replacement exists.

Do not infer model/effort activation from an agent saying "I am model X". Use dispatch settings and runtime metadata when available. User/client/managed settings can override project defaults. Record requested and observed settings separately if they differ or cannot be observed.

## 3. Runtime dispatch

Prefer native named roles when the available tool supports them. A named role’s explicit model/effort may take precedence over spawn overrides. For exceptional escalation, use the explicit-dispatch fallback below without selecting the fixed named role; if unsupported, report the capability limitation. Otherwise:

1. Read the selected `.codex/agents/<role>.toml`.
2. Include its `developer_instructions` and a complete task packet in the spawn prompt.
3. Supply the supported model and reasoning explicitly when the tool allows it.
4. If model overrides require a fresh context, use fresh-context spawning and include all relevant inputs. Do not assume it sees prior chat history.
5. If the tool cannot set model/effort, disclose inheritance and use the available session model only when it satisfies the role; otherwise report the constraint.

The current collaboration surface supports explicit model/effort with `fork_turns="none"`; full-history forks inherit the parent. Tool names and schemas can change, so inspect the current tool rather than inventing parameters. Do not create user-visible Codex chats as a substitute for subagents.

Agents inherit parent permissions. Planner/reviewer "do not edit" instructions define behavior, not a separate read-only sandbox. Never weaken managed permissions to make delegation work.

## 4. Plan contract

For T2/T3, root creates `docs/tasks/<task-id>.md` from `TEMPLATE.md` and sends planner:

- Original request and relevant conversation constraints, not only a paraphrase.
- Version scope and requirement IDs; list explicit non-goals.
- Current repository state, relevant file paths, and known constraints.
- Expected planning output, available worker roles and concurrency limit.
- Existing research and validation evidence, compact relevant context/file paths, and soft allocations for dispatch count, review rounds and stop/reassess checkpoints.

Planner reads relevant files, resolves routine choices, exposes unknowns and returns a dependency graph. Each subtask must include:

| Field | Required content |
| --- | --- |
| ID and objective | One independently verifiable outcome |
| Requirements | Acceptance IDs covered by the subtask |
| Dependencies | Preceding outputs/interfaces needed; no work against invented contracts |
| Inputs | Exact files, symbols, data formats or source references |
| Ownership | Allowed write paths; read-only context and shared-file owner |
| Steps | Ordered, concrete implementation steps; branch/error cases where relevant |
| Worker | Recommended configured role, model and effort |
| Validation | Actual command/check, expected result, necessary fixtures; distinguish proposed future scripts |
| Completion | Observable acceptance criterion and handoff artifacts |

Batch related mechanical changes into cohesive subtasks and reuse existing research rather than repeating it. Keep packets compact without losing required instructions or acceptance evidence. Split until workers can execute without redesigning the feature; do not create microtasks with no independently testable result. Planner returns its plan and does not edit product files or spawn agents. Root records it and coordinates changes. Planning is not an automatic permission question: proceed within the authorized scope.

## 5. Parallel ownership and integration

At most two concurrently open subagents by operational policy, or the lower host limit. Normally run one worker. Two parallel workers require substantial disjoint tasks and a recorded wall-time benefit worth extra context and integration overhead. `.codex/config.toml` preserves a configured ceiling of 10; that ceiling does not replace this lower instruction policy or enforce a cost budget deterministically. Completed agents should be closed/reused when the tool supports it; if idle agents still consume capacity, finish or reuse them before scheduling more.

Only the orchestrator dispatches. No worker/planner/reviewer spawns another agent. Dispatch only dependency-ready work after shared contracts are fixed. Shared manifests, lockfiles, interfaces and task records have one assigned writer. All agents may see the same working tree: this does not authorize edits outside their ownership.

If boundaries overlap, serialize work or use isolated checkouts with a clear integration order. Worktree creation and integration must follow the current environment's instructions. Workers report unexpected changes instead of reverting them. Root reviews the combined change, reconciles interfaces, and runs integration checks before final review.

## 6. Worker delivery

Every worker receives a bounded task packet and returns:

```text
Subtask ID / role / requested model + effort
Status: READY_FOR_REVIEW | NEEDS_REPLAN | BLOCKED
Files changed and behavior delivered
Acceptance IDs: evidence for each
Validation: exact commands, outcomes, failures or checks not run
Deviations / risks / remaining dependencies
Escalation recommendation, if needed
```

Workers follow the plan, select the smallest appropriate implementation, and validate actual behavior. When the plan is wrong, return evidence and request an update instead of silently expanding scope. Do not mark the parent task complete, approve your own work, or modify the central task record owned by root.

## 7. Independent review and repair

The reviewer receives the original request, current plan, acceptance mapping, final files/diff, worker reports and check results. It did not author the implementation. For an unborn Git branch, inspect the new files directly; an empty `git diff` does not mean no changes exist.

Reviewer checks requirement coverage, correctness, regressions, boundary handling and verification quality. For T3, explicitly inspect secret handling, Git integrity, restart/recovery, concurrency and relevant failure paths. Do not require irrelevant checks merely because they appear in a generic checklist.

Return:

```text
Verdict: PASS | CHANGES_REQUIRED | BLOCKED
Reviewed scope / patch snapshot
Acceptance coverage: PASS | FAIL | NOT_VERIFIED for each ID
Required findings: ID, priority, file/location, evidence, impact, fix acceptance
Suggestions: separate nonblocking improvements
Validation run / reviewed evidence / residual limits
```

Priority: P0 immediate severe harm, P1 major correctness/data-integrity issue, P2 material requirement or behavior defect, P3 minor issue. Priority and blocking status are separate: every explicitly required acceptance defect blocks regardless of priority; optional style suggestions do not.

Reuse the worker and reviewer for repairs when their context remains useful. Root routes findings to the owning worker with finding IDs. Worker supplies fixes and evidence. Reviewer checks the repair delta and impacted regressions against updated files while retaining coverage of every acceptance criterion and required finding. Never omit required checks to save cost. Disputed findings need evidence and explicit reviewer resolution. `PASS` is invalid with outstanding required findings or unverified required acceptance.

After two unsuccessful attempts or repair rounds on the same issue, stop that approach and reassess. Root changes approach when justified: stronger worker, planner revision, or clarification only if a real user decision is missing. A timeout, failure or unavailable reviewer never counts as approval.

## 8. State and completion

Task states: `TRIAGED → PLANNED → IMPLEMENTING → IN_REVIEW → DONE`; findings move work to `REWORK` and then back to `IN_REVIEW`. An external blocker uses `BLOCKED` with reason and next action; resume the relevant previous state when resolved. These are task-document states, not Codex goal-tool statuses.

Root owns task-record updates. Capture scope changes, dispatch settings, checks, review rounds and open findings. Do not demand a new user confirmation for routine replanning or repairs already within scope.

Completion requires implemented acceptance, valid integrated checks and an independent PASS when the tier requires it. Report incomplete checks and external blockers honestly instead of declaring success. Final delivery identifies files, outcomes, tests and remaining limitations. Product versions remain Planned until their actual release criteria pass.

## 9. Cost checkpoints and reporting

Task records plan expected dispatch count, review rounds and stop/reassess checkpoints as soft allocations, not invented monetary or token caps. Default budget: none. Honor a later explicit user cap; report unfinished work and the remaining step honestly rather than declaring completion at the cap.

Record runtime usage when exposed, otherwise mark it unknown. Exact dollar costs or savings percentages require a stated billing method, measured usage and current pricing. Subscription quota and API bills are different accounting modes. Context size, reasoning, tool use and caching affect usage; instructions alone do not enforce a hard budget. Do not add unsupported budget configuration keys.

Routing guidance: [OpenAI Models](https://learn.chatgpt.com/docs/models), [Pricing](https://learn.chatgpt.com/docs/pricing), and [Subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents). Recheck current documentation when settings or availability change.
