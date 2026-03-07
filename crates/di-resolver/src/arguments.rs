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

use std::collections::{HashMap, HashSet};

use php_extractor::ClassInfo;

use crate::graph::{
    ResolvedArg, ResolvedArgValue, ResolvedArrayItem, ResolvedArrayValue, ResolvedScalar,
};
use di_xml_reader::{Argument, DiConfig};

/// Resolve constructor arguments for all classes.
///
/// Returns a map of FQCN → Vec<ResolvedArg>.
pub fn resolve_all_arguments(
    class_map: &HashMap<String, ClassInfo>,
    di_config: &DiConfig,
    const_map: &HashMap<String, String>,
) -> HashMap<String, Vec<ResolvedArg>> {
    let mut result = HashMap::new();

    for (fqcn, info) in class_map {
        let resolved = resolve_for_class(fqcn, info, class_map, di_config, const_map);
        if !resolved.is_empty() {
            result.insert(fqcn.clone(), resolved);
        }
    }

    result
}

/// Resolve constructor arguments for an explicit set of type names.
///
/// Unlike `resolve_all_arguments`, this supports names that are not present in
/// `class_map` (for example virtual types and generated artifacts) by falling
/// back to their resolved instance type when possible.
pub fn resolve_all_arguments_for_named_types(
    type_names: &[String],
    class_map: &HashMap<String, ClassInfo>,
    di_config: &DiConfig,
    const_map: &HashMap<String, String>,
) -> HashMap<String, Vec<ResolvedArg>> {
    let mut result = HashMap::new();
    for type_name in type_names {
        let resolved = resolve_for_type_name(type_name, class_map, di_config, const_map);
        if !resolved.is_empty() {
            result.insert(type_name.clone(), resolved);
        }
    }
    result
}

/// Resolve constructor args for a single class.
pub fn resolve_for_class(
    fqcn: &str,
    info: &ClassInfo,
    class_map: &HashMap<String, ClassInfo>,
    di_config: &DiConfig,
    const_map: &HashMap<String, String>,
) -> Vec<ResolvedArg> {
    let di_args = merged_di_arguments_for_type_name(fqcn, class_map, di_config);
    let di_arg_map: HashMap<&str, &Argument> =
        di_args.iter().map(|a| (argument_name(a), *a)).collect();
    let Some(ctor) = &info.constructor else {
        return vec![];
    };

    let mut resolved = Vec::new();

    for param in &ctor.params {
        // Check if di.xml overrides this param
        if let Some(di_arg) = di_arg_map.get(param.name.as_str()) {
            let type_hint = param
                .type_hint
                .as_deref()
                .and_then(first_non_null_class_type_hint_arm);
            resolved.push(ResolvedArg {
                name: param.name.clone(),
                resolved: if type_hint.is_some() {
                    resolve_configured_instance_argument(di_arg, di_config, const_map)
                } else {
                    resolve_configured_non_object_argument(di_arg, param.default_value.as_deref(), di_config, const_map)
                },
            });
            continue;
        }

        // Magento precedence:
        // 1) non-required param => default value (non-object argument)
        // 2) required typed param => instance argument
        // 3) fallback => null
        let value = if param.is_variadic {
            ResolvedArgValue::Null
        } else if param.is_optional {
            resolve_non_object_default(param.default_value.as_deref())
        } else if let Some(type_hint) = param
                .type_hint
                .as_deref()
                .and_then(first_non_null_class_type_hint_arm)
            {
                let concrete = di_config.get_preference(&type_hint);
                if di_config.is_shared(&concrete) {
                    ResolvedArgValue::SharedInstance(concrete)
                } else {
                    ResolvedArgValue::NonSharedInstance(concrete)
                }
        } else {
            ResolvedArgValue::Null
        };

        resolved.push(ResolvedArg {
            name: param.name.clone(),
            resolved: value,
        });
    }

    resolved
}

