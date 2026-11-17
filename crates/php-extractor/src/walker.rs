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

    // lib/internal/*/* and lib/internal/*/*/*, as in app/etc/registration_globlist.php
    let lib_internal = magento_root.join("lib/internal");
    for vendor in subfolders(&lib_internal) {
        for package in subfolders(&vendor) {
            if registers_component(&package) {
                paths.push(package.clone());
            }
            for nested in subfolders(&package) {
                if registers_component(&nested) {
                    paths.push(nested);
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
    let composer_files =
        std::fs::read_to_string(vendor_dir.join("composer/autoload_files.php")).ok();
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
                        if composer_loads_registration(&composer_files, &vendor_dir, &module_root) {
                            paths.push(module_root);
                        }
                    }
                }
            }
        }
    }

    paths.sort();
    paths.dedup();
    paths
}

/// `ComponentRegistrar` only hears from a `registration.php` that Composer loads. A file of that
/// name elsewhere in a package is not a registration. Without the Composer list, the file counts.
fn composer_loads_registration(
    composer_files: &Option<String>,
    vendor_dir: &Path,
    module_root: &Path,
) -> bool {
    let (Some(list), Ok(package)) = (composer_files, module_root.strip_prefix(vendor_dir)) else {
        return true;
    };
    list.contains(&format!("/{}/registration.php'", package.display()))
}

/// `DiCompileCommand` scans the paths in `ComponentRegistrar`. A `registration.php` is only a plain
/// PHP file that Magento loads, so a folder is a component only when that file registers one.
fn registers_component(folder: &Path) -> bool {
    std::fs::read_to_string(folder.join("registration.php"))
        .is_ok_and(|registration| registration.contains("::register("))
}

fn subfolders(folder: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default()
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

#[derive(Clone, Copy)]
enum ComponentKind {
    Module,
    Library,
    Setup,
}

fn component_kind(root: &Path) -> ComponentKind {
    let Ok(registration) = std::fs::read_to_string(root.join("registration.php")) else {
        return if root.file_name().is_some_and(|name| name == "setup") {
            ComponentKind::Setup
        } else {
            ComponentKind::Module
        };
    };
    if registration.contains("ComponentRegistrar::LIBRARY") {
        ComponentKind::Library
    } else {
        ComponentKind::Module
    }
}

/// Starts with `Test` or `tests`. Magento's patterns have no end marker, so
/// `TestFramework` and `Testimonial` match as well.
fn is_test_segment(segment: &str) -> bool {
    segment.starts_with("Test") || segment.starts_with("tests")
}

fn is_word_segment(segment: &str) -> bool {
    segment
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The patterns of `DiCompileCommand::getExcludedModulePaths()`, `getExcludedLibraryPaths()`
/// and `getExcludedSetupPaths()`.
fn is_excluded_path(kind: ComponentKind, relative: &Path) -> bool {
    let mut segments = relative.iter().filter_map(|segment| segment.to_str());
    match kind {
        ComponentKind::Module => segments.next().is_some_and(is_test_segment),
        ComponentKind::Library => {
            let Some(first) = segments.next() else {
                return false;
            };
            is_test_segment(first)
                || (is_word_segment(first) && segments.next().is_some_and(is_test_segment))
        }
        ComponentKind::Setup => {
            for segment in segments {
                if segment.starts_with("Test") {
                    return true;
                }
                if !is_word_segment(segment) {
                    return false;
                }
            }
            false
        }
    }
}

fn collect_php_files(root: &Path, kind: Option<ComponentKind>) -> Vec<PathBuf> {
    use ignore::types::TypesBuilder;
    use ignore::WalkBuilder;

    let mut types_builder = TypesBuilder::new();
    types_builder.add("php", "*.php").unwrap();
    let php_types = types_builder.select("php").build().unwrap();

    let mut builder = WalkBuilder::new(root);
    // `ClassesScanner` knows no ignore files and walks into folders that start with a dot.
    builder.standard_filters(false);
    // `ClassesScanner::getList()` walks with `FilesystemIterator::FOLLOW_SYMLINKS`.
    builder.follow_links(true);
    builder.types(php_types);
    let walk_root = root.to_path_buf();
    builder.filter_entry(move |entry| {
        if entry.file_name() == "magento2-functional-testing-framework" {
            return false;
        }
        // `ClassesScanner::extract()` skips only files that start with a dot.
        let is_dir = entry
            .file_type()
            .is_some_and(|file_type| file_type.is_dir());
        if !is_dir && entry.file_name().as_encoded_bytes().starts_with(b".") {
            return false;
        }
        match (kind, entry.path().strip_prefix(&walk_root)) {
            (Some(kind), Ok(relative)) => !is_excluded_path(kind, relative),
            _ => true,
        }
    });
    builder
        .build()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                Some(entry.into_path())
            } else {
                None
            }
        })
        .collect()
}

