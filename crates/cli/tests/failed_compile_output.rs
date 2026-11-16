//! `DiCompileCommand::execute()` empties `generated/metadata` before it compiles, so a compile that
//! stops never leaves the metadata of an earlier run behind for the store to use.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn store_with(files: &[(&str, &str)]) -> Option<tempfile::TempDir> {
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
         $file = dirname(__DIR__) . '/app/code/' . str_replace('\\\\', '/', $class) . '.php';\n    \
         if (is_file($file)) {\n        require $file;\n    }\n});\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/Gadget.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Gadget\n{\n}\n",
    );
    write(
        root,
        "generated/metadata/global.php",
        "<?php return ['old' => true];\n",
    );
    for (relative, content) in files {
        write(root, relative, content);
    }
    Some(tmp)
}

fn compile(root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .output()
        .unwrap()
}

fn assert_stopped_without_metadata(tmp: &tempfile::TempDir) {
    let output = compile(tmp.path());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        !tmp.path().join("generated/metadata/global.php").exists(),
        "{stderr}"
    );
}

#[test]
fn class_that_cannot_load_leaves_no_metadata() {
    let Some(tmp) = store_with(&[(
        "app/code/Acme/On/Model/Thing.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Thing extends Gone\n{\n}\n",
    )]) else {
        return;
    };
    assert_stopped_without_metadata(&tmp);
}

#[test]
fn missing_plugin_class_leaves_no_metadata() {
    let Some(tmp) = store_with(&[(
        "app/code/Acme/On/etc/di.xml",
        "<config><type name=\"Acme\\On\\Model\\Gadget\"><plugin name=\"p\" type=\"Acme\\On\\Plugin\\Gone\"/></type></config>\n",
    )]) else {
        return;
    };
    assert_stopped_without_metadata(&tmp);
}

#[test]
fn missing_preference_target_leaves_no_metadata() {
    let Some(tmp) = store_with(&[(
        "app/code/Acme/On/etc/di.xml",
        "<config><preference for=\"Acme\\On\\Model\\Gadget\" type=\"Acme\\On\\Model\\Gone\"/></config>\n",
    )]) else {
        return;
    };
    assert_stopped_without_metadata(&tmp);
}

#[test]
fn a_passing_compile_replaces_the_metadata() {
    let Some(tmp) = store_with(&[]) else {
        return;
    };
    let output = compile(tmp.path());
    assert!(output.status.success());
    let global = fs::read_to_string(tmp.path().join("generated/metadata/global.php")).unwrap();
    assert!(!global.contains("'old'"));
}
