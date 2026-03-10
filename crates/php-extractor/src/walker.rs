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

    // root setup source (no registration.php)
    // NOTE: setup/src is included for class map / factory-candidate resolution only.
    // PHP treats setup as type SETUP (not MODULE) — interceptors for Setup classes
    // are suppressed post-detection in the CLI, not here.
    let setup = magento_root.join("setup");
    if setup.is_dir() && !paths.contains(&setup) {
        paths.push(setup);
    }

    // Other third-party vendor modules: vendor/{vendor}/{package}/.../registration.php
    // Discover module roots from registration.php paths inside each package.
    let vendor_dir = magento_root.join("vendor");
    if let Ok(vendors) = std::fs::read_dir(&vendor_dir) {
        for vendor in vendors.flatten() {
            if vendor
                .file_name()
                .to_str()
                .map(|s| s == "magento")
                .unwrap_or(false)
            {
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
                    for module_root in discover_module_roots_from_registration(&mpath) {
                        paths.push(module_root);
                    }
                }
            }
        }
    }

    paths.sort();
    paths.dedup();
    paths
}

fn discover_module_roots_from_registration(package_root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(package_root.to_path_buf(), 0)];
    const MAX_DEPTH: usize = 6;

    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_file() {
                if entry.file_name().to_string_lossy() == "registration.php" {
                    if let Some(parent) = path.parent() {
                        roots.push(parent.to_path_buf());
                    }
                }
                continue;
            }
            if file_type.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if should_skip_scan_dir(&name) {
                    continue;
                }
                stack.push((path, depth + 1));
            }
        }
    }

    roots.sort();
    roots.dedup();
    roots
}

fn should_skip_scan_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "node_modules"
            | "Test"
            | "Tests"
            | "test"
            | "tests"
            | "dev"
            | "TestFramework"
            | "magento2-functional-testing-framework"
    )
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
    // Exclude test directories and test framework packages that PHP's DI compiler also skips.
    builder.filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        !matches!(
            name.as_ref(),
            "Test"
                | "tests"
                | "Tests"
                | "test"
                | "TestFramework"
                | "magento2-functional-testing-framework"
        )
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
                !matches!(
                    name.as_ref(),
                    "Test"
                        | "tests"
                        | "Tests"
                        | "test"
                        | "TestFramework"
                        | "magento2-functional-testing-framework"
                )
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
        assert_eq!(
            files.len(),
            1,
            "Only Foo.php should be found, not test files"
        );
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
        let names: Vec<_> = r1
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, ["Aaa.php", "Mmm.php", "Zzz.php"]);
    }

    #[test]
    fn test_read_module_paths_includes_setup_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("setup/src/Magento/Setup")).unwrap();
        let paths = read_module_paths(root);
        assert!(paths.contains(&root.join("setup")));
    }

    #[test]
    fn test_read_module_paths_includes_nested_registration_paths() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let module_root = root.join("vendor/acme/pkg/src/deep/module-a");
        fs::create_dir_all(&module_root).unwrap();
        fs::write(module_root.join("registration.php"), "<?php").unwrap();

        let paths = read_module_paths(root);
        assert!(paths.contains(&module_root));
    }
}
