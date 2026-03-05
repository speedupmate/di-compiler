# 02: PHP File Walker

- Category: I/O
- Status: Planned
- Implementation Phase: 01-php-extractor
- Owner: Unassigned
- Feature ID: `php-file-walker`
- Suggested Dependencies: 01-workspace-scaffold

## Intent

Given a Magento root path, discover all PHP files that the DI compiler should process:
- Read module paths from `ComponentRegistrar` (via `registration.php` scan or `app/etc/config.php`)
- Walk those paths recursively for `*.php` files
- Exclude `Test/` and `tests/` directories

## Core State and Actions

1. `read_module_paths(magento_root: &Path) -> Vec<PathBuf>` — returns module root directories
   - Scan `vendor/magento/*/registration.php` and `app/code/*/registration.php`
   - Also include framework library paths
2. `walk_php_files(paths: &[PathBuf]) -> Vec<PathBuf>` — uses `ignore::WalkBuilder`
   - File type filter: `*.php` only
   - Exclude pattern: path contains `/Test/` or `/tests/`
   - Parallel walk via `ignore::WalkBuilder::build_parallel()`
3. Returns `Vec<PathBuf>` sorted deterministically

## Runtime Effects

- Reads filesystem: `registration.php` files, directory trees
- Uses `ignore` crate (respects `.gitignore` and `.ignore` files)

## Acceptance Criteria

- File list matches set discovered by PHP `ClassesScanner` (cross-check in Phase 00)
- Test/ directories excluded
- Deterministic output order (for reproducible downstream processing)
