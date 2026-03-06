//! TKT-014: Proxy detection.
//!
//! Proxy triggers mirror Magento XmlScanner paths:
//!   1. di.xml `<preference type="...\Proxy" />`
//!   2. di.xml `<argument|item xsi:type="object">...\Proxy</...>`
//!   3. di.xml `<virtualType type="...\Proxy" />`
//!
//! A proxy is generated only when:
//!   - proxy class does not already exist
//!   - target class/interface (proxy minus `\Proxy`) exists

use std::collections::{HashMap, HashSet};

use php_extractor::{types::ClassKind, ClassInfo};

use crate::graph::ProxySpec;
use di_xml_reader::{Argument, DiConfig};

/// Detect proxy classes to generate.
pub fn detect_proxies(
    class_map: &HashMap<String, ClassInfo>,
    di_config: &DiConfig,
) -> Vec<ProxySpec> {
    detect_proxies_from_configs(class_map, std::slice::from_ref(di_config))
}

/// Detect proxy classes using raw per-file DI configs for XmlScanner-style
/// candidate coverage (preserves candidates that are overridden during merge).
pub fn detect_proxies_from_configs(
    class_map: &HashMap<String, ClassInfo>,
    scanner_di_configs: &[DiConfig],
) -> Vec<ProxySpec> {
    let extra_existing_types = HashSet::new();
    detect_proxies_from_configs_with_existing(class_map, scanner_di_configs, &extra_existing_types)
}

/// Detect proxies with an explicit set of additional loadable class/interface
/// names that are not present in `class_map` (e.g. Composer-only libraries).
pub fn detect_proxies_from_configs_with_existing(
    class_map: &HashMap<String, ClassInfo>,
    scanner_di_configs: &[DiConfig],
    extra_existing_types: &HashSet<String>,
) -> Vec<ProxySpec> {
    let mut specs: Vec<ProxySpec> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let virtual_type_names: HashSet<&str> = scanner_di_configs
        .iter()
        .flat_map(|cfg| cfg.virtual_types.keys().map(String::as_str))
        .collect();

    let mut emit = |proxy_fqcn: String| {
        if !proxy_fqcn.ends_with("\\Proxy") {
            return;
        }
        // Baseline parity: skip proxy FQCNs that are virtual type names.
        if virtual_type_names.contains(proxy_fqcn.as_str()) {
            return;
        }
        if class_map.contains_key(&proxy_fqcn) {
            return;
        }
        let target_fqcn = proxy_fqcn[..proxy_fqcn.len() - 6].to_string();
        if !class_or_interface_exists(class_map, extra_existing_types, &target_fqcn) {
            return;
        }
        if seen.insert(proxy_fqcn.clone()) {
            specs.push(ProxySpec {
                target_fqcn,
                proxy_fqcn,
            });
        }
    };

    // Path 1: preference @type
    for cfg in scanner_di_configs {
        for proxy_fqcn in cfg.preferences.values() {
            emit(proxy_fqcn.clone());
        }
    }

    // Path 2: di.xml argument/item object values
    for cfg in scanner_di_configs {
        for tc in cfg.type_configs.values() {
            let mut candidates = Vec::new();
            collect_proxy_candidates_from_args(&tc.arguments, &mut candidates);
            for candidate in candidates {
                emit(candidate);
            }
        }
    }

    // Path 3: virtualType @type
    for cfg in scanner_di_configs {
        for vt in cfg.virtual_types.values() {
            emit(vt.type_name.clone());
        }
    }

    specs.sort_by(|a, b| a.proxy_fqcn.cmp(&b.proxy_fqcn));
    specs
}

fn collect_proxy_candidates_from_args(args: &[Argument], out: &mut Vec<String>) {
    for arg in args {
        match arg {
            Argument::Object { value, .. } => out.push(value.clone()),
            Argument::Array { items, .. } => {
                collect_proxy_candidates_from_args(items, out);
            }
            _ => {}
        }
    }
}

