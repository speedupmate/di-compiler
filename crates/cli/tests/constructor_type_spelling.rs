//! `ClassReader::getConstructor()` stores `ReflectionClass::getName()` of a parameter's class. That
//! is the name the file declares, not the spelling the constructor uses. A library that declares
//! two spellings in its PSR-4 map loads under both, but an authoritative class map only knows the
//! declared one.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn compile_with_library_class() -> Option<String> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return None;
    }
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
         $file = stripos($class, 'Lib\\\\') === 0\n        \
         ? dirname(__DIR__) . '/vendor/acme/lib/src/' . substr($class, 4) . '.php'\n        \
         : dirname(__DIR__) . '/app/code/' . str_replace('\\\\', '/', $class) . '.php';\n    \
         if (is_file($file)) {\n        require $file;\n    }\n});\n",
    );
    write(
        root,
        "vendor/composer/installed.json",
        r#"{"packages": [{"name": "acme/lib", "install-path": "../acme/lib",
            "autoload": {"psr-4": {"Lib\\": "src/", "LIB\\": "src/"}}}]}"#,
    );
    write(
        root,
        "vendor/acme/lib/src/Attr.php",
        "<?php\nnamespace LIB;\n\nclass Attr\n{\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/Thing.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nuse Lib\\Attr;\n\nclass Thing\n{\n    public function __construct(Attr $attr)\n    {\n    }\n}\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(fs::read_to_string(root.join("generated/metadata/global.php")).unwrap())
}

#[test]
fn a_library_class_gets_the_spelling_it_declares() {
    let Some(metadata) = compile_with_library_class() else {
        return;
    };
    assert!(metadata.contains("LIB\\\\Attr"), "{metadata}");
    assert!(!metadata.contains("Lib\\\\Attr"), "{metadata}");
}
