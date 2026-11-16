//! TKT-032: plugin-list metadata generation.
//!
//! Generates Magento-style compiled plugin list metadata files:
//! `primary|global|...|plugin-list.php`

use indexmap::IndexMap;
use rustc_hash::{FxHashMap, FxHashSet};

use di_xml_reader::{DiConfig, Plugin as DiPlugin};
use php_extractor::types::ClassKind;
use php_extractor::ClassInfo;

use crate::metadata::escape_php;

const LISTENER_BEFORE: u8 = 1;
const LISTENER_AROUND: u8 = 2;
const LISTENER_AFTER: u8 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginConfig {
    pub name: String,
    pub sort_order: i32,
    /// Empty when no scope or parent has given this entry a class yet.
    pub instance: String,
    /// `None` when no declaration sets `disabled`. Magento then writes no key.
    pub disabled: Option<bool>,
    /// Magento keeps array keys in first-seen order, so this row lists `disabled` last.
    pub disabled_after_instance: bool,
}

impl PluginConfig {
    fn is_disabled(&self) -> bool {
        self.disabled == Some(true)
    }

    /// array_replace_recursive() of two rows, as in `PluginListGenerator::inheritPlugins()`.
    fn replace_with(&mut self, later: &PluginConfig) {
        if !self.instance.is_empty() && self.disabled.is_none() && later.disabled.is_some() {
            self.disabled_after_instance = true;
        }
        self.sort_order = later.sort_order;
        if !later.instance.is_empty() {
            self.instance = later.instance.clone();
        }
        if later.disabled.is_some() {
            self.disabled = later.disabled;
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessedMethod {
    order: Vec<u8>,
    pub before: Vec<String>,
    pub around: Option<String>,
    pub after: Vec<String>,
}

impl ProcessedMethod {
    fn mark_order(&mut self, listener: u8) {
        if !self.order.contains(&listener) {
            self.order.push(listener);
        }
    }

    fn push_before(&mut self, code: String) {
        self.mark_order(LISTENER_BEFORE);
        self.before.push(code);
    }

    fn set_around(&mut self, code: String) {
        self.mark_order(LISTENER_AROUND);
        self.around = Some(code);
    }

    fn push_after(&mut self, code: String) {
        self.mark_order(LISTENER_AFTER);
        self.after.push(code);
    }

    fn order(&self) -> &[u8] {
        &self.order
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginListMetadata {
    pub plugin_data: IndexMap<String, Vec<PluginConfig>>,
    pub inherited: IndexMap<String, Option<Vec<PluginConfig>>>,
    pub processed: IndexMap<String, ProcessedMethod>,
    /// Active plugin classes that may fail `class_exists()`: not scanned, or not a class. Only
    /// PHP can tell, because Magento's check goes through the autoloader.
    pub unconfirmed_plugin_classes: Vec<String>,
}

/// Build the in-memory plugin list structures for one config:
/// section 0 => pluginData, section 1 => inherited, section 2 => processed.
///
/// `include_virtual_types`: visit every virtual type in the config first, in name order.
pub fn compile_plugin_list(
    di_config: &DiConfig,
    class_map: &FxHashMap<String, ClassInfo>,
    class_definitions: &[String],
    include_virtual_types: bool,
) -> PluginListMetadata {
    let mut virtual_types: Vec<String> = Vec::new();
    if include_virtual_types {
        virtual_types.extend(di_config.virtual_types.keys().cloned());
        virtual_types.sort();
    }
    compile_scope_plugin_list(
        di_config,
        di_config,
        class_map,
        class_definitions,
        &virtual_types,
    )
}

/// Build the plugin list of one scope in the order of `PluginListGenerator::write()`:
/// the scope's virtual types, the plugin owners, then the class definitions. The first visit
/// of a type fixes its position and what it hands to its children, see `inherit_plugins`.
///
/// `scope_config` holds the plugins of this scope. `om_config` resolves virtual types and is
/// the global config in every scope, because Magento's `omConfig` does not load area files.
pub fn compile_scope_plugin_list(
    scope_config: &DiConfig,
    om_config: &DiConfig,
    class_map: &FxHashMap<String, ClassInfo>,
    class_definitions: &[String],
    scope_virtual_types: &[String],
) -> PluginListMetadata {
    let plugin_data = collect_plugin_data(scope_config);
    let mut inherited: IndexMap<String, Option<Vec<PluginConfig>>> = IndexMap::new();
    let mut processed: IndexMap<String, ProcessedMethod> = IndexMap::new();
    let mut plugin_methods_cache: FxHashMap<String, IndexMap<String, u8>> = FxHashMap::default();

    // 1) Virtual types the scope declares.
    for vt in scope_virtual_types {
        inherit_plugins(
            vt,
            &plugin_data,
            om_config,
            class_map,
            &mut inherited,
            &mut processed,
            &mut plugin_methods_cache,
        );
    }

    // 2) Explicit plugin owners.
    for owner in plugin_data.keys() {
        inherit_plugins(
            owner,
            &plugin_data,
            om_config,
            class_map,
            &mut inherited,
            &mut processed,
            &mut plugin_methods_cache,
        );
    }

    // 3) Class definitions passed from interception phase.
    let mut class_names: Vec<String> = class_definitions.iter().map(|c| normalize(c)).collect();
    class_names.sort();
    class_names.dedup();
    for class_name in class_names {
        // TKT-049: PHP excludes pure interfaces from Section 1 unless they are directly
        // in Section 0 (i.e., have plugins registered on them). Skip them here.
        let is_pure_interface = class_map
            .get(class_name.trim_start_matches('\\'))
            .map(|c| matches!(c.kind, ClassKind::Interface))
            .unwrap_or(false);
        if is_pure_interface && !plugin_data.contains_key(class_name.as_str()) {
            continue;
        }
        inherit_plugins(
            &class_name,
            &plugin_data,
            om_config,
            class_map,
            &mut inherited,
            &mut processed,
            &mut plugin_methods_cache,
        );
    }

    // `plugin_method_types` runs only for plugins that are not disabled, as in Magento.
    let unconfirmed_plugin_classes = plugin_methods_cache
        .keys()
        .filter(|plugin_type| {
            class_map
                .get(plugin_type.as_str())
                .is_none_or(|info| matches!(info.kind, ClassKind::Interface | ClassKind::Trait))
        })
        .cloned()
        .collect();

    PluginListMetadata {
        plugin_data,
        inherited,
        processed,
        unconfirmed_plugin_classes,
    }
}

/// Generate final PHP metadata source for one plugin-list file.
pub fn generate_plugin_list_php(
    di_config: &DiConfig,
    class_map: &FxHashMap<String, ClassInfo>,
    class_definitions: &[String],
) -> String {
    let metadata = compile_plugin_list(di_config, class_map, class_definitions, true);
    serialize_plugin_list_php(&metadata)
}

/// Serialize plugin-list metadata into Magento-style PHP output.
pub fn serialize_plugin_list_php(metadata: &PluginListMetadata) -> String {
    let mut out = String::from("<?php return array (\n");

    // Section 0: pluginData
    out.push_str("  0 => \n  array (\n");
    for (type_name, plugins) in &metadata.plugin_data {
        out.push_str(&format!(
            "    '{}' => \n    array (\n",
            escape_php(type_name)
        ));
        serialize_plugin_map(&mut out, plugins, 6);
        out.push_str("    ),\n");
    }
    out.push_str("  ),\n");

    // Section 1: inherited
    out.push_str("  1 => \n  array (\n");
    for (type_name, plugins) in &metadata.inherited {
        match plugins {
            None => {
                out.push_str(&format!("    '{}' => NULL,\n", escape_php(type_name)));
            }
            Some(plugins) => {
                out.push_str(&format!(
                    "    '{}' => \n    array (\n",
                    escape_php(type_name)
                ));
                serialize_plugin_map(&mut out, plugins, 6);
                out.push_str("    ),\n");
            }
        }
    }
    out.push_str("  ),\n");

    // Section 2: processed
    out.push_str("  2 => \n  array (\n");
    for (processed_key, entry) in &metadata.processed {
        out.push_str(&format!(
            "    '{}' => \n    array (\n",
            escape_php(processed_key)
        ));
        for listener in entry.order() {
            match *listener {
                LISTENER_BEFORE => {
                    out.push_str("      1 => \n      array (\n");
                    for (idx, plugin_code) in entry.before.iter().enumerate() {
                        out.push_str(&format!(
                            "        {} => '{}',\n",
                            idx,
                            escape_php(plugin_code)
                        ));
                    }
                    out.push_str("      ),\n");
                }
                LISTENER_AROUND => {
                    if let Some(around) = &entry.around {
                        out.push_str(&format!("      2 => '{}',\n", escape_php(around)));
                    }
                }
                LISTENER_AFTER => {
                    out.push_str("      4 => \n      array (\n");
                    for (idx, plugin_code) in entry.after.iter().enumerate() {
                        out.push_str(&format!(
                            "        {} => '{}',\n",
                            idx,
                            escape_php(plugin_code)
                        ));
                    }
                    out.push_str("      ),\n");
                }
                _ => {}
            }
        }
        out.push_str("    ),\n");
    }
    out.push_str("  ),\n");

    out.push_str(");\n");
    out
}

fn collect_plugin_data(di_config: &DiConfig) -> IndexMap<String, Vec<PluginConfig>> {
    let mut unordered: Vec<&String> = di_config
        .plugins
        .keys()
        .filter(|owner| !di_config.plugin_owners.contains(*owner))
        .collect();
    unordered.sort();

    let mut out = IndexMap::new();
    for owner in di_config.plugin_owners.iter().chain(unordered) {
        let Some(source_plugins) = di_config.plugins.get(owner.as_str()) else {
            continue;
        };
        if source_plugins.is_empty() {
            continue;
        }
        let mut list = Vec::with_capacity(source_plugins.len());
        for plugin in source_plugins {
            list.push(from_di_plugin(plugin));
        }
        out.insert(normalize(owner), list);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn inherit_plugins(
    type_name: &str,
    plugin_data: &IndexMap<String, Vec<PluginConfig>>,
    di_config: &DiConfig,
    class_map: &FxHashMap<String, ClassInfo>,
    inherited: &mut IndexMap<String, Option<Vec<PluginConfig>>>,
    processed: &mut IndexMap<String, ProcessedMethod>,
    plugin_methods_cache: &mut FxHashMap<String, IndexMap<String, u8>>,
) -> Option<Vec<PluginConfig>> {
    let type_name = normalize(type_name);
    if let Some(existing) = inherited.get(&type_name) {
        return existing.clone();
    }

    let real_type = normalize(&di_config.get_instance_type(&type_name));
    let mut plugins: Vec<PluginConfig> = if real_type != type_name {
        inherit_plugins(
            &real_type,
            plugin_data,
            di_config,
            class_map,
            inherited,
            processed,
            plugin_methods_cache,
        )
        .unwrap_or_default()
    } else {
        let mut inherited_from_relations = Vec::new();
        for relation in get_parents(&type_name, class_map) {
            if relation.is_empty() {
                continue;
            }
            if let Some(relation_plugins) = inherit_plugins(
                &relation,
                plugin_data,
                di_config,
                class_map,
                inherited,
                processed,
                plugin_methods_cache,
            ) {
                if !relation_plugins.is_empty() {
                    merge_plugin_lists(&mut inherited_from_relations, &relation_plugins);
                }
            }
        }
        inherited_from_relations
    };

    if let Some(own_plugins) = plugin_data.get(&type_name) {
        if plugins.is_empty() {
            plugins = own_plugins.clone();
        } else {
            merge_plugin_lists(&mut plugins, own_plugins);
        }
    }

    inherited.insert(type_name.clone(), None);
    let mut first_visit_result = None;

    if !plugins.is_empty() {
        let mut sorted_plugins: Vec<PluginConfig> = plugins
            .into_iter()
            .filter(|plugin| !plugin.instance.is_empty())
            .collect();
        // PHP's uasort is stable, so plugins with equal sortOrder keep merge order.
        sorted_plugins.sort_by_key(|plugin| plugin.sort_order);
        for plugin in &mut sorted_plugins {
            plugin.instance = normalize(&plugin.instance);
        }

        inherited.insert(type_name.clone(), Some(sorted_plugins.clone()));

        let mut last_per_method: FxHashMap<String, String> = FxHashMap::default();
        for plugin in &sorted_plugins {
            if plugin.is_disabled() {
                continue;
            }
            let plugin_type = normalize(&di_config.get_instance_type(&plugin.instance));
            let method_types = plugin_method_types(&plugin_type, class_map, plugin_methods_cache);
            for (plugin_method, &listener_flags) in &method_types {
                let current = last_per_method
                    .get(plugin_method)
                    .map(|s| s.as_str())
                    .unwrap_or("__self");
                let current_key = format!("{}_{}_{}", type_name, plugin_method, current);
                let entry = processed.entry(current_key).or_default();

                if listener_flags & LISTENER_AROUND != 0 {
                    entry.set_around(plugin.name.clone());
                    last_per_method.insert(plugin_method.clone(), plugin.name.clone());
                }
                if listener_flags & LISTENER_BEFORE != 0 {
                    entry.push_before(plugin.name.clone());
                }
                if listener_flags & LISTENER_AFTER != 0 {
                    entry.push_after(plugin.name.clone());
                }
            }
        }

        // `inheritPlugins()` unsets disabled plugins from its local list before it returns,
        // while `$inherited` keeps them. So the first visit hands children a list without
        // them, and every later visit a list with them.
        sorted_plugins.retain(|plugin| !plugin.is_disabled());
        first_visit_result = Some(sorted_plugins);
    }

    first_visit_result
}

/// `ObjectManager\\Relations\\Runtime::getParents()`: `ClassReader::getParents()` for a class,
/// nothing for an interface or trait because `class_exists()` is false for those.
fn get_parents(type_name: &str, class_map: &FxHashMap<String, ClassInfo>) -> Vec<String> {
    let Some(info) = class_map.get(type_name) else {
        return Vec::new();
    };
    if matches!(info.kind, ClassKind::Interface | ClassKind::Trait) {
        return Vec::new();
    }

    let interfaces = class_implements(type_name, class_map);
    let parent = info
        .extends
        .as_deref()
        .map(normalize)
        .filter(|parent| !parent.is_empty());
    let Some(parent) = parent else {
        return interfaces;
    };
    let parent_interfaces = class_implements(&parent, class_map);
    let mut relations = vec![parent];
    relations.extend(
        interfaces
            .into_iter()
            .filter(|interface| !contains_name(&parent_interfaces, interface)),
    );
    relations
}

/// PHP's `class_implements()`, in the order of `zend_do_implement_interfaces()`: the parent's
/// interfaces, the declared ones (a class with `__toString` declares `Stringable` last), then
/// for each declared one the interfaces it has, last one first.
fn class_implements(type_name: &str, class_map: &FxHashMap<String, ClassInfo>) -> Vec<String> {
    let Some(info) = class_map.get(type_name) else {
        return builtin_interface_parents(type_name)
            .iter()
            .map(|name| name.to_string())
            .collect();
    };
    let is_interface = matches!(info.kind, ClassKind::Interface);

    let mut interfaces = Vec::new();
    let mut declared: Vec<String> = Vec::new();
    if let Some(extends) = info.extends.as_deref().map(normalize) {
        if is_interface {
            declared.push(extends);
        } else {
            interfaces = class_implements(&extends, class_map);
        }
    }
    declared.extend(info.implements.iter().map(|name| normalize(name)));
    let declares_to_string = info
        .public_methods
        .iter()
        .any(|method| method.name.eq_ignore_ascii_case("__toString"));
    if declares_to_string && !matches!(info.kind, ClassKind::Trait) {
        declared.push("Stringable".to_string());
    }

    for name in &declared {
        if !name.is_empty() && !contains_name(&interfaces, name) {
            interfaces.push(name.clone());
        }
    }
    for name in &declared {
        for extended in class_implements(name, class_map).into_iter().rev() {
            if !contains_name(&interfaces, &extended) {
                interfaces.push(extended);
            }
        }
    }
    interfaces
}

/// `class_implements()` of PHP's own interfaces, which no scan of the store can find.
fn builtin_interface_parents(type_name: &str) -> &'static [&'static str] {
    match type_name {
        "Iterator" | "IteratorAggregate" => &["Traversable"],
        "OuterIterator" | "RecursiveIterator" | "SeekableIterator" => &["Iterator", "Traversable"],
        "Throwable" => &["Stringable"],
        "BackedEnum" => &["UnitEnum"],
        _ => &[],
    }
}

fn contains_name(names: &[String], name: &str) -> bool {
    names.iter().any(|n| n.eq_ignore_ascii_case(name))
}

fn merge_plugin_lists(dst: &mut Vec<PluginConfig>, src: &[PluginConfig]) {
    for src_plugin in src {
        if let Some(existing) = dst.iter_mut().find(|p| p.name == src_plugin.name) {
            existing.replace_with(src_plugin);
        } else {
            dst.push(src_plugin.clone());
        }
    }
}

fn from_di_plugin(plugin: &DiPlugin) -> PluginConfig {
    PluginConfig {
        name: plugin.name.clone(),
        sort_order: plugin.effective_sort_order(),
        instance: normalize(&plugin.type_name),
        disabled: plugin.disabled,
        disabled_after_instance: plugin.disabled_after_type,
    }
}

fn plugin_method_types(
    plugin_type: &str,
    class_map: &FxHashMap<String, ClassInfo>,
    cache: &mut FxHashMap<String, IndexMap<String, u8>>,
) -> IndexMap<String, u8> {
    if let Some(cached) = cache.get(plugin_type) {
        return cached.clone();
    }

    let methods = collect_public_methods_with_inheritance(plugin_type, class_map);
    let mut result: IndexMap<String, u8> = IndexMap::new();
    for method in methods {
        if let Some((intercepted, listener)) = parse_plugin_listener(&method) {
            result
                .entry(intercepted)
                .and_modify(|existing| *existing |= listener)
                .or_insert(listener);
        }
    }

    cache.insert(plugin_type.to_string(), result.clone());
    result
}

fn collect_public_methods_with_inheritance(
    fqcn: &str,
    class_map: &FxHashMap<String, ClassInfo>,
) -> Vec<String> {
    let mut methods = Vec::new();
    let mut seen_methods = FxHashSet::default();
    let mut visited_types = FxHashSet::default();
    let mut current = Some(normalize(fqcn));

    while let Some(type_name) = current {
        if !visited_types.insert(type_name.clone()) {
            break;
        }
        let Some(info) = class_map.get(&type_name) else {
            break;
        };
        for method in &info.public_methods {
            if seen_methods.insert(method.name.clone()) {
                methods.push(method.name.clone());
            }
        }
        current = info.extends.as_ref().map(|parent| normalize(parent));
    }

    methods
}

fn parse_plugin_listener(method_name: &str) -> Option<(String, u8)> {
    if let Some(rest) = method_name.strip_prefix("before") {
        return Some((lcfirst(rest), LISTENER_BEFORE));
    }
    if let Some(rest) = method_name.strip_prefix("around") {
        return Some((lcfirst(rest), LISTENER_AROUND));
    }
    if let Some(rest) = method_name.strip_prefix("after") {
        return Some((lcfirst(rest), LISTENER_AFTER));
    }
    None
}

fn lcfirst(input: &str) -> String {
    let mut chars = input.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = first.to_lowercase().to_string();
    out.push_str(chars.as_str());
    out
}

fn serialize_plugin_map(out: &mut String, plugins: &[PluginConfig], indent: usize) {
    let pad = " ".repeat(indent);
    for plugin in plugins {
        out.push_str(&format!(
            "{}'{}' => \n{}array (\n",
            pad,
            escape_php(&plugin.name),
            pad
        ));
        out.push_str(&format!("{}  'sortOrder' => {},\n", pad, plugin.sort_order));
        let disabled = plugin
            .disabled
            .map(|disabled| format!("{}  'disabled' => {},\n", pad, disabled))
            .unwrap_or_default();
        if !plugin.disabled_after_instance {
            out.push_str(&disabled);
        }
        if !plugin.instance.is_empty() {
            out.push_str(&format!(
                "{}  'instance' => '{}',\n",
                pad,
                escape_php(&plugin.instance)
            ));
        }
        if plugin.disabled_after_instance {
            out.push_str(&disabled);
        }
        out.push_str(&format!("{}),\n", pad));
    }
}

fn normalize(s: &str) -> String {
    s.trim().trim_start_matches('\\').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use di_xml_reader::{apply_module_config_on_primary, merge_configs, DiConfig, Plugin};
    use php_extractor::types::{ClassInfo, ClassKind, MethodSignature};
    use std::path::PathBuf;

    fn method(name: &str) -> MethodSignature {
        MethodSignature {
            name: name.to_string(),
            params: vec![],
            return_type: None,
            is_static: false,
            returns_reference: false,
        }
    }

    fn class_info(
        fqcn: &str,
        extends: Option<&str>,
        implements: &[&str],
        methods: &[&str],
    ) -> ClassInfo {
        let parts: Vec<&str> = fqcn.rsplitn(2, '\\').collect();
        let (name, namespace) = if parts.len() == 2 {
            (parts[0].to_string(), parts[1].to_string())
        } else {
            (fqcn.to_string(), String::new())
        };

        ClassInfo {
            path: PathBuf::from("dummy.php"),
            namespace,
            name,
            fqcn: fqcn.to_string(),
            kind: ClassKind::Class,
            extends: extends.map(|v| v.to_string()),
            implements: implements.iter().map(|v| v.to_string()).collect(),
            constructor: None,
            is_abstract: false,
            is_final: false,
            public_methods: methods.iter().map(|m| method(m)).collect(),
            traits: Default::default(),
        }
    }

    fn plugin(name: &str, instance: &str, sort_order: i32, disabled: bool) -> Plugin {
        Plugin {
            name: name.to_string(),
            type_name: instance.to_string(),
            sort_order: Some(sort_order),
            disabled: Some(disabled),
            disabled_after_type: false,
        }
    }

    #[test]
    fn test_disabled_plugin_stays_in_inherited_but_not_processed() {
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Foo\\Target".to_string(),
            class_info("Foo\\Target", None, &[], &["run"]),
        );
        class_map.insert(
            "Foo\\Plugin".to_string(),
            class_info("Foo\\Plugin", None, &[], &["beforeRun"]),
        );

        let mut di = DiConfig::default();
        di.plugins.insert(
            "Foo\\Target".to_string(),
            vec![plugin("p", "Foo\\Plugin", 10, true)],
        );

        let class_defs: Vec<String> = class_map.keys().cloned().collect();
        let compiled = compile_plugin_list(&di, &class_map, &class_defs, true);
        let inherited = compiled
            .inherited
            .get("Foo\\Target")
            .and_then(|v| v.as_ref())
            .unwrap();
        assert_eq!(inherited.len(), 1);
        assert!(inherited[0].is_disabled());
        assert!(compiled.processed.is_empty());
    }

    #[test]
    fn test_around_chain_current_key_progression() {
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Foo\\Target".to_string(),
            class_info("Foo\\Target", None, &[], &["run"]),
        );
        class_map.insert(
            "Foo\\Around".to_string(),
            class_info("Foo\\Around", None, &[], &["aroundRun"]),
        );
        class_map.insert(
            "Foo\\Before".to_string(),
            class_info("Foo\\Before", None, &[], &["beforeRun"]),
        );

        let mut di = DiConfig::default();
        di.plugins.insert(
            "Foo\\Target".to_string(),
            vec![
                plugin("around_p", "Foo\\Around", 10, false),
                plugin("before_p", "Foo\\Before", 20, false),
            ],
        );

        let class_defs: Vec<String> = class_map.keys().cloned().collect();
        let compiled = compile_plugin_list(&di, &class_map, &class_defs, true);

        let self_key = compiled
            .processed
            .get("Foo\\Target_run___self")
            .expect("missing self key");
        assert_eq!(self_key.around.as_deref(), Some("around_p"));

        let around_key = compiled
            .processed
            .get("Foo\\Target_run_around_p")
            .expect("missing around key");
        assert_eq!(around_key.before, vec!["before_p".to_string()]);
    }

    #[test]
    fn test_parent_plugins_inherit_into_child() {
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Foo\\Parent".to_string(),
            class_info("Foo\\Parent", None, &[], &["run"]),
        );
        class_map.insert(
            "Foo\\Child".to_string(),
            class_info("Foo\\Child", Some("Foo\\Parent"), &[], &["childOnly"]),
        );
        class_map.insert(
            "Foo\\Plugin".to_string(),
            class_info("Foo\\Plugin", None, &[], &["beforeRun"]),
        );

        let mut di = DiConfig::default();
        di.plugins.insert(
            "Foo\\Parent".to_string(),
            vec![plugin("parent_plugin", "Foo\\Plugin", 10, false)],
        );

        let class_defs: Vec<String> = class_map.keys().cloned().collect();
        let compiled = compile_plugin_list(&di, &class_map, &class_defs, true);
        let child_plugins = compiled
            .inherited
            .get("Foo\\Child")
            .and_then(|v| v.as_ref())
            .expect("child should inherit plugins");
        assert_eq!(child_plugins.len(), 1);
        assert_eq!(child_plugins[0].name, "parent_plugin");
        assert!(compiled.processed.contains_key("Foo\\Child_run___self"));
    }

    #[test]
    fn test_php_serialization_sections() {
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Foo\\Target".to_string(),
            class_info("Foo\\Target", None, &[], &["run"]),
        );
        class_map.insert(
            "Foo\\Plugin".to_string(),
            class_info("Foo\\Plugin", None, &[], &["beforeRun"]),
        );

        let mut di = DiConfig::default();
        di.plugins.insert(
            "Foo\\Target".to_string(),
            vec![plugin("p", "Foo\\Plugin", 10, false)],
        );

        let class_defs: Vec<String> = class_map.keys().cloned().collect();
        let out = generate_plugin_list_php(&di, &class_map, &class_defs);
        assert!(out.starts_with("<?php return array (\n"));
        assert!(out.contains("  0 => "));
        assert!(out.contains("  1 => "));
        assert!(out.contains("  2 => "));
        assert!(out.contains("'Foo\\\\Target_run___self'"));
        assert!(out.ends_with(");\n"));
    }

    // Magento reads one scope at a time. Files inside a scope merge attribute by attribute
    // through the DOM, then `PluginListGenerator::merge()` lays each scope over the previous
    // one with array_replace_recursive.
    fn scope_config(files: &[&str]) -> DiConfig {
        merge_configs(
            files
                .iter()
                .map(|xml| {
                    di_xml_reader::parser::parse_di_xml_bytes(
                        format!("<config>{xml}</config>").as_bytes(),
                    )
                    .unwrap()
                })
                .collect(),
        )
    }

    fn scoped_config(scopes: &[&[&str]]) -> DiConfig {
        scopes.iter().fold(DiConfig::default(), |merged, files| {
            apply_module_config_on_primary(merged, scope_config(files))
        })
    }

    fn override_fixture_classes() -> FxHashMap<String, ClassInfo> {
        let mut class_map = FxHashMap::default();
        for (fqcn, extends, methods) in [
            ("Foo\\Parent", None, vec!["run"]),
            ("Foo\\Child", Some("Foo\\Parent"), vec!["run"]),
            ("Foo\\Plugin", None, vec!["beforeRun"]),
        ] {
            class_map.insert(fqcn.to_string(), class_info(fqcn, extends, &[], &methods));
        }
        class_map
    }

    const DECLARE_ON_PARENT: &str =
        r#"<type name="Foo\Parent"><plugin name="x" type="Foo\Plugin" sortOrder="10"/></type>"#;

    fn compile_overrides(scopes: &[&[&str]]) -> PluginListMetadata {
        let class_map = override_fixture_classes();
        let classes: Vec<String> = class_map.keys().cloned().collect();
        compile_plugin_list(&scoped_config(scopes), &class_map, &classes, true)
    }

    fn inherited_plugin<'a>(
        compiled: &'a PluginListMetadata,
        owner: &str,
        name: &str,
    ) -> Option<&'a PluginConfig> {
        compiled
            .inherited
            .get(owner)
            .and_then(|plugins| plugins.as_ref())
            .and_then(|plugins| plugins.iter().find(|plugin| plugin.name == name))
    }

