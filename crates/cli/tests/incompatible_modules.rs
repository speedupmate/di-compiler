//! A module on the incompatibility list stops the compile when its package is installed, enabled
//! or disabled, or when `app/etc/config.php` enables it. Magento 2.4.8 reads the configuration of a
//! disabled module too, so a disabled module still changes Magento's output. This is a deliberate
//! difference from Magento, whose own compile runs fine with such a module.
//! The stop comes first, so the output of an earlier compile stays as it was.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MODULE: &str = "Creatuity_Interception";

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// A store with the module on disk.
fn store(config_php: &str) -> Option<tempfile::TempDir> {
    store_with(config_php, true)
}

/// A store with the module on disk when `installed`, or without its package.
fn store_with(config_php: &str, installed: bool) -> Option<tempfile::TempDir> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return None;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "app/etc/config.php", config_php);
    let mut modules = vec![("Acme/On", "Acme_On")];
    if installed {
        modules.push(("Creatuity/Interception", MODULE));
    }
    for (folder, module) in modules {
        write(
            root,
            &format!("app/code/{folder}/registration.php"),
            "<?php\n",
        );
        write(
            root,
            &format!("app/code/{folder}/etc/module.xml"),
            &format!("<config><module name=\"{module}\"/></config>\n"),
        );
    }
    write(
        root,
        "vendor/autoload.php",
        "<?php\nspl_autoload_register(function ($class) {\n    \
         $file = dirname(__DIR__) . '/app/code/' . str_replace('\\\\', '/', $class) . '.php';\n    \
         if (is_file($file)) {\n        require $file;\n    }\n});\n",
    );
    Some(tmp)
}

fn config(status: &str) -> String {
    format!("<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n{status}    ],\n];\n")
}

fn enabled() -> String {
    config(&format!("        '{MODULE}' => 1,\n"))
}

fn compile(root: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .args(extra)
        .output()
        .unwrap()
}

fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else {
                let content = fs::read(&path).unwrap();
                files.insert(path, content);
            }
        }
    }
    files
}

fn assert_stopped(output: &Output) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    for expected in [
        MODULE,
        "creatuity/magento2-interceptors",
        "Interceptor generator",
        "Remove the package with composer",
        "bin/magento setup:di:compile",
    ] {
        assert!(stderr.contains(expected), "no {expected} in: {stderr}");
    }
}

fn assert_stopped_saying(output: &Output, state: &str) {
    assert_stopped(output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(state), "no {state} in: {stderr}");
}

#[test]
fn enabled_module_stops_and_leaves_earlier_output_alone() {
    let Some(tmp) = store(&enabled()) else {
        return;
    };
    let root = tmp.path();
    write(
        root,
        "generated/metadata/global.php",
        "<?php return ['old' => true];\n",
    );
    write(
        root,
        "generated/code/Acme/On/Old/Interceptor.php",
        "<?php\n",
    );
    let before = tree(&root.join("generated"));
    assert_stopped(&compile(root, &[]));
    assert_eq!(tree(&root.join("generated")), before);
}

#[test]
fn enabled_module_stops_before_the_output_folder_exists() {
    let Some(tmp) = store(&enabled()) else {
        return;
    };
    assert_stopped(&compile(tmp.path(), &[]));
    assert!(!tmp.path().join("generated").exists());
}

#[test]
fn enabled_module_in_compact_config_stops() {
    let compact = format!("<?php return ['modules' => ['Acme_On' => 1, '{MODULE}' => 1]];\n");
    let Some(tmp) = store(&compact) else {
        return;
    };
    assert_stopped(&compile(tmp.path(), &[]));
}

#[test]
fn enabled_module_stops_when_php_cannot_read_the_config() {
    let Some(tmp) = store(&enabled()) else {
        return;
    };
    assert_stopped(&compile(
        tmp.path(),
        &["--fallback-php", "/nonexistent/php"],
    ));
}

#[test]
fn enabled_module_says_it_is_installed_and_enabled() {
    let Some(tmp) = store(&enabled()) else {
        return;
    };
    assert_stopped_saying(&compile(tmp.path(), &[]), "is installed and enabled");
}

#[test]
fn installed_module_set_to_0_stops_and_writes_nothing() {
    let Some(tmp) = store(&config(&format!("        '{MODULE}' => 0,\n"))) else {
        return;
    };
    assert_stopped_saying(&compile(tmp.path(), &[]), "is installed but disabled");
    assert!(!tmp.path().join("generated").exists());
}

#[test]
fn installed_module_missing_from_the_config_stops() {
    let Some(tmp) = store(&config("")) else {
        return;
    };
    assert_stopped_saying(&compile(tmp.path(), &[]), "is installed but disabled");
}

#[test]
fn installed_module_set_to_0_stops_when_php_cannot_read_the_config() {
    let Some(tmp) = store(&config(&format!("        '{MODULE}' => 0,\n"))) else {
        return;
    };
    assert_stopped_saying(
        &compile(tmp.path(), &["--fallback-php", "/nonexistent/php"]),
        "is installed but disabled",
    );
}

#[test]
fn enabled_module_without_its_package_stops() {
    let Some(tmp) = store_with(&enabled(), false) else {
        return;
    };
    assert_stopped_saying(
        &compile(tmp.path(), &[]),
        "is enabled in app/etc/config.php",
    );
}

/// `composer remove` leaves the module in `app/etc/config.php`. Magento has nothing to read then.
#[test]
fn removed_package_set_to_0_does_not_stop_the_compile() {
    let Some(tmp) = store_with(&config(&format!("        '{MODULE}' => 0,\n")), false) else {
        return;
    };
    let output = compile(tmp.path(), &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains(MODULE), "{stderr}");
    assert!(tmp.path().join("generated/metadata/global.php").exists());
}

#[test]
fn store_without_the_package_compiles() {
    let Some(tmp) = store_with(&config(""), false) else {
        return;
    };
    let output = compile(tmp.path(), &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(tmp.path().join("generated/metadata/global.php").exists());
}
