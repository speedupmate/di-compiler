use std::path::{Path, PathBuf};

/// Discover module root paths from registration.php files and known library paths.
pub fn read_module_paths(magento_root: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();

    // vendor/magento/*/registration.php
    let vendor_magento = magento_root.join("vendor/magento");
    if vendor_magento.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&vendor_magento) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() && p.join("registration.php").exists() {
                    paths.push(p);
                }
            }
        }
    }

    // app/code/*/*/registration.php
    let app_code = magento_root.join("app/code");
    if app_code.is_dir() {
        if let Ok(vendors) = std::fs::read_dir(&app_code) {
            for vendor in vendors.flatten() {
                if let Ok(modules) = std::fs::read_dir(vendor.path()) {
                    for module in modules.flatten() {
                        let p = module.path();
                        if p.is_dir() && p.join("registration.php").exists() {
                            paths.push(p);
                        }
                    }
                }
            }
        }
    }

    // vendor/magento/framework (library, no registration.php at root)
    let framework = magento_root.join("vendor/magento/framework");
    if framework.is_dir() && !paths.contains(&framework) {
        paths.push(framework);
    }

    // Other third-party vendor modules: vendor/{vendor}/{module}/registration.php
    // Some modules nest their source under a `src/` subdirectory.
    let vendor_dir = magento_root.join("vendor");
    if let Ok(vendors) = std::fs::read_dir(&vendor_dir) {
        for vendor in vendors.flatten() {
            if vendor.file_name().to_str().map(|s| s == "magento").unwrap_or(false) {
                continue; // already handled above
            }
            let vpath = vendor.path();
            if !vpath.is_dir() {
                continue;
            }
            if let Ok(modules) = std::fs::read_dir(&vpath) {
                for module in modules.flatten() {
                    let mpath = module.path();
                    if !mpath.is_dir() {
                        continue;
                    }
                    // Direct registration.php
                    if mpath.join("registration.php").exists() {
                        paths.push(mpath.clone());
                    }
                    // Nested src/registration.php (e.g. hyva-themes pattern)
                    let src = mpath.join("src");
                    if src.is_dir() && src.join("registration.php").exists() {
                        paths.push(src);
                    }
                }
            }
        }
    }

    paths.sort();
    paths.dedup();
    paths
}

/// Walk PHP files across the given module paths, excluding Test directories.
pub fn walk_php_files(module_paths: &[PathBuf]) -> Vec<PathBuf> {
    use ignore::types::TypesBuilder;
    use ignore::WalkBuilder;

    let mut types_builder = TypesBuilder::new();
    types_builder.add("php", "*.php").unwrap();
    let php_types = types_builder.select("php").build().unwrap();

    let mut builder = WalkBuilder::new("/dev/null"); // dummy; we override below
    builder.types(php_types);
    // Exclude Test/tests directories
    builder.filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        !(name == "Test" || name == "tests" || name == "Tests" || name == "test")
    });

    // Build a fresh walker for each module path and collect results
    let mut files: Vec<PathBuf> = module_paths
        .iter()
        .flat_map(|module_path| {
            let mut types_builder2 = TypesBuilder::new();
            types_builder2.add("php", "*.php").unwrap();
            let php_types2 = types_builder2.select("php").build().unwrap();

            let mut b = WalkBuilder::new(module_path);
            b.types(php_types2);
            b.filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                !(name == "Test" || name == "tests" || name == "Tests" || name == "test")
            });
            b.build()
                .filter_map(|entry| {
                    let entry = entry.ok()?;
                    if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                        Some(entry.into_path())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect();

    files.sort();
    files.dedup();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_walk_excludes_test_dirs() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Create some PHP files
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/Foo.php"), "<?php").unwrap();
        fs::create_dir_all(root.join("src/Test")).unwrap();
        fs::write(root.join("src/Test/FooTest.php"), "<?php").unwrap();
        fs::create_dir_all(root.join("src/tests")).unwrap();
        fs::write(root.join("src/tests/bar_test.php"), "<?php").unwrap();

        let files = walk_php_files(&[root.to_path_buf()]);
        assert_eq!(files.len(), 1, "Only Foo.php should be found, not test files");
        assert!(files[0].ends_with("Foo.php"));
    }

    #[test]
    fn test_walk_deterministic() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("src")).unwrap();
        for name in &["Zzz.php", "Aaa.php", "Mmm.php"] {
            fs::write(root.join(format!("src/{name}")), "<?php").unwrap();
        }
        let r1 = walk_php_files(&[root.to_path_buf()]);
        let r2 = walk_php_files(&[root.to_path_buf()]);
        assert_eq!(r1, r2);
        // sorted
        let names: Vec<_> = r1.iter().map(|p| p.file_name().unwrap().to_str().unwrap()).collect();
        assert_eq!(names, ["Aaa.php", "Mmm.php", "Zzz.php"]);
    }
}
