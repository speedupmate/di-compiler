//! TKT-021: Area config PHP codegen.
//!
//! Generates `generated/metadata/{area}.php` with three top-level sections:
//!   - `arguments`   — FQCN → constructor arg map (from arguments resolver)
//!   - `preferences` — interface → implementation map (from DiConfig)
//!   - `instanceTypes` — virtualType name → concrete type map

use rustc_hash::FxHashMap;

use di_resolver::{ResolvedArg, ResolvedArgValue, ResolvedArrayItem, ResolvedArrayValue};
use di_xml_reader::DiConfig;

use crate::metadata::{escape_php, render_scalar, render_untyped_default};

/// Generate the PHP source for one area config file.
///
/// `args_map`: FQCN → constructor args (from arguments resolver).
/// `di_config`: merged DI configuration for the area.
pub fn generate_area_config(
    args_map: &FxHashMap<String, Vec<ResolvedArg>>,
    di_config: &DiConfig,
) -> String {
    generate_area_config_with_overrides(
        args_map,
        di_config,
        &FxHashMap::default(),
        &FxHashMap::default(),
    )
}

/// Generate area config while injecting additional preferences that should
/// appear in metadata output (for example interception preferences).
pub fn generate_area_config_with_extra_preferences(
    args_map: &FxHashMap<String, Vec<ResolvedArg>>,
    di_config: &DiConfig,
    extra_preferences: &FxHashMap<String, String>,
) -> String {
    generate_area_config_with_overrides(args_map, di_config, extra_preferences, extra_preferences)
}

