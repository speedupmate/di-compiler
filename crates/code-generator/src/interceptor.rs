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
    let mut s = String::new();
    s.push_str("    /**\n     * {@inheritdoc}\n     */\n");

    let static_kw = if m.is_static { "static " } else { "" };
    s.push_str(&format!("    public {}function {}(", static_kw, m.name));
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
    s.push_str(&format!(
        "        return $pluginInfo ? $this->___callPlugins('{}', func_get_args(), $pluginInfo) : parent::{}(",
        m.name, m.name
    ));
    // Forward args
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
    s.push_str(&arg_names.join(", "));
    s.push_str(");\n    }\n\n");
    s
}

fn render_params(params: &[php_extractor::types::ConstructorParam]) -> String {
    params
        .iter()
        .map(|p| {
            let mut s = String::new();
            if let Some(th) = &p.type_hint {
                if p.is_variadic {
                    s.push_str(&format!("...\\{} ", th));
                } else {
                    s.push_str(&format!("\\{} ", th));
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
                if p.is_variadic {
                    s.push_str(&format!("...\\{} ", th));
                } else {
                    s.push_str(&format!("\\{} ", th));
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
}
