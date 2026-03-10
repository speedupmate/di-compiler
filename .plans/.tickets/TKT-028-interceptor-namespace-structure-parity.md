---
id: TKT-028
title: Interceptor namespace and structure parity
phase: 08-parity-closure
feature: interceptor-namespace-structure-parity
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-017]
touches:
  - rust/di-compiler/crates/code-generator/src/interceptor.rs
acceptance:
  - Interceptor namespace matches Magento generated class namespace layout
  - Interceptor path-to-FQCN mapping is consistent for all generated interceptors
  - Interceptor diff count decreases vs archived _code baseline
test_plan:
  - Add unit tests for nested namespace target classes
  - Run validator and inspect interceptor diffs
---

# TKT-028: Interceptor namespace and structure parity

## Scope

Fix interceptor namespace/class structure generation mismatches.

## Implementation Notes

- Align namespace derivation with Magento output for `<TargetFQCN>\Interceptor`.
- Verify constructor/class declarations remain compatible with Magento pattern.

## Risks

- Namespace fix may expose downstream signature mismatches; coordinate with TKT-029.
