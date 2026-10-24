# 001 — Cost-aware agent routing

## Intake

- Status: DONE
- Original request: "Cần phải cân nhắc về cost nữa" after the repository/agent bootstrap.
- Tier: T2, changes to agent routing and workflow policy.
- Scope: cost-aware defaults, dispatch/context/review discipline, consistent live docs/config. No product code, release, commit, global config or paid service changes.
- Starting state: bootstrap files remain untracked. Current config has `max_concurrent_threads_per_session = 10`, differing from the earlier bootstrap's four; preserve the currently configured ceiling and document a lower operational concurrency policy.
- Root session was not switched. New task dispatch settings are recorded below; exact billed task usage is unavailable.

## Acceptance

| ID | Result | Verification | Status |
| --- | --- | --- | --- |
| AC1 | Economical model defaults synchronized across six roles and live docs | TOML/catalog/table checks | PASS |
| AC2 | High-reasoning independent review and T2/T3 planning/repair gates preserved | Workflow review | PASS |
| AC3 | Default one worker; at most two concurrent subagents by policy; current configured ceiling 10 preserved and clearly distinguished | Config/instruction review | PASS |
| AC4 | Scoped context, cohesive batches, reuse, delta review and evidence-based escalation reduce redundant work | Prompt/workflow review | PASS |
| AC5 | Cost checkpoints and observable usage recorded without invented prices, savings or hard budget enforcement | Template/review | PASS |
| AC6 | Prior bootstrap record/product roadmap preserved; no unsupported config fields or new roles | Inventory and independent review | PASS |

## Plan

Planner: existing setup_planner, gpt-6.1-sol/high, reused rather than spawning another planner. Returned C1–C5. Root adapted the concurrency proposal to preserve the currently configured ceiling of 10; operational policy is at most two concurrent subagents and normally one worker, subject to host limits.

| ID | Owner | Dependencies | Steps / completion |
| --- | --- | --- | --- |
| C1 | Root | None | Record acceptance, starting state and cost checkpoints here. |
| C2 | cost_worker (worker_standard) | C1 | Update README/AGENTS/workflow/template and .codex config, guide, six prompts. Apply agreed matrix, cost rules, concurrency distinction; parse TOML. |
| C3 | Root | C2 | Check all TOML/model pairs, role counts, document links, whitespace and policy consistency. |
| C4 | Independent cost_reviewer | C3 | Review all changed files against AC1–AC6 using Sol/high. |
| C5 | Same worker/reviewer | Required findings | Repair by finding ID and review affected changes; reassess after two unsuccessful rounds on same defect. |

Only one worker is needed: changes share model/routing contracts and cannot benefit from parallel writes. Root alone owns this task record; worker owns the named live docs/config. Historical `000-bootstrap.md`, roadmap, ignore rules and product files are outside worker scope.

## Cost checkpoints

- Initial allocation: one reused planner, one worker Sol/medium, one independent reviewer Sol/high, one initial review pass. Repair rounds only if required findings exist; these are planning checkpoints, not hard monetary/token budgets.
- No Astra or xhigh dispatch planned; escalation requires a recorded capability/risk justification.
- Default pairs: orchestrator Sol/medium, planner Sol/high, worker Luna/medium, worker_standard Sol/medium, worker_complex Sol/high, reviewer Sol/high. Generic subagent default Sol/medium.
- Current-session metadata does not supply per-task billed usage: unknown. No savings percentage or currency amount claimed; subscription allowance and API billing are different accounting modes.
- No user-specified monetary or token limit. Do not fabricate one or call a goal-budget tool.

## Dispatch / implementation

| Agent | Requested model / effort | Status / evidence |
| --- | --- | --- |
| setup_planner (reused) | gpt-6.1-sol / high | Plan returned |
| cost_worker | gpt-6.1-sol / medium | Completed C2; 12 owned files updated; local validation passed |
| cost_reviewer | gpt-6.1-sol / high | Fresh-context dispatch; independent review PASS |

## Validation and independent review

- Root parsed all seven TOMLs; exact six-role model/effort matrix matches the supported local catalog.
- Config retains ceiling 10; generic default is Sol/medium; no role or configuration keys added.
- Checked 18 files for UTF-8/BOM/trailing whitespace and resolved all 15 local Markdown links.
- SHA256 comparison confirms historical bootstrap record, product roadmap, .gitignore and .gitattributes unchanged.
- Worker independently parsed TOML and checked routing text; `git diff --check` passed with its untracked-content limitation noted.
- Native role discovery, in-session model hot-reload and per-task billed usage are not claimed by static checks. No application exists to build/test.
- Independent verdict: PASS from cost_reviewer, which did not author any changes. Reviewed the 12 changed live files and this task record before the final acceptance update.
- Reviewer independently parsed all seven TOMLs and confirmed all six role pairs, configured ceiling and generic default. All AC1–AC6 passed; no required findings or suggestions.
- Actual coordination matched allocation: one reused planner, one worker, one independent reviewer, one review pass, zero repair rounds and no Astra/xhigh dispatch. This count is not a monetary cost estimate; billed usage remains unknown.

## Handoff

- Cost-aware routing implemented and independently approved. Twelve live configuration/documentation files updated, plus this new task record.
- Reviewer retains high reasoning. Stronger models remain available only through justified escalation; quality gates remain intact.
- Configured ceiling 10 preserved; lower concurrency policy is explicitly instruction-based, not a runtime budget guarantee.
- Activate in a new/reloaded trusted session. No live-role discovery, quantified savings, hard budget or running-session model change is claimed.
- No app implementation, global config changes, commit or push.
