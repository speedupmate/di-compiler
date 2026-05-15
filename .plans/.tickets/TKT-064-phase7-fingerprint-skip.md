---
id: TKT-064
title: Phase 7 — full-phase input fingerprint + skip on match (FP-SCOPE-3)
phase: 09-performance-hardening
feature: 24-incremental-fingerprinting
owner: Unassigned
status: Done
estimate: S
depends_on: [TKT-063]
touches:
  - rust/di-compiler/crates/cli/src/main.rs
  - rust/di-compiler/crates/code-generator/src/area_config.rs
  - rust/di-compiler/crates/code-generator/src/metadata.rs
acceptance:
  - compute_phase7_fp hashes full ClassInfo values (not just keys), all DiConfig values, resolved constants, module_paths
  - fingerprint stored at generated/.fast-di-cache/phase7.fp
  - fp_hit also verifies all expected output files exist (7 area + interception + app_action_list + 7 plugin-list)
  - labeled block 'phase7: { if fp_hit { break 'phase7; } ... } wraps all of Phase 7
  - resolve_php_constants_in_config runs before fingerprint check
  - spad() returns Cow<'static, str> with dynamic fallback for nesting > 14
  - escape_php uses chars() not bytes() for non-ASCII correctness
  - fingerprint version = 3
  - warm run logs "Phase 7 skipped: fingerprint match"
  - cargo build --release clean, zero warnings
  - cargo test passes
---

# TKT-064: Phase 7 Full-Phase Fingerprint Skip (FP-SCOPE-3)

## Scope

Skip Phase 7 entirely when all inputs are unchanged and all expected outputs exist.

### Fingerprint design

`compute_phase7_fp` takes: `class_map`, `full_di_config`, `interceptors`, `factories`,
`proxies`, `search_results`, `proxy_deferred`, `extension_specs`,
`resolved_const_values`, `module_paths`. Returns `[u8; 8]` (FxHasher state).

Hashes full values (not just key counts) so changes to constructor signatures, plugin
sort orders, argument tree structure, and PHP constants all invalidate the cache.

FxHasher is deterministic across process runs (fixed-polynomial, zero-seeded).

### Fingerprint storage

Stored at `generated/.fast-di-cache/phase7.fp`. Directory created on first write via
`std::fs::create_dir_all`. Path is outside `generated/metadata/` to avoid false
positives in archive compare diff harness.

### Existence check

Even with a matching fingerprint, Phase 7 is NOT skipped if any expected output file
is missing:
- `metadata/{area}.php` for all 7 areas
- `metadata/interception.php`
- `metadata/app_action_list.php`
- `metadata/{plugin_list_cache_id(scope)}.php` for all 7 plugin scopes

### Labeled block skip

```rust
'phase7: {
    if fp_hit { log::info!("Phase 7 skipped: fingerprint match"); break 'phase7; }
    // ... all Phase 7 work ...
    let _ = std::fs::write(&fp_path, new_fp);
}
```

### Bug fixes bundled in this ticket

- `escape_php`: was using `bytes()` → `push(b as char)` which corrupts multi-byte UTF-8.
  Fixed to use `chars()` → `push(ch)`. Returns `Cow<'_, str>` — zero-alloc for strings
  with no `\` or `'`.
- `spad()`: static table only had 15 entries (0–14 spaces). Deeper nesting returned `""`
  silently (wrong PHP indentation). Fixed: returns `Cow<'static, str>` with
  `Cow::Owned(" ".repeat(n))` fallback for n > 14.
- Fingerprint version bumped to 3 (via `h.write_u64(3u64)`) to invalidate caches
  written by any prior version.

## Status

Implemented. Build clean. Warm run consistently ~1.29s total with Phase 7 at ~6ms.
