//! TKT-017: Interceptor PHP code generator.
//!
//! Generates `<ns>\Interceptor` class files in `generated/code/**`.

use di_resolver::InterceptorSpec;
use php_extractor::types::{ClassInfo, MethodParam, MethodSignature};

/// Generate the PHP source for an Interceptor class.
///
/// `spec` — the InterceptorSpec from the resolver.
/// `target_info` — optional ClassInfo for the target class (for constructor params).
pub fn generate_interceptor(spec: &InterceptorSpec, target_info: Option<&ClassInfo>) -> String {
    let fqcn = spec.fqcn.trim_start_matches('\\');
    let ns = interceptor_namespace(fqcn);
    let ctor_params = target_info
        .and_then(|i| i.constructor.as_ref())
        .map(|c| &c.params);

    let mut out = String::new();
    out.push_str("<?php\n");
    out.push_str(&format!("namespace {};\n\n", ns));
    out.push_str(&format!(
        "/**\n * Interceptor class for @see \\{}\n */\n",
        fqcn
    ));
    out.push_str(&format!(
        "class Interceptor extends \\{} implements \\Magento\\Framework\\Interception\\InterceptorInterface\n{{\n",
        fqcn
    ));
    out.push_str("    use \\Magento\\Framework\\Interception\\Interceptor;\n\n");

    // Constructor
    if let Some(params) = ctor_params {
        out.push_str("    public function __construct(");
        out.push_str(&render_params(params));
        out.push_str(")\n    {\n");
        out.push_str("        $this->___init();\n");
        out.push_str("        parent::__construct(");
        out.push_str(&render_param_names(params));
        out.push_str(");\n");
        out.push_str("    }\n\n");
    } else {
        out.push_str("    public function __construct()\n    {\n");
        out.push_str("        $this->___init();\n");
        out.push_str("    }\n\n");
    }

    // Intercepted methods
    let rendered_methods: Vec<String> = spec
        .public_methods
        .iter()
        .filter_map(|method| render_intercepted_method(method))
        .collect();
    if !rendered_methods.is_empty() {
        out.push_str(&rendered_methods.join("\n"));
    }

    out.push_str("}\n");
    out
}

fn render_intercepted_method(m: &MethodSignature) -> Option<String> {
    // Magento's interceptor framework does not support static methods.
    // Skip them to avoid `$this` in static context errors.
    if m.is_static {
        return None;
    }

    let is_void = m
        .return_type
        .as_deref()
        .map(|r| r == "void" || r == "never")
        .unwrap_or(false);

    let mut s = String::new();
    s.push_str("    /**\n     * {@inheritdoc}\n     */\n");
    let reference_prefix = if m.returns_reference { "& " } else { "" };
    s.push_str(&format!(
        "    public function {}{}(",
        reference_prefix, m.name
    ));
    s.push_str(&render_method_params(&m.params));
    if let Some(ret) = &m.return_type {
        s.push_str(&format!(") : {}", render_type_hint(ret)));
    } else {
        s.push(')');
    }
    s.push_str("\n    {\n");
    s.push_str(&format!(
        "        $pluginInfo = $this->pluginList->getNext($this->subjectType, '{}');\n",
        m.name
    ));

    let arg_names: Vec<String> = m
        .params
        .iter()
        .map(|p| {
            if p.is_variadic {
                format!("...${}", p.name)
            } else {
                format!("${}", p.name)
            }
        })
        .collect();

    if is_void {
        s.push_str(&format!(
            "        $pluginInfo ? $this->___callPlugins('{}', func_get_args(), $pluginInfo) : parent::{}({});\n",
            m.name, m.name, arg_names.join(", ")
        ));
    } else {
        s.push_str(&format!(
            "        return $pluginInfo ? $this->___callPlugins('{}', func_get_args(), $pluginInfo) : parent::{}({});\n",
            m.name, m.name, arg_names.join(", ")
        ));
    }
    s.push_str("    }\n");
    Some(s)
}

