use indexmap::IndexSet;
use rustc_hash::FxHashMap;

/// Merged DI configuration from all di.xml files.
#[derive(Debug, Default, Clone)]
pub struct DiConfig {
    /// preference @for → @type
    pub preferences: FxHashMap<String, String>,
    /// type @name → Vec<Plugin>
    pub plugins: FxHashMap<String, Vec<Plugin>>,
    /// virtualType @name → @type (parent)
    pub virtual_types: FxHashMap<String, VirtualType>,
    /// type @name → TypeConfig
    pub type_configs: FxHashMap<String, TypeConfig>,
    /// Lowercased preference key -> canonical key (for fast case-insensitive lookup)
    pub preference_keys_lc: FxHashMap<String, String>,
    /// Lowercased type config key -> canonical key (for fast case-insensitive lookup)
    pub type_config_keys_lc: FxHashMap<String, String>,
    /// `<type>` and `<virtualType>` names in first-seen order. Until `fold_spellings` runs
    /// they hold the spelling from the file, because `\\X` and `X` are separate nodes.
    pub type_nodes: IndexSet<String>,
    /// Types with plugins, in the order `PluginListGenerator::merge()` first sees them.
    pub plugin_owners: IndexSet<String>,
    /// Virtual types with a `type` attribute in the scope merged last, in file order.
    /// `PluginListGenerator` visits these first when it loads a scope.
    pub scope_virtual_types: Vec<String>,
}

impl DiConfig {
    /// Rebuild case-insensitive lookup indexes.
    pub fn refresh_lookup_indexes(&mut self) {
        self.preference_keys_lc.clear();
        self.preference_keys_lc.reserve(self.preferences.len());
        for key in self.preferences.keys() {
            let lower = key.to_ascii_lowercase();
            self.preference_keys_lc
                .entry(lower)
                .or_insert_with(|| key.clone());
        }

        self.type_config_keys_lc.clear();
        self.type_config_keys_lc.reserve(self.type_configs.len());
        for key in self.type_configs.keys() {
            let lower = key.to_ascii_lowercase();
            self.type_config_keys_lc
                .entry(lower)
                .or_insert_with(|| key.clone());
        }
    }

    /// Insert a preference and update the lowercase index in O(1).
    /// Preserves first-seen canonical casing (same invariant as refresh_lookup_indexes).
    pub fn insert_preference(&mut self, from: String, to: String) {
        let lower = from.to_ascii_lowercase();
        self.preference_keys_lc
            .entry(lower)
            .or_insert_with(|| from.clone());
        self.preferences.insert(from, to);
    }

    /// Insert a type_config entry and update the lowercase index in O(1).
    /// Preserves first-seen canonical casing (same invariant as refresh_lookup_indexes).
    pub fn insert_type_config_key(&mut self, name: &str) {
        let lower = name.to_ascii_lowercase();
        self.type_config_keys_lc
            .entry(lower)
            .or_insert_with(|| name.to_string());
    }
}

#[derive(Debug, Clone)]
pub struct Plugin {
    pub name: String,
    /// Empty when the entry has no `type` attribute: an override of a plugin declared
    /// elsewhere, in an earlier scope or on a parent type.
    pub type_name: String,
    /// `None` when the entry has no `sortOrder` attribute.
    pub sort_order: Option<i32>,
    /// `None` when the entry has no `disabled` attribute.
    pub disabled: Option<bool>,
    /// A later scope added `disabled` to an entry that already had a `type`, so Magento
    /// writes `disabled` after `instance` in this row.
    pub disabled_after_type: bool,
}

impl Plugin {
    pub fn is_disabled(&self) -> bool {
        self.disabled == Some(true)
    }

    /// Disabled entries and entries nobody gave a class never run.
    pub fn is_active(&self) -> bool {
        !self.type_name.is_empty() && !self.is_disabled()
    }

    /// `ObjectManager\Config\Mapper\Dom` writes 0 when `sortOrder` is absent.
    pub fn effective_sort_order(&self) -> i32 {
        self.sort_order.unwrap_or(0)
    }

    /// Merge a later file of the same scope. Magento merges these files as a DOM, so an
    /// absent attribute keeps the earlier value.
    pub fn merge_same_scope(&mut self, later: Plugin) {
        if !later.type_name.is_empty() {
            self.type_name = later.type_name;
        }
        if later.sort_order.is_some() {
            self.sort_order = later.sort_order;
        }
        if later.disabled.is_some() {
            self.disabled = later.disabled;
        }
    }

