---
id: TKT-040
title: Extension/factory content parity: ordering and target mapping
phase: 08-parity-closure
feature: code-content-parity-closure
owner: Unassigned
status: Ready
estimate: M
depends_on: [TKT-031, TKT-034]
touches:
  - rust/di-compiler/crates/code-generator/src/extension.rs
  - rust/di-compiler/crates/code-generator/src/factory.rs
  - rust/di-compiler/crates/di-resolver/src/factory.rs
  - rust/di-compiler/crates/cli/src/main.rs
acceptance:
  - Extension classes/methods follow Magento ordering semantics where currently drifting
  - Factory target mapping parity is closed for remaining `*ExtensionInterfaceFactory` and edge target shapes
  - Residual changed files in extension/factory category are removed or explicitly documented in TKT-041
test_plan:
  - Add focused unit tests for extension ordering and factory target mapping
  - Re-run archive compare and confirm extension/factory bucket reduction
  - Spot-check generated samples from previous drift list
---

# TKT-040: Extension/factory content parity: ordering and target mapping

## Scope

Close the remaining non-interceptor, non-proxy content drifts concentrated in extension and factory artifacts.

## Implementation Notes

- Preserve deterministic ordering rules matching Magento output.
- Keep target mapping logic explicit for extension-interface factory patterns.

## Risks

- Overfitting to one install can break generic mapping behavior on other Magento distributions.
