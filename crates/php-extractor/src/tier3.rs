//! Tier 3: asks PHP itself, through reflection, for a file Tiers 1 and 2 cannot read.
//!
//! Each file gets its own `php` process, so a fatal error while loading it ends only that run.
//! The file loads as it does in `bin/magento`: the store's bootstrap, stand-ins for generated
//! classes and a lazy ObjectManager. The answer goes to a result file, so anything the store or
//! the class file prints cannot change it.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::php_snippets::{BOOT_PHP, ENCODE_DEFAULT_PHP, OBJECT_MANAGER_PHP};
use crate::types::{ClassInfo, ClassKind, ExtractResult};

/// How Tier 3 runs PHP for one store.
#[derive(Debug, Clone)]
pub struct Tier3Config {
    /// The PHP binary (`--fallback-php`).
    pub php: String,
    pub magento_root: PathBuf,
    /// Where generated classes are, as `generated/code`.
    pub generated: PathBuf,
    /// A run that takes longer is stopped and counts as failed.
    pub timeout: Duration,
    /// Each run gets its own folder in here, removed when the run ends.
    pub scratch: PathBuf,
}

impl Tier3Config {
    pub fn new(php: impl Into<String>, magento_root: &Path, generated: &Path) -> Self {
        Self {
            php: php.into(),
            magento_root: magento_root.to_path_buf(),
            generated: generated.to_path_buf(),
            timeout: Duration::from_secs(30),
            scratch: std::env::temp_dir(),
        }
    }
}

static CONFIG: OnceLock<Tier3Config> = OnceLock::new();

/// Sets the store Tier 3 reads classes of. Only the first call counts; a later one gets its
/// config back.
pub fn configure(config: Tier3Config) -> Result<(), Tier3Config> {
    CONFIG.set(config)
}

/// Tier 3 as the extraction chain calls it.
pub fn extract_tier3(path: &Path) -> ExtractResult {
    let _ = path;
    if CONFIG.get().is_none() {
        return ExtractResult::PhpFallbackFailed("Tier 3 is not configured".into());
    }
    // `reflect` reads only which class a file declares so far. A class without its constructor
    // and methods would compile to wrong code, so the chain does not use it yet.
    ExtractResult::PhpFallbackFailed("Tier 3 does not read constructors and methods yet".into())
}

/// Reads the class a file declares, in PHP.
pub fn reflect(config: &Tier3Config, path: &Path) -> ExtractResult {
    match run(config, path) {
        Ok(Tier3Output::Class(class)) => ExtractResult::Ok(class.into_class_info(path)),
        Ok(Tier3Output::NoClass) => ExtractResult::NoClass,
        Ok(Tier3Output::Error { message }) => ExtractResult::PhpFallbackFailed(message),
        Err(reason) => ExtractResult::PhpFallbackFailed(reason),
    }
}

/// Needs `$file` and `$result` after `BOOT_PHP`. Writes one JSON object to `$result`.
const REFLECT_HEAD_PHP: &str = r#"<?php
[, $root, $generated, $file, $result] = $argv;
$finished = false;
$write = static function (array $row) use ($result): void {
    file_put_contents($result, json_encode($row, JSON_INVALID_UTF8_SUBSTITUTE | JSON_UNESCAPED_SLASHES));
};
register_shutdown_function(static function () use (&$finished, $write): void {
    if ($finished) {
        return;
    }
    while (ob_get_level() > 0) {
        ob_end_clean();
    }
    $last = error_get_last();
    $write(['result' => 'error', 'message' => $last !== null && ($last['type'] & (E_ERROR | E_PARSE | E_CORE_ERROR | E_COMPILE_ERROR))
        ? 'There is an error in ' . $last['file'] . ' at line: ' . $last['line'] . ': ' . $last['message']
        : 'The file ended the PHP process']);
});
"#;

const REFLECT_PHP: &str = r#"
$real = realpath($file) ?: $file;
$level = ob_get_level();
ob_start();
try {
    require_once $file;
} catch (\Throwable $e) {
    while (ob_get_level() > $level) {
        ob_end_clean();
    }
    $finished = true;
    $write(['result' => 'error', 'message' => 'There is an error in ' . $e->getFile() . ' at line: ' . $e->getLine() . ': ' . $e->getMessage()]);
    exit(0);
}
while (ob_get_level() > $level) {
    ob_end_clean();
}
// The class of the file is its first named class-like declaration in source order, as Tier 1
// reads it. Reflection gives only a line, which two declarations can share, so PHP's tokenizer
// finds it.
$tokens = PhpToken::tokenize((string) file_get_contents($file));
$count = count($tokens);
$next = static function (int $i) use ($tokens, $count): int {
    for ($i++; $i < $count && $tokens[$i]->isIgnorable(); $i++) {
    }
    return $i;
};
$namespace = '';
$previous = null;
$first = null;
for ($i = 0; $i < $count && $first === null; $i++) {
    $token = $tokens[$i];
    if ($token->isIgnorable()) {
        continue;
    }
    if ($token->is(T_NAMESPACE)) {
        $j = $next($i);
        if ($j < $count && $tokens[$j]->is([T_STRING, T_NAME_QUALIFIED])) {
            $namespace = $tokens[$j]->text;
            $i = $j;
        } elseif ($j < $count && $tokens[$j]->text === '{') {
            $namespace = '';
        }
    } elseif ($token->is([T_CLASS, T_INTERFACE, T_TRAIT, T_ENUM])
        && !($previous !== null && $previous->is([T_DOUBLE_COLON, T_NEW]))) {
        // `Foo::class` and `new class` are no declarations; an anonymous class has no name.
        $j = $next($i);
        if ($j < $count && $tokens[$j]->is(T_STRING)) {
            $first = ($namespace === '' ? '' : $namespace . '\\') . $tokens[$j]->text;
        }
    }
    $previous = $token;
}
$finished = true;
if ($first === null) {
    $write(['result' => 'no_class']);
    exit(0);
}
if (!class_exists($first, false) && !interface_exists($first, false) && !trait_exists($first, false)) {
    $write(['result' => 'error', 'message' => $first . ' is declared in ' . $file . ', but PHP did not declare it there']);
    exit(0);
}
$found = new ReflectionClass($first);
$declared = $found->getFileName();
if ($declared === false || (realpath($declared) ?: $declared) !== $real) {
    $write(['result' => 'error', 'message' => $first . ' is declared in ' . $file . ', but PHP has it from ' . ($declared ?: 'PHP itself')]);
    exit(0);
}
if ($found->isEnum()) {
    $write(['result' => 'no_class']);
    exit(0);
}
$write([
    'result' => 'class',
    'kind' => $found->isInterface() ? 'interface' : ($found->isTrait() ? 'trait' : ($found->isAbstract() ? 'abstract_class' : 'class')),
    'namespace' => $found->getNamespaceName(),
    'name' => $found->getShortName(),
    'fqcn' => $found->getName(),
]);
"#;

