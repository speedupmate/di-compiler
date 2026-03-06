//! ProxyDeferred PHP code generator.

use php_extractor::types::{ClassInfo, ClassKind, MethodParam, MethodSignature};

/// Generate the PHP source for a ProxyDeferred class.
pub fn generate_proxy_deferred(
    proxy_fqcn: &str,
    target_fqcn: &str,
    target_info: Option<&ClassInfo>,
) -> String {
    let (ns, class_name) = split_fqcn(proxy_fqcn);

    let is_interface = target_info
        .map(|i| matches!(i.kind, ClassKind::Interface))
        .unwrap_or(false);
    let inheritance = if is_interface {
        format!(
            "implements \\{}, \\Magento\\Framework\\ObjectManager\\NoninterceptableInterface",
            target_fqcn
        )
    } else {
        format!(
            "extends \\{} implements \\Magento\\Framework\\ObjectManager\\NoninterceptableInterface",
            target_fqcn
        )
    };

    let mut out = String::new();
    out.push_str("<?php\n");
    out.push_str(&format!("namespace {};\n\n", ns));
    out.push_str(&format!(
        "/**\n * ProxyDeferred class for @see \\{}\n */\n",
        target_fqcn
    ));
    out.push_str(&format!("class {} {}\n{{\n", class_name, inheritance));
    out.push_str(
        r#"    /**
     * Proxied instance
     *
     * @var string
     */
    private $instance = null;

    /**
     * Deferred to wait for
     *
     * @var string
     */
    private $deferred = null;

    /**
     * ProxyDeferred constructor
     *
     * @param \Magento\Framework\ObjectManager\DefinitionFactory $objectManager
     */
    public function __construct(\Magento\Framework\Async\DeferredInterface $deferred)
    {
        $this->deferred = $deferred;
    }

    /**
     * Serialize only the instance
     *
     * @return array
     */
    public function __sleep()
    {
        $this->wait();
        return ['instance'];
    }

    /**
     * Clone proxied instance
     */
    public function __clone()
    {
        $this->wait();
        $this->instance = clone $this->instance;
    }

"#,
    );
    out.push_str(&format!(
        r#"    /**
     * Get proxied instance
     *
     * @return \{target}
     */
    private function wait()
    {{
        if (!$this->instance) {{
            $this->instance = $this->deferred->get();
            if (!$this->instance instanceof \{target}) {{
                throw new \RuntimeException('Wrong instance returned by deferred');
            }}
        }}
        return $this->instance;
    }}

"#,
        target = target_fqcn
    ));

    let public_methods = target_info
        .map(|info| info.public_methods.clone())
        .unwrap_or_default();
    for method in &public_methods {
        if let Some(rendered) = render_deferred_proxy_method(method) {
            out.push_str(&rendered);
        }
    }
    out.push_str("}\n");
    out
}

/// Return the file path for a proxy deferred class.
pub fn proxy_deferred_path(proxy_fqcn: &str) -> String {
    format!("{}.php", proxy_fqcn.replace('\\', "/"))
}

fn render_deferred_proxy_method(m: &MethodSignature) -> Option<String> {
    if !is_proxyable_method(m) {
        return None;
    }

    let is_void = m
        .return_type
        .as_deref()
        .map(|r| r == "void" || r == "never")
        .unwrap_or(false);

    let mut s = String::new();
    s.push_str("    /**\n     * @inheritDoc\n     */\n");
    let reference_prefix = if m.returns_reference { "& " } else { "" };
    s.push_str(&format!(
        "    public function {}{}(",
        reference_prefix, m.name
    ));
    s.push_str(&render_params(&m.params));
    if let Some(ret) = &m.return_type {
        s.push_str(&format!(
            ") : {}",
            crate::interceptor::render_type_hint(ret)
        ));
    } else {
        s.push(')');
    }
    s.push_str("\n    {\n");
    s.push_str("        $this->wait();\n");

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
            "        $this->instance->{}({});\n",
            m.name,
            arg_names.join(", ")
        ));
    } else {
        s.push_str(&format!(
            "        return $this->instance->{}({});\n",
            m.name,
            arg_names.join(", ")
        ));
    }
    s.push_str("    }\n\n");
    Some(s)
}

fn render_params(params: &[MethodParam]) -> String {
    params
        .iter()
        .map(|p| {
            let mut s = String::new();
            if let Some(th) = &p.type_hint {
                s.push_str(&format!("{} ", crate::interceptor::render_type_hint(th)));
            }
            if p.is_variadic {
                s.push_str("...");
            }
            if p.is_by_ref {
                s.push('&');
            }
            s.push_str(&format!("${}", p.name));
            if !p.is_variadic {
                if let Some(default_value) = &p.default_value {
                    s.push_str(" = ");
                    s.push_str(default_value);
                }
            }
            s
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_proxyable_method(method: &MethodSignature) -> bool {
    if method.is_static {
        return false;
    }
    !matches!(
        method.name.as_str(),
        "__construct" | "__destruct" | "__sleep" | "__wakeup" | "__clone"
    )
}

fn split_fqcn(fqcn: &str) -> (String, String) {
    match fqcn.rfind('\\') {
        Some(pos) => (fqcn[..pos].to_string(), fqcn[pos + 1..].to_string()),
        None => (String::new(), fqcn.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use php_extractor::types::{ClassInfo, ClassKind};
    use std::path::PathBuf;

    #[test]
    fn test_proxy_deferred_path() {
        assert_eq!(
            proxy_deferred_path("Foo\\Bar\\ProxyDeferred"),
            "Foo/Bar/ProxyDeferred.php"
        );
    }

    #[test]
    fn test_generate_proxy_deferred() {
        let info = ClassInfo {
            path: PathBuf::from("dummy.php"),
            namespace: "Foo\\Bar".to_string(),
            name: "Target".to_string(),
            fqcn: "Foo\\Bar\\Target".to_string(),
            kind: ClassKind::Class,
            extends: None,
            implements: vec![],
            constructor: None,
            is_abstract: false,
            is_final: false,
            public_methods: vec![MethodSignature {
                name: "getValue".to_string(),
                params: vec![],
                return_type: None,
                is_static: false,
                returns_reference: false,
            }],
        };

        let out =
            generate_proxy_deferred("Foo\\Bar\\ProxyDeferred", "Foo\\Bar\\Target", Some(&info));
        assert!(out.contains("namespace Foo\\Bar;"));
        assert!(out.contains("class ProxyDeferred extends \\Foo\\Bar\\Target"));
        assert!(out.contains("private function wait()"));
        assert!(out.contains("return $this->instance->getValue();"));
    }
}
