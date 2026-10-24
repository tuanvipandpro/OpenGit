# 000 — Repository and Codex bootstrap

## Intake

- Status: DONE
- Date: 2026-10-24 (Asia/Saigon)
- Request: prepare README.md, AGENTS.md and .codex before application development. Configure orchestrator/planner/workers/reviewer with proportional model/reasoning choices, quick-task self-execution, stepwise planning, parallel subtasks, independent review and repair/re-review.
- Non-goals: application implementation, dependency installation, commits, remote setup or publication.
- Tier: T2, because shared orchestration/configuration governs subsequent development.
- Initial state: empty folder, no Git repository or ancestor AGENTS.md found.
- Git initialized on main within the authorized setup scope.

## Acceptance

| ID | Requirement | Verification | Status |
| --- | --- | --- | --- |
| AC1 | README reflects accepted product scope, stack, roadmap and unimplemented status | Content review | PASS |
| AC2 | AGENTS defines proportional routing and authorized delegation | Policy review | PASS |
| AC3 | Native TOMLs define all four roles and differentiated worker tiers with valid models/efforts | Parse/config/catalog checks | PASS (static configuration) |
| AC4 | Stepwise plans, ownership, dependency-aware parallelism and model escalation are actionable | Workflow/template review | PASS |
| AC5 | Independent reviewer findings route to workers and require re-review | Workflow and this task's review | PASS |
| AC6 | Activation/trust/runtime limits are accurate; no global config edits or app implementation | File inventory and independent review | PASS |

## Plan and ownership

Planner: setup_planner, requested gpt-6.1-sol/high, fresh context. Returned B1–B6 bootstrap plan.

| ID | Owner | Dependencies | Steps and completion |
| --- | --- | --- | --- |
| B1 | Root | None | Inspect environment/docs; establish AGENTS routing and ownership. |
| B2 | Root acting as documentation worker | B1 | Create README, roadmap, workflow and task templates consistent with user requirements. |
| B3 | setup_worker_config | B1 | Create .codex config, six standalone roles and activation guide; parse TOML. |
| B4 | Root | B2, B3 | Validate TOML, schema/settings, model catalog, file references and consistency. |
| B5 | Independent reviewer | B4 | Review acceptance and concrete files, including untracked files. |
| B6 | Owning worker/root | Findings | Repair, validate affected checks, re-review. |

B2 and B3 have disjoint ownership and may proceed in parallel. Root alone writes this task record.

## Dispatch

| Agent | Role | Requested model / effort | Evidence |
| --- | --- | --- | --- |
| setup_planner | planner | gpt-6.1-sol / high | Fresh-context spawn succeeded; plan returned |
| setup_worker_config | worker_standard | gpt-6.1-sol / high | Fresh-context spawn succeeded; eight files created, local checks passed |
| setup_reviewer | reviewer | gpt-6-astra / xhigh | Fresh-context spawn succeeded; independent PASS returned |

The local model catalog and tool-advertised model list were inspected for Sol, Luna and Astra reasoning support. No claim is made that this already-running parent session changed model when project files were created.

## Validation

- Bundled Python 3.12 `tomllib`: all seven TOML files parsed.
- Targeted configuration check against fetched official `https://developers.openai.com/codex/config-schema.json`: root keys and agents keys are recognized. This was not a full JSON Schema validation.
- All six standalone role files have exactly the required identity/instruction and selected model/effort fields; role names match filenames; all model/effort pairs match the local model catalog.
- Root defaults and unspecified-subagent fallback are consistent with the routing policy.
- Checked 17 non-Git files as UTF-8 without BOM or trailing whitespace; all 15 local Markdown links resolve.
- `git diff --check` passed, but new files are untracked; the separate whitespace scan covered those files.
- `codex --strict-config features list` was attempted and rejected because this CLI subcommand does not support strict-config. It is not recorded as a successful runtime config check.
- CLI version observed: `0.163.0-alpha.11.1`. Live native named-role discovery/new-session model selection remains untested; docs explain activation and the explicit-dispatch fallback.
- No application build/test run: there is no application scaffold in this task.

## Independent review

Verdict: PASS, returned by setup_reviewer, an agent that did not author any bootstrap file. Review covered the 17-file bootstrap snapshot before this final task-record update.

- AC1–AC6 passed; no required findings and no optional suggestions.
- Reviewer independently parsed all seven TOMLs, checked six model/effort pairs against the local catalog, checked all files for UTF-8/BOM/whitespace and resolved 15 Markdown links.
- Reviewer ran `git diff --check` plus separate checks for untracked content, and verified ignore behavior for agent config, `.env`, `.env.example`, dependencies and build output.
- Reviewer inspected routing scenarios: typo (T0), UI behavior (T1 plus review), multiple components (T2), critical mutation (T3), overlapping ownership, two unsuccessful repair rounds, and unavailable model fallback.
- Runtime native role discovery and new-session selection were not exercised, as documented. Static configuration checks and explicit model dispatch during this task must not be described as native auto-loading verification.
- Repair rounds: none needed; there were no required findings to route back to a worker.

## Handoff

- Repository setup complete: README, AGENTS, .codex roles/config/guide, workflow, roadmap, task template and record, ignore/line-ending rules.
- Git initialized on main. No commit, remote, push, global Codex edits or app implementation.
- Validation and independent review passed within the bootstrap scope. Activate project configuration in a new/reloaded trusted Codex session; explicit runtime dispatch remains the documented fallback.
- Application versions remain Planned. Next product implementation requires a new user task specifying scope/version.