fn resolve_for_type_name(
    type_name: &str,
    class_map: &HashMap<String, ClassInfo>,
    di_config: &DiConfig,
    const_map: &HashMap<String, String>,
) -> Vec<ResolvedArg> {
    let normalized = normalize(type_name);
    let is_virtual = di_config.virtual_types.contains_key(&normalized);
    if let Some(info) = class_info_with_inherited_constructor(&normalized, class_map) {
        if info.constructor.is_none() && is_virtual {
            let di_args = merged_di_arguments_for_type_name(&normalized, class_map, di_config);
            if !di_args.is_empty() {
                return di_args
                    .iter()
                    .map(|arg| ResolvedArg {
                        name: argument_name(arg).to_string(),
                        resolved: resolve_di_argument(arg, di_config, const_map),
                    })
                    .collect();
            }
        }
        return resolve_for_class(&normalized, &info, class_map, di_config, const_map);
    }

    let instance_type = normalize(&di_config.get_instance_type(&normalized));
    if instance_type != normalized {
        if let Some(mut info) = class_info_with_inherited_constructor(&instance_type, class_map) {
            // Keep constructor shape from resolved concrete type, but resolve
            // DI overrides against the requested logical type name.
            info.fqcn = normalized.clone();
            if info.constructor.is_none() && is_virtual {
                let di_args = merged_di_arguments_for_type_name(&normalized, class_map, di_config);
                if !di_args.is_empty() {
                    return di_args
                        .iter()
                        .map(|arg| ResolvedArg {
                            name: argument_name(arg).to_string(),
                            resolved: resolve_di_argument(arg, di_config, const_map),
                        })
                        .collect();
                }
            }
            return resolve_for_class(&normalized, &info, class_map, di_config, const_map);
        }
    }

    if !is_virtual {
        return Vec::new();
    }

    let di_args = merged_di_arguments_for_type_name(&normalized, class_map, di_config);
    if di_args.is_empty() {
        return Vec::new();
    }
    di_args
        .iter()
        .map(|arg| ResolvedArg {
            name: argument_name(arg).to_string(),
            resolved: resolve_di_argument(arg, di_config, const_map),
        })
        .collect()
}

fn class_info_with_inherited_constructor(
    fqcn: &str,
    class_map: &HashMap<String, ClassInfo>,
) -> Option<ClassInfo> {
    let mut info = class_map.get(fqcn)?.clone();
    if info.constructor.is_some() {
        return Some(info);
    }
    let mut seen = HashSet::new();
    let mut cursor = info.extends.clone();
    while let Some(parent) = cursor {
        if !seen.insert(parent.clone()) {
            break;
        }
        let Some(parent_info) = class_map.get(&parent) else {
            break;
        };
        if parent_info.constructor.is_some() {
            info.constructor = parent_info.constructor.clone();
            break;
        }
        cursor = parent_info.extends.clone();
    }
    Some(info)
}

fn merged_di_arguments_for_type_name<'a>(
    type_name: &str,
    class_map: &HashMap<String, ClassInfo>,
    di_config: &'a DiConfig,
) -> Vec<&'a Argument> {
    // Build the full type chain: PHP class hierarchy (root→leaf) followed by
    // the virtual-type chain for each entry.  More-specific types override
    // less-specific ones, so we process from least specific to most specific.
    //
    // PHP's ArgumentsResolver walks the class hierarchy the same way: a
    // <type name="Parent"> di.xml override is inherited by all subclasses
    // unless the subclass declares its own override for that argument name.

    // 1. Collect the PHP extends chain for the concrete class that `type_name`
    //    resolves to.  The chain is ordered root-ancestor-first.
    let concrete = {
        let vchain = virtual_type_chain(type_name, di_config);
        normalize(vchain.last().unwrap_or(&type_name.to_string()))
    };

    let mut class_hierarchy: Vec<String> = Vec::new();
    {
        let mut cursor = concrete.clone();
        let mut seen: HashSet<String> = HashSet::new();
        while !cursor.is_empty() && seen.insert(cursor.clone()) {
            class_hierarchy.push(cursor.clone());
            match class_map.get(&cursor).and_then(|i| i.extends.as_ref()) {
                Some(parent) => cursor = normalize(parent),
                None => break,
            }
        }
        class_hierarchy.reverse(); // root-ancestor first (lowest priority)
    }

    // 2. If `type_name` is a virtual type or differs from `concrete`, append it
    //    at the end so its own virtual-type chain has the highest priority.
    if normalize(type_name) != concrete {
        class_hierarchy.push(normalize(type_name));
    }

    // 3. For each entry in the hierarchy, walk its virtual-type chain and merge
    //    arguments (later entries / more-specific names override earlier ones).
    let mut merged: Vec<&'a Argument> = Vec::new();
    let mut by_name: HashMap<String, usize> = HashMap::new();

    for ancestor in &class_hierarchy {
        let chain = virtual_type_chain(ancestor, di_config);
        for current in chain.iter().rev() {
            for arg in di_config.get_arguments(current) {
                let name = argument_name(arg).to_string();
                if let Some(idx) = by_name.get(&name).copied() {
                    merged[idx] = arg;
                } else {
                    by_name.insert(name, merged.len());
                    merged.push(arg);
                }
            }
        }
    }

    merged
}

