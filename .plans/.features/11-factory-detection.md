# 11: Factory Detection

- Category: Resolution
- Status: Planned
- Implementation Phase: 03-di-resolver
- Owner: Unassigned
- Feature ID: `factory-detection`
- Suggested Dependencies: 06-extract-result-type, 09-di-config-model

## Intent

Determine which Factory classes need to be generated (they are referenced in constructors
but do not exist in the codebase).

## Detection Rule

A Factory needs generating if:
- A constructor param type hint ends in `Factory` (e.g. `ProductFactory`)
- AND that class FQN does not already exist in `class_map` (not on disk)

## Core State and Actions

```rust
pub fn detect_factories(
    class_map: &HashMap<String, ClassInfo>,
) -> Vec<FactorySpec>

pub struct FactorySpec {
    pub target_fqcn: String,    // the class the factory creates (strip "Factory" suffix)
    pub factory_fqcn: String,   // the full FQN of the factory class to generate
}
```

## Example

Constructor param: `ProductFactory $productFactory`
type_hint = `Magento\Catalog\Model\ProductFactory`
Does `Magento\Catalog\Model\ProductFactory` exist in class_map? No → generate it.
target_fqcn = `Magento\Catalog\Model\Product`
factory_fqcn = `Magento\Catalog\Model\ProductFactory`

## Acceptance Criteria

- Factory list matches PHP APPLICATION_CODE_GENERATOR output
- Already-existing Factory classes (in vendor/ or app/) are not re-generated
- Extension attribute classes also detected (confirm scope in Phase 00)
