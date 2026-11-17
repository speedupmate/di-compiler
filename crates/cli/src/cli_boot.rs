//! Classes that `bin/magento` builds before `setup:di:compile` runs.
//!
//! Composer loads every file in `vendor/composer/autoload_files.php` in every process. A
//! `cli_commands.php` among them calls `CommandLocator::register()` with a command list class,
//! and `Cli::getVendorCommands()` creates that list and each command it returns before any
//! command runs, `setup:di:compile` included. Metadata is not compiled at that point, so
//! Magento's generator writes the interceptors and factories those objects ask for into
//! `generated/code`, and the compile then scans that folder. A list can live in a module that
//! the compile does not scan, such as a disabled module on Magento 2.4.9.

use php_extractor::types::{ClassInfo, ClassKind};
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::Path;

const COMMAND: &str = "Symfony\\Component\\Console\\Command\\Command";

#[derive(Default)]
pub(crate) struct BootClasses {
    /// Classes the boot creates that the compile does not scan.
    pub built: FxHashMap<String, ClassInfo>,
    /// Classes the boot asks for that exist only once Magento generates them.
    pub asked: FxHashSet<String>,
}

/// The command list classes that Composer's autoload files register.
pub(crate) fn registered_command_lists(magento_root: &Path) -> Vec<String> {
    let vendor = magento_root.join("vendor");
    let Ok(list) = std::fs::read_to_string(vendor.join("composer/autoload_files.php")) else {
        return Vec::new();
    };
    let mut lists = Vec::new();
    for line in list.lines() {
        let path = if let Some((_, rest)) = line.split_once("$vendorDir . '/") {
            vendor.join(rest.split('\'').next().unwrap_or_default())
        } else if let Some((_, rest)) = line.split_once("$baseDir . '/") {
            magento_root.join(rest.split('\'').next().unwrap_or_default())
        } else {
            continue;
        };
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let names = Names::of(&source);
        let mut rest = source.as_str();
        while let Some((_, after)) = rest.split_once("CommandLocator::register(") {
            rest = after;
            let argument = after.split(')').next().unwrap_or_default().trim();
            if let Some(class) = argument.strip_suffix("::class") {
                lists.push(names.resolve(class.trim()));
            }
        }
    }
    lists
}

/// What creating `lists`, and every command class each list names, creates and asks for.
///
/// The object manager builds a class from its constructor: each required parameter with a
/// class type becomes an object, after the global preferences. Classes the compile scans are
/// left alone, because the compile itself covers them.
pub(crate) fn classes_built_at_boot(
    lists: &[String],
    lookup: &dyn Fn(&str) -> Option<ClassInfo>,
    scanned: &dyn Fn(&str) -> bool,
    preferences: &FxHashMap<String, String>,
    is_generated_name: &dyn Fn(&str) -> bool,
) -> BootClasses {
    let mut boot = BootClasses::default();
    let mut pending: Vec<String> = Vec::new();
    for list in lists {
        pending.push(list.clone());
        if let Some(info) = lookup(list) {
            pending.extend(command_classes_named_by(&info, lookup));
        }
    }
    let mut seen = FxHashSet::default();
    while let Some(requested) = pending.pop() {
        let class = preferred(&requested, preferences);
        if !seen.insert(class.clone()) || scanned(&class) {
            continue;
        }
        let Some(info) = lookup(&class) else {
            if is_generated_name(&class) {
                boot.asked.insert(class);
            }
            continue;
        };
        if info.kind != ClassKind::Class || info.is_abstract {
            continue;
        }
        for param in constructor_params(&info, lookup) {
            if param.is_optional || param.is_variadic || param.is_primitive {
                continue;
            }
            let Some(type_hint) = param.type_hint.as_deref() else {
                continue;
            };
            let type_hint = type_hint.trim_start_matches('?').trim_start_matches('\\');
            if !type_hint.contains(['|', '&']) {
                pending.push(type_hint.to_string());
            }
        }
        boot.built.insert(class, info);
    }
    boot
}

fn preferred(class: &str, preferences: &FxHashMap<String, String>) -> String {
    let mut current = class.trim_start_matches('\\').to_string();
    let mut visited = FxHashSet::default();
    while visited.insert(current.clone()) {
        match preferences.get(&current) {
            Some(next) => current = next.trim_start_matches('\\').to_string(),
            None => break,
        }
    }
    current
}

