# Phase 05: Validator

## Purpose

Build the `validator` crate (dev/CI only). Run PHP compiler and Rust compiler against the
same Magento install, diff the `generated/` trees, and report `ValidationResult`.

## Gate To Enter

Phase 04 complete (all code generators producing output).

## Gate To Complete

- TKT-023 green: zero diffs against PHP ground truth
- `ValidationResult` reports correctly categorize missing, extra, and changed files

## Current Status (2026-03-05)

Latest repo-local run against archived ground truth:

- Code: 723 missing, 13 extra, 4219 changed
- Metadata: 8 missing, 0 extra, 8 changed

Phase 05 tooling is implemented, but Phase 08 is required to close the remaining parity gaps.

## Features In This Phase

| Feature | Deps |
|---------|------|
| [20-validator-harness](../.features/20-validator-harness.md) | all phase 04 |

## Validation Targets

Run on:
1. Clean Magento 2.4.x CE
2. Magento with sample data
3. This specific install (primary target, baseline at `generated/_code` and `generated/_metadata`)

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
