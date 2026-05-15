//! TKT-021: Area config PHP codegen.
//!
//! Generates `generated/metadata/{area}.php` with three top-level sections:
//!   - `arguments`   — FQCN → constructor arg map (from arguments resolver)
//!   - `preferences` — interface → implementation map (from DiConfig)
//!   - `instanceTypes` — virtualType name → concrete type map

use std::borrow::Cow;
use std::fmt::Write as FmtWrite;

use rustc_hash::FxHashMap;

use di_resolver::{ResolvedArg, ResolvedArgValue, ResolvedArrayItem, ResolvedArrayValue};
use di_xml_reader::DiConfig;

use crate::metadata::{escape_php, render_scalar, render_untyped_default};

// Static pad strings eliminate " ".repeat(n) heap allocations in hot serialization loops.
const PADS: &[&str] = &[
    "",               // 0
    " ",              // 1
    "  ",             // 2
    "   ",            // 3
    "    ",           // 4
    "     ",          // 5
    "      ",         // 6
    "       ",        // 7
    "        ",       // 8
    "         ",      // 9
    "          ",     // 10
    "           ",    // 11
    "            ",   // 12
    "             ",  // 13
    "              ", // 14
];

fn spad(n: usize) -> Cow<'static, str> {
    match PADS.get(n) {
        Some(&s) => Cow::Borrowed(s),
        None => Cow::Owned(" ".repeat(n)),
    }
}

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
        &FxHashMap::default(),
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
    generate_area_config_with_overrides(
        args_map,
        &FxHashMap::default(),
        di_config,
        extra_preferences,
        extra_preferences,
    )
}

/// Generate area config while injecting separate override maps:
/// - `args_delta` entries override `args_baseline` entries in the arguments section
///   (avoids a full clone of `args_baseline` in the delta-resolve path).
/// - `preference_overrides` are applied to the preferences section.
/// - `instance_type_overrides` are applied only to resolved instanceTypes targets.
pub fn generate_area_config_with_overrides(
    args_baseline: &FxHashMap<String, Vec<ResolvedArg>>,
    args_delta: &FxHashMap<String, Vec<ResolvedArg>>,
    di_config: &DiConfig,
    preference_overrides: &FxHashMap<String, String>,
    instance_type_overrides: &FxHashMap<String, String>,
) -> String {
    // Pre-allocate: estimate output size from entry counts to avoid String reallocations.
    // ~100 bytes/arg-entry (NULL entries ~50, non-empty ~150), ~60/pref, ~80/VT.
    let cap = (args_baseline.len() + args_delta.len()) * 100
        + (di_config.preferences.len() + preference_overrides.len()) * 60
        + di_config.virtual_types.len() * 80
        + 64;
    let mut out = String::with_capacity(cap);

    out.push_str("<?php return array (\n  'arguments' => \n  array (\n");

    // Section: arguments — union baseline + delta keys, sorted for deterministic output.
    // Delta entries shadow baseline entries (area-specific resolution overrides global).
    let mut sorted_fqcns: Vec<&String> = args_baseline.keys().collect();
    for k in args_delta.keys() {
        if !args_baseline.contains_key(k) {
            sorted_fqcns.push(k);
        }
    }
    sorted_fqcns.sort_unstable();
    for fqcn in sorted_fqcns {
        let args = args_delta.get(fqcn).unwrap_or_else(|| &args_baseline[fqcn]);
        if args.is_empty() {
            // PHP emits NULL for types whose constructor args resolve to nothing.
            write!(out, "    '{}' => NULL,\n", escape_php(fqcn)).unwrap();
            continue;
        }
        write!(out, "    '{}' => \n    array (\n", escape_php(fqcn)).unwrap();
        for arg in args {
            serialize_arg_indent(&mut out, &arg.name, &arg.resolved, 6);
        }
        out.push_str("    ),\n");
    }
    out.push_str("  ),\n");

    // Section: preferences — merge di.xml preferences with interception overrides.
    // Virtual type names must NOT appear here; VTs go in instanceTypes.
    out.push_str("  'preferences' => \n  array (\n");
    let mut merged_preferences = di_config.preferences.clone();
    for (from, to) in preference_overrides {
        merged_preferences.insert(from.clone(), to.clone());
    }
    let mut sorted_prefs: Vec<(&String, &String)> = merged_preferences.iter().collect();
    sorted_prefs.sort_unstable_by_key(|(k, _)| k.as_str());
    for (from, to) in sorted_prefs {
        write!(out, "    '{}' => '{}',\n", escape_php(from), escape_php(to)).unwrap();
    }
    out.push_str("  ),\n");

    // Section: instanceTypes — fully-resolved VT chains (VT → VT → Concrete → Concrete).
    out.push_str("  'instanceTypes' => \n  array (\n");
    let mut sorted_vt: Vec<&String> = di_config.virtual_types.keys().collect();
    sorted_vt.sort_unstable();
    for name in sorted_vt {
        let mut concrete = di_config.virtual_types[name].type_name.as_str();
        let mut steps = 0;
        while let Some(vt) = di_config.virtual_types.get(concrete) {
            concrete = vt.type_name.as_str();
            steps += 1;
            if steps > 64 {
                break;
            }
        }
        let resolved = instance_type_overrides
            .get(concrete.trim_start_matches('\\'))
            .map(|s| s.as_str())
            .unwrap_or(concrete);
        write!(
            out,
            "    '{}' => '{}',\n",
            escape_php(name),
            escape_php(resolved)
        )
        .unwrap();
    }
    out.push_str("  ),\n);\n");
    out
}

