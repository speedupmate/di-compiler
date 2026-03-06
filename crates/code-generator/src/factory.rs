//! TKT-018: Factory PHP code generator.

use di_resolver::FactorySpec;

/// Generate the PHP source for a Factory class.
pub fn generate_factory(spec: &FactorySpec) -> String {
    let (ns, class_name) = split_fqcn(&spec.factory_fqcn);
    let is_extension_interface_factory = spec.factory_fqcn.ends_with("ExtensionInterfaceFactory");
    let target_fqcn =
        if is_extension_interface_factory && spec.target_fqcn.ends_with("ExtensionInterface") {
            spec.target_fqcn.trim_end_matches("Interface")
        } else {
            spec.target_fqcn.as_str()
        };
    let target_fqcn_escaped = format!("\\\\{}", target_fqcn.replace('\\', "\\\\"));
    let mut out = String::new();
    out.push_str("<?php\n");
    if !ns.is_empty() {
        out.push_str(&format!("namespace {};\n\n", ns));
    }
    let class_doc = if is_extension_interface_factory {
        "ExtensionInterfaceFactory class"
    } else {
        "Factory class"
    };
    let ctor_doc = if is_extension_interface_factory {
        "ExtensionInterfaceFactory constructor"
    } else {
        "Factory constructor"
    };
    out.push_str(&format!(
        r#"/**
 * {class_doc} for @see \{target}
 */
class {class_name}
{{
    /**
     * Object Manager instance
     *
     * @var \Magento\Framework\ObjectManagerInterface
     */
    protected $_objectManager = null;

    /**
     * Instance name to create
     *
     * @var string
     */
    protected $_instanceName = null;

    /**
     * {ctor_doc}
     *
     * @param \Magento\Framework\ObjectManagerInterface $objectManager
     * @param string $instanceName
     */
    public function __construct(\Magento\Framework\ObjectManagerInterface $objectManager, $instanceName = '{target_escaped}')
    {{
        $this->_objectManager = $objectManager;
        $this->_instanceName = $instanceName;
    }}

    /**
     * Create class instance with specified parameters
     *
     * @param array $data
     * @return \{target}
     */
    public function create(array $data = [])
    {{
        return $this->_objectManager->create($this->_instanceName, $data);
    }}
}}
"#,
        class_doc = class_doc,
        ctor_doc = ctor_doc,
        target = target_fqcn,
        target_escaped = target_fqcn_escaped,
        class_name = class_name,
    ));
    out
}

/// Return the file path for a factory: `generated/code/Foo/Bar/BazFactory.php`.
pub fn factory_path(factory_fqcn: &str) -> String {
    format!("{}.php", factory_fqcn.replace('\\', "/"))
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
    fn test_factory_path() {
        assert_eq!(
            factory_path("Foo\\Bar\\WidgetFactory"),
            "Foo/Bar/WidgetFactory.php"
        );
    }

    #[test]
    fn test_generate_factory() {
        let spec = FactorySpec {
            target_fqcn: "Foo\\Bar\\Widget".to_string(),
            factory_fqcn: "Foo\\Bar\\WidgetFactory".to_string(),
        };
        let out = generate_factory(&spec);
        assert!(out.contains("namespace Foo\\Bar;"));
        assert!(out.contains("class WidgetFactory"));
        assert!(out.contains("\\Foo\\Bar\\Widget"));
        assert!(
            out.contains(
                "public function __construct(\\Magento\\Framework\\ObjectManagerInterface $objectManager, $instanceName = '\\\\Foo\\\\Bar\\\\Widget')"
            )
        );
        assert!(out.contains("public function create(array $data = [])"));
    }
}
