# Metadata Debugging Pattern — Diff Comparison + Reflection Oracle

## The Core Problem

`generated/metadata/*.php` files are large PHP arrays with nested structure.
A raw `diff` shows too much noise: key-ordering differences (BTreeMap vs PHP insertion
order) obscure the actual content differences we care about.

---

## Step 1: Make Diffs Comparable

Before you can compare content, you need to normalize key ordering out of the picture.
The pattern we used was to write PHP scripts that load both files, then compare
structured data directly — not text.

### Canonical comparison script shape

```php
<?php
// Load both
$truth = include '/var/www/application/generated/metadata/global.php'; // PHP baseline
$ours  = include '/tmp/di-rust-output/metadata/global.php';            // Rust output

// ---- arguments ----
$missing = array_diff_key($truth['arguments'], $ours['arguments']);
$extra   = array_diff_key($ours['arguments'], $truth['arguments']);

$wrong = [];
foreach ($truth['arguments'] as $fqcn => $t_args) {
    if (!isset($ours['arguments'][$fqcn])) continue;
    if ($t_args !== $ours['arguments'][$fqcn]) {
        $wrong[$fqcn] = ['truth' => $t_args, 'ours' => $ours['arguments'][$fqcn]];
    }
}

echo "arguments: missing=" . count($missing) . " extra=" . count($extra) . " wrong=" . count($wrong) . "\n";

// ---- preferences ----
$missing_p = array_diff_key($truth['preferences'], $ours['preferences']);
$extra_p   = array_diff_key($ours['preferences'], $truth['preferences']);
$wrong_p   = array_filter($truth['preferences'],
    fn($v, $k) => isset($ours['preferences'][$k]) && $ours['preferences'][$k] !== $v, ARRAY_FILTER_USE_BOTH);

echo "preferences: missing=" . count($missing_p) . " extra=" . count($extra_p) . " wrong=" . count($wrong_p) . "\n";

// ---- instanceTypes ----
$missing_i = array_diff_key($truth['instanceTypes'], $ours['instanceTypes']);
$extra_i   = array_diff_key($ours['instanceTypes'], $truth['instanceTypes']);
$wrong_i   = array_filter($truth['instanceTypes'],
    fn($v, $k) => isset($ours['instanceTypes'][$k]) && $ours['instanceTypes'][$k] !== $v, ARRAY_FILTER_USE_BOTH);

echo "instanceTypes: missing=" . count($missing_i) . " extra=" . count($extra_i) . " wrong=" . count($wrong_i) . "\n";
```

**Key principle:** compare with `!==` on structured data, not text diff.
Key ordering is irrelevant; content is what matters.

### The iteration loop

```
1. Make a Rust code change
2. cargo build --release 2>&1 | tail -3
3. fast-di-compile --magento-root /var/www/application --output /tmp/di-rust-output
4. Run the PHP comparison script → get missing/extra/wrong counts
5. Drill into specific wrong entries → understand root cause
6. Goto 1
```

The counts become the scoreboard. Target: `missing=0 extra=0 wrong=0` for all three sections.

---

## Step 2: Drill Into Specific Cases

When `wrong` count is nonzero, drill into the specific entries:

```php
<?php
// Show first N wrong argument entries
$shown = 0;
foreach ($truth['arguments'] as $fqcn => $t_args) {
    if (!isset($ours['arguments'][$fqcn])) continue;
    if ($t_args === $ours['arguments'][$fqcn]) continue;
    if ($shown++ >= 5) break;

    echo "$fqcn:\n";
    foreach ($t_args as $param => $tv) {
        $ov = $ours['arguments'][$fqcn][$param] ?? null;
        if ($tv !== $ov) {
            echo "  $param:\n";
            echo "    truth: " . json_encode($tv) . "\n";
            echo "    ours:  " . json_encode($ov) . "\n";
        }
    }
}
```

This tells you exactly which class, which constructor param, and what the shape difference is.

---

## Step 3: Reflection as a Diagnostic Oracle

### What reflection is good for in this context

PHP reflection is NOT the primary metadata engine — DI merge semantics, VT
resolution, `_a_`/`_d_`/`_vac_` encoding rules all need to live in Rust.

But reflection is a fast **oracle** for specific failing cases:

| Use reflection for | Don't use reflection for |
|---|---|
| Normalizing tricky constructor defaults your extractor can't represent | DI merge semantics (virtualType chaining/overrides) |
| Validating that referenced classes are actually loadable | `_a_`/`_d_`, `_vac_` encoding shape rules |
| Spot-checking generated argument payloads vs runtime expectations | instanceTypes/preference mapping correctness |
| Diagnosing missing constructor params (inherited from outside scan scope) | Core argument resolution logic |

