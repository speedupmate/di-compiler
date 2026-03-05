---
id: TKT-029
title: Interceptor method and signature parity
phase: 08-parity-closure
feature: interceptor-method-signature-parity
owner: Unassigned
status: Ready
estimate: L
depends_on: [TKT-028]
touches:
  - rust/di-compiler/crates/php-extractor/src/lexer.rs
  - rust/di-compiler/crates/di-resolver/src/interceptor.rs
  - rust/di-compiler/crates/code-generator/src/interceptor.rs
acceptance:
  - Wrapped method sets match Magento compile-time interception behavior on sampled classes
  - Nullable and union signatures preserved where Magento preserves them
  - Interceptor content diffs decrease materially
test_plan:
  - Add lexer tests for nullable/union method params
  - Add interceptor generation tests for magic-method filtering and signature rendering
  - Run full validator
---

# TKT-029: Interceptor method and signature parity

## Scope

Align method filtering and signature generation with Magento interceptor output.

## Implementation Notes

- Implement method skip rules consistent with Magento (`__sleep`, `__wakeup`, `__clone`, `_resetState`, static/final/etc).
- Preserve nullable method parameter type hints.
- Ensure generated wrappers and return behavior match Magento semantics.

## Risks

- Method-set parity may require scanner/config parity from TKT-030.
