---
id: TKT-049
title: Exclude pure interfaces from plugin-list Section 1 unless directly registered
phase: 08-parity-closure
feature: 30-plugin-list-metadata-generation
owner: Unassigned
status: Ready
estimate: S
depends_on: []
touches:
  - rust/di-compiler/crates/code-generator/src/plugin_list.rs
acceptance:
  - HttpGetActionInterface, HttpPostActionInterface, HttpHeadActionInterface, OrderInterface, IndexInterface, CsrfAwareActionInterface are absent from Section 1 in our output (matching PHP ground truth)
  - Pure interfaces with plugins directly registered on them (in Section 0 / plugin_data) still appear in Section 1
  - No valid plugin entries dropped for concrete classes
test_plan:
  - PHP ground truth verification: run php -r against generated/_metadata/pluginList.php; confirm all 6 interfaces NOT PRESENT in Section 1
  - After fix: run against generated/metadata/pluginList.php; confirm same 6 interfaces absent
  - Confirm concrete classes implementing those interfaces still have inherited plugins in Section 1
---

# TKT-049: Exclude pure interfaces from plugin-list Section 1 unless directly registered

## Scope

PHP's plugin-list compiler does not propagate inherited plugins to sub-interfaces via interface→interface chains. Our `inherit_plugins()` function walks the full `extends+implements` hierarchy without checking ClassKind, causing pure interfaces (like `HttpGetActionInterface` which extends `ActionInterface` which has plugins) to incorrectly appear in Section 1 with plugin arrays.

Verified against PHP ground truth (`generated/_metadata/pluginList.php`): all 6 reported interfaces are NOT PRESENT in Section 1.

## Implementation Notes

In the loop that builds Section 1 (inherited) in `crates/code-generator/src/plugin_list.rs`, guard pure interfaces:

```rust
for type_name in all_types {
    // PHP does not include interfaces in Section 1 unless they have
    // plugins directly registered on them in Section 0.
    let is_pure_interface = class_map
        .get(type_name.trim_start_matches('\\'))
        .map(|c| matches!(c.kind, ClassKind::Interface))
        .unwrap_or(false);
    if is_pure_interface && !plugin_data.contains_key(type_name) {
        continue;
    }
    // existing inherit_plugins logic...
}
```

`plugin_data` is the Section 0 map; if an interface has plugins directly registered (rare but possible), it should still appear in Section 1.

## Risks

- Types not in `class_map` (di.xml-only) default to `false` for `is_pure_interface` — they still get processed. This is correct: if we don't know the kind, we err on the side of inclusion.
- Interface types that DO have plugins registered directly (not via inheritance) must not be skipped. The `!plugin_data.contains_key(type_name)` guard handles this.