### Reflection-as-oracle pattern

When you have a class with a wrong or missing argument payload:

```bash
# Reflect the class's constructor at runtime
php -r "
require '/var/www/application/app/bootstrap.php';
\$r = new ReflectionClass('Magento\\\\Some\\\\Class');
\$c = \$r->getConstructor();
foreach (\$c->getParameters() as \$p) {
    \$t = \$p->getType();
    echo \$p->getName() . ': ' . (\$t ? \$t->getName() : 'none');
    echo ', optional=' . (\$p->isOptional() ? 'true' : 'false');
    if (\$p->isDefaultValueAvailable()) {
        echo ', default=' . var_export(\$p->getDefaultValue(), true);
    }
    echo '\n';
}
" 2>/dev/null
```

This tells you what PHP actually sees for that class at runtime — the ground truth
for constructor shape. Compare against what your Rust resolver produced.

### The practical debugging loop with reflection

```
1. Identify failing metadata key/class (from diff script)
2. Reflect constructor/method signature for that class:
      php -r "require 'bootstrap'; $r = new ReflectionClass('...')..."
3. Compare reflected shape vs generated metadata payload
4. Identify which Rust rule is wrong (resolver? merger? serializer?)
5. Patch the rule
6. Add regression test for that exact pattern
7. Run diff script → confirm count drops
```

This gives fast root-cause isolation without making reflection the primary path.

---

## Step 4: What the Rust Compiler Actually Uses for Metadata

Understanding which code path produces each metadata section prevents confusion
about where to apply fixes.

```
generated/metadata/*.php
├── arguments      ← resolve_all_arguments_for_named_types() [main.rs:~49]
│                    Uses: merged DI config (di.xml args + inheritance)
│                          class_map constructor signatures
│                          PHP reflection for edge cases (inherited ctors, defaults)
│
├── preferences    ← generate_area_config_with_extra_preferences() [main.rs:~28]
│                    Uses: merged DI config preferences
│                          interception_preferences (VT→Interceptor mappings)
│
└── instanceTypes  ← area_config.rs (instanceTypes section)
                     Uses: merged DI config virtual_types
                           NOW: fully resolves VT chains to final concrete
```

**PHP reflection contributes selectively:**
- `enrich_interceptor_specs_with_reflection()` [main.rs:~2123] — interceptor/proxy method enrichment
- `enrich_constructor_defaults_with_reflection()` [main.rs:~1400, called ~1055] — constructor default enrichment before metadata emission
- `enrich_inherited_constructors_with_reflection()` [main.rs:~1449] — classes that inherit constructors from outside scan scope

Reflection is not the metadata engine. It fills edge-case gaps the extractor misses.

---

## When to Add Reflection for a New Edge Case

If the diff script shows a class with wrong/missing argument data, and:

1. The class exists in the codebase but has no constructor in Rust's class map
2. The class extends something outside the scan scope (PHP built-in, third-party lib)
3. The constructor has a default value that can't be parsed statically (e.g. a constant
   expression, a class constant, a `new` expression)

→ That's a reflection candidate. Add it to one of the enrichment passes or extend an
existing one.

If the class IS in the class map but the argument resolution is wrong, the bug is in
Rust DI resolver/merger logic — reflection won't help, fix the rule.

---

## Summary

```
Raw diff     → too noisy (key ordering)
PHP compare  → shows missing/extra/wrong counts by section
Drill-in     → shows exact class + param + shape mismatch
Reflection   → oracle for "what should this actually be?"
Rust fix     → resolver / merger / serializer rule
Regression   → test for that exact pattern
```

The counts are your scoreboard. Every fix should move at least one number to zero.

---

## Step 5: Type-Shape Content Diff (Critical Runtime Guard)

### Why this matters

A metadata entry can be "present" and still be wrong at runtime if the value type
is wrong.

Example incident:

- `Magento\Store\Model\StoreResolver\ReaderList::resolverMap` was emitted as:
  - `'_v_' => 'array (...)'` (string)
- Magento expects:
  - `'_v_' => [ ... ]` (actual PHP array)

This produced frontend runtime failure:

`TypeError: Cannot access offset of type string on string`  
at `vendor/magento/module-store/Model/StoreResolver/ReaderList.php:50`

So key existence parity is not enough. We need **shape parity** (type + structure).

