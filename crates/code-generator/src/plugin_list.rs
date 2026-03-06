//! TKT-032: plugin-list metadata generation.
//!
//! Generates Magento-style compiled plugin list metadata files:
//! `primary|global|...|plugin-list.php`

use std::collections::{BTreeMap, HashMap, HashSet};

use di_xml_reader::{DiConfig, Plugin as DiPlugin};
use php_extractor::ClassInfo;

use crate::metadata::escape_php;

const LISTENER_BEFORE: u8 = 1;
const LISTENER_AROUND: u8 = 2;
const LISTENER_AFTER: u8 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginConfig {
    pub name: String,
    pub sort_order: i32,
    pub instance: String,
    pub disabled: bool,
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
    pub plugin_data: BTreeMap<String, Vec<PluginConfig>>,
    pub inherited: BTreeMap<String, Option<Vec<PluginConfig>>>,
    pub processed: BTreeMap<String, ProcessedMethod>,
}

/// Build the in-memory plugin list structures:
/// section 0 => pluginData, section 1 => inherited, section 2 => processed.
pub fn compile_plugin_list(
    di_config: &DiConfig,
    class_map: &HashMap<String, ClassInfo>,
    class_definitions: &[String],
) -> PluginListMetadata {
    let plugin_data = collect_plugin_data(di_config);
    let mut inherited: HashMap<String, Option<Vec<PluginConfig>>> = HashMap::new();
    let mut processed: HashMap<String, ProcessedMethod> = HashMap::new();
    let mut plugin_methods_cache: HashMap<String, HashMap<String, u8>> = HashMap::new();

    // 1) Virtual types from merged scope config.
    let mut virtual_types: Vec<&String> = di_config.virtual_types.keys().collect();
    virtual_types.sort();
    for vt in virtual_types {
        inherit_plugins(
            vt,
            &plugin_data,
            di_config,
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
            di_config,
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
        inherit_plugins(
            &class_name,
            &plugin_data,
            di_config,
            class_map,
            &mut inherited,
            &mut processed,
            &mut plugin_methods_cache,
        );
    }

    let mut inherited_sorted = BTreeMap::new();
    for (k, v) in inherited {
        inherited_sorted.insert(k, v);
    }

    let mut processed_sorted = BTreeMap::new();
    for (k, v) in processed {
        processed_sorted.insert(k, v);
    }

    PluginListMetadata {
        plugin_data,
        inherited: inherited_sorted,
        processed: processed_sorted,
    }
}

/// Generate final PHP metadata source for one plugin-list file.
pub fn generate_plugin_list_php(
    di_config: &DiConfig,
    class_map: &HashMap<String, ClassInfo>,
    class_definitions: &[String],
) -> String {
    let metadata = compile_plugin_list(di_config, class_map, class_definitions);
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

fn collect_plugin_data(di_config: &DiConfig) -> BTreeMap<String, Vec<PluginConfig>> {
    let mut out = BTreeMap::new();
    let mut owners: Vec<&String> = di_config.plugins.keys().collect();
    owners.sort();
    for owner in owners {
        let owner_name = normalize(owner);
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
        out.insert(owner_name, list);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn inherit_plugins(
    type_name: &str,
    plugin_data: &BTreeMap<String, Vec<PluginConfig>>,
    di_config: &DiConfig,
    class_map: &HashMap<String, ClassInfo>,
    inherited: &mut HashMap<String, Option<Vec<PluginConfig>>>,
    processed: &mut HashMap<String, ProcessedMethod>,
    plugin_methods_cache: &mut HashMap<String, HashMap<String, u8>>,
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

    if !plugins.is_empty() {
        let mut sorted_plugins: Vec<PluginConfig> = plugins
            .into_iter()
            .filter(|plugin| !plugin.instance.is_empty())
            .collect();
        sorted_plugins.sort_by(|a, b| a.sort_order.cmp(&b.sort_order));
        for plugin in &mut sorted_plugins {
            plugin.instance = normalize(&plugin.instance);
        }

        inherited.insert(type_name.clone(), Some(sorted_plugins.clone()));

        let mut last_per_method: HashMap<String, String> = HashMap::new();
        for plugin in &sorted_plugins {
            if plugin.disabled {
                continue;
            }
            let plugin_type = normalize(&di_config.get_instance_type(&plugin.instance));
            let method_types = plugin_method_types(&plugin_type, class_map, plugin_methods_cache);
            if method_types.is_empty() {
                continue;
            }

            let mut method_names: Vec<&String> = method_types.keys().collect();
            method_names.sort();
            for plugin_method in method_names {
                let listener_flags = method_types[plugin_method];
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
    }

    inherited.get(&type_name).cloned().unwrap_or(None)
}

fn get_parents(type_name: &str, class_map: &HashMap<String, ClassInfo>) -> Vec<String> {
    let Some(info) = class_map.get(type_name) else {
        return Vec::new();
    };

    let mut relations = Vec::new();
    if let Some(parent) = &info.extends {
        let parent = normalize(parent);
        if !parent.is_empty() {
            relations.push(parent);
        }
    }
    for interface in &info.implements {
        let interface = normalize(interface);
        if !interface.is_empty() {
            relations.push(interface);
        }
    }
    relations
}

fn merge_plugin_lists(dst: &mut Vec<PluginConfig>, src: &[PluginConfig]) {
    for src_plugin in src {
        if let Some(existing) = dst.iter_mut().find(|p| p.name == src_plugin.name) {
            *existing = src_plugin.clone();
        } else {
            dst.push(src_plugin.clone());
        }
    }
}

fn from_di_plugin(plugin: &DiPlugin) -> PluginConfig {
    PluginConfig {
        name: plugin.name.clone(),
        sort_order: plugin.sort_order,
        instance: normalize(&plugin.type_name),
        disabled: plugin.disabled,
    }
}

fn plugin_method_types(
    plugin_type: &str,
    class_map: &HashMap<String, ClassInfo>,
    cache: &mut HashMap<String, HashMap<String, u8>>,
) -> HashMap<String, u8> {
    if let Some(cached) = cache.get(plugin_type) {
        return cached.clone();
    }

    let methods = collect_public_methods_with_inheritance(plugin_type, class_map);
    let mut result: HashMap<String, u8> = HashMap::new();
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
    class_map: &HashMap<String, ClassInfo>,
) -> Vec<String> {
    let mut methods = Vec::new();
    let mut seen_methods = HashSet::new();
    let mut visited_types = HashSet::new();
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
        out.push_str(&format!(
            "{}  'instance' => '{}',\n",
            pad,
            escape_php(&plugin.instance)
        ));
        if plugin.disabled {
            out.push_str(&format!("{}  'disabled' => true,\n", pad));
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
    use di_xml_reader::{DiConfig, Plugin};
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
        }
    }

    fn plugin(name: &str, instance: &str, sort_order: i32, disabled: bool) -> Plugin {
        Plugin {
            name: name.to_string(),
            type_name: instance.to_string(),
            sort_order,
            disabled,
        }
    }

    #[test]
    fn test_disabled_plugin_stays_in_inherited_but_not_processed() {
        let mut class_map = HashMap::new();
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
        let compiled = compile_plugin_list(&di, &class_map, &class_defs);
        let inherited = compiled
            .inherited
            .get("Foo\\Target")
            .and_then(|v| v.as_ref())
            .unwrap();
        assert_eq!(inherited.len(), 1);
        assert!(inherited[0].disabled);
        assert!(compiled.processed.is_empty());
    }

    #[test]
    fn test_around_chain_current_key_progression() {
        let mut class_map = HashMap::new();
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
        let compiled = compile_plugin_list(&di, &class_map, &class_defs);

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
        let mut class_map = HashMap::new();
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
        let compiled = compile_plugin_list(&di, &class_map, &class_defs);
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
        let mut class_map = HashMap::new();
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
}
