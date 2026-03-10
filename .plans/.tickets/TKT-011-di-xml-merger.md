---
id: TKT-011
title: di.xml merger
phase: 02-di-xml-reader
feature: di-xml-merger
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-010]
touches:
  - rust/di-compiler/crates/di-xml-reader/src/merger.rs
acceptance:
  - Merged preferences match PHP DomMapper output for global area
  - Plugin accumulation correct (same plugin name in two files = override, not duplicate)
  - Disabled plugins retained with disabled=true flag
---

# TKT-011: di.xml Merger

## Scope

Merge `Vec<PartialDiConfig>` (in Magento load order) into a single `DiConfig`.

## Implementation Notes

```rust
pub fn merge_di_configs(configs: Vec<(LoadOrder, PartialDiConfig)>) -> DiConfig
```

Merge rules:
- `preferences`: later `HashMap::insert` wins
- `type_configs[name].arguments`: `HashMap::extend` (later arg name overrides earlier)
- `type_configs[name].plugins`: if same plugin name appears → update the existing entry; if new name → push
- `virtual_types`: later wins
- `shared`: later wins

Sort input by `LoadOrder` before merging:
1. `vendor/magento/*/etc/di.xml`
2. `vendor/*/etc/di.xml` (non-magento)
3. `app/etc/di.xml`
4. `app/code/*/etc/di.xml`

## Risks

- Load order must exactly match PHP FileResolver order — verify against Phase 00 strace output
