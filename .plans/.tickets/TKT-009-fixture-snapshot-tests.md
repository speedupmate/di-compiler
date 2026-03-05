---
id: TKT-009
title: Fixture corpus + insta snapshot tests
phase: 01-php-extractor
feature: snapshot-test-corpus
owner: Unassigned
status: Ready
estimate: M
depends_on: [TKT-008]
touches:
  - rust/di-compiler/tests/fixtures/
  - rust/di-compiler/tests/snapshots/
  - rust/di-compiler/tests/extraction.rs
acceptance:
  - 20 fixture .php files created (see feature spec 22)
  - insta snapshot test for each fixture passes
  - cargo test --test extraction runs in < 5 s
---

# TKT-009: Fixture Corpus + Snapshot Tests

## Scope

Create all fixture PHP files and write snapshot tests using `insta`.

## Implementation Notes

Create `tests/fixtures/*.php` — one per edge case listed in feature spec 22.

```rust
// tests/extraction.rs
use insta::assert_json_snapshot;
use php_extractor::{extract_file, ExtractConfig};

macro_rules! fixture_test {
    ($name:ident) => {
        #[test]
        fn $name() {
            let path = std::path::Path::new(concat!("tests/fixtures/", stringify!($name), ".php"));
            let result = extract_file(path, &ExtractConfig::default());
            assert_json_snapshot!(result);
        }
    };
}

fixture_test!(basic_class);
fixture_test!(abstract_class);
fixture_test!(interface);
fixture_test!(trait_file);
fixture_test!(no_namespace);
fixture_test!(constructor_promotion);
// ... all 20 fixtures
```

Run `cargo insta review` to approve initial snapshots, then commit them.

## Risks

- Initial snapshot approval requires manual review — do not auto-approve in CI
