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
            match self.preferences.get(&current) {
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
        self.type_configs
            .get(&name)
            .map(|tc| tc.arguments.iter().collect())
            .unwrap_or_default()
    }

    /// Whether the type is shared (singleton). Defaults to `true`.
    pub fn is_shared(&self, fqcn: &str) -> bool {
        let name = normalize(fqcn);
        self.type_configs
            .get(&name)
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
    /// Order: vendor/magento → vendor/* (non-magento) → app/etc → app/code
    pub fn load_from_magento_root(
        magento_root: &std::path::Path,
    ) -> Result<DiConfig, crate::Error> {
        use crate::parser::parse_di_xml_impl;

        let mut configs = Vec::new();
        let di_xml_paths = find_di_xml_files(magento_root);
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

/// Collect di.xml files in Magento merge order.
pub fn find_di_xml_files(magento_root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<(u8, std::path::PathBuf)> = Vec::new();

    // Helper: walk a base dir collecting **/etc/di.xml
    fn collect_di_xml(
        base: &std::path::Path,
        priority: u8,
        out: &mut Vec<(u8, std::path::PathBuf)>,
    ) {
        if !base.is_dir() {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(base) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let di = p.join("etc/di.xml");
                    if di.exists() {
                        out.push((priority, di));
                    }
                    // Also check area-specific: etc/{area}/di.xml
                    let etc = p.join("etc");
                    if etc.is_dir() {
                        if let Ok(area_entries) = std::fs::read_dir(&etc) {
                            for ae in area_entries.flatten() {
                                let ap = ae.path();
                                if ap.is_dir() {
                                    let area_di = ap.join("di.xml");
                                    if area_di.exists() {
                                        out.push((priority, area_di));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Priority 1: vendor/magento/*
    collect_di_xml(&magento_root.join("vendor/magento"), 1, &mut paths);

    // Priority 2: other vendor/*/* (non-magento)
    if let Ok(vendors) = std::fs::read_dir(magento_root.join("vendor")) {
        for vendor in vendors.flatten() {
            let vname = vendor.file_name();
            if vname.to_str().map(|s| s == "magento").unwrap_or(false) {
                continue;
            }
            collect_di_xml(&vendor.path(), 2, &mut paths);
        }
    }

    // Priority 3: app/etc/di.xml
    let app_etc_di = magento_root.join("app/etc/di.xml");
    if app_etc_di.exists() {
        paths.push((3, app_etc_di));
    }

    // Priority 4: app/code/*/*/etc/di.xml
    if let Ok(vendors) = std::fs::read_dir(magento_root.join("app/code")) {
        for vendor in vendors.flatten() {
            if let Ok(modules) = std::fs::read_dir(vendor.path()) {
                for module in modules.flatten() {
                    let di = module.path().join("etc/di.xml");
                    if di.exists() {
                        paths.push((4, di));
                    }
                }
            }
        }
    }

    // Sort by priority, then path for determinism
    paths.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    paths.into_iter().map(|(_, p)| p).collect()
}

fn normalize(s: &str) -> String {
    s.trim().trim_start_matches('\\').to_string()
}

#[cfg(test)]
mod tests {
    
    use crate::model::{DiConfig, Plugin};

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
        config.virtual_types.insert("MyVirtual".into(), crate::model::VirtualType {
            name: "MyVirtual".into(),
            type_name: "ConcreteClass".into(),
        });
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
        config.plugins.insert("Foo".into(), vec![
            Plugin { name: "c".into(), type_name: "TC".into(), sort_order: 30, disabled: false },
            Plugin { name: "b".into(), type_name: "TB".into(), sort_order: 10, disabled: false },
            Plugin { name: "a".into(), type_name: "TA".into(), sort_order: 10, disabled: true },
        ]);
        let active = config.get_active_plugins("Foo");
        assert_eq!(active.len(), 2); // 'a' is disabled
        assert_eq!(active[0].name, "b"); // lower sort_order first
        assert_eq!(active[1].name, "c");
    }
}
