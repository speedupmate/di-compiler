//! A file no tier can read stops the compile when Magento needs it: when Magento's scanner finds
//! its class, or when active configuration or a compiled class uses a name it declares. Otherwise
//! it is left out with a warning, as before. When it cannot be told, the compile stops too.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Skips without php, as the other tests here do, and fails on a php older than 8.4: property
/// hooks need 8.4.
fn has_php_8_4() -> bool {
    let Ok(output) = Command::new("php")
        .args(["-r", "echo PHP_VERSION_ID, ' ', PHP_VERSION;"])
        .output()
    else {
        eprintln!("skipped: these tests need php on the PATH");
        return false;
    };
    let version = String::from_utf8_lossy(&output.stdout).into_owned();
    let (id, text) = version.split_once(' ').unwrap_or(("0", version.as_str()));
    assert!(
        id.parse::<u32>().unwrap_or(0) >= 80400,
        "these tests need PHP 8.4 or later, found {text}"
    );
    true
}

/// Reads only classes and traits, at the top level or in a braced namespace, as Magento's does.
const FILE_CLASS_SCANNER: &str = r#"<?php
namespace Magento\Setup\Module\Di\Code\Reader;

class FileClassScanner
{
    public function __construct(private string $filename)
    {
    }

    public function getClassName(): string
    {
        $namespace = '';
        $level = 0;
        $braced = false;
        $tokens = \PhpToken::tokenize((string) file_get_contents($this->filename));
        foreach ($tokens as $i => $token) {
            if ($token->text === '{') {
                $level++;
            } elseif ($token->text === '}') {
                $level--;
            } elseif ($token->is(T_NAMESPACE)) {
                for ($j = $i + 1; $tokens[$j]->isIgnorable(); $j++) {
                }
                $namespace = $tokens[$j]->is([T_STRING, T_NAME_QUALIFIED]) ? $tokens[$j]->text : '';
                for ($k = $j; !in_array($tokens[$k]->text, [';', '{'], true); $k++) {
                }
                $braced = $tokens[$k]->text === '{';
            } elseif ($token->is([T_CLASS, T_TRAIT]) && ($level === 0 || ($braced && $level === 1))) {
                for ($j = $i + 1; $tokens[$j]->isIgnorable(); $j++) {
                }
                if ($tokens[$j]->is(T_STRING)) {
                    return ($namespace === '' ? '' : $namespace . '\\') . $tokens[$j]->text;
                }
            }
        }
        return '';
    }
}
"#;

/// Tier 1 rejects the intersection type, tree-sitter-php does not know property hooks, and PHP
/// cannot load the class because its parent does not exist.
const UNREAD_BODY: &str = "class Lost extends \\Acme\\Missing\\Base\n{\n    \
     public string $label { get => 'lost'; }\n\n    \
     public function __construct(\\Countable&\\Traversable $items)\n    {\n    }\n}\n";

fn top_level() -> String {
    format!("<?php\nnamespace Acme\\On\\Model;\n\n{UNREAD_BODY}")
}

fn inside_if() -> String {
    format!("<?php\nnamespace Acme\\On\\Model;\n\nif (true) {{\n{UNREAD_BODY}}}\n")
}

