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

use rustc_hash::{FxHashMap, FxHashSet};

use php_extractor::{ClassInfo, MethodSignature};

use crate::graph::{InterceptorSpec, PluginRef};
use di_xml_reader::DiConfig;

/// Build the list of classes that need interceptors.
pub fn detect_interceptors(
    class_map: &FxHashMap<String, ClassInfo>,
    di_config: &DiConfig,
) -> Vec<InterceptorSpec> {
    // -----------------------------------------------------------------------
    // Phase 1: Classes with direct plugin registrations in di.xml
    // -----------------------------------------------------------------------
    let mut specs: Vec<InterceptorSpec> = Vec::new();
    let mut directly_intercepted: FxHashSet<String> = FxHashSet::default();

    for (owner_name, plugins) in &di_config.plugins {
        let active: Vec<&di_xml_reader::Plugin> = plugins.iter().filter(|p| !p.disabled).collect();
        if active.is_empty() {
            continue;
        }

        // Resolve the concrete type to check is_final
        let concrete = di_config.get_instance_type(owner_name);
        let info = class_map
            .get(&concrete)
            .or_else(|| class_map.get(owner_name));

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
        // Skip Proxy-suffix NoninterceptableInterface implementors (generated proxy wrappers);
        // other source classes that implement NoninterceptableInterface (e.g. StructureLazy)
        // still get interceptors per PHP truth.
        let is_concrete = match info {
            None => false, // class not found on disk → skip
            Some(info) => {
                use php_extractor::types::ClassKind;
                let short = owner_name
                    .split('\\')
                    .next_back()
                    .unwrap_or(owner_name.as_str());
                let is_proxy_noninterceptable = short.ends_with("Proxy")
                    && info.implements.iter().any(|iface| {
                        iface.trim_start_matches('\\')
                            == "Magento\\Framework\\ObjectManager\\NoninterceptableInterface"
                    });
                !info.is_abstract
                    && !matches!(info.kind, ClassKind::Interface | ClassKind::Trait)
                    && !is_proxy_noninterceptable
            }
        };
        if !is_concrete {
            continue;
        }

        // When the plugin owner is a virtual type, PHP generates the interceptor for
        // the resolved concrete class, not for the VT name. The VT → Interceptor mapping
        // is handled via instanceTypes, not via area preferences. Using the concrete
        // class fqcn ensures we don't emit spurious VtName → VtName\Interceptor entries
        // in the area config preferences section.
        let spec_fqcn = if di_config.virtual_types.contains_key(owner_name.as_str()) {
            concrete.clone()
        } else {
            owner_name.clone()
        };

        // If the concrete fqcn is already directly intercepted (perhaps it has its own
        // plugins registered separately), skip to avoid duplicate specs.
        if spec_fqcn != *owner_name && directly_intercepted.contains(spec_fqcn.as_str()) {
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

        // Even for directly intercepted classes, runtime plugin resolution includes
        // plugins declared on ancestors in the extends chain. Use the inherited
        // plugin method surface to avoid dropping parent-plugin methods.
        let intercepted_method_names =
            derive_intercepted_methods_from_ancestor_plugins(owner_name, class_map, di_config);
        let public_methods = if intercepted_method_names.is_empty() {
            // When plugin class methods cannot be resolved, avoid emitting the full
            // inherited method surface. Fall back to target-declared methods only.
            select_interceptor_methods(&spec_fqcn, class_map, None, false)
        } else {
            select_interceptor_methods(&spec_fqcn, class_map, Some(&intercepted_method_names), true)
        };

        specs.push(InterceptorSpec {
            fqcn: spec_fqcn,
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
    let intercepted_set: FxHashSet<&str> =
        directly_intercepted.iter().map(|s| s.as_str()).collect();

    // Build a cache to avoid repeated ancestor walks.
    // `ancestor_intercepted` memoizes: fqcn → bool
    let mut ancestor_cache: FxHashMap<&str, bool> = FxHashMap::default();

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
        // Skip Proxy-suffix classes that implement NoninterceptableInterface — these
        // are generated Proxy wrappers (Layout\Proxy, Cache\Proxy, LoggerProxy, etc.).
        // Source classes like StructureLazy that happen to implement NoninterceptableInterface
        // are NOT skipped; PHP still generates their Interceptor and metadata args.
        let short_name = fqcn.split('\\').next_back().unwrap_or(fqcn.as_str());
        if short_name.ends_with("Proxy")
            && info.implements.iter().any(|iface| {
                iface.trim_start_matches('\\')
                    == "Magento\\Framework\\ObjectManager\\NoninterceptableInterface"
            })
        {
            continue;
        }
        // Check inheritance chain.
        if has_intercepted_ancestor(fqcn, class_map, &intercepted_set, &mut ancestor_cache) {
            let inherited_method_names =
                derive_intercepted_methods_from_ancestor_plugins(fqcn, class_map, di_config);
            let public_methods = if inherited_method_names.is_empty() {
                select_interceptor_methods(fqcn, class_map, None, false)
            } else {
                select_interceptor_methods(fqcn, class_map, Some(&inherited_method_names), true)
            };
            specs.push(InterceptorSpec {
                fqcn: fqcn.clone(),
                plugins: vec![], // resolved at runtime by plugin framework
                public_methods,
            });
        }
    }

    specs.sort_by(|a, b| a.fqcn.cmp(&b.fqcn));
    specs
}

fn select_interceptor_methods(
    fqcn: &str,
    class_map: &FxHashMap<String, ClassInfo>,
    intercepted_method_names: Option<&FxHashSet<String>>,
    include_inherited: bool,
) -> Vec<MethodSignature> {
    let methods = if include_inherited {
        collect_public_methods_with_inheritance(fqcn, class_map)
    } else {
        collect_public_methods_declared_only(fqcn, class_map)
    };
    methods
        .into_iter()
        .filter(|m| is_interceptable_method(m))
        .filter(|m| {
            if let Some(names) = intercepted_method_names {
                names.contains(&m.name)
            } else {
                true
            }
        })
        .collect()
}

fn collect_public_methods_declared_only(
    fqcn: &str,
    class_map: &FxHashMap<String, ClassInfo>,
) -> Vec<MethodSignature> {
    class_map
        .get(fqcn)
        .map(|info| info.public_methods.clone())
        .unwrap_or_default()
}

fn collect_public_methods_with_inheritance(
    fqcn: &str,
    class_map: &FxHashMap<String, ClassInfo>,
) -> Vec<MethodSignature> {
    let mut result = Vec::new();
    let mut seen = FxHashSet::default();
    let mut cursor = Some(fqcn.to_string());

    while let Some(current) = cursor {
        let Some(info) = class_map.get(&current) else {
            break;
        };
        for method in &info.public_methods {
            if seen.insert(method.name.clone()) {
                result.push(method.clone());
            }
        }
        cursor = info.extends.clone();
    }

    result
}

fn derive_intercepted_methods_from_plugins(
    plugins: &[PluginRef],
    class_map: &FxHashMap<String, ClassInfo>,
    di_config: &DiConfig,
) -> FxHashSet<String> {
    let mut methods = FxHashSet::default();

    for plugin in plugins {
        let resolved_plugin_type = di_config.get_instance_type(&plugin.type_name);
        let plugin_info = class_map
            .get(&resolved_plugin_type)
            .or_else(|| class_map.get(&plugin.type_name));
        let Some(plugin_info) = plugin_info else {
            continue;
        };

        for method in &plugin_info.public_methods {
            if let Some(intercepted_method) = plugin_method_to_intercepted(&method.name) {
                methods.insert(intercepted_method);
            }
        }
    }

    methods
}

fn derive_intercepted_methods_from_ancestor_plugins(
    fqcn: &str,
    class_map: &FxHashMap<String, ClassInfo>,
    di_config: &DiConfig,
) -> FxHashSet<String> {
    let mut methods = FxHashSet::default();
    let mut seen = FxHashSet::default();
    let mut stack = vec![fqcn.to_string()];

    while let Some(current) = stack.pop() {
        if !seen.insert(current.clone()) {
            continue;
        }

        if let Some(plugins) = di_config.plugins.get(&current) {
            let plugin_refs: Vec<PluginRef> = plugins
                .iter()
                .filter(|p| !p.disabled)
                .map(|p| PluginRef {
                    name: p.name.clone(),
                    type_name: p.type_name.clone(),
                    sort_order: p.sort_order,
                })
                .collect();
            if !plugin_refs.is_empty() {
                methods.extend(derive_intercepted_methods_from_plugins(
                    &plugin_refs,
                    class_map,
                    di_config,
                ));
            }
        }

        if let Some(info) = class_map.get(&current) {
            if let Some(parent) = &info.extends {
                stack.push(parent.clone());
            }
            for interface in &info.implements {
                stack.push(interface.clone());
            }
        }
    }

    methods
}

fn plugin_method_to_intercepted(method: &str) -> Option<String> {
    if let Some(rest) = method.strip_prefix("before") {
        return lcfirst_nonempty(rest);
    }
    if let Some(rest) = method.strip_prefix("around") {
        return lcfirst_nonempty(rest);
    }
    if let Some(rest) = method.strip_prefix("after") {
        return lcfirst_nonempty(rest);
    }
    None
}

fn lcfirst_nonempty(s: &str) -> Option<String> {
    let mut chars = s.chars();
    let first = chars.next()?;
    let mut out = first.to_lowercase().to_string();
    out.push_str(chars.as_str());
    Some(out)
}

fn is_interceptable_method(method: &MethodSignature) -> bool {
    if method.is_static {
        return false;
    }
    !matches!(
        method.name.as_str(),
        "__construct" | "__destruct" | "__sleep" | "__wakeup" | "__clone" | "_resetState"
    )
}

/// Walk the `extends` chain AND all `implements` interfaces of `fqcn`, returning
/// `true` if any ancestor / interface is in `intercepted_set`.
/// Mirrors Magento's `Relations::getParents()` which includes both extends and implements.
/// Uses `cache` to memoize results.
fn has_intercepted_ancestor<'a>(
    fqcn: &'a str,
    class_map: &'a FxHashMap<String, ClassInfo>,
    intercepted_set: &FxHashSet<&str>,
    cache: &mut FxHashMap<&'a str, bool>,
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
    use php_extractor::types::{ClassInfo, ClassKind, MethodSignature};
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
        Plugin {
            name: name.to_string(),
            type_name: type_name.to_string(),
            sort_order,
            disabled,
        }
    }

    fn make_method(name: &str, is_static: bool) -> MethodSignature {
        MethodSignature {
            name: name.to_string(),
            params: vec![],
            return_type: None,
            is_static,
            returns_reference: false,
        }
    }

    #[test]
    fn test_detects_class_with_plugin() {
        let mut class_map = FxHashMap::default();
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
        let mut class_map = FxHashMap::default();
        class_map.insert("Foo\\Final".to_string(), make_class("Foo\\Final", true));
        let mut di_config = DiConfig::default();
        di_config.plugins.insert(
            "Foo\\Final".to_string(),
            vec![make_plugin("p", "Foo\\P", 0, false)],
        );
        let specs = detect_interceptors(&class_map, &di_config);
        assert!(specs.is_empty());
    }

    #[test]
    fn test_skips_disabled_plugins() {
        let mut class_map = FxHashMap::default();
        class_map.insert("Foo\\Bar".to_string(), make_class("Foo\\Bar", false));
        let mut di_config = DiConfig::default();
        di_config.plugins.insert(
            "Foo\\Bar".to_string(),
            vec![make_plugin("p", "Foo\\P", 0, true)],
        );
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
        let mut class_map = FxHashMap::default();
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
        let mut class_map = FxHashMap::default();
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
        let mut class_map = FxHashMap::default();
        class_map.insert("Foo\\Bar".to_string(), make_class("Foo\\Bar", false));
        let mut di_config = DiConfig::default();
        di_config.plugins.insert(
            "Foo\\Bar".to_string(),
            vec![
                make_plugin("b", "B", 20, false),
                make_plugin("a", "A", 10, false),
            ],
        );
        let specs = detect_interceptors(&class_map, &di_config);
        assert_eq!(specs[0].plugins[0].name, "a");
        assert_eq!(specs[0].plugins[1].name, "b");
    }

    #[test]
    fn test_methods_filtered_by_plugin_method_list_and_skip_rules() {
        let mut target = make_class("Foo\\Bar", false);
        target.public_methods = vec![
            make_method("run", false),
            make_method("__sleep", false),
            make_method("_resetState", false),
            make_method("staticMethod", true),
        ];

        let mut plugin = make_class("Foo\\Plugin", false);
        plugin.public_methods = vec![
            make_method("beforeRun", false),
            make_method("aroundRun", false),
            make_method("afterRun", false),
            make_method("beforeStaticMethod", false),
        ];

        let mut class_map = FxHashMap::default();
        class_map.insert("Foo\\Bar".to_string(), target);
        class_map.insert("Foo\\Plugin".to_string(), plugin);

        let mut di_config = DiConfig::default();
        di_config.plugins.insert(
            "Foo\\Bar".to_string(),
            vec![make_plugin("p", "Foo\\Plugin", 10, false)],
        );

        let specs = detect_interceptors(&class_map, &di_config);
        assert_eq!(specs.len(), 1);
        let names: Vec<&str> = specs[0]
            .public_methods
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(names, vec!["run"]);
    }

    #[test]
    fn test_unresolved_plugin_class_falls_back_to_declared_methods_only() {
        let mut base = make_class("Foo\\Base", false);
        base.public_methods = vec![make_method("baseMethod", false)];

        let mut target = make_class("Foo\\Bar", false);
        target.extends = Some("Foo\\Base".to_string());
        target.public_methods = vec![make_method("run", false)];

        let mut class_map = FxHashMap::default();
        class_map.insert("Foo\\Base".to_string(), base);
        class_map.insert("Foo\\Bar".to_string(), target);

        let mut di_config = DiConfig::default();
        di_config.plugins.insert(
            "Foo\\Bar".to_string(),
            vec![make_plugin("p", "Foo\\MissingPluginClass", 10, false)],
        );

        let specs = detect_interceptors(&class_map, &di_config);
        assert_eq!(specs.len(), 1);
        let names: Vec<&str> = specs[0]
            .public_methods
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(names, vec!["run"]);
    }

    #[test]
    fn test_inherited_interceptor_uses_ancestor_plugin_method_surface() {
        let mut parent = make_class("Foo\\Parent", false);
        parent.public_methods = vec![make_method("getForm", false)];

        let mut child = make_class("Foo\\Child", false);
        child.extends = Some("Foo\\Parent".to_string());
        child.public_methods = vec![
            make_method("getSupportTopics", false),
            make_method("getIssuesTopics", false),
        ];

        let mut plugin = make_class("Foo\\Plugin", false);
        plugin.public_methods = vec![make_method("afterGetForm", false)];

        let mut class_map = FxHashMap::default();
        class_map.insert("Foo\\Parent".to_string(), parent);
        class_map.insert("Foo\\Child".to_string(), child);
        class_map.insert("Foo\\Plugin".to_string(), plugin);

        let mut di_config = DiConfig::default();
        di_config.plugins.insert(
            "Foo\\Parent".to_string(),
            vec![make_plugin("p", "Foo\\Plugin", 10, false)],
        );

        let specs = detect_interceptors(&class_map, &di_config);
        let child_spec = specs
            .iter()
            .find(|s| s.fqcn == "Foo\\Child")
            .expect("child interceptor");
        let names: Vec<&str> = child_spec
            .public_methods
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(names, vec!["getForm"]);
    }
}
