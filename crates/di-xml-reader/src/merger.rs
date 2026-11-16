//! TKT-011: Merge multiple DiConfig instances in Magento load order.

use indexmap::IndexSet;

use crate::model::{Argument, DiConfig, Plugin, TypeConfig};

/// Merge a list of partial DiConfig instances into one.
///
/// The configs belong to one scope and are in load order. Later configs override earlier
/// ones per Magento merge rules. Spellings of a type merge separately, as DOM nodes, and
/// fold together at the end.
pub fn merge_configs(configs: Vec<DiConfig>) -> DiConfig {
    let mut result = DiConfig::default();
    for config in configs {
        merge_into_impl(&mut result, config);
    }
    result.plugin_owners = result
        .type_nodes
        .iter()
        .filter(|node| result.plugins.contains_key(*node))
        .map(|node| normalize(node).to_string())
        .collect();
    result.fold_spellings();
    result.scope_virtual_types = result
        .type_nodes
        .iter()
        .filter(|node| result.virtual_types.contains_key(*node))
        .cloned()
        .collect();
    result
}

impl DiConfig {
    /// Fold `\\X` into `X`, in the order the nodes were first seen.
    ///
    /// `Config\Reader\Filesystem` merges the files as a DOM, where `\\X` and `X` are separate
    /// nodes. Then `ObjectManager\Config\Config::_mergeConfiguration()` and
    /// `PluginListGenerator::merge()` walk the mapped nodes and trim the name. The later node
    /// replaces plugin rows and whole arguments by name.
    pub fn fold_spellings(&mut self) {
        if !self.type_nodes.iter().any(|n| normalize(n) != *n) {
            return;
        }
        let mut folded = IndexSet::with_capacity(self.type_nodes.len());
        for spelled in std::mem::take(&mut self.type_nodes) {
            let name = normalize(&spelled).to_string();
            if spelled != name {
                let plain_is_first = folded.contains(&name);
                if let Some(plugins) = self.plugins.remove(&spelled) {
                    let plugins = match self.plugins.remove(&name) {
                        Some(plain) if plain_is_first => later_plugin_rows(plain, plugins),
                        Some(plain) => later_plugin_rows(plugins, plain),
                        None => plugins,
                    };
                    self.plugins.insert(name.clone(), plugins);
                }
                if let Some(config) = self.type_configs.remove(&spelled) {
                    let config = match self.type_configs.remove(&name) {
                        Some(plain) if plain_is_first => later_type_config(plain, config),
                        Some(plain) => later_type_config(config, plain),
                        None => config,
                    };
                    self.type_configs.insert(name.clone(), config);
                }
            }
            folded.insert(name);
        }
        self.type_nodes = folded;
        self.refresh_lookup_indexes();
    }
}

fn normalize(name: &str) -> &str {
    name.trim_start_matches('\\')
}

fn later_plugin_rows(mut first: Vec<Plugin>, later: Vec<Plugin>) -> Vec<Plugin> {
    merge_later_plugins(&mut first, later);
    first
}

fn later_type_config(mut first: TypeConfig, later: TypeConfig) -> TypeConfig {
    apply_type_config_shallow(&mut first, later);
    first
}

fn merge_later_plugins(dst: &mut Vec<Plugin>, later: Vec<Plugin>) {
    for plugin in later {
        match dst.iter_mut().find(|p| p.name == plugin.name) {
            Some(existing) => existing.merge_later_scope(plugin),
            None => dst.push(plugin),
        }
    }
}

pub fn merge_into(dst: &mut DiConfig, src: DiConfig) {
    merge_into_impl(dst, src);
}

fn merge_into_impl(dst: &mut DiConfig, src: DiConfig) {
    dst.type_nodes.extend(src.type_nodes);

    // Preferences: last wins; incremental index update avoids full O(n) rebuild.
    for (k, v) in src.preferences {
        dst.insert_preference(k, v);
    }

    // Virtual types: merge by name.
    // If a later declaration omits `type`, keep the previously resolved base
    // type and only let accompanying <arguments> overrides merge via TypeConfig.
    for (k, v) in src.virtual_types {
        if let Some(existing) = dst.virtual_types.get_mut(&k) {
            if !v.type_name.trim().is_empty() {
                existing.type_name = v.type_name;
            }
        } else {
            dst.virtual_types.insert(k, v);
        }
    }

    // Plugins: same scope, so merged attribute by attribute.
    for (owner, src_plugins) in src.plugins {
        let dst_plugins = dst.plugins.entry(owner).or_default();
        for src_plugin in src_plugins {
            match dst_plugins.iter_mut().find(|p| p.name == src_plugin.name) {
                Some(existing) => existing.merge_same_scope(src_plugin),
                None => dst_plugins.push(src_plugin),
            }
        }
    }

    // TypeConfigs: merge arguments by name, shared overrides; update index for new keys.
    for (type_name, src_tc) in src.type_configs {
        dst.insert_type_config_key(&type_name);
        let dst_tc = dst.type_configs.entry(type_name).or_default();
        merge_type_config(dst_tc, src_tc);
    }
}

