# 19: Metadata PHP Serializer

- Category: Code Generation
- Status: Planned
- Implementation Phase: 04-code-generator
- Owner: Unassigned
- Feature ID: `metadata-php-serializer`
- Suggested Dependencies: 13-arguments-resolver

## Intent

Serialize Rust data structures to PHP `var_export()`-compatible array syntax.
This is used by both the area config generator and the plugin list generator.

## Output Format Contract

PHP's `var_export()` produces a specific whitespace/indentation format:
```php
<?php
return array (
  'key' =>
  array (
    'nested' => 'value',
    '_i_' => 'SomeClass',
  ),
);
```

**The exact format must be confirmed against ground truth in Phase 00.**
Differences in trailing commas, indentation (2 spaces), or newlines will cause diff failures.

## Core State and Actions

```rust
pub fn serialize_php_array(data: &PhpValue, indent: usize) -> String

pub enum PhpValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
    Array(Vec<(PhpKey, PhpValue)>),  // ordered map (PHP arrays are ordered)
}

pub enum PhpKey { String(String), Int(i64) }
```

## Acceptance Criteria

- Output is valid PHP (`php -r "include 'file.php';"` succeeds)
- Matches PHP `var_export()` whitespace exactly (verify against Phase 00 ground truth)
- Handles nested arrays of arbitrary depth
- Special chars in string values properly escaped
