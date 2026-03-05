use std::path::Path;

use crate::types::ExtractResult;

/// Tier 3: PHP shell fallback.
/// Implemented in TKT-007.
pub fn extract_tier3(_path: &Path) -> ExtractResult {
    ExtractResult::PhpFallbackFailed("tier3 not yet implemented".into())
}
