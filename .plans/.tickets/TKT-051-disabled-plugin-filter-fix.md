---
id: TKT-051
title: Fix filter_enabled_di_xml dropping disabled plugin entries
phase: 08-parity-closure
feature: 30-plugin-list-metadata-generation
owner: Unassigned
status: Ready
estimate: S
depends_on: []
touches:
  - rust/di-compiler/crates/cli/src/main.rs
acceptance:
  - 47 missing disabled plugin entries appear in Section 0 output
  - filter_enabled_di_xml does not drop di.xml files that contain disabled plugin definitions for enabled modules
  - FunctionalTestingFramework and other test packages remain excluded
test_plan:
  - Diagnose: grep for missing plugin names across vendor di.xml files; confirm which files are being dropped
  - After fix: check_all_areas.php or plugin-list diff shows 47 fewer missing paths
  - Verify FunctionalTestingFramework classes still absent from output
---

# TKT-051: Fix filter_enabled_di_xml dropping disabled plugin entries

## Scope

`filter_enabled_di_xml` (added in Wave 2) drops vendor packages that have no `module.xml` AND no `registration.php`, treating them as utility/test packages. However, some packages that fall into this category contain `<plugin disabled="true">` entries that PHP's DI compiler processes. These disabled entries appear in plugin-list Section 0 with `'disabled' => true` and we're missing them.

Missing entries confirmed: `update_bundle_products_stock_item_status`, `stockedProductsFilterPlugin`, `updateStockChangedAuto`, and 44 others.

## Implementation Notes

**Step 1 — Diagnose:**
```bash
grep -r "update_bundle_products_stock_item_status\|stockedProductsFilterPlugin\|updateStockChangedAuto" \
  /var/www/application/vendor /var/www/application/app --include="di.xml" -l
```
Identify which di.xml files own these disabled plugins.

**Step 2 — Check filter:**
For each identified file, check if it has `module.xml` or `registration.php` in its inferred module root.

**Step 3 — Fix:**
If the di.xml files are from enabled modules that happen to lack both files (rare edge case), adjust `filter_enabled_di_xml` to also check if the di.xml's owning package is referenced as a dependency by an enabled module (via `vendor/composer/installed.json`). Alternatively, only drop packages from the curated exclusion list (FunctionalTestingFramework) by name rather than by file-presence heuristic.

A simpler targeted fix: instead of heuristic file-presence filtering, maintain an explicit denylist of test packages:
```rust
const EXCLUDED_PACKAGES: &[&str] = &[
    "magento2-functional-testing-framework",
    "magento2-functional-testing-framework-allure-adapter",
];
fn filter_enabled_di_xml(files: Vec<PathBuf>, ...) -> Vec<PathBuf> {
    files.into_iter().filter(|path| {
        // Explicit denylist beats heuristic
        if EXCLUDED_PACKAGES.iter().any(|pkg| path.to_string_lossy().contains(pkg)) {
            return false;
        }
        // ...existing module.xml / registration.php logic for unknown packages
    }).collect()
}
```

## Risks

- If the missing plugins are from packages correctly excluded (true test packages), the fix direction is wrong — the missing entries must be from a different root cause.
- Broadening the filter could re-introduce FunctionalTestingFramework test classes. Use the explicit denylist approach to stay safe.