### Fast detection rules

1. Hard-fail any stringified array in scalar slot:
   - `'_v_' => 'array ('...`
2. Hard-fail scalar/array type mismatch at the same key path between truth and rust.
3. Keep ignoring map key order, but never ignore value type differences.

### Minimal check script pattern (shape-focused)

```php
<?php
$truth = include '/var/www/application/generated/_metadata/global.php';
$ours  = include '/var/www/application/generated/metadata/global.php';

$issues = [];

$walk = function ($t, $o, $path = '') use (&$walk, &$issues) {
    if (gettype($t) !== gettype($o)) {
        $issues[] = "TYPE_MISMATCH $path truth=" . gettype($t) . " ours=" . gettype($o);
        return;
    }
    if (!is_array($t)) return;

    foreach ($t as $k => $tv) {
        if (!array_key_exists($k, $o)) continue;
        $ov = $o[$k];
        $kp = $path === '' ? (string)$k : "$path.$k";
        $walk($tv, $ov, $kp);
    }
};

$walk($truth, $ours);
foreach ($issues as $i) echo $i, "\n";
```

### Known high-risk symptom classes

When this check fails, first validate these runtime-sensitive paths:

- `StoreResolver` / scope resolution (`ReaderList::resolverMap`)
- `ConfigSourceAggregated` source maps (`sortOrder` + `source` tuples)
- command/config lists where many modules merge array values

### Required smoke checks after metadata changes

After each rebuild, run:

1. `bin/magento list` (CLI bootstrap + command registry)
2. one frontend bootstrap hit (store/scope resolver + config resolver path)

These catch shape regressions that pure key-count diffs miss.

### Round 1 Results (2026-03-08, before numeric fixes)

Type-aware comparison across all metadata files (`generated/_metadata` vs `generated/metadata`):

- `string|integer`: 476
- `integer|string`: 6
- `NULL|array`: 51
- `array|string` or `string|array`: 0

### Round 2 Results (2026-03-08, after numeric type + comment-strip fixes)

- `string|integer`: **0** ✓
- `integer|string`: **0** ✓
- `NULL|array`: **51** (unchanged, functionally harmless — see below)
- `array|string` or `string|array`: **0** ✓

### Fixes applied

**string|integer (476 → 0)**: `xsi:type="number"` in di.xml stored as string.
PHP's `Number::evaluate()` returns raw XML text string (not cast to int/float).
`var_export()` then quotes it. Fix: di-xml-reader parser now emits `xsi:type="number"`
as `Argument::String` (not `Argument::Number`). PHP constants that resolve to integers
still use `Argument::Number` via constant-resolution path, remaining unquoted.

**integer|string (6 → 0)**: Constructor default `3145728 // 3mb` — tier-1 lexer
captured PHP line comment as part of default value string. Fix: `read_default_value`
breaks at `//` or `#`, leaving pos at comment start for outer loop's `skip_noise`.

### Remaining: NULL|array (51, plugin-list section 1)

Pattern: truth has `null`, ours has array of plugins for interface/disabled-plugin types.

Affected types fall into two categories:
1. **Marker interfaces** (`HttpPostActionInterface`, `HttpGetActionInterface`,
   `HttpHeadActionInterface`, `CsrfAwareActionInterface`, `OrderInterface`,
   `WishlistController\IndexInterface`) — extend `ActionInterface` which has plugins.
   PHP stores null because compiled Relations has no entry for interfaces; our code
   walks the class map and inherits plugins from `ActionInterface`.
2. **Abstract collection** types (`Eav/Catalog/Sales AbstractCollection`) in scopes
   where `currentPageDetection` plugin is disabled — PHP strips disabled plugins from
   return value of `inheritPlugins`, making children resolve to null; our code includes
   disabled plugins in the inherited return value.

**Runtime impact**: None expected. Plugin lookup at runtime uses concrete class names
(never interface names). Concrete classes that implement these interfaces have their own
correct entries in section 1. The interface entries are never looked up by `getNext()`.

**Root cause** (for reference, not yet fixed):
- PHP `PluginListGenerator::inheritPlugins()` returns `$plugins` AFTER `unset`ing
  disabled entries (line 342), so caller sees empty array → children inherit nothing.
- Compiled Relations doesn't include interfaces → `relations->has(interface)` = false
  → interface resolves via `pluginData[$type]` only (no parent traversal).
  PHP DOES visit interfaces (they appear as parents of concrete types in `getParents()`),
  but since `relations->has(interface)` = false in some contexts, they get `null`.
