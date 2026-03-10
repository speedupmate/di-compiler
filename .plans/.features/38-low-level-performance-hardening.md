# 38: Low-Level Performance Hardening

- Category: Performance
- Status: Implemented
- Implementation Phase: 09-performance-hardening
- Owner: Unassigned
- Feature ID: `low-level-performance-hardening`
- Suggested Dependencies: 34-php-reflection-worker-pool, 23-parallel-rayon, 24-incremental-fingerprinting

## Intent

Apply a set of low-level, independently verifiable optimizations to reduce core compile
wall-clock time from ~7.7 s toward the ≤5 s warm target, and reduce archive compare
time from ~16.2 s toward a practical debugging floor. All changes must preserve
correctness (zero regression in archive compare counts).

## Changes Delivered

### A1 — FxHashMap/FxHashSet across all crates

Replace `std::collections::HashMap`/`HashSet` with `rustc_hash::FxHashMap`/`FxHashSet`
throughout all six crates. `rustc-hash` was already a workspace dependency in
`code-generator`; this extends it uniformly. FxHasher uses a non-cryptographic
identity-based hash that is ~2× faster than SipHash for short string keys.

Files touched: all `src/` files in `di-resolver`, `di-xml-reader`, `php-extractor`,
`code-generator`, and `cli`.

### A2 — Lock-free class extraction (Phase 1+2)

The previous `par_iter().for_each()` path serialized every successful class insert
through a global `Arc<Mutex<FxHashMap>>`. Replace with
`par_iter().filter_map(...).collect()`, which lets Rayon's work-stealing scheduler
accumulate results into per-thread local buffers and merge them without any locks.

Location: `crates/cli/src/main.rs` Phase 1+2 block.

### A3 — Arc<DiConfig> in area loop

`metadata_base_di_config` (the merged global DI config) was cloned once per area in
the `AREAS.par_iter()` loop (7 areas × 1 clone = 7 full DiConfig clones). Wrapping
it in `Arc` reduces the no-override path to a refcount increment; only areas that
actually merge area-specific files perform a real clone.

### B — Merged reflection passes

Three sequential `par_iter()` calls (`enrich_constructor_defaults_with_reflection`,
`enrich_inherited_constructors_with_reflection`,
`enrich_virtual_target_constructors_with_reflection`) ran with two Rayon barriers
between them. Their candidate sets are disjoint (kind-0: has ctor with const default;
kind-1: no ctor + extends; kind-2: VT target absent from class_map). A single
`enrich_all_constructors_with_reflection` call collects all candidates in one
sequential scan and dispatches one `par_iter()`.

### C1 — Incremental di.xml cache correctness fix

`IncrementalCache::is_unchanged()` was called on each di.xml path inside the
`filter_map` before parsing. When true, the file was dropped from the resulting
`Vec` entirely — never reaching `merge_configs`. This silently produced an incomplete
`di_config` on incremental runs. Fix: remove the skip entirely. Di.xml parsing is
fast XML work; correctness requires all files to participate in the merge.

### C2 — BFS + reverse-index for interface propagation

The fixed-point interface propagation loop in `build_interception_registry` scanned
all class_map entries on every iteration, with O(n × depth) total work (depth ≈ 4 for
Magento's interface hierarchy). Replace with a prebuilt reverse-index
(`interface → Vec<implementors>`) and a BFS expansion from the current intercepted
seed set, completing in one O(n + edges) pass.

### D — Archive compare parallelization

`write_comparable_metadata_reports` looped sequentially over every common metadata
file, spawning two fresh `php -r` subprocesses per file (archive normalization +
output normalization). With ~N files this was responsible for ~16.2 s of the 23.9 s
total validation time. Replace with `par_iter()` over the file list; each iteration
is independent (distinct paths, no shared output). Results are collected in order for
deterministic manifest output.

## Behavior Contract

- Output of every generated PHP file is byte-for-byte identical to the pre-optimization baseline.
- `cargo test` passes (197 tests across all crates).
- Archive compare summary counts are unchanged.
- Incremental cache still records file hashes and saves/loads correctly; it no longer
  controls di.xml parse inclusion.

## Test Plan

1. `cargo test` — 197 tests green.
2. Full run with `--compare-archive`; confirm archive summary counts unchanged.
3. New unit tests added:
   - `IncrementalCache`: 6 tests for hash stability, record/load round-trip, missing-file safety.
   - `enrich_all_constructors_with_reflection`: 6 tests covering all three candidate kinds
     and their exclusion criteria.
   - `merge_configs` C1 regression: 3 tests verifying all input configs appear in merged output.

## Acceptance Criteria

- Core compile (no archive compare) measurably below 7.7 s baseline.
- Archive compare measurably below 16.2 s baseline.
- `cargo test` green with zero warnings.
- No regression in archive compare counts (`code_missing`, `code_extra`, `metadata_changed`).
