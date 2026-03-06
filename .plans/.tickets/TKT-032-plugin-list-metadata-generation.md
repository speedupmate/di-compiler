---
id: TKT-032
title: Plugin-list metadata generation
phase: 08-parity-closure
feature: plugin-list-metadata-generation
owner: Unassigned
status: Done
estimate: L
depends_on: [TKT-027, TKT-030]
touches:
  - rust/di-compiler/crates/code-generator/src/
  - rust/di-compiler/crates/di-resolver/src/
  - rust/di-compiler/crates/cli/src/main.rs
acceptance:
  - All expected plugin-list metadata files are generated
  - Generated plugin-list data structure matches archived metadata format
  - Metadata missing count decreases by plugin-list file set
test_plan:
  - File-set assertion for expected plugin-list filenames
  - Structural assertions on plugin-list arrays
  - Full metadata diff run
---

# TKT-032: Plugin-list metadata generation

## Scope

Generate plugin-list compiled metadata files currently missing from Rust output.

## Implementation Notes

- Implement scope-aware plugin list output names and content format.
- Keep deterministic ordering to avoid unstable diffs.

## Implementation Update (2026-03-06)

- Landed via commit `dde719b`.
- Scope-specific plugin-list metadata files are emitted and included in archive compare workflow.

## Risks

- Plugin inheritance logic is easy to over/under-merge; validate using sample plugin chains.
