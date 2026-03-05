use std::path::Path;

use crate::lexer::Lexer;
use crate::tier2::extract_tier2;
use crate::tier3::extract_tier3;
use crate::types::{ExtractResult, LexError};

/// Try Tier 1 → Tier 2 → Tier 3 extraction.
pub fn extract_file(path: &Path) -> ExtractResult {
    match Lexer::extract(path) {
        Ok(info) => ExtractResult::Ok(info),
        Err(LexError::Io(e)) => ExtractResult::PhpFallbackFailed(format!("IO: {e}")),
        Err(e @ LexError::Unsupported(_)) | Err(e @ LexError::UnexpectedEof) => {
            log::debug!("Tier1 failed for {}: {e} — trying tree-sitter", path.display());
            match extract_tier2(path) {
                ExtractResult::Ok(info) => ExtractResult::Ok(info),
                ExtractResult::NoClass => ExtractResult::NoClass,
                _ => {
                    log::debug!(
                        "Tier2 failed for {} — trying PHP shell fallback",
                        path.display()
                    );
                    extract_tier3(path)
                }
            }
        }
    }
}
