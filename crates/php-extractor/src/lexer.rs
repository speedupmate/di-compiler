use std::path::Path;

use crate::types::{ClassInfo, LexError};

/// Tier 1: custom state-machine lexer.
/// Implemented in TKT-003, TKT-004, TKT-005.
pub struct Lexer;

impl Lexer {
    pub fn extract(_path: &Path) -> Result<ClassInfo, LexError> {
        Err(LexError::Unsupported("lexer not yet implemented".into()))
    }
}
