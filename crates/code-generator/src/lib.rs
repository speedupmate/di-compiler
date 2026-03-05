pub mod interceptor;
pub mod factory;
pub mod proxy;
pub mod repository;
pub mod area_config;
pub mod metadata;
pub mod writer;

pub use interceptor::{generate_interceptor, interceptor_path};
pub use factory::{generate_factory, factory_path};
pub use proxy::{generate_proxy, proxy_path};
pub use metadata::{serialize_arguments_php, serialize_interception_php};
pub use area_config::{generate_area_config, AREAS};
pub use writer::write_if_changed;
