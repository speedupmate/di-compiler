//! TKT-010: quick-xml SAX parser for a single di.xml file.
use rustc_hash::FxHashMap;
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::model::{Argument, DiConfig, Plugin, VirtualType};
use crate::Error;

/// Parse a single di.xml file into a DiConfig.
pub fn parse_di_xml(path: &Path) -> Result<DiConfig, Error> {
    parse_di_xml_impl(path)
}

pub fn parse_di_xml_impl(path: &Path) -> Result<DiConfig, Error> {
    let content = std::fs::read(path)?;
    parse_di_xml_bytes(&content)
}

pub fn parse_di_xml_bytes(content: &[u8]) -> Result<DiConfig, Error> {
    let mut reader = Reader::from_reader(content);
    reader.config_mut().trim_text(true);
    reader.config_mut().check_end_names = false;

    let mut config = DiConfig::default();

    let mut current_type: Option<String> = None;
    let mut current_virtual: Option<String> = None;
    let mut in_arguments = false;
    let mut arg_stack: Vec<ArgContext> = Vec::new();

    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) => {
                let local = local_name(e.name().as_ref());
                let attrs = parse_attrs(e);

                match local.as_str() {
                    "preference" => {
                        let for_type = normalize(&attrs.get("for").cloned().unwrap_or_default());
                        let to_type = normalize(&attrs.get("type").cloned().unwrap_or_default());
                        if !for_type.is_empty() && !to_type.is_empty() {
                            config.preferences.insert(for_type, to_type);
                        }
                    }
                    "type" if !in_arguments => {
                        let name = normalize(&attrs.get("name").cloned().unwrap_or_default());
                        if !name.is_empty() {
                            current_virtual = None;
                            let entry = config.type_configs.entry(name).or_default();
                            if let Some(shared_str) = attrs.get("shared") {
                                entry.shared = Some(shared_str != "false");
                            }
                            // self-closing <type .../> — no body, clear immediately
                            current_type = None;
                        }
                    }
                    "virtualType" => {
                        let name = normalize(&attrs.get("name").cloned().unwrap_or_default());
                        let type_name = normalize(&attrs.get("type").cloned().unwrap_or_default());
                        if !name.is_empty() {
                            config.virtual_types.insert(
                                name.clone(),
                                VirtualType {
                                    name: name.clone(),
                                    type_name,
                                },
                            );
                            let entry = config.type_configs.entry(name).or_default();
                            if let Some(shared_str) = attrs.get("shared") {
                                entry.shared = Some(shared_str != "false");
                            }
                        }
                    }
                    "plugin" if !in_arguments => {
                        let owner = current_type.as_ref().or(current_virtual.as_ref());
                        if let Some(owner) = owner.cloned() {
                            let name = attrs.get("name").cloned().unwrap_or_default();
                            let plugin_type =
                                normalize(&attrs.get("type").cloned().unwrap_or_default());
                            let sort_order: i32 = attrs
                                .get("sortOrder")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0);
                            let disabled =
                                attrs.get("disabled").map(|s| s == "true").unwrap_or(false);
                            if !name.is_empty() && (!plugin_type.is_empty() || disabled) {
                                config.plugins.entry(owner).or_default().push(Plugin {
                                    name,
                                    type_name: plugin_type,
                                    sort_order,
                                    disabled,
                                });
                            }
                        }
                    }
                    "argument" if in_arguments => {
                        // Self-closing argument (e.g. xsi:type="null") — push and immediately pop
                        let arg_name = attrs.get("name").cloned().unwrap_or_default();
                        let xsi_type = attrs
                            .get("xsi:type")
                            .or_else(|| attrs.get("type"))
                            .cloned()
                            .unwrap_or_default();
                        let shared = attrs.get("shared").map(|s| s != "false");
                        let ctx = ArgContext {
                            name: arg_name,
                            xsi_type,
                            shared,
                            sort_order: 0,
                            text: String::new(),
                            items: Vec::new(),
                        };
                        if let Some(arg) = ctx_to_argument(ctx) {
                            let owner = current_type.as_ref().or(current_virtual.as_ref()).cloned();
                            if let Some(owner) = owner {
                                config
                                    .type_configs
                                    .entry(owner)
                                    .or_default()
                                    .arguments
                                    .push(arg);
                            }
                        }
                    }
                    "item" if !arg_stack.is_empty() => {
                        // Self-closing item — push and immediately pop into parent
                        let arg_name = attrs.get("name").cloned().unwrap_or_default();
                        let xsi_type = attrs
                            .get("xsi:type")
                            .or_else(|| attrs.get("type"))
                            .cloned()
                            .unwrap_or_default();
                        let sort_order: i32 = attrs
                            .get("sortOrder")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);
                        let ctx = ArgContext {
                            name: arg_name,
                            xsi_type,
                            shared: None,
                            sort_order,
                            text: String::new(),
                            items: Vec::new(),
                        };
                        if let Some(child_arg) = ctx_to_argument(ctx) {
                            if let Some(parent) = arg_stack.last_mut() {
                                parent.items.push((sort_order, child_arg));
                            }
                        }
                    }
                    _ => {}
                }

                buf.clear();
            }
            Ok(Event::Start(ref e)) => {
                let local = local_name(e.name().as_ref());
                let attrs = parse_attrs(e);

                match local.as_str() {
                    "preference" => {
                        let for_type = normalize(&attrs.get("for").cloned().unwrap_or_default());
                        let to_type = normalize(&attrs.get("type").cloned().unwrap_or_default());
                        if !for_type.is_empty() && !to_type.is_empty() {
                            config.preferences.insert(for_type, to_type);
                        }
                    }
                    "type" if !in_arguments => {
                        let name = normalize(&attrs.get("name").cloned().unwrap_or_default());
                        if !name.is_empty() {
                            current_type = Some(name.clone());
                            current_virtual = None;
                            let entry = config.type_configs.entry(name).or_default();
                            if let Some(shared_str) = attrs.get("shared") {
                                entry.shared = Some(shared_str != "false");
                            }
                        }
                    }
                    "virtualType" => {
                        let name = normalize(&attrs.get("name").cloned().unwrap_or_default());
                        let type_name = normalize(&attrs.get("type").cloned().unwrap_or_default());
                        if !name.is_empty() {
                            current_virtual = Some(name.clone());
                            current_type = None;
                            config.virtual_types.insert(
                                name.clone(),
                                VirtualType {
                                    name: name.clone(),
                                    type_name,
                                },
                            );
                            let entry = config.type_configs.entry(name).or_default();
                            if let Some(shared_str) = attrs.get("shared") {
                                entry.shared = Some(shared_str != "false");
                            }
                        }
                    }
                    "plugin" if !in_arguments => {
                        let owner = current_type.as_ref().or(current_virtual.as_ref());
                        if let Some(owner) = owner.cloned() {
                            let name = attrs.get("name").cloned().unwrap_or_default();
                            let plugin_type =
                                normalize(&attrs.get("type").cloned().unwrap_or_default());
                            let sort_order: i32 = attrs
                                .get("sortOrder")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0);
                            let disabled =
                                attrs.get("disabled").map(|s| s == "true").unwrap_or(false);
                            if !name.is_empty() && (!plugin_type.is_empty() || disabled) {
                                config.plugins.entry(owner).or_default().push(Plugin {
                                    name,
                                    type_name: plugin_type,
                                    sort_order,
                                    disabled,
                                });
                            }
                        }
                    }
                    "arguments" => {
                        in_arguments = true;
                    }
                    "argument" if in_arguments => {
                        let arg_name = attrs.get("name").cloned().unwrap_or_default();
                        let xsi_type = attrs
                            .get("xsi:type")
                            .or_else(|| attrs.get("type"))
                            .cloned()
                            .unwrap_or_default();
                        let shared = attrs.get("shared").map(|s| s != "false");
                        arg_stack.push(ArgContext {
                            name: arg_name,
                            xsi_type,
                            shared,
                            sort_order: 0,
                            text: String::new(),
                            items: Vec::new(),
                        });
                    }
                    "item" if !arg_stack.is_empty() => {
                        let arg_name = attrs.get("name").cloned().unwrap_or_default();
                        let xsi_type = attrs
                            .get("xsi:type")
                            .or_else(|| attrs.get("type"))
                            .cloned()
                            .unwrap_or_default();
                        let sort_order: i32 = attrs
                            .get("sortOrder")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);
                        arg_stack.push(ArgContext {
                            name: arg_name,
                            xsi_type,
                            shared: None,
                            sort_order,
                            text: String::new(),
                            items: Vec::new(),
                        });
                    }
                    _ => {}
                }

                buf.clear();
            }
            Ok(Event::Text(ref t)) => {
                let text = t.unescape().unwrap_or_default().trim().to_string();
                if !text.is_empty() {
                    if let Some(ctx) = arg_stack.last_mut() {
                        ctx.text = text;
                    }
                }
                buf.clear();
            }
            Ok(Event::CData(ref t)) => {
                let text = String::from_utf8_lossy(t.as_ref()).trim().to_string();
                if !text.is_empty() {
                    if let Some(ctx) = arg_stack.last_mut() {
                        ctx.text = text;
                    }
                }
                buf.clear();
            }
            Ok(Event::End(ref e)) => {
                let local = local_name(e.name().as_ref());
                match local.as_str() {
                    "type" if !in_arguments => {
                        current_type = None;
                    }
                    "virtualType" => {
                        current_virtual = None;
                    }
                    "arguments" => {
                        in_arguments = false;
                    }
                    "argument" if in_arguments || !arg_stack.is_empty() => {
                        if let Some(ctx) = arg_stack.pop() {
                            if let Some(arg) = ctx_to_argument(ctx) {
                                let owner =
                                    current_type.as_ref().or(current_virtual.as_ref()).cloned();
                                if let Some(owner) = owner {
                                    config
                                        .type_configs
                                        .entry(owner)
                                        .or_default()
                                        .arguments
                                        .push(arg);
                                }
                            }
                        }
                    }
                    "item" if arg_stack.len() >= 2 => {
                        if let Some(child_ctx) = arg_stack.pop() {
                            let child_sort_order = child_ctx.sort_order;
                            if let Some(child_arg) = ctx_to_argument(child_ctx) {
                                if let Some(parent) = arg_stack.last_mut() {
                                    parent.items.push((child_sort_order, child_arg));
                                }
                            }
                        }
                    }
                    _ => {}
                }
                buf.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::Xml(e)),
            _ => {
                buf.clear();
            }
        }
    }

    config.refresh_lookup_indexes();
    Ok(config)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct ArgContext {
    name: String,
    xsi_type: String,
    shared: Option<bool>,
    /// sortOrder attribute on this item element (0 if not specified).
    sort_order: i32,
    text: String,
    /// (sort_order, Argument) pairs for child items — used to stable-sort by sortOrder.
    items: Vec<(i32, Argument)>,
}

