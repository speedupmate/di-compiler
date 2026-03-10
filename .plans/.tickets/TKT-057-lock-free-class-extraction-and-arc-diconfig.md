---
id: TKT-057
title: Eliminate lock contention in class extraction and DiConfig cloning in area loop
phase: 09-performance-hardening
feature: 38-low-level-performance-hardening
owner: Unassigned
status: Done
estimate: S
depends_on: [TKT-056]
touches:
  - rust/di-compiler/crates/cli/src/main.rs
acceptance:
  - Arc<Mutex<FxHashMap>> removed from Phase 1+2; replaced with par_iter().filter_map().collect()
  - AtomicUsize counters (fallback_count, failure_count) no longer need Arc wrapping
  - metadata_base_di_config wrapped in Arc<DiConfig> after final mutation
  - area_di_configs type changed to FxHashMap<String, Arc<DiConfig>>
  - Areas without area-specific overrides use Arc::clone (no real DiConfig clone)
  - cargo build --release clean, cargo test passes
test_plan:
  - cargo build --release
  - cargo test
  - Full run with --compare-archive; verify archive summary counts unchanged
---

# TKT-057: Eliminate lock contention in class extraction and DiConfig cloning in area loop

## Scope

Two independent but structurally similar changes that remove unnecessary synchronization
and cloning overhead:

### A2 — Lock-free class extraction

The Phase 1+2 parallel extraction previously used:

```rust
let class_map: Arc<Mutex<FxHashMap<...>>> = Arc::new(Mutex::new(...));
php_files.par_iter().for_each(|path| {
    // ...
    let mut map = class_map.lock().unwrap();  // serialized on every insert
    map.insert(info.fqcn.clone(), info);
});
let class_map = Arc::try_unwrap(class_map).unwrap().into_inner().unwrap();
```

Replace with:

```rust
let class_map: FxHashMap<String, ClassInfo> = php_files
    .par_iter()
    .filter_map(|path| { /* ... */ Some((info.fqcn.clone(), info)) })
    .collect();
```

Rayon's `ParallelIterator::collect` into `HashMap`-family types uses per-thread local
accumulation and a merge step at the end, eliminating the lock.

### A3 — Arc<DiConfig> in area loop

`metadata_base_di_config` (the large merged DI config) was cloned once per area inside
`AREAS.par_iter()`, including for the 5+ areas that have no area-specific di.xml
overrides and produce an identical config. Freeze the config into `Arc` after the last
mutation:

```rust
let metadata_base_di_config = Arc::new(metadata_base_di_config);
```

Inside the area loop, no-override areas use `Arc::clone` (refcount increment only).
Only areas with actual overrides perform one real clone as the base for `apply_module_config_on_primary`.

## Implementation Notes

- `area_di_configs` downstream type changes to `FxHashMap<String, Arc<DiConfig>>`.
- The plugin-list loop accesses `scope_di_config` via `(**scope_di_config).clone()`
  to get an owned `DiConfig` for mutation before passing to `generate_plugin_list_php`.
- `Arc<T>` auto-derefs to `T` via Deref coercion, so function call sites that take
  `&DiConfig` continue to work without changes.

## Status

Implemented. Build clean, all tests pass.
