---
id: TKT-021
title: Area config code generator
phase: 04-code-generator
feature: area-config-codegen
owner: Unassigned
status: Ready
estimate: M
depends_on: [TKT-015]
touches:
  - rust/di-compiler/crates/code-generator/src/area_config.rs
acceptance:
  - generated/metadata/{global,frontend,adminhtml,...}.php match PHP output byte-for-byte
  - All areas from Phase 00 analysis covered
---

# TKT-021: Area Config Code Generator

## Scope

Write `generated/metadata/{area}.php` files for each area.

## Implementation Notes

```rust
pub fn generate_area_configs(
    areas: &[&str],
    resolved_by_area: &HashMap<String, ResolvedGraph>,
    serializer: &PhpSerializer,
) -> Vec<(String, String)>   // (filename, content)
```

Areas: `global`, `frontend`, `adminhtml`, `webapi_rest`, `webapi_soap`, `cron` — confirm full list in Phase 00.

Each file: `<?php\nreturn {serialized PHP array};\n`

Use TKT-016's serializer for the array content.

## Risks

- Must confirm exact list of areas from Phase 00 (which areas have area-specific di.xml?)
