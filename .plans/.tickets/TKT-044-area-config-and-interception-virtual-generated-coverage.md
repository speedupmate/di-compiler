---
id: TKT-044
title: Area-config and interception metadata coverage for virtual/generated types
phase: 08-parity-closure
feature: runtime-map-generator-coverage
owner: Unassigned
status: Ready
estimate: L
depends_on: [TKT-043]
touches:
  - rust/di-compiler/crates/di-resolver/src/arguments.rs
  - rust/di-compiler/crates/cli/src/main.rs
  - rust/di-compiler/crates/code-generator/src/metadata.rs
acceptance:
  - `global.php` and area `*.php` arguments include virtual/generated type keys expected by baseline
  - `interception.php` includes virtual type keys and generated interceptor entries required by baseline
  - Missing-key buckets identified in current analysis are materially reduced and tracked in compare reports
test_plan:
  - Compare per-scope section counts (`arguments`, `preferences`, `instanceTypes`) against baseline
  - Compare interception key-set (`missing/extra`) before/after and categorize residuals
  - Run full compile with `--compare-archive` and store updated `generated/diff` artifacts
---

# TKT-044: Area-config and interception metadata coverage for virtual/generated types

## Scope

Implement metadata completeness for the shared universe defined in TKT-043:

1. Resolve constructor arguments for virtual/generated types (not just real extracted classes).
2. Include required virtual/generated keys in interception registry output.

## Implementation Notes

- Keep key normalization aligned with Magento compiled metadata key shapes.
- Prefer deterministic iteration order to avoid unstable changed-file output.

## Risks

- Incorrect argument defaults on generated keys can create semantic regressions despite matching key presence.
- Adding generated types to interception map without proper intercepted flags may distort plugin execution metadata.
