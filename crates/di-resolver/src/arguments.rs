//! TKT-015: Arguments resolver.
//!
//! Resolves a class's constructor parameters to `ResolvedArg` values using
//! di.xml configuration and the class map. Implements the same logic as
//! Magento's `ArgumentsResolver.php`.
//!
//! Argument notation (used later by metadata serializer):
//!   Shared instance   → `['_i_'   => 'FQN']`
//!   Non-shared        → `['_ins_' => 'FQN']`
//!   Scalar            → `['_v_'   => value]`
//!   Null              → `['_vn_'  => true]`
//!   Array             → `['_vac_' => [...]]`
//!   Global arg ref    → `['_a_'   => 'name', '_d_' => default]`

use std::collections::HashMap;

use php_extractor::ClassInfo;

use crate::graph::{ResolvedArg, ResolvedArgValue};
use di_xml_reader::{Argument, DiConfig};

/// Resolve constructor arguments for all classes.
///
/// Returns a map of FQCN → Vec<ResolvedArg>.
pub fn resolve_all_arguments(
    class_map: &HashMap<String, ClassInfo>,
    di_config: &DiConfig,
) -> HashMap<String, Vec<ResolvedArg>> {
    let mut result = HashMap::new();

    for (fqcn, info) in class_map {
        let resolved = resolve_for_class(fqcn, info, di_config);
        if !resolved.is_empty() {
            result.insert(fqcn.clone(), resolved);
        }
    }

    result
}

/// Resolve constructor args for a single class.
pub fn resolve_for_class(
    fqcn: &str,
    info: &ClassInfo,
    di_config: &DiConfig,
) -> Vec<ResolvedArg> {
    let Some(ctor) = &info.constructor else {
        return vec![];
    };

    // Collect di.xml arguments for this type (following preference chain)
    let instance_type = di_config.get_instance_type(fqcn);
    let di_args = di_config.get_arguments(&instance_type);
    let di_arg_map: HashMap<&str, &Argument> =
        di_args.iter().map(|a| (argument_name(a), *a)).collect();

    let mut resolved = Vec::new();

    for param in &ctor.params {
        // Check if di.xml overrides this param
        if let Some(di_arg) = di_arg_map.get(param.name.as_str()) {
            resolved.push(ResolvedArg {
                name: param.name.clone(),
                resolved: resolve_di_argument(di_arg, di_config),
            });
            continue;
        }

        // Fall back to type-hint based resolution
        let value = if param.is_variadic || param.type_hint.is_none() || param.is_primitive {
            ResolvedArgValue::Null
        } else {
            let type_hint = param.type_hint.as_ref().unwrap();
            let concrete = di_config.get_preference(type_hint);
            if di_config.is_shared(&concrete) {
                ResolvedArgValue::SharedInstance(concrete)
            } else {
                ResolvedArgValue::NonSharedInstance(concrete)
            }
        };

        resolved.push(ResolvedArg { name: param.name.clone(), resolved: value });
    }

    resolved
}

fn resolve_di_argument(arg: &Argument, di_config: &DiConfig) -> ResolvedArgValue {
    match arg {
        Argument::Object { value, shared, .. } => {
            let concrete = di_config.get_preference(value);
            let is_shared = shared.unwrap_or_else(|| di_config.is_shared(&concrete));
            if is_shared {
                ResolvedArgValue::SharedInstance(concrete)
            } else {
                ResolvedArgValue::NonSharedInstance(concrete)
            }
        }
        Argument::String { value, .. } => ResolvedArgValue::Scalar(value.clone()),
        Argument::Boolean { value, .. } => {
            ResolvedArgValue::Scalar(if *value { "true".to_string() } else { "false".to_string() })
        }
        Argument::Number { value, .. } => ResolvedArgValue::Scalar(value.clone()),
        Argument::Null { .. } => ResolvedArgValue::Null,
        Argument::Array { items, .. } => {
            let resolved_items: Vec<ResolvedArg> = items
                .iter()
                .map(|item| ResolvedArg {
                    name: argument_name(item).to_string(),
                    resolved: resolve_di_argument(item, di_config),
                })
                .collect();
            ResolvedArgValue::Array(resolved_items)
        }
        Argument::Init { value, .. } => ResolvedArgValue::Scalar(value.clone()),
        Argument::Const { value, .. } => ResolvedArgValue::Scalar(value.clone()),
    }
}

fn argument_name(arg: &Argument) -> &str {
    match arg {
        Argument::Object { name, .. }
        | Argument::String { name, .. }
        | Argument::Boolean { name, .. }
        | Argument::Number { name, .. }
        | Argument::Null { name }
        | Argument::Array { name, .. }
        | Argument::Init { name, .. }
        | Argument::Const { name, .. } => name.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use di_xml_reader::{Argument, DiConfig, TypeConfig};
    use php_extractor::types::{ClassInfo, ClassKind, Constructor, ConstructorParam};
    use std::path::PathBuf;

    fn make_class(fqcn: &str, params: Vec<(&str, Option<&str>)>) -> ClassInfo {
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
                params: params
                    .into_iter()
                    .map(|(n, t)| ConstructorParam {
                        name: n.to_string(),
                        type_hint: t.map(|s| s.to_string()),
                        is_optional: false,
                        is_primitive: t.map(|s| matches!(s, "string" | "int" | "bool" | "float")).unwrap_or(false),
                        is_variadic: false,
                        is_promoted: false,
                    })
                    .collect(),
            }),
            is_abstract: false,
            is_final: false,
            public_methods: vec![],
        }
    }

    #[test]
    fn test_resolves_object_param_as_shared_instance() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "App\\Service".to_string(),
            make_class("App\\Service", vec![("logger", Some("App\\Logger"))]),
        );
        let di_config = DiConfig::default();
        let map = resolve_all_arguments(&class_map, &di_config);
        let args = &map["App\\Service"];
        assert_eq!(args.len(), 1);
        assert!(
            matches!(&args[0].resolved, ResolvedArgValue::SharedInstance(fqcn) if fqcn == "App\\Logger")
        );
    }

    #[test]
    fn test_di_xml_string_override() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "App\\Service".to_string(),
            make_class("App\\Service", vec![("label", Some("string"))]),
        );
        let mut di_config = DiConfig::default();
        di_config.type_configs.insert(
            "App\\Service".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::String {
                    name: "label".to_string(),
                    value: "Hello World".to_string(),
                }],
            },
        );
        let map = resolve_all_arguments(&class_map, &di_config);
        let args = &map["App\\Service"];
        assert!(
            matches!(&args[0].resolved, ResolvedArgValue::Scalar(v) if v == "Hello World")
        );
    }

    #[test]
    fn test_null_for_no_type_hint() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "App\\Service".to_string(),
            make_class("App\\Service", vec![("options", None)]),
        );
        let di_config = DiConfig::default();
        let map = resolve_all_arguments(&class_map, &di_config);
        let args = &map["App\\Service"];
        assert!(matches!(&args[0].resolved, ResolvedArgValue::Null));
    }
}
