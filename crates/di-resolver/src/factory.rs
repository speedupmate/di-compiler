//! TKT-014: Factory + Proxy detection.
//!
//! Factory: generated when a constructor param type ends with `Factory` and that
//!   class does not already exist on disk (not in class_map).
//!
//! This module handles factory detection; proxy detection is in proxy.rs.

use rustc_hash::{FxHashMap, FxHashSet};

use php_extractor::ClassInfo;

use crate::graph::FactorySpec;
use di_xml_reader::{Argument, DiConfig};

/// Detect factory classes to generate.
///
/// Scans constructor params of all extracted classes for type hints ending with `Factory`.
/// If the factory class does not already exist in class_map, emit a FactorySpec.
pub fn detect_factories(
    class_map: &FxHashMap<String, ClassInfo>,
    di_config: &DiConfig,
) -> Vec<FactorySpec> {
    detect_factories_from_configs(class_map, std::slice::from_ref(di_config), di_config)
}

/// Detect factory classes using raw per-file DI configs for XmlScanner-style candidate
/// coverage.
pub fn detect_factories_from_configs(
    class_map: &FxHashMap<String, ClassInfo>,
    scanner_di_configs: &[DiConfig],
    global_di_config: &DiConfig,
) -> Vec<FactorySpec> {
    let mut specs: Vec<FactorySpec> = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();
    let virtual_type_names: FxHashSet<&str> = scanner_di_configs
        .iter()
        .flat_map(|cfg| cfg.virtual_types.keys().map(String::as_str))
        .collect();
    let global_virtual_type_names: FxHashSet<&str> = global_di_config
        .virtual_types
        .keys()
        .map(String::as_str)
        .collect();

    let mut emit = |candidate: String, from_xml: bool| {
        let Some(mut factory_fqcn) = first_non_null_type_hint_arm(&candidate) else {
            return;
        };
        // `Generator::generateClass()` loads the source of `XFactory\Proxy`, and the autoloader
        // generates that factory on the spot.
        if let Some(source) = factory_fqcn.strip_suffix("\\Proxy") {
            if source.ends_with("Factory") {
                factory_fqcn = source.to_string();
            }
        }
        if !factory_fqcn.ends_with("Factory") {
            return;
        }
        // Magento XmlScanner excludes virtual type names from XML object
        // candidates. For archive/runtime parity, we still allow area-only
        // virtualType names (e.g. etc/adminhtml/di.xml).
        if from_xml
            && virtual_type_names.contains(factory_fqcn.as_str())
            && global_virtual_type_names.contains(factory_fqcn.as_str())
        {
            return;
        }
        if class_map.contains_key(&factory_fqcn) {
            return;
        }

        // `Code\Generator::generateClass()` takes the name without the suffix. A preference
        // on the factory decides who gets the factory, not what the factory creates.
        let target_fqcn = factory_fqcn[..factory_fqcn.len() - 7].to_string();

        // If the factory FQCN has no namespace separator it is root-namespace —
        // almost certainly a use-import resolution bug in our lexer. Suppress it
        // only when a class with the same simple name already exists somewhere in
        // the class map (meaning the type hint resolved to the wrong namespace).
        if !factory_fqcn.contains('\\') && class_map.values().any(|info| info.name == factory_fqcn)
        {
            return;
        }

        if seen.insert(factory_fqcn.clone()) {
            specs.push(FactorySpec {
                target_fqcn,
                factory_fqcn,
            });
        }
    };

    // Path 1: constructor type hints.
    for info in class_map.values() {
        let Some(ctor) = &info.constructor else {
            continue;
        };
        for param in &ctor.params {
            let Some(type_hint) = &param.type_hint else {
                continue;
            };
            emit(type_hint.clone(), false);
        }
    }

    // Path 2: XML argument/item object values.
    for cfg in scanner_di_configs {
        for tc in cfg.type_configs.values() {
            let mut candidates = Vec::new();
            collect_factory_candidates_from_args(&tc.arguments, &mut candidates);
            for candidate in candidates {
                emit(candidate, true);
            }
        }
    }

    specs.sort_by(|a, b| a.factory_fqcn.cmp(&b.factory_fqcn));
    specs
}

