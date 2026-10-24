# Task records

Use [TEMPLATE.md](TEMPLATE.md) for T2/T3 tasks. T0/T1 work may keep a concise checklist in the conversation unless durable coordination is needed.

- The orchestrator is the only writer of the central task record.
- Planner supplies the plan; workers supply implementation reports; reviewer supplies findings/verdicts.
- Use a stable filename with the full version, such as `001-v0.1.0-repository-foundation.md`. Record current/target version, change class (`major`, `minor`, `patch`, or `none`) and reason in the template; a task does not automatically bump `VERSION`. Release and tag authorization are recorded separately. See [the versioning policy](../versioning.md).
- Record actual checks, not anticipated successes. Redact credentials, private URLs containing secrets and unrelated user data.
- `DONE` means every acceptance criterion and required review gate passed. It does not imply a release, commit, push or deployment.

[000-bootstrap.md](000-bootstrap.md) records this setup task.

[003-application-planning.md](003-application-planning.md) ghi task lập kế hoạch ứng dụng. Kết quả thiết kế nằm ở [kiến trúc](../architecture.md), [kế hoạch tổng thể](../plans/implementation-plan.md) và [packet 0.1.0](../plans/0.1.0.md); đây là tài liệu Planned, chưa phải evidence implementation hoặc release.
