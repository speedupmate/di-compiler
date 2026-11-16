//! Checks every di.xml the way Magento reads it.
//!
//! `Config\Reader\Filesystem::_readFiles()` hands each file to `Config\Dom::_initDom()`, which
//! stops the compile when `DOMDocument::loadXML()` fails. The Rust parser accepts more than
//! libxml does, so PHP answers this question. `Scanner\XmlScanner::collectEntities()` then looks
//! for proxies in the same files. It runs beside the rest of the compile.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::JoinHandle;

use crate::class_checks::{BOOT_PHP, OBJECT_MANAGER_PHP};

const VALID_XML_PHP: &str = r#"
ini_set('display_errors', '0');
[, $root, $generated] = $argv;
libxml_use_internal_errors(true);
$input = file('php://stdin', FILE_IGNORE_NEW_LINES);
$split = array_search('', $input, true);
$files = array_slice($input, 0, $split);
$attributeFiles = array_slice($input, $split + 1);
foreach ($files as $file) {
    libxml_clear_errors();
    try {
        $loaded = (new DOMDocument())->loadXML((string) file_get_contents($file));
        $messages = [];
        foreach (libxml_get_errors() as $error) {
            $messages[] = trim($error->message) . "\nLine: " . $error->line;
        }
    } catch (\Throwable $e) {
        $loaded = false;
        $messages = [$e->getMessage()];
    }
    if (!$loaded) {
        echo "invalid\n", 'The XML in file "', $file, "\" is invalid:\n", implode("\n", $messages),
            "\nVerify the XML and try again.\n";
        exit(0);
    }
}
if (!is_file($root . '/vendor/autoload.php')) {
    echo "valid\n";
    exit(0);
}
"#;

const PROXIES_PHP: &str = r#"
ob_start();
// `_initDom()` was the only reader that asks for these errors. A warning goes to the error handler.
libxml_use_internal_errors(false);
$names = [];
$pattern = '/^(\s+)?(.*)\\\(.*)Proxy(\s+)?$/';
$query = "/config/preference[ php:functionString('preg_match', '{$pattern}', @type) > 0]/@type | "
    . "//argument[@xsi:type='object' and php:functionString('preg_match', '{$pattern}', text()) > 0] |"
    . "//item[@xsi:type='object' and php:functionString('preg_match', '{$pattern}', text()) > 0] |"
    . "/config/virtualType[ php:functionString('preg_match', '{$pattern}', @type) > 0]/@type";
foreach ($files as $file) {
    $dom = new DOMDocument();
    try {
        $dom->load($file);
    } catch (\Exception $e) {
        while (ob_get_level() > 0) {
            ob_end_clean();
        }
        echo "invalid\n", $e->getMessage(), "\n";
        exit(0);
    }
    $xpath = new DOMXPath($dom);
    $xpath->registerNamespace('php', 'http://php.net/xpath');
    $xpath->registerPhpFunctions('preg_match');
    // A file that does not declare the xsi prefix gives false here. Magento passes those too.
    foreach (@$xpath->query($query) ?: [] as $node) {
        $names[ltrim(trim($node->nodeValue), '\\')] = true;
    }
}
// `ServiceDataAttributesScanner::collectEntities()` reads these with plain `loadXML()`.
foreach ($attributeFiles as $file) {
    try {
        $loaded = (new DOMDocument())->loadXML((string) file_get_contents($file));
        $message = 'loadXML() failed';
    } catch (\Throwable $e) {
        $loaded = false;
        $message = $e->getMessage();
    }
    if (!$loaded) {
        while (ob_get_level() > 0) {
            ob_end_clean();
        }
        echo "invalid\nThe XML in file \"", $file, "\" is invalid: ", $message, "\n";
        exit(0);
    }
}
$invalid = [];
foreach (array_keys($names) as $name) {
    $source = str_ends_with($name, '\\Proxy') ? substr($name, 0, -6) : $name;
    try {
        $exists = class_exists($name);
    } catch (\RuntimeException) {
        $exists = false;
    }
    if (!$exists && !class_exists($source) && !interface_exists($source)) {
        $invalid[] = $name . ': Invalid proxy class for ' . rtrim(substr($name, 0, -5), '\\');
    }
}
while (ob_get_level() > 0) {
    ob_end_clean();
}
if ($invalid !== []) {
    echo "invalid\nErrors during configuration scanning:\n", implode("\n", $invalid), "\n";
    exit(0);
}
echo "valid\n";
"#;

pub(crate) struct DiXmlCheck(JoinHandle<Result<(), String>>);

impl DiXmlCheck {
    pub(crate) fn start(
        files: Vec<PathBuf>,
        attribute_files: Vec<PathBuf>,
        root: &Path,
        code_root: &Path,
        php_bin: String,
    ) -> Self {
        let root = root.to_path_buf();
        let code_root = code_root.to_path_buf();
        Self(std::thread::spawn(move || {
            check(&files, &attribute_files, &root, &code_root, &php_bin)
        }))
    }

    /// The message Magento stops with, if it stops.
    pub(crate) fn finish(self) -> Result<(), String> {
        self.0
            .join()
            .unwrap_or_else(|_| Err("The di.xml check ended unexpectedly".to_string()))
    }
}

fn check(
    files: &[PathBuf],
    attribute_files: &[PathBuf],
    root: &Path,
    code_root: &Path,
    php_bin: &str,
) -> Result<(), String> {
    let could_not_run = |reason: String| format!("Could not check di.xml files in PHP: {reason}");
    let spawned = Command::new(php_bin)
        .args(["-d", "display_errors=0", "-r"])
        .arg(format!(
            "{VALID_XML_PHP}{BOOT_PHP}{OBJECT_MANAGER_PHP}{PROXIES_PHP}"
        ))
        .arg("--")
        .arg(root)
        .arg(code_root)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        // Without a PHP binary nothing can run in PHP, here or in any other phase.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::warn!("Could not check di.xml files: no PHP binary at {php_bin}");
            return Ok(());
        }
        Err(e) => return Err(could_not_run(e.to_string())),
    };
    let mut input = String::new();
    for file in files {
        input.push_str(&file.to_string_lossy());
        input.push('\n');
    }
    input.push('\n');
    for file in attribute_files {
        input.push_str(&file.to_string_lossy());
        input.push('\n');
    }
    let mut stdin = child.stdin.take().expect("stdin is piped");
    // PHP reads all of stdin first, so this cannot block on PHP's output.
    stdin
        .write_all(input.as_bytes())
        .map_err(|e| could_not_run(e.to_string()))?;
    drop(stdin);
    let output = child
        .wait_with_output()
        .map_err(|e| could_not_run(e.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    match stdout.split_once('\n') {
        Some(("valid", _)) => Ok(()),
        Some(("invalid", message)) => Err(message.trim_end().to_string()),
        _ => Err(could_not_run(format!(
            "no answer, PHP exit code {:?}",
            output.status.code()
        ))),
    }
}