fn virtual_type_chain(type_name: &str, di_config: &DiConfig) -> Vec<String> {
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut current = normalize(type_name);
    loop {
        if !seen.insert(current.clone()) {
            break;
        }
        chain.push(current.clone());
        let Some(vt) = di_config.virtual_types.get(&current) else {
            break;
        };
        let next = normalize(&vt.type_name);
        if next.is_empty() || next == current {
            break;
        }
        current = next;
    }
    chain
}

fn normalize(s: &str) -> String {
    s.trim().trim_start_matches('\\').to_string()
}

fn first_non_null_class_type_hint_arm(type_hint: &str) -> Option<String> {
    const NON_CLASS_TYPES: &[&str] = &[
        "null", "false", "true", "int", "float", "string", "bool", "array", "callable", "iterable",
        "object", "mixed", "void", "never", "self", "parent", "static",
    ];

    type_hint
        .split('|')
        .map(str::trim)
        .map(|arm| arm.trim_start_matches('?').trim_start_matches('\\'))
        .find(|arm| !arm.is_empty() && !NON_CLASS_TYPES.contains(arm))
        .map(ToOwned::to_owned)
}

fn resolve_di_argument(arg: &Argument, di_config: &DiConfig, const_map: &HashMap<String, String>) -> ResolvedArgValue {
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
        Argument::String { value, .. } => {
            ResolvedArgValue::Scalar(ResolvedScalar::String(value.clone()))
        }
        Argument::Boolean { value, .. } => ResolvedArgValue::Scalar(ResolvedScalar::Bool(*value)),
        Argument::Number { value, .. } => {
            ResolvedArgValue::Scalar(ResolvedScalar::Number(value.clone()))
        }
        Argument::Null { .. } => ResolvedArgValue::Null,
        Argument::Array { items, .. } => {
            if is_configured_argument_array(items) {
                let resolved_items: Vec<ResolvedArg> = items
                    .iter()
                    .map(|item| ResolvedArg {
                        name: argument_name(item).to_string(),
                        resolved: resolve_di_argument(item, di_config, const_map),
                    })
                    .collect();
                ResolvedArgValue::Array(resolved_items)
            } else {
                let values: Vec<ResolvedArrayItem> = items
                    .iter()
                    .map(|item| ResolvedArrayItem {
                        name: argument_name(item).to_string(),
                        value: resolve_plain_array_value(item),
                    })
                    .collect();
                ResolvedArgValue::PlainArray(values)
            }
        }
        Argument::Init { value, .. } => ResolvedArgValue::GlobalArgRef {
            arg_name: resolve_php_constant_expr(value, const_map),
            default: None,
        },
        Argument::Const { value, .. } => {
            ResolvedArgValue::Scalar(ResolvedScalar::String(value.clone()))
        }
    }
}

fn resolve_configured_instance_argument(arg: &Argument, di_config: &DiConfig, const_map: &HashMap<String, String>) -> ResolvedArgValue {
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
        _ => resolve_di_argument(arg, di_config, const_map),
    }
}

