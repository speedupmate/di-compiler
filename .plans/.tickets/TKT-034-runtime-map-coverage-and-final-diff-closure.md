---
id: TKT-034
title: Runtime-map coverage and final diff closure
phase: 08-parity-closure
feature: runtime-map-generator-coverage
owner: Unassigned
status: In Progress
estimate: L
depends_on: [TKT-031, TKT-032, TKT-033]
touches:
  - rust/di-compiler/crates/code-generator/src/
  - rust/di-compiler/crates/cli/src/main.rs
  - rust/di-compiler/crates/validator/src/lib.rs
acceptance:
  - Remaining missing/extra files tied to runtime generator map are resolved or explicitly gated
  - Final validator run reports zero diffs against archived _code/_metadata
  - Final generated metadata files all pass php -n -l
test_plan:
  - Full compile to temp output
  - Full validator compare against generated/_code and generated/_metadata
  - Metadata lint sweep with php -n -l
---

# TKT-034: Runtime-map coverage and final diff closure

## Scope

Close remaining parity gaps tied to unimplemented generator entity types and drive the final zero-diff pass.

## Implementation Notes

- Map residual diffs to specific generator entity categories.
- Implement missing generators required by this install, or provide explicit compatibility gating if not required.

## Current Snapshot (2026-03-06)

- Runtime-map/file-presence closure is substantially improved; missing counts are currently `0` for code and metadata.
- Final zero-diff closure is still open:
  - code extra `39`, code changed `32`
  - metadata changed `16`

## Risks

- Late-cycle generator additions can introduce regressions in previously matched outputs; rerun full validator after each merge.
