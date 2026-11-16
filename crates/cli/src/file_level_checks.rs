//! Runs the class files that call `class_exists()` or a sibling outside any class.
//!
//! `ClassesScanner::includeClass()` loads every class file, so Magento runs this
//! code during compile. It can ask for a generated class by any name, computed
//! ones included, and `Autoloader::load()` generates that class on the spot. The
//! answer can also decide which of two declarations of a class exists. Neither
//! can be read from the source, so PHP runs the file here too, once per file:
//! a file that ends the process only costs its own result.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use php_extractor::{lexer::Lexer, ClassInfo};

use crate::class_checks::OBJECT_MANAGER_PHP;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Deserialize;

/// `Generator::generateClass()` needs a loadable source class, so the same rule
/// applies here. A stub stands in for the generated class, because the file under
/// test must see the same answer as during a real compile.
const RUN_CLASS_FILE_PHP: &str = r#"
ini_set('display_errors', '0');
[, $root, $class, $file] = $argv;
$asked = [];
ob_start();
try {
    require $root . '/vendor/autoload.php';
    // OBJECT MANAGER
    spl_autoload_register(function (string $name) use (&$asked): void {
        $source = match (true) {
            str_ends_with($name, '\\Proxy') => substr($name, 0, -6),
            str_ends_with($name, 'Factory') => substr($name, 0, -7),
            default => '',
        };
        if ($source === '' || (!class_exists($source) && !interface_exists($source))) {
            return;
        }
        $asked[] = $name;
        $split = strrpos($name, '\\');
        $namespace = $split === false ? '' : 'namespace ' . substr($name, 0, $split) . '; ';
        eval($namespace . 'class ' . substr($name, $split === false ? 0 : $split + 1) . ' {}');
    });
    if (!class_exists($class)) {
        require_once $file;
    }
    $reflection = class_exists($class, false) || interface_exists($class, false) || trait_exists($class, false)
        ? new ReflectionClass($class)
        : null;
    $result = [
        'declared' => $reflection !== null,
        'line' => $reflection?->getFileName() === realpath($file) ? $reflection->getStartLine() : null,
        'asked' => $asked,
    ];
} catch (\Throwable) {
    $result = null;
}
ob_end_clean();
fwrite(STDOUT, json_encode($result) . "\n");
"#;

#[derive(Deserialize)]
struct FileOutcome {
    declared: bool,
    line: Option<usize>,
    asked: Vec<String>,
}

/// Runs each `(fqcn, path)` file and brings `class_map` in line with what PHP
/// declared. Returns the generated class names these files asked for.
///
/// A file that PHP cannot run keeps the class from the static scan, which reads
/// the first declaration in the file.
pub(crate) fn run_file_level_checks(
    class_map: &mut FxHashMap<String, ClassInfo>,
    checks: Vec<(String, PathBuf)>,
    magento_root: &Path,
    php_bin: &str,
) -> FxHashSet<String> {
    let outcomes: Vec<_> = checks
        .into_par_iter()
        .map(|(fqcn, path)| {
            let outcome = run_class_file(&fqcn, &path, magento_root, php_bin);
            (fqcn, path, outcome)
        })
        .collect();

    let mut asked = FxHashSet::default();
    for (fqcn, path, outcome) in outcomes {
        let Some(outcome) = outcome else {
            log::warn!(
                "Could not run {}: using the first declaration of {fqcn}",
                path.display()
            );
            continue;
        };
        asked.extend(outcome.asked);
        if !outcome.declared {
            class_map.remove(&fqcn);
        } else if let Some(line) = outcome.line {
            match Lexer::extract_declared_at(&path, line) {
                Ok(info) if info.fqcn == fqcn => {
                    class_map.insert(fqcn, info);
                }
                _ => log::warn!(
                    "No declaration of {fqcn} on line {line} of {}",
                    path.display()
                ),
            }
        }
    }
    asked
}

fn run_class_file(
    fqcn: &str,
    path: &Path,
    magento_root: &Path,
    php_bin: &str,
) -> Option<FileOutcome> {
    let output = Command::new(php_bin)
        .args(["-d", "display_errors=0", "-r"])
        .arg(RUN_CLASS_FILE_PHP.replace("// OBJECT MANAGER", OBJECT_MANAGER_PHP))
        .arg("--")
        .arg(magento_root)
        .arg(fqcn)
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str::<Option<FileOutcome>>(stdout.lines().last()?)
        .ok()
        .flatten()
}