fn ctx_to_argument(ctx: ArgContext) -> Option<Argument> {
    let name = ctx.name;
    if name.is_empty() {
        return None;
    }
    let so = ctx.sort_order;
    let arg = match ctx.xsi_type.as_str() {
        "object" => Argument::Object {
            name,
            value: normalize(&ctx.text),
            shared: ctx.shared,
            sort_order: so,
        },
        "string" => Argument::String {
            name,
            value: ctx.text,
            sort_order: so,
        },
        "boolean" => Argument::Boolean {
            name,
            value: ctx.text == "true" || ctx.text == "1",
            sort_order: so,
        },
        // PHP's Number::evaluate() returns the raw XML string (not cast to int/float).
        // var_export() then quotes it as a string in metadata. Use Argument::String so
        // render_scalar outputs it quoted, matching PHP truth.
        "number" => Argument::String {
            name,
            value: ctx.text,
            sort_order: so,
        },
        "null" => Argument::Null {
            name,
            sort_order: so,
        },
        "array" => {
            // Items carry their individual sort_order on the Argument struct.
            // Sorting happens at resolution time (after cross-file merge) in arguments.rs.
            Argument::Array {
                name,
                items: ctx.items.into_iter().map(|(_, arg)| arg).collect(),
                sort_order: so,
            }
        }
        "init_parameter" => Argument::Init {
            name,
            value: ctx.text,
            sort_order: so,
        },
        "const" => Argument::Const {
            name,
            value: ctx.text,
            sort_order: so,
        },
        _ => Argument::String {
            name,
            value: ctx.text,
            sort_order: so,
        },
    };
    Some(arg)
}

