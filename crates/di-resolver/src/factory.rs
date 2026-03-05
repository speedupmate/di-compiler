//! TKT-014: Factory + Proxy detection.
//!
//! Factory: generated when a constructor param type ends with `Factory` and that
//!   class does not already exist on disk (not in class_map).
//!
//! Proxy: generated when:
//!   1. A constructor param type ends with `\Proxy` (and not in class_map), OR
//!   2. A di.xml `<argument xsi:type="object">` value ends with `\Proxy` (and not in class_map)
//!
//! This module handles factory detection; proxy detection is in proxy.rs.

use std::collections::HashMap;

use php_extractor::ClassInfo;

use crate::graph::FactorySpec;
use di_xml_reader::DiConfig;

/// Detect factory classes to generate.
///
/// Scans constructor params of all extracted classes for type hints ending with `Factory`.
/// If the factory class does not already exist in class_map, emit a FactorySpec.
pub fn detect_factories(
    class_map: &HashMap<String, ClassInfo>,
    di_config: &DiConfig,
) -> Vec<FactorySpec> {
    let mut specs: Vec<FactorySpec> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for info in class_map.values() {
        let Some(ctor) = &info.constructor else { continue };
        for param in &ctor.params {
            let Some(type_hint) = &param.type_hint else { continue };
            // Must end with "Factory" but not be a built-in
            if !type_hint.ends_with("Factory") {
                continue;
            }
            // Resolve via preferences
            let target = di_config.get_preference(type_hint);
            let factory_fqcn = type_hint.clone();

            // Skip if already exists
            if class_map.contains_key(&factory_fqcn) {
                continue;
            }
            // The target class is the name minus "Factory" suffix
            let target_fqcn = if target != factory_fqcn {
                target
            } else {
                // Strip "Factory" suffix to get the target
                factory_fqcn[..factory_fqcn.len() - 7].to_string()
            };

            if seen.insert(factory_fqcn.clone()) {
                specs.push(FactorySpec { target_fqcn, factory_fqcn });
            }
        }
    }

    specs.sort_by(|a, b| a.factory_fqcn.cmp(&b.factory_fqcn));
    specs
}

#[cfg(test)]
mod tests {
    use super::*;
    use php_extractor::types::{ClassInfo, ClassKind, Constructor, ConstructorParam};
    use std::path::PathBuf;

    fn make_class_with_ctor(fqcn: &str, param_type: &str) -> ClassInfo {
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
            constructor: Some(Constructor {
                params: vec![ConstructorParam {
                    name: "dep".to_string(),
                    type_hint: Some(param_type.to_string()),
                    is_optional: false,
                    is_primitive: false,
                    is_variadic: false,
                    is_promoted: false,
                }],
            }),
            is_abstract: false,
            is_final: false,
            public_methods: vec![],
        }
    }

    #[test]
    fn test_detects_factory() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "Foo\\Bar".to_string(),
            make_class_with_ctor("Foo\\Bar", "Foo\\Baz\\WidgetFactory"),
        );
        let di_config = DiConfig::default();
        let specs = detect_factories(&class_map, &di_config);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].factory_fqcn, "Foo\\Baz\\WidgetFactory");
        assert_eq!(specs[0].target_fqcn, "Foo\\Baz\\Widget");
    }

    #[test]
    fn test_skips_existing_factory() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "Foo\\Bar".to_string(),
            make_class_with_ctor("Foo\\Bar", "Foo\\Baz\\WidgetFactory"),
        );
        // Factory already exists
        class_map.insert(
            "Foo\\Baz\\WidgetFactory".to_string(),
            make_class_with_ctor("Foo\\Baz\\WidgetFactory", ""),
        );
        let di_config = DiConfig::default();
        let specs = detect_factories(&class_map, &di_config);
        assert!(specs.is_empty());
    }
}
