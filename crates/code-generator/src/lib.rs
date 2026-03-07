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
