---
id: TKT-023
title: Validator diff harness
phase: 05-validator
feature: validator-harness
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-016, TKT-017, TKT-018, TKT-019, TKT-020, TKT-021, TKT-022]
touches:
  - rust/di-compiler/crates/validator/src/lib.rs
  - rust/di-compiler/tests/integration.rs
acceptance:
  - Zero diffs against PHP ground truth on this Magento install
  - ValidationResult correctly categorizes missing/extra/changed files
  - --validate flag exits non-zero on any diff
---

# TKT-023: Validator Diff Harness

## Scope

Build the `validator` crate and the integration test.

## Implementation Notes

```rust
pub fn validate(
    php_generated: &Path,
    rust_generated: &Path,
) -> ValidationResult
```

Algorithm:
1. Walk both directories, collect relative paths
2. Files in PHP only → `files_only_in_php`
3. Files in Rust only → `files_only_in_rust`
4. Files in both → compare bytes → if different, `files_with_diff` with unified diff (first 50 lines)

Integration test (CI only, `#[ignore]`):
```rust
#[test]
#[ignore]
fn test_full_magento_diff() {
    // 1. Use ground truth from generated/_code/ and generated/_metadata/ (Phase 00 output)
    // 2. Run Rust compiler
    // 3. Validate
    // 4. assert!(result.is_clean())
}
```

## Risks

- Ground truth dirs generated/_code/ and generated/_metadata/ must exist (created in Phase 00)
- PHP and Rust runs must use identical Magento state (no intervening changes)
