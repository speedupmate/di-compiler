//! TKT-012: DiConfig query methods replicating PHP ObjectManager/Config/Config.php behavior.
use std::collections::HashSet;

use crate::model::{Argument, DiConfig, Plugin};

impl DiConfig {
    /// Resolve the concrete implementation for a given interface or abstract class.
    ///
    /// Follows the preference chain with cycle detection.
    /// Returns the input type if no preference is configured.
    pub fn get_preference(&self, fqcn: &str) -> String {
        let mut visited = HashSet::new();
        let mut current = normalize(fqcn);
        loop {
            if visited.contains(&current) {
                // Cycle detected — return what we have
                log::warn!("Circular preference chain detected for: {current}");
                return current;
            }
            visited.insert(current.clone());
            match preference_value_case_insensitive(self, &current) {
                Some(next) => {
                    let next = normalize(next);
                    if next == current {
                        return current;
                    }
                    current = next;
                }
                None => return current,
            }
        }
    }

    /// Resolve the concrete class for a virtual type (following the `type` attribute chain).
    ///
    /// If `name` is a virtualType, returns the concrete type_name.
    /// Otherwise returns `name` unchanged.
    pub fn get_instance_type(&self, name: &str) -> String {
        let mut visited = HashSet::new();
        let mut current = normalize(name);
        loop {
            if visited.contains(&current) {
                log::warn!("Circular virtualType chain for: {current}");
                return current;
            }
            visited.insert(current.clone());
            match self.virtual_types.get(&current) {
                Some(vt) => {
                    let next = normalize(&vt.type_name);
                    if next.is_empty() || next == current {
                        return current;
                    }
                    current = next;
                }
                None => return current,
            }
        }
    }

    /// Return the configured `<argument>` list for a type.
    pub fn get_arguments(&self, type_name: &str) -> Vec<&Argument> {
        let name = normalize(type_name);
        type_config_case_insensitive(self, &name)
            .map(|tc| tc.arguments.iter().collect())
            .unwrap_or_default()
    }

    /// Whether the type is shared (singleton). Defaults to `true`.
    pub fn is_shared(&self, fqcn: &str) -> bool {
        let name = normalize(fqcn);
        type_config_case_insensitive(self, &name)
            .and_then(|c| c.shared)
            .unwrap_or(true)
    }

    /// Get active (non-disabled) plugins for a type, sorted by sort_order then name.
    pub fn get_active_plugins(&self, fqcn: &str) -> Vec<&Plugin> {
        let name = normalize(fqcn);
        let mut plugins: Vec<&Plugin> = self
            .plugins
            .get(&name)
            .map(|ps| ps.iter().filter(|p| !p.disabled).collect())
            .unwrap_or_default();
        plugins.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then(a.name.cmp(&b.name)));
        plugins
    }

    /// Whether any class or virtual type with this name exists in the config.
    pub fn class_exists_in_config(&self, fqcn: &str) -> bool {
        let name = normalize(fqcn);
        self.type_configs.contains_key(&name) || self.virtual_types.contains_key(&name)
    }

    /// Walk all di.xml files for a Magento install in correct merge order.
    ///
    /// Order: app/etc (primary) → vendor/magento → vendor/* (non-magento) → app/code
    pub fn load_from_magento_root(
        magento_root: &std::path::Path,
    ) -> Result<DiConfig, crate::Error> {
        use crate::parser::parse_di_xml_impl;

        let mut configs = Vec::new();
        let di_xml_paths = find_di_xml_files(magento_root, &std::collections::HashMap::new());
        for path in di_xml_paths {
            match parse_di_xml_impl(&path) {
                Ok(c) => configs.push(c),
                Err(e) => {
                    log::warn!("Failed to parse {}: {e}", path.display());
                }
            }
        }
        Ok(crate::merger::merge_configs(configs))
    }
}

fn preference_value_case_insensitive<'a>(config: &'a DiConfig, key: &str) -> Option<&'a String> {
    config.preferences.get(key).or_else(|| {
        let key_lower = key.to_ascii_lowercase();
        config
            .preference_keys_lc
            .get(&key_lower)
            .and_then(|canonical| config.preferences.get(canonical))
    })
}

