---
id: TKT-030
title: Scanner parity for XML and PHP triggers
phase: 08-parity-closure
feature: scanner-parity-xml-php
owner: Unassigned
status: Ready
estimate: L
depends_on: [TKT-014]
touches:
  - rust/di-compiler/crates/di-resolver/src/factory.rs
  - rust/di-compiler/crates/di-resolver/src/proxy.rs
  - rust/di-compiler/crates/php-extractor/src/extractor.rs
  - rust/di-compiler/crates/cli/src/main.rs
acceptance:
  - Proxy/factory candidate sets reflect Magento XML/PHP scanner trigger coverage
  - Over-generation and under-generation from scanner mismatch trends downward
  - Candidate logic covered by fixtures/tests
test_plan:
  - Add XML trigger fixtures (preference, virtualType, argument/item cases)
  - Add PHP scanner fixtures for class generation trigger cases
  - Run validator and compare missing/extra counts
---

# TKT-030: Scanner parity for XML and PHP triggers

## Scope

Close gaps between Rust candidate detection logic and Magento scanner trigger behavior.

## Implementation Notes

- Port missing XML-triggered detection paths for proxies and factories.
- Add scanner-level validation/skip checks equivalent to Magento.

## Risks

- Expanded scanner triggers can increase extra file generation if skip rules are incomplete.
