//! TKT-013: Interceptor detection.
//!
//! A class needs an interceptor when ANY of the following hold:
//!   1. It has at least one active (non-disabled) plugin registered in di.xml, OR
//!   2. It is a non-abstract, non-final concrete class that inherits (directly or
//!      transitively) from a class that needs an interceptor.
//!
//! Phase 2 (inheritance propagation) ensures that when a parent class is intercepted,
//! all concrete subclasses are also intercepted so the plugin system fires correctly
//! when those subclasses are instantiated via the DI container.

use std::collections::{HashMap, HashSet};

use php_extractor::ClassInfo;

use crate::graph::{InterceptorSpec, PluginRef};
use di_xml_reader::DiConfig;

/// Build the list of classes that need interceptors.
pub fn detect_interceptors(
    class_map: &HashMap<String, ClassInfo>,
    di_config: &DiConfig,
) -> Vec<InterceptorSpec> {
    // -----------------------------------------------------------------------
    // Phase 1: Classes with direct plugin registrations in di.xml
    // -----------------------------------------------------------------------
    let mut specs: Vec<InterceptorSpec> = Vec::new();
    let mut directly_intercepted: HashSet<String> = HashSet::new();

    for (owner_name, plugins) in &di_config.plugins {
        let active: Vec<&di_xml_reader::Plugin> =
            plugins.iter().filter(|p| !p.disabled).collect();
        if active.is_empty() {
            continue;
        }

        // Resolve the concrete type to check is_final
        let concrete = di_config.get_instance_type(owner_name);
        let info = class_map.get(&concrete).or_else(|| class_map.get(owner_name));

        // Always record in directly_intercepted so concrete subclasses can inherit
        // via Phase 2. However, only generate an interceptor spec for classes that
        // are "concrete" (non-abstract, non-final, non-interface, non-trait).
        // This matches the PHP compiler's `isConcrete()` check.
        directly_intercepted.insert(owner_name.clone());

        // Skip final classes entirely — they can't be subclassed.
        if let Some(info) = info {
            if info.is_final {
                continue;
            }
        }

        // Skip non-concrete: abstract classes, interfaces, traits don't get spec files.
        // Also skip classes that don't exist in class_map (not in scanned PHP files).
        let is_concrete = match info {
            None => false, // class not found on disk → skip
            Some(info) => {
                use php_extractor::types::ClassKind;
                !info.is_abstract && !matches!(info.kind, ClassKind::Interface | ClassKind::Trait)
            }
        };
        if !is_concrete {
            continue;
        }

        let mut plugin_refs: Vec<PluginRef> = active
            .iter()
            .map(|p| PluginRef {
                name: p.name.clone(),
                type_name: p.type_name.clone(),
                sort_order: p.sort_order,
            })
            .collect();
        plugin_refs.sort_by_key(|p| p.sort_order);

        let public_methods = info.map(|i| i.public_methods.clone()).unwrap_or_default();

        specs.push(InterceptorSpec {
            fqcn: owner_name.clone(),
            plugins: plugin_refs,
            public_methods,
        });
    }

    // -----------------------------------------------------------------------
    // Phase 2: Propagate through inheritance
    //
    // For every concrete (non-abstract, non-final) class in class_map that is
    // NOT already intercepted, walk its `extends` chain. If any ancestor is in
    // the intercepted set, this class also needs an interceptor.
    // -----------------------------------------------------------------------
    let intercepted_set: HashSet<&str> =
        directly_intercepted.iter().map(|s| s.as_str()).collect();

    // Build a cache to avoid repeated ancestor walks.
    // `ancestor_intercepted` memoizes: fqcn → bool
    let mut ancestor_cache: HashMap<&str, bool> = HashMap::new();

    for (fqcn, info) in class_map {
        // Already directly intercepted — skip.
        if directly_intercepted.contains(fqcn.as_str()) {
            continue;
        }
        // Final classes can never be intercepted.
        if info.is_final {
            continue;
        }
        // Abstract classes, interfaces, and traits are not instantiated directly; skip.
        if info.is_abstract {
            continue;
        }
        {
            use php_extractor::types::ClassKind;
            if matches!(info.kind, ClassKind::Interface | ClassKind::Trait) {
                continue;
            }
        }
        // Check inheritance chain.
        if has_intercepted_ancestor(fqcn, class_map, &intercepted_set, &mut ancestor_cache) {
            specs.push(InterceptorSpec {
                fqcn: fqcn.clone(),
                plugins: vec![],   // resolved at runtime by plugin framework
                public_methods: info.public_methods.clone(),
            });
        }
    }

    specs.sort_by(|a, b| a.fqcn.cmp(&b.fqcn));
    specs
}

