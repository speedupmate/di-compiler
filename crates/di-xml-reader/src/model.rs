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
    pub type_name: String,
    pub sort_order: i32,
    pub disabled: bool,
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
}
