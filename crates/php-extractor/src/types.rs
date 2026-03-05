use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassInfo {
    pub path: PathBuf,
    pub namespace: String,
    pub name: String,
    /// Fully qualified class name, e.g. `Magento\Framework\App\Action`
    pub fqcn: String,
    pub kind: ClassKind,
    pub extends: Option<String>,
    pub implements: Vec<String>,
    pub constructor: Option<Constructor>,
    pub is_abstract: bool,
    pub is_final: bool,
    /// Public non-final methods (needed for InterceptorSpec)
    pub public_methods: Vec<MethodSignature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassKind {
    Class,
    AbstractClass,
    Interface,
    Trait,
    Enum,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constructor {
    pub params: Vec<ConstructorParam>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstructorParam {
    pub name: String,
    pub type_hint: Option<String>,
    pub is_optional: bool,
    pub is_primitive: bool,
    pub is_variadic: bool,
    /// Constructor promotion: visibility keyword present
    pub is_promoted: bool,
}

/// A public non-final method signature needed for interceptor codegen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodSignature {
    pub name: String,
    pub params: Vec<MethodParam>,
    pub return_type: Option<String>,
    pub is_static: bool,
    pub returns_reference: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodParam {
    pub name: String,
    pub type_hint: Option<String>,
    pub has_default: bool,
    pub is_variadic: bool,
    pub is_by_ref: bool,
}

#[derive(Debug)]
pub enum ExtractResult {
    /// Successfully extracted class info
    Ok(ClassInfo),
    /// File contains no class declaration (interfaces in wrong tier, scripts, etc.)
    NoClass,
    /// Tier 1 lexer failed — escalate to Tier 2
    LexError(LexError),
    /// Tier 2 tree-sitter failed — escalate to Tier 3
    ParseFailure(String),
    /// Tier 3 PHP shell also failed
    PhpFallbackFailed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum LexError {
    #[error("unsupported syntax: {0}")]
    Unsupported(String),
    #[error("unexpected EOF")]
    UnexpectedEof,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