    #[test]
    fn area_disable_without_type_turns_the_plugin_off_in_that_area() {
        let area = r#"<type name="Foo\Parent"><plugin name="x" disabled="true"/></type>"#;
        let compiled = compile_overrides(&[&[DECLARE_ON_PARENT], &[area]]);

        assert!(!compiled.processed.contains_key("Foo\\Parent_run___self"));
        let plugin = inherited_plugin(&compiled, "Foo\\Parent", "x").unwrap();
        assert_eq!(plugin.instance, "Foo\\Plugin");
    }

    #[test]
    fn child_disable_without_type_turns_off_a_plugin_declared_on_the_parent() {
        let child = r#"<type name="Foo\Child"><plugin name="x" disabled="true"/></type>"#;
        let compiled = compile_overrides(&[&[DECLARE_ON_PARENT, child]]);

        assert!(compiled.processed.contains_key("Foo\\Parent_run___self"));
        assert!(!compiled.processed.contains_key("Foo\\Child_run___self"));
        let plugin = inherited_plugin(&compiled, "Foo\\Child", "x").unwrap();
        assert_eq!(plugin.instance, "Foo\\Plugin");
    }

    fn section_keys<V>(section: &IndexMap<String, V>) -> Vec<&str> {
        section.keys().map(String::as_str).collect()
    }

