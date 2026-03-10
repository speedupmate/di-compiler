---
id: TKT-008
title: Extract result orchestrator
phase: 01-php-extractor
feature: extract-result-type
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-003, TKT-004, TKT-005, TKT-006, TKT-007]
touches:
  - rust/di-compiler/crates/php-extractor/src/lib.rs
acceptance:
  - extract_file() tries Tier 1 → 2 → 3 correctly
  - PhpFallbackFailed causes non-zero exit
  - NoClass is silently ignored
  - Stats printed with --verbose
---

# TKT-008: Extract Result Orchestrator

## Scope

Top-level `extract_file(path, config) -> ExtractResult` function and the `ExtractResult` enum.

## Implementation Notes

```rust
pub fn extract_file(path: &Path, config: &ExtractConfig) -> ExtractResult {
    match extract_tier1(path) {
        Ok(Some(info))  => ExtractResult::Ok(info),
        Ok(None)        => ExtractResult::NoClass,
        Err(LexError::Unsupported(f)) => {
            log::warn!("tier1 unsupported: {} in {}", f, path.display());
            match extract_tier2(read_bytes(path)) {
                Ok(Some(info)) => ExtractResult::Ok(info),
                Ok(None)       => ExtractResult::NoClass,
                Err(_) if config.fallback_php => {
                    match extract_tier3(path) {
                        Ok(r) => r.map_or(ExtractResult::NoClass, ExtractResult::Ok),
                        Err(e) => ExtractResult::PhpFallbackFailed { path: path.into() },
                    }
                }
                Err(e) => ExtractResult::ParseFailure { path: path.into(), reason: e.to_string() },
            }
        }
        Err(e) => ExtractResult::ParseFailure { path: path.into(), reason: e.to_string() },
    }
}
```

Expose public API:
```rust
pub fn extract_all(paths: &[PathBuf], config: &ExtractConfig) -> (Vec<ClassInfo>, ExtractionStats)
```

## Risks

- `PhpFallbackFailed` must bubble up to abort compilation — ensure caller checks this
