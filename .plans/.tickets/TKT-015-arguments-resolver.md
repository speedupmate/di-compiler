---
id: TKT-015
title: Arguments resolver
phase: 03-di-resolver
feature: arguments-resolver
owner: Unassigned
status: Ready
estimate: L
depends_on: [TKT-012, TKT-008]
touches:
  - rust/di-compiler/crates/di-resolver/src/arguments.rs
acceptance:
  - Output matches PHP ArgumentsResolver for 100-class validation sample
  - _i_ / _ins_ / _v_ / _vn_ / _vac_ / _a_ notation all produced correctly
  - Preference and virtualType chains followed
  - di.xml <argument> overrides applied
---

# TKT-015: Arguments Resolver

## Scope

Map each class's constructor params to Magento's resolved argument notation.
This populates `ResolvedGraph.constructor_map`.

## Implementation Notes

```rust
pub fn resolve_arguments(
    class_fqcn: &str,
    constructor: &Constructor,
    config: &DiConfig,
) -> Vec<ResolvedParam>
```

Per parameter:
1. Get di.xml `<argument name="$param_name">` override if present → use that directly
2. If type hint is present:
   - Is primitive (string/int/bool/array/null keyword)? → `_v_` with default
   - Otherwise: resolve preference + virtualType chain
   - Check `is_shared` → `_i_` or `_ins_`
3. If no type hint:
   - Has default? → `_v_` with default value
   - Default is null? → `_vn_`
   - No default and no type? → `_v_` with null

Parent class argument inheritance:
- Look up parent class's `type_config.arguments` in config
- Child arguments override parent by name

## Risks

- PHP default value representation must match exactly (PHP uses `null`, not JSON `null`)
