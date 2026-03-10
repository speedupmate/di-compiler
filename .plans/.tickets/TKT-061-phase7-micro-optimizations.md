---
id: TKT-061
title: Phase 7 micro-optimizations — case index, di.xml cache, plugin-list clone reduction
phase: 09-performance-hardening
feature: 38-low-level-performance-hardening
owner: Unassigned
status: Done
estimate: S
depends_on: [TKT-057, TKT-058]
touches:
  - rust/di-compiler/crates/cli/src/main.rs
  - rust/di-compiler/crates/code-generator/src/plugin_list.rs
acceptance:
  - build_case_index extracted from canonicalize_instance_reference_case; built once before AREAS.par_iter()
  - apply_case_index used inside area loop (7x rebuild → 1x)
  - Phase 3b produces area_di_xml_cache (FxHashMap<PathBuf, DiConfig>); area loop looks up from cache
  - compile_plugin_list takes include_virtual_types bool; clone+clear of DiConfig eliminated
  - area_instance_type_overrides clone per area removed; &global_interceptor_map passed directly
  - Sub-timers added: Phase 7 area-config loop and plugin-list loop logged at debug level
  - cargo build --release clean, zero warnings
  - cargo test passes (all existing tests green)
test_plan:
  - cargo build --release
  - cargo test
  - Run with --verbose; verify Phase 7 area-config and plugin-list sub-timers appear
---

# TKT-061: Phase 7 micro-optimizations

## Scope

Five targeted reductions inside Phase 7 (Generate metadata files):

### 1 — Pre-build case index once

`canonicalize_instance_reference_case` previously rebuilt a full lowercase→canonical
FQCN index from `interception_class_map` on every area iteration (7×). Extracted
`build_case_index` and `apply_case_index`; the index is built once before
`AREAS.par_iter()` and passed in via reference.

### 2 — Area di.xml parse cache

Phase 3b already parses all area-specific di.xml files (280 files beyond the global
set). The area loop at Phase 7 re-parsed the same files on demand. Phase 3b now
produces `area_di_xml_cache: FxHashMap<PathBuf, DiConfig>`. The area loop looks up
parsed configs from cache; `parse_di_xml` is only called as a fallback for files not
in the cache (should not occur in practice).

### 3 — Plugin-list clone reduction

`compile_plugin_list` previously required a full `DiConfig` clone followed by
`.virtual_types.clear()` for every non-global scope (6 of 7 plugin scopes).
Added `include_virtual_types: bool` parameter: when `false`, the virtual-type
inheritance pass is skipped entirely. The caller passes `scope == "global"` and
the clone+clear is eliminated.

### 4 — Eliminate area_instance_type_overrides clone

`global_interceptor_map.clone()` was called once per area (7×) to produce
`area_instance_type_overrides`, which was only used for read-only `.contains_key()`
calls. Replaced with `&global_interceptor_map` passed directly to the filter closure
and `generate_area_config_with_overrides`.

### 5 — Sub-timers inside Phase 7

Added `Instant::now()` timers with `log::debug!` output around the area-config loop
and plugin-list loop. Visible with `--verbose`. Measured results:
- Area-config loop: ~450ms (dominated by `resolve_all_arguments_for_named_types` × 7)
- Plugin-list loop: ~44ms

## Status

Implemented. Build clean, all tests pass.
