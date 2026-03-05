//! TKT-014: Proxy detection.
//!
//! Two proxy trigger paths (per ground truth analysis):
//!   1. Constructor type hints ending with `\Proxy` (12 occurrences)
//!   2. di.xml `<argument xsi:type="object">...\Proxy</argument>` (330 occurrences)
//!
//! A proxy is only generated if the proxy class does not already exist in class_map.

use std::collections::{HashMap, HashSet};

use php_extractor::ClassInfo;

use crate::graph::ProxySpec;
use di_xml_reader::{Argument, DiConfig};

/// Detect proxy classes to generate.
pub fn detect_proxies(
    class_map: &HashMap<String, ClassInfo>,
    di_config: &DiConfig,
) -> Vec<ProxySpec> {
    let mut specs: Vec<ProxySpec> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let mut emit = |proxy_fqcn: String, class_map: &HashMap<String, ClassInfo>| {
        if !proxy_fqcn.ends_with("\\Proxy") && !proxy_fqcn.ends_with("/Proxy") {
            return;
        }
        if class_map.contains_key(&proxy_fqcn) {
            return;
        }
        if seen.insert(proxy_fqcn.clone()) {
            // Target is proxy_fqcn with "\\Proxy" stripped
            let target_fqcn = proxy_fqcn[..proxy_fqcn.len() - 6].to_string();
            specs.push(ProxySpec {
                target_fqcn,
                proxy_fqcn,
            });
        }
    };

    // Path 1: constructor type hints
    for info in class_map.values() {
        let Some(ctor) = &info.constructor else {
            continue;
        };
        for param in &ctor.params {
            let Some(type_hint) = &param.type_hint else {
                continue;
            };
            let Some(type_hint) = first_non_null_type_hint_arm(type_hint) else {
                continue;
            };
            if type_hint.ends_with("\\Proxy") {
                emit(type_hint, class_map);
            }
        }
    }

    // Path 2: di.xml argument objects
    for tc in di_config.type_configs.values() {
        collect_proxy_from_args(&tc.arguments, class_map, &mut seen, &mut specs);
    }

    specs.sort_by(|a, b| a.proxy_fqcn.cmp(&b.proxy_fqcn));
    specs
}

/// Constructor type hints may include nullable/union notation.
/// Proxy detection should inspect a normalized class-like arm.
fn first_non_null_type_hint_arm(type_hint: &str) -> Option<String> {
    type_hint
        .split('|')
        .map(str::trim)
        .map(|arm| arm.trim_start_matches('?').trim_start_matches('\\'))
        .find(|arm| !arm.is_empty() && !matches!(*arm, "null" | "false" | "true"))
        .map(ToOwned::to_owned)
}

fn collect_proxy_from_args(
    args: &[Argument],
    class_map: &HashMap<String, ClassInfo>,
    seen: &mut HashSet<String>,
    specs: &mut Vec<ProxySpec>,
) {
    for arg in args {
        match arg {
            Argument::Object { value, .. } => {
                if value.ends_with("\\Proxy") && !class_map.contains_key(value) {
                    if seen.insert(value.clone()) {
                        let target_fqcn = value[..value.len() - 6].to_string();
                        specs.push(ProxySpec {
                            target_fqcn,
                            proxy_fqcn: value.clone(),
                        });
                    }
                }
            }
            Argument::Array { items, .. } => {
                collect_proxy_from_args(items, class_map, seen, specs);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use di_xml_reader::{Argument, DiConfig, TypeConfig};
    use php_extractor::types::{ClassInfo, ClassKind, Constructor, ConstructorParam};
    use std::path::PathBuf;

    fn make_class_with_proxy_param(fqcn: &str, proxy_type: &str) -> ClassInfo {
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
            kind: ClassKind::Class,
            extends: None,
            implements: vec![],
            constructor: Some(Constructor {
                params: vec![ConstructorParam {
                    name: "dep".to_string(),
                    type_hint: Some(proxy_type.to_string()),
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
    fn test_proxy_from_constructor() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "Foo\\Bar".to_string(),
            make_class_with_proxy_param("Foo\\Bar", "Foo\\Baz\\Proxy"),
        );
        let di_config = DiConfig::default();
        let specs = detect_proxies(&class_map, &di_config);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].proxy_fqcn, "Foo\\Baz\\Proxy");
        assert_eq!(specs[0].target_fqcn, "Foo\\Baz");
    }

    #[test]
    fn test_proxy_from_di_xml_argument() {
        let class_map = HashMap::new();
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
            "Foo\\Bar".to_string(),
            make_class_with_proxy_param("Foo\\Bar", "Foo\\Baz\\Proxy"),
        );
        // The proxy already exists
        class_map.insert(
            "Foo\\Baz\\Proxy".to_string(),
            make_class_with_proxy_param("Foo\\Baz\\Proxy", ""),
        );
        let di_config = DiConfig::default();
        let specs = detect_proxies(&class_map, &di_config);
        assert!(specs.is_empty());
    }

    #[test]
    fn test_proxy_from_nullable_constructor_type_hint() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "Foo\\Bar".to_string(),
            make_class_with_proxy_param("Foo\\Bar", "?Foo\\Baz\\Proxy"),
        );
        let di_config = DiConfig::default();
        let specs = detect_proxies(&class_map, &di_config);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].proxy_fqcn, "Foo\\Baz\\Proxy");
    }

    #[test]
    fn test_proxy_from_union_constructor_type_hint() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "Foo\\Bar".to_string(),
            make_class_with_proxy_param("Foo\\Bar", "null|Foo\\Baz\\Proxy"),
        );
        let di_config = DiConfig::default();
        let specs = detect_proxies(&class_map, &di_config);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].proxy_fqcn, "Foo\\Baz\\Proxy");
    }
}
