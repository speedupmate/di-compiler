# Parity Wave 3 — Root Cause Analysis (2026-03-09)

Current scores after Wave 2 fixes: **37 extra / 87–155 missing / 173–189 mismatches** per area.

Seven distinct root-cause patterns identified from `generated/diff/comparable_metadata/` reports.

---

## Pattern 2 — config.php HashMap ordering (172+ value mismatches) ← HIGHEST VALUE

### Symptom
Preferences resolve to wrong concrete class. Example: truth `Hyva\Checkout\Model\Config\Structure\Interceptor`, output `Magento\Config\Model\Config\Structure\Interceptor`. Affects ~20+ `configStructure` arguments and other Customer model arguments.

### Root cause
`enabled_modules` is built by `config_modules.iter().enumerate()` where `config_modules` is a Rust `HashMap`. HashMap iterates in arbitrary hash order, not config.php insertion order. So the module-order index assigned to each module is random, making the `(priority, module_order_index, path)` sort key meaningless noise.

Verified: `Magento_Config` is at position 21 in config.php; Hyva modules are at positions 377–382. With a correct index, Hyva preferences override Magento core ones (last write wins in merge). With a random index, the outcome is undefined.

### Fix pointer
`crates/cli/src/main.rs` — `parse_config_php` must return `Vec<(String, i64)>` preserving insertion order, not `HashMap`. See TKT-048.

---

## Pattern 4 — Interface entries in plugin-list Section 1 (6 high-risk extra)

### Symptom
Report shows `[1].Magento\Framework\App\Action\HttpGetActionInterface [NULL → object]` — PHP has NULL (absent) but Rust outputs a plugin array.

### Root cause confirmed
Verified against `/var/www/application/generated/_metadata/pluginList.php` Section 1: all 6 interfaces (`HttpGetActionInterface`, `HttpPostActionInterface`, `HttpHeadActionInterface`, `OrderInterface`, `IndexInterface`, `CsrfAwareActionInterface`) are **NOT PRESENT** in PHP output.

PHP does not propagate plugins to sub-interfaces via interface→interface inheritance. It only propagates to concrete implementors. Our `inherit_plugins()` walks the full extends+implements chain without checking ClassKind, so interfaces that extend plugin-bearing parent interfaces incorrectly accumulate those plugins in Section 1.

### Fix pointer
`crates/code-generator/src/plugin_list.rs` — skip pure interfaces (ClassKind::Interface) from Section 1 unless they have plugins directly registered in Section 0 (plugin_data). See TKT-049.

---

## Pattern 6 — Plugin sort-order ties (14+ mismatches in Section 2)

### Symptom
`[2].Magento\Quote\Model\QuoteManagement_submit___self.1` — truth `persistent_convert_customer_cart_to_guest_cart`, output `validate_purchase_order_number`. Plugins are in wrong order.

### Root cause
`sorted_plugins.sort_by(|a, b| a.sort_order.cmp(&b.sort_order))` — single-key sort. Plugins with the same `sort_order` produce non-deterministic ordering. PHP uses a stable secondary sort by plugin name for tie-breaking.

### Fix pointer
`crates/code-generator/src/plugin_list.rs` — add `.then(a.name.cmp(&b.name))` secondary key. See TKT-050.

---

## Pattern 5 — Missing disabled plugin entries (47 missing from Section 0)

### Symptom
Missing paths of the form `[0].Magento\CatalogImportExport\Model\StockItemImporterInterface.update_bundle_products_stock_item_status.disabled`. PHP Section 0 has explicit disabled plugin entries that we don't emit.

### Root cause hypothesis
`filter_enabled_di_xml` (added to reduce test-framework noise) drops vendor packages with no `module.xml` AND no `registration.php`. Some of these packages contain `<plugin disabled="true">` entries that PHP's compiler does process. The disabled plugins are in di.xml files we're silently dropping.

### Diagnostic
```bash
grep -r "update_bundle_products_stock_item_status\|stockedProductsFilterPlugin\|updateStockChangedAuto" \
  /var/www/application/vendor /var/www/application/app --include="di.xml" -l
```
Check if those di.xml files survive `filter_enabled_di_xml`.