fn resolve_configured_non_object_argument(
    arg: &Argument,
    constructor_default: Option<&str>,
    di_config: &DiConfig,
    const_map: &HashMap<String, String>,
) -> ResolvedArgValue {
    match arg {
        Argument::Init { value, .. } => ResolvedArgValue::GlobalArgRef {
            // _a_: resolve PHP class constant expressions (e.g. ClassName::CONST_NAME → "MAGE_MODE")
            arg_name: resolve_php_constant_expr(value, const_map),
            // _d_: constructor default is raw PHP source (e.g. `'default'`) — unquote it
            default: constructor_default.and_then(|d| {
                let trimmed = d.trim();
                if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
                    None
                } else if let Some(s) = parse_quoted_php_string(trimmed) {
                    Some(s)
                } else {
                    Some(trimmed.to_string())
                }
            }),
        },
        _ => resolve_di_argument(arg, di_config, const_map),
    }
}

/// Resolve a PHP constant expression of the form `ClassName::CONST_NAME` to its actual
/// value using the pre-built const_map.  If the expression is not in `ClassName::CONST_NAME`
/// form (or has no match), the raw string is returned unchanged.
fn resolve_php_constant_expr(expr: &str, const_map: &HashMap<String, String>) -> String {
    let normalized = expr.trim().trim_start_matches('\\');
    if let Some(resolved) = const_map.get(normalized) {
        return resolved.clone();
    }
    normalized.to_string()
}

fn resolve_non_object_default(default_value: Option<&str>) -> ResolvedArgValue {
    let Some(raw) = default_value else {
        return ResolvedArgValue::Null;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
        return ResolvedArgValue::Null;
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return ResolvedArgValue::Scalar(ResolvedScalar::Bool(true));
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return ResolvedArgValue::Scalar(ResolvedScalar::Bool(false));
    }
    if trimmed == "[]" {
        return ResolvedArgValue::PlainArray(Vec::new());
    }
    if let Some(s) = parse_quoted_php_string(trimmed) {
        return ResolvedArgValue::Scalar(ResolvedScalar::String(s));
    }
    if is_numeric_literal(trimmed) {
        return ResolvedArgValue::Scalar(ResolvedScalar::Number(trimmed.to_string()));
    }
    ResolvedArgValue::Scalar(ResolvedScalar::String(trimmed.to_string()))
}

fn parse_quoted_php_string(input: &str) -> Option<String> {
    if input.len() < 2 {
        return None;
    }
    if let Some(inner) = input.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        let value = inner.replace("\\\\", "\\").replace("\\'", "'");
        return Some(value);
    }
    if let Some(inner) = input.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        let value = inner.replace("\\\\", "\\").replace("\\\"", "\"");
        return Some(value);
    }
    None
}

fn is_numeric_literal(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('-').or_else(|| s.strip_prefix('+')) {
        if rest.is_empty() {
            return false;
        }
        return rest.parse::<f64>().is_ok();
    }
    s.parse::<f64>().is_ok()
}

fn is_configured_argument_array(items: &[Argument]) -> bool {
    items.iter().any(|item| match item {
        Argument::Object { .. } | Argument::Init { .. } => true,
        Argument::Array { items, .. } => is_configured_argument_array(items),
        _ => false,
    })
}

