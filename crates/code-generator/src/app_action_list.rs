//! TKT-033: app_action_list.php generation.

use rustc_hash::FxHashMap;
use std::collections::BTreeMap;

use php_extractor::ClassInfo;

use crate::metadata::escape_php;

/// Build and serialize Magento's `app_action_list.php` metadata map.
///
/// Keys are lower-cased FQCNs for classes in a `\Controller\` namespace segment.
pub fn generate_app_action_list_php(class_map: &FxHashMap<String, ClassInfo>) -> String {
    let entries = collect_action_entries(class_map);
    serialize_app_action_list_php(&entries)
}

fn collect_action_entries(class_map: &FxHashMap<String, ClassInfo>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for info in class_map.values() {
        let fqcn = &info.fqcn;
        if !fqcn.to_ascii_lowercase().contains("\\controller\\") {
            continue;
        }
        if !is_controller_file_path(&info.path) {
            continue;
        }
        if is_framework_library_class(&info.path) {
            continue;
        }
        out.insert(
            fqcn.to_ascii_lowercase(),
            spelled_as_on_disk(fqcn, &info.path),
        );
    }
    out
}

/// `Dir\Reader::getActionFiles()` builds the class name from the file path, so the value has the
/// spelling of the disk. `ActionList` loads the class by that name, and on a case-sensitive file
/// system the autoloader only finds that spelling.
fn spelled_as_on_disk(fqcn: &str, path: &std::path::Path) -> String {
    let mut on_disk: Vec<String> = Vec::new();
    if let Some(stem) = path.file_stem() {
        on_disk.push(stem.to_string_lossy().to_string());
    }
    let folders = path
        .parent()
        .into_iter()
        .flat_map(|folder| folder.components().rev());
    on_disk.extend(folders.map(|part| part.as_os_str().to_string_lossy().to_string()));
    let mut parts: Vec<String> = fqcn.split('\\').map(str::to_string).collect();
    for (part, disk) in parts.iter_mut().rev().zip(on_disk) {
        if !part.eq_ignore_ascii_case(&disk) {
            break;
        }
        let stop = disk.eq_ignore_ascii_case("Controller");
        *part = disk;
        if stop {
            break;
        }
    }
    parts.join("\\")
}

fn is_controller_file_path(path: &std::path::Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("Controller")
    })
}

fn is_framework_library_class(path: &std::path::Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized.contains("/vendor/magento/framework/")
}

pub fn serialize_app_action_list_php(entries: &BTreeMap<String, String>) -> String {
    let mut out = String::from("<?php return array (\n");
    for (key, fqcn) in entries {
        out.push_str(&format!(
            "  '{}' => '{}',\n",
            escape_php(key),
            escape_php(fqcn)
        ));
    }
    out.push_str(");\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use php_extractor::types::{ClassInfo, ClassKind};
    use std::path::PathBuf;

    #[test]
    fn value_has_the_spelling_of_the_file_on_disk() {
        let mut classes = FxHashMap::default();
        classes.insert(
            "Acme\\On\\Controller\\Main\\Index".to_string(),
            class_info(
                "Acme\\On\\Controller\\Main\\Index",
                "/store/vendor/acme/on/Controller/Main/index.php",
            ),
        );
        let entries = collect_action_entries(&classes);
        assert_eq!(
            entries["acme\\on\\controller\\main\\index"],
            "Acme\\On\\Controller\\Main\\index"
        );
    }

    fn class_info(fqcn: &str, path: &str) -> ClassInfo {
        let parts: Vec<&str> = fqcn.rsplitn(2, '\\').collect();
        let (name, namespace) = if parts.len() == 2 {
            (parts[0].to_string(), parts[1].to_string())
        } else {
            (fqcn.to_string(), String::new())
        };
        ClassInfo {
            path: PathBuf::from(path),
            namespace,
            name,
            fqcn: fqcn.to_string(),
            kind: ClassKind::Class,
            extends: None,
            implements: vec![],
            constructor: None,
            is_abstract: false,
            is_final: false,
            public_methods: vec![],
            traits: Default::default(),
        }
    }

    #[test]
    fn test_collects_only_controller_namespaces() {
        let mut classes = FxHashMap::default();
        classes.insert(
            "Magento\\Backend\\Controller\\Adminhtml\\Cache\\Index".to_string(),
            class_info(
                "Magento\\Backend\\Controller\\Adminhtml\\Cache\\Index",
                "/var/www/application/vendor/magento/module-backend/Controller/Adminhtml/Cache/Index.php",
            ),
        );
        classes.insert(
            "Magento\\Backend\\Model\\Config".to_string(),
            class_info(
                "Magento\\Backend\\Model\\Config",
                "/var/www/application/vendor/magento/module-backend/Model/Config.php",
            ),
        );
        classes.insert(
            "Magento\\Framework\\Controller\\Result\\Forward".to_string(),
            class_info(
                "Magento\\Framework\\Controller\\Result\\Forward",
                "/var/www/application/vendor/magento/framework/Controller/Result/Forward.php",
            ),
        );

        let out = generate_app_action_list_php(&classes);
        assert!(out.contains(
            "'magento\\\\backend\\\\controller\\\\adminhtml\\\\cache\\\\index' => 'Magento\\\\Backend\\\\Controller\\\\Adminhtml\\\\Cache\\\\Index'"
        ));
        assert!(!out.contains("magento\\\\backend\\\\model\\\\config"));
        assert!(!out.contains("magento\\\\framework\\\\controller\\\\result\\\\forward"));
    }

    #[test]
    fn test_serialization_format() {
        let mut entries = BTreeMap::new();
        entries.insert(
            "foo\\controller\\bar\\index".to_string(),
            "Foo\\Controller\\Bar\\Index".to_string(),
        );
        let out = serialize_app_action_list_php(&entries);
        assert!(out.starts_with("<?php return array (\n"));
        assert!(out.ends_with(");\n"));
    }
}
