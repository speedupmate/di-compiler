//! TKT-031: Extension attributes interface/class generators.

/// An extension attribute entry from extension_attributes.xml.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionAttributeSpec {
    pub code: String,
    pub php_type: String,
}

/// Generation inputs for ExtensionInterface + Extension class artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionSpec {
    /// Source data interface, e.g. `Magento\Sales\Api\Data\OrderInterface`.
    pub source_interface_fqcn: String,
    /// Generated extension interface FQCN, e.g. `...\OrderExtensionInterface`.
    pub extension_interface_fqcn: String,
    /// Generated extension class FQCN, e.g. `...\OrderExtension`.
    pub extension_class_fqcn: String,
    /// Declared extension attributes for this source interface.
    pub attributes: Vec<ExtensionAttributeSpec>,
}

/// Generate PHP source for `*ExtensionInterface`.
pub fn generate_extension_interface(spec: &ExtensionSpec) -> String {
    let (ns, class_name) = split_fqcn(&spec.extension_interface_fqcn);
    let mut methods = String::new();
    for attr in &spec.attributes {
        let property_name = snake_case_to_camel_case(&attr.code);
        let suffix = ucfirst(&property_name);
        let getter = format!("get{suffix}");
        let setter = format!("set{suffix}");
        let doc_type = normalize_doc_type(&attr.php_type);
        let setter_param = render_setter_param(&property_name, &attr.php_type);

        methods.push_str(&format!(
            r#"    /**
     * @return {doc_type}|null
     */
    public function {getter}();

    /**
     * @param {doc_type} ${property_name}
     * @return $this
     */
    public function {setter}({setter_param});

"#
        ));
    }

    format!(
        r#"<?php
namespace {ns};

/**
 * ExtensionInterface class for @see \{source}
 */
interface {class_name} extends \Magento\Framework\Api\ExtensionAttributesInterface
{{
{methods}}}
"#,
        ns = ns,
        source = spec.source_interface_fqcn,
        class_name = class_name,
        methods = methods,
    )
}

/// Generate PHP source for `*Extension`.
pub fn generate_extension(spec: &ExtensionSpec) -> String {
    let (ns, class_name) = split_fqcn(&spec.extension_class_fqcn);
    let interface_name = short_name(&spec.extension_interface_fqcn);
    let mut methods = String::new();
    for attr in &spec.attributes {
        let property_name = snake_case_to_camel_case(&attr.code);
        let suffix = ucfirst(&property_name);
        let getter = format!("get{suffix}");
        let setter = format!("set{suffix}");
        let doc_type = normalize_doc_type(&attr.php_type);
        let setter_param = render_setter_param(&property_name, &attr.php_type);

        methods.push_str(&format!(
            r#"    /**
     * @return {doc_type}|null
     */
    public function {getter}()
    {{
        return $this->_get('{code}');
    }}

    /**
     * @param {doc_type} ${property_name}
     * @return $this
     */
    public function {setter}({setter_param})
    {{
        $this->setData('{code}', ${property_name});
        return $this;
    }}

"#,
            code = attr.code
        ));
    }

    format!(
        r#"<?php
namespace {ns};

/**
 * Extension class for @see \{source}
 */
class {class_name} extends \Magento\Framework\Api\AbstractSimpleObject implements {interface_name}
{{
{methods}}}
"#,
        ns = ns,
        source = spec.source_interface_fqcn,
        class_name = class_name,
        interface_name = interface_name,
        methods = methods,
    )
}

/// Return file path for an extension artifact FQCN.
pub fn extension_path(fqcn: &str) -> String {
    format!("{}.php", fqcn.replace('\\', "/"))
}

fn split_fqcn(fqcn: &str) -> (String, String) {
    match fqcn.rfind('\\') {
        Some(pos) => (fqcn[..pos].to_string(), fqcn[pos + 1..].to_string()),
        None => (String::new(), fqcn.to_string()),
    }
}

fn short_name(fqcn: &str) -> &str {
    fqcn.rsplit('\\').next().unwrap_or(fqcn)
}

fn snake_case_to_camel_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut upper = false;
    for ch in input.chars() {
        if ch == '_' {
            upper = true;
            continue;
        }
        if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

fn ucfirst(input: &str) -> String {
    let mut chars = input.chars();
    match chars.next() {
        Some(first) => {
            let mut out = String::new();
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
            out
        }
        None => String::new(),
    }
}

fn normalize_doc_type(php_type: &str) -> String {
    let t = php_type.trim().trim_start_matches('\\');
    if t.is_empty() {
        return "mixed".to_string();
    }
    if let Some(base) = t.strip_suffix("[]") {
        if base.contains('\\') {
            return format!("\\{}[]", base.trim_start_matches('\\'));
        }
        return format!("{base}[]");
    }
    if t.contains('\\') {
        return format!("\\{t}");
    }
    t.to_string()
}

fn render_setter_param(property_name: &str, php_type: &str) -> String {
    let var = format!("${property_name}");
    match setter_type_hint(php_type) {
        Some(type_hint) => format!("{type_hint} {var}"),
        None => var,
    }
}

fn setter_type_hint(php_type: &str) -> Option<String> {
    let t = php_type.trim().trim_start_matches('\\');
    if t.is_empty() || t.ends_with("[]") || t.contains('|') || t.contains('&') {
        return None;
    }
    let lowered = t.to_ascii_lowercase();
    if matches!(
        lowered.as_str(),
        "string" | "int" | "integer" | "float" | "double" | "bool" | "boolean" | "mixed"
    ) {
        return None;
    }
    if t.contains('\\') {
        return Some(format!("\\{t}"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_spec() -> ExtensionSpec {
        ExtensionSpec {
            source_interface_fqcn: "Magento\\GiftMessage\\Api\\Data\\MessageInterface".to_string(),
            extension_interface_fqcn:
                "Magento\\GiftMessage\\Api\\Data\\MessageExtensionInterface".to_string(),
            extension_class_fqcn: "Magento\\GiftMessage\\Api\\Data\\MessageExtension".to_string(),
            attributes: vec![
                ExtensionAttributeSpec {
                    code: "entity_id".to_string(),
                    php_type: "string".to_string(),
                },
                ExtensionAttributeSpec {
                    code: "gift_message".to_string(),
                    php_type: "Magento\\GiftMessage\\Api\\Data\\MessageInterface".to_string(),
                },
            ],
        }
    }

    #[test]
    fn test_extension_path() {
        assert_eq!(
            extension_path("Magento\\GiftMessage\\Api\\Data\\MessageExtensionInterface"),
            "Magento/GiftMessage/Api/Data/MessageExtensionInterface.php"
        );
    }

    #[test]
    fn test_generate_extension_interface_methods() {
        let out = generate_extension_interface(&sample_spec());
        assert!(out.contains("interface MessageExtensionInterface extends \\Magento\\Framework\\Api\\ExtensionAttributesInterface"));
        assert!(out.contains("public function getEntityId();"));
        assert!(out.contains("public function setEntityId($entityId);"));
        assert!(out.contains(
            "public function setGiftMessage(\\Magento\\GiftMessage\\Api\\Data\\MessageInterface $giftMessage);"
        ));
    }

    #[test]
    fn test_generate_extension_class_methods() {
        let out = generate_extension(&sample_spec());
        assert!(out.contains("class MessageExtension extends \\Magento\\Framework\\Api\\AbstractSimpleObject implements MessageExtensionInterface"));
        assert!(out.contains("return $this->_get('entity_id');"));
        assert!(out.contains("$this->setData('gift_message', $giftMessage);"));
    }
}
