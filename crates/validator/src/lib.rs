//! TKT-023: Validator diff harness.
//!
//! Compares two generated directories (PHP ground truth vs Rust output) and
//! emits a ValidationResult describing all differences.

use std::collections::HashSet;
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

    /// Human-readable summary.
    pub fn summary(&self) -> String {
        if self.is_clean() {
            return "✓ Clean: Rust output matches PHP ground truth.".to_string();
        }
        let mut lines = Vec::new();
        if !self.files_only_in_php.is_empty() {
            lines.push(format!(
                "  Missing from Rust: {} files",
                self.files_only_in_php.len()
            ));
        }
        if !self.files_only_in_rust.is_empty() {
            lines.push(format!(
                "  Extra in Rust: {} files",
                self.files_only_in_rust.len()
            ));
        }
        if !self.files_with_diff.is_empty() {
            lines.push(format!("  Content differs: {} files", self.files_with_diff.len()));
        }
        if !self.extraction_failures.is_empty() {
            lines.push(format!(
                "  Extraction failures: {} files",
                self.extraction_failures.len()
            ));
        }
        lines.join("\n")
    }
}

#[derive(Debug)]
pub struct FileDiff {
    pub path: PathBuf,
    pub php_sha256: String,
    pub rust_sha256: String,
    /// First 50 lines of unified diff (line-level)
    pub unified_diff: String,
}

/// Compare `php_generated` vs `rust_generated` directories recursively.
pub fn validate(php_generated: &Path, rust_generated: &Path) -> ValidationResult {
    let mut result = ValidationResult::default();

    let php_files = collect_relative_paths(php_generated);
    let rust_files = collect_relative_paths(rust_generated);

    let php_set: HashSet<PathBuf> = php_files.iter().cloned().collect();
    let rust_set: HashSet<PathBuf> = rust_files.iter().cloned().collect();

    for rel in &php_set {
        if !rust_set.contains(rel) {
            result.files_only_in_php.push(rel.clone());
        }
    }
    for rel in &rust_set {
        if !php_set.contains(rel) {
            result.files_only_in_rust.push(rel.clone());
        }
    }

    // Compare shared files
    for rel in php_set.intersection(&rust_set) {
        let php_path = php_generated.join(rel);
        let rust_path = rust_generated.join(rel);

        let php_content = std::fs::read(&php_path).unwrap_or_default();
        let rust_content = std::fs::read(&rust_path).unwrap_or_default();

        if php_content != rust_content {
            let php_sha = sha256_hex(&php_content);
            let rust_sha = sha256_hex(&rust_content);
            let diff = unified_diff_first50(
                &String::from_utf8_lossy(&php_content),
                &String::from_utf8_lossy(&rust_content),
            );
            result.files_with_diff.push(FileDiff {
                path: rel.clone(),
                php_sha256: php_sha,
                rust_sha256: rust_sha,
                unified_diff: diff,
            });
        }
    }

    result.files_only_in_php.sort();
    result.files_only_in_rust.sort();
    result.files_with_diff.sort_by(|a, b| a.path.cmp(&b.path));

    result
}

fn collect_relative_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if !root.exists() {
        return paths;
    }
    collect_recursive(root, root, &mut paths);
    paths
}

fn collect_recursive(root: &Path, current: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(current) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_recursive(root, &path, out);
        } else if path.is_file() {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_path_buf());
            }
        }
    }
}

fn sha256_hex(data: &[u8]) -> String {
    // Simple FNV-like hex digest for diff identification (not crypto-grade)
    // Using a simple checksum since we don't depend on sha2 crate
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", hash)
}

fn unified_diff_first50(php: &str, rust: &str) -> String {
    let php_lines: Vec<&str> = php.lines().collect();
    let rust_lines: Vec<&str> = rust.lines().collect();

    let mut out = Vec::new();
    let mut i = 0;
    let mut j = 0;
    let mut count = 0;

    while count < 50 && (i < php_lines.len() || j < rust_lines.len()) {
        let a = php_lines.get(i);
        let b = rust_lines.get(j);
        match (a, b) {
            (Some(al), Some(bl)) if al == bl => {
                i += 1;
                j += 1;
            }
            (Some(al), _) => {
                out.push(format!("-{}", al));
                i += 1;
                count += 1;
            }
            (None, Some(bl)) => {
                out.push(format!("+{}", bl));
                j += 1;
                count += 1;
            }
            (None, None) => break,
        }
    }

    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_clean_when_identical() {
        let php_dir = tempdir().unwrap();
        let rust_dir = tempdir().unwrap();
        fs::write(php_dir.path().join("file.php"), "<?php echo 1;").unwrap();
        fs::write(rust_dir.path().join("file.php"), "<?php echo 1;").unwrap();
        let result = validate(php_dir.path(), rust_dir.path());
        assert!(result.is_clean());
    }

    #[test]
    fn test_missing_from_rust() {
        let php_dir = tempdir().unwrap();
        let rust_dir = tempdir().unwrap();
        fs::write(php_dir.path().join("extra.php"), "<?php").unwrap();
        let result = validate(php_dir.path(), rust_dir.path());
        assert_eq!(result.files_only_in_php.len(), 1);
    }

    #[test]
    fn test_content_diff() {
        let php_dir = tempdir().unwrap();
        let rust_dir = tempdir().unwrap();
        fs::write(php_dir.path().join("f.php"), "<?php return 1;").unwrap();
        fs::write(rust_dir.path().join("f.php"), "<?php return 2;").unwrap();
        let result = validate(php_dir.path(), rust_dir.path());
        assert_eq!(result.files_with_diff.len(), 1);
    }
}
