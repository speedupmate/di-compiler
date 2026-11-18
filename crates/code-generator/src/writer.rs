use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);
static WRITTEN: AtomicUsize = AtomicUsize::new(0);
static UNCHANGED: AtomicUsize = AtomicUsize::new(0);
static STOPPED: AtomicBool = AtomicBool::new(false);

/// Files that `write_if_changed()` wrote and left as they were, in this process.
pub fn write_counts() -> (usize, usize) {
    (
        WRITTEN.load(Ordering::Relaxed),
        UNCHANGED.load(Ordering::Relaxed),
    )
}

/// Every later write fails with `ErrorKind::Interrupted`. A compile that is told to stop calls this
/// before it removes its output, so no thread writes a file back into it.
pub fn stop_writes() {
    STOPPED.store(true, Ordering::SeqCst);
}

fn interrupted() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Interrupted, "the compile is stopping")
}

/// Writes `content` to a temporary file in the folder of `path`, then renames it over `path`. A
/// request that reads the file meanwhile sees the old or the new content, never a part of it. A
/// replaced file keeps its permission bits. The temporary name starts with a dot and ends in
/// `.tmp`, so no autoloader maps a class to it.
pub fn write_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    if STOPPED.load(Ordering::SeqCst) {
        return Err(interrupted());
    }
    let folder = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(folder)?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = folder.join(format!(
        ".{name}.{}-{}.tmp",
        std::process::id(),
        NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(content)?;
        if let Ok(existing) = std::fs::metadata(path) {
            file.set_permissions(existing.permissions())?;
        }
        drop(file);
        if STOPPED.load(Ordering::SeqCst) {
            return Err(interrupted());
        }
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

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
            UNCHANGED.fetch_add(1, Ordering::Relaxed);
            return Ok(false);
        }
    }

    write_atomic(path, content.as_bytes())?;
    WRITTEN.fetch_add(1, Ordering::Relaxed);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{write_atomic, write_if_changed};
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

    fn entries(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn replacing_a_file_leaves_no_temporary_file() {
        let dir = tmp();
        let path = dir.path().join("Interceptor.php");
        write_if_changed(&path, "<?php echo 1;").unwrap();
        write_if_changed(&path, "<?php echo 2;").unwrap();
        assert_eq!(entries(dir.path()), ["Interceptor.php"]);
        assert_eq!(fs::read_to_string(&path).unwrap(), "<?php echo 2;");
    }

    #[test]
    fn a_reader_holding_the_old_file_keeps_reading_the_old_content() {
        use std::io::Read;
        let dir = tmp();
        let path = dir.path().join("global.php");
        fs::write(&path, "<?php return 'old';").unwrap();
        let mut held = fs::File::open(&path).unwrap();
        write_if_changed(&path, "<?php return 'new';").unwrap();
        let mut old = String::new();
        held.read_to_string(&mut old).unwrap();
        assert_eq!(old, "<?php return 'old';");
        assert_eq!(fs::read_to_string(&path).unwrap(), "<?php return 'new';");
    }

    #[cfg(unix)]
    #[test]
    fn a_replaced_file_keeps_its_permission_bits() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp();
        let path = dir.path().join("Factory.php");
        fs::write(&path, "<?php").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        write_if_changed(&path, "<?php echo 1;").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_write_keeps_the_old_file_and_leaves_no_temporary_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp();
        let folder = dir.path().join("locked");
        fs::create_dir(&folder).unwrap();
        let path = folder.join("Proxy.php");
        fs::write(&path, "<?php return 'old';").unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o555)).unwrap();
        let probe = folder.join("probe");
        if fs::write(&probe, "").is_ok() {
            // Running as root, which ignores the folder mode.
            let _ = fs::remove_file(&probe);
            fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }
        let result = write_if_changed(&path, "<?php return 'new';");
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "<?php return 'old';");
        assert_eq!(entries(&folder), ["Proxy.php"]);
    }

    #[test]
    fn write_atomic_creates_missing_folders() {
        let dir = tmp();
        let path = dir.path().join("a/b/.di-compiler-cache.json");
        write_atomic(&path, b"{}").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{}");
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
