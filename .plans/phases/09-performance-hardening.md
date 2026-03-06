# Phase 09: Performance Hardening

## Purpose

Drive hot-path optimizations that materially reduce wall-clock compile time without changing output semantics:

- reflection hot-path reduction (batching, selective reflection, persistent PHP worker)
- DI config discovery caching and repeated-work elimination
- phase-level timing visibility for prioritization

## Gate To Enter

- Phase 06 CLI flow is operational and produces stable output.
- Archive compare reporting exists so parity regressions are visible on every run.

## Gate To Complete

- Warm run on this install is consistently near 5 seconds with `--compare-archive`.
- No regression in archive compare counts (`code/metadata missing|extra|changed`).
- Performance work is documented and linked to local tickets.

## Current Status (2026-03-06)

- Warm benchmark with worker branch:
  - Total: `5.029s`
  - Phase 4: `0.659s`
  - Phase 6: `0.150s`
  - Phase 7: `1.908s`
- Residual parity deltas still tracked in Phase 08:
  - code extra `39`, code changed `32`
  - metadata changed `16`

## Features In This Phase

| Feature | Deps |
|---------|------|
| [34-php-reflection-worker-pool](../.features/34-php-reflection-worker-pool.md) | 21, 23 |

## Tickets In This Phase

TKT-035 through TKT-037, TKT-042