/// Walk the `extends` chain AND all `implements` interfaces of `fqcn`, returning
/// `true` if any ancestor / interface is in `intercepted_set`.
/// Mirrors Magento's `Relations::getParents()` which includes both extends and implements.
/// Uses `cache` to memoize results.
fn has_intercepted_ancestor<'a>(
    fqcn: &'a str,
    class_map: &'a HashMap<String, ClassInfo>,
    intercepted_set: &HashSet<&str>,
    cache: &mut HashMap<&'a str, bool>,
) -> bool {
    if let Some(&cached) = cache.get(fqcn) {
        return cached;
    }
    // Guard against cycles (insert false first; overwrite on true).
    cache.insert(fqcn, false);

    let result = match class_map.get(fqcn) {
        None => false,
        Some(info) => {
            // Collect all parents: extends + implements.
            let mut parents: Vec<&str> = Vec::new();
            if let Some(ext) = &info.extends {
                parents.push(ext.as_str());
            }
            for iface in &info.implements {
                parents.push(iface.as_str());
            }

            let mut found = false;
            for parent in parents {
                if intercepted_set.contains(parent) {
                    found = true;
                    break;
                }
                if has_intercepted_ancestor(parent, class_map, intercepted_set, cache) {
                    found = true;
                    break;
                }
            }
            found
        }
    };
    // Overwrite the guard entry.
    cache.insert(fqcn, result);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use di_xml_reader::{DiConfig, Plugin};
    use php_extractor::types::{ClassInfo, ClassKind};
    use std::path::PathBuf;

    fn make_class(fqcn: &str, is_final: bool) -> ClassInfo {
        let parts: Vec<&str> = fqcn.rsplitn(2, '\\').collect();
        let (name, ns) = if parts.len() == 2 {
            (parts[0].to_string(), parts[1].to_string())
        } else {
            (fqcn.to_string(), String::new())
        };
        ClassInfo {
            path: PathBuf::from("dummy.php"),
            namespace: ns,
            name: name.clone(),
            fqcn: fqcn.to_string(),
            kind: ClassKind::Class,
            extends: None,
            implements: vec![],
            constructor: None,
            is_abstract: false,
            is_final,
            public_methods: vec![],
        }
    }

    fn make_plugin(name: &str, type_name: &str, sort_order: i32, disabled: bool) -> Plugin {
        Plugin { name: name.to_string(), type_name: type_name.to_string(), sort_order, disabled }
    }

    #[test]
    fn test_detects_class_with_plugin() {
        let mut class_map = HashMap::new();
        class_map.insert("Foo\\Bar".to_string(), make_class("Foo\\Bar", false));
        let mut di_config = DiConfig::default();
        di_config.plugins.insert(
            "Foo\\Bar".to_string(),
            vec![make_plugin("my_plugin", "Foo\\Plugin", 10, false)],
        );
        let specs = detect_interceptors(&class_map, &di_config);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].fqcn, "Foo\\Bar");
        assert_eq!(specs[0].plugins.len(), 1);
    }

    #[test]
    fn test_skips_final_class() {
        let mut class_map = HashMap::new();
        class_map.insert("Foo\\Final".to_string(), make_class("Foo\\Final", true));
        let mut di_config = DiConfig::default();
        di_config
            .plugins
            .insert("Foo\\Final".to_string(), vec![make_plugin("p", "Foo\\P", 0, false)]);
        let specs = detect_interceptors(&class_map, &di_config);
        assert!(specs.is_empty());
    }

    #[test]
    fn test_skips_disabled_plugins() {
        let mut class_map = HashMap::new();
        class_map.insert("Foo\\Bar".to_string(), make_class("Foo\\Bar", false));
        let mut di_config = DiConfig::default();
        di_config
            .plugins
            .insert("Foo\\Bar".to_string(), vec![make_plugin("p", "Foo\\P", 0, true)]);
        let specs = detect_interceptors(&class_map, &di_config);
        assert!(specs.is_empty());
    }

    #[test]
    fn test_child_class_inherits_interceptor() {
        // Parent has a plugin, child extends parent → child also needs interceptor.
        let mut parent = make_class("Foo\\Parent", false);
        parent.extends = None;
        let mut child = make_class("Foo\\Child", false);
        child.extends = Some("Foo\\Parent".to_string());
        let mut class_map = HashMap::new();
        class_map.insert("Foo\\Parent".to_string(), parent);
        class_map.insert("Foo\\Child".to_string(), child);

        let mut di_config = DiConfig::default();
        di_config.plugins.insert(
            "Foo\\Parent".to_string(),
            vec![make_plugin("p", "Foo\\P", 10, false)],
        );
        let specs = detect_interceptors(&class_map, &di_config);
        let fqcns: Vec<&str> = specs.iter().map(|s| s.fqcn.as_str()).collect();
        assert!(fqcns.contains(&"Foo\\Parent"));
        assert!(fqcns.contains(&"Foo\\Child"));
    }

    #[test]
    fn test_final_child_not_inherited() {
        // Parent has a plugin, child is final → child must not get an interceptor.
        let mut parent = make_class("Foo\\Parent", false);
        parent.extends = None;
        let mut child = make_class("Foo\\Child", true); // is_final = true
        child.extends = Some("Foo\\Parent".to_string());
        let mut class_map = HashMap::new();
        class_map.insert("Foo\\Parent".to_string(), parent);
        class_map.insert("Foo\\Child".to_string(), child);

        let mut di_config = DiConfig::default();
        di_config.plugins.insert(
            "Foo\\Parent".to_string(),
            vec![make_plugin("p", "Foo\\P", 10, false)],
        );
        let specs = detect_interceptors(&class_map, &di_config);
        let fqcns: Vec<&str> = specs.iter().map(|s| s.fqcn.as_str()).collect();
        assert!(fqcns.contains(&"Foo\\Parent"));
        assert!(!fqcns.contains(&"Foo\\Child"));
    }

    #[test]
    fn test_plugin_sort_order() {
        let mut class_map = HashMap::new();
        class_map.insert("Foo\\Bar".to_string(), make_class("Foo\\Bar", false));
        let mut di_config = DiConfig::default();
        di_config.plugins.insert(
            "Foo\\Bar".to_string(),
            vec![make_plugin("b", "B", 20, false), make_plugin("a", "A", 10, false)],
        );
        let specs = detect_interceptors(&class_map, &di_config);
        assert_eq!(specs[0].plugins[0].name, "a");
        assert_eq!(specs[0].plugins[1].name, "b");
    }
}
