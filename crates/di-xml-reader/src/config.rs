//! TKT-012: DiConfig query methods replicating PHP ObjectManager/Config/Config.php behavior.
use rustc_hash::{FxHashMap, FxHashSet};

use crate::model::{Argument, DiConfig, Plugin};

impl DiConfig {
    /// Resolve the concrete implementation for a given interface or abstract class.
    ///
    /// Follows the preference chain with cycle detection.
    /// Returns the input type if no preference is configured.
    pub fn get_preference(&self, fqcn: &str) -> String {
        let mut visited = FxHashSet::default();
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
        let mut visited = FxHashSet::default();
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

    /// Get active (non-disabled) plugins for a type, sorted by sort_order. Plugins with
    /// equal sort_order keep merge order, as PHP's stable uasort does.
    pub fn get_active_plugins(&self, fqcn: &str) -> Vec<&Plugin> {
        let name = normalize(fqcn);
        let mut plugins: Vec<&Plugin> = self
            .plugins
            .get(&name)
            .map(|ps| ps.iter().filter(|p| p.is_active()).collect())
            .unwrap_or_default();
        plugins.sort_by_key(|plugin| plugin.effective_sort_order());
        plugins
    }

    /// Whether any class or virtual type with this name exists in the config.
    pub fn class_exists_in_config(&self, fqcn: &str) -> bool {
        let name = normalize(fqcn);
        self.type_configs.contains_key(&name) || self.virtual_types.contains_key(&name)
    }

    /// Walk all di.xml files for a Magento install in correct merge order.
    ///
    /// Order: app/etc (primary), then modules in `app/etc/config.php` order.
    pub fn load_from_magento_root(
        magento_root: &std::path::Path,
    ) -> Result<DiConfig, crate::Error> {
        use crate::parser::parse_di_xml_unfolded;

        let mut configs = Vec::new();
        let di_xml_paths = find_di_xml_files(magento_root, &FxHashMap::default());
        for path in di_xml_paths {
            match parse_di_xml_unfolded(&path) {
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
/// Merge order: app/etc (primary), then modules by load order from `app/etc/config.php`
/// (the `module_order` map), wherever the module lives, then by path. This is
/// `Module\Dir\Reader::getFiles()` walking `ModuleList::getNames()`.
pub fn find_di_xml_files(
    magento_root: &std::path::Path,
    module_order: &FxHashMap<String, usize>,
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
    module_order: &FxHashMap<String, usize>,
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
    module_order: &FxHashMap<String, usize>,
    areas: &[&str],
) -> Vec<std::path::PathBuf> {
    let mut seen = FxHashSet::default();
    let mut result = Vec::new();
    // Global first
    for p in collect_di_xml_files(magento_root, None, module_order) {
        if seen.insert(p.clone()) {
            result.push(p);
        }
    }
    // Then each area overlay
    for area in areas {
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
/// Each entry is `(priority, module_idx, path)` where priority puts the primary files
/// first and module_idx is the Magento module load order index from `app/etc/config.php`.
fn collect_di_xml_files(
    magento_root: &std::path::Path,
    area: Option<&str>,
    module_order: &FxHashMap<String, usize>,
) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<(u8, usize, std::path::PathBuf)> = Vec::new();

    fn push_module_di_paths(
        module_root: &std::path::Path,
        priority: u8,
        module_idx: usize,
        area: Option<&str>,
        out: &mut Vec<(u8, usize, std::path::PathBuf)>,
    ) {
        for root in registered_component_roots(module_root) {
            let global = root.join("etc/di.xml");
            if global.exists() {
                out.push((priority, module_idx, global));
            }
            if let Some(a) = area {
                let area_di = root.join(format!("etc/{}/di.xml", a));
                if area_di.exists() {
                    out.push((priority, module_idx, area_di));
                }
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
        module_order: &FxHashMap<String, usize>,
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
        static CACHE: OnceLock<Mutex<FxHashMap<std::path::PathBuf, Vec<std::path::PathBuf>>>> =
            OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(FxHashMap::default()));
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

    // Priority 0: app/etc/di.xml and app/etc/*/di.xml (primary scope)
    let app_etc_di = magento_root.join("app/etc/di.xml");
    if app_etc_di.exists() {
        paths.push((0, 0, app_etc_di));
    }
    // Then app/etc/*/di.xml, like the glob `{di.xml,*/di.xml}` in Magento's FileResolver.
    // The glob's `*` does not match hidden folders.
    if let Ok(entries) = std::fs::read_dir(magento_root.join("app/etc")) {
        for entry in entries.flatten() {
            let hidden = entry.file_name().to_string_lossy().starts_with('.');
            let di_xml = entry.path().join("di.xml");
            if !hidden && di_xml.exists() {
                paths.push((0, 1, di_xml));
            }
        }
    }

    // Priority 1, all modules. vendor/magento/*
    collect_vendor(
        &magento_root.join("vendor/magento"),
        1,
        area,
        module_order,
        &mut paths,
        false,
    );

    // Other vendor/vendor/* (non-magento)
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
            collect_vendor(&vendor.path(), 1, area, module_order, &mut paths, true);
        }
    }

    // Modules that Composer registers from a folder the search above skips.
    let found: FxHashSet<std::path::PathBuf> = paths.iter().map(|(_, _, p)| p.clone()).collect();
    for module_root in composer_registered_module_roots(magento_root) {
        let module_idx = cached_module_name(&module_root)
            .and_then(|n| module_order.get(&n).copied())
            .unwrap_or(usize::MAX);
        let mut module_paths = Vec::new();
        push_module_di_paths(&module_root, 1, module_idx, area, &mut module_paths);
        paths.extend(
            module_paths
                .into_iter()
                .filter(|(_, _, path)| !found.contains(path)),
        );
    }

    // app/code/*/*/etc/di.xml
    if let Ok(vendors) = std::fs::read_dir(magento_root.join("app/code")) {
        for vendor in vendors.flatten() {
            if let Ok(modules) = std::fs::read_dir(vendor.path()) {
                for module in modules.flatten() {
                    let module_idx = cached_module_name(&module.path())
                        .and_then(|n| module_order.get(&n).copied())
                        .unwrap_or(usize::MAX);
                    push_module_di_paths(&module.path(), 1, module_idx, area, &mut paths);
                }
            }
        }
    }

    // Sort by (priority, module_idx, path) for deterministic Magento-order merging
    paths.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    paths.into_iter().map(|(_, _, p)| p).collect()
}

/// The folders that the `registration.php` in `folder` registers. `ComponentRegistrar::register()`
/// takes any path, for example `__DIR__ . '/src'`. Only that form is read here, a computed path
/// counts as the folder itself. Without a `registration.php` the answer is the folder itself.
pub fn registered_component_roots(folder: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Ok(registration) = std::fs::read_to_string(folder.join("registration.php")) {
        for (at, _) in registration.match_indices("__DIR__") {
            let rest = registration[at + "__DIR__".len()..].trim_start();
            let Some(rest) = rest.strip_prefix('.') else {
                // A bare `__DIR__` registers the folder itself, next to any `__DIR__ . '/...'` path.
                if !roots.iter().any(|root| root == folder) {
                    roots.push(folder.to_path_buf());
                }
                continue;
            };
            let rest = rest.trim_start();
            let Some(quote) = rest.chars().next().filter(|c| *c == '\'' || *c == '"') else {
                continue;
            };
            let Some((relative, _)) = rest[1..].split_once(quote) else {
                continue;
            };
            let root = folder.join(relative.trim_start_matches('/'));
            if root.is_dir() {
                roots.push(root);
            }
        }
    }
    if roots.is_empty() {
        roots.push(folder.to_path_buf());
    }
    roots
}

/// The folders of the `registration.php` files in `vendor/composer/autoload_files.php`.
/// Composer loads these on every request, which is how Magento learns about these modules.
pub fn composer_registered_module_roots(magento_root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let vendor = magento_root.join("vendor");
    let Ok(list) = std::fs::read_to_string(vendor.join("composer/autoload_files.php")) else {
        return Vec::new();
    };
    list.lines()
        .filter_map(|line| {
            let (_, path) = line.split_once("$vendorDir . '/")?;
            let (path, _) = path.split_once('\'')?;
            path.strip_suffix("/registration.php")
        })
        .map(|package_path| vendor.join(package_path))
        .filter(|module_root| module_root.is_dir())
        .collect()
}

fn normalize(s: &str) -> String {
    s.trim().trim_start_matches('\\').to_string()
}

/// Read the module name from `etc/module.xml` using a simple string search.
///
/// Returns `None` if the file doesn't exist or has no `name="..."` attribute.
fn module_name_from_module_xml(module_root: &std::path::Path) -> Option<String> {
    let content = registered_component_roots(module_root)
        .iter()
        .find_map(|root| std::fs::read_to_string(root.join("etc/module.xml")).ok())?;
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
    static CACHE: OnceLock<Mutex<FxHashMap<std::path::PathBuf, Option<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(FxHashMap::default()));
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
    use rustc_hash::FxHashMap;
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
                    sort_order: Some(30),
                    disabled: Some(false),
                    disabled_after_type: false,
                },
                Plugin {
                    name: "b".into(),
                    type_name: "TB".into(),
                    sort_order: Some(10),
                    disabled: Some(false),
                    disabled_after_type: false,
                },
                Plugin {
                    name: "a".into(),
                    type_name: "TA".into(),
                    sort_order: Some(10),
                    disabled: Some(true),
                    disabled_after_type: false,
                },
            ],
        );
        let active = config.get_active_plugins("Foo");
        assert_eq!(active.len(), 2); // 'a' is disabled
        assert_eq!(active[0].name, "b"); // lower sort_order first
        assert_eq!(active[1].name, "c");
    }

    #[test]
    fn test_get_active_plugins_equal_sort_order_keeps_merge_order() {
        let plugin = |name: &str| Plugin {
            name: name.into(),
            type_name: "T".into(),
            sort_order: Some(10),
            disabled: None,
            disabled_after_type: false,
        };
        let mut config = DiConfig::default();
        config
            .plugins
            .insert("Foo".into(), vec![plugin("z"), plugin("a")]);
        let active = config.get_active_plugins("Foo");
        assert_eq!(active[0].name, "z");
        assert_eq!(active[1].name, "a");
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

        let empty_order = FxHashMap::default();
        let global = super::find_di_xml_files(root, &empty_order);
        assert!(global.contains(&module_root.join("etc/di.xml")));

        let frontend = super::find_di_xml_files_for_area(root, "frontend", &empty_order);
        assert!(frontend.contains(&module_root.join("etc/frontend/di.xml")));
    }

    #[test]
    fn test_find_di_xml_files_includes_module_registered_with_bare_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let module_root = root.join("vendor/acme/pkg/src");
        fs::create_dir_all(module_root.join("etc")).unwrap();
        fs::create_dir_all(root.join("vendor/acme/pkg/dist")).unwrap();
        fs::write(
            module_root.join("registration.php"),
            "<?php\nComponentRegistrar::register(ComponentRegistrar::MODULE, 'Acme_Pkg', __DIR__);\n\
             ComponentRegistrar::register(ComponentRegistrar::LIBRARY, 'Acme_Pkg', __DIR__ . '/../dist');\n",
        )
        .unwrap();
        fs::write(
            module_root.join("etc/di.xml"),
            r#"<?xml version="1.0"?><config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"></config>"#,
        )
        .unwrap();

        let roots = super::registered_component_roots(&module_root);
        assert!(roots.contains(&module_root));
        assert!(roots.contains(&module_root.join("../dist")));

        let global = super::find_di_xml_files(root, &FxHashMap::default());
        assert!(global.contains(&module_root.join("etc/di.xml")));
    }

