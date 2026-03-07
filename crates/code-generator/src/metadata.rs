//! TKT-016: Metadata PHP serializer.
//!
//! Generates:
//!   - `generated/metadata/{area}.php`  — arguments map per area
//!   - `generated/metadata/interception.php` — full FQCN → bool map (all classes)
//!
//! Output format uses PHP's `var_export()` style:
//!   ```php
//!   <?php return array (
//!     'key' => value,
//!   );
//!   ```
//!
//! Argument notation (from Magento's ArgumentsResolver.php):
//!   Shared instance   → `['_i_'   => 'FQN']`
//!   Non-shared        → `['_ins_' => 'FQN']`
//!   Scalar            → `['_v_'   => value]`
//!   Null              → `['_vn_'  => true]``
//!   Array             → `['_vac_' => [...]]`
//!   Global arg ref    → `['_a_'   => 'name', '_d_' => default]`

use std::collections::HashMap;

use di_resolver::{
    ResolvedArg, ResolvedArgValue, ResolvedArrayItem, ResolvedArrayValue, ResolvedScalar,
};

/// Serialize a `<?php return array (...);` arguments map.
///
/// `args_map`: FQCN → Vec<ResolvedArg>
pub fn serialize_arguments_php(args_map: &HashMap<String, Vec<ResolvedArg>>) -> String {
    let mut out = String::from("<?php return array (\n");

    let mut sorted_keys: Vec<&String> = args_map.keys().collect();
    sorted_keys.sort();

    for fqcn in sorted_keys {
        let args = &args_map[fqcn];
        if args.is_empty() {
            continue;
        }
        out.push_str(&format!("  '{}' => \n  array (\n", escape_php(fqcn)));
        for arg in args {
            serialize_resolved_arg(&mut out, &arg.name, &arg.resolved, 4);
        }
        out.push_str("  ),\n");
    }

    out.push_str(");\n");
    out
}

/// Serialize the interception.php file: FQCN → bool (true = intercepted).
pub fn serialize_interception_php(all_fqcns: &HashMap<String, bool>) -> String {
    let mut out = String::from("<?php return array (\n");

    let mut sorted: Vec<(&String, bool)> = all_fqcns.iter().map(|(k, &v)| (k, v)).collect();
    sorted.sort_by_key(|(k, _)| k.as_str());

    for (fqcn, intercepted) in sorted {
        let val = if intercepted { "true" } else { "false" };
        out.push_str(&format!("  '{}' => {},\n", escape_php(fqcn), val));
    }

    out.push_str(");\n");
    out
}

fn serialize_resolved_arg(out: &mut String, name: &str, value: &ResolvedArgValue, indent: usize) {
    let pad = " ".repeat(indent);
    out.push_str(&format!(
        "{}'{}' => \n{}array (\n",
        pad,
        escape_php(name),
        pad
    ));
    match value {
        ResolvedArgValue::SharedInstance(fqcn) => {
            out.push_str(&format!("{}  '_i_' => '{}',\n", pad, escape_php(fqcn)));
        }
        ResolvedArgValue::NonSharedInstance(fqcn) => {
            out.push_str(&format!("{}  '_ins_' => '{}',\n", pad, escape_php(fqcn)));
        }
        ResolvedArgValue::Scalar(val) => {
            out.push_str(&format!("{}  '_v_' => {},\n", pad, render_scalar(val)));
        }
        ResolvedArgValue::Null => {
            out.push_str(&format!("{}  '_vn_' => true,\n", pad));
        }
        ResolvedArgValue::Array(items) => {
            out.push_str(&format!("{}  '_vac_' => \n{}  array (\n", pad, pad));
            for item in items {
                serialize_vac_entry(out, &item.name, &item.resolved, indent + 4);
            }
            out.push_str(&format!("{}  ),\n", pad));
        }
        ResolvedArgValue::PlainArray(items) => {
            out.push_str(&format!("{}  '_v_' => \n{}  array (\n", pad, pad));
            serialize_plain_array_items(out, items, indent + 4);
            out.push_str(&format!("{}  ),\n", pad));
        }
        ResolvedArgValue::GlobalArgRef { arg_name, default } => {
            out.push_str(&format!("{}  '_a_' => '{}',\n", pad, escape_php(arg_name)));
            let default_str = match default {
                Some(d) => render_untyped_default(d),
                None => "NULL".to_string(),
            };
            out.push_str(&format!("{}  '_d_' => {},\n", pad, default_str));
        }
    }
    out.push_str(&format!("{}),\n", pad));
}