fn render_params(params: &[php_extractor::types::ConstructorParam]) -> String {
    params
        .iter()
        .map(|p| {
            let mut s = String::new();
            if let Some(th) = &p.type_hint {
                let rendered = render_type_hint(th);
                s.push_str(&format!("{} ", rendered));
            }
            if p.is_variadic {
                s.push_str("...");
            }
            s.push_str(&format!("${}", p.name));
            if p.is_optional && !p.is_variadic {
                s.push_str(" = null");
            }
            s
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_param_names(params: &[php_extractor::types::ConstructorParam]) -> String {
    params
        .iter()
        .map(|p| {
            if p.is_variadic {
                format!("...${}", p.name)
            } else {
                format!("${}", p.name)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_method_params(params: &[MethodParam]) -> String {
    params
        .iter()
        .map(|p| {
            let mut s = String::new();
            if let Some(th) = &p.type_hint {
                let rendered = render_type_hint(th);
                s.push_str(&format!("{} ", rendered));
            }
            if p.is_variadic {
                s.push_str("...");
            }
            if p.is_by_ref {
                s.push('&');
            }
            s.push_str(&format!("${}", p.name));
            if p.has_default && !p.is_variadic {
                s.push_str(" = null");
            }
            s
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render a PHP type hint, adding `\` only for class/interface names (not primitives).
/// Public so proxy.rs can reuse it.
///
/// Handles:
///   - Nullable prefix `?`
///   - PHP built-in scalar/pseudo types (no `\`)
///   - Union types `Foo|Bar` (prefix each non-primitive part)
pub fn render_type_hint(th: &str) -> String {
    const PRIMITIVES: &[&str] = &[
        "string", "int", "float", "bool", "array", "callable", "iterable", "object", "null",
        "void", "mixed", "never", "self", "parent", "static", "false", "true",
    ];

    let (nullable, core) = if let Some(rest) = th.strip_prefix('?') {
        ("?", rest)
    } else {
        ("", th)
    };

    if PRIMITIVES.contains(&core) {
        return format!("{}{}", nullable, core);
    }

    // Union types: split on `|`
    if core.contains('|') {
        let parts: Vec<String> = core
            .split('|')
            .map(|p| {
                let p = p.trim();
                if PRIMITIVES.contains(&p) {
                    p.to_string()
                } else {
                    format!("\\{}", p)
                }
            })
            .collect();
        return format!("{}{}", nullable, parts.join("|"));
    }

    // Plain class name
    format!("{}\\{}", nullable, core)
}

/// Namespace for generated interceptor class: `<TargetFQCN>`.
fn interceptor_namespace(target_fqcn: &str) -> String {
    target_fqcn.trim_start_matches('\\').to_string()
}

/// Return the file path for an interceptor: `generated/code/Foo/Bar/Interceptor.php`.
pub fn interceptor_path(fqcn: &str) -> String {
    format!("{}/Interceptor.php", fqcn.replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use di_resolver::PluginRef;

    #[test]
    fn test_interceptor_path() {
        assert_eq!(
            interceptor_path("Foo\\Bar\\Baz"),
            "Foo/Bar/Baz/Interceptor.php"
        );
    }

    #[test]
    fn test_generate_interceptor_basic() {
        let spec = InterceptorSpec {
            fqcn: "Foo\\Bar".to_string(),
            plugins: vec![PluginRef {
                name: "p".to_string(),
                type_name: "P".to_string(),
                sort_order: 0,
            }],
            public_methods: vec![],
        };
        let out = generate_interceptor(&spec, None);
        assert!(out.contains("namespace Foo\\Bar;"));
        assert!(out.contains("class Interceptor extends \\Foo\\Bar"));
        assert!(out.contains("use \\Magento\\Framework\\Interception\\Interceptor;"));
    }

    #[test]
    fn test_nested_target_namespace_matches_path_mapping() {
        let spec = InterceptorSpec {
            fqcn: "Vendor\\Module\\Service\\Runner".to_string(),
            plugins: vec![],
            public_methods: vec![],
        };
        let out = generate_interceptor(&spec, None);
        assert!(out.contains("namespace Vendor\\Module\\Service\\Runner;"));
        assert!(out.contains("class Interceptor extends \\Vendor\\Module\\Service\\Runner"));
        assert_eq!(
            interceptor_path("Vendor\\Module\\Service\\Runner"),
            "Vendor/Module/Service/Runner/Interceptor.php"
        );
    }

    #[test]
    fn test_render_type_hint_primitives() {
        assert_eq!(render_type_hint("string"), "string");
        assert_eq!(render_type_hint("int"), "int");
        assert_eq!(render_type_hint("bool"), "bool");
        assert_eq!(render_type_hint("array"), "array");
        assert_eq!(render_type_hint("void"), "void");
    }

    #[test]
    fn test_render_type_hint_class() {
        assert_eq!(render_type_hint("Foo\\Bar"), "\\Foo\\Bar");
        assert_eq!(render_type_hint("?Foo\\Bar"), "?\\Foo\\Bar");
    }

    #[test]
    fn test_render_type_hint_nullable_primitive() {
        assert_eq!(render_type_hint("?string"), "?string");
        assert_eq!(render_type_hint("?int"), "?int");
    }

    #[test]
    fn test_static_method_skipped() {
        use php_extractor::types::MethodSignature;
        let method = MethodSignature {
            name: "getInstance".to_string(),
            params: vec![],
            return_type: Some("self".to_string()),
            is_static: true,
            returns_reference: false,
        };
        let result = render_intercepted_method(&method);
        assert!(result.is_none(), "static methods must be skipped");
    }

    #[test]
    fn test_void_method_no_return() {
        use php_extractor::types::MethodSignature;
        let method = MethodSignature {
            name: "doSomething".to_string(),
            params: vec![],
            return_type: Some("void".to_string()),
            is_static: false,
            returns_reference: false,
        };
        let result = render_intercepted_method(&method).unwrap();
        assert!(
            !result.contains("return $pluginInfo"),
            "void method must not use return"
        );
        assert!(result.contains("$pluginInfo ? $this->___callPlugins"));
    }

    #[test]
    fn test_method_signature_renders_by_ref_and_variadic_order() {
        use php_extractor::types::{MethodParam, MethodSignature};
        let method = MethodSignature {
            name: "resolve".to_string(),
            params: vec![
                MethodParam {
                    name: "value".to_string(),
                    type_hint: Some("?Foo\\Bar|null".to_string()),
                    has_default: false,
                    is_variadic: false,
                    is_by_ref: true,
                },
                MethodParam {
                    name: "rest".to_string(),
                    type_hint: Some("Baz\\Qux".to_string()),
                    has_default: false,
                    is_variadic: true,
                    is_by_ref: false,
                },
            ],
            return_type: Some("string|null".to_string()),
            is_static: false,
            returns_reference: true,
        };

        let rendered = render_intercepted_method(&method).unwrap();
        assert!(rendered.contains("public function & resolve("));
        assert!(rendered.contains("?\\Foo\\Bar|null &$value"));
        assert!(rendered.contains("\\Baz\\Qux ...$rest"));
        assert!(rendered.contains(") : string|null"));
    }
}