### Fix pointer
`crates/cli/src/main.rs` — relax `filter_enabled_di_xml` for packages that own disabled plugin entries. See TKT-051.

---

## Pattern 7 — Missing class entries (63–131 missing per area)

### Symptom
Entire argument sections missing for classes like `Magento\CatalogSearch\Block\SearchResult\ListProduct`, `Magento\CatalogSearch\Model\Session`, Hyva checkout classes, some GraphQL types.

### Two sub-causes

**A) di.xml-only types** — PHP emits `'ClassName' => NULL` for types in `<type name="...">` entries even when the PHP file can't be found on disk. Our code requires `base_class_fqcns` membership (source PHP scan) to emit NULL, so di.xml-configured types we never found as files are silently dropped.

**B) Scan coverage gaps** — packages without `registration.php` are not walked. Composer `autoload_psr4.php` has 614 namespace→directory mappings that cover all composer packages including those without `registration.php`. TKT-035 (In Progress) adds this PSR-4 seed.

### Why no PHP worker calls are needed for FQCN scan
FQCNs are derivable from path with pure string math: `Namespace\Prefix\` + `path/to/Class.php` → `Namespace\Prefix\Path\To\Class`. The existing PHP worker pool (`crates/cli/src/main.rs:314`) handles only ClassKind disambiguation for files where the Rust lexer is ambiguous. Timing: 42,538 PHP files found via `find` in ~0.67s; PSR-4 derivation adds ~0ms.

### Fix pointer
- PSR-4 scan: TKT-035 (In Progress) — `crates/php-extractor/src/walker.rs`
- di.xml-only NULL emission: TKT-052 — `crates/di-resolver/src/arguments.rs`

---

## Pattern 1 — _i_ vs _ins_ mismatch (~60+ paths)

### Symptom
`arguments.Magento\Bundle\Model\ResourceModel\Indexer\Stock.tableStrategy._ins_` is missing; `_i_` is emitted instead. PHP emits `_ins_` (non-shared instance) but we emit `_i_` (shared instance).

### Root cause hypothesis
`is_shared()` correctly normalizes FQCN and looks up `type_configs`. The mismatch implies the class has `<type name="..." shared="false">` in a di.xml file that we're currently filtering out (same filter issue as Pattern 5), OR the `shared="false"` config is on the type hint interface rather than the resolved concrete class.

### Diagnostic
```bash
grep -rn "tableStrategy\|shared.*false\|false.*shared" \
  /var/www/application/vendor/magento/module-bundle --include="di.xml"
```

### Fix pointer
After diagnosis: either unfilter the relevant di.xml, or check `is_shared` on the type hint before preference resolution. See TKT-053.

---

## Pattern 3 — PHP extension constants not in const_map (4 paths)

### Symptom
`truth="blowfish", output="MCRYPT_BLOWFISH"` — PHP constant name emitted as literal string instead of its value.

### Root cause
`MCRYPT_BLOWFISH` → `"blowfish"`, `MCRYPT_MODE_ECB` → `"ecb"` are PHP mcrypt extension built-in constants. They're not defined in any Magento PHP file, so they never enter the `const_map` (which is built by scanning PHP source only).

### Fix pointer
`crates/cli/src/main.rs` — run `php -r 'echo json_encode(array_map("strval", call_user_func_array("array_merge", array_values(get_defined_constants(true)))));'` once at startup to bootstrap const_map with all PHP runtime constants. Source-scan constants added after and win on collision. See TKT-054.

---

## Execution order

1. TKT-048 (Pattern 2) — largest single-ticket impact: 172+ mismatches
2. TKT-049 (Pattern 4) — removes 6 high-risk extra entries
3. TKT-050 (Pattern 6) — 14+ sort mismatches
4. TKT-051 (Pattern 5) — diagnose + fix disabled plugin filter
5. TKT-052 (Pattern 7B) — di.xml-only NULL emission; depends on TKT-035 completing Pattern 7A
6. TKT-053 (Pattern 1) — after diagnosis
7. TKT-054 (Pattern 3) — PHP constant bootstrap
