# 002 — Versioning policy

## Intake and plan

- Status: DONE
- Original request: "Nên thêm 1 rules đánh version, nên theo format major.minor.patch, bắt đầu từ bản 0.1.0".
- Tier: T2, a shared project rule. No app implementation, release, tag, commit or push.
- Starting state: bootstrap/config files remain untracked; application manifests do not exist.
- Current / target application version: initial planned target 0.1.0; change class none (establish source version, no released version is being bumped).
- Release/tag authorization: none.
- Planner: reused setup_planner (Sol/high). Plan: one cohesive worker change, targeted checks, independent Sol/high review, repairs only on required findings.
- Root alone owns this record. Worker owns VERSION, docs/versioning.md, AGENTS.md, README.md, docs/roadmap.md, docs/development-workflow.md, docs/tasks/TEMPLATE.md and docs/tasks/README.md.
- Historical task records 000/001 and .codex configuration are outside worker scope.

## Acceptance

| ID | Result | Status / evidence |
| --- | --- | --- |
| AC1 | VERSION contains 0.1.0; version format is major.minor.patch | PASS |
| AC2 | Patch/minor/major and pre-1.0.0 breaking-change rules are explicit | PASS |
| AC3 | Live roadmap/examples use full three-part versions | PASS |
| AC4 | Agents/tasks use the rule; docs-only/per-task edits do not cause automatic bumps; future manifests synchronize from VERSION | PASS |
| AC5 | Planned version is distinct from released state; no tag/release authorization inferred; history/config preserved | PASS |

## Steps and cost checkpoints

1. Worker reads scoped docs and records, writes the policy/source version, normalizes live references and updates task fields.
2. Root checks TOML untouched, exact VERSION, full live version references, UTF-8/whitespace and Markdown links.
3. Independent reviewer checks AC1–AC5; same worker/reviewer handle any findings.

One reused planner, one Luna/medium worker and one reused Sol/high reviewer are planned. No parallel writes are useful for this shared policy. No monetary/token budget is specified; usage is unknown. No Astra/xhigh dispatch is justified.

## Dispatch

| Agent | Role / requested settings | Result |
| --- | --- | --- |
| setup_planner (reused) | planner / Sol high | Plan returned |
| version_worker | worker / Luna medium | Native role dispatch succeeded; eight owned files updated and focused checks passed |
| cost_reviewer (reused) | reviewer / Sol high | Independent PASS; no required findings |

## Validation and review

- Worker checked exact VERSION, policy text, complete live version references, newline/UTF-8 and Markdown links. One missed README example was corrected before handoff.
- Root checked exact VERSION bytes `0.1.0\n`, seven live Markdown documents for whitespace/BOM, absence of incomplete milestone references and 18 local Markdown links.
- Root SHA256 comparison confirms task records 000/001 and .codex/config.toml unchanged.
- Independent review PASS across AC1–AC5. Reviewer read all eight changed files/relevant passages, directly checked VERSION bytes and reviewed root evidence. No required findings; no repair rounds.
- Reviewer suggested current/target/change-class fields in this record; they were already added during review and remain recorded above.
- No app build is applicable to this documentation/setup change.

## Handoff

- Completed: canonical planned version 0.1.0, numbering policy, live roadmap normalization, agent/workflow/task integration.
- Version/config/documentation checks and independent review passed. No release/tag/commit/push or product implementation.
- Actual allocation: one reused planner, one Luna/medium worker, one reused Sol/high reviewer, one review pass; billed usage unknown.
