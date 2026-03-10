---
id: TKT-013
title: Interceptor detection
phase: 03-di-resolver
feature: interceptor-detection
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-008, TKT-012]
touches:
  - rust/di-compiler/crates/di-resolver/src/interceptors.rs
acceptance:
  - Interceptor list matches PHP INTERCEPTION operation output
  - final classes excluded
  - abstract classes excluded
  - Plugin sort order preserved
---

# TKT-013: Interceptor Detection

## Scope

Determine which classes need interceptors and build `Vec<InterceptorSpec>`.

## Implementation Notes

```rust
pub fn detect_interceptors(
    class_map: &HashMap<String, ClassInfo>,
    config: &DiConfig,
) -> Vec<InterceptorSpec>
```

Algorithm:
1. Collect all FQCNs that appear in any `<plugin type="X">` across merged config
2. For each such FQCN:
   a. Resolve through preferences: `config.get_preference(fqcn)` → concrete class
   b. Check if concrete class is in `class_map`
   c. Check `ClassInfo.is_abstract` → skip if true
   d. Check if class is `final` — need to extract `final` flag from ClassInfo (add to TKT-003/004)
   e. Get active plugins: `config.get_active_plugins(fqcn)`
   f. Get public methods from ClassInfo
3. Build `InterceptorSpec { fqcn, plugins, public_methods }`

## Risks

- Plugin `type=` attribute may reference an interface, not the concrete class — preference resolution needed
