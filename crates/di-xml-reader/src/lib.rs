pub mod model;
pub mod parser;
pub mod merger;
pub mod config;

pub use model::{DiConfig, Plugin, TypeConfig, Argument, VirtualType, Preference};
pub use parser::{parse_di_xml, parse_di_xml_impl};
pub use merger::{merge_configs, merge_into};
pub use config::{find_di_xml_files, find_di_xml_files_for_area};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("XML parse error: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid di.xml: {0}")]
    Invalid(String),
}
