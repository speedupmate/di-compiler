---
id: TKT-046
title: Proxy method surface, order, and class-shape parity
phase: 08-parity-closure
feature: code-content-parity-closure
owner: Unassigned
status: Ready
estimate: M
depends_on: [TKT-039, TKT-042]
touches:
  - rust/di-compiler/crates/cli/src/main.rs
  - rust/di-compiler/crates/code-generator/src/proxy.rs
  - rust/di-compiler/crates/php-extractor/src/types.rs
acceptance:
  - Remaining proxy changed files are resolved for method ordering and method-set parity
  - Interface/class declaration parity is correct for unresolved targets (e.g. interface proxies)
  - Return type rendering parity is correct for `static`/self-like signatures in proxy methods
test_plan:
  - Add proxy snapshot tests covering ordering, class declaration (`extends` vs `implements`), and return-type rendering
  - Re-run archive compare and verify proxy changed-file bucket reduction/closure
  - Spot-check known outliers (HTTP/WebAPI proxies, logger interface proxy, message queue command proxies)
---

# TKT-046: Proxy method surface, order, and class-shape parity

## Scope

Close remaining proxy content diffs that are not covered by previous default/interface tickets:

- method ordering parity with Magento reflection output
- missing/extra proxied method wrappers on selected classes
- class declaration shape and return-type fidelity

## Implementation Notes

- Leverage reflection-worker method lists for ordering/surface canonicalization where needed.
- Preserve performance wins while tightening parity behavior.

## Risks

- Reflection-driven ordering can hide extractor bugs if used indiscriminately; keep explicit fallback rules.
- Class-shape fixes for one target category can regress another if resolution precedence is unclear.