fn type_config_case_insensitive<'a>(
    config: &'a DiConfig,
    key: &str,
) -> Option<&'a crate::model::TypeConfig> {
    config.type_configs.get(key).or_else(|| {
        let key_lower = key.to_ascii_lowercase();
        config
            .type_config_keys_lc
            .get(&key_lower)
            .and_then(|canonical| config.type_configs.get(canonical))
    })
}

/// Collect **global-only** di.xml files in Magento merge order.
///
/// Only includes `etc/di.xml` files (not area-specific `etc/{area}/di.xml`).
/// Merge order: app/etc (primary) → vendor/magento/* → vendor/*/* → app/code/*/*
/// Within each priority tier, files are ordered by Magento module load order
/// from `app/etc/config.php` (the `module_order` map), then by path.
pub fn find_di_xml_files(
    magento_root: &std::path::Path,
    module_order: &std::collections::HashMap<String, usize>,
) -> Vec<std::path::PathBuf> {
    collect_di_xml_files(magento_root, None, module_order)
}

/// Collect di.xml files for a specific area in Magento merge order.
///
/// Returns the global `etc/di.xml` files **plus** `etc/{area}/di.xml` overlays.
/// Area files are loaded after global files with the same source priority, so
/// they override global settings for that area.
pub fn find_di_xml_files_for_area(
    magento_root: &std::path::Path,
    area: &str,
    module_order: &std::collections::HashMap<String, usize>,
) -> Vec<std::path::PathBuf> {
    collect_di_xml_files(magento_root, Some(area), module_order)
}

/// Collect ALL di.xml files from all areas (global + every area).
///
/// Used when detecting which classes need interceptors/factories/proxies.
/// Magento determines interception requirements by considering plugin
/// registrations across ALL areas, not just the global scope.
pub fn find_all_di_xml_files(
    magento_root: &std::path::Path,
    module_order: &std::collections::HashMap<String, usize>,
) -> Vec<std::path::PathBuf> {
    const AREAS: &[&str] = &[
        "global",
        "frontend",
        "adminhtml",
        "crontab",
        "webapi_rest",
        "webapi_soap",
        "graphql",
    ];
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    // Global first
    for p in collect_di_xml_files(magento_root, None, module_order) {
        if seen.insert(p.clone()) {
            result.push(p);
        }
    }
    // Then each area overlay
    for area in AREAS {
        for p in collect_di_xml_files(magento_root, Some(area), module_order) {
            if seen.insert(p.clone()) {
                result.push(p);
            }
        }
    }
    result
}

