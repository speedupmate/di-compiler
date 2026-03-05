# Phase 08: Parity Closure

## Purpose

Close the verified parity gaps between Rust output and archived Magento ground truth:

- `generated/_code/`
- `generated/_metadata/`

This phase is correctness-only. No performance-only changes are accepted unless they are required for parity.

## Gate To Enter

- Phase 05 validator harness operational
- Baseline mismatch metrics captured and committed to planning docs

## Gate To Complete

- Zero code diffs vs `generated/_code/`
- Zero metadata diffs vs `generated/_metadata/`
- All generated metadata files pass `php -n -l`
- No extra generated files outside Magento ground truth

## Features In This Phase

| Feature | Deps |
|---------|------|
| [25-metadata-validity-parity](../.features/25-metadata-validity-parity.md) | 19, 20 |
| [26-interceptor-namespace-structure-parity](../.features/26-interceptor-namespace-structure-parity.md) | 14, 20 |
| [27-interceptor-method-signature-parity](../.features/27-interceptor-method-signature-parity.md) | 14, 20 |
| [28-scanner-parity-xml-php](../.features/28-scanner-parity-xml-php.md) | 07, 10, 11, 12 |
| [29-extension-attributes-generation](../.features/29-extension-attributes-generation.md) | 28 |
| [30-plugin-list-metadata-generation](../.features/30-plugin-list-metadata-generation.md) | 18, 19 |
| [31-app-action-list-and-output-layout](../.features/31-app-action-list-and-output-layout.md) | 18, 21 |
| [32-runtime-map-generator-coverage](../.features/32-runtime-map-generator-coverage.md) | 28, 29 |

## Ticket Pack

TKT-027 through TKT-034

## Recommended External Tracker Shape

1. One epic: "Phase 08 — Magento parity closure"
2. One issue per ticket in `.plans/.tickets/TKT-027...034`
3. Every issue links back to the matching local ticket file
