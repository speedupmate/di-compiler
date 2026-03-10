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

## Current Status (2026-03-10)

- Area metadata semantic drift is now reduced to argument-surface extras only:
  - all areas: `missing=0`, `mismatches=0`
  - all areas: `arguments extra=40`
  - `preferences` and `instanceTypes` are zero-drift in every area
  - current milestone treats `extra=40` as accepted residual (not a blocker)
- Archive compare still reports residual closure work:
  - `code_extra=3`, `metadata_changed=16`
- Ticket closure in this phase:
  - Done recently: TKT-048, TKT-052, TKT-053, TKT-054
  - Remaining open tracks: plugin-list parity (TKT-049/050/051)
  - Deferred backlog: argument extra-surface closure (TKT-055 / Feature 37)

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
| [36-di-merge-order-and-null-surface](../.features/36-di-merge-order-and-null-surface.md) | 13, 09, 35 |
| [37-argument-surface-extra-closure](../.features/37-argument-surface-extra-closure.md) | 36 |

## Ticket Pack

TKT-027 through TKT-041, TKT-043 through TKT-055

## Recommended External Tracker Shape

1. One epic: "Phase 08 — Magento parity closure"
2. One issue per active ticket in `.plans/.tickets/` for this phase (currently TKT-040, TKT-041, TKT-049, TKT-050, TKT-051)
3. Every issue links back to the matching local ticket file
