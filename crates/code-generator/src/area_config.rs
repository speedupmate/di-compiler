//! TKT-021: Area config PHP codegen.
//!
//! Generates `generated/metadata/{area}.php` with three top-level sections:
//!   - `arguments`   — FQCN → constructor arg map (from arguments resolver)
//!   - `preferences` — interface → implementation map (from DiConfig)
//!   - `instanceTypes` — virtualType name → concrete type map

use std::collections::HashMap;

use di_resolver::ResolvedArg;
use di_xml_reader::DiConfig;

use crate::metadata::escape_php;

/// Generate the PHP source for one area config file.
///
/// `args_map`: FQCN → constructor args (from arguments resolver).
/// `di_config`: merged DI configuration for the area.
pub fn generate_area_config(
    args_map: &HashMap<String, Vec<ResolvedArg>>,
    di_config: &DiConfig,
) -> String {
    let mut out = String::from("<?php return array (\n");

    // Section: arguments
    out.push_str("  'arguments' => \n  array (\n");
    let mut sorted_fqcns: Vec<&String> = args_map.keys().collect();
    sorted_fqcns.sort();
    for fqcn in sorted_fqcns {
        let args = &args_map[fqcn];
        if args.is_empty() {
            continue;
        }
        out.push_str(&format!("    '{}' => \n    array (\n", escape_php(fqcn)));
        for arg in args {
            serialize_arg_indent(&mut out, &arg.name, &arg.resolved, 6);
        }
        out.push_str("    ),\n");
    }
    out.push_str("  ),\n");

    // Section: preferences
    out.push_str("  'preferences' => \n  array (\n");
    let mut sorted_prefs: Vec<(&String, &String)> = di_config.preferences.iter().collect();
    sorted_prefs.sort_by_key(|(k, _)| k.as_str());
    for (from, to) in sorted_prefs {
        out.push_str(&format!(
            "    '{}' => '{}',\n",
            escape_php(from),
            escape_php(to)
        ));
    }
    out.push_str("  ),\n");

    // Section: instanceTypes (virtualTypes)
    out.push_str("  'instanceTypes' => \n  array (\n");
    let mut sorted_vt: Vec<(&String, &di_xml_reader::VirtualType)> =
        di_config.virtual_types.iter().collect();
    sorted_vt.sort_by_key(|(k, _)| k.as_str());
    for (name, vt) in sorted_vt {
        out.push_str(&format!(
            "    '{}' => '{}',\n",
            escape_php(name),
            escape_php(&vt.type_name)
        ));
    }
    out.push_str("  ),\n");

    out.push_str(");\n");
    out
}

fn serialize_arg_indent(
    out: &mut String,
    name: &str,
    value: &di_resolver::ResolvedArgValue,
    indent: usize,
) {
    use di_resolver::ResolvedArgValue::*;
    let pad = " ".repeat(indent);
    out.push_str(&format!("{}'{}' => \n{}array (\n", pad, escape_php(name), pad));
    match value {
        SharedInstance(fqcn) => {
            out.push_str(&format!("{}  '_i_' => '{}',\n", pad, escape_php(fqcn)));
        }
        NonSharedInstance(fqcn) => {
            out.push_str(&format!("{}  '_ins_' => '{}',\n", pad, escape_php(fqcn)));
        }
        Scalar(val) => {
            if is_numeric(val) {
                out.push_str(&format!("{}  '_v_' => {},\n", pad, val));
            } else {
                out.push_str(&format!("{}  '_v_' => '{}',\n", pad, escape_php(val)));
            }
        }
        Null => {
            out.push_str(&format!("{}  '_vn_' => true,\n", pad));
        }
        Array(items) => {
            out.push_str(&format!("{}  '_vac_' => \n{}  array (\n", pad, pad));
            for item in items {
                serialize_arg_indent(out, &item.name, &item.resolved, indent + 4);
            }
            out.push_str(&format!("{}  ),\n", pad));
        }
        GlobalArgRef { arg_name, default } => {
            out.push_str(&format!("{}  '_a_' => '{}',\n", pad, escape_php(arg_name)));
            if let Some(d) = default {
                if is_numeric(d) {
                    out.push_str(&format!("{}  '_d_' => {},\n", pad, d));
                } else {
                    out.push_str(&format!("{}  '_d_' => '{}',\n", pad, escape_php(d)));
                }
            }
        }
    }
    out.push_str(&format!("{}),\n", pad));
}

fn is_numeric(s: &str) -> bool {
    s.parse::<i64>().is_ok() || s.parse::<f64>().is_ok()
}

/// Standard area names in Magento DI.
pub const AREAS: &[&str] = &[
    "global",
    "frontend",
    "adminhtml",
    "crontab",
    "webapi_rest",
    "webapi_soap",
    "graphql",
];

#[cfg(test)]
mod tests {
    use super::*;
    use di_xml_reader::DiConfig;
    use std::collections::HashMap;

    #[test]
    fn test_empty_area_config() {
        let out = generate_area_config(&HashMap::new(), &DiConfig::default());
        assert!(out.starts_with("<?php return array (\n"));
        assert!(out.contains("'arguments'"));
        assert!(out.contains("'preferences'"));
        assert!(out.contains("'instanceTypes'"));
        assert!(out.ends_with(");\n"));
    }

    #[test]
    fn test_preferences_in_area_config() {
        let mut di_config = DiConfig::default();
        di_config
            .preferences
            .insert("Foo\\Interface".to_string(), "Foo\\Impl".to_string());
        let out = generate_area_config(&HashMap::new(), &di_config);
        assert!(out.contains("'Foo\\\\Interface' => 'Foo\\\\Impl'"));
    }
}
