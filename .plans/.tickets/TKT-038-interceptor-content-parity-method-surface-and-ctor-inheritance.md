---
id: TKT-038
title: Interceptor content parity: method surface and constructor inheritance
phase: 08-parity-closure
feature: code-content-parity-closure
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-029, TKT-034]
touches:
  - rust/di-compiler/crates/di-resolver/src/interceptor.rs
  - rust/di-compiler/crates/cli/src/main.rs
  - rust/di-compiler/crates/code-generator/src/interceptor.rs
acceptance:
  - Interceptor files do not fall back to full inherited method surfaces when plugin method discovery is unresolved
  - Interceptor constructor generation preserves inherited parent constructor signature when target class does not declare one
  - Interceptor changed-file count decreases significantly in `generated/diff/code.changed.txt`
  - No regression in interceptor path/method signature tests
test_plan:
  - Add/adjust unit tests for inherited constructor fallback and unresolved plugin method handling
  - Run full compile with `--compare-archive` and record interceptor changed-file delta
  - Spot-check previous large-diff interceptors for method and constructor parity
---

# TKT-038: Interceptor content parity: method surface and constructor inheritance

## Scope

Close the dominant interceptor content drift patterns by constraining method emission behavior and fixing constructor resolution for inherited constructors.

## Implementation Notes

- Keep plugin-driven method filtering strict; avoid “emit everything” fallback in unresolved plugin-class scenarios.
- Preserve current interceptable-method exclusions (`__construct`, `_resetState`, etc.).
- Add constructor-chain lookup across parent classes for interceptor constructor rendering.

## Implementation Update (2026-03-06)

- Landed via commit `d31d697`.
- Follow-up reflection normalization and timing instrumentation in `a5048e4` and `248c9a9` retained interceptor parity behavior while reducing runtime cost.

## Risks

- Over-constraining method emission may miss valid interception wrappers in edge plugin setups.
- Constructor-chain lookup must remain deterministic and avoid cycles.
