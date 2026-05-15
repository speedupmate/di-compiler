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

## Current Status (2026-05-15)

All Phase 9 optimizations complete. Phase 7 reduced from 4.1s to <1s first run, ~6ms on FP-hit runs.

- Latest full run (warm filesystem cache, no FP hit):
  - Total: `~2.2s`
  - Phase 7: `~0.85s`
- Repeat run (FP-SCOPE-3 hit — Phase 7 entirely skipped):
  - Total: `~1.29s`
  - Phase 7: `~6ms`
- Residual parity closure tracked in Phase 08:
  - `code_extra=3`
  - `metadata_changed=16`
  - `arguments` key-surface extras: 40 keys across all areas (deferred)

## Features In This Phase

| Feature | Deps |
|---------|------|
| [34-php-reflection-worker-pool](../.features/34-php-reflection-worker-pool.md) | 21, 23 |
| [38-low-level-performance-hardening](../.features/38-low-level-performance-hardening.md) | 34, 23, 24 |

## Tickets In This Phase

TKT-035 through TKT-037, TKT-042, TKT-056 through TKT-064

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

## Round 4 Status (2026-05-15)

| Change | Label | Status |
|--------|-------|--------|
| OPT-PLUGIN: par_iter plugin-list loop | DELTA-F | Done (44ms→11ms) |
| OPT-TYPENAMES: Arc<Vec<String>> type names | OPT-TYPENAMES | Done |
| OPT-INDEXES: O(1) preference/typeconfig index inserts | OPT-INDEXES | Done |
| Global arg resolution once + par_iter | OPT-DELTA + DELTA-E | Done (→37ms global) |
| Dep reverse index build parallel | DEP-IDX | Done (~40ms) |
| Serializer baseline+delta split | OVERLAY | Done (204ms→153ms area loop) |
| `write!` + pre-allocated String in area config | SER-1/SER-2 | Done |
| `escape_php` Cow (zero-alloc, chars-based) | SER-4 | Done |
| Phase 7 input fingerprint + full-phase skip | FP-SCOPE-3 | Done (~850ms→6ms warm) |
| Code review fixes (fp completeness, file existence, non-ASCII, spad) | — | Done |

Full analysis and deferred items: `analyzis/phase-7-performance.md`
