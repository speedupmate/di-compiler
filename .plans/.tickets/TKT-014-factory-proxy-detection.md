---
id: TKT-014
title: Factory + Proxy detection
phase: 03-di-resolver
feature: factory-detection, proxy-detection
owner: Unassigned
status: Ready
estimate: M
depends_on: [TKT-008, TKT-012]
touches:
  - rust/di-compiler/crates/di-resolver/src/factories.rs
  - rust/di-compiler/crates/di-resolver/src/proxies.rs
acceptance:
  - Factory list matches PHP APPLICATION_CODE_GENERATOR output
  - Proxy list matches PHP PROXY_GENERATOR output
  - Already-existing classes not re-generated
---

# TKT-014: Factory + Proxy Detection

## Scope

Scan all constructor params for `Factory`/`\Proxy` suffixes that don't exist on disk.

## Implementation Notes

**Factory detection:**
```rust
pub fn detect_factories(class_map: &HashMap<String, ClassInfo>) -> Vec<FactorySpec> {
    class_map.values()
        .filter_map(|info| info.constructor.as_ref())
        .flat_map(|c| &c.params)
        .filter(|p| p.type_hint.as_deref().map(|t| t.ends_with("Factory")).unwrap_or(false))
        .filter(|p| !class_map.contains_key(p.type_hint.as_deref().unwrap()))
        .map(|p| {
            let factory_fqcn = p.type_hint.clone().unwrap();
            let target_fqcn = factory_fqcn.trim_end_matches("Factory").to_string();
            FactorySpec { target_fqcn, factory_fqcn }
        })
        .collect::<HashSet<_>>()  // deduplicate
        .into_iter().collect()
}
```

**Proxy detection:** similar scan for `\Proxy` suffix + di.xml `xsi:type="object"` values ending in `\Proxy`.

## Risks

- Same Factory may be referenced by multiple classes — deduplicate
