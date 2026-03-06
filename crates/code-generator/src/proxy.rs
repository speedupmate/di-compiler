//! TKT-019: Proxy PHP code generator.
//!
//! Two proxy variants:
//!   - `class Proxy extends \Target` — when target is a concrete class
//!   - `class Proxy implements \Target` — when target is an interface
//!
//! The proxy delegates all public methods to `_getSubject()`.

use di_resolver::ProxySpec;
use php_extractor::types::{ClassInfo, ClassKind, MethodSignature};

/// Generate the PHP source for a Proxy class.
///
/// `target_info` — ClassInfo for the target, used to determine extends vs implements
///                 and to generate delegating method bodies.
pub fn generate_proxy(spec: &ProxySpec, target_info: Option<&ClassInfo>) -> String {
    let (ns, _class_name) = split_fqcn(&spec.proxy_fqcn);
    let target = &spec.target_fqcn;

    // Decide whether target is interface or concrete class
    let is_interface = target_info
        .map(|i| matches!(i.kind, ClassKind::Interface))
        .unwrap_or(false);

    let inheritance = if is_interface {
        format!(
            "implements \\{}, \\Magento\\Framework\\ObjectManager\\NoninterceptableInterface",
            target
        )
    } else {
        format!(
            "extends \\{} implements \\Magento\\Framework\\ObjectManager\\NoninterceptableInterface",
            target
        )
    };

    let public_methods = target_info
        .map(|i| i.public_methods.clone())
        .unwrap_or_default();

    let mut out = String::new();
    out.push_str("<?php\n");
    out.push_str(&format!("namespace {};\n\n", ns));
    out.push_str(&format!("/**\n * Proxy class for @see \\{}\n */\n", target));
    out.push_str(&format!("class Proxy {}\n{{\n", inheritance));

    out.push_str(
        r#"    /**
     * Object Manager instance
     *
     * @var \Magento\Framework\ObjectManagerInterface
     */
    protected $_objectManager = null;

    /**
     * Proxied instance name
     *
     * @var string
     */
    protected $_instanceName = null;

    /**
     * Proxied instance
     *
"#,
    );
    out.push_str(&format!("     * @var \\{}\n     */\n", target));
    out.push_str("    protected $_subject = null;\n\n");
    out.push_str(
        r#"    /**
     * Instance shareability flag
     *
     * @var bool
     */
    protected $_isShared = null;

    /**
     * Proxy constructor
     *
     * @param \Magento\Framework\ObjectManagerInterface $objectManager
     * @param string $instanceName
     * @param bool $shared
     */
"#,
    );
    out.push_str(&format!(
        "    public function __construct(\\Magento\\Framework\\ObjectManagerInterface $objectManager, $instanceName = '\\\\{}', $shared = true)\n    {{\n",
        target.replace('\\', "\\\\")
    ));
    out.push_str(
        r#"        $this->_objectManager = $objectManager;
        $this->_instanceName = $instanceName;
        $this->_isShared = $shared;
    }

    /**
     * @return array
     */
    public function __sleep()
    {
        return ['_subject', '_isShared', '_instanceName'];
    }

    /**
     * Retrieve ObjectManager from global scope
     */
    public function __wakeup()
    {
        $this->_objectManager = \Magento\Framework\App\ObjectManager::getInstance();
    }

    /**
     * Clone proxied instance
     */
    public function __clone()
    {
        if ($this->_subject) {
            $this->_subject = clone $this->_getSubject();
        }
    }

    /**
     * Debug proxied instance
     */
    public function __debugInfo()
    {
        return ['i' => $this->_subject];
    }

"#,
    );

    // _getSubject()
    out.push_str(&format!(
        r#"    /**
     * Get proxied instance
     *
     * @return \{target}
     */
    protected function _getSubject()
    {{
        if (!$this->_subject) {{
            $this->_subject = true === $this->_isShared
                ? $this->_objectManager->get($this->_instanceName)
                : $this->_objectManager->create($this->_instanceName);
        }}
        return $this->_subject;
    }}

"#,
        target = target
    ));

    // Delegating methods
    let mut rendered_methods: Vec<String> = Vec::new();
    for method in &public_methods {
        if method.name == "_resetState" {
            rendered_methods.push(
                "    /**\n     * Reset state of proxied instance\n     */\n    public function _resetState() : void\n    {\n        if ($this->_subject) {\n            $this->_subject->_resetState(); \n        }\n    }\n"
                    .to_string(),
            );
            continue;
        }
        if let Some(rendered) = render_proxy_method(method) {
            rendered_methods.push(rendered);
        }
    }
    if !rendered_methods.is_empty() {
        out.push_str(&rendered_methods.join("\n"));
    }

    out.push_str("}\n");
    out
}

fn render_proxy_method(m: &MethodSignature) -> Option<String> {
    if matches!(
        m.name.as_str(),
        "__sleep" | "__wakeup" | "__clone" | "__debugInfo" | "_resetState"
    ) {
        return None;
    }
    // Skip static methods — _getSubject() is an instance method and cannot be
    // called in a static context.
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
    s.push_str(&format!("    public function {}(", m.name));

    let params_str: Vec<String> = m
        .params
        .iter()
        .map(|p| {
            let mut part = String::new();
            if let Some(th) = &p.type_hint {
                let rendered = crate::interceptor::render_type_hint(th);
                if p.is_variadic {
                    part.push_str(&format!("...{} ", rendered));
                } else {
                    part.push_str(&format!("{} ", rendered));
                }
            }
            if p.is_by_ref {
                part.push('&');
            }
            part.push_str(&format!("${}", p.name));
            if !p.is_variadic {
                if let Some(default_value) = &p.default_value {
                    part.push_str(" = ");
                    part.push_str(default_value);
                }
            }
            part
        })
        .collect();
    s.push_str(&params_str.join(", "));

    if let Some(ret) = &m.return_type {
        s.push_str(&format!(
            ") : {}",
            crate::interceptor::render_type_hint(ret)
        ));
    } else {
        s.push(')');
    }
    s.push_str("\n    {\n");

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
            "        $this->_getSubject()->{}({});\n    }}\n",
            m.name,
            arg_names.join(", ")
        ));
    } else {
        s.push_str(&format!(
            "        return $this->_getSubject()->{}({});\n    }}\n",
            m.name,
            arg_names.join(", ")
        ));
    }
    Some(s)
}

/// Return the file path for a proxy: `generated/code/Foo/Bar/Proxy.php`.
pub fn proxy_path(proxy_fqcn: &str) -> String {
    format!("{}.php", proxy_fqcn.replace('\\', "/"))
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

    #[test]
    fn test_proxy_path() {
        assert_eq!(proxy_path("Foo\\Bar\\Proxy"), "Foo/Bar/Proxy.php");
    }

    #[test]
    fn test_generate_proxy_for_class() {
        let spec = ProxySpec {
            target_fqcn: "Foo\\Heavy".to_string(),
            proxy_fqcn: "Foo\\Heavy\\Proxy".to_string(),
        };
        let out = generate_proxy(&spec, None);
        assert!(out.contains("namespace Foo\\Heavy;"));
        assert!(out.contains("class Proxy extends \\Foo\\Heavy"));
        assert!(out.contains("_getSubject"));
    }
}
