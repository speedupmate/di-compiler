---
id: TKT-055
title: Deferred closure of remaining arguments extra=40 surface
phase: 08-parity-closure
feature: 37-argument-surface-extra-closure
owner: Unassigned
status: Planned
estimate: M
depends_on: [TKT-052]
touches:
  - rust/di-compiler/crates/cli/src/main.rs
  - rust/di-compiler/crates/di-resolver/src/arguments.rs
  - rust/di-compiler/crates/code-generator/src/area_config.rs
acceptance:
  - `compare_metadata_parity.php` keeps missing=0 and mismatches=0 across all areas
  - arguments extra count drops from 40 or is reduced to a documented residual set
  - no regression in preferences/instanceTypes parity
test_plan:
  - run compare_metadata_parity.php before/after and capture delta
  - inspect comparable metadata reports for changed argument key-surface
  - run full compile with --compare-archive and record metadata_changed trend
---

# TKT-055: Deferred closure of arguments extra=40 surface

## Scope

Track the remaining argument key-surface extras as a deferred correctness item.
Current milestone treats this as non-blocking because all missing and mismatch
counts are zero.

## Current Decision

Do not execute immediately. Keep this ticket queued for a later parity wave.

## Implementation Notes

- Focus on shared key-universe logic, not area-specific merge behavior.
- Preserve current zero-drift behavior for `preferences` and `instanceTypes`.
- Avoid introducing new missing keys while reducing extras.

## Risks

- Over-constraining argument type-universe can reintroduce missing paths.
- Narrow fixes should be validated in all seven areas before landing.
