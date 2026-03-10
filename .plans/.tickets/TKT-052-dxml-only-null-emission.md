---
id: TKT-052
title: Emit NULL for di.xml-only configured types missing from PHP scan
phase: 08-parity-closure
feature: 36-di-merge-order-and-null-surface
owner: Unassigned
status: Ready
estimate: S
depends_on: [TKT-035]
touches:
  - rust/di-compiler/crates/di-resolver/src/arguments.rs
acceptance:
  - Types present in di_config.type_configs but absent from class_map emit NULL in area config arguments (matching PHP behavior)
  - No NULL emitted for types ending in Interface, Interceptor, Factory, or Proxy
  - check_all_areas.php missing count drops from 87–155 toward < 40 per area
  - No new extra entries introduced (the heuristic filters must be tight)
test_plan:
  - Unit test: type_name "Foo\Bar" in type_configs but not class_map → NULL emitted
  - Unit test: "Foo\BarInterface" → not emitted
  - Unit test: "Foo\BarInterceptor" → not emitted
  - Full compile + check_all_areas.php before/after comparison
---

# TKT-052: Emit NULL for di.xml-only configured types

## Scope

PHP's DI compiler emits `'ClassName' => NULL` in area config arguments for any concrete class in the type universe that has no configured constructor arguments. This includes classes that appear in di.xml `<type name="...">` entries even when no PHP file was found on disk.

Our code only emits NULL for types in `base_class_fqcns` (source PHP scan). Types that are known only via di.xml `<type>` entries but whose PHP files are not in our scan paths silently drop out of the output, causing 63–131 missing entries per area.

**Note:** The scan-path coverage gap (PSR-4 packages without `registration.php`) is addressed by TKT-035. This ticket addresses the complementary case: types in di.xml that are genuinely not found by any scanner.

## Implementation Notes

In `crates/di-resolver/src/arguments.rs`, `resolve_all_arguments_for_named_types`, add a branch for types not found in `class_map`:

```rust
for type_name in type_names {
    let name = type_name.trim_start_matches('\\');

    let kind = class_map.get(name).map(|c| c.kind.clone());

    // Skip known non-concrete kinds
    if matches!(kind, Some(ClassKind::Interface) | Some(ClassKind::AbstractClass)) {
        continue;
    }

    if !class_map.contains_key(name) {
        // Type not found by any scanner.
        // PHP emits NULL for di.xml-configured concrete types not found on disk.
        let looks_like_interface = name.ends_with("Interface");
        let is_generated = name.ends_with("Interceptor")
            || name.ends_with("Factory")
            || name.ends_with("Proxy");
        if !looks_like_interface && !is_generated && di_config.type_configs.contains_key(name) {
            result.insert(type_name.clone(), vec![]); // → NULL in area config
        }
        continue;
    }

    // ... existing resolution logic
}
```

The heuristics (`ends_with`) are intentionally conservative to avoid introducing extra entries. If a type name doesn't match any exclusion, we default to not emitting it unless it's explicitly in `type_configs`.

## Risks

- The heuristic may exclude some legitimate concrete classes whose names end in `Interface` (rare but possible). Monitor `extra` count after applying this fix.
- If TKT-035 (PSR-4 scan) resolves many of the missing entries first, the remaining gap for this ticket will be smaller.
