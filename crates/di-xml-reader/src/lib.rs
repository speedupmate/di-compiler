pub mod model;
pub mod parser;
pub mod merger;

pub use model::{DiConfig, Plugin, TypeConfig, Argument, VirtualType, Preference};
pub use parser::parse_di_xml;
pub use merger::merge_configs;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("XML parse error: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid di.xml: {0}")]
    Invalid(String),
}
