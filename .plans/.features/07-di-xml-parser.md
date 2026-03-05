# 07: DI XML Parser

- Category: Configuration
- Status: Planned
- Implementation Phase: 02-di-xml-reader
- Owner: Unassigned
- Feature ID: `di-xml-parser`
- Suggested Dependencies: 01-workspace-scaffold

## Intent

Parse a single `di.xml` file into a `PartialDiConfig` struct using `quick-xml` SAX parsing.
Handle all `xsi:type` argument variants.

## Core State and Actions

```rust
pub fn parse_di_xml(path: &Path) -> Result<PartialDiConfig, DiXmlError>
```

**Input**: path to one `di.xml` file
**Output**: `PartialDiConfig { preferences, type_configs, virtual_types, source_path }`

### Elements to parse

| XML element | Maps to |
|-------------|---------|
| `<preference for="X" type="Y"/>` | `preferences["X"] = "Y"` |
| `<type name="X" shared="false">` | `type_configs["X"].shared = Some(false)` |
| `<virtualType name="X" type="Y">` | `virtual_types["X"] = VirtualType { instance_of: "Y", ... }` |
| `<plugin name="X" type="Y" sortOrder="N" disabled="true">` | `type_configs[parent].plugins.push(Plugin {...})` |
| `<argument name="X" xsi:type="object">Y</argument>` | `Argument::Object { name, value }` |

### xsi:type variants

| xsi:type | Rust variant |
|----------|-------------|
| `object` | `Argument::Object { name, value, shared }` |
| `string` | `Argument::String { name, value }` |
| `boolean` | `Argument::Boolean { name, value }` |
| `number` | `Argument::Number { name, value }` |
| `null` | `Argument::Null { name }` |
| `array` | `Argument::Array { name, items }` |
| `init_parameter` | `Argument::Init { name }` |
| `const` | `Argument::Const { name, value }` |

## Acceptance Criteria

- Parses all `di.xml` files in this Magento install without errors
- All `xsi:type` variants handled (verify counts match Phase 00 analysis)
- Array arguments with nested `<item>` elements parsed recursively
- Disabled plugins parsed correctly (`disabled="true"`)
