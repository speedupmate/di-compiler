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

## Current Status (2026-03-10)

- Recent regression fix: Phase 5 is back under 1s after replacing linear
  case-insensitive lookups with indexed lookups in DI config access paths.
- Latest full run with `--compare-archive`:
  - Total: `23.885s`
  - Phase 5: `0.672s`
  - Phase 7: `4.142s`
  - Archive compare: `16.121s`
- Residual parity closure remains tracked in Phase 08:
  - `code_extra=3`
  - `metadata_changed=16`

## Features In This Phase

| Feature | Deps |
|---------|------|
| [34-php-reflection-worker-pool](../.features/34-php-reflection-worker-pool.md) | 21, 23 |
| [38-low-level-performance-hardening](../.features/38-low-level-performance-hardening.md) | 34, 23, 24 |

## Tickets In This Phase

TKT-035 through TKT-037, TKT-042, TKT-056 through TKT-061

## Round 2 Status (2026-03-10)

TKT-056 through TKT-060 landed in one batch:

| Ticket | Change | Status |
|--------|--------|--------|
| TKT-056 | FxHashMap/FxHashSet across all crates | Done |
| TKT-057 | Lock-free class extraction + Arc\<DiConfig\> area loop | Done |
| TKT-058 | Merged reflection passes + BFS interface propagation | Done |
| TKT-059 | Incremental di.xml cache correctness fix | Done |
| TKT-060 | Archive compare parallelization | Done |

Test count: 197 (up from 176). Build clean, zero warnings.

## Round 3 Status (2026-03-10)

| Ticket | Change | Status |
|--------|--------|--------|
| TKT-061 | Phase 7 micro-optimizations (case index, di.xml cache, plugin-list clone, sub-timers) | Done |

Measured Phase 7 sub-timers: area-config loop ~450ms, plugin-list ~44ms.
Bottleneck identified as `resolve_all_arguments_for_named_types` × 7 areas (pure Rust, 25k types).
Total runtime: ~2.4s clean run on 12-CPU dev box.
