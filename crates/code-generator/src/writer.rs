// Content-addressed file writer — TKT-022
use std::path::Path;

/// Write content to path only if it differs from the existing file.
/// Returns true if the file was written (content changed or new).
pub fn write_if_changed(path: &Path, content: &str) -> std::io::Result<bool> {
    use rustc_hash::FxHasher;
    use std::hash::{Hash, Hasher};

    if let Ok(existing) = std::fs::read_to_string(path) {
        if existing == content {
            return Ok(false);
        }
        // Also compare by hash for speed on large files
        let mut h1 = FxHasher::default();
        existing.hash(&mut h1);
        let mut h2 = FxHasher::default();
        content.hash(&mut h2);
        if h1.finish() == h2.finish() {
            return Ok(false);
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(true)
}