fn local_name(name: &[u8]) -> String {
    let s = std::str::from_utf8(name).unwrap_or("");
    if let Some(pos) = s.rfind(':') {
        s[pos + 1..].to_string()
    } else {
        s.to_string()
    }
}

fn parse_attrs(e: &quick_xml::events::BytesStart) -> FxHashMap<String, String> {
    let mut map = FxHashMap::default();
    for attr in e.attributes().flatten() {
        let key = std::str::from_utf8(attr.key.as_ref())
            .unwrap_or("")
            .to_string();
        let val = attr.unescape_value().unwrap_or_default().to_string();
        map.insert(key.clone(), val.clone());
        // Also store by local name for namespace-prefixed attrs (e.g. xsi:type → type)
        if let Some(pos) = key.rfind(':') {
            map.entry(key[pos + 1..].to_string()).or_insert(val);
        }
    }
    map
}

fn normalize(s: &str) -> String {
    s.trim().trim_start_matches('\\').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_preference() {
        let xml = br#"<?xml version="1.0"?>
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <preference for="Foo\Bar\Interface" type="Foo\Bar\Impl"/>
</config>"#;
        let config = parse_di_xml_bytes(xml).unwrap();
        assert_eq!(
            config
                .preferences
                .get("Foo\\Bar\\Interface")
                .map(String::as_str),
            Some("Foo\\Bar\\Impl")
        );
    }

    #[test]
    fn test_parse_type_plugin() {
        let xml = br#"<?xml version="1.0"?>
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <type name="Foo\Bar">
        <plugin name="myPlugin" type="My\Plugin" sortOrder="10" disabled="false"/>
    </type>
</config>"#;
        let config = parse_di_xml_bytes(xml).unwrap();
        let plugins = config.plugins.get("Foo\\Bar").unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "myPlugin");
        assert_eq!(plugins[0].sort_order, 10);
        assert!(!plugins[0].disabled);
    }

    #[test]
    fn test_parse_disabled_plugin() {
        let xml = br#"<?xml version="1.0"?>
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <type name="Foo\Bar">
        <plugin name="disabledPlugin" type="My\Plugin" disabled="true"/>
    </type>
</config>"#;
        let config = parse_di_xml_bytes(xml).unwrap();
        let plugins = config.plugins.get("Foo\\Bar").unwrap();
        assert!(plugins[0].disabled);
    }

    #[test]
    fn test_parse_virtual_type() {
        let xml = br#"<?xml version="1.0"?>
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <virtualType name="MyVirtual" type="Concrete\Class"/>
</config>"#;
        let config = parse_di_xml_bytes(xml).unwrap();
        assert!(config.virtual_types.contains_key("MyVirtual"));
        assert_eq!(
            config.virtual_types["MyVirtual"].type_name,
            "Concrete\\Class"
        );
    }

    #[test]
    fn test_parse_arguments() {
        let xml = br#"<?xml version="1.0"?>
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <type name="Foo\Bar">
        <arguments>
            <argument name="dep" xsi:type="object">Some\Dep</argument>
            <argument name="label" xsi:type="string">hello</argument>
            <argument name="flag" xsi:type="boolean">true</argument>
            <argument name="nul" xsi:type="null"/>
        </arguments>
    </type>
</config>"#;
        let config = parse_di_xml_bytes(xml).unwrap();
        let tc = config.type_configs.get("Foo\\Bar").unwrap();
        assert_eq!(tc.arguments.len(), 4);
        assert!(
            matches!(&tc.arguments[0], Argument::Object { name, value, .. } if name == "dep" && value == "Some\\Dep")
        );
        assert!(
            matches!(&tc.arguments[1], Argument::String { name, value, .. } if name == "label" && value == "hello")
        );
        assert!(
            matches!(&tc.arguments[2], Argument::Boolean { name, value, .. } if name == "flag" && *value)
        );
        assert!(matches!(&tc.arguments[3], Argument::Null { name, .. } if name == "nul"));
    }

    #[test]
    fn test_parse_array_argument() {
        let xml = br#"<?xml version="1.0"?>
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <type name="Foo\Bar">
        <arguments>
            <argument name="handlers" xsi:type="array">
                <item name="default" xsi:type="object">Handler\Default</item>
                <item name="fallback" xsi:type="string">fallback_value</item>
            </argument>
        </arguments>
    </type>
</config>"#;
        let config = parse_di_xml_bytes(xml).unwrap();
        let tc = config.type_configs.get("Foo\\Bar").unwrap();
        assert_eq!(tc.arguments.len(), 1);
        if let Argument::Array { items, .. } = &tc.arguments[0] {
            assert_eq!(items.len(), 2);
        } else {
            panic!("Expected array argument");
        }
    }

    #[test]
    fn test_parse_cdata_string_argument() {
        let xml = br#"<?xml version="1.0"?>
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <type name="Foo\Bar">
        <arguments>
            <argument name="clientIdRegex" xsi:type="string"><![CDATA[/[^a-z_\-0-9]/i]]></argument>
        </arguments>
    </type>
</config>"#;
        let config = parse_di_xml_bytes(xml).unwrap();
        let tc = config.type_configs.get("Foo\\Bar").unwrap();
        assert_eq!(tc.arguments.len(), 1);
        assert!(
            matches!(&tc.arguments[0], Argument::String { name, value, .. } if name == "clientIdRegex" && value == "/[^a-z_\\-0-9]/i")
        );
    }

    #[test]
    fn test_parse_real_magento_app_etc_di_xml() {
        let path = std::path::Path::new("/var/www/application/app/etc/di.xml");
        if path.exists() {
            let result = parse_di_xml(path);
            assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
            let config = result.unwrap();
            assert!(!config.preferences.is_empty());
        }
    }
}
