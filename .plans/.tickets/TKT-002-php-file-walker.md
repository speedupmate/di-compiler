---
id: TKT-002
title: PHP file walker + module path reader
phase: 01-php-extractor
feature: php-file-walker
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-001]
touches:
  - rust/di-compiler/crates/php-extractor/src/walker.rs
acceptance:
  - Returns same file list as PHP ClassesScanner for test module
  - Test/ directories excluded
  - Output order is deterministic
---

# TKT-002: PHP File Walker + Module Path Reader

## Scope

Discover all PHP files the DI compiler should process.

## Implementation Notes

1. `read_module_paths(root: &Path) -> Vec<PathBuf>`
   - Scan `vendor/magento/*/registration.php` for magento modules
   - Scan `app/code/*/*/registration.php` for local modules
   - Also walk `vendor/magento/framework/` directly (library, not a module)
   - Return deduplicated sorted list of module root paths

2. `walk_php_files(module_paths: &[PathBuf]) -> Vec<PathBuf>`
   - Use `ignore::WalkBuilder` — add each path, set `types` filter to `*.php`
   - Add `overrides` to exclude `/Test/` and `/tests/` path components
   - `.build_parallel()` then collect into `Vec<PathBuf>`
   - Sort for deterministic order

## Test

- Unit test with a temp dir containing known PHP files
- Assert Test/ subdirectory files are excluded

## Risks

- `registration.php` scanning may miss some edge cases — verify against Phase 00 module count