fn resolve_plain_array_value(arg: &Argument) -> ResolvedArrayValue {
    match arg {
        Argument::String { value, .. } => ResolvedArrayValue::Scalar(ResolvedScalar::String(value.clone())),
        Argument::Boolean { value, .. } => ResolvedArrayValue::Scalar(ResolvedScalar::Bool(*value)),
        Argument::Number { value, .. } => ResolvedArrayValue::Scalar(ResolvedScalar::Number(value.clone())),
        Argument::Null { .. } => ResolvedArrayValue::Null,
        Argument::Const { value, .. } => ResolvedArrayValue::Scalar(ResolvedScalar::String(value.clone())),
        Argument::Array { items, .. } => ResolvedArrayValue::Array(
            items
                .iter()
                .map(|item| ResolvedArrayItem {
                    name: argument_name(item).to_string(),
                    value: resolve_plain_array_value(item),
                })
                .collect(),
        ),
        // Fallbacks: treat configured-only forms as scalar strings when they
        // unexpectedly appear in a non-configured array branch.
        Argument::Object { value, .. } => ResolvedArrayValue::Scalar(ResolvedScalar::String(value.clone())),
        Argument::Init { value, .. } => ResolvedArrayValue::Scalar(ResolvedScalar::String(value.clone())),
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
                        default_value: None,
                        is_primitive: t
                            .map(|s| matches!(s, "string" | "int" | "bool" | "float"))
                            .unwrap_or(false),
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

    fn make_class_with_constructor(fqcn: &str, params: Vec<ConstructorParam>) -> ClassInfo {
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
            constructor: Some(Constructor { params }),
            is_abstract: false,
            is_final: false,
            public_methods: vec![],
        }
    }

    fn preprocessor_pool_fixture() -> (HashMap<String, ClassInfo>, DiConfig) {
        let mut class_map = HashMap::new();
        class_map.insert(
            "Magento\\Framework\\View\\Asset\\PreProcessor\\Pool".to_string(),
            make_class_with_constructor(
                "Magento\\Framework\\View\\Asset\\PreProcessor\\Pool",
                vec![
                    ConstructorParam {
                        name: "objectManager".to_string(),
                        type_hint: Some("Magento\\Framework\\ObjectManagerInterface".to_string()),
                        is_optional: false,
                        default_value: None,
                        is_primitive: false,
                        is_variadic: false,
                        is_promoted: false,
                    },
                    ConstructorParam {
                        name: "sorter".to_string(),
                        type_hint: Some(
                            "Magento\\Framework\\View\\Asset\\PreProcessor\\Helper\\SortInterface"
                                .to_string(),
                        ),
                        is_optional: false,
                        default_value: None,
                        is_primitive: false,
                        is_variadic: false,
                        is_promoted: false,
                    },
                    ConstructorParam {
                        name: "defaultPreprocessor".to_string(),
                        type_hint: None,
                        is_optional: false,
                        default_value: None,
                        is_primitive: false,
                        is_variadic: false,
                        is_promoted: false,
                    },
                    ConstructorParam {
                        name: "preprocessors".to_string(),
                        type_hint: Some("array".to_string()),
                        is_optional: true,
                        default_value: Some("[]".to_string()),
                        is_primitive: true,
                        is_variadic: false,
                        is_promoted: false,
                    },
                ],
            ),
        );

        let mut di_config = DiConfig::default();
        di_config.virtual_types.insert(
            "AssetPreProcessorPool".to_string(),
            di_xml_reader::VirtualType {
                name: "AssetPreProcessorPool".to_string(),
                type_name: "Magento\\Framework\\View\\Asset\\PreProcessor\\Pool".to_string(),
            },
        );
        di_config.type_configs.insert(
            "Magento\\Framework\\View\\Asset\\PreProcessor\\Pool".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::String {
                    name: "defaultPreprocessor".to_string(),
                    value: "Magento\\Framework\\View\\Asset\\PreProcessor\\Passthrough"
                        .to_string(),
                }],
            },
        );
        di_config.type_configs.insert(
            "AssetPreProcessorPool".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Array {
                    name: "preprocessors".to_string(),
                    items: vec![Argument::String {
                        name: "less".to_string(),
                        value: "custom".to_string(),
                    }],
                }],
            },
        );

        (class_map, di_config)
    }

    #[test]
    fn test_resolves_object_param_as_shared_instance() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "App\\Service".to_string(),
            make_class("App\\Service", vec![("logger", Some("App\\Logger"))]),
        );
        let di_config = DiConfig::default();
        let map = resolve_all_arguments(&class_map, &di_config, &HashMap::new());
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
        let map = resolve_all_arguments(&class_map, &di_config, &HashMap::new());
        let args = &map["App\\Service"];
        assert!(matches!(
            &args[0].resolved,
            ResolvedArgValue::Scalar(ResolvedScalar::String(v)) if v == "Hello World"
        ));
    }

    #[test]
    fn test_null_for_no_type_hint() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "App\\Service".to_string(),
            make_class("App\\Service", vec![("options", None)]),
        );
        let di_config = DiConfig::default();
        let map = resolve_all_arguments(&class_map, &di_config, &HashMap::new());
        let args = &map["App\\Service"];
        assert!(matches!(&args[0].resolved, ResolvedArgValue::Null));
    }

    #[test]
    fn test_nullable_object_type_hint_normalized() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "App\\Service".to_string(),
            make_class("App\\Service", vec![("dep", Some("?App\\Dep"))]),
        );

        let di_config = DiConfig::default();
        let map = resolve_all_arguments(&class_map, &di_config, &HashMap::new());
        let args = &map["App\\Service"];
        assert!(matches!(
            &args[0].resolved,
            ResolvedArgValue::SharedInstance(fqcn) if fqcn == "App\\Dep"
        ));
    }

    #[test]
    fn test_union_with_primitive_uses_class_arm() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "App\\Service".to_string(),
            make_class("App\\Service", vec![("dep", Some("string|App\\Dep"))]),
        );

        let di_config = DiConfig::default();
        let map = resolve_all_arguments(&class_map, &di_config, &HashMap::new());
        let args = &map["App\\Service"];
        assert!(matches!(
            &args[0].resolved,
            ResolvedArgValue::SharedInstance(fqcn) if fqcn == "App\\Dep"
        ));
    }

    #[test]
    fn test_union_without_class_resolves_null() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "App\\Service".to_string(),
            make_class("App\\Service", vec![("dep", Some("string|null"))]),
        );

        let di_config = DiConfig::default();
        let map = resolve_all_arguments(&class_map, &di_config, &HashMap::new());
        let args = &map["App\\Service"];
        assert!(matches!(&args[0].resolved, ResolvedArgValue::Null));
    }

    #[test]
    fn test_named_virtual_type_inherits_base_constructor_and_virtual_overrides() {
        let (class_map, di_config) = preprocessor_pool_fixture();
        let map = resolve_all_arguments_for_named_types(
            &["AssetPreProcessorPool".to_string()],
            &class_map,
            &di_config,
            &HashMap::new(),
        );
        let args = &map["AssetPreProcessorPool"];

        let by_name: HashMap<_, _> = args.iter().map(|a| (a.name.as_str(), &a.resolved)).collect();

        assert!(matches!(
            by_name.get("objectManager").copied(),
            Some(ResolvedArgValue::SharedInstance(v))
            if v == "Magento\\Framework\\ObjectManagerInterface"
        ));
        assert!(matches!(
            by_name.get("sorter").copied(),
            Some(ResolvedArgValue::SharedInstance(v))
            if v == "Magento\\Framework\\View\\Asset\\PreProcessor\\Helper\\SortInterface"
        ));
        assert!(matches!(
            by_name.get("defaultPreprocessor").copied(),
            Some(ResolvedArgValue::Scalar(ResolvedScalar::String(v)))
            if v == "Magento\\Framework\\View\\Asset\\PreProcessor\\Passthrough"
        ));
        assert!(matches!(
            by_name.get("preprocessors").copied(),
            Some(ResolvedArgValue::PlainArray(items))
            if items.len() == 1
                && items[0].name == "less"
                && matches!(&items[0].value, ResolvedArrayValue::Scalar(ResolvedScalar::String(v)) if v == "custom")
        ));
    }

    #[test]
    fn test_named_virtual_type_resolution_normalizes_leading_backslash() {
        let (class_map, di_config) = preprocessor_pool_fixture();
        let map = resolve_all_arguments_for_named_types(
            &["\\AssetPreProcessorPool".to_string()],
            &class_map,
            &di_config,
            &HashMap::new(),
        );
        let args = map.get("\\AssetPreProcessorPool").expect("resolved args");
        assert!(args.iter().any(|a| a.name == "objectManager"));
        assert!(args.iter().any(|a| a.name == "sorter"));
    }

    #[test]
    fn test_init_parameter_without_constructor_default_sets_global_arg_ref_without_default() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "App\\Service".to_string(),
            make_class("App\\Service", vec![("mode", None)]),
        );

        let mut di_config = DiConfig::default();
        di_config.type_configs.insert(
            "App\\Service".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Init {
                    name: "mode".to_string(),
                    value: "App\\State::MODE".to_string(),
                }],
            },
        );

        let mut const_map = HashMap::new();
        const_map.insert("App\\State::MODE".to_string(), "MAGE_MODE".to_string());

        let map = resolve_all_arguments(&class_map, &di_config, &const_map);
        let args = &map["App\\Service"];
        assert!(matches!(
            &args[0].resolved,
            ResolvedArgValue::GlobalArgRef { arg_name, default }
            if arg_name == "MAGE_MODE" && default.is_none()
        ));
    }
}