/// Apply a merged module-level config on top of a primary (`app/etc/di.xml`) config.
///
/// Preferences, virtual types, and plugins follow the same merge rules as `merge_into`.
/// For type configs, argument values are replaced **at the argument-name level** (shallow),
/// replicating PHP's `Config::_mergeConfiguration` which uses `array_replace` at the
/// arguments level — not a recursive item-level deep merge.
///
/// This is the correct two-phase merge:
///   Phase 1: deep-merge all module di.xml files together (use `merge_configs`)
///   Phase 2: apply the merged module result onto `app/etc/di.xml` (use this fn)
pub fn apply_module_config_on_primary(mut primary: DiConfig, module: DiConfig) -> DiConfig {
    primary.type_nodes.extend(module.type_nodes);
    primary.plugin_owners.extend(module.plugin_owners);
    primary.scope_virtual_types = module.scope_virtual_types;

    // Preferences: last wins; incremental index update avoids full O(n) rebuild.
    for (k, v) in module.preferences {
        primary.insert_preference(k, v);
    }

    // Virtual types: same logic as merge_into
    for (k, v) in module.virtual_types {
        if let Some(existing) = primary.virtual_types.get_mut(&k) {
            if !v.type_name.trim().is_empty() {
                existing.type_name = v.type_name;
            }
        } else {
            primary.virtual_types.insert(k, v);
        }
    }

    // Plugins: a later scope over an earlier one.
    for (owner, src_plugins) in module.plugins {
        merge_later_plugins(primary.plugins.entry(owner).or_default(), src_plugins);
    }

    // TypeConfigs: SHALLOW replacement — module arg replaces primary arg by name.
    // PHP's array_replace($existing_args, $module_args) replaces the whole value
    // for matching argument names rather than merging array items recursively.
    for (type_name, src_tc) in module.type_configs {
        primary.insert_type_config_key(&type_name);
        let dst_tc = primary.type_configs.entry(type_name).or_default();
        apply_type_config_shallow(dst_tc, src_tc);
    }

    primary
}

fn apply_type_config_shallow(dst: &mut TypeConfig, src: TypeConfig) {
    if src.shared.is_some() {
        dst.shared = src.shared;
    }
    // Replace whole argument value by name (not item-level deep merge)
    for src_arg in src.arguments {
        let name = arg_name(&src_arg).to_string();
        if let Some(pos) = dst.arguments.iter().position(|a| arg_name(a) == name) {
            dst.arguments[pos] = src_arg; // REPLACE whole value
        } else {
            dst.arguments.push(src_arg); // NEW arg: append
        }
    }
}

fn merge_type_config(dst: &mut TypeConfig, src: TypeConfig) {
    // shared: last wins
    if src.shared.is_some() {
        dst.shared = src.shared;
    }
    // arguments: merge by name; Array arguments recursively merge items by key
    for src_arg in src.arguments {
        let name = arg_name(&src_arg).to_string();
        if let Some(existing) = dst.arguments.iter_mut().find(|a| arg_name(a) == name) {
            merge_argument(existing, src_arg);
        } else {
            dst.arguments.push(src_arg);
        }
    }
}

