//! The folders watch mode watches, and which changes in them call for a new compile.
//!
//! Only folders are watched. A watch on a single file ends when an editor or Composer replaces the
//! file through a rename, and a new file has no watch yet. Events are filtered by path instead.

use std::path::{Path, PathBuf};

use di_xml_reader::{composer_registered_module_roots, registered_component_roots};
use php_extractor::read_module_paths;

/// A folder watched without its subfolders, for the named files in it only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredFolder {
    pub folder: PathBuf,
    pub names: Vec<&'static str>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchSet {
    /// Watched with all subfolders.
    pub recursive: Vec<PathBuf>,
    /// Watched one level deep.
    pub filtered: Vec<FilteredFolder>,
}

impl WatchSet {
    pub fn folder_count(&self) -> usize {
        self.recursive.len() + self.filtered.len()
    }
}

/// The files Composer writes when packages change. Composer replaces them by rename.
const COMPOSER_FILES: [&str; 2] = ["installed.json", "autoload_files.php"];

/// The watch set for the store at `magento_root`, from the same module discovery a compile uses.
///
/// - `app/code/` and `lib/internal/`, with all subfolders;
/// - a module outside `vendor/`, reached through a symlink in `vendor/` (a Composer path
///   repository), with all subfolders, at its real path;
/// - the `etc/` folder of every module inside `vendor/`, with all subfolders, since vendor code
///   changes through Composer and only `di.xml` there changes by hand;
/// - `app/etc/` with all subfolders, for `di.xml`, `config.php` and `*/di.xml`;
/// - `vendor/composer/` for the Composer files.
pub fn resolve(magento_root: &Path) -> WatchSet {
    let root = real(magento_root);
    let vendor = root.join("vendor");
    let mut recursive: Vec<PathBuf> = ["app/code", "lib/internal", "app/etc"]
        .iter()
        .map(|folder| root.join(folder))
        .filter(|folder| folder.is_dir())
        .collect();

    let mut module_roots = read_module_paths(&root);
    module_roots.extend(composer_registered_module_roots(&root));
    let mut module_roots: Vec<PathBuf> = module_roots
        .iter()
        .flat_map(|path| registered_component_roots(path))
        .collect();
    module_roots.sort();
    module_roots.dedup();
    for module in module_roots {
        let module = real(&module);
        let folder = if module.starts_with(&vendor) {
            module.join("etc")
        } else {
            module
        };
        if folder.is_dir() {
            recursive.push(folder);
        }
    }
    recursive.sort();
    recursive.dedup();
    let recursive = without_nested(recursive);

    let filtered = [(vendor.join("composer"), COMPOSER_FILES.to_vec())]
        .into_iter()
        .filter(|(folder, _)| folder.is_dir())
        .map(|(folder, names)| FilteredFolder { folder, names })
        .collect();

    WatchSet {
        recursive,
        filtered,
    }
}

/// True when a change of `path` can change the compile result. `is_folder` is true when the event
/// is about a folder (a module folder created, removed or renamed).
pub fn is_relevant(path: &Path, is_folder: bool, magento_root: &Path) -> bool {
    let root = real(magento_root);
    for ignored in ["generated", "var", "pub/static"] {
        if path.starts_with(root.join(ignored)) {
            return false;
        }
    }
    if path
        .components()
        .any(|part| matches!(part.as_os_str().to_str(), Some(".git" | "node_modules")))
    {
        return false;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if is_editor_temporary(name) {
        return false;
    }
    if let Ok(inside) = path.strip_prefix(root.join("app/etc")) {
        return is_relevant_in_app_etc(inside, is_folder);
    }
    if is_folder {
        return true;
    }
    let parent = path.parent().unwrap_or(Path::new(""));
    if parent == root.join("vendor/composer") {
        return COMPOSER_FILES.contains(&name);
    }
    name.ends_with(".php") || matches!(name, "di.xml" | "module.xml")
}

/// The compiler reads `app/etc/di.xml` and `app/etc/*/di.xml`, the glob of Magento's primary
/// `FileResolver`, whose `*` skips hidden folders. `config.php` holds the module list.
fn is_relevant_in_app_etc(inside: &Path, is_folder: bool) -> bool {
    let parts: Vec<&str> = inside.iter().filter_map(|part| part.to_str()).collect();
    match parts.as_slice() {
        [name] if !is_folder => matches!(*name, "di.xml" | "config.php"),
        [folder] => !folder.starts_with('.'),
        [folder, "di.xml"] => !is_folder && !folder.starts_with('.'),
        _ => false,
    }
}

/// True when the change is a Composer change, after which the module list itself can differ.
pub fn is_composer_change(path: &Path, magento_root: &Path) -> bool {
    let root = real(magento_root);
    path.parent() == Some(root.join("vendor/composer").as_path())
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| COMPOSER_FILES.contains(&name))
}

/// True when the change is the module list in `app/etc/config.php`, which needs
/// `cache:clean config` after the compile.
pub fn is_module_list_change(path: &Path, magento_root: &Path) -> bool {
    path == real(magento_root).join("app/etc/config.php")
}

fn is_editor_temporary(name: &str) -> bool {
    name.ends_with(".swp")
        || name.ends_with(".swx")
        || name.ends_with('~')
        || name.ends_with(".tmp")
        || name.starts_with(".#")
        || name.contains("___jb_")
}

