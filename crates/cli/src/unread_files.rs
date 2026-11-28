//! What to do with a file that no tier could read.
//!
//! Magento's `ClassesScanner` compiles the class `FileClassScanner` finds in a file, so for such a
//! file the compile would write code and metadata without its constructor and methods. A passing
//! PHP class check proves only that the file loads. Only a file outside Magento's scan, whose names
//! nothing active uses, can be left out with a warning.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use di_xml_reader::{Argument, DiConfig};
use php_extractor::php_snippets::BOOT_PHP;
use php_extractor::ClassInfo;
use rustc_hash::FxHashMap;
use serde::Deserialize;

/// A file every tier failed on, with each tier's reason.
pub(crate) struct UnreadFile {
    pub(crate) path: PathBuf,
    pub(crate) reason: String,
}

/// What Magento's scanner and PHP's tokenizer say about one file. Neither runs the file.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct FileScope {
    /// `FileClassScanner::getClassName()`; `None` when the store has no scanner.
    scanner: Option<String>,
    scanner_error: Option<String>,
    /// The class-like names the file declares, at any depth.
    names: Option<Vec<String>>,
    names_error: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ScopeOutput {
    Files { files: Vec<FileScope> },
    Error { error: String },
}

const SCOPE_HEAD_PHP: &str = r#"<?php
[, $root, $generated, $list, $result] = $argv;
$finished = false;
register_shutdown_function(static function () use (&$finished, $result): void {
    if ($finished) {
        return;
    }
    while (ob_get_level() > 0) {
        ob_end_clean();
    }
    $last = error_get_last();
    file_put_contents($result, json_encode(['error' => $last === null ? 'The scope check ended the PHP process' : $last['message']]));
});
"#;

const SCOPE_PHP: &str = r#"
$scannerClass = 'Magento\\Setup\\Module\\Di\\Code\\Reader\\FileClassScanner';
$hasScanner = class_exists($scannerClass);
$rows = [];
foreach (json_decode((string) file_get_contents($list), true) as $file) {
    $row = ['scanner' => null, 'scanner_error' => null, 'names' => null, 'names_error' => null];
    if ($hasScanner) {
        try {
            $scan = new $scannerClass($file);
            $row['scanner'] = method_exists($scan, 'getClassName')
                ? $scan->getClassName()
                : (string) (((array) $scan->getClassNames())[0] ?? '');
        } catch (\Throwable $e) {
            $row['scanner_error'] = $e->getMessage();
        }
    }
    try {
        $tokens = PhpToken::tokenize((string) file_get_contents($file), TOKEN_PARSE);
        $significant = array_values(array_filter($tokens, static fn (PhpToken $t): bool => !$t->isIgnorable()));
        $names = [];
        $namespace = '';
        foreach ($significant as $i => $token) {
            $next = $significant[$i + 1] ?? null;
            if ($token->is(T_NAMESPACE)) {
                $namespace = $next !== null && $next->is([T_STRING, T_NAME_QUALIFIED]) ? $next->text : '';
            } elseif ($token->is([T_CLASS, T_INTERFACE, T_TRAIT, T_ENUM])
                && !($i > 0 && $significant[$i - 1]->is([T_DOUBLE_COLON, T_NEW]))
                && $next !== null && $next->is(T_STRING)) {
                $names[] = ($namespace === '' ? '' : $namespace . '\\') . $next->text;
            }
        }
        $row['names'] = $names;
    } catch (\Throwable $e) {
        $row['names_error'] = $e->getMessage();
    }
    $rows[] = $row;
}
$finished = true;
file_put_contents($result, json_encode(['files' => $rows], JSON_INVALID_UTF8_SUBSTITUTE | JSON_UNESCAPED_SLASHES));
"#;

