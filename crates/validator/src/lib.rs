use std::path::{Path, PathBuf};

/// Result of comparing PHP ground truth vs Rust-generated output.
#[derive(Debug, Default)]
pub struct ValidationResult {
    /// Files present in PHP output but missing from Rust output
    pub files_only_in_php: Vec<PathBuf>,
    /// Files present in Rust output but not in PHP output
    pub files_only_in_rust: Vec<PathBuf>,
    /// Files present in both but with different content
    pub files_with_diff: Vec<FileDiff>,
    /// Files Rust couldn't generate
    pub extraction_failures: Vec<PathBuf>,
}

impl ValidationResult {
    pub fn is_clean(&self) -> bool {
        self.files_only_in_php.is_empty()
            && self.files_only_in_rust.is_empty()
            && self.files_with_diff.is_empty()
            && self.extraction_failures.is_empty()
    }
}

#[derive(Debug)]
pub struct FileDiff {
    pub path: PathBuf,
    pub php_sha256: String,
    pub rust_sha256: String,
    /// First 50 lines of unified diff
    pub unified_diff: String,
}

/// Compare php_generated vs rust_generated directories and return a ValidationResult.
/// Implemented in TKT-023.
pub fn validate(php_generated: &Path, rust_generated: &Path) -> ValidationResult {
    let _ = (php_generated, rust_generated);
    ValidationResult::default()
}
