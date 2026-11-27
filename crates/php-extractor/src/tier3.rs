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

use crate::lexer::{is_primitive_type, ScanFacts};
use crate::php_snippets::{BOOT_PHP, ENCODE_DEFAULT_PHP, OBJECT_MANAGER_PHP, TYPE_STRING_PHP};
use crate::types::{
    ClassInfo, ClassKind, Constructor, ConstructorParam, ExtractResult, MethodParam,
    MethodSignature, TraitData, TraitMethod,
};

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

/// Tier 3 as the extraction chain calls it, with the store `configure` set.
pub fn extract_tier3(path: &Path) -> (ExtractResult, ScanFacts) {
    match CONFIG.get() {
        Some(config) => reflect_with_facts(config, path),
        None => (
            ExtractResult::PhpFallbackFailed("Tier 3 is not configured".into()),
            ScanFacts::default(),
        ),
    }
}

/// Reads the class a file declares, in PHP: its identity, header, constructor and methods.
pub fn reflect(config: &Tier3Config, path: &Path) -> ExtractResult {
    reflect_with_facts(config, path).0
}

/// Like [`reflect`], and also says whether the declaration is inside a block. `class_check`
/// stays false: PHP has run any `class_exists()` of the file already.
pub fn reflect_with_facts(config: &Tier3Config, path: &Path) -> (ExtractResult, ScanFacts) {
    match run(config, path) {
        Ok(Tier3Output::Class(class)) => {
            let facts = ScanFacts {
                class_check: false,
                inside_block: class.inside_block,
            };
            (ExtractResult::Ok(class.into_class_info(path)), facts)
        }
        Ok(Tier3Output::NoClass) => (ExtractResult::NoClass, ScanFacts::default()),
        Ok(Tier3Output::Error { message }) => (
            ExtractResult::PhpFallbackFailed(message),
            ScanFacts::default(),
        ),
        Err(reason) => (
            ExtractResult::PhpFallbackFailed(reason),
            ScanFacts::default(),
        ),
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
$fail = static function (string $message) use (&$finished, $write): never {
    $finished = true;
    $write(['result' => 'error', 'message' => $message]);
    exit(0);
};
// The class of the file is its first named class-like declaration in source order, as Tier 1
// reads it. Reflection gives only a line, which two declarations can share, so PHP's tokenizer
// finds it, and the names of its header as the source spells them.
$tokens = PhpToken::tokenize((string) file_get_contents($file));
$count = count($tokens);
$next = static function (int $i) use ($tokens, $count): int {
    for ($i++; $i < $count && $tokens[$i]->isIgnorable(); $i++) {
    }
    return $i;
};
$nameTokens = [T_STRING, T_NAME_QUALIFIED, T_NAME_FULLY_QUALIFIED, T_NAME_RELATIVE];
$namespace = '';
$imports = [];
$depth = 0;
$importDepth = 0;
$resolve = static function (PhpToken $name) use (&$namespace, &$imports): string {
    if ($name->is(T_NAME_FULLY_QUALIFIED)) {
        return substr($name->text, 1);
    }
    $text = $name->is(T_NAME_RELATIVE) ? substr($name->text, strlen('namespace\\')) : $name->text;
    $parts = explode('\\', $text, 2);
    if (!$name->is(T_NAME_RELATIVE) && isset($imports[strtolower($parts[0])])) {
        return $imports[strtolower($parts[0])] . (isset($parts[1]) ? '\\' . $parts[1] : '');
    }
    return $namespace === '' ? $text : $namespace . '\\' . $text;
};
// `use A\B, C as D;` and `use A\{B, C as D};` from position $i (the `use`); returns the last token.
$readImports = static function (int $i) use ($tokens, $count, $next, &$imports): int {
    $add = static function (string $name, ?string $alias) use (&$imports): void {
        $name = ltrim($name, '\\');
        $short = $alias ?? substr($name, (int) strrpos('\\' . $name, '\\'));
        $imports[strtolower($short)] = $name;
    };
    $j = $next($i);
    if ($j < $count && $tokens[$j]->is([T_FUNCTION, T_CONST])) {
        return $j;
    }
    while ($j < $count && $tokens[$j]->is([T_STRING, T_NAME_QUALIFIED, T_NAME_FULLY_QUALIFIED])) {
        $base = $tokens[$j]->text;
        $k = $next($j);
        if ($k < $count && $tokens[$k]->is(T_NS_SEPARATOR) && ($group = $next($k)) < $count && $tokens[$group]->text === '{') {
            for ($k = $next($group); $k < $count && $tokens[$k]->text !== '}'; $k = $next($k)) {
                if ($tokens[$k]->is([T_STRING, T_NAME_QUALIFIED])) {
                    $part = $tokens[$k]->text;
                    $as = $next($k);
                    $alias = null;
                    if ($as < $count && $tokens[$as]->is(T_AS)) {
                        $k = $next($as);
                        $alias = $tokens[$k]->text;
                    }
                    $add($base . '\\' . $part, $alias);
                } elseif ($tokens[$k]->is([T_FUNCTION, T_CONST])) {
                    // `use A\{function f}`: no class name.
                    while (($k = $next($k)) < $count && !in_array($tokens[$k]->text, [',', '}'], true)) {
                    }
                    if ($tokens[$k]->text === '}') {
                        break;
                    }
                }
            }
            return $k;
        }
        $alias = null;
        if ($k < $count && $tokens[$k]->is(T_AS)) {
            $alias = $tokens[$next($k)]->text;
            $k = $next($next($k));
        }
        $add($base, $alias);
        if ($k >= $count || $tokens[$k]->text !== ',') {
            return $k;
        }
        $j = $next($k);
    }
    return $j;
};
$previous = null;
$first = null;
$at = null;
$insideBlock = false;
for ($i = 0; $i < $count && $first === null; $i++) {
    $token = $tokens[$i];
    if ($token->isIgnorable()) {
        continue;
    }
    if ($token->text === '{' || $token->is([T_CURLY_OPEN, T_DOLLAR_OPEN_CURLY_BRACES])) {
        $depth++;
    } elseif ($token->text === '}') {
        $depth--;
    } elseif ($token->is(T_NAMESPACE)) {
        $j = $next($i);
        $imports = [];
        $importDepth = $depth;
        if ($j < $count && $tokens[$j]->is([T_STRING, T_NAME_QUALIFIED])) {
            $namespace = $tokens[$j]->text;
            $i = $j;
            $j = $next($j);
        } else {
            $namespace = '';
        }
        if ($j < $count && $tokens[$j]->text === '{') {
            $importDepth = $depth + 1;
        }
    } elseif ($token->is(T_USE) && $depth === $importDepth) {
        // Only an import statement is at this depth: a closure's `use` and a trait `use` are
        // inside braces.
        $i = $readImports($i);
        $token = $tokens[$i] ?? $token;
    } elseif ($token->is([T_CLASS, T_INTERFACE, T_TRAIT, T_ENUM])
        && !($previous !== null && $previous->is([T_DOUBLE_COLON, T_NEW]))) {
        // `Foo::class` and `new class` are no declarations; an anonymous class has no name.
        $j = $next($i);
        if ($j < $count && $tokens[$j]->is(T_STRING)) {
            $first = ($namespace === '' ? '' : $namespace . '\\') . $tokens[$j]->text;
            $at = $j;
            // Inside a block, such as `if`, as Tier 1 reads it: a braced namespace is none.
            $insideBlock = $depth > $importDepth;
        }
    }
    $previous = $token;
}
if ($first === null) {
    $finished = true;
    $write(['result' => 'no_class']);
    exit(0);
}
if (!class_exists($first, false) && !interface_exists($first, false) && !trait_exists($first, false)) {
    $fail($first . ' is declared in ' . $file . ', but PHP did not declare it there');
}
$found = new ReflectionClass($first);
$declared = $found->getFileName();
if ($declared === false || (realpath($declared) ?: $declared) !== $real) {
    $fail($first . ' is declared in ' . $file . ', but PHP has it from ' . ($declared ?: 'PHP itself'));
}
if ($found->isEnum()) {
    $finished = true;
    $write(['result' => 'no_class']);
    exit(0);
}
// The names after `extends` and `implements`, up to the body.
$header = ['extends' => [], 'implements' => []];
$list = null;
for ($k = $at + 1; $k < $count && $tokens[$k]->text !== '{'; $k++) {
    if ($tokens[$k]->is(T_EXTENDS)) {
        $list = 'extends';
    } elseif ($tokens[$k]->is(T_IMPLEMENTS)) {
        $list = 'implements';
    } elseif ($list !== null && $tokens[$k]->is($nameTokens)) {
        $header[$list][] = $resolve($tokens[$k]);
    }
}
// Tier 1 gives an interface's first parent as `extends` and the others as `implements`.
$extends = $header['extends'][0] ?? null;
$implements = $found->isInterface() ? array_slice($header['extends'], 1) : $header['implements'];
$mismatch = static function (string $what) use ($fail, $first): never {
    $fail('The header of ' . $first . ' does not match reflection: ' . $what);
};
if (!$found->isInterface()) {
    $parent = $found->getParentClass();
    if ($extends === null && $parent !== false) {
        $mismatch('no parent in the source, PHP has ' . $parent->getName());
    }
    if ($extends !== null && ($parent === false || strcasecmp($extends, $parent->getName()) !== 0)) {
        $mismatch('extends ' . $extends . ', PHP has ' . ($parent === false ? 'no parent' : $parent->getName()));
    }
}
$interfaces = array_map('strtolower', $found->getInterfaceNames());
foreach ($found->isInterface() ? $header['extends'] : $implements as $name) {
    if (!in_array(strtolower($name), $interfaces, true)) {
        $mismatch($name . ' is not an interface PHP has for it');
    }
}
$parameter = static function (ReflectionParameter $p): array {
    $hasDefault = $p->isDefaultValueAvailable();
    return [
        'name' => $p->getName(),
        'type_hint' => tstr($p->getType()),
        'has_default' => $hasDefault,
        'default_value' => $hasDefault ? encode_default($p->getDefaultValue()) : null,
        'is_variadic' => $p->isVariadic(),
        'is_promoted' => $p->isPromoted(),
        'is_by_ref' => $p->isPassedByReference(),
    ];
};
$declares = static fn (ReflectionMethod $m): bool => strcasecmp($m->getDeclaringClass()->getName(), $found->getName()) === 0;
// A constructor this class declares, its traits' included; an inherited one is the parent's. An
// abstract constructor of a trait only asks for one.
$constructor = null;
$methods = [];
$reading = 'the constructor';
try {
    $method = $found->getConstructor();
    if ($method !== null && $declares($method) && !($found->isTrait() && $method->isAbstract())) {
        $constructor = array_map($parameter, $method->getParameters());
    }
    // The methods this class declares. PHP has already added those of its traits, with their
    // aliases and `insteadof` rules applied, as declared by this class.
    $reading = 'the methods';
    foreach ($found->getMethods() as $method) {
        if (!$declares($method) || strcasecmp($method->getName(), '__construct') === 0) {
            continue;
        }
        $methods[] = [
            'name' => $method->getName(),
            'params' => array_map($parameter, $method->getParameters()),
            'return_type' => tstr($method->getReturnType()),
            'is_static' => $method->isStatic(),
            'returns_reference' => $method->returnsReference(),
            'is_public' => $method->isPublic(),
            'is_final' => $method->isFinal(),
            'is_abstract' => $method->isAbstract(),
        ];
    }
} catch (\Throwable $e) {
    $fail('Could not read ' . $reading . ' of ' . $first . ': ' . $e->getMessage());
}
$kind = $found->isInterface() ? 'interface' : ($found->isTrait() ? 'trait' : ($found->isAbstract() ? 'abstract_class' : 'class'));
// Magento's scanner decides which classes the compile loads. It reads classes and traits only,
// so for those it must agree with the block rule; an interface is never its answer.
$scanner = 'Magento\\Setup\\Module\\Di\\Code\\Reader\\FileClassScanner';
if ($kind !== 'interface' && class_exists($scanner)) {
    try {
        $scan = new $scanner($file);
        $scanned = method_exists($scan, 'getClassName') ? $scan->getClassName() : (string) (((array) $scan->getClassNames())[0] ?? '');
    } catch (\Throwable $e) {
        $fail('Magento\'s FileClassScanner cannot read ' . $file . ': ' . $e->getMessage());
    }
    if ((strcasecmp(ltrim($scanned, '\\'), $first) === 0) === $insideBlock) {
        $fail('Magento\'s FileClassScanner gives ' . ($scanned === '' ? 'no class' : $scanned) . ' for ' . $file
            . ', but ' . $first . ' is ' . ($insideBlock ? 'inside a block' : 'at the top level'));
    }
}
$finished = true;
$write([
    'result' => 'class',
    'kind' => $kind,
    'namespace' => $found->getNamespaceName(),
    'name' => $found->getShortName(),
    'fqcn' => $found->getName(),
    'is_final' => $kind === 'class' && $found->isFinal(),
    'extends' => $extends,
    'implements' => $implements,
    'constructor' => $constructor,
    'methods' => $methods,
    'inside_block' => $insideBlock,
]);
"#;

fn script() -> &'static str {
    static SCRIPT: OnceLock<String> = OnceLock::new();
    SCRIPT.get_or_init(|| {
        format!(
            "{REFLECT_HEAD_PHP}{BOOT_PHP}{OBJECT_MANAGER_PHP}{ENCODE_DEFAULT_PHP}{TYPE_STRING_PHP}{REFLECT_PHP}"
        )
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
    is_final: bool,
    extends: Option<String>,
    implements: Vec<String>,
    constructor: Option<Vec<Tier3Param>>,
    methods: Vec<Tier3Method>,
    inside_block: bool,
}

#[derive(Deserialize)]
struct Tier3Param {
    name: String,
    type_hint: Option<String>,
    has_default: bool,
    default_value: Option<String>,
    is_variadic: bool,
    is_promoted: bool,
    is_by_ref: bool,
}

#[derive(Deserialize)]
struct Tier3Method {
    name: String,
    params: Vec<Tier3Param>,
    return_type: Option<String>,
    is_static: bool,
    returns_reference: bool,
    is_public: bool,
    is_final: bool,
    is_abstract: bool,
}

impl Tier3Class {
    fn into_class_info(self, path: &Path) -> ClassInfo {
        let kind = match self.kind {
            Tier3Kind::Class => ClassKind::Class,
            Tier3Kind::AbstractClass => ClassKind::AbstractClass,
            Tier3Kind::Interface => ClassKind::Interface,
            Tier3Kind::Trait => ClassKind::Trait,
        };
        // PHP has applied the trait rules, so `uses` and `rules` stay empty and `apply_traits`
        // leaves the class as it is.
        let mut traits = TraitData::default();
        let mut public_methods = Vec::new();
        for method in self.methods {
            let (is_public, is_final, is_abstract) =
                (method.is_public, method.is_final, method.is_abstract);
            let signature = method.into_signature();
            if kind == ClassKind::Trait {
                traits.methods.push(TraitMethod {
                    signature: signature.clone(),
                    is_public,
                    is_final,
                    is_abstract,
                });
            }
            if is_public && !is_final {
                public_methods.push(signature);
            } else {
                traits.hidden_methods.push(signature.name);
            }
        }
        ClassInfo {
            path: path.to_path_buf(),
            namespace: self.namespace,
            name: self.name,
            fqcn: self.fqcn,
            is_abstract: kind == ClassKind::AbstractClass,
            kind,
            extends: self.extends,
            implements: self.implements,
            constructor: self.constructor.map(|params| Constructor {
                params: params.into_iter().map(Tier3Param::into_param).collect(),
            }),
            is_final: self.is_final,
            public_methods,
            traits: Box::new(traits),
        }
    }
}

impl Tier3Method {
    fn into_signature(self) -> MethodSignature {
        MethodSignature {
            name: self.name,
            params: self
                .params
                .into_iter()
                .map(Tier3Param::into_method_param)
                .collect(),
            return_type: self.return_type,
            is_static: self.is_static,
            returns_reference: self.returns_reference,
        }
    }
}

impl Tier3Param {
    /// `is_optional` is PHP's `isDefaultValueAvailable()`. Tier 1 sets it from the source, which
    /// differs only for a default before a required parameter, and `optional_in_php` gives the
    /// same answer for both.
    fn into_param(self) -> ConstructorParam {
        ConstructorParam {
            is_primitive: self.type_hint.as_deref().is_none_or(is_primitive_type),
            name: self.name,
            type_hint: self.type_hint,
            is_optional: self.has_default,
            default_value: self.default_value,
            is_variadic: self.is_variadic,
            is_promoted: self.is_promoted,
        }
    }

    /// `has_default` is PHP's `isDefaultValueAvailable()`, as for a constructor parameter.
    fn into_method_param(self) -> MethodParam {
        MethodParam {
            name: self.name,
            type_hint: self.type_hint,
            has_default: self.has_default,
            default_value: self.default_value,
            is_variadic: self.is_variadic,
            is_by_ref: self.is_by_ref,
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
        match extract_tier3(Path::new("/nonexistent/Foo.php")).0 {
            ExtractResult::PhpFallbackFailed(reason) => {
                assert_eq!(reason, "Tier 3 is not configured")
            }
            other => panic!("expected PhpFallbackFailed, got {other:?}"),
        }
    }
}
