---
id: TKT-063
title: Phase 7 — global arg resolution once + area deltas + parallelization
phase: 09-performance-hardening
feature: 38-low-level-performance-hardening
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-061]
touches:
  - rust/di-compiler/crates/cli/src/main.rs
  - rust/di-compiler/crates/di-resolver/src/arguments.rs
  - rust/di-compiler/crates/code-generator/src/area_config.rs
acceptance:
  - resolve_all_arguments_for_named_types uses par_iter internally (rayon)
  - global arg resolution runs once before area loop (~37ms)
  - dep reverse index built in parallel after global resolution (~40ms)
  - per-area delta resolves only affected types (union of area-specific di.xml keys + interception overrides, expanded via dep index)
  - generate_area_config_with_overrides accepts (args_baseline, args_delta) — no HashMap clone
  - area-config loop: ~153ms (from ~450ms)
  - cargo build --release clean, zero warnings
  - cargo test passes
---

# TKT-063: Phase 7 Global Arg Resolution + Delta + Parallelization

## Scope

Three coordinated changes to eliminate redundant argument resolution work in Phase 7.

### 1 — par_iter inside resolve_all_arguments_for_named_types (DELTA-E)

Changed `type_names.iter()` to `type_names.par_iter()` with `filter_map(...).collect()`.
All inputs (`DiConfig`, `ClassInfo` map) are `Sync`. Added `rayon` to `di-resolver/Cargo.toml`.
Global resolution: ~37ms (from sequential baseline of ~135ms).

### 2 — Global args once + dep reverse index (OPT-DELTA + DEP-IDX)

Before the area loop, resolve all ~25,234 types against the global (area-less) DiConfig.
After resolution, build a dep reverse index: `referenced_fqcn → Vec<owner_fqcn>`. Used
to expand the "affected" type set when area-specific preferences change.

Both passes run in parallel (rayon par_iter). Combined ~77ms vs previous ~850ms (7 passes).

### 3 — Per-area delta merge (OVERLAY via split-map)

`generate_area_config_with_overrides` accepts `(args_baseline, args_delta)` instead of
one pre-merged map. In the delta path: `args_delta.get(fqcn).unwrap_or(&args_baseline[fqcn])`
— no clone. In the fast path: pass `&FxHashMap::default()` as delta.

Eliminates 25K `HashMap::clone()` on every area iteration. Wrapper functions
(`generate_area_config`, `generate_area_config_with_extra_preferences`) pass
`&FxHashMap::default()` as the delta for the simple case.

## Status

Implemented. All tests pass. Area-config loop: 153ms (from 450ms).