/// A class without its own constructor uses its parent's.
fn constructor_params(
    info: &ClassInfo,
    lookup: &dyn Fn(&str) -> Option<ClassInfo>,
) -> Vec<php_extractor::types::ConstructorParam> {
    let mut current = info.clone();
    for _ in 0..32 {
        if let Some(constructor) = &current.constructor {
            return constructor.params.clone();
        }
        let Some(parent) = current.extends.as_deref().and_then(lookup) else {
            break;
        };
        current = parent;
    }
    Vec::new()
}

/// The `X::class` names in the list's source that are console commands.
fn command_classes_named_by(
    list: &ClassInfo,
    lookup: &dyn Fn(&str) -> Option<ClassInfo>,
) -> Vec<String> {
    let Ok(source) = std::fs::read_to_string(&list.path) else {
        return Vec::new();
    };
    let names = Names::of(&source);
    let mut commands = Vec::new();
    let mut rest = source.as_str();
    while let Some((before, after)) = rest.split_once("::class") {
        rest = after;
        let start = before
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '\\'))
            .map_or(0, |at| at + 1);
        let name = &before[start..];
        if name.is_empty() || matches!(name, "self" | "static" | "parent") {
            continue;
        }
        let class = names.resolve(name);
        if is_command(&class, lookup) && !commands.contains(&class) {
            commands.push(class);
        }
    }
    commands
}

fn is_command(class: &str, lookup: &dyn Fn(&str) -> Option<ClassInfo>) -> bool {
    let mut current = class.to_string();
    for _ in 0..32 {
        if current == COMMAND {
            return true;
        }
        let Some(parent) = lookup(&current).and_then(|info| info.extends) else {
            return false;
        };
        current = parent.trim_start_matches('\\').to_string();
    }
    false
}

/// The namespace and `use` imports of one PHP file, enough to resolve a class name.
struct Names {
    namespace: String,
    imports: FxHashMap<String, String>,
}

impl Names {
    fn of(source: &str) -> Self {
        let mut namespace = String::new();
        let mut imports = FxHashMap::default();
        for statement in source.split(';') {
            let statement = statement.trim();
            let statement = statement
                .rsplit(['{', '}'])
                .next()
                .unwrap_or(statement)
                .trim();
            let statement = statement.strip_prefix("<?php").unwrap_or(statement).trim();
            if let Some(name) = statement.strip_prefix("namespace ") {
                namespace = name.trim().to_string();
            } else if let Some(import) = statement.strip_prefix("use ") {
                let import = import.trim();
                if import.starts_with("function ") || import.starts_with("const ") {
                    continue;
                }
                let (class, alias) = match import.split_once(" as ") {
                    Some((class, alias)) => (class.trim(), alias.trim()),
                    None => (import, import.rsplit('\\').next().unwrap_or(import)),
                };
                imports.insert(
                    alias.to_ascii_lowercase(),
                    class.trim_start_matches('\\').to_string(),
                );
            }
        }
        Self { namespace, imports }
    }

    fn resolve(&self, name: &str) -> String {
        if let Some(absolute) = name.strip_prefix('\\') {
            return absolute.to_string();
        }
        let (first, rest) = match name.split_once('\\') {
            Some((first, rest)) => (first, Some(rest)),
            None => (name, None),
        };
        if let Some(import) = self.imports.get(&first.to_ascii_lowercase()) {
            return match rest {
                Some(rest) => format!("{import}\\{rest}"),
                None => import.clone(),
            };
        }
        if self.namespace.is_empty() {
            name.to_string()
        } else {
            format!("{}\\{name}", self.namespace)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Names;

    #[test]
    fn names_resolve_through_imports_and_namespace() {
        let names = Names::of(
            "<?php\nnamespace Acme\\Cli;\n\nuse Acme\\Cli\\Command\\Run;\nuse Acme\\Other as Elsewhere;\n",
        );
        assert_eq!(names.resolve("Run"), "Acme\\Cli\\Command\\Run");
        assert_eq!(names.resolve("Elsewhere\\Stop"), "Acme\\Other\\Stop");
        assert_eq!(names.resolve("Local"), "Acme\\Cli\\Local");
        assert_eq!(names.resolve("\\Top\\Level"), "Top\\Level");
    }
}
