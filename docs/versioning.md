# Versioning policy

The root [`VERSION`](../VERSION) file is the canonical source for the current planned OpenGit application version. It contains exactly `major.minor.patch` (three non-negative integer components) with no `v` prefix. The initial target is `0.1.0`. A planned version is not evidence that the application or a release exists.

## Choosing a version

- A backward-compatible bug fix increments patch: `0.1.0` → `0.1.1`.
- A backward-compatible feature increments minor and resets patch: `0.1.1` → `0.2.0`.
- Before `1.0.0`, a breaking change increments minor and resets patch (for example, `0.4.2` → `0.5.0`); patch remains for compatible fixes. `1.0.0` is the explicit stable baseline.
- After `1.0.0`, a breaking change increments major and resets minor and patch; a compatible feature increments minor and resets patch; a compatible fix increments patch.

Do not bump for each task or documentation-only edit. Work targeting the same unreleased version shares that target. Choose the next target using the increment rules from the last released version; a planned version alone does not establish that anything was released. For every task, record current and target version, change class (`major`, `minor`, `patch`, or `none`) and reason. Change `VERSION` only when the task explicitly authorizes changing the intended release/target version.

When package manifests or Tauri configuration are introduced, synchronize their application version fields from `VERSION`; do not create scaffolding only to store a version.

A `v`-prefixed Git tag (for example, `v0.1.0`) is created only when release/tag authorization is explicit. This policy does not authorize a release, tag, commit or push, and it does not create released-history entries.