/// Compiles a store with module `Acme_On`, the given files under `app/code/Acme/On`, and
/// `di.xml`. `scanner` adds Magento's `FileClassScanner` under `setup/src`.
fn compile(files: &[(&str, &str)], di_xml: &str, scanner: bool) -> Output {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n    ],\n];\n",
    );
    write(root, "app/code/Acme/On/registration.php", "<?php\n");
    write(
        root,
        "app/code/Acme/On/etc/module.xml",
        "<config><module name=\"Acme_On\"/></config>\n",
    );
    write(
        root,
        "vendor/autoload.php",
        "<?php\nspl_autoload_register(function ($class) {\n    \
         foreach (['/app/code/', '/setup/src/'] as $base) {\n        \
         $file = dirname(__DIR__) . $base . str_replace('\\\\', '/', $class) . '.php';\n        \
         if (is_file($file)) {\n            require $file;\n            return;\n        }\n    }\n});\n",
    );
    if scanner {
        write(
            root,
            "setup/src/Magento/Setup/Module/Di/Code/Reader/FileClassScanner.php",
            FILE_CLASS_SCANNER,
        );
    }
    for (relative, source) in files {
        write(root, &format!("app/code/Acme/On/{relative}"), source);
    }
    write(root, "app/code/Acme/On/etc/di.xml", di_xml);
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .output()
        .unwrap();
    // Nothing is written when the compile stops for an unread file.
    if !output.status.success() {
        assert!(
            !root.join("generated/metadata/global.php").exists(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    output
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stopped_with(output: &Output, parts: &[&str]) {
    let log = stderr(output);
    assert!(!output.status.success(), "the compile did not stop\n{log}");
    let error = log
        .lines()
        .find(|line| line.starts_with("error: No tier could read"))
        .unwrap_or_else(|| panic!("no unread-file error in\n{log}"));
    for part in parts {
        assert!(error.contains(part), "{part:?} not in {error}");
    }
}

const NO_DI: &str = "<config/>\n";
const FILE: &str = "Model/Lost.php";

#[test]
fn a_file_magento_scans_stops_the_compile_with_each_tier_s_reason() {
    if !has_php_8_4() {
        return;
    }
    let output = compile(&[(FILE, &top_level())], NO_DI, true);
    stopped_with(
        &output,
        &[
            "Acme/On/Model/Lost.php",
            "Tier 1: unsupported syntax: intersection_type",
            "Tier 2: tree-sitter-php found a syntax error",
            "Tier 3: ",
            "Acme\\Missing\\Base",
            "Magento compiles its class Acme\\On\\Model\\Lost",
            "a passing PHP class check only shows that the file loads",
        ],
    );
}

#[test]
fn a_file_magento_does_not_scan_and_nothing_uses_is_left_out_with_a_warning() {
    if !has_php_8_4() {
        return;
    }
    let output = compile(&[(FILE, &inside_if())], NO_DI, true);
    let log = stderr(&output);
    assert!(output.status.success(), "{log}");
    assert!(log.contains("Fallback failed for "), "{log}");
    assert!(
        log.contains("Lost.php out of the compile: Magento does not compile it"),
        "{log}"
    );
}

#[test]
fn a_preference_on_a_class_of_an_unscanned_file_stops_the_compile() {
    if !has_php_8_4() {
        return;
    }
    let output = compile(
        &[
            (FILE, &inside_if()),
            (
                "Model/LostInterface.php",
                "<?php\nnamespace Acme\\On\\Model;\n\ninterface LostInterface\n{\n}\n",
            ),
        ],
        "<config><preference for=\"Acme\\On\\Model\\LostInterface\" type=\"Acme\\On\\Model\\Lost\"/></config>\n",
        true,
    );
    stopped_with(
        &output,
        &[
            "Magento does not compile it, but it declares Acme\\On\\Model\\Lost",
            "used by the preference for Acme\\On\\Model\\LostInterface",
        ],
    );
}

#[test]
fn a_compiled_class_that_needs_a_class_of_an_unscanned_file_stops_the_compile() {
    if !has_php_8_4() {
        return;
    }
    let output = compile(
        &[
            (FILE, &inside_if()),
            (
                "Model/Needs.php",
                "<?php\nnamespace Acme\\On\\Model;\n\nclass Needs\n{\n    public function __construct(Lost $lost)\n    {\n    }\n}\n",
            ),
        ],
        NO_DI,
        true,
    );
    stopped_with(
        &output,
        &["used by the constructor parameter $lost of Acme\\On\\Model\\Needs"],
    );
}

#[test]
fn a_store_without_magento_s_scanner_stops_the_compile() {
    if !has_php_8_4() {
        return;
    }
    let output = compile(&[(FILE, &inside_if())], NO_DI, false);
    stopped_with(
        &output,
        &[
            "Acme/On/Model/Lost.php",
            "the store has no Magento\\Setup\\Module\\Di\\Code\\Reader\\FileClassScanner",
        ],
    );
}

#[test]
fn a_file_php_cannot_tokenize_stops_the_compile() {
    if !has_php_8_4() {
        return;
    }
    // The closing brace of `if` is missing: no tier reads it, the scanner sees no class at the
    // top level, and PHP's tokenizer cannot list what it declares.
    let broken = format!("<?php\nnamespace Acme\\On\\Model;\n\nif (true) {{\n{UNREAD_BODY}");
    let output = compile(&[(FILE, &broken)], NO_DI, true);
    stopped_with(
        &output,
        &["PHP's tokenizer cannot list the classes it declares"],
    );
}
