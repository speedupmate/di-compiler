//! TKT-011: Merge multiple DiConfig instances in Magento load order.

use crate::model::{Argument, DiConfig, TypeConfig};

/// Merge a list of partial DiConfig instances into one.
///
/// Load order is assumed to be pre-sorted by caller (vendor/magento → vendor/* → app/etc → app/code).
/// Later configs override earlier ones per Magento merge rules.
pub fn merge_configs(configs: Vec<DiConfig>) -> DiConfig {
    let mut result = DiConfig::default();
    for config in configs {
        merge_into(&mut result, config);
    }
    result
}

pub fn merge_into(dst: &mut DiConfig, src: DiConfig) {
    // Preferences: last wins
    for (k, v) in src.preferences {
        dst.preferences.insert(k, v);
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
                *existing = src_plugin;
            } else {
                dst_plugins.push(src_plugin);
            }
        }
    }

    // TypeConfigs: merge arguments by name, shared overrides
    for (type_name, src_tc) in src.type_configs {
        let dst_tc = dst.type_configs.entry(type_name).or_default();
        merge_type_config(dst_tc, src_tc);
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
        (Argument::Array { items: dst_items, .. }, Argument::Array { items: src_items, .. }) => {
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
        Argument::Null { name } => name,
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
                    }],
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
                    }],
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
}
