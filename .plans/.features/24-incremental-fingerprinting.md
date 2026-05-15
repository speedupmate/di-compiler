# 24: Incremental Fingerprinting

- Category: Performance
- Status: Implemented (Phase 7 full-skip via FP-SCOPE-3; TKT-064)
- Implementation Phase: 09-performance-hardening
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

## Delivered (TKT-064)

Phase 7 full-phase fingerprint skip is implemented (FP-SCOPE-3). No-change repeat runs
complete in ~1.29s total with Phase 7 at ~6ms (fingerprint hit, skip). See TKT-064 for
implementation details.

The per-file incremental cache (blake3 hash per PHP file, cache JSON) was a separate
earlier deliverable (TKT-059). The di.xml incremental cache correctness fix is in TKT-059.

## Remaining Acceptance Criteria (not yet implemented)

- `--incremental` flag for per-file source tracking (deferred; FP-SCOPE-3 covers the
  common dev-loop use case of no-change re-runs)
