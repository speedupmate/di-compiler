# 18: Area Config Code Generator

- Category: Code Generation
- Status: Planned
- Implementation Phase: 04-code-generator
- Owner: Unassigned
- Feature ID: `area-config-codegen`
- Suggested Dependencies: 13-arguments-resolver

## Intent

Generate per-area metadata files: `generated/metadata/{global,frontend,adminhtml,...}.php`.
Each file is a PHP array containing the resolved DI config for that area (preferences + arguments).

## Core State and Actions

```rust
pub fn generate_area_config(area: &str, config: &DiConfig, resolved: &ResolvedGraph) -> String
```

Output format:
```php
<?php
return array (
  'preferences' =>
  array (
    'Psr\\Log\\LoggerInterface' => 'Magento\\Framework\\Logger\\Monolog',
    ...
  ),
  'Magento\\SomeClass' =>
  array (
    'arguments' =>
    array (
      'dep' => array ( '_i_' => 'Magento\\SomeDep' ),
    ),
  ),
  ...
);
```

## Areas to Generate

- `global` (always)
- `frontend`, `adminhtml`, `webapi_rest`, `webapi_soap`, `cron` (if area-specific di.xml exists)

## Acceptance Criteria

- Each `generated/metadata/{area}.php` matches PHP output byte-for-byte
- PHP `var_export()` format replicated exactly (confirm whitespace in Phase 00)
- All areas from Phase 00 analysis covered
