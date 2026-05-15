//! TKT-011: Merge multiple DiConfig instances in Magento load order.

use crate::model::{Argument, DiConfig, TypeConfig};

/// Merge a list of partial DiConfig instances into one.
///
/// Load order is assumed to be pre-sorted by caller (vendor/magento → vendor/* → app/etc → app/code).
/// Later configs override earlier ones per Magento merge rules.
pub fn merge_configs(configs: Vec<DiConfig>) -> DiConfig {
    let mut result = DiConfig::default();
    for config in configs {
        merge_into_impl(&mut result, config);
    }
    result
}

pub fn merge_into(dst: &mut DiConfig, src: DiConfig) {
    merge_into_impl(dst, src);
}

fn merge_into_impl(dst: &mut DiConfig, src: DiConfig) {
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

    // Plugins: merge by owner type, then by plugin name (last wins per name)
    for (owner, src_plugins) in src.plugins {
        let dst_plugins = dst.plugins.entry(owner).or_default();
        for src_plugin in src_plugins {
            if let Some(existing) = dst_plugins.iter_mut().find(|p| p.name == src_plugin.name) {
                if src_plugin.type_name.is_empty() {
                    // Disabled-only override (no type attribute): preserve original type_name.
                    existing.disabled = src_plugin.disabled;
                    if src_plugin.sort_order != 0 {
                        existing.sort_order = src_plugin.sort_order;
                    }
                } else {
                    let was_disabled = existing.disabled;
                    *existing = src_plugin;
                    // PHP "sticky disabled": once disabled by an explicit override, the plugin
                    // stays disabled even if a later module re-declares it as active.
                    if was_disabled {
                        existing.disabled = true;
                    }
                }
            } else if !src_plugin.type_name.is_empty() {
                // Only add a new plugin if it has a type; disabled-only entries with no prior
                // declaration are no-ops.
                dst_plugins.push(src_plugin);
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

    // Plugins: same logic as merge_into
    for (owner, src_plugins) in module.plugins {
        let dst_plugins = primary.plugins.entry(owner).or_default();
        for src_plugin in src_plugins {
            if let Some(existing) = dst_plugins.iter_mut().find(|p| p.name == src_plugin.name) {
                if src_plugin.type_name.is_empty() {
                    existing.disabled = src_plugin.disabled;
                    if src_plugin.sort_order != 0 {
                        existing.sort_order = src_plugin.sort_order;
                    }
                } else {
                    let was_disabled = existing.disabled;
                    *existing = src_plugin;
                    if was_disabled {
                        existing.disabled = true;
                    }
                }
            } else if !src_plugin.type_name.is_empty() {
                dst_plugins.push(src_plugin);
            }
        }
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
fn merge_argument(dst: &mut Argument, src: Argument) {
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
                let name = arg_name(&src_item).to_string();
                if let Some(existing) = dst_items.iter_mut().find(|a| arg_name(a) == name) {
                    merge_argument(existing, src_item);
                } else {
                    dst_items.push(src_item);
                }
            }
        }
        (dst, src) => *dst = src,
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Argument, DiConfig, Plugin, TypeConfig, VirtualType};

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
                    sort_order: 10,
                    disabled: false,
                },
                Plugin {
                    name: "p2".into(),
                    type_name: "T2".into(),
                    sort_order: 20,
                    disabled: false,
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
                    sort_order: 5,
                    disabled: false,
                },
                Plugin {
                    name: "p3".into(),
                    type_name: "T3".into(),
                    sort_order: 30,
                    disabled: false,
                },
            ],
        );

        let merged = merge_configs(vec![c1, c2]);
        let plugins = merged.plugins.get("Foo").unwrap();
        assert_eq!(plugins.len(), 3);
        let p1 = plugins.iter().find(|p| p.name == "p1").unwrap();
        assert_eq!(p1.type_name, "T1Updated"); // overridden
        assert_eq!(p1.sort_order, 5);
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
                sort_order: 10,
                disabled: false,
            }],
        );

        // Disabled-only override (no type) flips plugin off.
        let mut c2 = DiConfig::default();
        c2.plugins.insert(
            "Foo\\Bar".into(),
            vec![Plugin {
                name: "p".into(),
                type_name: String::new(),
                sort_order: 0,
                disabled: true,
            }],
        );

        // Later redeclaration should not re-enable the plugin.
        let mut c3 = DiConfig::default();
        c3.plugins.insert(
            "Foo\\Bar".into(),
            vec![Plugin {
                name: "p".into(),
                type_name: "Foo\\Plugin\\Override".into(),
                sort_order: 20,
                disabled: false,
            }],
        );

        let merged = merge_configs(vec![c1, c2, c3]);
        let plugin = merged
            .plugins
            .get("Foo\\Bar")
            .and_then(|v| v.iter().find(|p| p.name == "p"))
            .expect("merged plugin");
        assert_eq!(plugin.type_name, "Foo\\Plugin\\Override");
        assert!(plugin.disabled);
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
            sort_order: 0,
            disabled: false,
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
