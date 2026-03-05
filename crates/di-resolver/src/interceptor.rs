//! TKT-013: Interceptor detection.
//!
//! A class needs an interceptor when ALL of the following hold:
//!   1. It has at least one active (non-disabled) plugin registered in di.xml
//!   2. It is not `final`
//!
//! Note: Magento DOES generate interceptors for abstract classes that have plugins.

use std::collections::HashMap;

use php_extractor::ClassInfo;

use crate::graph::{InterceptorSpec, PluginRef};
use di_xml_reader::DiConfig;

/// Build the list of classes that need interceptors.
pub fn detect_interceptors(
    class_map: &HashMap<String, ClassInfo>,
    di_config: &DiConfig,
) -> Vec<InterceptorSpec> {
    let mut specs = Vec::new();

    for (owner_name, plugins) in &di_config.plugins {
        let active: Vec<&di_xml_reader::Plugin> =
            plugins.iter().filter(|p| !p.disabled).collect();
        if active.is_empty() {
            continue;
        }

        // Resolve the concrete type to check is_final
        let concrete = di_config.get_instance_type(owner_name);
        let info = class_map.get(&concrete).or_else(|| class_map.get(owner_name));

        // Skip final classes
        if let Some(info) = info {
            if info.is_final {
                continue;
            }
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

    specs.sort_by(|a, b| a.fqcn.cmp(&b.fqcn));
    specs
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
