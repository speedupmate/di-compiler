# 08: DI XML Merger

- Category: Configuration
- Status: Planned
- Implementation Phase: 02-di-xml-reader
- Owner: Unassigned
- Feature ID: `di-xml-merger`
- Suggested Dependencies: 07-di-xml-parser

## Intent

Merge a list of `PartialDiConfig` structs (in Magento load order) into a single `DiConfig`
per area, following Magento's id-attribute merge rules.

## Core State and Actions

```rust
pub fn merge_di_configs(configs: Vec<PartialDiConfig>) -> DiConfig
```

**Merge rules:**

| Element | Merge behavior |
|---------|---------------|
| `preference` | Later entry wins (override) |
| `type` arguments | `array_replace`: later entry's arguments override earlier by name |
| `type` plugins | Accumulate (never override by name, unless same plugin name appears again) |
| `virtualType` | Later entry wins |
| `shared` attribute | Later entry wins |

**Load order (lower index = loaded first, later = overrides earlier):**
1. `vendor/magento/*/etc/di.xml`
2. `vendor/*/etc/di.xml` (non-magento vendors)
3. `app/etc/di.xml`
4. `app/code/*/etc/di.xml`
5. Area-specific variants of the above: `etc/{area}/di.xml`

## Acceptance Criteria

- Preferences resolved identically to PHP `Config::extend()` output
- Plugin lists accumulate in correct sort order
- VirtualType chains can be followed after merge
- Disabled plugins (`disabled="true"`) retained but flagged
