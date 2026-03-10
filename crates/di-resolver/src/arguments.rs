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

use php_extractor::{types::ClassKind, ClassInfo};

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
/// `base_class_fqcns` is the set of FQCNs from the source PHP scan (not generated artifacts).
/// PHP emits NULL only for source concrete classes with empty resolved args, not for
/// generated classes (interceptors, factories, proxies) or types missing from the scan.
pub fn resolve_all_arguments_for_named_types(
    type_names: &[String],
    class_map: &HashMap<String, ClassInfo>,
    base_class_fqcns: &HashSet<String>,
    di_config: &DiConfig,
    const_map: &HashMap<String, String>,
) -> HashMap<String, Vec<ResolvedArg>> {
    let mut result = HashMap::new();
    for type_name in type_names {
        let name = type_name.trim_start_matches('\\');
        let kind = class_map.get(name).map(|c| c.kind.clone());
        // PHP never emits argument entries for pure interfaces or abstract classes.
        if matches!(
            kind,
            Some(ClassKind::Interface) | Some(ClassKind::AbstractClass)
        ) {
            continue;
        }

        // TKT-052: For types not found by the PHP scanner but present in di.xml <type>
        // entries, PHP still emits NULL (empty args). Replicate that behaviour here.
        // Virtual types are not in class_map but have their own resolution path below.
        if !class_map.contains_key(name) && !di_config.virtual_types.contains_key(name) {
            let looks_like_interface = name.ends_with("Interface");
            let is_generated = name.ends_with("Interceptor")
                || name.ends_with("Factory")
                || name.ends_with("Proxy");
            if !looks_like_interface && !is_generated && di_config.type_configs.contains_key(name) {
                result.insert(type_name.clone(), vec![]);
            }
            continue;
        }

        let resolved = resolve_for_type_name(type_name, class_map, di_config, const_map);
        if resolved.is_empty() {
            // PHP emits NULL for source concrete classes, traits, AND virtual types with
            // no resolved constructor args. It does NOT emit NULL for generated artifacts
            // (interceptors, factories, proxies) or unknown non-VT types.
            let is_virtual = di_config.virtual_types.contains_key(name);
            let is_source_classlike = matches!(kind, Some(ClassKind::Class | ClassKind::Trait))
                && base_class_fqcns.contains(name);
            if !is_source_classlike && !is_virtual {
                continue;
            }
        }
        result.insert(type_name.clone(), resolved);
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
        di_args.iter().map(|a| (argument_name(a), a)).collect();
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
                    resolve_configured_non_object_argument(
                        di_arg,
                        param.default_value.as_deref(),
                        di_config,
                        const_map,
                    )
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
            resolve_non_object_default(param.default_value.as_deref(), const_map)
        } else if let Some(type_hint) = param
            .type_hint
            .as_deref()
            .and_then(first_non_null_class_type_hint_arm)
        {
            let concrete = di_config.get_preference(&type_hint);
            // PHP's ArgumentsResolver.getInstanceArgument() calls isShared() on the
            // TYPE HINT as-is (not the resolved concrete). So TransactionManagerInterface
            // with no shared=false entry → _i_, even if the concrete TransactionManager
            // has shared=false. Only the hint's own shared config matters on this path.
            let shared = di_config.is_shared(&type_hint);
            if shared {
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

    // Interceptor alias collision handling:
    // When `Foo\Bar\Interceptor` is the interception alias for `Foo\Bar`,
    // resolve constructor shape from `Foo\Bar` rather than from any real class
    // named `Foo\Bar\Interceptor` (for example setup code generator classes).
    if let Some(base_type) = normalized.strip_suffix("\\Interceptor") {
        let is_interception_alias = di_config
            .preferences
            .get(base_type)
            .map(|to| normalize(to) == normalized)
            .unwrap_or(false);
        if is_interception_alias {
            if let Some(info) = class_info_with_inherited_constructor(base_type, class_map) {
                return resolve_for_class(base_type, &info, class_map, di_config, const_map);
            }
        }
    }

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

fn merged_di_arguments_for_type_name(
    type_name: &str,
    class_map: &HashMap<String, ClassInfo>,
    di_config: &DiConfig,
) -> Vec<Argument> {
    // Build the full type chain: PHP class hierarchy (root→leaf) followed by
    // the virtual-type chain for each entry.  More-specific types override
    // less-specific ones, so we process from least specific to most specific.
    //
    // PHP's ArgumentsResolver walks the class hierarchy the same way: a
    // <type name="Parent"> di.xml override is inherited by all subclasses
    // unless the subclass declares its own override for that argument name.
    //
    // Additionally, PHP's Config::_collectConfiguration also recurses into
    // implemented interfaces (via ClassReader::getParents which returns
    // [parentClass, ...new_interfaces] or [null, ...interfaces]).  This means
    // arguments registered on an interface (e.g. CommandListInterface) flow
    // into the concrete preference class (e.g. CommandList).  We mirror this
    // by inserting each class's "new" interfaces (not already in parent) just
    // before the class in the hierarchy, giving interfaces lower priority than
    // the class's own arguments.

    // 1. Collect the PHP extends chain for the concrete class that `type_name`
    //    resolves to.  The chain is ordered root-ancestor-first.
    let concrete = {
        let vchain = virtual_type_chain(type_name, di_config);
        normalize(vchain.last().unwrap_or(&type_name.to_string()))
    };

    let mut extends_chain: Vec<String> = Vec::new();
    {
        let mut cursor = concrete.clone();
        let mut seen: HashSet<String> = HashSet::new();
        while !cursor.is_empty() && seen.insert(cursor.clone()) {
            extends_chain.push(cursor.clone());
            match class_map.get(&cursor).and_then(|i| i.extends.as_ref()) {
                Some(parent) => cursor = normalize(parent),
                None => break,
            }
        }
        extends_chain.reverse(); // root-ancestor first (lowest priority)
    }

    // 1b. Expand extends_chain to include interfaces "new" to each class level.
    //     Mirrors PHP ClassReader::getParents: array_diff(class.implements, parent.implements).
    //     Interfaces are inserted before the class so the class's own args win.
    let mut class_hierarchy: Vec<String> = Vec::new();
    {
        let mut parent_implements: HashSet<String> = HashSet::new();
        for class_name in &extends_chain {
            if let Some(info) = class_map.get(class_name) {
                let class_implements: HashSet<String> =
                    info.implements.iter().map(|i| normalize(i)).collect();
                let mut new_interfaces: Vec<String> = class_implements
                    .difference(&parent_implements)
                    .cloned()
                    .collect();
                new_interfaces.sort(); // stable ordering
                class_hierarchy.extend(new_interfaces);
                parent_implements = class_implements;
            }
            class_hierarchy.push(class_name.clone());
        }
    }

    // 2. If `type_name` is a virtual type or differs from `concrete`, append it
    //    at the end so its own virtual-type chain has the highest priority.
    if normalize(type_name) != concrete {
        class_hierarchy.push(normalize(type_name));
    }

    // 3. For each entry in the hierarchy, walk its virtual-type chain and merge
    //    arguments (later entries / more-specific names override earlier ones).
    //    For Array arguments, recursively merge items (matching PHP's array_replace_recursive
    //    semantics) rather than replacing — this is critical for arguments like `commands`
    //    that are contributed by many di.xml files across many modules.
    let mut merged: Vec<Argument> = Vec::new();
    let mut by_name: HashMap<String, usize> = HashMap::new();

    for ancestor in &class_hierarchy {
        let chain = virtual_type_chain(ancestor, di_config);
        for current in chain.iter().rev() {
            for arg in di_config.get_arguments(current) {
                let name = argument_name(arg).to_string();
                if let Some(idx) = by_name.get(&name).copied() {
                    merge_argument_into(&mut merged[idx], arg);
                } else {
                    by_name.insert(name, merged.len());
                    merged.push(arg.clone());
                }
            }
        }
    }

    merged
}

/// Merge `src` into `dst`.
/// For Array arguments both sides are merged recursively by item name (same-name item wins src).
/// All other types: src replaces dst.
fn merge_argument_into(dst: &mut Argument, src: &Argument) {
    match (dst, src) {
        (
            Argument::Array {
                items: dst_items, ..
            },
            Argument::Array {
                items: src_items, ..
            },
        ) => {
            for src_item in src_items {
                let name = argument_name(src_item).to_string();
                if let Some(existing) = dst_items.iter_mut().find(|a| argument_name(a) == name) {
                    merge_argument_into(existing, src_item);
                } else {
                    dst_items.push(src_item.clone());
                }
            }
        }
        (dst, src) => *dst = src.clone(),
    }
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

fn resolve_di_argument(
    arg: &Argument,
    di_config: &DiConfig,
    const_map: &HashMap<String, String>,
) -> ResolvedArgValue {
    match arg {
        Argument::Object { value, shared, .. } => {
            let name = normalize(value);
            // Virtual types are usually kept as their VT alias in arguments.
            // However, if DI config declares an explicit preference for that alias
            // (e.g. adminhtml CsrfRequestValidator -> BackendValidator), PHP resolves
            // to the preferred concrete type.
            let is_virtual_type = di_config.virtual_types.contains_key(&name);
            let has_explicit_preference = di_config.preferences.contains_key(&name)
                || di_config
                    .preference_keys_lc
                    .contains_key(&name.to_ascii_lowercase());
            // For real class / interface references, apply the full preference chain
            // (including interception preferences so intercepted concretes become \Interceptor).
            let concrete = if is_virtual_type && !has_explicit_preference {
                name
            } else {
                di_config.get_preference(value)
            };
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
            // Sort items by sortOrder (stable: items with equal sortOrder keep their
            // di.xml merge order). This mirrors PHP's SortItems::sortItems() which is
            // called after each _collectConfiguration merge step.
            let has_any_sort_order = items.iter().any(|i| argument_sort_order(i) != 0);
            let sorted_items: Vec<&Argument> = if has_any_sort_order {
                let mut indexed: Vec<(usize, &Argument)> = items.iter().enumerate().collect();
                indexed.sort_by(|(ai, a), (bi, b)| {
                    argument_sort_order(a)
                        .cmp(&argument_sort_order(b))
                        .then(ai.cmp(bi))
                });
                indexed.into_iter().map(|(_, a)| a).collect()
            } else {
                items.iter().collect()
            };
            if is_configured_argument_array(items) {
                let resolved_items: Vec<ResolvedArg> = sorted_items
                    .iter()
                    .map(|item| ResolvedArg {
                        name: argument_name(item).to_string(),
                        resolved: resolve_di_argument(item, di_config, const_map),
                    })
                    .collect();
                ResolvedArgValue::Array(resolved_items)
            } else {
                let values: Vec<ResolvedArrayItem> = sorted_items
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

fn resolve_configured_instance_argument(
    arg: &Argument,
    di_config: &DiConfig,
    const_map: &HashMap<String, String>,
) -> ResolvedArgValue {
    resolve_di_argument(arg, di_config, const_map)
}

fn resolve_configured_non_object_argument(
    arg: &Argument,
    constructor_default: Option<&str>,
    di_config: &DiConfig,
    const_map: &HashMap<String, String>,
) -> ResolvedArgValue {
    match arg {
        // When constructor param is non-object but di.xml provides xsi:type="object",
        // PHP serializes it as scalar payload: ['_v_' => ['instance' => 'FQCN']].
        Argument::Object { value, .. } => ResolvedArgValue::PlainArray(vec![ResolvedArrayItem {
            name: "instance".to_string(),
            value: ResolvedArrayValue::Scalar(ResolvedScalar::String(normalize(value))),
        }]),
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

fn resolve_non_object_default(
    default_value: Option<&str>,
    const_map: &HashMap<String, String>,
) -> ResolvedArgValue {
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
    // PHP reflection worker emits `__json__:<json>` for array defaults so we
    // get the resolved values rather than constant expressions.
    if let Some(json_str) = trimmed.strip_prefix("__json__:") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
            return ResolvedArgValue::PlainArray(json_to_plain_array_items(&v));
        }
    }
    if let Some(items) = parse_php_array_default(trimmed) {
        return ResolvedArgValue::PlainArray(items);
    }
    if let Some(s) = parse_quoted_php_string(trimmed) {
        return ResolvedArgValue::Scalar(ResolvedScalar::String(s));
    }
    if is_numeric_literal(trimmed) {
        return ResolvedArgValue::Scalar(ResolvedScalar::Number(trimmed.to_string()));
    }
    // Bare PHP constant (e.g. MCRYPT_BLOWFISH, SORT_REGULAR) — look up in const_map,
    // which is pre-seeded from PHP's get_defined_constants() at startup.
    let normalized = trimmed.trim_start_matches('\\');
    if let Some(resolved) = const_map.get(normalized) {
        return ResolvedArgValue::Scalar(ResolvedScalar::String(resolved.clone()));
    }
    ResolvedArgValue::Scalar(ResolvedScalar::String(trimmed.to_string()))
}

fn json_to_plain_array_items(v: &serde_json::Value) -> Vec<ResolvedArrayItem> {
    match v {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| ResolvedArrayItem {
                name: k.clone(),
                value: json_to_plain_array_value(v),
            })
            .collect(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .enumerate()
            .map(|(i, v)| ResolvedArrayItem {
                name: i.to_string(),
                value: json_to_plain_array_value(v),
            })
            .collect(),
        _ => vec![],
    }
}

fn json_to_plain_array_value(v: &serde_json::Value) -> ResolvedArrayValue {
    match v {
        serde_json::Value::String(s) => {
            ResolvedArrayValue::Scalar(ResolvedScalar::String(s.clone()))
        }
        serde_json::Value::Number(n) => {
            ResolvedArrayValue::Scalar(ResolvedScalar::Number(n.to_string()))
        }
        serde_json::Value::Bool(b) => ResolvedArrayValue::Scalar(ResolvedScalar::Bool(*b)),
        serde_json::Value::Null => ResolvedArrayValue::Null,
        other => ResolvedArrayValue::Array(json_to_plain_array_items(other)),
    }
}

fn parse_php_array_default(input: &str) -> Option<Vec<ResolvedArrayItem>> {
    let mut p = PhpArrayDefaultParser::new(input);
    let items = p.parse_array()?;
    p.skip_ws();
    if !p.is_eof() {
        return None;
    }
    Some(items)
}

struct PhpArrayDefaultParser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> PhpArrayDefaultParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            src: input.as_bytes(),
            pos: 0,
        }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn consume_char(&mut self, ch: u8) -> bool {
        if self.peek() == Some(ch) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn consume_bytes(&mut self, bytes: &[u8]) -> bool {
        let end = self.pos + bytes.len();
        if end > self.src.len() {
            return false;
        }
        if &self.src[self.pos..end] == bytes {
            self.pos = end;
            true
        } else {
            false
        }
    }

    fn consume_keyword_ci(&mut self, kw: &str) -> bool {
        let bytes = kw.as_bytes();
        let end = self.pos + bytes.len();
        if end > self.src.len() {
            return false;
        }
        let segment = &self.src[self.pos..end];
        if !segment.eq_ignore_ascii_case(bytes) {
            return false;
        }
        if let Some(next) = self.src.get(end).copied() {
            if next.is_ascii_alphanumeric() || next == b'_' {
                return false;
            }
        }
        self.pos = end;
        true
    }

    fn parse_array(&mut self) -> Option<Vec<ResolvedArrayItem>> {
        self.skip_ws();
        let close = if self.consume_keyword_ci("array") {
            self.skip_ws();
            if !self.consume_char(b'(') {
                return None;
            }
            b')'
        } else if self.consume_char(b'[') {
            b']'
        } else {
            return None;
        };

        let mut items = Vec::new();
        let mut auto_index: usize = 0;
        loop {
            self.skip_ws();
            if self.consume_char(close) {
                break;
            }

            let first = self.parse_value()?;
            self.skip_ws();

            if self.consume_bytes(b"=>") {
                let key = value_to_php_array_key(&first)?;
                let value = self.parse_value()?;
                items.push(ResolvedArrayItem { name: key, value });
            } else {
                items.push(ResolvedArrayItem {
                    name: auto_index.to_string(),
                    value: first,
                });
                auto_index += 1;
            }

            self.skip_ws();
            if self.consume_char(b',') {
                self.skip_ws();
                if self.consume_char(close) {
                    break;
                }
                continue;
            }
            if self.consume_char(close) {
                break;
            }
            return None;
        }
        Some(items)
    }

    fn parse_value(&mut self) -> Option<ResolvedArrayValue> {
        self.skip_ws();

        if self.peek() == Some(b'[') {
            let nested = self.parse_array()?;
            return Some(ResolvedArrayValue::Array(nested));
        }

        if self.starts_with_keyword_ci("array") {
            let nested = self.parse_array()?;
            return Some(ResolvedArrayValue::Array(nested));
        }

        if let Some(s) = self.parse_string_literal() {
            return Some(ResolvedArrayValue::Scalar(ResolvedScalar::String(s)));
        }

        if let Some(n) = self.parse_numeric_literal() {
            return Some(ResolvedArrayValue::Scalar(ResolvedScalar::Number(n)));
        }

        if self.consume_keyword_ci("true") {
            return Some(ResolvedArrayValue::Scalar(ResolvedScalar::Bool(true)));
        }
        if self.consume_keyword_ci("false") {
            return Some(ResolvedArrayValue::Scalar(ResolvedScalar::Bool(false)));
        }
        if self.consume_keyword_ci("null") {
            return Some(ResolvedArrayValue::Null);
        }

        self.parse_bareword_string()
            .map(|s| ResolvedArrayValue::Scalar(ResolvedScalar::String(s)))
    }

    fn starts_with_keyword_ci(&self, kw: &str) -> bool {
        let bytes = kw.as_bytes();
        let end = self.pos + bytes.len();
        if end > self.src.len() {
            return false;
        }
        let segment = &self.src[self.pos..end];
        if !segment.eq_ignore_ascii_case(bytes) {
            return false;
        }
        if let Some(next) = self.src.get(end).copied() {
            if next.is_ascii_alphanumeric() || next == b'_' {
                return false;
            }
        }
        true
    }

    fn parse_string_literal(&mut self) -> Option<String> {
        let quote = match self.peek() {
            Some(b'\'') => b'\'',
            Some(b'"') => b'"',
            _ => return None,
        };
        self.pos += 1; // opening quote

        let mut out = String::new();
        let mut escaped = false;
        while let Some(c) = self.peek() {
            self.pos += 1;
            if escaped {
                out.push(c as char);
                escaped = false;
                continue;
            }
            if c == b'\\' {
                escaped = true;
                continue;
            }
            if c == quote {
                return Some(out);
            }
            out.push(c as char);
        }
        None
    }

    fn parse_numeric_literal(&mut self) -> Option<String> {
        let start = self.pos;
        let Some(first) = self.peek() else {
            return None;
        };
        if !(first.is_ascii_digit() || first == b'+' || first == b'-') {
            return None;
        }

        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace()
                || c == b','
                || c == b')'
                || c == b']'
                || (c == b'=' && self.src.get(self.pos + 1) == Some(&b'>'))
            {
                break;
            }
            self.pos += 1;
        }
        let token = std::str::from_utf8(&self.src[start..self.pos]).ok()?.trim();
        if token.parse::<f64>().is_ok() {
            Some(token.to_string())
        } else {
            self.pos = start;
            None
        }
    }

    fn parse_bareword_string(&mut self) -> Option<String> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace()
                || c == b','
                || c == b')'
                || c == b']'
                || (c == b'=' && self.src.get(self.pos + 1) == Some(&b'>'))
            {
                break;
            }
            self.pos += 1;
        }
        if self.pos == start {
            return None;
        }
        let token = std::str::from_utf8(&self.src[start..self.pos]).ok()?.trim();
        if token.is_empty() {
            None
        } else {
            Some(token.to_string())
        }
    }
}

fn value_to_php_array_key(value: &ResolvedArrayValue) -> Option<String> {
    match value {
        ResolvedArrayValue::Scalar(ResolvedScalar::String(s))
        | ResolvedArrayValue::Scalar(ResolvedScalar::Number(s)) => Some(s.clone()),
        ResolvedArrayValue::Scalar(ResolvedScalar::Bool(true)) => Some("1".to_string()),
        ResolvedArrayValue::Scalar(ResolvedScalar::Bool(false)) => Some("0".to_string()),
        ResolvedArrayValue::Null => Some(String::new()),
        ResolvedArrayValue::Array(_) => None,
    }
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
        Argument::String { value, .. } => {
            ResolvedArrayValue::Scalar(ResolvedScalar::String(value.clone()))
        }
        Argument::Boolean { value, .. } => ResolvedArrayValue::Scalar(ResolvedScalar::Bool(*value)),
        Argument::Number { value, .. } => {
            ResolvedArrayValue::Scalar(ResolvedScalar::Number(value.clone()))
        }
        Argument::Null { .. } => ResolvedArrayValue::Null,
        Argument::Const { value, .. } => {
            ResolvedArrayValue::Scalar(ResolvedScalar::String(value.clone()))
        }
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
        Argument::Object { value, .. } => {
            ResolvedArrayValue::Scalar(ResolvedScalar::String(value.clone()))
        }
        Argument::Init { value, .. } => {
            ResolvedArrayValue::Scalar(ResolvedScalar::String(value.clone()))
        }
    }
}

fn argument_name(arg: &Argument) -> &str {
    match arg {
        Argument::Object { name, .. }
        | Argument::String { name, .. }
        | Argument::Boolean { name, .. }
        | Argument::Number { name, .. }
        | Argument::Null { name, .. }
        | Argument::Array { name, .. }
        | Argument::Init { name, .. }
        | Argument::Const { name, .. } => name.as_str(),
    }
}

fn argument_sort_order(arg: &Argument) -> i32 {
    match arg {
        Argument::Object { sort_order, .. }
        | Argument::String { sort_order, .. }
        | Argument::Boolean { sort_order, .. }
        | Argument::Number { sort_order, .. }
        | Argument::Null { sort_order, .. }
        | Argument::Array { sort_order, .. }
        | Argument::Init { sort_order, .. }
        | Argument::Const { sort_order, .. } => *sort_order,
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
                    value: "Magento\\Framework\\View\\Asset\\PreProcessor\\Passthrough".to_string(),
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
    fn test_non_object_param_with_object_di_value_serializes_as_v_instance_shape() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "App\\Method".to_string(),
            make_class("App\\Method", vec![("formBlockType", None)]),
        );

        let mut di_config = DiConfig::default();
        di_config.type_configs.insert(
            "App\\Method".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Object {
                    name: "formBlockType".to_string(),
                    value: "Magento\\Payment\\Block\\Form".to_string(),
                    shared: None,
                }],
            },
        );

        let map = resolve_all_arguments(&class_map, &di_config, &HashMap::new());
        let args = &map["App\\Method"];
        assert!(matches!(
            &args[0].resolved,
            ResolvedArgValue::PlainArray(items)
            if items.len() == 1
                && items[0].name == "instance"
                && matches!(
                    &items[0].value,
                    ResolvedArrayValue::Scalar(ResolvedScalar::String(v))
                    if v == "Magento\\Payment\\Block\\Form"
                )
        ));
    }

    #[test]
    fn test_virtual_type_object_item_honors_explicit_preference() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "App\\RequestValidator".to_string(),
            make_class("App\\RequestValidator", vec![("validators", Some("array"))]),
        );

        let mut di_config = DiConfig::default();
        di_config.virtual_types.insert(
            "CsrfRequestValidator".to_string(),
            di_xml_reader::VirtualType {
                name: "CsrfRequestValidator".to_string(),
                type_name: "Magento\\Framework\\App\\Request\\CsrfValidator".to_string(),
            },
        );
        di_config.preferences.insert(
            "CsrfRequestValidator".to_string(),
            "Magento\\Backend\\App\\Request\\BackendValidator".to_string(),
        );
        di_config.type_configs.insert(
            "App\\RequestValidator".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Array {
                    name: "validators".to_string(),
                    items: vec![Argument::Object {
                        name: "csrf_validator".to_string(),
                        value: "CsrfRequestValidator".to_string(),
                        shared: None,
                    }],
                }],
            },
        );
        di_config.refresh_lookup_indexes();

        let map = resolve_all_arguments(&class_map, &di_config, &HashMap::new());
        let args = &map["App\\RequestValidator"];
        let validators = args
            .iter()
            .find(|a| a.name == "validators")
            .expect("validators arg");
        let by_name: HashMap<_, _> = match &validators.resolved {
            ResolvedArgValue::Array(items) => items
                .iter()
                .map(|i| (i.name.as_str(), &i.resolved))
                .collect(),
            other => panic!("expected configured array, got {other:?}"),
        };
        assert!(matches!(
            by_name.get("csrf_validator").copied(),
            Some(ResolvedArgValue::SharedInstance(v))
            if v == "Magento\\Backend\\App\\Request\\BackendValidator"
        ));
    }

    #[test]
    fn test_virtual_type_object_item_without_explicit_preference_keeps_alias() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "App\\RequestValidator".to_string(),
            make_class("App\\RequestValidator", vec![("validators", Some("array"))]),
        );

        let mut di_config = DiConfig::default();
        di_config.virtual_types.insert(
            "CsrfRequestValidator".to_string(),
            di_xml_reader::VirtualType {
                name: "CsrfRequestValidator".to_string(),
                type_name: "Magento\\Framework\\App\\Request\\CsrfValidator".to_string(),
            },
        );
        di_config.type_configs.insert(
            "App\\RequestValidator".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Array {
                    name: "validators".to_string(),
                    items: vec![Argument::Object {
                        name: "csrf_validator".to_string(),
                        value: "CsrfRequestValidator".to_string(),
                        shared: None,
                    }],
                }],
            },
        );

        let map = resolve_all_arguments(&class_map, &di_config, &HashMap::new());
        let args = &map["App\\RequestValidator"];
        let validators = args
            .iter()
            .find(|a| a.name == "validators")
            .expect("validators arg");
        let by_name: HashMap<_, _> = match &validators.resolved {
            ResolvedArgValue::Array(items) => items
                .iter()
                .map(|i| (i.name.as_str(), &i.resolved))
                .collect(),
            other => panic!("expected configured array, got {other:?}"),
        };
        assert!(matches!(
            by_name.get("csrf_validator").copied(),
            Some(ResolvedArgValue::SharedInstance(v))
            if v == "CsrfRequestValidator"
        ));
    }

    #[test]
    fn test_named_virtual_type_inherits_base_constructor_and_virtual_overrides() {
        let (class_map, di_config) = preprocessor_pool_fixture();
        let map = resolve_all_arguments_for_named_types(
            &["AssetPreProcessorPool".to_string()],
            &class_map,
            &HashSet::new(),
            &di_config,
            &HashMap::new(),
        );
        let args = &map["AssetPreProcessorPool"];

        let by_name: HashMap<_, _> = args
            .iter()
            .map(|a| (a.name.as_str(), &a.resolved))
            .collect();

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
            &HashSet::new(),
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

    #[test]
    fn test_interface_arguments_merge_into_concrete_with_recursive_array_merge() {
        let mut class_map = HashMap::new();
        let mut command_list = make_class(
            "Magento\\Framework\\Console\\CommandList",
            vec![("commands", Some("array"))],
        );
        command_list.implements =
            vec!["Magento\\Framework\\Console\\CommandListInterface".to_string()];
        class_map.insert(command_list.fqcn.clone(), command_list);

        let mut di_config = DiConfig::default();
        di_config.type_configs.insert(
            "Magento\\Framework\\Console\\CommandListInterface".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Array {
                    name: "commands".to_string(),
                    items: vec![
                        Argument::Object {
                            name: "core".to_string(),
                            value: "Vendor\\Core\\Command".to_string(),
                            shared: None,
                        },
                        Argument::Object {
                            name: "override_me".to_string(),
                            value: "Vendor\\Legacy\\Command".to_string(),
                            shared: None,
                        },
                    ],
                }],
            },
        );
        di_config.type_configs.insert(
            "Magento\\Framework\\Console\\CommandList".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Array {
                    name: "commands".to_string(),
                    items: vec![
                        Argument::Object {
                            name: "local".to_string(),
                            value: "Vendor\\Local\\Command".to_string(),
                            shared: None,
                        },
                        Argument::Object {
                            name: "override_me".to_string(),
                            value: "Vendor\\New\\Command".to_string(),
                            shared: None,
                        },
                    ],
                }],
            },
        );

        let resolved = resolve_all_arguments(&class_map, &di_config, &HashMap::new());
        let args = resolved
            .get("Magento\\Framework\\Console\\CommandList")
            .expect("resolved arguments");
        let commands = args
            .iter()
            .find(|a| a.name == "commands")
            .expect("commands argument must exist");
        let by_name: HashMap<_, _> = match &commands.resolved {
            ResolvedArgValue::Array(items) => items
                .iter()
                .map(|i| (i.name.as_str(), &i.resolved))
                .collect(),
            other => panic!("expected configured array for commands, got {other:?}"),
        };

        assert_eq!(by_name.len(), 3);
        assert!(matches!(
            by_name.get("core").copied(),
            Some(ResolvedArgValue::SharedInstance(v)) if v == "Vendor\\Core\\Command"
        ));
        assert!(matches!(
            by_name.get("local").copied(),
            Some(ResolvedArgValue::SharedInstance(v)) if v == "Vendor\\Local\\Command"
        ));
        assert!(matches!(
            by_name.get("override_me").copied(),
            Some(ResolvedArgValue::SharedInstance(v)) if v == "Vendor\\New\\Command"
        ));
    }

    #[test]
    fn test_virtual_type_array_argument_merges_parent_sources_without_dropping_entries() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "Magento\\Framework\\App\\Config\\ConfigSourceAggregated".to_string(),
            make_class(
                "Magento\\Framework\\App\\Config\\ConfigSourceAggregated",
                vec![("sources", Some("array"))],
            ),
        );

        let mut di_config = DiConfig::default();
        di_config.virtual_types.insert(
            "systemConfigSnapshotSourceAggregated".to_string(),
            di_xml_reader::VirtualType {
                name: "systemConfigSnapshotSourceAggregated".to_string(),
                type_name: "Magento\\Framework\\App\\Config\\ConfigSourceAggregated".to_string(),
            },
        );
        di_config.type_configs.insert(
            "Magento\\Framework\\App\\Config\\ConfigSourceAggregated".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Array {
                    name: "sources".to_string(),
                    items: vec![
                        Argument::Array {
                            name: "modular".to_string(),
                            items: vec![
                                Argument::Object {
                                    name: "source".to_string(),
                                    value:
                                        "Magento\\Config\\App\\Config\\Source\\ModularConfigSource"
                                            .to_string(),
                                    shared: None,
                                },
                                Argument::String {
                                    name: "sortOrder".to_string(),
                                    value: "10".to_string(),
                                },
                            ],
                        },
                        Argument::Array {
                            name: "dynamic".to_string(),
                            items: vec![
                                Argument::Object {
                                    name: "source".to_string(),
                                    value:
                                        "Magento\\Config\\App\\Config\\Source\\RuntimeConfigSource"
                                            .to_string(),
                                    shared: None,
                                },
                                Argument::String {
                                    name: "sortOrder".to_string(),
                                    value: "100".to_string(),
                                },
                            ],
                        },
                    ],
                }],
            },
        );
        di_config.type_configs.insert(
            "systemConfigSnapshotSourceAggregated".to_string(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Array {
                    name: "sources".to_string(),
                    items: vec![Argument::Array {
                        name: "initial".to_string(),
                        items: vec![
                            Argument::Object {
                                name: "source".to_string(),
                                value: "Magento\\Config\\App\\Config\\Source\\InitialSnapshotConfigSource"
                                    .to_string(),
                                shared: None,
                            },
                            Argument::String {
                                name: "sortOrder".to_string(),
                                value: "1000".to_string(),
                            },
                        ],
                    }],
                }],
            },
        );

        let resolved = resolve_all_arguments_for_named_types(
            &["systemConfigSnapshotSourceAggregated".to_string()],
            &class_map,
            &HashSet::new(),
            &di_config,
            &HashMap::new(),
        );
        let args = resolved
            .get("systemConfigSnapshotSourceAggregated")
            .expect("resolved virtual type arguments");
        let sources = args
            .iter()
            .find(|a| a.name == "sources")
            .expect("sources argument must exist");
        let by_name: HashMap<_, _> = match &sources.resolved {
            ResolvedArgValue::Array(items) => items
                .iter()
                .map(|i| (i.name.as_str(), &i.resolved))
                .collect(),
            other => panic!("expected configured array for sources, got {other:?}"),
        };

        assert_eq!(by_name.len(), 3);
        for key in ["modular", "dynamic", "initial"] {
            let entry = by_name.get(key).copied().expect("source entry");
            let inner: HashMap<_, _> = match entry {
                ResolvedArgValue::Array(items) => items
                    .iter()
                    .map(|i| (i.name.as_str(), &i.resolved))
                    .collect(),
                other => panic!("expected nested configured array for {key}, got {other:?}"),
            };
            assert!(inner.contains_key("source"));
            assert!(inner.contains_key("sortOrder"));
        }
    }

    #[test]
    fn test_optional_constructor_var_export_array_default_is_parsed_as_plain_array() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "Magento\\Store\\Model\\StoreResolver\\ReaderList".to_string(),
            make_class_with_constructor(
                "Magento\\Store\\Model\\StoreResolver\\ReaderList",
                vec![ConstructorParam {
                    name: "resolverMap".to_string(),
                    type_hint: None,
                    is_optional: true,
                    default_value: Some(
                        "array (\n  'website' => 'Magento\\\\Store\\\\Model\\\\StoreResolver\\\\Website',\n  'group' => 'Magento\\\\Store\\\\Model\\\\StoreResolver\\\\Group',\n  'store' => 'Magento\\\\Store\\\\Model\\\\StoreResolver\\\\Store',\n)"
                            .to_string(),
                    ),
                    is_primitive: false,
                    is_variadic: false,
                    is_promoted: false,
                }],
            ),
        );

        let map = resolve_all_arguments(&class_map, &DiConfig::default(), &HashMap::new());
        let args = &map["Magento\\Store\\Model\\StoreResolver\\ReaderList"];
        let resolver_map = args
            .iter()
            .find(|a| a.name == "resolverMap")
            .expect("resolverMap arg");

        let by_key: HashMap<_, _> = match &resolver_map.resolved {
            ResolvedArgValue::PlainArray(items) => {
                items.iter().map(|i| (i.name.as_str(), &i.value)).collect()
            }
            other => panic!("expected plain array default, got {other:?}"),
        };

        assert_eq!(by_key.len(), 3);
        assert!(matches!(
            by_key.get("website").copied(),
            Some(ResolvedArrayValue::Scalar(ResolvedScalar::String(v)))
            if v == "Magento\\Store\\Model\\StoreResolver\\Website"
        ));
        assert!(matches!(
            by_key.get("group").copied(),
            Some(ResolvedArrayValue::Scalar(ResolvedScalar::String(v)))
            if v == "Magento\\Store\\Model\\StoreResolver\\Group"
        ));
        assert!(matches!(
            by_key.get("store").copied(),
            Some(ResolvedArrayValue::Scalar(ResolvedScalar::String(v)))
            if v == "Magento\\Store\\Model\\StoreResolver\\Store"
        ));
    }

    #[test]
    fn test_optional_constructor_short_array_default_is_parsed_as_plain_array() {
        let mut class_map = HashMap::new();
        class_map.insert(
            "App\\ArrayDefaults".to_string(),
            make_class_with_constructor(
                "App\\ArrayDefaults",
                vec![ConstructorParam {
                    name: "payload".to_string(),
                    type_hint: None,
                    is_optional: true,
                    default_value: Some(
                        "['a' => 'alpha', 'b' => 2, 'c' => ['d' => false], 'e' => null]"
                            .to_string(),
                    ),
                    is_primitive: false,
                    is_variadic: false,
                    is_promoted: false,
                }],
            ),
        );

        let map = resolve_all_arguments(&class_map, &DiConfig::default(), &HashMap::new());
        let args = &map["App\\ArrayDefaults"];
        let payload = args
            .iter()
            .find(|a| a.name == "payload")
            .expect("payload arg");

        let by_key: HashMap<_, _> = match &payload.resolved {
            ResolvedArgValue::PlainArray(items) => {
                items.iter().map(|i| (i.name.as_str(), &i.value)).collect()
            }
            other => panic!("expected plain array default, got {other:?}"),
        };

        assert!(matches!(
            by_key.get("a").copied(),
            Some(ResolvedArrayValue::Scalar(ResolvedScalar::String(v))) if v == "alpha"
        ));
        assert!(matches!(
            by_key.get("b").copied(),
            Some(ResolvedArrayValue::Scalar(ResolvedScalar::Number(v))) if v == "2"
        ));
        assert!(matches!(
            by_key.get("e").copied(),
            Some(ResolvedArrayValue::Null)
        ));

        let nested = by_key.get("c").copied().expect("nested c");
        let nested_map: HashMap<_, _> = match nested {
            ResolvedArrayValue::Array(items) => {
                items.iter().map(|i| (i.name.as_str(), &i.value)).collect()
            }
            other => panic!("expected nested array, got {other:?}"),
        };
        assert!(matches!(
            nested_map.get("d").copied(),
            Some(ResolvedArrayValue::Scalar(ResolvedScalar::Bool(false)))
        ));
    }
}
