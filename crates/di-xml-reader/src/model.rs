use std::collections::HashMap;

/// Merged DI configuration from all di.xml files.
#[derive(Debug, Default, Clone)]
pub struct DiConfig {
    /// preference @for → @type
    pub preferences: HashMap<String, String>,
    /// type @name → Vec<Plugin>
    pub plugins: HashMap<String, Vec<Plugin>>,
    /// virtualType @name → @type (parent)
    pub virtual_types: HashMap<String, VirtualType>,
    /// type @name → TypeConfig
    pub type_configs: HashMap<String, TypeConfig>,
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
    },
    String {
        name: String,
        value: String,
    },
    Boolean {
        name: String,
        value: bool,
    },
    Number {
        name: String,
        value: String,
    },
    Null {
        name: String,
    },
    Array {
        name: String,
        items: Vec<Argument>,
    },
    Init {
        name: String,
        value: String,
    },
    Const {
        name: String,
        value: String,
    },
}