/// Merge `src_arg` into `dst_arg`.
/// For Array arguments both sides are merged recursively by item name.
/// All other types: last-wins (src replaces dst).
fn merge_argument(dst: &mut Argument, mut src: Argument) {
    // `Config\Dom::_mergeNode()` merges node by node, so an attribute of an earlier file stays.
    // A node with another `xsi:type` replaces the earlier node, with its attributes. A node
    // without one is the same node.
    if (std::mem::discriminant(&*dst) == std::mem::discriminant(&src)
        || has_no_type(dst)
        || has_no_type(&src))
        && *sort_order_of(&mut src) == 0
    {
        *sort_order_of(&mut src) = *sort_order_of(dst);
    }
    let sort_order = *sort_order_of(&mut src);
    *sort_order_of(dst) = sort_order;
    if has_no_type(&src) {
        if let Argument::Untyped { value, items, .. } = src {
            take_node_without_type(dst, value, items);
        }
        return;
    }
    if has_no_type(dst) {
        give_type_to_earlier_node(dst, src);
        return;
    }
    match (dst, src) {
        (
            Argument::Array {
                items: dst_items, ..
            },
            Argument::Array {
                items: src_items, ..
            },
        ) => merge_items(dst_items, src_items),
        (dst, mut src) => {
            // `Config\Dom` merges node by node, so an attribute of an earlier file stays.
            if let (
                Argument::Object {
                    shared: earlier,
                    value: earlier_value,
                    ..
                },
                Argument::Object {
                    shared: later,
                    value: later_value,
                    ..
                },
            ) = (&*dst, &mut src)
            {
                *later = later.or(*earlier);
                if later_value.is_empty() {
                    later_value.clone_from(earlier_value);
                }
            }
            *dst = src;
        }
    }
}

fn has_no_type(arg: &Argument) -> bool {
    matches!(arg, Argument::Untyped { xsi_type, .. } if xsi_type.is_empty())
}

fn merge_items(dst_items: &mut Vec<Argument>, src_items: Vec<Argument>) {
    for src_item in src_items {
        let name = arg_name(&src_item).to_string();
        if let Some(existing) = dst_items.iter_mut().find(|a| arg_name(a) == name) {
            merge_argument(existing, src_item);
        } else {
            dst_items.push(src_item);
        }
    }
}

/// A later node without `xsi:type`: the earlier node keeps its type and takes the new content.
fn take_node_without_type(dst: &mut Argument, later_value: String, later_items: Vec<Argument>) {
    match dst {
        Argument::Array { items, .. } => merge_items(items, later_items),
        Argument::Untyped { value, items, .. } => {
            if !later_value.is_empty() {
                *value = later_value;
            }
            merge_items(items, later_items);
        }
        _ if later_value.is_empty() => {}
        Argument::Object { value, .. } => *value = later_value.trim().to_string(),
        Argument::String { value, .. }
        | Argument::Number { value, .. }
        | Argument::Init { value, .. }
        | Argument::Const { value, .. } => *value = later_value,
        Argument::Boolean { value, .. } => *value = later_value == "true" || later_value == "1",
        Argument::Null { .. } => {}
    }
}

/// An earlier node without `xsi:type` takes the type of the later node. Its items stay.
fn give_type_to_earlier_node(dst: &mut Argument, mut src: Argument) {
    if let (Argument::Untyped { items, .. }, Argument::Array { items: later, .. }) =
        (&mut *dst, &mut src)
    {
        let mut merged = std::mem::take(items);
        merge_items(&mut merged, std::mem::take(later));
        *later = merged;
    }
    *dst = src;
}

fn sort_order_of(arg: &mut Argument) -> &mut i32 {
    match arg {
        Argument::Object { sort_order, .. }
        | Argument::String { sort_order, .. }
        | Argument::Boolean { sort_order, .. }
        | Argument::Number { sort_order, .. }
        | Argument::Null { sort_order, .. }
        | Argument::Array { sort_order, .. }
        | Argument::Init { sort_order, .. }
        | Argument::Const { sort_order, .. }
        | Argument::Untyped { sort_order, .. } => sort_order,
    }
}

