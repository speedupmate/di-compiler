---
id: TKT-026
title: Incremental compilation fingerprinting
phase: 07-performance
feature: incremental-fingerprinting
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-025]
touches:
  - rust/di-compiler/crates/cli/src/incremental.rs
acceptance:
  - --incremental re-run with no changes completes in < 1 s
  - Re-run with one changed PHP file only re-processes that file
  - di.xml change triggers full re-run
  - TKT-023 still green after incremental run
---

# TKT-026: Incremental Compilation Fingerprinting

## Scope

Cache `blake3` file hashes in `generated/.di-compiler-cache.json`.
Skip re-processing unchanged files on subsequent runs.

## Implementation Notes

```rust
pub struct Cache {
    pub version: u32,
    pub files: HashMap<PathBuf, FileCacheEntry>,
    pub di_xml_hash: String,   // combined hash of all di.xml content
}

pub struct FileCacheEntry {
    pub hash: String,          // blake3 hex
    pub output_files: Vec<PathBuf>,
}
```

Algorithm:
1. Load cache from `generated/.di-compiler-cache.json` (or empty cache if not present)
2. Compute di.xml combined hash → if different from cache, full re-run (invalidate everything)
3. For each PHP file: compute blake3 hash → if same as cache entry, skip extraction + codegen
4. After run: update cache with new hashes and output file lists
5. Save cache to disk

```rust
use blake3::Hasher;

pub fn hash_file(path: &Path) -> String {
    let mut hasher = Hasher::new();
    hasher.update(&std::fs::read(path).unwrap());
    hasher.finalize().to_hex().to_string()
}
```

Add `blake3 = "1"` to `cli` crate dependencies.

## Risks

- Cache file format must be versioned (bump version when format changes)
- Deleted source files → must remove cached entries and output files