/// Internal: collect di.xml files with optional area overlay.
///
/// Each entry is `(priority, module_idx, path)` where priority controls the
/// broad merge tier and module_idx is the Magento module load order index from
/// `app/etc/config.php`, providing deterministic tie-breaking within each tier.
fn collect_di_xml_files(
    magento_root: &std::path::Path,
    area: Option<&str>,
    module_order: &std::collections::HashMap<String, usize>,
) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<(u8, usize, std::path::PathBuf)> = Vec::new();

    fn push_module_di_paths(
        module_root: &std::path::Path,
        priority: u8,
        module_idx: usize,
        area: Option<&str>,
        out: &mut Vec<(u8, usize, std::path::PathBuf)>,
    ) {
        let global = module_root.join("etc/di.xml");
        if global.exists() {
            out.push((priority, module_idx, global));
        }
        if let Some(a) = area {
            let area_di = module_root.join(format!("etc/{}/di.xml", a));
            if area_di.exists() {
                out.push((priority, module_idx, area_di));
            }
        }
    }

    fn should_skip_scan_dir(name: &str) -> bool {
        matches!(
            name,
            ".git" | "node_modules" | "Test" | "Tests" | "test" | "tests" | "dev"
        )
    }

    fn discover_module_roots_from_registration(
        package_root: &std::path::Path,
    ) -> Vec<std::path::PathBuf> {
        let mut roots = Vec::new();
        let mut stack: Vec<(std::path::PathBuf, usize)> = vec![(package_root.to_path_buf(), 0)];
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

    // Collect `<base>/<module>/etc/di.xml` and optionally `<base>/<module>/etc/<area>/di.xml`.
    // For third-party packages, also honor nested module roots discovered via registration.php.
    fn collect_vendor(
        base: &std::path::Path,
        priority: u8,
        area: Option<&str>,
        module_order: &std::collections::HashMap<String, usize>,
        out: &mut Vec<(u8, usize, std::path::PathBuf)>,
        discover_nested_modules: bool,
    ) {
        if !base.is_dir() {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(base) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    // Direct module root (classic vendor/{vendor}/{module} layout)
                    let module_idx = cached_module_name(&p)
                        .and_then(|n| module_order.get(&n).copied())
                        .unwrap_or(usize::MAX);
                    push_module_di_paths(&p, priority, module_idx, area, out);

                    // Nested module roots, e.g. package/src/<module>/registration.php
                    if discover_nested_modules {
                        for module_root in cached_discover_module_roots(&p) {
                            let nested_idx = cached_module_name(&module_root)
                                .and_then(|n| module_order.get(&n).copied())
                                .unwrap_or(usize::MAX);
                            push_module_di_paths(&module_root, priority, nested_idx, area, out);
                        }
                    }
                }
            }
        }
    }

    /// Cached wrapper around `discover_module_roots_from_registration`.
    ///
    /// `collect_di_xml_files` is called up to 16 times per process run (Phase 3a,
    /// Phase 3b × 8 areas, Phase 7 × 7 areas) for the same package roots. Without
    /// caching this means 1,392 DFS traversals across 87 non-magento vendor packages.
    /// The cache reduces that to at most 87 traversals (one per unique package root).
    fn cached_discover_module_roots(package_root: &std::path::Path) -> Vec<std::path::PathBuf> {
        use std::sync::{Mutex, OnceLock};
        static CACHE: OnceLock<
            Mutex<std::collections::HashMap<std::path::PathBuf, Vec<std::path::PathBuf>>>,
        > = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
        {
            let map = cache.lock().unwrap();
            if let Some(cached) = map.get(package_root) {
                return cached.clone();
            }
        }
        let roots = discover_module_roots_from_registration(package_root);
        cache
            .lock()
            .unwrap()
            .insert(package_root.to_path_buf(), roots.clone());
        roots
    }

    // Priority 0: app/etc/di.xml (primary scope)
    let app_etc_di = magento_root.join("app/etc/di.xml");
    if app_etc_di.exists() {
        paths.push((0, 0, app_etc_di));
    }

    // Priority 1: vendor/magento/*
    collect_vendor(
        &magento_root.join("vendor/magento"),
        1,
        area,
        module_order,
        &mut paths,
        false,
    );

    // Priority 2: other vendor/vendor/* (non-magento)
    if let Ok(vendors) = std::fs::read_dir(magento_root.join("vendor")) {
        for vendor in vendors.flatten() {
            if vendor
                .file_name()
                .to_str()
                .map(|s| s == "magento")
                .unwrap_or(false)
            {
                continue;
            }
            collect_vendor(&vendor.path(), 2, area, module_order, &mut paths, true);
        }
    }

    // Priority 3: app/code/*/*/etc/di.xml
    if let Ok(vendors) = std::fs::read_dir(magento_root.join("app/code")) {
        for vendor in vendors.flatten() {
            if let Ok(modules) = std::fs::read_dir(vendor.path()) {
                for module in modules.flatten() {
                    let module_idx = cached_module_name(&module.path())
                        .and_then(|n| module_order.get(&n).copied())
                        .unwrap_or(usize::MAX);
                    let global = module.path().join("etc/di.xml");
                    if global.exists() {
                        paths.push((3, module_idx, global));
                    }
                    if let Some(a) = area {
                        let area_di = module.path().join(format!("etc/{}/di.xml", a));
                        if area_di.exists() {
                            paths.push((3, module_idx, area_di));
                        }
                    }
                }
            }
        }
    }

    // Sort by (priority, module_idx, path) for deterministic Magento-order merging
    paths.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    paths.into_iter().map(|(_, _, p)| p).collect()
}

fn normalize(s: &str) -> String {
    s.trim().trim_start_matches('\\').to_string()
}

