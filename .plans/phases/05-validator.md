# Phase 05: Validator

## Purpose

Build the `validator` crate (dev/CI only). Run PHP compiler and Rust compiler against the
same Magento install, diff the `generated/` trees, and report `ValidationResult`.

## Gate To Enter

Phase 04 complete (all code generators producing output).

## Gate To Complete

- TKT-023 green: zero diffs against PHP ground truth
- `ValidationResult` reports correctly categorize missing, extra, and changed files

## Features In This Phase

| Feature | Deps |
|---------|------|
| [20-validator-harness](./.features/20-validator-harness.md) | all phase 04 |

## Validation Targets

Run on:
1. Clean Magento 2.4.x CE
2. Magento with sample data
3. This specific install (primary target)

## ValidationResult Fields

```rust
pub struct ValidationResult {
    pub files_only_in_php: Vec<PathBuf>,      // Rust missed generating
    pub files_only_in_rust: Vec<PathBuf>,     // Rust generated extras
    pub files_with_diff: Vec<FileDiff>,       // Content mismatch
    pub extraction_failures: Vec<PathBuf>,    // Files Rust couldn't parse
}
```

## Tickets In This Phase

TKT-023
