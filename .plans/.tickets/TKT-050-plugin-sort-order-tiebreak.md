---
id: TKT-050
title: Add secondary sort-by-name tiebreak for plugins with equal sort_order
phase: 08-parity-closure
feature: 30-plugin-list-metadata-generation
owner: Unassigned
status: Done
estimate: XS
depends_on: []
touches:
  - rust/di-compiler/crates/code-generator/src/plugin_list.rs
acceptance:
  - Plugins with equal sort_order values are ordered alphabetically by plugin name
  - Section 2 processed entries match PHP ground truth ordering for Magento\Quote\Model\QuoteManagement and other tie cases
  - No sort regression for plugins with unique sort_order values
test_plan:
  - Unit test: two plugins same sort_order, different names → alphabetical order in output
  - check_all_areas.php plugin-list mismatches drop by 14+
---

# TKT-050: Secondary sort-by-name tiebreak for plugins with equal sort_order

## Scope

Plugin sort uses a single key (`sort_order` integer). Plugins with equal sort_order produce non-deterministic output because Rust's sort is not stable by default when the comparison returns `Equal`. PHP uses a stable secondary sort by plugin name as a tiebreak.

Confirmed example: `Magento\Quote\Model\QuoteManagement_submit___self` — position 1 should be `persistent_convert_customer_cart_to_guest_cart` but we emit `validate_purchase_order_number`.

## Implementation Notes

Change all `sort_by` calls on plugin slices in `plugin_list.rs`:

```rust
// BEFORE
sorted_plugins.sort_by(|a, b| a.sort_order.cmp(&b.sort_order));

// AFTER
sorted_plugins.sort_by(|a, b| {
    a.sort_order.cmp(&b.sort_order)
        .then_with(|| a.name.cmp(&b.name))
});
```

Search for all occurrences of `sort_order.cmp` in `plugin_list.rs` and apply the tiebreak consistently.

## Risks

- None significant. Secondary sort by name is deterministic and matches PHP behavior.