    /// Merge a later scope. `PluginListGenerator::merge()` uses array_replace_recursive on
    /// mapped rows, and those always hold `sortOrder`, so an absent one resets to 0.
    pub fn merge_later_scope(&mut self, later: Plugin) {
        if !self.type_name.is_empty() && self.disabled.is_none() && later.disabled.is_some() {
            self.disabled_after_type = true;
        }
        let sort_order = later.effective_sort_order();
        self.merge_same_scope(later);
        self.sort_order = Some(sort_order);
    }
}

#[derive(Debug, Clone)]
pub struct VirtualType {
    pub name: String,
    pub type_name: String,
}

#[derive(Debug, Clone, Default)]
pub struct TypeConfig {
    pub shared: Option<bool>,
    pub arguments: Vec<Argument>,
}

#[derive(Debug, Clone)]
pub struct Preference {
    pub for_type: String,
    pub to_type: String,
}

/// Argument value variants matching Magento's `xsi:type` attribute.
#[derive(Debug, Clone)]
pub enum Argument {
    Object {
        name: String,
        value: String,
        shared: Option<bool>,
        /// sortOrder attribute from di.xml (for sorting within parent Array)
        sort_order: i32,
    },
    String {
        name: String,
        value: String,
        sort_order: i32,
    },
    Boolean {
        name: String,
        value: bool,
        sort_order: i32,
    },
    Number {
        name: String,
        value: String,
        sort_order: i32,
    },
    Null {
        name: String,
        sort_order: i32,
    },
    Array {
        name: String,
        items: Vec<Argument>,
        sort_order: i32,
    },
    Init {
        name: String,
        value: String,
        sort_order: i32,
    },
    Const {
        name: String,
        value: String,
        sort_order: i32,
    },
    /// A node `Interpreter\\Composite` stops on: no `xsi:type`, one it does not know, or a number or
    /// boolean with a value its interpreter rejects. Another file of the scope can still give the
    /// type or a new value, so this only counts after the merge.
    Untyped {
        name: String,
        /// Empty when the node has no `xsi:type`.
        xsi_type: String,
        value: String,
        items: Vec<Argument>,
        sort_order: i32,
    },
}

impl Argument {
    /// True for this node or any item below it.
    pub fn is_unreadable(&self) -> bool {
        match self {
            Argument::Untyped { .. } => true,
            Argument::Object { value, .. } => value.is_empty(),
            Argument::Array { items, .. } => items.iter().any(Argument::is_unreadable),
            _ => false,
        }
    }
}

impl DiConfig {
    pub fn has_unreadable_argument(&self) -> bool {
        self.type_configs
            .values()
            .any(|config| config.arguments.iter().any(Argument::is_unreadable))
    }

    /// What `Mapper\\Dom::convert()` stops with for this config, as one merged scope.
    pub fn unreadable_argument_failure(&self) -> Option<String> {
        let mut failures: Vec<String> = Vec::new();
        for (type_name, config) in &self.type_configs {
            for argument in &config.arguments {
                collect_unreadable(argument, "", type_name, &mut failures);
            }
        }
        failures.sort();
        failures.into_iter().next()
    }
}

fn collect_unreadable(argument: &Argument, parent: &str, type_name: &str, out: &mut Vec<String>) {
    let mut fail = |name: &str, reason: String| {
        out.push(format!(
            "{reason} Type {type_name}, argument {parent}{name}"
        ));
    };
    match argument {
        Argument::Untyped { name, xsi_type, .. } => {
            let reason = match xsi_type.as_str() {
                "" => "Value for key \"xsi:type\" is missing in the argument data.".to_string(),
                "number" => "Numeric value is expected.".to_string(),
                "boolean" => "Boolean value is expected.".to_string(),
                other => format!("Argument interpreter named '{other}' has not been defined."),
            };
            fail(name, reason);
        }
        Argument::Object { name, value, .. } if value.is_empty() => {
            fail(name, "Object argument has no class name.".to_string());
        }
        Argument::Array { name, items, .. } => {
            let path = format!("{parent}{name}/");
            for item in items {
                collect_unreadable(item, &path, type_name, out);
            }
        }
        _ => {}
    }
}
