pub mod constants;
pub mod default_literal;
pub mod extractor;
pub mod lexer;
pub mod tier2;
pub mod tier3;
pub mod traits;
pub mod types;
pub mod walker;

pub use constants::extract_string_constants;
pub use default_literal::is_plain_literal_default;
pub use extractor::{extract_file, extract_file_with_class_check};
pub use traits::apply_traits;
pub use types::{
    ClassInfo, ClassKind, Constructor, ConstructorParam, ExtractResult, MethodSignature,
};
pub use walker::{read_module_paths, walk_generated_php_files, walk_php_files};
