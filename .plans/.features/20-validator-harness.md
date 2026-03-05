# 20: Validator Harness

- Category: Quality
- Status: Planned
- Implementation Phase: 05-validator
- Owner: Unassigned
- Feature ID: `validator-harness`
- Suggested Dependencies: All phase 04 features

## Intent

Run PHP compiler and Rust compiler against the same Magento install, diff the
`generated/` trees, and report structured `ValidationResult`. Used in CI and via `--validate` flag.

## Core State and Actions

```rust
pub struct ValidationResult {
    pub files_only_in_php: Vec<PathBuf>,        // Rust missed generating
    pub files_only_in_rust: Vec<PathBuf>,       // Rust generated extras
    pub files_with_diff: Vec<FileDiff>,         // Content mismatch
    pub extraction_failures: Vec<PathBuf>,      // Files Rust couldn't parse
}

pub struct FileDiff {
    pub path: PathBuf,
    pub php_sha256: String,
    pub rust_sha256: String,
    pub unified_diff: String,                   // first 50 lines
}

pub fn validate(magento_root: &Path, rust_output: &Path, php_output: &Path) -> ValidationResult
```

## Validation Steps

```bash
# 1. Generate with PHP (ground truth) — done once in Phase 00
#    Results live at generated/_code/ and generated/_metadata/

# 2. Generate with Rust
rm -rf generated/code generated/metadata
./target/release/fast-di-compile --magento-root .

# 3. Diff Rust output against PHP ground truth
diff -rq generated/_code/ generated/code/
diff -rq generated/_metadata/ generated/metadata/
```

## Acceptance Criteria

- `ValidationResult` with zero diffs = phase complete
- `--validate` CLI flag runs this and exits non-zero on any diff
- Diff output includes file path + first 50 lines of unified diff
- Can be run against: clean install, sample data, this install
