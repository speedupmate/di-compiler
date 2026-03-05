# Phase 07: Performance

## Purpose

Achieve < 5 s warm / < 15 s cold wall clock targets. Parallelize all safe phases with
`rayon`. Add incremental fingerprinting to skip unchanged files on re-runs.

## Gate To Enter

TKT-023 green — validator reports zero diffs. Must not regress correctness.

## Gate To Complete

- `cargo bench` wall clock ≤ 5 s warm, ≤ 15 s cold on this Magento install
- PHP fallback rate < 0.5% of files
- Incremental re-run (no changes) completes in < 1 s

## Features In This Phase

| Feature | Deps |
|---------|------|
| [23-parallel-rayon](./.features/23-parallel-rayon.md) | 06, 22 |
| [24-incremental-fingerprinting](./.features/24-incremental-fingerprinting.md) | 23 |

## Profiling Approach

```bash
cargo flamegraph --bin fast-di-compile -- --magento-root /var/www/application
```

Expected hot paths: string allocation in lexer, di.xml parse, file I/O.

## Tickets In This Phase

TKT-025 through TKT-026