fn arg_name(arg: &Argument) -> &str {
    match arg {
        Argument::Object { name, .. } => name,
        Argument::String { name, .. } => name,
        Argument::Boolean { name, .. } => name,
        Argument::Number { name, .. } => name,
        Argument::Null { name, .. } => name,
        Argument::Array { name, .. } => name,
        Argument::Init { name, .. } => name,
        Argument::Const { name, .. } => name,
        Argument::Untyped { name, .. } => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Argument, DiConfig, Plugin, TypeConfig, VirtualType};
    use crate::parser::parse_di_xml_bytes_unfolded;

    fn unfolded(xml: &str) -> DiConfig {
        parse_di_xml_bytes_unfolded(format!("<config>{xml}</config>").as_bytes()).unwrap()
    }

    fn plugin_names(config: &DiConfig, owner: &str) -> Vec<String> {
        config.plugins[owner]
            .iter()
            .map(|p| p.name.clone())
            .collect()
    }

    fn array_items(config: &DiConfig, owner: &str) -> Vec<String> {
        match &config.type_configs[owner].arguments[0] {
            Argument::Array { items, .. } => {
                items.iter().map(|a| arg_name(a).to_string()).collect()
            }
            other => panic!("expected an array, got {other:?}"),
        }
    }

    #[test]
    fn test_argument_without_type_adds_its_items_to_the_array_of_an_earlier_file() {
        let merged = merge_configs(vec![
            unfolded(
                r#"<type name="Grid"><arguments><argument name="columns" xsi:type="array">
                       <item name="first" xsi:type="string">a</item>
                   </argument></arguments></type>"#,
            ),
            unfolded(
                r#"<type name="Grid"><arguments><argument name="columns">
                       <item name="second" xsi:type="string">b</item>
                   </argument></arguments></type>"#,
            ),
        ]);
        assert_eq!(array_items(&merged, "Grid"), ["first", "second"]);
    }

    #[test]
    fn test_plugin_owners_follow_the_first_node_with_plugins() {
        let merged = merge_configs(vec![
            unfolded(
                r#"<type name="Zed" shared="false"/>
                   <type name="Middle"><plugin name="a" type="A"/></type>
                   <type name="\\Alpha" shared="false"/>"#,
            ),
            unfolded(
                r#"<type name="Zed"><plugin name="b" type="B"/></type>
                   <type name="Alpha"><plugin name="c" type="C"/></type>"#,
            ),
        ]);
        let owners: Vec<&str> = merged.plugin_owners.iter().map(String::as_str).collect();
        assert_eq!(owners, ["Zed", "Middle", "Alpha"]);
    }

    #[test]
    fn test_scope_virtual_types_keep_file_order_and_skip_a_missing_type() {
        let area = merge_configs(vec![unfolded(
            r#"<virtualType name="Zed" type="A"/><type name="Plain"/><virtualType name="Alpha" type="B"/>"#,
        )]);
        assert_eq!(area.scope_virtual_types, ["Zed", "Alpha"]);

        let mut base = merge_configs(vec![unfolded(r#"<virtualType name="Global" type="A"/>"#)]);
        base = apply_module_config_on_primary(base, area);
        assert_eq!(base.scope_virtual_types, ["Zed", "Alpha"]);
    }

    #[test]
    fn test_type_spellings_merge_per_spelling_in_first_seen_order() {
        let merged = merge_configs(vec![
            unfolded(r#"<type name="Foo\Bar"><plugin name="a" type="A"/></type>"#),
            unfolded(r#"<type name="\Foo\Bar"><plugin name="b" type="B"/></type>"#),
            unfolded(r#"<type name="Foo\Bar"><plugin name="c" type="C"/></type>"#),
        ]);
        assert_eq!(plugin_names(&merged, "Foo\\Bar"), ["a", "c", "b"]);
        assert!(!merged.plugins.contains_key("\\Foo\\Bar"));
    }

    #[test]
    fn test_type_spelling_seen_first_on_a_node_without_plugins_comes_first() {
        let merged = merge_configs(vec![
            unfolded(r#"<type name="\Foo\Bar" shared="false"/>"#),
            unfolded(r#"<type name="Foo\Bar"><plugin name="a" type="A"/></type>"#),
            unfolded(r#"<type name="\Foo\Bar"><plugin name="b" type="B"/></type>"#),
        ]);
        assert_eq!(plugin_names(&merged, "Foo\\Bar"), ["b", "a"]);
    }

    #[test]
    fn test_type_spellings_in_one_file_stay_separate() {
        let merged = merge_configs(vec![
            unfolded(
                r#"<type name="\Foo\Bar"><plugin name="a" type="A"/></type>
                   <type name="Foo\Bar"><plugin name="b" type="B"/></type>"#,
            ),
            unfolded(r#"<type name="\Foo\Bar"><plugin name="c" type="C"/></type>"#),
        ]);
        assert_eq!(plugin_names(&merged, "Foo\\Bar"), ["a", "c", "b"]);
    }

    #[test]
    fn test_same_plugin_in_two_spellings_is_a_later_row_that_resets_sort_order() {
        let merged = merge_configs(vec![
            unfolded(r#"<type name="Foo\Bar"><plugin name="a" type="A" sortOrder="5"/></type>"#),
            unfolded(r#"<type name="\Foo\Bar"><plugin name="a" disabled="true"/></type>"#),
        ]);
        let plugin = &merged.plugins["Foo\\Bar"][0];
        assert_eq!(plugin.sort_order, Some(0));
        assert_eq!(plugin.type_name, "A");
        assert!(plugin.is_disabled());
    }

    #[test]
    fn test_argument_in_another_spelling_replaces_the_whole_value() {
        let array = |item: &str| {
            format!(
                r#"<arguments><argument name="list" xsi:type="array"><item name="{item}" xsi:type="string">v</item></argument></arguments>"#
            )
        };
        let merged = merge_configs(vec![
            unfolded(&format!(r#"<type name="Foo\Bar">{}</type>"#, array("a"))),
            unfolded(&format!(r#"<type name="Foo\Bar">{}</type>"#, array("b"))),
            unfolded(&format!(r#"<type name="\Foo\Bar">{}</type>"#, array("c"))),
        ]);
        assert_eq!(array_items(&merged, "Foo\\Bar"), ["c"]);
    }

    #[test]
    fn test_preference_last_wins() {
        let mut c1 = DiConfig::default();
        c1.preferences.insert("Iface".into(), "Impl1".into());

        let mut c2 = DiConfig::default();
        c2.preferences.insert("Iface".into(), "Impl2".into());

        let merged = merge_configs(vec![c1, c2]);
        assert_eq!(
            merged.preferences.get("Iface").map(|s| s.as_str()),
            Some("Impl2")
        );
    }

    #[test]
    fn test_plugins_merged_by_name() {
        let mut c1 = DiConfig::default();
        c1.plugins.insert(
            "Foo".into(),
            vec![
                Plugin {
                    name: "p1".into(),
                    type_name: "T1".into(),
                    sort_order: Some(10),
                    disabled: Some(false),
                    disabled_after_type: false,
                },
                Plugin {
                    name: "p2".into(),
                    type_name: "T2".into(),
                    sort_order: Some(20),
                    disabled: Some(false),
                    disabled_after_type: false,
                },
            ],
        );

        let mut c2 = DiConfig::default();
        c2.plugins.insert(
            "Foo".into(),
            vec![
                Plugin {
                    name: "p1".into(),
                    type_name: "T1Updated".into(),
                    sort_order: Some(5),
                    disabled: Some(false),
                    disabled_after_type: false,
                },
                Plugin {
                    name: "p3".into(),
                    type_name: "T3".into(),
                    sort_order: Some(30),
                    disabled: Some(false),
                    disabled_after_type: false,
                },
            ],
        );

        let merged = merge_configs(vec![c1, c2]);
        let plugins = merged.plugins.get("Foo").unwrap();
        assert_eq!(plugins.len(), 3);
        let p1 = plugins.iter().find(|p| p.name == "p1").unwrap();
        assert_eq!(p1.type_name, "T1Updated"); // overridden
        assert_eq!(p1.sort_order, Some(5));
    }

    #[test]
    fn test_virtual_type_override_without_type_keeps_previous_base_type() {
        let mut c1 = DiConfig::default();
        c1.virtual_types.insert(
            "AssetPreProcessorPool".into(),
            VirtualType {
                name: "AssetPreProcessorPool".into(),
                type_name: "Magento\\Framework\\View\\Asset\\PreProcessor\\Pool".into(),
            },
        );
        c1.type_configs.insert(
            "AssetPreProcessorPool".into(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Array {
                    name: "preprocessors".into(),
                    items: vec![Argument::Array {
                        name: "js".into(),
                        items: vec![],
                        sort_order: 0,
                    }],
                    sort_order: 0,
                }],
            },
        );

        // Later config overrides only arguments and omits `type` attribute.
        let mut c2 = DiConfig::default();
        c2.virtual_types.insert(
            "AssetPreProcessorPool".into(),
            VirtualType {
                name: "AssetPreProcessorPool".into(),
                type_name: String::new(),
            },
        );
        c2.type_configs.insert(
            "AssetPreProcessorPool".into(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Array {
                    name: "preprocessors".into(),
                    items: vec![Argument::Array {
                        name: "json".into(),
                        items: vec![],
                        sort_order: 0,
                    }],
                    sort_order: 0,
                }],
            },
        );

        let merged = merge_configs(vec![c1, c2]);
        let vt = merged.virtual_types.get("AssetPreProcessorPool").unwrap();
        assert_eq!(
            vt.type_name,
            "Magento\\Framework\\View\\Asset\\PreProcessor\\Pool"
        );
    }

    #[test]
    fn test_virtual_type_override_with_non_empty_type_replaces_previous_base_type() {
        let mut c1 = DiConfig::default();
        c1.virtual_types.insert(
            "FooVirtual".into(),
            VirtualType {
                name: "FooVirtual".into(),
                type_name: "BaseA".into(),
            },
        );

        let mut c2 = DiConfig::default();
        c2.virtual_types.insert(
            "FooVirtual".into(),
            VirtualType {
                name: "FooVirtual".into(),
                type_name: "BaseB".into(),
            },
        );

        let merged = merge_configs(vec![c1, c2]);
        let vt = merged.virtual_types.get("FooVirtual").unwrap();
        assert_eq!(vt.type_name, "BaseB");
    }

    #[test]
    fn test_apply_module_config_on_primary_replaces_argument_value_shallow() {
        let mut primary = DiConfig::default();
        primary.type_configs.insert(
            "Magento\\Framework\\EntityManager\\OperationPool".into(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Array {
                    name: "operations".into(),
                    items: vec![Argument::Array {
                        name: "default".into(),
                        items: vec![Argument::String {
                            name: "create".into(),
                            value: "Create".into(),
                            sort_order: 0,
                        }],
                        sort_order: 0,
                    }],
                    sort_order: 0,
                }],
            },
        );

        let mut module = DiConfig::default();
        module.type_configs.insert(
            "Magento\\Framework\\EntityManager\\OperationPool".into(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Array {
                    name: "operations".into(),
                    items: vec![Argument::Array {
                        name: "Magento\\AsynchronousOperations\\Api\\Data\\OperationListInterface"
                            .into(),
                        items: vec![Argument::String {
                            name: "create".into(),
                            value: "CreateByTopic".into(),
                            sort_order: 0,
                        }],
                        sort_order: 0,
                    }],
                    sort_order: 0,
                }],
            },
        );

        let merged = apply_module_config_on_primary(primary, module);
        let tc = merged
            .type_configs
            .get("Magento\\Framework\\EntityManager\\OperationPool")
            .expect("operation pool config");
        let operations = tc
            .arguments
            .iter()
            .find(|a| matches!(a, Argument::Array { name, .. } if name == "operations"))
            .expect("operations argument");
        let names: Vec<&str> = match operations {
            Argument::Array { items, .. } => items
                .iter()
                .map(|item| match item {
                    Argument::Array { name, .. } => name.as_str(),
                    other => panic!("expected nested array item, got {other:?}"),
                })
                .collect(),
            other => panic!("expected array argument, got {other:?}"),
        };
        assert_eq!(
            names,
            vec!["Magento\\AsynchronousOperations\\Api\\Data\\OperationListInterface"]
        );
    }

    #[test]
    fn test_apply_module_config_on_primary_keeps_non_overridden_arguments() {
        let mut primary = DiConfig::default();
        primary.type_configs.insert(
            "Foo\\Type".into(),
            TypeConfig {
                shared: None,
                arguments: vec![
                    Argument::String {
                        name: "first".into(),
                        value: "one".into(),
                        sort_order: 0,
                    },
                    Argument::String {
                        name: "second".into(),
                        value: "two".into(),
                        sort_order: 0,
                    },
                ],
            },
        );

        let mut module = DiConfig::default();
        module.type_configs.insert(
            "Foo\\Type".into(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::String {
                    name: "second".into(),
                    value: "override".into(),
                    sort_order: 0,
                }],
            },
        );

        let merged = apply_module_config_on_primary(primary, module);
        let tc = merged.type_configs.get("Foo\\Type").expect("foo config");

        let first = tc
            .arguments
            .iter()
            .find_map(|arg| match arg {
                Argument::String { name, value, .. } if name == "first" => Some(value),
                _ => None,
            })
            .expect("first argument");
        let second = tc
            .arguments
            .iter()
            .find_map(|arg| match arg {
                Argument::String { name, value, .. } if name == "second" => Some(value),
                _ => None,
            })
            .expect("second argument");

        assert_eq!(first, "one");
        assert_eq!(second, "override");
    }

    #[test]
    fn test_plugin_disable_is_sticky_across_later_redeclarations() {
        let mut c1 = DiConfig::default();
        c1.plugins.insert(
            "Foo\\Bar".into(),
            vec![Plugin {
                name: "p".into(),
                type_name: "Foo\\Plugin".into(),
                sort_order: Some(10),
                disabled: Some(false),
                disabled_after_type: false,
            }],
        );

        // Disabled-only override (no type) flips plugin off.
        let mut c2 = DiConfig::default();
        c2.plugins.insert(
            "Foo\\Bar".into(),
            vec![Plugin {
                name: "p".into(),
                type_name: String::new(),
                sort_order: Some(0),
                disabled: Some(true),
                disabled_after_type: false,
            }],
        );

        // A later redeclaration with no disabled attribute keeps the earlier value.
        let mut c3 = DiConfig::default();
        c3.plugins.insert(
            "Foo\\Bar".into(),
            vec![Plugin {
                name: "p".into(),
                type_name: "Foo\\Plugin\\Override".into(),
                sort_order: Some(20),
                disabled: None,
                disabled_after_type: false,
            }],
        );

        let merged = merge_configs(vec![c1, c2, c3]);
        let plugin = merged
            .plugins
            .get("Foo\\Bar")
            .and_then(|v| v.iter().find(|p| p.name == "p"))
            .expect("merged plugin");
        assert_eq!(plugin.type_name, "Foo\\Plugin\\Override");
        assert!(plugin.is_disabled());
    }

    // =========================================================================
    // C1 regression: merge_configs must include ALL input configs
    //
    // The original incremental-cache bug in the CLI dropped unchanged di.xml
    // files before they reached merge_configs, producing an incomplete DiConfig.
    // This test ensures that merge_configs always includes every passed config,
    // so that if anyone re-introduces a pre-merge filter they get a failing test.
    // =========================================================================

    #[test]
    fn merge_configs_includes_preferences_from_all_inputs() {
        let mut c1 = DiConfig::default();
        c1.preferences.insert("Iface\\A".into(), "Impl\\A".into());

        let mut c2 = DiConfig::default();
        c2.preferences.insert("Iface\\B".into(), "Impl\\B".into());

        let mut c3 = DiConfig::default();
        c3.preferences.insert("Iface\\C".into(), "Impl\\C".into());

        let merged = merge_configs(vec![c1, c2, c3]);

        assert_eq!(
            merged.preferences.get("Iface\\A").map(|s| s.as_str()),
            Some("Impl\\A"),
            "c1 preference must be present"
        );
        assert_eq!(
            merged.preferences.get("Iface\\B").map(|s| s.as_str()),
            Some("Impl\\B"),
            "c2 preference must be present"
        );
        assert_eq!(
            merged.preferences.get("Iface\\C").map(|s| s.as_str()),
            Some("Impl\\C"),
            "c3 preference must be present"
        );
    }

    #[test]
    fn merge_configs_includes_plugins_from_all_inputs() {
        let make_plugin = |name: &str, type_name: &str| Plugin {
            name: name.to_string(),
            type_name: type_name.to_string(),
            sort_order: Some(0),
            disabled: Some(false),
            disabled_after_type: false,
        };

        let mut c1 = DiConfig::default();
        c1.plugins
            .insert("Owner\\A".into(), vec![make_plugin("p1", "Type\\P1")]);

        let mut c2 = DiConfig::default();
        c2.plugins
            .insert("Owner\\B".into(), vec![make_plugin("p2", "Type\\P2")]);

        let mut c3 = DiConfig::default();
        c3.plugins
            .insert("Owner\\C".into(), vec![make_plugin("p3", "Type\\P3")]);

        let merged = merge_configs(vec![c1, c2, c3]);

        assert!(
            merged.plugins.contains_key("Owner\\A"),
            "c1 plugins must be present"
        );
        assert!(
            merged.plugins.contains_key("Owner\\B"),
            "c2 plugins must be present"
        );
        assert!(
            merged.plugins.contains_key("Owner\\C"),
            "c3 plugins must be present"
        );
    }

    #[test]
    fn merge_configs_empty_input_produces_empty_config() {
        let merged = merge_configs(vec![]);
        assert!(merged.preferences.is_empty());
        assert!(merged.plugins.is_empty());
        assert!(merged.virtual_types.is_empty());
    }
}
