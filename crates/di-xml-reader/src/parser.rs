use std::path::Path;

use crate::model::DiConfig;

/// Parse a single di.xml file into a partial DiConfig.
/// Implemented in TKT-010.
pub fn parse_di_xml(_path: &Path) -> Result<DiConfig, crate::Error> {
    Ok(DiConfig::default())
}
