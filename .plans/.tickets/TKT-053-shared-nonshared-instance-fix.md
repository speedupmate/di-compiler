---
id: TKT-053
title: Fix _i_ vs _ins_ shared/non-shared instance resolution
phase: 08-parity-closure
feature: 36-di-merge-order-and-null-surface
owner: Unassigned
status: Ready
estimate: S
depends_on: [TKT-048, TKT-051]
touches:
  - rust/di-compiler/crates/di-resolver/src/arguments.rs
  - rust/di-compiler/crates/di-xml-reader/src/config.rs
acceptance:
  - tableStrategy and ~60 other arguments emit _ins_ (non-shared) instead of _i_ (shared) where PHP does
  - is_shared() correctly reflects shared="false" from all enabled di.xml files
test_plan:
  - Diagnose: grep for tableStrategy shared=false in vendor/magento/module-bundle/etc/di.xml
  - Unit test: type with shared="false" in type_configs → NonSharedInstance
  - Full compile + check_all_areas.php mismatches for _i_/_ins_ paths
---

# TKT-053: Fix _i_ vs _ins_ shared/non-shared instance resolution

## Scope

~60 argument paths emit `_i_` (SharedInstance) where PHP emits `_ins_` (NonSharedInstance). The decision is made by `is_shared(&concrete)` which checks `type_configs` for a `shared` attribute. If the `shared="false"` config lives in a di.xml file that `filter_enabled_di_xml` is dropping, `is_shared()` falls back to `true` (default) and we wrongly emit `_i_`.

**Depends on TKT-048 and TKT-051 first** — module-order fix and filter fix may resolve some of these automatically. Run diagnostics after those land before investing in additional fixes here.

## Implementation Notes

**Step 1 — Diagnose after TKT-048 + TKT-051:**
```bash
grep -rn "tableStrategy\|shared.*false\|false.*shared" \
  /var/www/application/vendor/magento/module-bundle --include="di.xml"
```
Check if the `shared="false"` config is in a file that survives the current filter, and whether the FQCN lookup in `is_shared()` uses the right normalized form.

**Step 2 — Check type hint vs concrete lookup:**
Currently, for constructor params without di.xml override, `is_shared()` is called on the **preference-resolved concrete** (not the raw type hint). If `shared="false"` is set on the interface (type hint) rather than the concrete class, we miss it.

Possible fix: also check `is_shared` on the raw type hint before preference resolution:
```rust
// In arguments.rs for constructor param resolution:
let type_shared = di_config.is_shared(&type_hint);
let concrete = di_config.get_preference(&type_hint);
let concrete_shared = di_config.is_shared(&concrete);
let is_shared = type_shared && concrete_shared; // both must be true for shared
```

## Risks

- Changing shared resolution for constructor params could affect many classes. Scope change against baseline carefully before landing.
- Some `_i_` → `_ins_` corrections may expose other downstream issues.
