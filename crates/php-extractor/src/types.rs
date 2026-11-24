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
    /// Trait use. `traits::apply_traits` turns it into members of this class.
    pub traits: Box<TraitData>,
}

/// What PHP's trait rules need. A class only has its real members once every trait is known.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraitData {
    /// Traits named in `use` statements in the class body.
    pub uses: Vec<String>,
    pub rules: Vec<TraitRule>,
    /// Filled for a trait only: all its methods, because `as public` can change what counts.
    pub methods: Vec<TraitMethod>,
    /// Methods of this class that `public_methods` leaves out: not public, or final.
    pub hidden_methods: Vec<String>,
}

/// One line of the `{ ... }` block of a trait `use` statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraitRule {
    /// `A::run insteadof B, C;`
    InsteadOf {
        method: String,
        excluded: Vec<String>,
    },
    /// `A::run as protected walk;`, where the trait, the visibility and the new name are optional.
    Alias {
        trait_name: Option<String>,
        method: String,
        alias: Option<String>,
        is_public: Option<bool>,
        is_final: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraitMethod {
    pub signature: MethodSignature,
    pub is_public: bool,
    pub is_final: bool,
    pub is_abstract: bool,
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

/// Whether each parameter is optional for `ReflectionParameter::isOptional()`.
///
/// PHP treats a parameter with a default as required when a required parameter follows it,
/// `Type $param = null` included, and its default is then not available either.
pub fn optional_in_php(params: &[ConstructorParam]) -> Vec<bool> {
    let mut flags = vec![false; params.len()];
    let mut rest_is_optional = true;
    for (index, param) in params.iter().enumerate().rev() {
        let optional = param.is_variadic || param.is_optional;
        flags[index] = rest_is_optional && param.is_optional;
        rest_is_optional &= optional;
    }
    flags
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstructorParam {
    pub name: String,
    pub type_hint: Option<String>,
    pub is_optional: bool,
    pub default_value: Option<String>,
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
    pub default_value: Option<String>,
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

#[cfg(test)]
mod optional_in_php_tests {
    use super::*;

    fn param(is_optional: bool, is_variadic: bool) -> ConstructorParam {
        ConstructorParam {
            name: "p".to_string(),
            type_hint: None,
            is_optional,
            default_value: is_optional.then(|| "null".to_string()),
            is_primitive: false,
            is_variadic,
            is_promoted: false,
        }
    }

    #[test]
    fn a_default_before_a_required_parameter_is_required() {
        let params = [param(true, false), param(false, false), param(true, false)];
        assert_eq!(optional_in_php(&params), [false, false, true]);
    }

    #[test]
    fn a_variadic_parameter_keeps_the_defaults_before_it_optional() {
        let params = [param(true, false), param(false, true)];
        assert_eq!(optional_in_php(&params), [true, false]);
    }
}
