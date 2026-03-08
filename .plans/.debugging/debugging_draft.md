# Debugging Loop — `bin/magento` Class Not Found (VT Chain Resolution)

## The Error

```
There is an error in AbstractFactory.php at line: 121
Class "Magento\Eav\Model\Api\SearchCriteria\CollectionProcessor" not found
#0 AbstractFactory.php(121): createObject('Magento\Eav\...', Array)
#1 Compiled.php(108): create('Magento\Catalog\...')
#2 Compiled.php(79): get('Magento\Catalog\...')
...
```

Thrown by `new $type(...)` where `$type` was a virtual type name, not a real PHP class.

---

## How the ObjectManager Works in Compiled Mode

When `generated/metadata/global.php` exists, `EnvironmentFactory::getMode()` returns
`'compiled'` regardless of `MAGE_MODE`. This means `Factory\Compiled` and
`Config\Compiled` are used instead of the runtime equivalents.

**`Factory\Compiled::create($requestedType)`:**
```php
$args = $this->config->getArguments($requestedType); // from generated/metadata/*.php
$type = $this->config->getInstanceType($requestedType); // VT → concrete lookup
// ...process args...
return $this->createObject($type, $args); // new $type(...$args)
```

**`Config\Compiled::getInstanceType($name)`:**
```php
if (isset($this->virtualTypes[$name])) return $this->virtualTypes[$name];
return $name; // only ONE hop — does NOT follow chains
```

**`Config\Config::getInstanceType($name)` (runtime version, NOT used in compiled mode):**
```php
while (isset($this->_virtualTypes[$name])) {
    $name = $this->_virtualTypes[$name]; // follows full chain
}
return $name;
```

---

## The Bug

Magento's `di.xml` has chained virtual types. Example:

```
ProductCollectionProcessor (VT) → EavCollectionProcessor (VT) → CollectionProcessor (real class)
```

`instanceTypes` in compiled metadata stored this as:
```
ProductCollectionProcessor => EavCollectionProcessor   ← still a VT!
EavCollectionProcessor     => CollectionProcessor      ← correct
```

When `create('ProductCollectionProcessor')` runs:
1. `getInstanceType('ProductCollectionProcessor')` → `'EavCollectionProcessor'` (one hop)
2. `createObject('EavCollectionProcessor', $args)` → `new EavCollectionProcessor(...)` → **FAIL**

`EavCollectionProcessor` is not a real PHP class — it's a virtual type name only.

PHP's own `setup:di:compile` had the same bug: **104 entries** in `instanceTypes` were
VT→VT instead of VT→Concrete. The bug was invisible in developer mode because the
runtime `Config\Config::getInstanceType` uses a `while` loop. It only surfaces when
compiled metadata is active.

---

## Diagnosis Steps

1. **Run `bin/magento list`** → get stack trace with failing class name
2. **Read `Factory\Compiled`** ([Compiled.php:108](../../vendor/magento/framework/ObjectManager/Factory/Compiled.php#L108)) →
   saw `createObject($type, $args)` where `$type` comes from `getInstanceType`
3. **Read `Config\Compiled::getInstanceType`** ([Compiled.php:109](../../vendor/magento/framework/ObjectManager/Config/Compiled.php#L109)) →
   spotted single-hop: `if (isset(...)) return ...; return $name;`
4. **Check `generated/metadata/global.php`** → VT IS in `instanceTypes` (data is there,
   but value is another VT)
5. **Read `Config\Config::getInstanceType`** (runtime) → has `while` loop, confirms
   Magento knows about chains but only handles them at runtime
6. **Verify PHP baseline** (`/tmp/di-rust-output`) has same 104 VT-of-VT entries →
   confirmed pre-existing PHP bug, not something we introduced
7. **Apply fix in Rust compiler** — resolve chains at compile time instead

---

## The Fix

In [`crates/code-generator/src/area_config.rs`](../../crates/code-generator/src/area_config.rs),
when writing `instanceTypes`, follow the VT chain to the final concrete:

```rust
// Before (one hop — matches PHP's buggy output):
let direct_type = &di_config.virtual_types[name].type_name;

// After (full chain — correct):
let mut concrete = di_config.virtual_types[name].type_name.as_str();
let mut steps = 0;
while let Some(vt) = di_config.virtual_types.get(concrete) {
    concrete = vt.type_name.as_str();
    steps += 1;
    if steps > 64 { break; } // guard against cycles
}
// write `concrete` — guaranteed to be a real class name
```

**Result:** 0 VT-of-VT entries in our output vs 104 in PHP's. `bin/magento list` works.

---

## Key Insight

> The same data (`instanceTypes`) is read by both:
> - `Config\Compiled::getInstanceType` (compiled mode, one hop only)
> - `Config\Config::getInstanceType` (developer mode, follows chain)
>
> PHP's `setup:di:compile` was always generating data that only worked correctly
> in developer mode. The Rust compiler now generates data that works correctly
> in both modes by fully resolving chains at compile time.

---

## Commit

`619d5f1` — fix: resolve VT chains in instanceTypes + inherited constructor reflection