fn script() -> &'static str {
    static SCRIPT: OnceLock<String> = OnceLock::new();
    SCRIPT.get_or_init(|| {
        format!("{REFLECT_HEAD_PHP}{BOOT_PHP}{OBJECT_MANAGER_PHP}{ENCODE_DEFAULT_PHP}{REFLECT_PHP}")
    })
}

#[derive(Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
enum Tier3Output {
    Class(Tier3Class),
    NoClass,
    Error { message: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Tier3Kind {
    Class,
    AbstractClass,
    Interface,
    Trait,
}

#[derive(Deserialize)]
struct Tier3Class {
    kind: Tier3Kind,
    namespace: String,
    name: String,
    fqcn: String,
}

impl Tier3Class {
    fn into_class_info(self, path: &Path) -> ClassInfo {
        ClassInfo {
            path: path.to_path_buf(),
            namespace: self.namespace,
            name: self.name,
            fqcn: self.fqcn,
            kind: match self.kind {
                Tier3Kind::Class => ClassKind::Class,
                Tier3Kind::AbstractClass => ClassKind::AbstractClass,
                Tier3Kind::Interface => ClassKind::Interface,
                Tier3Kind::Trait => ClassKind::Trait,
            },
            extends: None,
            implements: Vec::new(),
            constructor: None,
            is_abstract: false,
            is_final: false,
            public_methods: Vec::new(),
            traits: Default::default(),
        }
    }
}

fn run(config: &Tier3Config, path: &Path) -> Result<Tier3Output, String> {
    // A new folder with a random name, readable by this user only. Dropping it removes it, so
    // it goes on every return, and only what this run created goes with it.
    let folder = tempfile::Builder::new()
        .prefix("fast-di-tier3-")
        .tempdir_in(&config.scratch)
        .map_err(|e| {
            format!(
                "Could not create a folder in {}: {e}",
                config.scratch.display()
            )
        })?;
    let script_path = folder.path().join("tier3.php");
    let result_path = folder.path().join("result.json");
    let stderr_path = folder.path().join("stderr.txt");
    std::fs::write(&script_path, script())
        .map_err(|e| format!("Could not write {}: {e}", script_path.display()))?;
    let stderr = File::create(&stderr_path)
        .map_err(|e| format!("Could not write {}: {e}", stderr_path.display()))?;

    let mut child = Command::new(&config.php)
        .args(["-d", "display_errors=stderr"])
        .arg(&script_path)
        .arg(&config.magento_root)
        .arg(&config.generated)
        .arg(path)
        .arg(&result_path)
        .current_dir(&config.magento_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .map_err(|e| format!("Could not start {}: {e}", config.php))?;

    let deadline = Instant::now() + config.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timeout after {} s", config.timeout.as_secs_f32()));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Could not wait for {}: {e}", config.php));
            }
        }
    };

    let result = std::fs::read_to_string(&result_path)
        .ok()
        .map(|json| serde_json::from_str::<Tier3Output>(&json));
    match (status.success(), result) {
        (true, Some(Ok(output))) => return Ok(output),
        (true, Some(Err(e))) => return Err(format!("Invalid Tier 3 result: {e}")),
        // A fatal error ends PHP with an exit code; the shutdown function wrote why.
        (false, Some(Ok(Tier3Output::Error { message }))) => {
            return Ok(Tier3Output::Error { message })
        }
        _ => {}
    }
    let first_stderr_line = std::fs::read_to_string(&stderr_path)
        .ok()
        .and_then(|text| {
            text.lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_default();
    Err(format!(
        "php exited with {} without a usable result{}{}",
        status
            .code()
            .map_or_else(|| "a signal".to_string(), |code| format!("code {code}")),
        if first_stderr_line.is_empty() {
            ""
        } else {
            ": "
        },
        first_stderr_line
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // No test in this binary configures Tier 3.
    #[test]
    fn unconfigured_tier3_is_not_attempted() {
        match extract_tier3(Path::new("/nonexistent/Foo.php")) {
            ExtractResult::PhpFallbackFailed(reason) => {
                assert_eq!(reason, "Tier 3 is not configured")
            }
            other => panic!("expected PhpFallbackFailed, got {other:?}"),
        }
    }
}
