//! Which modules the store's own `setup:di:compile` scans for classes.
//!
//! `DiCompileCommand::execute()` passes `'application' => <paths>` to its operations. Magento
//! 2.4.9 filters those paths on the status in `app/etc/config.php`, 2.4.8 does not, and a store
//! can patch either way. So the answer comes from the store's source and not from its version.

use std::path::Path;

const COMPILE_COMMAND: &str = "setup/src/Magento/Setup/Console/Command/DiCompileCommand.php";
const CONFIG_READER: &str = "setup/src/Magento/Setup/Module/Di/Compiler/Config/Reader.php";

/// The preference targets that the store's `Reader::generateCachePerScope()` stops on.
///
/// Magento 2.4.8 added both checks, 2.4.7 has neither, and a store can patch either way. 2.4.7
/// ships a preference to a class that does not exist (`Magento\Catalog\Api\Data\MassActionInterface`),
/// so a store on 2.4.7 compiles only when this follows its own reader.
pub(crate) struct PreferenceTargetChecks {
    /// A target that fails `class_exists()`, or whose file fails to load.
    pub(crate) missing: bool,
    /// A target that the scan does not list.
    pub(crate) not_scanned: bool,
}

/// A store without the reader keeps both checks, because then there is no compile to follow.
pub(crate) fn preference_target_checks(magento_root: &Path) -> PreferenceTargetChecks {
    let path = magento_root.join(CONFIG_READER);
    let Ok(source) = std::fs::read_to_string(&path) else {
        log::warn!(
            "{} not found; preference targets are checked as Magento 2.4.8 does",
            path.display()
        );
        return PreferenceTargetChecks {
            missing: true,
            not_scanned: true,
        };
    };
    let code = without_comments(&source);
    let checks = PreferenceTargetChecks {
        missing: code.contains("but the latter does not exist."),
        not_scanned: code.contains("has not been included in dependency injection compilation."),
    };
    // Normal on Magento 2.4.7 and older. On a newer store it means the message text changed.
    let unchecked = match (checks.missing, checks.not_scanned) {
        (true, true) => None,
        (false, false) => Some("a missing or unscanned class"),
        (false, true) => Some("a missing class"),
        (true, false) => Some("an unscanned class"),
    };
    if let Some(unchecked) = unchecked {
        log::info!(
            "{} does not stop on a preference to {unchecked}, so this compile does not either",
            path.display()
        );
    }
    checks
}

/// Whether the store's command scans the classes of disabled modules.
///
/// A store without the command has no compile to follow, so disabled modules stay out.
/// A command this code cannot read is an error: both guesses can break a store silently.
pub(crate) fn compiles_disabled_modules(magento_root: &Path) -> Result<bool, String> {
    let path = magento_root.join(COMPILE_COMMAND);
    let Ok(source) = std::fs::read_to_string(&path) else {
        log::warn!(
            "{} not found; classes of disabled modules are not scanned",
            path.display()
        );
        return Ok(false);
    };
    scans_disabled_modules(&without_comments(&source)).ok_or_else(|| {
        format!(
            "Cannot tell which modules {} compiles: its 'application' paths have an unknown form",
            path.display()
        )
    })
}

/// Whether the store's command passes `setup/src` to the area configuration it compiles.
///
/// Magento 2.4.8 added `$compiledPathsList['setup']` to `AREA_CONFIG_GENERATOR`. Without it, the
/// classes of `setup/src` have no arguments in `generated/metadata`, and no interceptor preference
/// follows from them. A store without the command, or with a form this code cannot read, keeps
/// them, as Magento 2.4.8 does.
pub(crate) fn area_config_compiles_setup_classes(magento_root: &Path) -> bool {
    let path = magento_root.join(COMPILE_COMMAND);
    let Ok(source) = std::fs::read_to_string(&path) else {
        return true;
    };
    area_config_paths(&without_comments(&source))
        .map(|paths| paths.contains("$compiledPathsList['setup']"))
        .unwrap_or_else(|| {
            log::warn!(
                "Cannot read the AREA_CONFIG_GENERATOR paths of {}; setup/src classes are compiled as Magento 2.4.8 does",
                path.display()
            );
            true
        })
}

fn area_config_paths(code: &str) -> Option<&str> {
    let (_, after) = code.split_once("OperationFactory::AREA_CONFIG_GENERATOR")?;
    let list = after.trim_start().strip_prefix("=>")?.trim_start();
    if !list.starts_with('[') {
        return None;
    }
    let mut depth = 0usize;
    for (index, c) in list.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&list[..=index]);
                }
            }
            _ => {}
        }
    }
    None
}

fn scans_disabled_modules(code: &str) -> Option<bool> {
    let (before, after) = code.split_once("'application'")?;
    let variable = after.trim_start().strip_prefix("=>")?.trim_start();
    let end = variable.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))?;
    let (variable, rest) = variable.split_at(end);
    if !variable.starts_with('$') || !rest.trim_start().starts_with([',', ']']) {
        return None;
    }
    let (_, assigned) = before.rsplit_once(&format!("{variable} ="))?;
    let (expression, _) = assigned.split_once(';')?;
    if expression.contains("array_filter") {
        return Some(false);
    }
    expression
        .contains("getPaths(ComponentRegistrar::MODULE)")
        .then_some(true)
}

/// PHP source without comments. Strings stay, because they can hold `//` and `#`.
fn without_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                out.push(c);
                while let Some(inner) = chars.next() {
                    out.push(inner);
                    if inner == '\\' {
                        out.extend(chars.next());
                    } else if inner == c {
                        break;
                    }
                }
            }
            '#' if chars.peek() != Some(&'[') => skip_line(&mut chars),
            '/' if chars.peek() == Some(&'/') => skip_line(&mut chars),
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = ' ';
                for inner in chars.by_ref() {
                    if previous == '*' && inner == '/' {
                        break;
                    }
                    previous = inner;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn skip_line(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    for c in chars.by_ref() {
        if c == '\n' {
            break;
        }
    }
}