/// Read the module name from `etc/module.xml` using a simple string search.
///
/// Returns `None` if the file doesn't exist or has no `name="..."` attribute.
fn module_name_from_module_xml(module_root: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(module_root.join("etc/module.xml")).ok()?;
    let name_pos = content.find("name=\"")?;
    let rest = &content[name_pos + 6..];
    let end_pos = rest.find('"')?;
    let name = &rest[..end_pos];
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Cached wrapper around `module_name_from_module_xml`.
///
/// Called for every module root during DI collection — caching avoids
/// re-reading the same module.xml files on repeated `collect_di_xml_files` calls.
fn cached_module_name(module_root: &std::path::Path) -> Option<String> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<std::collections::HashMap<std::path::PathBuf, Option<String>>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    {
        let map = cache.lock().unwrap();
        if let Some(cached) = map.get(module_root) {
            return cached.clone();
        }
    }
    let name = module_name_from_module_xml(module_root);
    cache
        .lock()
        .unwrap()
        .insert(module_root.to_path_buf(), name.clone());
    name
}

#[cfg(test)]
mod tests {

    use crate::model::{DiConfig, Plugin};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_get_preference_simple() {
        let mut config = DiConfig::default();
        config.preferences.insert("Iface".into(), "Impl".into());
        assert_eq!(config.get_preference("Iface"), "Impl");
    }

    #[test]
    fn test_get_preference_chain() {
        let mut config = DiConfig::default();
        config.preferences.insert("A".into(), "B".into());
        config.preferences.insert("B".into(), "C".into());
        assert_eq!(config.get_preference("A"), "C");
    }

    #[test]
    fn test_get_preference_no_cycle_panic() {
        let mut config = DiConfig::default();
        config.preferences.insert("A".into(), "B".into());
        config.preferences.insert("B".into(), "A".into()); // cycle
                                                           // Should not panic or loop forever
        let _ = config.get_preference("A");
    }

    #[test]
    fn test_get_preference_not_found() {
        let config = DiConfig::default();
        assert_eq!(config.get_preference("SomeClass"), "SomeClass");
    }

    #[test]
    fn test_get_instance_type_virtual() {
        let mut config = DiConfig::default();
        config.virtual_types.insert(
            "MyVirtual".into(),
            crate::model::VirtualType {
                name: "MyVirtual".into(),
                type_name: "ConcreteClass".into(),
            },
        );
        assert_eq!(config.get_instance_type("MyVirtual"), "ConcreteClass");
    }

    #[test]
    fn test_is_shared_default() {
        let config = DiConfig::default();
        assert!(config.is_shared("AnyClass")); // default true
    }

    #[test]
    fn test_get_active_plugins_sorted() {
        let mut config = DiConfig::default();
        config.plugins.insert(
            "Foo".into(),
            vec![
                Plugin {
                    name: "c".into(),
                    type_name: "TC".into(),
                    sort_order: 30,
                    disabled: false,
                },
                Plugin {
                    name: "b".into(),
                    type_name: "TB".into(),
                    sort_order: 10,
                    disabled: false,
                },
                Plugin {
                    name: "a".into(),
                    type_name: "TA".into(),
                    sort_order: 10,
                    disabled: true,
                },
            ],
        );
        let active = config.get_active_plugins("Foo");
        assert_eq!(active.len(), 2); // 'a' is disabled
        assert_eq!(active[0].name, "b"); // lower sort_order first
        assert_eq!(active[1].name, "c");
    }

    #[test]
    fn test_find_di_xml_files_includes_nested_registration_modules() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let module_root = root.join("vendor/acme/pkg/src/magento-cms");
        fs::create_dir_all(module_root.join("etc/frontend")).unwrap();
        fs::write(module_root.join("registration.php"), "<?php").unwrap();
        fs::write(
            module_root.join("etc/di.xml"),
            r#"<?xml version="1.0"?><config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"></config>"#,
        )
        .unwrap();
        fs::write(
            module_root.join("etc/frontend/di.xml"),
            r#"<?xml version="1.0"?><config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"></config>"#,
        )
        .unwrap();

        let empty_order = std::collections::HashMap::new();
        let global = super::find_di_xml_files(root, &empty_order);
        assert!(global.contains(&module_root.join("etc/di.xml")));

        let frontend = super::find_di_xml_files_for_area(root, "frontend", &empty_order);
        assert!(frontend.contains(&module_root.join("etc/frontend/di.xml")));
    }
}
