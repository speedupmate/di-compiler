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

use di_resolver::{ResolvedArg, ResolvedArgValue};

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
    out.push_str(&format!("{}'{}' => \n{}array (\n", pad, escape_php(name), pad));
    match value {
        ResolvedArgValue::SharedInstance(fqcn) => {
            out.push_str(&format!(
                "{}  '_i_' => '{}',\n",
                pad,
                escape_php(fqcn)
            ));
        }
        ResolvedArgValue::NonSharedInstance(fqcn) => {
            out.push_str(&format!(
                "{}  '_ins_' => '{}',\n",
                pad,
                escape_php(fqcn)
            ));
        }
        ResolvedArgValue::Scalar(val) => {
            // Numeric scalars are not quoted; strings are
            if is_numeric(val) {
                out.push_str(&format!("{}  '_v_' => {},\n", pad, val));
            } else {
                out.push_str(&format!("{}  '_v_' => '{}',\n", pad, escape_php(val)));
            }
        }
        ResolvedArgValue::Null => {
            out.push_str(&format!("{}  '_vn_' => true,\n", pad));
        }
        ResolvedArgValue::Array(items) => {
            out.push_str(&format!("{}  '_vac_' => \n{}  array (\n", pad, pad));
            for item in items {
                serialize_resolved_arg(out, &item.name, &item.resolved, indent + 4);
            }
            out.push_str(&format!("{}  ),\n", pad));
        }
        ResolvedArgValue::GlobalArgRef { arg_name, default } => {
            out.push_str(&format!("{}  '_a_' => '{}',\n", pad, escape_php(arg_name)));
            if let Some(default) = default {
                if is_numeric(default) {
                    out.push_str(&format!("{}  '_d_' => {},\n", pad, default));
                } else {
                    out.push_str(&format!(
                        "{}  '_d_' => '{}',\n",
                        pad,
                        escape_php(default)
                    ));
                }
            }
        }
    }
    out.push_str(&format!("{}).\n", pad));
}

/// Escape a PHP string value (backslash and single-quote).
pub fn escape_php(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

fn is_numeric(s: &str) -> bool {
    s.parse::<i64>().is_ok() || s.parse::<f64>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use di_resolver::ResolvedArgValue;

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
}
