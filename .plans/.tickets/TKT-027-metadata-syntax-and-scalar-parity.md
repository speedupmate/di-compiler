---
id: TKT-027
title: Metadata syntax and scalar parity
phase: 08-parity-closure
feature: metadata-validity-parity
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-016, TKT-023]
touches:
  - rust/di-compiler/crates/code-generator/src/metadata.rs
  - rust/di-compiler/crates/code-generator/src/area_config.rs
  - rust/di-compiler/crates/validator/src/lib.rs
acceptance:
  - All generated metadata files pass php -n -l
  - No invalid numeric literal output for leading-zero strings
  - Metadata diff count decreases vs archived baseline
test_plan:
  - Lint generated metadata files with php -n -l
  - Run validator against generated/_metadata archive
---

# TKT-027: Metadata syntax and scalar parity

## Scope

Fix metadata serialization correctness issues that currently produce invalid PHP and large metadata diffs.

## Implementation Notes

- Remove unsafe string-to-number coercion for scalar values where Magento output is string.
- Keep output deterministic and var_export-compatible.
- Add/extend validator checks for metadata syntax validity.

## Risks

- Over-correcting scalar typing can create new content diffs; validate against archived baseline.
