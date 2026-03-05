---
id: TKT-016
title: Metadata PHP serializer
phase: 04-code-generator
feature: metadata-php-serializer
owner: Unassigned
status: Ready
estimate: M
depends_on: [TKT-015]
touches:
  - rust/di-compiler/crates/code-generator/src/serializer.rs
acceptance:
  - generated/metadata/global.php matches ground truth byte-for-byte
  - PHP var_export() whitespace format replicated exactly
  - Valid PHP (php -r "include 'file.php';" succeeds)
---

# TKT-016: Metadata PHP Serializer

## Scope

Serialize `ResolvedGraph` data to `<?php return [...];` PHP array syntax matching `var_export()` output.

## Implementation Notes

**Critical:** The exact whitespace format of `var_export()` must be confirmed against Phase 00 ground truth before implementing this ticket. Expected format:

```php
<?php
return array (
  'key' =>
  array (
    '_i_' => 'SomeClass\\Name',
  ),
  'preferences' =>
  array (
    'Interface' => 'Concrete',
  ),
);
```

Key formatting rules (verify against ground truth):
- 2-space indentation per nesting level
- Array key on its own line when value is an array
- Trailing comma after last array element
- Backslashes in class names doubled: `\\`
- Integer keys use bare integers (no quotes)
- Boolean `true`/`false` as PHP keywords

```rust
pub fn serialize_php_value(val: &PhpValue, indent: usize, out: &mut String)
```

## Risks

- Any whitespace difference = diff failure in TKT-023 — must match PHP exactly
- Verify: does PHP var_export use `array (` or `Array (`? (lowercase in modern PHP)
