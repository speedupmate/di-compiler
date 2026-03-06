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

    // Virtual types: last wins (later config overrides type_name)
    for (k, v) in src.virtual_types {
        dst.virtual_types.insert(k, v);
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
    // arguments: merge by name (last wins)
    for src_arg in src.arguments {
        let name = arg_name(&src_arg);
        if let Some(existing) = dst.arguments.iter_mut().find(|a| arg_name(a) == name) {
            *existing = src_arg;
        } else {
            dst.arguments.push(src_arg);
        }
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
    use crate::model::{DiConfig, Plugin};

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
}
