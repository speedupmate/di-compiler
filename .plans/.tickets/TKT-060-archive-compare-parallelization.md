---
id: TKT-060
title: Parallelize archive compare metadata normalization loop
phase: 09-performance-hardening
feature: 38-low-level-performance-hardening
owner: Unassigned
status: Done
estimate: S
depends_on: [TKT-036]
touches:
  - rust/di-compiler/crates/cli/src/main.rs
acceptance:
  - write_comparable_metadata_reports uses par_iter() over common metadata files
  - Each iteration spawns two php -r subprocesses independently (archive + output normalization)
  - Manifest lines collected in original sorted order after par_iter completes
  - First io::Error in any parallel iteration is propagated after collect
  - cargo build --release clean, cargo test passes
  - Archive compare wall-clock time measurably below 16.2 s baseline on this install
test_plan:
  - cargo build --release
  - Run with --compare-archive; record wall clock and compare summary counts
  - Verify counts are identical to pre-change baseline
---

# TKT-060: Parallelize archive compare metadata normalization loop

## Scope

`write_comparable_metadata_reports` previously processed each common metadata file
sequentially:

```rust
for rel in common {
    let archive_json = normalize_metadata_to_json_bytes(&archive_src, php_bin)?;  // php -r spawn
    let output_json  = normalize_metadata_to_json_bytes(&output_src,  php_bin)?;  // php -r spawn
    // write 4 output files, append manifest line
}
```

With ~N files, this spawns 2N fresh PHP subprocesses sequentially. On this install the
measured cost was **16.2 s**, representing 68 % of total `--compare-archive` runtime.

## Fix

Replace with `common.par_iter().map(...)`. Each iteration:
1. Calls `normalize_metadata_to_json_bytes` twice (two `php -r` spawns — independent, no shared state)
2. Writes 4 output files (distinct paths per file — no write conflicts)
3. Returns an `io::Result<String>` manifest line

After `collect`, results are in the original sorted order (Rayon preserves `par_iter`
order for indexed iterators). The manifest is assembled by appending each returned line.
The first error short-circuits via `?` on the `Result`.

## Expected Outcome

With 8 threads and N files, wall clock reduces from O(N × per-file) to
O(ceil(N/8) × per-file). Expected time: ~2–3 s (down from 16.2 s).

## Notes

- `normalize_metadata_to_json_bytes` spawns `Command::new(php_bin)` — not the
  persistent worker pool. Each subprocess is independent and the function is safe to
  call from multiple Rayon threads simultaneously.
- This path is only active under `--compare-archive` (debugging / validation workflow).
  Production compile time is unaffected.

## Status

Implemented. Build clean, all tests pass.