    #[test]
    fn test_find_di_xml_files_respects_module_order_tiebreak() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::create_dir_all(root.join("app/etc")).unwrap();
        fs::write(
            root.join("app/etc/di.xml"),
            r#"<?xml version="1.0"?><config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"></config>"#,
        )
        .unwrap();

        let module_a = root.join("vendor/magento/module-a");
        let module_b = root.join("vendor/magento/module-b");
        for module in [&module_a, &module_b] {
            fs::create_dir_all(module.join("etc")).unwrap();
            fs::write(
                module.join("etc/di.xml"),
                r#"<?xml version="1.0"?><config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"></config>"#,
            )
            .unwrap();
        }
        fs::write(
            module_a.join("etc/module.xml"),
            r#"<?xml version="1.0"?><config><module name="Magento_A"/></config>"#,
        )
        .unwrap();
        fs::write(
            module_b.join("etc/module.xml"),
            r#"<?xml version="1.0"?><config><module name="Magento_B"/></config>"#,
        )
        .unwrap();

        let mut module_order = FxHashMap::default();
        module_order.insert("Magento_B".to_string(), 0usize);
        module_order.insert("Magento_A".to_string(), 1usize);

        let paths = super::find_di_xml_files(root, &module_order);
        let vendor_paths: Vec<_> = paths
            .iter()
            .filter(|p| p.starts_with(root.join("vendor/magento")))
            .collect();

