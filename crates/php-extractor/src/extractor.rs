use std::path::Path;

use crate::lexer::{Lexer, ScanFacts};
use crate::tier2::extract_tier2_with_facts;
use crate::tier3::extract_tier3;
use crate::types::{ExtractResult, LexError};

/// Try Tier 1 → Tier 2 → Tier 3 extraction.
pub fn extract_file(path: &Path) -> ExtractResult {
    extract_file_with_class_check(path).0
}

/// Like [`extract_file`], and also says what the tier that read the file saw around the class
/// declaration.
pub fn extract_file_with_class_check(path: &Path) -> (ExtractResult, ScanFacts) {
    let result = match Lexer::extract_with_facts(path) {
        Ok((info, facts)) => return (ExtractResult::Ok(info), facts),
        Err(LexError::Io(e)) => ExtractResult::PhpFallbackFailed(format!("IO: {e}")),
        Err(LexError::Unsupported(ref reason)) if reason == "no_class" => ExtractResult::NoClass,
        Err(LexError::Unsupported(ref reason)) if reason == "enum" => ExtractResult::NoClass,
        Err(LexError::Unsupported(ref reason)) if reason == "anonymous_class" => {
            ExtractResult::NoClass
        }
        Err(e @ LexError::Unsupported(_)) | Err(e @ LexError::UnexpectedEof) => {
            log::debug!(
                "Tier1 failed for {}: {e} — trying tree-sitter",
                path.display()
            );
            match extract_tier2_with_facts(path) {
                (ExtractResult::Ok(info), facts) => return (ExtractResult::Ok(info), facts),
                (ExtractResult::NoClass, _) => ExtractResult::NoClass,
                (failed, _) => {
                    log::debug!(
                        "Tier2 failed for {}: {failed:?} — trying PHP",
                        path.display()
                    );
                    let (result, facts) = extract_tier3(path);
                    if let ExtractResult::Ok(info) = &result {
                        log::info!("Read {} with PHP (Tier 3): {}", path.display(), info.fqcn);
                    }
                    return (result, facts);
                }
            }
        }
    };
    (result, ScanFacts::default())
}