fn class_or_interface_exists(
    class_map: &HashMap<String, ClassInfo>,
    extra_existing_types: &HashSet<String>,
    fqcn: &str,
) -> bool {
    if extra_existing_types.contains(fqcn) {
        return true;
    }
    matches!(
        class_map.get(fqcn).map(|info| &info.kind),
        Some(ClassKind::Class | ClassKind::AbstractClass | ClassKind::Interface)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use di_xml_reader::{Argument, DiConfig, TypeConfig, VirtualType};
    use php_extractor::types::{ClassInfo, ClassKind, Constructor};
    use std::path::PathBuf;

    fn make_class(fqcn: &str, kind: ClassKind) -> ClassInfo {
        let parts: Vec<&str> = fqcn.rsplitn(2, '\\').collect();
        let (name, ns) = if parts.len() == 2 {
            (parts[0].to_string(), parts[1].to_string())
        } else {
            (fqcn.to_string(), String::new())
        };
        ClassInfo {
            path: PathBuf::from("dummy.php"),
            namespace: ns,
            name,
            fqcn: fqcn.to_string(),
            kind,
            extends: None,
            implements: vec![],
            constructor: Some(Constructor { params: vec![] }),
            is_abstract: false,
            is_final: false,
            public_methods: vec![],
        }
    }

    #[test]
    fn test_proxy_from_preference_type() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "Foo\\Baz".to_string(),
            make_class("Foo\\Baz", ClassKind::Class),
        );
        let mut di_config = DiConfig::default();
        di_config
            .preferences
            .insert("Foo\\Iface".to_string(), "Foo\\Baz\\Proxy".to_string());
        let specs = detect_proxies(&class_map, &di_config);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].proxy_fqcn, "Foo\\Baz\\Proxy");
        assert_eq!(specs[0].target_fqcn, "Foo\\Baz");
    }

    #[test]
    fn test_proxy_from_di_xml_argument() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "Foo\\Heavy".to_string(),
            make_class("Foo\\Heavy", ClassKind::Class),
        );
        let mut di_config = DiConfig::default();
        di_config.type_configs.insert(
            "Foo\\Service".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Object {
                    name: "dep".to_string(),
                    value: "Foo\\Heavy\\Proxy".to_string(),
                    shared: None,
                }],
            },
        );
        let specs = detect_proxies(&class_map, &di_config);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].proxy_fqcn, "Foo\\Heavy\\Proxy");
    }

    #[test]
    fn test_proxy_skips_existing() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "Foo\\Baz".to_string(),
            make_class("Foo\\Baz", ClassKind::Class),
        );
        // The proxy already exists
        class_map.insert(
            "Foo\\Baz\\Proxy".to_string(),
            make_class("Foo\\Baz\\Proxy", ClassKind::Class),
        );
        let mut di_config = DiConfig::default();
        di_config
            .preferences
            .insert("Foo\\Iface".to_string(), "Foo\\Baz\\Proxy".to_string());
        let specs = detect_proxies(&class_map, &di_config);
        assert!(specs.is_empty());
    }

    #[test]
    fn test_proxy_requires_existing_target_class_or_interface() {
        let class_map = HashMap::new();
        let di_config = DiConfig::default();
        let specs = detect_proxies(&class_map, &di_config);
        assert!(specs.is_empty());
    }

    #[test]
    fn test_proxy_from_virtual_type_parent() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "Foo\\Baz\\Api".to_string(),
            make_class("Foo\\Baz\\Api", ClassKind::Interface),
        );
        let mut di_config = DiConfig::default();
        di_config.virtual_types.insert(
            "Foo\\Some\\Virtual".to_string(),
            VirtualType {
                name: "Foo\\Some\\Virtual".to_string(),
                type_name: "Foo\\Baz\\Api\\Proxy".to_string(),
            },
        );
        let specs = detect_proxies(&class_map, &di_config);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].proxy_fqcn, "Foo\\Baz\\Api\\Proxy");
        assert_eq!(specs[0].target_fqcn, "Foo\\Baz\\Api");
    }

    #[test]
    fn test_proxy_from_nested_di_xml_item_argument() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "Foo\\Heavy".to_string(),
            make_class("Foo\\Heavy", ClassKind::Class),
        );
        let mut di_config = DiConfig::default();
        di_config.type_configs.insert(
            "Foo\\Service".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Array {
                    name: "deps".to_string(),
                    items: vec![Argument::Object {
                        name: "dep".to_string(),
                        value: "Foo\\Heavy\\Proxy".to_string(),
                        shared: None,
                    }],
                }],
            },
        );
        let specs = detect_proxies(&class_map, &di_config);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].proxy_fqcn, "Foo\\Heavy\\Proxy");
    }

    #[test]
    fn test_proxy_scanner_preserves_preference_candidates_across_file_overrides() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "Foo\\One\\Target".to_string(),
            make_class("Foo\\One\\Target", ClassKind::Class),
        );
        class_map.insert(
            "Foo\\Two\\Target".to_string(),
            make_class("Foo\\Two\\Target", ClassKind::Class),
        );

        let mut cfg1 = DiConfig::default();
        cfg1.preferences.insert(
            "Foo\\Iface".to_string(),
            "Foo\\One\\Target\\Proxy".to_string(),
        );
        let mut cfg2 = DiConfig::default();
        cfg2.preferences.insert(
            "Foo\\Iface".to_string(),
            "Foo\\Two\\Target\\Proxy".to_string(),
        );

        let specs = detect_proxies_from_configs(&class_map, &[cfg1, cfg2]);
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].proxy_fqcn, "Foo\\One\\Target\\Proxy");
        assert_eq!(specs[1].proxy_fqcn, "Foo\\Two\\Target\\Proxy");
    }

    #[test]
    fn test_proxy_virtual_type_name_is_skipped() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "Foo\\Target".to_string(),
            make_class("Foo\\Target", ClassKind::Class),
        );
        let mut di_config = DiConfig::default();
        di_config.virtual_types.insert(
            "Foo\\Target\\Proxy".to_string(),
            VirtualType {
                name: "Foo\\Target\\Proxy".to_string(),
                type_name: "Foo\\Other\\Type".to_string(),
            },
        );
        di_config.type_configs.insert(
            "Foo\\Service".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Object {
                    name: "dep".to_string(),
                    value: "Foo\\Target\\Proxy".to_string(),
                    shared: None,
                }],
            },
        );
        let specs = detect_proxies(&class_map, &di_config);
        assert!(specs.is_empty());
    }

    #[test]
    fn test_proxy_target_can_be_resolved_from_extra_existing_types() {
        let class_map = HashMap::new();
        let mut di_config = DiConfig::default();
        di_config.type_configs.insert(
            "Foo\\Service".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Object {
                    name: "dep".to_string(),
                    value: "Psr\\Log\\LoggerInterface\\Proxy".to_string(),
                    shared: None,
                }],
            },
        );

        let mut extra = HashSet::new();
        extra.insert("Psr\\Log\\LoggerInterface".to_string());

        let specs = detect_proxies_from_configs_with_existing(&class_map, &[di_config], &extra);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].proxy_fqcn, "Psr\\Log\\LoggerInterface\\Proxy");
        assert_eq!(specs[0].target_fqcn, "Psr\\Log\\LoggerInterface");
    }
}