fn serialize_arg_indent(out: &mut String, name: &str, value: &ResolvedArgValue, indent: usize) {
    use ResolvedArgValue::*;
    let p = spad(indent);
    if let GlobalArgRef { arg_name, default } = value {
        let ds = match default {
            Some(d) => render_untyped_default(d),
            None => "NULL".to_string(),
        };
        write!(
            out,
            "{}'{}' => \n{}array (\n{}  '_a_' => '{}',\n{}  '_d_' => {},\n{}),\n",
            p,
            escape_php(name),
            p,
            p,
            escape_php(arg_name),
            p,
            ds,
            p
        )
        .unwrap();
        return;
    }
    write!(out, "{}'{}' => \n{}array (\n", p, escape_php(name), p).unwrap();
    match value {
        SharedInstance(fqcn) => write!(out, "{}  '_i_' => '{}',\n", p, escape_php(fqcn)).unwrap(),
        NonSharedInstance(fqcn) => {
            write!(out, "{}  '_ins_' => '{}',\n", p, escape_php(fqcn)).unwrap()
        }
        Scalar(val) => write!(out, "{}  '_v_' => {},\n", p, render_scalar(val)).unwrap(),
        Null => write!(out, "{}  '_vn_' => true,\n", p).unwrap(),
        Array(items) => {
            write!(out, "{}  '_vac_' => \n{}  array (\n", p, p).unwrap();
            for item in items {
                serialize_vac_entry(out, &item.name, &item.resolved, indent + 4);
            }
            write!(out, "{}  ),\n", p).unwrap();
        }
        PlainArray(items) => {
            write!(out, "{}  '_v_' => \n{}  array (\n", p, p).unwrap();
            serialize_plain_array_items(out, items, indent + 4);
            write!(out, "{}  ),\n", p).unwrap();
        }
        GlobalArgRef { .. } => unreachable!(),
    }
    write!(out, "{}),\n", p).unwrap();
}

fn serialize_vac_entry(out: &mut String, name: &str, value: &ResolvedArgValue, indent: usize) {
    use ResolvedArgValue::*;
    let p = spad(indent);
    match value {
        SharedInstance(fqcn) => write!(
            out,
            "{}'{}' => \n{}array (\n{}  '_i_' => '{}',\n{}),\n",
            p,
            escape_php(name),
            p,
            p,
            escape_php(fqcn),
            p
        )
        .unwrap(),
        NonSharedInstance(fqcn) => write!(
            out,
            "{}'{}' => \n{}array (\n{}  '_ins_' => '{}',\n{}),\n",
            p,
            escape_php(name),
            p,
            p,
            escape_php(fqcn),
            p
        )
        .unwrap(),
        GlobalArgRef { arg_name, default } => {
            let ds = match default {
                Some(d) => render_untyped_default(d),
                None => "NULL".to_string(),
            };
            write!(
                out,
                "{}'{}' => \n{}array (\n{}  '_a_' => '{}',\n{}  '_d_' => {},\n{}),\n",
                p,
                escape_php(name),
                p,
                p,
                escape_php(arg_name),
                p,
                ds,
                p
            )
            .unwrap();
        }
        Scalar(val) => write!(
            out,
            "{}'{}' => {},\n",
            p,
            escape_php(name),
            render_scalar(val)
        )
        .unwrap(),
        Null => write!(out, "{}'{}' => NULL,\n", p, escape_php(name)).unwrap(),
        Array(items) => {
            write!(out, "{}'{}' => \n{}array (\n", p, escape_php(name), p).unwrap();
            for item in items {
                serialize_vac_entry(out, &item.name, &item.resolved, indent + 2);
            }
            write!(out, "{}),\n", p).unwrap();
        }
        PlainArray(items) => {
            write!(out, "{}'{}' => \n{}array (\n", p, escape_php(name), p).unwrap();
            serialize_plain_array_items(out, items, indent + 2);
            write!(out, "{}),\n", p).unwrap();
        }
    }
}

fn serialize_plain_array_items(out: &mut String, items: &[ResolvedArrayItem], indent: usize) {
    let p = spad(indent);
    for item in items {
        write!(out, "{}'{}' => ", p, escape_php(&item.name)).unwrap();
        serialize_plain_array_value(out, &item.value, indent);
        out.push_str(",\n");
    }
}

fn serialize_plain_array_value(out: &mut String, value: &ResolvedArrayValue, indent: usize) {
    let p = spad(indent);
    match value {
        ResolvedArrayValue::Scalar(s) => out.push_str(&render_scalar(s)),
        ResolvedArrayValue::Null => out.push_str("NULL"),
        ResolvedArrayValue::Array(items) => {
            write!(out, "\n{}array (\n", p).unwrap();
            serialize_plain_array_items(out, items, indent + 2);
            write!(out, "{})", p).unwrap();
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
