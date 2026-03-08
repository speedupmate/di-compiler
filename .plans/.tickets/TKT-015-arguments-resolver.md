---
id: TKT-015
title: Arguments resolver
phase: 03-di-resolver
feature: arguments-resolver
owner: Unassigned
status: Done
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

## Completion Note (2026-03-08)

Two critical correctness fixes were implemented in `crates/di-resolver/src/arguments.rs`
(`merged_di_arguments_for_type_name`):

**Fix 1 — Interface argument inheritance:** PHP's `Config::_collectConfiguration` calls
`ClassReader::getParents()` which returns both parent classes and directly-implemented
interfaces. The function previously only walked the `extends` chain. Fix: for each class
in the extends chain, inject its "new" interfaces (those not inherited from the parent
class, mirroring `array_diff(class.implements, parent.implements)`) just before the class
in merge order so interfaces get lower priority than the class's own args.

Key impact: arguments registered on `Magento\Framework\Console\CommandListInterface`
(commands from 50+ modules) now flow into `Magento\Framework\Console\CommandList`.

**Fix 2 — Recursive array merge in type hierarchy:** The function was replacing same-name
args (`merged[idx] = arg`) instead of recursively merging array items. Return type changed
from `Vec<&'a Argument>` (borrow) to `Vec<Argument>` (owned); added `merge_argument_into()`
that recursively merges array items by key, mirroring PHP `array_replace_recursive`.

Key impact: the `commands` array with 103 items from `CommandListInterface` was being
overwritten by `CommandList`'s own 3-item array. Now properly merged to 103 items.

**Result:** `bin/magento list` reports 177 commands in compiled mode, matching developer
mode exactly (was 62 before the fixes). Verified multiple classes against PHP runtime via
`$config->getArguments(...)` — all match.
