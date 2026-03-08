# Interface Argument Inheritance — Root Cause & Fix

## The Problem

80 commands were missing from `bin/magento list` in compiled mode vs developer mode.

Tracing the root cause:
- Commands (indexer, cache, deploy, etc.) are registered via `di.xml` on `CommandListInterface`:
  ```xml
  <type name="Magento\Framework\Console\CommandListInterface">
      <arguments>
          <argument name="commands" xsi:type="array">
              <item name="reindex" xsi:type="object">Magento\Indexer\Console\Command\IndexerReindexCommand</item>
              ...
          </argument>
      </arguments>
  </type>
  ```
- `CommandList` is the preference for `CommandListInterface`
- `CommandList` implements `CommandListInterface`
- Our Rust argument resolver only walked the PHP `extends` chain — NOT `implements`
- Result: `CommandList.commands` resolved as `{_v_: []}` (empty), ignoring the interface-level di.xml args

## PHP's Mechanism

PHP's `Config::_collectConfiguration` uses `ClassReader::getParents` which returns:
- For class with parent: `[parentClass, ...interfaces_not_in_parent]`
- For class without parent but with interfaces: `[null, interface1, interface2, ...]`
- For class with neither: `[]`

It recursively collects arguments from each relation — including interfaces. Interface arguments
have **lower priority** than the class's own arguments (interface args form the base, class args
override via `array_replace_recursive`).

## The Fix

In `crates/di-resolver/src/arguments.rs`, `merged_di_arguments_for_type_name`:

```rust
// Before: only walked extends chain
let mut class_hierarchy: Vec<String> = extends_chain;

// After: for each class in extends chain, also insert "new" interfaces
// (not inherited from parent) before the class, mirroring PHP's getParents behavior
let mut class_hierarchy: Vec<String> = Vec::new();
let mut parent_implements: HashSet<String> = HashSet::new();
for class_name in &extends_chain {
    if let Some(info) = class_map.get(class_name) {
        let class_implements: HashSet<String> =
            info.implements.iter().map(|i| normalize(i)).collect();
        let mut new_interfaces: Vec<String> = class_implements
            .difference(&parent_implements)
            .cloned()
            .collect();
        new_interfaces.sort(); // stable ordering
        class_hierarchy.extend(new_interfaces); // interfaces BEFORE class → lower priority
        parent_implements = class_implements;
    }
    class_hierarchy.push(class_name.clone());
}
```

Interfaces are inserted **before** the class in the merge order, so the class's own args
override them (higher priority), matching PHP semantics.

## Impact

Before any fix: 62 commands in compiled mode (115 missing vs developer mode's 177)
After interface hierarchy fix (Part 1): 73 commands (hierarchy correct, but array merge wrong)
After array merge fix (Part 2): **177 commands — exact match with developer mode**

## Two-Part Fix

### Part 1 — Interface hierarchy (extends class hierarchy to include implements)
See code in The Fix section above. Adds `CommandListInterface` etc. to the type chain.

### Part 2 — Recursive array merge in type hierarchy resolution

`merged_di_arguments_for_type_name` was replacing same-name args instead of recursively
merging array items. With interface hierarchy active, this caused the class's own 3-item
`commands` array (from encryption-key module) to overwrite the interface's 103-item
`commands` array accumulated from all modules.

```rust
// Before: simple replacement
if let Some(idx) = by_name.get(&name).copied() {
    merged[idx] = arg;  // WRONG for arrays: discards accumulated items
}

// After: recursive array merge
if let Some(idx) = by_name.get(&name).copied() {
    merge_argument_into(&mut merged[idx], arg);  // array items additive, non-arrays replaced
}
```

`merge_argument_into` mirrors PHP's `array_replace_recursive`:
- Arrays: merge items by key (new key → add, existing key → recurse)
- Other types: src replaces dst

Also required changing return type from `Vec<&'a Argument>` (borrows) to `Vec<Argument>` (owned
clones) to allow in-place mutation during the merge pass.

## Confirmed by PHP Runtime

Verified that multiple classes where old baseline had wrong values now match PHP runtime:

| Class | Old baseline | New (correct) | PHP runtime |
|---|---|---|---|
| `AdobeStockClient\Model\Client.searchParametersProvider` | `{_i_: SearchParameterProviderInterface}` | `{_i_: SearchParametersProviderComposite}` | `{_i_: SearchParametersProviderComposite}` |
| `DefaultPrice.priceModifiers` | `{_v_: []}` | `{_vac_: {...}}` | `{_vac_: {...}}` |
| `CrontabManager.shell` | `{_i_: ShellInterface}` | `{_i_: App\Shell}` | `{_i_: App\Shell}` |
| `TasksProvider.tasks` | `{_v_: []}` | `{_v_: {cronMagento: ...}}` | `{_v_: {cronMagento: ...}}` |
| `SearchCriteriaResolverChain.resolvers` | `{_vn_: true}` | `{_vac_: {...}}` | `{_vac_: {...}}` |
| `GetEntities.entities` | `{_v_: []}` | `{_v_: {catalog_product: ...}}` | `{_v_: {catalog_product: ...}}` |

## Key Insight from Magento DI Architecture (FireBear article)

The Magento 2 DI system treats interfaces as first-class configuration targets. Modules
declare arguments on interfaces (`di.xml <type name="SomeInterface">`), not on concretes,
to ensure loose coupling. The ObjectManager merges interface-level args into the preference
concrete at resolution time.

This is intentional DI design: **interface = contract, concrete = implementation**.
The di.xml arguments registered on `SomeInterface` should be visible to all classes
that are preferences for (or implement) that interface.

---

## Diagnostic Commands

```bash
# Verify PHP runtime sees correct args (ground truth)
php -r "
require '/var/www/application/app/bootstrap.php';
\$bootstrap = \Magento\Framework\App\Bootstrap::create(BP, []);
\$om = \$bootstrap->getObjectManager();
\$config = \$om->get('Magento\Framework\ObjectManager\ConfigInterface');
echo json_encode(\$config->getArguments('Magento\Framework\Console\CommandList')) . \"\n\";
" 2>/dev/null | python3 -m json.tool

# Count commands: compiled vs dev mode
bin/magento list 2>&1 | grep '^\s' | wc -l         # compiled mode
MAGE_MODE=developer php bin/magento list 2>&1 | grep '^\s' | wc -l  # dev mode
```

## PHP Reference Code

`Config::_collectConfiguration` (vendor/magento/framework/ObjectManager/Config/Config.php):
```php
} elseif ($this->_relations->has($type)) {
    $relations = $this->_relations->getParents($type);
    $arguments = [];
    foreach ($relations as $relation) {
        if ($relation) {
            $relationArguments = $this->_collectConfiguration($relation);
            if ($relationArguments) {
                $arguments = array_replace($arguments, $relationArguments);
            }
        }
    }
}
if (isset($this->_arguments[$type])) {
    $arguments = array_replace_recursive($arguments, $this->_arguments[$type]);
}
```

`ClassReader::getParents` (vendor/magento/framework/Code/Reader/ClassReader.php):
```php
if ($parentClass) {
    $interfaces = class_implements($className);
    $parentInterfaces = class_implements($parentClass);
    $result = $parentInterfaces
        ? array_values(array_diff($interfaces, $parentInterfaces))
        : array_values($interfaces);
    array_unshift($result, $parentClass);
} else {
    $result = array_values(class_implements($className));
    if ($result) array_unshift($result, null);
}
```
