---
id: TKT-039
title: Proxy content parity: defaults and interface resolution
phase: 08-parity-closure
feature: code-content-parity-closure
owner: Unassigned
status: Ready
estimate: M
depends_on: [TKT-034]
touches:
  - rust/di-compiler/crates/php-extractor/src/types.rs
  - rust/di-compiler/crates/php-extractor/src/lexer.rs
  - rust/di-compiler/crates/php-extractor/src/tier2.rs
  - rust/di-compiler/crates/code-generator/src/proxy.rs
  - rust/di-compiler/crates/cli/src/main.rs
acceptance:
  - Proxy method signatures preserve literal defaults (e.g. `false`, `true`, `[]`) where baseline does
  - Interface targets not present in scanned class map are still rendered as `implements` when composer-resolvable
  - Proxy changed-file count drops across default-value and interface-structure buckets
test_plan:
  - Add extractor tests for default literal capture and proxy generator tests for literal rendering
  - Add/adjust proxy generation tests for interface targets resolved from composer fallback
  - Run full archive compare and validate reduced proxy changed set
---

# TKT-039: Proxy content parity: defaults and interface resolution

## Scope

Address the two dominant proxy-content mismatches: default-value coercion and incorrect class/interface inheritance mode when target metadata is incomplete.

## Risks

- Capturing default literals across lexer/tier2 paths can introduce parser edge cases.
- Interface/class fallback logic must avoid misclassifying unresolved classes.
