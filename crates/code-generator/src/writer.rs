// Content-addressed file writer — TKT-022
use std::path::Path;

/// Write content to path only if it differs from the existing file.
/// Returns true if the file was written (content changed or new).
pub fn write_if_changed(path: &Path, content: &str) -> std::io::Result<bool> {
    use rustc_hash::FxHasher;
    use std::hash::{Hash, Hasher};

    if let Ok(existing) = std::fs::read_to_string(path) {
        // Hash-first: fast reject on definite difference, then confirm with
        // string equality to guard against hash collisions.
        let mut h1 = FxHasher::default();
        existing.hash(&mut h1);
        let mut h2 = FxHasher::default();
        content.hash(&mut h2);
        if h1.finish() == h2.finish() && existing == content {
            return Ok(false);
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::write_if_changed;
    use std::fs;
    use tempfile::TempDir;

    fn tmp() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn new_file_is_written_and_returns_true() {
        let dir = tmp();
        let path = dir.path().join("new.php");
        let result = write_if_changed(&path, "<?php echo 1;").unwrap();
        assert!(result, "should return true for new file");
        assert_eq!(fs::read_to_string(&path).unwrap(), "<?php echo 1;");
    }

    #[test]
    fn unchanged_file_is_not_rewritten_and_returns_false() {
        let dir = tmp();
        let path = dir.path().join("same.php");
        fs::write(&path, "<?php echo 1;").unwrap();
        let mtime_before = fs::metadata(&path).unwrap().modified().unwrap();

        let result = write_if_changed(&path, "<?php echo 1;").unwrap();
        assert!(!result, "should return false for identical content");

        let mtime_after = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after, "file should not be touched");
    }

    #[test]
    fn changed_file_is_rewritten_and_returns_true() {
        let dir = tmp();
        let path = dir.path().join("changed.php");
        fs::write(&path, "<?php echo 1;").unwrap();

        let result = write_if_changed(&path, "<?php echo 2;").unwrap();
        assert!(result, "should return true for changed content");
        assert_eq!(fs::read_to_string(&path).unwrap(), "<?php echo 2;");
    }

    #[test]
    fn creates_parent_directories() {
        let dir = tmp();
        let path = dir.path().join("deep/nested/dir/file.php");
        let result = write_if_changed(&path, "<?php").unwrap();
        assert!(result);
        assert!(path.exists());
    }

    #[test]
    fn empty_string_content_works() {
        let dir = tmp();
        let path = dir.path().join("empty.php");
        // Write empty file
        let r1 = write_if_changed(&path, "").unwrap();
        assert!(r1);
        // Re-check: should be unchanged
        let r2 = write_if_changed(&path, "").unwrap();
        assert!(!r2);
    }

    #[test]
    fn hash_first_does_not_skip_write_for_different_content() {
        // Regression: previous code did string equality FIRST, then hash — but only
        // returned early on hash match after confirming strings differ, creating a
        // silent no-write on hash collision. New code: hash-first, then string confirm.
        // This test verifies that two strings with identical hashes would still be
        // correctly distinguished by the string equality guard — but since we can't
        // reliably produce FxHasher collisions, we test the observable contract:
        // different content always results in a write.
        let dir = tmp();
        let path = dir.path().join("collision_guard.php");
        fs::write(&path, "<?php $a = 1;").unwrap();

        let result = write_if_changed(&path, "<?php $b = 2;").unwrap();
        assert!(result, "different content must always be written");
        assert_eq!(fs::read_to_string(&path).unwrap(), "<?php $b = 2;");
    }

    #[test]
    fn large_file_unchanged_returns_false() {
        let dir = tmp();
        let path = dir.path().join("large.php");
        let content: String = "<?php\n".to_string() + &"// line\n".repeat(10_000);
        fs::write(&path, &content).unwrap();

        let result = write_if_changed(&path, &content).unwrap();
        assert!(!result, "large unchanged file should not be rewritten");
    }
}
