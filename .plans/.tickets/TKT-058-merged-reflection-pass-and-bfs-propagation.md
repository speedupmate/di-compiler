---
id: TKT-058
title: Merge three sequential reflection passes and replace fixed-point propagation with BFS
phase: 09-performance-hardening
feature: 38-low-level-performance-hardening
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-056]
touches:
  - rust/di-compiler/crates/cli/src/main.rs
acceptance:
  - Three separate enrich_*_with_reflection functions replaced by one enrich_all_constructors_with_reflection
  - Single par_iter() over unified candidate list; two Rayon barriers eliminated
  - Fixed-point interface propagation loop in build_interception_registry replaced with BFS + reverse-index
  - Old three-function implementations removed (not just dead-code suppressed)
  - cargo build --release with zero warnings
  - cargo test passes (197 tests including 6 new enrich_all_constructors tests)
test_plan:
  - cargo test
  - Verify new tests cover all three candidate kinds (kind-0, kind-1, kind-2) and exclusion criteria
---

# TKT-058: Merge three sequential reflection passes and replace fixed-point propagation with BFS

## Scope

### B — Merged reflection passes

Three helper functions ran sequentially with two Rayon work-stealing barriers:

1. `enrich_constructor_defaults_with_reflection` — classes with `::` in ctor defaults (kind-0)
2. `enrich_inherited_constructors_with_reflection` — classes in type universe with no ctor that extend something (kind-1)
3. `enrich_virtual_target_constructors_with_reflection` — VT targets absent from class_map (kind-2)

These candidate sets are provably disjoint:
- kind-0 requires `constructor.is_some()` with const defaults
- kind-1 requires `constructor.is_none() && !is_abstract && extends.is_some()`
- kind-2 requires class not in class_map at all

Replace with a single `enrich_all_constructors_with_reflection` that:
1. Collects candidates from all three passes in one sequential scan (tagged by kind)
2. Dispatches one `par_iter()` over the unified list
3. Applies all results (kind-0/1: update existing entry; kind-2: or_insert_with synthetic)

### C2 — BFS interface propagation

The fixed-point loop in `build_interception_registry`:

```rust
let mut changed = true;
while changed {
    changed = false;
    for (fqcn, info) in class_map {
        // scan all entries every iteration
    }
}
```

This is O(n × depth) with depth ≈ 4 for Magento's interface hierarchy. Replace with:

1. Build reverse-index once: `interface_fqcn → Vec<implementor_fqcn>` (O(n × avg_ifaces))
2. BFS from all currently-intercepted seeds via `VecDeque<String>`
3. Each newly-intercepted class becomes a new seed; terminates in one O(n + edges) pass

## Implementation Notes

- The VecDeque must hold owned `String` (not `&str`) to avoid borrow conflicts when
  `intercepted_targets.insert()` mutably borrows the set that was originally borrowed
  for the seed construction.
- `implementors` map holds `&str` borrowed from `class_map`, which lives for the
  duration of the function — lifetimes are safe.

## Status

Implemented. Build clean, all tests pass. Three old functions removed.