/// Serialize one key/value entry inside a `_vac_` (configured array) block.
///
/// Inside `_vac_`, Magento's ArgumentsResolver stores values differently from
/// top-level constructor args:
///   - Instance refs:  `array('_i_' => 'FQN')`  (same notation as top-level)
///   - Global arg refs: `array('_a_' => ..., '_d_' => ...)`  (same)
///   - Scalars:  **plain PHP value** — NOT wrapped in `array('_v_' => ...)`
///   - Nulls:    **plain `NULL`**   — NOT wrapped in `array('_vn_' => true)`
///   - Nested arrays: **flat `array(...)`** — NOT wrapped in `array('_vac_' => ...)`
///
/// This matches PHP `ArgumentsResolver::getConfiguredArrayAttribute()`.
fn serialize_vac_entry(out: &mut String, name: &str, value: &ResolvedArgValue, indent: usize) {
    let pad = " ".repeat(indent);
    match value {
        ResolvedArgValue::SharedInstance(fqcn) => {
            out.push_str(&format!(
                "{}'{}' => \n{}array (\n{}  '_i_' => '{}',\n{}),\n",
                pad, escape_php(name), pad, pad, escape_php(fqcn), pad
            ));
        }
        ResolvedArgValue::NonSharedInstance(fqcn) => {
            out.push_str(&format!(
                "{}'{}' => \n{}array (\n{}  '_ins_' => '{}',\n{}),\n",
                pad, escape_php(name), pad, pad, escape_php(fqcn), pad
            ));
        }
        ResolvedArgValue::GlobalArgRef { arg_name, default } => {
            out.push_str(&format!(
                "{}'{}' => \n{}array (\n{}  '_a_' => '{}',\n",
                pad, escape_php(name), pad, pad, escape_php(arg_name)
            ));
            let default_str = match default {
                Some(d) => render_untyped_default(d),
                None => "NULL".to_string(),
            };
            out.push_str(&format!("{}  '_d_' => {},\n{}),\n", pad, default_str, pad));
        }
        ResolvedArgValue::Scalar(val) => {
            // Plain scalar — no _v_ wrapper inside _vac_
            out.push_str(&format!("{}'{}' => {},\n", pad, escape_php(name), render_scalar(val)));
        }
        ResolvedArgValue::Null => {
            // Plain NULL — no _vn_ wrapper inside _vac_
            out.push_str(&format!("{}'{}' => NULL,\n", pad, escape_php(name)));
        }
        ResolvedArgValue::Array(items) => {
            // Nested configured array — flat, no _vac_ re-wrap
            out.push_str(&format!("{}'{}' => \n{}array (\n", pad, escape_php(name), pad));
            for item in items {
                serialize_vac_entry(out, &item.name, &item.resolved, indent + 2);
            }
            out.push_str(&format!("{}),\n", pad));
        }
        ResolvedArgValue::PlainArray(items) => {
            out.push_str(&format!("{}'{}' => \n{}array (\n", pad, escape_php(name), pad));
            serialize_plain_array_items(out, items, indent + 2);
            out.push_str(&format!("{}),\n", pad));
        }
    }
}

fn serialize_plain_array_items(out: &mut String, items: &[ResolvedArrayItem], indent: usize) {
    let pad = " ".repeat(indent);
    for item in items {
        out.push_str(&format!("{}'{}' => ", pad, escape_php(&item.name)));
        serialize_plain_array_value(out, &item.value, indent);
        out.push_str(",\n");
    }
}

fn serialize_plain_array_value(out: &mut String, value: &ResolvedArrayValue, indent: usize) {
    let pad = " ".repeat(indent);
    match value {
        ResolvedArrayValue::Scalar(s) => out.push_str(&render_scalar(s)),
        ResolvedArrayValue::Null => out.push_str("NULL"),
        ResolvedArrayValue::Array(items) => {
            out.push_str("\n");
            out.push_str(&format!("{}array (\n", pad));
            serialize_plain_array_items(out, items, indent + 2);
            out.push_str(&format!("{})", pad));
        }
    }
}

