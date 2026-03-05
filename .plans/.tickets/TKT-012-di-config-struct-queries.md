---
id: TKT-012
title: DiConfig struct + query methods
phase: 02-di-xml-reader
feature: di-config-model
owner: Unassigned
status: Ready
estimate: M
depends_on: [TKT-011]
touches:
  - rust/di-compiler/crates/di-xml-reader/src/config.rs
acceptance:
  - get_preference() follows chain correctly, detects cycles
  - get_instance_type() resolves virtualType chains
  - get_arguments() merges parent + own arguments
  - get_active_plugins() returns sorted, non-disabled plugins
---

# TKT-012: DiConfig Struct + Query Methods

## Scope

The in-memory DI config with query methods. Replicates PHP `ObjectManager/Config/Config.php` behavior.

## Implementation Notes

```rust
impl DiConfig {
    pub fn get_preference(&self, fqcn: &str) -> &str {
        // Follow chain with cycle detection (visited set)
        // Return last concrete type found
    }

    pub fn get_instance_type(&self, name: &str) -> &str {
        // Follow virtualType.instance_of chain
        // Return concrete type (may itself be a preference target)
    }

    pub fn get_arguments(&self, fqcn: &str) -> Vec<&Argument> {
        // Return di.xml <argument> overrides for this type
        // Note: parent class argument inheritance is handled in ArgumentsResolver (TKT-015)
    }

    pub fn is_shared(&self, fqcn: &str) -> bool {
        self.type_configs.get(fqcn)
            .and_then(|c| c.shared)
            .unwrap_or(true)  // default: shared (singleton)
    }

    pub fn get_active_plugins(&self, fqcn: &str) -> Vec<&Plugin> {
        // Get plugins, filter disabled, sort by sort_order
    }
}
```

## Risks

- Circular preference chains must be detected (PHP throws exception, we should error)
