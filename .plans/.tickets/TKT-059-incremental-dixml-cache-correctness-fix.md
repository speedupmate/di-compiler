---
id: TKT-059
title: Fix incremental cache incorrectly dropping di.xml files from merge
phase: 09-performance-hardening
feature: 38-low-level-performance-hardening
owner: Unassigned
status: Done
estimate: XS
depends_on: []
touches:
  - rust/di-compiler/crates/cli/src/main.rs
  - rust/di-compiler/crates/di-xml-reader/src/merger.rs
acceptance:
  - is_unchanged() call removed from di.xml parse filter_map path
  - IncrementalCache::is_unchanged() method removed (dead code)
  - _cache_ref binding renamed to _cache_ref (unused, documented)
  - cargo build --release clean (zero dead-code warnings for this path)
  - cargo test passes including 6 new IncrementalCache tests and 3 merge_configs regression tests
test_plan:
  - cargo test
  - Verify merge_configs tests cover all-input-included property
  - Run incremental compile twice; second run should still produce correct output
---

# TKT-059: Fix incremental cache incorrectly dropping di.xml files from merge

## Problem

`IncrementalCache::is_unchanged()` was called inside the `filter_map` that parsed
di.xml files in Phase 3a:

```rust
let global_di_path_configs: Vec<_> = di_xml_files
    .par_iter()
    .filter_map(|path| {
        if args.incremental && cache_ref.is_unchanged(path) {
            return None;  // BUG: drops the file from Vec entirely
        }
        let r = parse_di_xml(path);
        // ...
    })
    .collect();
```

When `is_unchanged` returned `true`, the file was dropped from `global_di_path_configs`
before reaching `merge_configs`. The merged `di_config` was therefore missing all type
configs, plugins, preferences, and virtual types declared in the skipped files. On the
second incremental run, the output would silently diverge from a clean run.

## Fix

Remove the `is_unchanged` skip entirely from the di.xml parsing path. Di.xml files are
small, fast-to-parse XML; the CPU time saved by skipping them is negligible compared
to the correctness guarantee lost. The incremental cache continues to record di.xml
file hashes for potential future use (e.g. a proper per-file parsed-config cache), but
does not control parse inclusion.

## Correctness Impact

- Incremental `--incremental` runs now always produce output identical to clean runs.
- The incremental cache still saves/loads, records file hashes after the run, and could
  in principle be extended to store serialized DiConfig per file in a future ticket.

## New Tests

- `IncrementalCache`: 6 unit tests covering `hash_of` stability, `record` updates on
  file change, save/load round-trip, and missing-file safety.
- `merge_configs` regression: 3 tests in `di-xml-reader/src/merger.rs` verifying all
  input configs' preferences and plugins appear in the merged output (named with a
  comment referencing the C1 bug).

## Status

Implemented. Build clean, all tests pass.
