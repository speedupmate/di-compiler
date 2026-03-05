# 22: Snapshot Test Corpus

- Category: Testing
- Status: Planned
- Implementation Phase: 01-php-extractor
- Owner: Unassigned
- Feature ID: `snapshot-test-corpus`
- Suggested Dependencies: 06-extract-result-type

## Intent

Build a fixture corpus of PHP files covering all edge cases. Use `insta` snapshot testing
to lock extraction output and catch regressions automatically.

## Fixture Files Required

One `.php` file per edge case (in `tests/fixtures/`):

| Fixture | Tests |
|---------|-------|
| `basic_class.php` | Simple class with typed constructor params |
| `abstract_class.php` | Abstract class extraction |
| `interface.php` | Interface (no constructor) |
| `trait.php` | Trait extraction |
| `no_namespace.php` | Class without namespace declaration |
| `constructor_promotion.php` | `public readonly Foo $foo` in constructor |
| `nullable_type.php` | `?Foo $param` |
| `union_type.php` | `Foo\|Bar $param` (first type extracted) |
| `intersection_type.php` | `Foo&Bar $param` (→ Unsupported → Tier 2) |
| `variadic_param.php` | `...$args` param |
| `no_constructor.php` | Class with no `__construct` |
| `optional_params.php` | Mix of required and optional params |
| `multiple_classes.php` | Two classes in one file (first extracted) |
| `enum_class.php` | PHP 8.1 enum (→ NoClass) |
| `readonly_class.php` | PHP 8.2 readonly class |
| `final_class.php` | Final class (no interceptor generated) |
| `extends_implements.php` | Class with extends + implements list |
| `deeply_nested_namespace.php` | Long namespace with backslashes |
| `crlf_line_endings.php` | Windows CRLF line endings |
| `docblock_params.php` | `@param` docblock (no type hint) |

## Snapshot Tests

```rust
// tests/extraction.rs
#[test]
fn test_basic_class() {
    let result = extract_file(Path::new("tests/fixtures/basic_class.php"), &default_config());
    insta::assert_json_snapshot!(result);
}
```

## CI Integration Test

```rust
// tests/integration.rs
#[test]
#[ignore]  // requires real Magento install
fn test_full_magento_corpus() {
    // Walk tests/corpus/ (symlink to vendor/)
    // Extract all files
    // Assert: < 0.5% fallback rate, 0 PhpFallbackFailed
}
```

## Acceptance Criteria

- All 20 fixture snapshot tests pass
- `insta` review mode used to approve initial snapshots
- CI runs fixtures on every PR (fast, < 5 s)
- Integration test marked `#[ignore]`, runs in separate CI job