/// Walk PHP files across the given module paths, without the paths Magento excludes.
pub fn walk_php_files(module_paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = module_paths
        .iter()
        .flat_map(|module_path| collect_php_files(module_path, Some(component_kind(module_path))))
        .collect();

    files.sort();
    files.dedup();
    files
}

/// Walk `generated/code`. Magento excludes nothing there, and `Foo\Test\Interceptor` is real.
pub fn walk_generated_php_files(code_root: &Path) -> Vec<PathBuf> {
    let mut files = collect_php_files(code_root, None);
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn names(files: &[PathBuf], root: &Path) -> Vec<String> {
        files
            .iter()
            .map(|file| file.strip_prefix(root).unwrap().display().to_string())
            .collect()
    }

    fn touch(root: &Path, relative: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "<?php").unwrap();
    }

    #[test]
    fn libraries_in_lib_internal_are_found_at_both_depths() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let registers =
            "<?php\nComponentRegistrar::register(ComponentRegistrar::LIBRARY, 'a/b', __DIR__);\n";
        for file in [
            "lib/internal/Acme/Lib/registration.php",
            "lib/internal/Acme/Deep/Lib/registration.php",
        ] {
            touch(root, file);
            fs::write(root.join(file), registers).unwrap();
        }
        touch(root, "lib/internal/Acme/Plain/Thing.php");

        let paths = read_module_paths(root);

        assert!(paths.contains(&root.join("lib/internal/Acme/Lib")));
        assert!(paths.contains(&root.join("lib/internal/Acme/Deep/Lib")));
        assert!(!paths.contains(&root.join("lib/internal/Acme/Plain")));
    }

    #[test]
    fn folder_in_lib_internal_whose_registration_registers_nothing_is_no_library() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        for file in [
            "lib/internal/Acme/Loader/registration.php",
            "lib/internal/Acme/Loader/Part/registration.php",
        ] {
            touch(root, file);
            fs::write(
                root.join(file),
                "<?php\n$loader = new AcmeLoader('Part', __DIR__);\n$loader->register();\n",
            )
            .unwrap();
        }

        let paths = read_module_paths(root);

        assert!(!paths.contains(&root.join("lib/internal/Acme/Loader")));
        assert!(!paths.contains(&root.join("lib/internal/Acme/Loader/Part")));
    }

    #[test]
    fn module_excludes_only_test_paths_at_its_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        touch(root, "registration.php");
        touch(root, "Controller/Adminhtml/Test/Run.php");
        touch(root, "Service/Tests/Check.php");
        touch(root, "Model/test/Lower.php");
        touch(root, "Model/tests/Lower.php");
        touch(root, "Test/Unit/FooTest.php");
        touch(root, "tests/bar_test.php");
        touch(root, "TestFramework/Bypass.php");
        touch(root, "Testimonial/Model.php");
        touch(root, "Tests/AlsoMatches.php");

        let files = walk_php_files(&[root.to_path_buf()]);

        assert_eq!(
            names(&files, root),
            [
                "Controller/Adminhtml/Test/Run.php",
                "Model/test/Lower.php",
                "Model/tests/Lower.php",
                "Service/Tests/Check.php",
                "registration.php",
            ]
        );
    }

    #[test]
    fn library_also_excludes_test_paths_one_level_down() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("registration.php"),
            "<?php\nuse Magento\\Framework\\Component\\ComponentRegistrar;\nComponentRegistrar::register(ComponentRegistrar::LIBRARY, 'a/b', __DIR__);\n",
        )
        .unwrap();
        touch(root, "Test/Root.php");
        touch(root, "App/Test/One.php");
        touch(root, "App/tests/One.php");
        touch(root, "App/Sub/Test/Two.php");

        let files = walk_php_files(&[root.to_path_buf()]);

        assert_eq!(
            names(&files, root),
            ["App/Sub/Test/Two.php", "registration.php"]
        );
    }

    #[test]
    fn setup_excludes_test_paths_at_any_depth() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("setup");
        touch(&root, "src/Magento/Setup/Test/Unit/A.php");
        touch(&root, "src/Magento/Setup/tests/B.php");
        touch(&root, "src/Magento/Setup/C.php");

        let files = walk_php_files(std::slice::from_ref(&root));

        assert_eq!(
            names(&files, &root),
            ["src/Magento/Setup/C.php", "src/Magento/Setup/tests/B.php"]
        );
    }

    #[test]
    fn generated_code_has_no_excluded_paths() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        touch(root, "Test/Interceptor.php");
        touch(root, "Acme/Service/Test/Interceptor.php");

        let files = walk_generated_php_files(root);

        assert_eq!(files.len(), 2);
    }

    /// `ClassesScanner::getList()` uses a plain `RecursiveDirectoryIterator`. It knows
    /// nothing of git, so a class hidden from git is still a class.
    #[test]
    fn ignore_files_do_not_hide_classes() {
        let tmp = TempDir::new().unwrap();
        let store = tmp.path();
        fs::create_dir_all(store.join(".git/info")).unwrap();
        fs::write(store.join(".git/info/exclude"), "Excluded/\n").unwrap();
        fs::write(store.join(".gitignore"), "StoreIgnored/\n").unwrap();
        let module = store.join("app/code/Acme/On");
        touch(&module, "registration.php");
        fs::write(module.join(".gitignore"), "Model/\n").unwrap();
        fs::write(module.join(".ignore"), "Api/\n").unwrap();
        touch(&module, "Model/Hidden.php");
        touch(&module, "Api/Hidden.php");
        touch(&module, "Excluded/Hidden.php");
        touch(&module, "StoreIgnored/Hidden.php");

        let files = walk_php_files(std::slice::from_ref(&module));

        assert_eq!(
            names(&files, &module),
            [
                "Api/Hidden.php",
                "Excluded/Hidden.php",
                "Model/Hidden.php",
                "StoreIgnored/Hidden.php",
                "registration.php",
            ]
        );
    }

    /// `ClassesScanner::extract()` skips a file whose name starts with a dot, and nothing
    /// else. The iterator walks into folders whose name starts with a dot.
    #[test]
    fn dot_files_are_skipped_and_dot_folders_are_read() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        touch(root, "registration.php");
        touch(root, ".php-cs-fixer.php");
        touch(root, ".ci/Tool.php");
        touch(root, ".ci/.hidden.php");

        let files = walk_php_files(&[root.to_path_buf()]);

        assert_eq!(names(&files, root), [".ci/Tool.php", "registration.php"]);
    }

    #[test]
    fn generated_code_ignores_ignore_files_too() {
        let tmp = TempDir::new().unwrap();
        let store = tmp.path();
        fs::create_dir_all(store.join(".git")).unwrap();
        let code = store.join("generated/code");
        touch(&code, "Acme/Foo/Interceptor.php");
        fs::write(code.join(".gitignore"), "Acme/\n").unwrap();

        let files = walk_generated_php_files(&code);

        assert_eq!(names(&files, &code), ["Acme/Foo/Interceptor.php"]);
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