/// Escape a PHP string value (backslash and single-quote).
pub fn escape_php(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

pub(crate) fn render_scalar(val: &ResolvedScalar) -> String {
    match val {
        ResolvedScalar::String(v) => format!("'{}'", escape_php(v)),
        ResolvedScalar::Bool(v) => {
            if *v {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        ResolvedScalar::Number(v) => {
            if is_safe_php_numeric_literal(v) {
                v.clone()
            } else {
                format!("'{}'", escape_php(v))
            }
        }
    }
}

pub(crate) fn render_untyped_default(default: &str) -> String {
    match default {
        "true" => "true".to_string(),
        "false" => "false".to_string(),
        "NULL" | "null" => "NULL".to_string(),
        // Empty PHP array default: PHP reflection returns [], var_export gives array ()
        "[]" => "array ()".to_string(),
        _ if is_safe_php_numeric_literal(default) => default.to_string(),
        _ => format!("'{}'", escape_php(default)),
    }
}

fn is_safe_php_numeric_literal(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }

    // PHP accepts decimal numbers and scientific notation; reject numeric
    // strings with leading zeros (e.g. 0123) because they can be parsed as
    // octal/invalid literals depending on digits.
    let (negative, rest) = if let Some(r) = s.strip_prefix('-') {
        (true, r)
    } else if let Some(r) = s.strip_prefix('+') {
        (false, r)
    } else {
        (false, s)
    };

    if rest.is_empty() || !rest.parse::<f64>().is_ok() {
        return false;
    }
    if rest.eq_ignore_ascii_case("inf") || rest.eq_ignore_ascii_case("nan") {
        return false;
    }

    let _ = negative; // sign does not affect literal safety checks below

    if let Some(int_part) = rest.split(['.', 'e', 'E']).next() {
        if int_part.len() > 1 && int_part.starts_with('0') {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use di_resolver::{ResolvedArgValue, ResolvedScalar};

    #[test]
    fn test_interception_php_format() {
        let mut map = HashMap::new();
        map.insert("Foo\\Bar".to_string(), false);
        map.insert("Foo\\Intercepted".to_string(), true);
        let out = serialize_interception_php(&map);
        assert!(out.starts_with("<?php return array (\n"));
        assert!(out.contains("'Foo\\\\Bar' => false,"));
        assert!(out.contains("'Foo\\\\Intercepted' => true,"));
        assert!(out.ends_with(");\n"));
    }

    #[test]
    fn test_shared_instance() {
        let mut map = HashMap::new();
        map.insert(
            "App\\Service".to_string(),
            vec![ResolvedArg {
                name: "logger".to_string(),
                resolved: ResolvedArgValue::SharedInstance("App\\Logger".to_string()),
            }],
        );
        let out = serialize_arguments_php(&map);
        assert!(out.contains("'_i_' => 'App\\\\Logger'"));
    }

    #[test]
    fn test_null_arg() {
        let mut map = HashMap::new();
        map.insert(
            "App\\Service".to_string(),
            vec![ResolvedArg {
                name: "opt".to_string(),
                resolved: ResolvedArgValue::Null,
            }],
        );
        let out = serialize_arguments_php(&map);
        assert!(out.contains("'_vn_' => true,"));
    }

    #[test]
    fn test_arg_closes_with_comma_not_dot() {
        // Regression: array arg closing must be `),` not `).`
        let mut map = HashMap::new();
        map.insert(
            "App\\Service".to_string(),
            vec![ResolvedArg {
                name: "x".to_string(),
                resolved: ResolvedArgValue::SharedInstance("App\\X".to_string()),
            }],
        );
        let out = serialize_arguments_php(&map);
        assert!(!out.contains(")."), "closing must be ), not ).");
        assert!(out.contains("),"));
    }

    #[test]
    fn test_scalar_string_numeric_stays_quoted() {
        let mut map = HashMap::new();
        map.insert(
            "App\\Service".to_string(),
            vec![ResolvedArg {
                name: "num".to_string(),
                resolved: ResolvedArgValue::Scalar(ResolvedScalar::String(
                    "0123456789".to_string(),
                )),
            }],
        );
        let out = serialize_arguments_php(&map);
        assert!(out.contains("'_v_' => '0123456789'"));
    }

    #[test]
    fn test_scalar_number_renders_unquoted_when_safe() {
        let mut map = HashMap::new();
        map.insert(
            "App\\Service".to_string(),
            vec![ResolvedArg {
                name: "count".to_string(),
                resolved: ResolvedArgValue::Scalar(ResolvedScalar::Number("10".to_string())),
            }],
        );
        let out = serialize_arguments_php(&map);
        assert!(out.contains("'_v_' => 10,"));
    }

    #[test]
    fn test_scalar_number_with_leading_zero_is_quoted() {
        let mut map = HashMap::new();
        map.insert(
            "App\\Service".to_string(),
            vec![ResolvedArg {
                name: "bad".to_string(),
                resolved: ResolvedArgValue::Scalar(ResolvedScalar::Number(
                    "0123456789".to_string(),
                )),
            }],
        );
        let out = serialize_arguments_php(&map);
        assert!(out.contains("'_v_' => '0123456789'"));
    }

    #[test]
    fn test_scalar_bool_renders_unquoted() {
        let mut map = HashMap::new();
        map.insert(
            "App\\Service".to_string(),
            vec![ResolvedArg {
                name: "enabled".to_string(),
                resolved: ResolvedArgValue::Scalar(ResolvedScalar::Bool(true)),
            }],
        );
        let out = serialize_arguments_php(&map);
        assert!(out.contains("'_v_' => true,"));
    }

    #[test]
    fn test_configured_array_nested_entries_are_flat_inside_vac() {
        let mut map = HashMap::new();
        map.insert(
            "Magento\\Framework\\App\\Config\\ConfigSourceAggregated".to_string(),
            vec![ResolvedArg {
                name: "sources".to_string(),
                resolved: ResolvedArgValue::Array(vec![ResolvedArg {
                    name: "modular".to_string(),
                    resolved: ResolvedArgValue::Array(vec![
                        ResolvedArg {
                            name: "source".to_string(),
                            resolved: ResolvedArgValue::SharedInstance(
                                "Magento\\Config\\App\\Config\\Source\\ModularConfigSource"
                                    .to_string(),
                            ),
                        },
                        ResolvedArg {
                            name: "sortOrder".to_string(),
                            resolved: ResolvedArgValue::Scalar(ResolvedScalar::String(
                                "10".to_string(),
                            )),
                        },
                    ]),
                }]),
            }],
        );

        let out = serialize_arguments_php(&map);

        assert!(
            out.contains("'modular' =>"),
            "configured array entries must stay flat inside _vac_"
        );
        assert!(
            out.contains("'sortOrder' => '10',"),
            "scalar values inside _vac_ must not use _v_ wrapper"
        );
        assert!(
            !out.contains("'modular' => \n      array (\n        '_vac_' =>"),
            "nested configured arrays must not re-wrap with _vac_"
        );
        assert!(
            !out.contains("'sortOrder' => \n          array (\n            '_v_' => '10',"),
            "nested scalar values must not re-wrap with _v_"
        );
    }

    #[test]
    fn test_global_arg_ref_always_emits_default_key() {
        let mut map = HashMap::new();
        map.insert(
            "App\\Service".to_string(),
            vec![ResolvedArg {
                name: "mode".to_string(),
                resolved: ResolvedArgValue::GlobalArgRef {
                    arg_name: "MAGE_MODE".to_string(),
                    default: None,
                },
            }],
        );
        let out = serialize_arguments_php(&map);
        assert!(out.contains("'_a_' => 'MAGE_MODE'"));
        assert!(out.contains("'_d_' => NULL,"));
    }

    #[test]
    fn test_configured_array_global_arg_ref_emits_default_key() {
        let mut map = HashMap::new();
        map.insert(
            "App\\Service".to_string(),
            vec![ResolvedArg {
                name: "nested".to_string(),
                resolved: ResolvedArgValue::Array(vec![ResolvedArg {
                    name: "mode".to_string(),
                    resolved: ResolvedArgValue::GlobalArgRef {
                        arg_name: "MAGE_MODE".to_string(),
                        default: None,
                    },
                }]),
            }],
        );
        let out = serialize_arguments_php(&map);
        assert!(out.contains("'mode' =>"));
        assert!(out.contains("'_a_' => 'MAGE_MODE'"));
        assert!(out.contains("'_d_' => NULL,"));
    }
}