    #[test]
    fn sections_follow_the_visit_order_of_the_scope() {
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Plugin\\A".to_string(),
            class_info(
                "Plugin\\A",
                None,
                &[],
                &["beforeZed", "afterRun", "beforeAlpha"],
            ),
        );
        class_map.insert(
            "Zed\\Child".to_string(),
            class_info("Zed\\Child", Some("Alpha\\Parent"), &[], &[]),
        );
        let mut di = DiConfig::default();
        di.plugin_owners.insert("Zed\\Child".to_string());
        di.plugin_owners.insert("Middle".to_string());
        for owner in ["Zed\\Child", "Middle"] {
            di.plugins
                .insert(owner.to_string(), vec![plugin("p", "Plugin\\A", 0, false)]);
        }

        let metadata =
            compile_scope_plugin_list(&di, &di, &class_map, &[], &["Virtual".to_string()]);

        assert_eq!(
            section_keys(&metadata.plugin_data),
            ["Zed\\Child", "Middle"]
        );
        // A parent gets its place before its child, and a type with no plugins has one too.
        assert_eq!(
            section_keys(&metadata.inherited),
            ["Virtual", "Alpha\\Parent", "Zed\\Child", "Middle"]
        );
        assert_eq!(
            section_keys(&metadata.processed),
            [
                "Zed\\Child_zed___self",
                "Zed\\Child_run___self",
                "Zed\\Child_alpha___self",
                "Middle_zed___self",
                "Middle_run___self",
                "Middle_alpha___self",
            ]
        );
    }

    #[test]
    fn first_visit_of_a_type_hands_children_a_list_without_disabled_plugins() {
        let mut class_map = FxHashMap::default();
        for child in ["Early", "Late"] {
            class_map.insert(
                child.to_string(),
                class_info(child, Some("Parent"), &[], &[]),
            );
        }
        let mut di = DiConfig::default();
        di.plugin_owners.insert("Early".to_string());
        di.plugin_owners.insert("Late".to_string());
        di.plugin_owners.insert("Parent".to_string());
        di.plugins
            .insert("Parent".to_string(), vec![plugin("off", "P", 0, true)]);
        for child in ["Early", "Late"] {
            di.plugins
                .insert(child.to_string(), vec![plugin("own", "P", 0, false)]);
        }

        let metadata = compile_scope_plugin_list(&di, &di, &class_map, &[], &[]);

        let names = |owner: &str| -> Vec<String> {
            metadata.inherited[owner]
                .as_ref()
                .unwrap()
                .iter()
                .map(|p| p.name.clone())
                .collect()
        };
        // Early visits Parent for the first time. Late gets the cached list.
        assert_eq!(names("Early"), ["own"]);
        assert_eq!(names("Late"), ["off", "own"]);
        assert_eq!(names("Parent"), ["off"]);
    }

    #[test]
    fn an_interface_has_no_parents_of_its_own() {
        let mut class_map = FxHashMap::default();
        let mut child = class_info("ChildInterface", Some("ParentInterface"), &[], &[]);
        child.kind = ClassKind::Interface;
        class_map.insert("ChildInterface".to_string(), child);
        let mut parent = class_info("ParentInterface", None, &[], &[]);
        parent.kind = ClassKind::Interface;
        class_map.insert("ParentInterface".to_string(), parent);
        let mut di = DiConfig::default();
        di.plugins.insert(
            "ParentInterface".to_string(),
            vec![plugin("p", "P", 0, false)],
        );
        di.plugin_owners.insert("ChildInterface".to_string());
        di.plugins.insert(
            "ChildInterface".to_string(),
            vec![plugin("c", "P", 0, false)],
        );

        let metadata = compile_scope_plugin_list(&di, &di, &class_map, &[], &[]);

        let child_list = metadata.inherited["ChildInterface"].as_ref().unwrap();
        assert_eq!(child_list.len(), 1);
        assert_eq!(child_list[0].name, "c");
    }

    #[test]
    fn parents_follow_class_implements_order() {
        let mut class_map = FxHashMap::default();
        let interface = |name: &str, extends: Option<&str>| {
            let mut info = class_info(name, extends, &[], &[]);
            info.kind = ClassKind::Interface;
            info
        };
        class_map.insert("Top".to_string(), interface("Top", None));
        class_map.insert("Mid".to_string(), interface("Mid", Some("Top")));
        class_map.insert("Other".to_string(), interface("Other", None));
        class_map.insert("FromBase".to_string(), interface("FromBase", None));
        class_map.insert(
            "Base".to_string(),
            class_info("Base", None, &["FromBase"], &[]),
        );
        class_map.insert(
            "Target".to_string(),
            class_info("Target", Some("Base"), &["Mid", "Other"], &["__toString"]),
        );

        assert_eq!(
            get_parents("Target", &class_map),
            ["Base", "Mid", "Other", "Stringable", "Top"]
        );
        assert_eq!(get_parents("Base", &class_map), ["FromBase"]);
        assert!(get_parents("Mid", &class_map).is_empty());

        // Checked against PHP 8.4: `interface R extends P, F`, `interface P extends A, B`.
        class_map.insert("A".to_string(), interface("A", None));
        class_map.insert("B".to_string(), interface("B", None));
        class_map.insert("F".to_string(), interface("F", None));
        let mut p = interface("P", Some("A"));
        p.implements = vec!["B".to_string()];
        class_map.insert("P".to_string(), p);
        let mut r = interface("R", Some("P"));
        r.implements = vec!["F".to_string()];
        class_map.insert("R".to_string(), r);
        class_map.insert("UsesR".to_string(), class_info("UsesR", None, &["R"], &[]));
        assert_eq!(class_implements("R", &class_map), ["P", "F", "B", "A"]);
        assert_eq!(
            class_implements("UsesR", &class_map),
            ["R", "A", "B", "F", "P"]
        );

        class_map.insert(
            "Items".to_string(),
            class_info("Items", None, &["IteratorAggregate", "Countable"], &[]),
        );
        assert_eq!(
            get_parents("Items", &class_map),
            ["IteratorAggregate", "Countable", "Traversable"]
        );
    }

    #[test]
    fn an_area_virtual_type_is_unknown_to_the_object_manager_config() {
        let mut class_map = FxHashMap::default();
        class_map.insert("Real".to_string(), class_info("Real", None, &[], &[]));
        let global = {
            let mut di = DiConfig::default();
            di.plugins
                .insert("Real".to_string(), vec![plugin("p", "P", 0, false)]);
            di
        };
        let mut area = global.clone();
        area.virtual_types.insert(
            "AreaVirtual".to_string(),
            di_xml_reader::VirtualType {
                name: "AreaVirtual".to_string(),
                type_name: "Real".to_string(),
            },
        );

        let metadata = compile_scope_plugin_list(
            &area,
            &global,
            &class_map,
            &[],
            &["AreaVirtual".to_string()],
        );

        assert_eq!(metadata.inherited["AreaVirtual"], None);
    }

    #[test]
    fn plugins_with_equal_sort_order_keep_merge_order() {
        let mut di_config = DiConfig::default();
        di_config.plugins.insert(
            "Foo".to_string(),
            vec![
                plugin("z", "Plugin\\Z", 10, false),
                plugin("a", "Plugin\\A", 10, false),
                plugin("first", "Plugin\\First", 5, false),
            ],
        );
        let mut class_map = FxHashMap::default();
        for fqcn in ["Plugin\\Z", "Plugin\\A", "Plugin\\First"] {
            class_map.insert(
                fqcn.to_string(),
                class_info(fqcn, None, &[], &["beforeRun"]),
            );
        }

        let metadata = compile_plugin_list(&di_config, &class_map, &[], false);

        let names: Vec<&str> = metadata.inherited["Foo"]
            .as_ref()
            .unwrap()
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, ["first", "z", "a"]);
        assert_eq!(
            metadata.processed["Foo_run___self"].before,
            ["first", "z", "a"]
        );
    }

    #[test]
    fn child_sort_order_override_keeps_the_instance_from_the_parent() {
        let child = r#"<type name="Foo\Child"><plugin name="x" sortOrder="5"/></type>"#;
        let compiled = compile_overrides(&[&[DECLARE_ON_PARENT, child]]);

        assert!(compiled.processed.contains_key("Foo\\Child_run___self"));
        let plugin = inherited_plugin(&compiled, "Foo\\Child", "x").unwrap();
        assert_eq!(plugin.instance, "Foo\\Plugin");
        assert_eq!(plugin.sort_order, 5);
    }

    #[test]
    fn override_in_a_later_scope_without_sort_order_resets_it_to_zero() {
        let area = r#"<type name="Foo\Parent"><plugin name="x" disabled="true"/></type>"#;
        let compiled = compile_overrides(&[&[DECLARE_ON_PARENT], &[area]]);

        let plugin = inherited_plugin(&compiled, "Foo\\Parent", "x").unwrap();
        assert_eq!(plugin.sort_order, 0);
    }

    #[test]
    fn override_in_the_same_scope_without_sort_order_keeps_it() {
        let later = r#"<type name="Foo\Parent"><plugin name="x" disabled="true"/></type>"#;
        let compiled = compile_overrides(&[&[DECLARE_ON_PARENT, later]]);

        let plugin = inherited_plugin(&compiled, "Foo\\Parent", "x").unwrap();
        assert_eq!(plugin.sort_order, 10);
    }

    #[test]
    fn later_explicit_enable_wins_over_an_earlier_disable() {
        let off = r#"<type name="Foo\Parent"><plugin name="x" disabled="true"/></type>"#;
        let on = r#"<type name="Foo\Parent"><plugin name="x" disabled="false"/></type>"#;

        let same_scope = compile_overrides(&[&[DECLARE_ON_PARENT, off, on]]);
        assert!(same_scope.processed.contains_key("Foo\\Parent_run___self"));

        let next_scope = compile_overrides(&[&[DECLARE_ON_PARENT, off], &[on]]);
        assert!(next_scope.processed.contains_key("Foo\\Parent_run___self"));
    }

    #[test]
    fn override_of_a_plugin_nobody_declares_never_reaches_the_inherited_list() {
        let orphan = r#"<type name="Foo\Child"><plugin name="y" disabled="true"/></type>"#;
        let compiled = compile_overrides(&[&[orphan]]);

        assert!(compiled.plugin_data.contains_key("Foo\\Child"));
        assert!(inherited_plugin(&compiled, "Foo\\Child", "y").is_none());
    }

    #[test]
    fn plugin_rows_are_written_with_the_attributes_magento_writes() {
        let compiled = compile_overrides(&[
            &[
                r#"<type name="Foo\Parent"><plugin name="on" type="Foo\Plugin" disabled="false"/></type>"#,
                r#"<type name="Foo\Child"><plugin name="ref" disabled="true"/></type>"#,
                DECLARE_ON_PARENT,
            ],
            &[r#"<type name="Foo\Parent"><plugin name="x" disabled="true"/></type>"#],
        ]);
        let php = serialize_plugin_list_php(&compiled);
        let section_0 = &php[..php.find("  1 => ").unwrap()];

        assert!(section_0.contains(
            "'on' => \n      array (\n        'sortOrder' => 0,\n        'disabled' => false,\n        'instance' => 'Foo\\\\Plugin',\n"
        ));
        assert!(section_0.contains(
            "'ref' => \n      array (\n        'sortOrder' => 0,\n        'disabled' => true,\n      ),\n"
        ));
        assert!(section_0.contains(
            "'x' => \n      array (\n        'sortOrder' => 0,\n        'instance' => 'Foo\\\\Plugin',\n        'disabled' => true,\n"
        ));
    }
}