fn real(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Drops every folder that sits inside another folder of the sorted list.
fn without_nested(sorted: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut kept: Vec<PathBuf> = Vec::new();
    for folder in sorted {
        if !kept.iter().any(|outer| folder.starts_with(outer)) {
            kept.push(folder);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn store() -> (tempfile::TempDir, tempfile::TempDir) {
        let store = tempfile::TempDir::new().unwrap();
        let packages = tempfile::TempDir::new().unwrap();
        let root = store.path();
        write(root, "app/etc/config.php", "<?php return [];");
        write(root, "app/code/Acme/On/registration.php", "<?php");
        write(root, "app/code/Acme/On/etc/di.xml", "<config/>");
        write(root, "vendor/magento/module-a/registration.php", "<?php");
        write(root, "vendor/magento/module-a/etc/di.xml", "<config/>");
        write(root, "vendor/magento/module-a/Model/A.php", "<?php");
        write(root, "vendor/magento/module-b/registration.php", "<?php");
        write(root, "vendor/composer/installed.json", "{}");
        write(packages.path(), "local-module/registration.php", "<?php");
        write(packages.path(), "local-module/etc/di.xml", "<config/>");
        fs::create_dir_all(root.join("vendor/acme")).unwrap();
        std::os::unix::fs::symlink(
            packages.path().join("local-module"),
            root.join("vendor/acme/local-module"),
        )
        .unwrap();
        write(
            root,
            "vendor/composer/autoload_files.php",
            "<?php\nreturn array(\n    'x' => $vendorDir . '/acme/local-module/registration.php',\n);\n",
        );
        (store, packages)
    }

    #[test]
    fn watches_folders_from_the_module_discovery() {
        let (store, packages) = store();
        let root = real(store.path());
        let set = resolve(store.path());
        assert!(set.recursive.contains(&root.join("app/code")));
        assert!(set
            .recursive
            .contains(&root.join("vendor/magento/module-a/etc")));
        assert!(set
            .recursive
            .contains(&real(&packages.path().join("local-module"))));
        // Inside app/code, so covered by its watch.
        assert!(!set.recursive.contains(&root.join("app/code/Acme/On")));
        // No etc/ folder, nothing to watch.
        assert!(!set
            .recursive
            .iter()
            .any(|folder| folder.starts_with(root.join("vendor/magento/module-b"))));
        // Vendor PHP changes through Composer only.
        assert!(!set
            .recursive
            .contains(&root.join("vendor/magento/module-a")));
        assert!(set.recursive.contains(&root.join("app/etc")));
        assert_eq!(
            set.filtered,
            vec![FilteredFolder {
                folder: root.join("vendor/composer"),
                names: vec!["installed.json", "autoload_files.php"],
            }]
        );
        assert!(!set
            .recursive
            .iter()
            .any(|folder| folder.starts_with(root.join("generated"))));
    }

    #[test]
    fn filters_changes_by_path() {
        let (store, _packages) = store();
        let root = real(store.path());
        let relevant = |relative: &str| is_relevant(&root.join(relative), false, &root);
        assert!(relevant("app/code/Acme/On/etc/frontend/di.xml"));
        assert!(relevant("app/code/Acme/On/Model/Thing.php"));
        assert!(relevant("app/code/Acme/On/etc/module.xml"));
        assert!(relevant("app/etc/config.php"));
        assert!(relevant("app/etc/di.xml"));
        assert!(relevant("vendor/composer/installed.json"));
        assert!(relevant("vendor/composer/autoload_files.php"));

        assert!(relevant("app/etc/local/di.xml"));
        assert!(is_relevant(&root.join("app/etc/local"), true, &root));

        assert!(!relevant("app/etc/env.php"));
        assert!(!relevant("app/etc/local/env.php"));
        assert!(!relevant("app/etc/local/deeper/di.xml"));
        assert!(!relevant("app/etc/.hidden/di.xml"));
        assert!(!is_relevant(&root.join("app/etc/.hidden"), true, &root));
        assert!(!relevant("vendor/composer/autoload_classmap.php"));
        assert!(!relevant("app/code/Acme/On/etc/acl.xml"));
        assert!(!relevant("app/code/Acme/On/view/frontend/web/js/a.js"));
        assert!(!relevant("generated/metadata/global.php"));
        assert!(!relevant("generated/code/Acme/On/Model/ThingFactory.php"));
        assert!(!relevant("var/cache/x.php"));
        assert!(!relevant("pub/static/frontend/x.php"));
        assert!(!relevant("app/code/Acme/On/.git/config.php"));
        assert!(!relevant("app/code/Acme/On/Model/.Thing.php.swp"));
        assert!(!relevant("app/code/Acme/On/Model/Thing.php~"));
        assert!(!relevant("app/code/Acme/On/Model/.#Thing.php"));
        assert!(!relevant("app/code/Acme/On/Model/Thing.php___jb_tmp___"));
        assert!(!relevant("app/code/Acme/On/etc/.di.xml.123-4.tmp"));

        assert!(is_relevant(&root.join("app/code/Acme/Gone"), true, &root));
        assert!(!is_relevant(&root.join("generated/code/Acme"), true, &root));
    }

    #[test]
    fn tells_composer_and_module_list_changes_apart() {
        let (store, _packages) = store();
        let root = real(store.path());
        assert!(is_composer_change(
            &root.join("vendor/composer/installed.json"),
            &root
        ));
        assert!(!is_composer_change(&root.join("app/etc/config.php"), &root));
        assert!(is_module_list_change(
            &root.join("app/etc/config.php"),
            &root
        ));
        assert!(!is_module_list_change(&root.join("app/etc/di.xml"), &root));
    }

    #[test]
    fn nested_folders_are_dropped() {
        let folders = vec![
            PathBuf::from("/a"),
            PathBuf::from("/a/b"),
            PathBuf::from("/ab"),
        ];
        assert_eq!(
            without_nested(folders),
            vec![PathBuf::from("/a"), PathBuf::from("/ab")]
        );
    }
}