/// Factories for class names that compile asks for by name, such as the target of
/// a virtual type. `Generator::generateClass()` only generates one when its source
/// class exists, because the generator has nothing to read otherwise.
pub fn detect_named_factories<'a>(
    names: impl Iterator<Item = &'a str>,
    class_map: &FxHashMap<String, ClassInfo>,
    source_exists: impl Fn(&str) -> bool,
) -> Vec<FactorySpec> {
    names
        .map(|name| name.trim().trim_start_matches('\\'))
        .filter(|name| !class_map.contains_key(*name))
        .filter_map(|name| {
            let target = name.strip_suffix("Factory")?;
            source_exists(target).then(|| FactorySpec {
                target_fqcn: target.to_string(),
                factory_fqcn: name.to_string(),
            })
        })
        .collect()
}

/// Constructor type hints may now preserve nullable/union notation.
/// Factory detection must use a normalized class-like arm.
pub(crate) fn first_non_null_type_hint_arm(type_hint: &str) -> Option<String> {
    type_hint
        .split('|')
        .map(str::trim)
        .map(|arm| arm.trim_start_matches('?').trim_start_matches('\\'))
        .find(|arm| !arm.is_empty() && !matches!(*arm, "null" | "false" | "true"))
        .map(ToOwned::to_owned)
}

