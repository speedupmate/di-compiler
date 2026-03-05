# 13: Arguments Resolver

- Category: Resolution
- Status: Planned
- Implementation Phase: 03-di-resolver
- Owner: Unassigned
- Feature ID: `arguments-resolver`
- Suggested Dependencies: 09-di-config-model, 06-extract-result-type

## Intent

Map each class's constructor parameters to Magento's resolved argument notation.
This is the core of the DI graph — it produces the `constructor_map` that goes into
`generated/metadata/*.php`.

Replicates `setup/src/Magento/Setup/Module/Di/Compiler/ArgumentsResolver.php`.

## Notation Rules

| Case | Notation |
|------|----------|
| Typed param, shared instance | `['_i_' => 'FullyQualifiedClassName']` |
| Typed param, non-shared (`shared="false"`) | `['_ins_' => 'FullyQualifiedClassName']` |
| Optional scalar with default | `['_v_' => defaultValue]` |
| Optional with null default | `['_vn_' => true]` |
| di.xml `<argument xsi:type="array">` | `['_vac_' => [...]]` |
| Global argument reference | `['_a_' => 'argName', '_d_' => defaultValue]` |

## Resolution Order

1. Start with constructor param type hint
2. Follow preference chain: `interface → concrete`
3. Follow virtualType chain: `virtualName → concrete`
4. Check `di.xml` `<argument>` override for this param name (overrides everything)
5. Check parent class arguments (inherited)

## Core State and Actions

```rust
pub fn resolve_arguments(
    class_fqcn: &str,
    constructor: &Constructor,
    config: &DiConfig,
) -> Vec<ResolvedParam>

pub struct ResolvedParam {
    pub name: String,
    pub notation: ArgumentNotation,
}

pub enum ArgumentNotation {
    SharedInstance(String),
    NonSharedInstance(String),
    Value(serde_json::Value),
    NullValue,
    ArrayValue(Vec<serde_json::Value>),
    GlobalArg { name: String, default: serde_json::Value },
}
```

## Acceptance Criteria

- Output matches PHP `ArgumentsResolver` for 100-class validation sample
- Preference and virtualType chains followed correctly
- di.xml `<argument>` overrides reflection-based defaults
- Scalar/primitive params (string, int, bool, array) → `_v_` notation
