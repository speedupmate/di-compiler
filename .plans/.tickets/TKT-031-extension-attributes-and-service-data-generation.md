---
id: TKT-031
title: Extension attributes and service data generation
phase: 08-parity-closure
feature: extension-attributes-generation
owner: Unassigned
status: Done
estimate: L
depends_on: [TKT-030]
touches:
  - rust/di-compiler/crates/code-generator/src/lib.rs
  - rust/di-compiler/crates/code-generator/src/
  - rust/di-compiler/crates/cli/src/main.rs
acceptance:
  - Missing Extension and ExtensionInterface artifacts are generated where required
  - Related factory outputs for extension attributes align with archived baseline
  - Missing file count for extension-attribute classes trends to near zero
test_plan:
  - Add focused generator tests for ExtensionInterface/Extension/Factory naming
  - Validate against representative Magento API/Data interfaces
  - Run full validator
---

# TKT-031: Extension attributes and service data generation

## Scope

Implement missing extension-attribute generation paths currently responsible for a large share of missing files.

## Implementation Notes

- Cover both `extension_attributes.xml`-driven and interface method pattern-driven generation.
- Keep generated naming/layout consistent with Magento output conventions.

## Risks

- Partial implementation can reduce misses but increase content diffs; gate with validator.
