# 09: DI Config Model

- Category: Configuration
- Status: Planned
- Implementation Phase: 02-di-xml-reader
- Owner: Unassigned
- Feature ID: `di-config-model`
- Suggested Dependencies: 08-di-xml-merger

## Intent

The merged `DiConfig` struct with query methods. Pure logic — no I/O.
Replicates the query behavior of PHP's `ObjectManager\Config\Config.php`.

## Core State and Actions

```rust
pub struct DiConfig {
    pub preferences: HashMap<String, String>,
    pub plugins: HashMap<String, Vec<Plugin>>,
    pub virtual_types: HashMap<String, VirtualType>,
    pub type_configs: HashMap<String, TypeConfig>,
}

impl DiConfig {
    /// Follow preference chain, detect cycles
    pub fn get_preference(&self, type_fqcn: &str) -> &str

    /// Resolve virtual type to its concrete instance_of (follow chains)
    pub fn get_instance_type(&self, virtual_name: &str) -> &str

    /// Get merged arguments for a type (own + parent class + virtualType)
    pub fn get_arguments(&self, type_fqcn: &str) -> Vec<&Argument>

    /// Is this type a singleton (shared)?
    pub fn is_shared(&self, type_fqcn: &str) -> bool

    /// Get plugins for a type (sorted by sort_order, disabled excluded)
    pub fn get_active_plugins(&self, type_fqcn: &str) -> Vec<&Plugin>
}
```

## Acceptance Criteria

- `get_preference()` returns the concrete class after following chains
- `get_instance_type()` resolves virtualType → concrete (detect infinite loops)
- `get_arguments()` merges parent class arguments with child overrides
- `get_active_plugins()` returns plugins sorted by sort_order, disabled=true excluded