/// Asks Magento's scanner and PHP's tokenizer about every file in one `php` run. An error means
/// the scope of no file is known.
pub(crate) fn check_scope(
    files: &[UnreadFile],
    magento_root: &Path,
    generated: &Path,
    php: &str,
) -> Result<Vec<FileScope>, String> {
    let folder = tempfile::Builder::new()
        .prefix("fast-di-scope-")
        .tempdir()
        .map_err(|e| format!("could not create a temporary folder: {e}"))?;
    let script = folder.path().join("scope.php");
    let list = folder.path().join("files.json");
    let result = folder.path().join("result.json");
    let paths: Vec<String> = files
        .iter()
        .map(|file| file.path.to_string_lossy().into_owned())
        .collect();
    std::fs::write(&script, format!("{SCOPE_HEAD_PHP}{BOOT_PHP}{SCOPE_PHP}"))
        .and_then(|()| std::fs::write(&list, serde_json::to_vec(&paths).unwrap_or_default()))
        .map_err(|e| format!("could not write the scope check: {e}"))?;
    let output = Command::new(php)
        .args(["-d", "display_errors=stderr"])
        .arg(&script)
        .arg(magento_root)
        .arg(generated)
        .arg(&list)
        .arg(&result)
        .current_dir(magento_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .map_err(|e| format!("could not start {php}: {e}"))?;
    let text = std::fs::read_to_string(&result).map_err(|_| {
        let stderr = String::from_utf8_lossy(&output.stderr);
        format!(
            "{php} exited with {} without a result{}",
            output.status,
            stderr
                .lines()
                .find(|line| !line.trim().is_empty())
                .map(|line| format!(": {}", line.trim()))
                .unwrap_or_default()
        )
    })?;
    match serde_json::from_str::<ScopeOutput>(&text) {
        Ok(ScopeOutput::Files { files: scopes }) if scopes.len() == files.len() => Ok(scopes),
        Ok(ScopeOutput::Files { .. }) => Err("the scope check answered for other files".into()),
        Ok(ScopeOutput::Error { error }) => Err(error),
        Err(e) => Err(format!("invalid scope check result: {e}")),
    }
}

/// The first use of each class name, by lowercase name, in active DI configuration and in
/// compiled classes.
#[derive(Default)]
pub(crate) struct Uses {
    first: FxHashMap<String, String>,
}

impl Uses {
    fn add(&mut self, name: &str, what: impl FnOnce() -> String) {
        let name = name.trim().trim_start_matches('\\');
        if name.is_empty() {
            return;
        }
        self.first
            .entry(name.to_ascii_lowercase())
            .or_insert_with(what);
    }

    pub(crate) fn first_use(&self, name: &str) -> Option<&str> {
        self.first
            .get(&name.trim_start_matches('\\').to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Preferences, types, virtual types, plugins and object arguments of one area's
    /// configuration, or the global one. Sorted, so the use named first does not depend on hash
    /// order.
    pub(crate) fn add_di_config(&mut self, config: &DiConfig, scope: &str) {
        let scope = if scope == "global" {
            "the global configuration".to_string()
        } else {
            format!("area {scope}")
        };
        let mut preferences: Vec<_> = config.preferences.iter().collect();
        preferences.sort();
        for (for_type, to_type) in preferences {
            self.add(for_type, || {
                format!("the preference for {for_type} in {scope}")
            });
            self.add(to_type, || {
                format!("the preference for {for_type} in {scope}")
            });
        }
        let mut types: Vec<_> = config.type_configs.iter().collect();
        types.sort_by(|a, b| a.0.cmp(b.0));
        for (type_name, type_config) in types {
            self.add(type_name, || {
                format!("the di.xml type {type_name} in {scope}")
            });
            for argument in &type_config.arguments {
                self.add_argument(type_name, argument, &scope);
            }
        }
        let mut virtual_types: Vec<_> = config.virtual_types.values().collect();
        virtual_types.sort_by(|a, b| a.name.cmp(&b.name));
        for virtual_type in virtual_types {
            self.add(&virtual_type.type_name, || {
                format!("the virtual type {} in {scope}", virtual_type.name)
            });
        }
        let mut plugins: Vec<_> = config.plugins.iter().collect();
        plugins.sort_by(|a, b| a.0.cmp(b.0));
        for (owner, list) in plugins {
            self.add(owner, || format!("the plugins on {owner} in {scope}"));
            for plugin in list {
                self.add(&plugin.type_name, || {
                    format!("the plugin {} on {owner} in {scope}", plugin.name)
                });
            }
        }
    }

    fn add_argument(&mut self, type_name: &str, argument: &Argument, scope: &str) {
        match argument {
            Argument::Object { name, value, .. } => {
                self.add(value, || {
                    format!("the argument {name} of {type_name} in {scope}")
                });
            }
            Argument::Array { items, .. } => {
                for item in items {
                    self.add_argument(type_name, item, scope);
                }
            }
            _ => {}
        }
    }

    /// Parents, interfaces, traits and constructor parameter types of compiled classes. Trait
    /// uses are gone once `apply_traits` has run, so this comes before it.
    pub(crate) fn add_classes(&mut self, class_map: &FxHashMap<String, ClassInfo>) {
        let mut classes: Vec<&ClassInfo> = class_map.values().collect();
        classes.sort_by(|a, b| a.fqcn.cmp(&b.fqcn));
        for info in classes {
            let class = &info.fqcn;
            if let Some(parent) = &info.extends {
                self.add(parent, || format!("class {class} as its parent"));
            }
            for interface in &info.implements {
                self.add(interface, || format!("class {class} as an interface"));
            }
            for used in &info.traits.uses {
                self.add(used, || format!("class {class} as a trait"));
            }
            for param in info.constructor.iter().flat_map(|c| &c.params) {
                for name in param.type_hint.iter().flat_map(|t| class_names(t)) {
                    self.add(name, || {
                        format!("the constructor parameter ${} of {class}", param.name)
                    });
                }
            }
        }
    }
}

/// The names in a type: `?A`, `A|null`, `(A&B)|C`.
fn class_names(type_hint: &str) -> impl Iterator<Item = &str> {
    type_hint
        .split(['|', '&', '(', ')', '?'])
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

/// The message to stop the compile with for this file, or `None` when a warning is enough.
pub(crate) fn stop_reason(file: &UnreadFile, scope: &FileScope, uses: &Uses) -> Option<String> {
    let path = file.path.display();
    let unread = format!("No tier could read {path} ({})", file.reason);
    if let Some(error) = &scope.scanner_error {
        return Some(format!(
            "{unread}, and Magento's FileClassScanner cannot read it either: {error}"
        ));
    }
    let Some(scanned) = &scope.scanner else {
        return Some(format!(
            "{unread}, and the store has no Magento\\Setup\\Module\\Di\\Code\\Reader\\FileClassScanner to say whether Magento compiles it"
        ));
    };
    if !scanned.is_empty() {
        return Some(format!(
            "{unread}. Magento compiles its class {}, and the compile needs that class's constructor and methods; a passing PHP class check only shows that the file loads",
            scanned.trim_start_matches('\\')
        ));
    }
    let names = match (&scope.names, &scope.names_error) {
        (Some(names), _) => names,
        (None, error) => {
            return Some(format!(
                "{unread}, and PHP's tokenizer cannot list the classes it declares: {}",
                error.as_deref().unwrap_or("no answer")
            ))
        }
    };
    names.iter().find_map(|name| {
        uses.first_use(name).map(|used| {
            format!(
                "{unread}. Magento does not compile it, but it declares {name}, which is used by {used}"
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use di_xml_reader::{Plugin, TypeConfig, VirtualType};

    fn unread() -> UnreadFile {
        UnreadFile {
            path: PathBuf::from("/store/app/code/Acme/Lost.php"),
            reason: "Tier 1: a; Tier 2: b; Tier 3: c".into(),
        }
    }

    fn scope(scanner: Option<&str>, names: &[&str]) -> FileScope {
        FileScope {
            scanner: scanner.map(str::to_string),
            names: Some(names.iter().map(|name| name.to_string()).collect()),
            ..FileScope::default()
        }
    }

    #[test]
    fn a_scanner_that_throws_stops_the_compile() {
        let scope = FileScope {
            scanner_error: Some("Namespace not defined properly".into()),
            ..scope(Some(""), &[])
        };
        let reason = stop_reason(&unread(), &scope, &Uses::default()).unwrap();
        assert!(
            reason
                .contains("FileClassScanner cannot read it either: Namespace not defined properly"),
            "{reason}"
        );
    }

    #[test]
    fn a_class_the_scanner_finds_stops_the_compile_whether_used_or_not() {
        let reason = stop_reason(
            &unread(),
            &scope(Some("Acme\\Lost"), &["Acme\\Lost"]),
            &Uses::default(),
        )
        .unwrap();
        assert!(
            reason.contains("Magento compiles its class Acme\\Lost"),
            "{reason}"
        );
    }

    #[test]
    fn an_unscanned_file_nothing_uses_is_no_stop() {
        let mut uses = Uses::default();
        uses.add("Acme\\Other", || "something".into());
        assert_eq!(
            stop_reason(&unread(), &scope(Some(""), &["Acme\\Lost"]), &uses),
            None
        );
    }

    #[test]
    fn plugins_virtual_types_and_nested_object_arguments_are_uses() {
        let mut config = DiConfig::default();
        config.plugins.insert(
            "Acme\\Target".into(),
            vec![Plugin {
                name: "watch".into(),
                type_name: "\\Acme\\LostPlugin".into(),
                sort_order: None,
                disabled: None,
                disabled_after_type: false,
            }],
        );
        config.virtual_types.insert(
            "acmeVirtual".into(),
            VirtualType {
                name: "acmeVirtual".into(),
                type_name: "Acme\\LostParent".into(),
            },
        );
        config.type_configs.insert(
            "Acme\\Holder".into(),
            TypeConfig {
                shared: None,
                arguments: vec![Argument::Array {
                    name: "handlers".into(),
                    items: vec![Argument::Object {
                        name: "first".into(),
                        value: "Acme\\LostHandler".into(),
                        shared: None,
                        sort_order: 0,
                    }],
                    sort_order: 0,
                }],
            },
        );
        let mut uses = Uses::default();
        uses.add_di_config(&config, "frontend");
        assert_eq!(
            uses.first_use("acme\\lostplugin"),
            Some("the plugin watch on Acme\\Target in area frontend")
        );
        assert_eq!(
            uses.first_use("Acme\\LostParent"),
            Some("the virtual type acmeVirtual in area frontend")
        );
        assert_eq!(
            uses.first_use("\\Acme\\LostHandler"),
            Some("the argument first of Acme\\Holder in area frontend")
        );
    }

    #[test]
    fn the_names_of_a_type_are_its_classes() {
        let names: Vec<&str> = class_names("?(A\\B&C)|null").collect();
        assert_eq!(names, ["A\\B", "C", "null"]);
    }
}