fn collect_factory_candidates_from_args(args: &[Argument], out: &mut Vec<String>) {
    for arg in args {
        match arg {
            Argument::Object { value, .. } => out.push(value.clone()),
            Argument::Array { items, .. } => collect_factory_candidates_from_args(items, out),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use di_xml_reader::{Argument, TypeConfig, VirtualType};
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
                    default_value: None,
                    is_primitive: false,
                    is_variadic: false,
                    is_promoted: false,
                }],
            }),
            is_abstract: false,
            is_final: false,
            public_methods: vec![],
            traits: Default::default(),
        }
    }

    #[test]
    fn test_detects_factory() {
        let mut class_map = FxHashMap::default();
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
    fn test_factory_with_a_preference_still_creates_its_own_source_class() {
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Foo\\Bar".to_string(),
            make_class_with_ctor("Foo\\Bar", "Foo\\Baz\\WidgetFactory"),
        );
        let mut di_config = DiConfig::default();
        di_config.preferences.insert(
            "Foo\\Baz\\WidgetFactory".to_string(),
            "Other\\WidgetFactory".to_string(),
        );
        di_config.refresh_lookup_indexes();
        let specs = detect_factories(&class_map, &di_config);
        assert_eq!(specs[0].target_fqcn, "Foo\\Baz\\Widget");
    }

    #[test]
    fn test_skips_existing_factory() {
        let mut class_map = FxHashMap::default();
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

    #[test]
    fn test_nullable_factory_type_hint_normalized() {
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Foo\\Bar".to_string(),
            make_class_with_ctor("Foo\\Bar", "?Foo\\Baz\\WidgetFactory"),
        );
        let di_config = DiConfig::default();
        let specs = detect_factories(&class_map, &di_config);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].factory_fqcn, "Foo\\Baz\\WidgetFactory");
    }

    #[test]
    fn test_union_factory_type_hint_uses_first_non_null_arm() {
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Foo\\Bar".to_string(),
            make_class_with_ctor("Foo\\Bar", "null|Foo\\Baz\\WidgetFactory"),
        );
        let di_config = DiConfig::default();
        let specs = detect_factories(&class_map, &di_config);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].factory_fqcn, "Foo\\Baz\\WidgetFactory");
    }

    #[test]
    fn test_factory_from_xml_argument_object() {
        let class_map = FxHashMap::default();
        let mut di_config = DiConfig::default();
        di_config.type_configs.insert(
            "Foo\\Service".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Object {
                    name: "factory".to_string(),
                    value: "Foo\\Baz\\WidgetFactory".to_string(),
                    shared: None,
                    sort_order: 0,
                }],
            },
        );
        let specs = detect_factories(&class_map, &di_config);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].factory_fqcn, "Foo\\Baz\\WidgetFactory");
    }

    #[test]
    fn test_factory_virtual_type_name_is_skipped() {
        let class_map = FxHashMap::default();
        let mut di_config = DiConfig::default();
        di_config.virtual_types.insert(
            "Foo\\Baz\\WidgetFactory".to_string(),
            VirtualType {
                name: "Foo\\Baz\\WidgetFactory".to_string(),
                type_name: "Foo\\Real\\Type".to_string(),
            },
        );
        di_config.type_configs.insert(
            "Foo\\Service".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Object {
                    name: "factory".to_string(),
                    value: "Foo\\Baz\\WidgetFactory".to_string(),
                    shared: None,
                    sort_order: 0,
                }],
            },
        );
        let specs = detect_factories(&class_map, &di_config);
        assert!(specs.is_empty());
    }

    #[test]
    fn test_factory_scanner_preserves_candidates_across_file_overrides() {
        let class_map = FxHashMap::default();

        let mut cfg1 = DiConfig::default();
        cfg1.type_configs.insert(
            "Foo\\Service".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Object {
                    name: "factory".to_string(),
                    value: "Foo\\One\\WidgetFactory".to_string(),
                    shared: None,
                    sort_order: 0,
                }],
            },
        );

        let mut cfg2 = DiConfig::default();
        cfg2.type_configs.insert(
            "Foo\\Service".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Object {
                    name: "factory".to_string(),
                    value: "Foo\\Two\\WidgetFactory".to_string(),
                    shared: None,
                    sort_order: 0,
                }],
            },
        );

        let specs = detect_factories_from_configs(&class_map, &[cfg1, cfg2], &DiConfig::default());
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].factory_fqcn, "Foo\\One\\WidgetFactory");
        assert_eq!(specs[1].factory_fqcn, "Foo\\Two\\WidgetFactory");
    }

    #[test]
    fn test_factory_virtual_type_name_from_area_only_is_not_skipped() {
        let class_map = FxHashMap::default();

        let mut area_cfg = DiConfig::default();
        area_cfg.virtual_types.insert(
            "Foo\\Baz\\WidgetFactory".to_string(),
            VirtualType {
                name: "Foo\\Baz\\WidgetFactory".to_string(),
                type_name: "Foo\\Real\\TypeFactory".to_string(),
            },
        );
        area_cfg.type_configs.insert(
            "Foo\\Service".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Object {
                    name: "factory".to_string(),
                    value: "Foo\\Baz\\WidgetFactory".to_string(),
                    shared: None,
                    sort_order: 0,
                }],
            },
        );

        let specs = detect_factories_from_configs(
            &class_map,
            std::slice::from_ref(&area_cfg),
            &DiConfig::default(),
        );
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].factory_fqcn, "Foo\\Baz\\WidgetFactory");
    }

    fn exists_in(class_map: &FxHashMap<String, ClassInfo>) -> impl Fn(&str) -> bool + '_ {
        |fqcn| class_map.contains_key(fqcn)
    }

    #[test]
    fn test_named_factory_with_a_source_class_is_detected() {
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Foo\\Baz\\Widget".to_string(),
            make_class_with_ctor("Foo\\Baz\\Widget", ""),
        );
        let specs = detect_named_factories(
            ["\\Foo\\Baz\\WidgetFactory", "Foo\\Baz\\Widget"].into_iter(),
            &class_map,
            exists_in(&class_map),
        );
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].factory_fqcn, "Foo\\Baz\\WidgetFactory");
        assert_eq!(specs[0].target_fqcn, "Foo\\Baz\\Widget");
    }

    #[test]
    fn test_named_factory_without_a_source_class_is_skipped() {
        let class_map = FxHashMap::default();
        let specs = detect_named_factories(
            ["Foo\\Baz\\WidgetFactory"].into_iter(),
            &class_map,
            exists_in(&class_map),
        );
        assert!(specs.is_empty());
    }

    #[test]
    fn test_named_factory_that_exists_in_code_is_skipped() {
        let mut class_map = FxHashMap::default();
        for fqcn in ["Foo\\Baz\\Widget", "Foo\\Baz\\WidgetFactory"] {
            class_map.insert(fqcn.to_string(), make_class_with_ctor(fqcn, ""));
        }
        let specs = detect_named_factories(
            ["Foo\\Baz\\WidgetFactory"].into_iter(),
            &class_map,
            exists_in(&class_map),
        );
        assert!(specs.is_empty());
    }
}
