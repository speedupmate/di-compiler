---
id: TKT-048
title: Fix config.php module-order HashMap ordering bug
phase: 08-parity-closure
feature: 36-di-merge-order-and-null-surface
owner: Unassigned
status: Ready
estimate: S
depends_on: []
touches:
  - rust/di-compiler/crates/cli/src/main.rs
acceptance:
  - config.php modules are enumerated in PHP array insertion order (not HashMap hash order)
  - Hyva module index > Magento core module index (e.g. Hyva_Checkout=381 > Magento_Config=21)
  - DI merge sorts di.xml files by (priority, config.php_position, path) with correct positions
  - check_all_areas.php value mismatches drop from 173–189 toward < 50
test_plan:
  - Unit test parse_config_php returns Vec preserving insertion order; reversed input gives reversed indices
  - Compile with Hyva modules; assert configStructure in frontend.php resolves to Hyva\Checkout\...\Interceptor
  - Full compile + check_all_areas.php before/after comparison
---

# TKT-048: Fix config.php module-order HashMap ordering bug

## Scope

The `enabled_modules` HashMap is built from `config_modules.iter().enumerate()` where `config_modules` is a Rust `HashMap<String, i64>`. HashMap iteration order is arbitrary (hash order), not insertion order. This means every module gets a random positional index, making the `(priority, module_order_index, path)` di.xml sort key meaningless — module-order-based preference resolution produces undefined results.

Verified: `Magento_Config` is position 21 in config.php; Hyva modules are 377–382. With correct indices Hyva preferences override Magento core (last-write-wins in merge). With random indices the outcome is undefined, and PHP ground truth consistently differs from our output in Hyva-overridden classes.

## Implementation Notes

Find the function that reads `app/etc/config.php` and returns the modules map (likely near the `parse_config_php` / PHP literal parser in `crates/cli/src/main.rs`). Change the return type from `HashMap<String, i64>` to `Vec<(String, i64)>`. Update the call site to use `Vec::iter().enumerate()`:

```rust
// BEFORE
let enabled_modules: HashMap<String, usize> = parse_config_php(&magento_root)
    .iter()          // arbitrary hash order
    .enumerate()
    .filter(|(_, (_, &v))| v != 0)
    .map(|(i, (k, _))| (k.clone(), i))
    .collect();

// AFTER
let ordered_modules: Vec<(String, i64)> = parse_config_php_ordered(&magento_root);
let enabled_modules: HashMap<String, usize> = ordered_modules
    .iter()
    .enumerate()
    .filter(|(_, (_, v))| *v != 0)
    .map(|(i, (k, _))| (k.clone(), i))
    .collect();
```

The PHP literal parser that currently builds a `HashMap` must instead build a `Vec<(String, i64)>` by appending keys in the order they appear in the file (not inserting into a map).

## Risks

- If the PHP parser rebuilds the map before returning, we lose order at the parser level — fix must go into the parser itself, not just the call site.
- If any code path uses `config_modules` as a HashMap directly elsewhere, those sites also need updating.
