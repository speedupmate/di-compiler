# Phase 07: Performance

## Purpose

Achieve < 5 s warm / < 15 s cold wall clock targets. Parallelize all safe phases with
`rayon`. Add incremental fingerprinting to skip unchanged files on re-runs.

## Gate To Enter

TKT-023 green — validator reports zero diffs. Must not regress correctness.

If parity is not yet green, performance work is exploratory only and must not be used as a release gate.

## Gate To Complete

- `cargo bench` wall clock ≤ 5 s warm, ≤ 15 s cold on this Magento install
- PHP fallback rate < 0.5% of files
- Incremental re-run (no changes) completes in < 1 s

## Features In This Phase

| Feature | Deps |
|---------|------|
| [23-parallel-rayon](../.features/23-parallel-rayon.md) | 06, 22 |
| [24-incremental-fingerprinting](../.features/24-incremental-fingerprinting.md) | 23 |

## Profiling Approach

```bash
cargo flamegraph --bin fast-di-compile -- --magento-root /var/www/application
```

Expected hot paths: string allocation in lexer, di.xml parse, file I/O.

## Tickets In This Phase

TKT-025 through TKT-026

## Current Snapshot (2026-05-15)

Warm-cache target of < 5s achieved. Incremental re-run (FP-SCOPE-3 skip) consistently ~1.29s.

- First run (cold fingerprint): `~2.2s` total
- Repeat run (Phase 7 skipped): `~1.29s` total
- Phase 7 (first run): `~0.85s`
- Phase 7 (warm FP hit): `~6ms`

All Phase 7 optimizations are complete and tracked in Phase 09:
  - [09-performance-hardening](09-performance-hardening.md)
  - [analyzis/phase-7-performance.md](../analyzis/phase-7-performance.md)