        assert_eq!(vendor_paths.len(), 2);
        assert_eq!(vendor_paths[0], &module_b.join("etc/di.xml"));
        assert_eq!(vendor_paths[1], &module_a.join("etc/di.xml"));
    }

    #[test]
    fn test_find_di_xml_files_uses_config_php_order_wherever_the_module_lives() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let modules = [
            ("app/code/Acme/Local", "Acme_Local"),
            ("vendor/acme/module-other", "Acme_Other"),
            ("vendor/magento/module-core", "Magento_Core"),
        ];
        let mut module_order = FxHashMap::default();
        for (idx, (dir, name)) in modules.iter().enumerate() {
            fs::create_dir_all(root.join(dir).join("etc")).unwrap();
            fs::write(root.join(dir).join("etc/di.xml"), "<config/>").unwrap();
            fs::write(
                root.join(dir).join("etc/module.xml"),
                format!(r#"<config><module name="{name}"/></config>"#),
            )
            .unwrap();
            module_order.insert(name.to_string(), idx);
        }
        fs::create_dir_all(root.join("app/etc")).unwrap();
        fs::write(root.join("app/etc/di.xml"), "<config/>").unwrap();

        let paths = super::find_di_xml_files(root, &module_order);

        assert_eq!(
            paths,
            vec![
                root.join("app/etc/di.xml"),
                root.join("app/code/Acme/Local/etc/di.xml"),
                root.join("vendor/acme/module-other/etc/di.xml"),
                root.join("vendor/magento/module-core/etc/di.xml"),
            ]
        );
    }

    #[test]
    fn test_find_di_xml_files_reads_app_etc_subfolders_after_app_etc_di_xml() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let empty = r#"<?xml version="1.0"?><config></config>"#;
        for dir in ["app/etc", "app/etc/aaa", "app/etc/zzz", "app/etc/.hidden"] {
            fs::create_dir_all(root.join(dir)).unwrap();
            fs::write(root.join(dir).join("di.xml"), empty).unwrap();
        }
        // Two levels deep is not matched by Magento's `*/di.xml`.
        fs::create_dir_all(root.join("app/etc/aaa/deep")).unwrap();
        fs::write(root.join("app/etc/aaa/deep/di.xml"), empty).unwrap();
        // A folder without a di.xml is ignored.
        fs::create_dir_all(root.join("app/etc/nothing")).unwrap();

        let paths = super::find_di_xml_files(root, &FxHashMap::default());

        // Same order as the glob `{di.xml,*/di.xml}` in FileResolver.
        assert_eq!(
            paths,
            vec![
                root.join("app/etc/di.xml"),
                root.join("app/etc/aaa/di.xml"),
                root.join("app/etc/zzz/di.xml"),
            ]
        );
    }
}
