//! Which modules the store's own `setup:di:compile` scans for classes.
//!
//! `DiCompileCommand::execute()` passes `'application' => <paths>` to its operations. Magento
//! 2.4.9 filters those paths on the status in `app/etc/config.php`, 2.4.8 does not, and a store
//! can patch either way. So the answer comes from the store's source and not from its version.

use std::path::Path;

const COMPILE_COMMAND: &str = "setup/src/Magento/Setup/Console/Command/DiCompileCommand.php";

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
