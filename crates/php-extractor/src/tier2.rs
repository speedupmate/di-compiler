use std::path::Path;

use crate::types::ExtractResult;

/// Tier 2: tree-sitter-php fallback.
/// Implemented in TKT-006.
pub fn extract_tier2(_path: &Path) -> ExtractResult {
    ExtractResult::ParseFailure("tier2 not yet implemented".into())
}
