# 06: Extract Result Type + Orchestrator

- Category: Extraction
- Status: Planned
- Implementation Phase: 01-php-extractor
- Owner: Unassigned
- Feature ID: `extract-result-type`
- Suggested Dependencies: 03-php-lexer-tier1, 04-php-treesitter-tier2, 05-php-fallback-tier3

## Intent

Define the `ExtractResult` enum and the top-level `extract_file(path)` function that
tries Tier 1 → 2 → 3 and returns the unified result. Never panics.

## Core State and Actions

```rust
pub enum ExtractResult {
    Ok(ClassInfo),
    NoClass,                            // file has no class — normal (helpers, config)
    ParseFailure { path: PathBuf, reason: String },    // tried all tiers, log + continue
    Unsupported { path: PathBuf, feature: String },    // logged, fallback attempted
    PhpFallbackFailed { path: PathBuf },               // log + abort compilation
}

pub fn extract_file(path: &Path, config: &ExtractConfig) -> ExtractResult {
    match extract_tier1(path) {
        Ok(Some(info)) => ExtractResult::Ok(info),
        Ok(None) => ExtractResult::NoClass,
        Err(LexError::Unsupported(feat)) => {
            log::warn!("Tier1 unsupported: {} in {:?}", feat, path);
            match extract_tier2(path) {
                Ok(Some(info)) => ExtractResult::Ok(info),
                _ if config.fallback_php => match extract_tier3(path) {
                    Ok(Some(info)) => ExtractResult::Ok(info),
                    _ => ExtractResult::PhpFallbackFailed { path: path.into() },
                },
                _ => ExtractResult::ParseFailure { path: path.into(), reason: feat },
            }
        }
        Err(e) => ExtractResult::ParseFailure { path: path.into(), reason: e.to_string() },
    }
}
```

## Acceptance Criteria

- `PhpFallbackFailed` causes the overall compile to exit non-zero
- `ParseFailure` is logged but does not abort (unless count exceeds threshold)
- `NoClass` is silently ignored
- Extraction stats (ok/no-class/parse-failure/fallback counts) printed at end of run with `--verbose`
