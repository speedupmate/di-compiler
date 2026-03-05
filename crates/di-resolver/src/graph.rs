use std::collections::HashMap;

use php_extractor::MethodSignature;

/// The fully resolved dependency graph ready for code generation.
#[derive(Debug, Default)]
pub struct ResolvedGraph {
    pub classes_needing_interceptor: Vec<InterceptorSpec>,
    pub classes_needing_factory: Vec<FactorySpec>,
    pub classes_needing_proxy: Vec<ProxySpec>,
    /// FQCN → resolved constructor args (for metadata generation)
    pub constructor_map: HashMap<String, Vec<ResolvedArg>>,
    /// All FQCNs encountered during scan (for interception.php)
    pub all_fqcns: HashMap<String, bool>,
}

#[derive(Debug, Clone)]
pub struct InterceptorSpec {
    pub fqcn: String,
    pub plugins: Vec<PluginRef>,
    pub public_methods: Vec<MethodSignature>,
}

#[derive(Debug, Clone)]
pub struct PluginRef {
    pub name: String,
    pub type_name: String,
    pub sort_order: i32,
}

#[derive(Debug, Clone)]
pub struct FactorySpec {
    pub target_fqcn: String,
    pub factory_fqcn: String,
}

#[derive(Debug, Clone)]
pub struct ProxySpec {
    pub target_fqcn: String,
    pub proxy_fqcn: String,
}

#[derive(Debug, Clone)]
pub enum ResolvedType {
    Class(String),
    Interface(String),
    VirtualType(String),
    Scalar,
}

#[derive(Debug, Clone)]
pub struct ResolvedArg {
    pub name: String,
    pub resolved: ResolvedArgValue,
}

#[derive(Debug, Clone)]
pub enum ResolvedArgValue {
    SharedInstance(String),
    NonSharedInstance(String),
    Scalar(String),
    Null,
    Array(Vec<ResolvedArg>),
    GlobalArgRef { arg_name: String, default: Option<String> },
}
