//! TKT-017: Interceptor PHP code generator.
//!
//! Generates `<ns>\Interceptor` class files in `generated/code/**`.

use php_extractor::types::{ClassInfo, MethodParam, MethodSignature};
use di_resolver::InterceptorSpec;

/// Generate the PHP source for an Interceptor class.
///
/// `spec` — the InterceptorSpec from the resolver.
/// `target_info` — optional ClassInfo for the target class (for constructor params).
pub fn generate_interceptor(spec: &InterceptorSpec, target_info: Option<&ClassInfo>) -> String {
    let fqcn = &spec.fqcn;
    let (ns, _class_name) = split_fqcn(fqcn);
    let ctor_params = target_info.and_then(|i| i.constructor.as_ref()).map(|c| &c.params);

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
    for method in &spec.public_methods {
        out.push_str(&render_intercepted_method(method));
    }

    out.push_str("}\n");
    out
}

fn render_intercepted_method(m: &MethodSignature) -> String {
    // Magento's interceptor framework does not support static methods.
    // Skip them to avoid `$this` in static context errors.
    if m.is_static {
        return String::new();
    }

    let is_void = m
        .return_type
        .as_deref()
        .map(|r| r == "void" || r == "never")
        .unwrap_or(false);

    let mut s = String::new();
    s.push_str("    /**\n     * {@inheritdoc}\n     */\n");
    s.push_str(&format!("    public function {}(", m.name));
    s.push_str(&render_method_params(&m.params));
    if let Some(ret) = &m.return_type {
        s.push_str(&format!(") : {}", ret));
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
    s.push_str("    }\n\n");
    s
}

fn render_params(params: &[php_extractor::types::ConstructorParam]) -> String {
    params
        .iter()
        .map(|p| {
            let mut s = String::new();
            if let Some(th) = &p.type_hint {
                let rendered = render_type_hint(th);
                if p.is_variadic {
                    s.push_str(&format!("...{} ", rendered));
                } else {
                    s.push_str(&format!("{} ", rendered));
                }
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
                if p.is_variadic {
                    s.push_str(&format!("...{} ", rendered));
                } else {
                    s.push_str(&format!("{} ", rendered));
                }
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
        "string", "int", "float", "bool", "array", "callable", "iterable",
        "object", "null", "void", "mixed", "never", "self", "parent", "static",
        "false", "true",
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

/// Split `Foo\Bar\Baz` into (`Foo\Bar`, `Baz`).
fn split_fqcn(fqcn: &str) -> (String, String) {
    match fqcn.rfind('\\') {
        Some(pos) => (fqcn[..pos].to_string(), fqcn[pos + 1..].to_string()),
        None => (String::new(), fqcn.to_string()),
    }
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
        assert!(out.contains("namespace Foo;"));
        assert!(out.contains("class Interceptor extends \\Foo\\Bar"));
        assert!(out.contains("use \\Magento\\Framework\\Interception\\Interceptor;"));
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
        use php_extractor::types::{MethodParam, MethodSignature};
        let method = MethodSignature {
            name: "getInstance".to_string(),
            params: vec![],
            return_type: Some("self".to_string()),
            is_static: true,
        };
        let result = render_intercepted_method(&method);
        assert!(result.is_empty(), "static methods must be skipped");
    }

    #[test]
    fn test_void_method_no_return() {
        use php_extractor::types::{MethodParam, MethodSignature};
        let method = MethodSignature {
            name: "doSomething".to_string(),
            params: vec![],
            return_type: Some("void".to_string()),
            is_static: false,
        };
        let result = render_intercepted_method(&method);
        assert!(!result.contains("return $pluginInfo"), "void method must not use return");
        assert!(result.contains("$pluginInfo ? $this->___callPlugins"));
    }
}
