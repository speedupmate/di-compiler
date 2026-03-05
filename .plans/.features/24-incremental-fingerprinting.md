# 24: Incremental Fingerprinting

- Category: Performance
- Status: Planned
- Implementation Phase: 07-performance
- Owner: Unassigned
- Feature ID: `incremental-fingerprinting`
- Suggested Dependencies: 23-parallel-rayon

## Intent

Skip re-processing of unchanged files on subsequent runs by caching `blake3` content hashes.
Dramatically reduces re-compile time when only a few files changed.

## Cache Format

`generated/.di-compiler-cache.json`:
```json
{
  "version": 1,
  "files": {
    "/var/www/application/vendor/magento/module-catalog/Model/Product.php": {
      "hash": "abc123...",
      "last_compiled": "2026-03-05T10:00:00Z",
      "output_files": [
        "generated/code/Magento/Catalog/Model/Product/Interceptor.php"
      ]
    }
  },
  "di_xml_hash": "def456..."
}
```

## Core State and Actions

```rust
pub fn compute_file_hash(path: &Path) -> [u8; 32]  // blake3
pub fn load_cache(generated_dir: &Path) -> Cache
pub fn save_cache(cache: &Cache, generated_dir: &Path)
pub fn needs_recompile(path: &Path, cache: &Cache) -> bool
```

## Invalidation Rules

- Any di.xml file change → invalidate ALL classes (di config changed)
- PHP file change (hash different) → re-extract that class and regenerate its outputs
- Cache miss (new file) → process normally

## Acceptance Criteria

- `--incremental` flag enables fingerprinting
- Re-run with no changes completes in < 1 s
- Re-run with one changed PHP file only re-processes that file
- Di.xml change triggers full re-run
- Output still passes TKT-023 diff harness