/// Generate area config while injecting separate override maps:
/// - `preference_overrides` are applied to the preferences section.
/// - `instance_type_overrides` are applied only to resolved instanceTypes targets.
pub fn generate_area_config_with_overrides(
    args_map: &FxHashMap<String, Vec<ResolvedArg>>,
    di_config: &DiConfig,
    preference_overrides: &FxHashMap<String, String>,
    instance_type_overrides: &FxHashMap<String, String>,
) -> String {
    let mut out = String::from("<?php return array (\n");

    // Section: arguments
    out.push_str("  'arguments' => \n  array (\n");
    let mut sorted_fqcns: Vec<&String> = args_map.keys().collect();
    sorted_fqcns.sort();
    for fqcn in sorted_fqcns {
        let args = &args_map[fqcn];
        if args.is_empty() {
            // PHP emits NULL for types in the universe whose constructor args resolve to nothing
            // (class doesn't exist on disk, has no constructor, or all args are defaults).
            out.push_str(&format!("    '{}' => NULL,\n", escape_php(fqcn)));
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
    // Merge di.xml preferences with interception preferences (extra_preferences maps
    // Concrete → Concrete\Interceptor). Also add VTs whose DIRECT type is an intercepted
    // concrete — they get a preference entry pointing to the Interceptor.
    out.push_str("  'preferences' => \n  array (\n");
    let mut merged_preferences = di_config.preferences.clone();
    for (from, to) in preference_overrides {
        merged_preferences.insert(from.clone(), to.clone());
    }
    // NOTE: Virtual type names must NOT appear in the preferences section.
    // PHP only maps concrete class → ClassName\Interceptor in preferences.
    // VTs are resolved via instanceTypes, not preferences.
    let mut sorted_prefs: Vec<(&String, &String)> = merged_preferences.iter().collect();
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
    // Config\Compiled::getInstanceType only follows ONE hop, so we must fully resolve
    // VT chains here (VT → VT → Concrete becomes VT → Concrete).
    out.push_str("  'instanceTypes' => \n  array (\n");
    let mut sorted_vt: Vec<&String> = di_config.virtual_types.keys().collect();
    sorted_vt.sort();
    for name in sorted_vt {
        // Follow VT chain to the final concrete type
        let mut concrete = di_config.virtual_types[name].type_name.as_str();
        let mut steps = 0;
        while let Some(vt) = di_config.virtual_types.get(concrete) {
            concrete = vt.type_name.as_str();
            steps += 1;
            if steps > 64 {
                break;
            } // guard against cycles
        }
        // If the resolved concrete is intercepted, point to its Interceptor.
        let resolved = instance_type_overrides
            .get(concrete.trim_start_matches('\\'))
            .map(|s| s.as_str())
            .unwrap_or(concrete);
        out.push_str(&format!(
            "    '{}' => '{}',\n",
            escape_php(name),
            escape_php(resolved)
        ));
    }
    out.push_str("  ),\n");

    out.push_str(");\n");
    out
}

fn serialize_arg_indent(out: &mut String, name: &str, value: &ResolvedArgValue, indent: usize) {
    use ResolvedArgValue::*;
    let pad = " ".repeat(indent);
    out.push_str(&format!(
        "{}'{}' => \n{}array (\n",
        pad,
        escape_php(name),
        pad
    ));
    match value {
        SharedInstance(fqcn) => {
            out.push_str(&format!("{}  '_i_' => '{}',\n", pad, escape_php(fqcn)));
        }
        NonSharedInstance(fqcn) => {
            out.push_str(&format!("{}  '_ins_' => '{}',\n", pad, escape_php(fqcn)));
        }
        Scalar(val) => {
            out.push_str(&format!("{}  '_v_' => {},\n", pad, render_scalar(val)));
        }
        Null => {
            out.push_str(&format!("{}  '_vn_' => true,\n", pad));
        }
        Array(items) => {
            out.push_str(&format!("{}  '_vac_' => \n{}  array (\n", pad, pad));
            for item in items {
                serialize_vac_entry(out, &item.name, &item.resolved, indent + 4);
            }
            out.push_str(&format!("{}  ),\n", pad));
        }
        PlainArray(items) => {
            out.push_str(&format!("{}  '_v_' => \n{}  array (\n", pad, pad));
            serialize_plain_array_items(out, items, indent + 4);
            out.push_str(&format!("{}  ),\n", pad));
        }
        GlobalArgRef { arg_name, default } => {
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

fn serialize_vac_entry(out: &mut String, name: &str, value: &ResolvedArgValue, indent: usize) {
    use ResolvedArgValue::*;
    let pad = " ".repeat(indent);
    match value {
        SharedInstance(fqcn) => {
            out.push_str(&format!(
                "{}'{}' => \n{}array (\n{}  '_i_' => '{}',\n{}),\n",
                pad,
                escape_php(name),
                pad,
                pad,
                escape_php(fqcn),
                pad
            ));
        }
        NonSharedInstance(fqcn) => {
            out.push_str(&format!(
                "{}'{}' => \n{}array (\n{}  '_ins_' => '{}',\n{}),\n",
                pad,
                escape_php(name),
                pad,
                pad,
                escape_php(fqcn),
                pad
            ));
        }
        GlobalArgRef { arg_name, default } => {
            out.push_str(&format!(
                "{}'{}' => \n{}array (\n{}  '_a_' => '{}',\n",
                pad,
                escape_php(name),
                pad,
                pad,
                escape_php(arg_name)
            ));
            let default_str = match default {
                Some(d) => render_untyped_default(d),
                None => "NULL".to_string(),
            };
            out.push_str(&format!("{}  '_d_' => {},\n{}),\n", pad, default_str, pad));
        }
        Scalar(val) => {
            out.push_str(&format!(
                "{}'{}' => {},\n",
                pad,
                escape_php(name),
                render_scalar(val)
            ));
        }
        Null => {
            out.push_str(&format!("{}'{}' => NULL,\n", pad, escape_php(name)));
        }
        Array(items) => {
            out.push_str(&format!(
                "{}'{}' => \n{}array (\n",
                pad,
                escape_php(name),
                pad
            ));
            for item in items {
                serialize_vac_entry(out, &item.name, &item.resolved, indent + 2);
            }
            out.push_str(&format!("{}),\n", pad));
        }
        PlainArray(items) => {
            out.push_str(&format!(
                "{}'{}' => \n{}array (\n",
                pad,
                escape_php(name),
                pad
            ));
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
    use di_resolver::{ResolvedArg, ResolvedArgValue, ResolvedScalar};
    use di_xml_reader::DiConfig;
    use rustc_hash::FxHashMap;

    #[test]
    fn test_empty_area_config() {
        let out = generate_area_config(&FxHashMap::default(), &DiConfig::default());
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
        let out = generate_area_config(&FxHashMap::default(), &di_config);
        assert!(out.contains("'Foo\\\\Interface' => 'Foo\\\\Impl'"));
    }

    #[test]
    fn test_nested_configured_arrays_are_flat_in_vac() {
        let mut args = FxHashMap::default();
        args.insert(
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

        let out = generate_area_config(&args, &DiConfig::default());
        assert!(out.contains("'sortOrder' => '10',"));
        assert!(
            !out.contains("'modular' => \n        array (\n          '_vac_' =>"),
            "nested configured arrays must not re-wrap with _vac_"
        );
        assert!(
            !out.contains("'sortOrder' => \n            array (\n              '_v_' => '10',"),
            "nested configured scalars must not re-wrap with _v_"
        );
    }

    #[test]
    fn test_global_arg_ref_always_emits_default_key() {
        let mut args = FxHashMap::default();
        args.insert(
            "App\\Service".to_string(),
            vec![ResolvedArg {
                name: "mode".to_string(),
                resolved: ResolvedArgValue::GlobalArgRef {
                    arg_name: "MAGE_MODE".to_string(),
                    default: None,
                },
            }],
        );
        let out = generate_area_config(&args, &DiConfig::default());
        assert!(out.contains("'_a_' => 'MAGE_MODE'"));
        assert!(out.contains("'_d_' => NULL,"));
    }

    #[test]
    fn test_nested_global_arg_ref_always_emits_default_key() {
        let mut args = FxHashMap::default();
        args.insert(
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
        let out = generate_area_config(&args, &DiConfig::default());
        assert!(out.contains("'mode' =>"));
        assert!(out.contains("'_a_' => 'MAGE_MODE'"));
        assert!(out.contains("'_d_' => NULL,"));
    }
}
