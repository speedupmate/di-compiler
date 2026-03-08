pub mod app_action_list;
pub mod area_config;
pub mod extension;
pub mod factory;
pub mod interceptor;
pub mod metadata;
pub mod plugin_list;
pub mod proxy;
pub mod proxy_deferred;
pub mod repository;
pub mod search_results;
pub mod writer;

pub use app_action_list::{generate_app_action_list_php, serialize_app_action_list_php};
pub use area_config::{generate_area_config, generate_area_config_with_extra_preferences, AREAS};
pub use extension::{
    extension_path, generate_extension, generate_extension_interface, ExtensionAttributeSpec,
    ExtensionSpec,
};
pub use factory::{factory_path, generate_factory};
pub use interceptor::{generate_interceptor, interceptor_path};
pub use metadata::{serialize_arguments_php, serialize_interception_php};
pub use plugin_list::{compile_plugin_list, generate_plugin_list_php, serialize_plugin_list_php};
pub use proxy::{generate_proxy, proxy_path};
pub use proxy_deferred::{generate_proxy_deferred, proxy_deferred_path};
pub use search_results::{generate_search_results, search_results_path};
pub use writer::write_if_changed;

/// Convert a `default_value` string from the PHP extractor into valid PHP syntax
/// for use in generated code (interceptors, proxies, etc.).
///
/// The PHP reflection worker emits `__json__:<json>` for array defaults.
/// This function converts those back to PHP array literal syntax.
/// All other values are returned as-is (they are already valid PHP literals).
pub fn render_php_default(default_value: &str) -> std::borrow::Cow<str> {
    if let Some(json_str) = default_value.strip_prefix("__json__:") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
            return std::borrow::Cow::Owned(json_value_to_php_literal(&v));
        }
    }
    std::borrow::Cow::Borrowed(default_value)
}

fn json_value_to_php_literal(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_value_to_php_literal).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    format!("'{}' => {}", k.replace('\\', "\\\\").replace('\'', "\\'"), json_value_to_php_literal(v))
                })
                .collect();
            format!("[{}]", items.join(", "))
        }
    }
}
