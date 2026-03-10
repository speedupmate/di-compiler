pub mod config;
pub mod merger;
pub mod model;
pub mod parser;

pub use config::{find_all_di_xml_files, find_di_xml_files, find_di_xml_files_for_area};
pub use merger::{apply_module_config_on_primary, merge_configs, merge_into};
pub use model::{Argument, DiConfig, Plugin, Preference, TypeConfig, VirtualType};
pub use parser::{parse_di_xml, parse_di_xml_impl};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("XML parse error: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid di.xml: {0}")]
    Invalid(String),
}
