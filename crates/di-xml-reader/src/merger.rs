use crate::model::DiConfig;

/// Merge multiple partial DiConfig instances in Magento load order.
/// Implemented in TKT-011.
pub fn merge_configs(configs: Vec<DiConfig>) -> DiConfig {
    configs.into_iter().fold(DiConfig::default(), |mut acc, c| {
        acc.preferences.extend(c.preferences);
        for (k, v) in c.plugins {
            acc.plugins.entry(k).or_default().extend(v);
        }
        acc.virtual_types.extend(c.virtual_types);
        for (k, v) in c.type_configs {
            acc.type_configs.insert(k, v);
        }
        acc
    })
}
