//! Magento reads `app/etc/di.xml`, `app/etc/*/di.xml`, and the di.xml files of the modules that
//! `app/etc/config.php` enables. A folder that is no module of that list contributes nothing,
//! even when it sits in `app/code` or in a package with a `registration.php`.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn has_php() -> bool {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return false;
    }
    true
}

fn preference(target: &str) -> String {
    format!("<config><preference for=\"Acme\\On\\Model\\Gadget\" type=\"{target}\"/></config>\n")
}

/// A store with the enabled module `Acme_On` and the given extra files.
fn compile(files: &[(&str, &str)]) -> (tempfile::TempDir, Output) {
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
    for name in ["Gadget", "Widget"] {
        write(
            root,
            &format!("app/code/Acme/On/Model/{name}.php"),
            &format!("<?php\nnamespace Acme\\On\\Model;\n\nclass {name}\n{{\n}}\n"),
        );
    }
    for (path, content) in files {
        write(root, path, content);
    }
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .output()
        .unwrap();
    (tmp, output)
}

fn assert_compiles(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn gadget_preference(store: &tempfile::TempDir) -> Option<String> {
    let global = fs::read_to_string(store.path().join("generated/metadata/global.php")).unwrap();
    global
        .lines()
        .find(|line| {
            line.trim_start()
                .starts_with("'Acme\\\\On\\\\Model\\\\Gadget' => '")
        })
        .map(|line| line.trim().to_string())
}

const WIDGET: &str = "'Acme\\\\On\\\\Model\\\\Gadget' => 'Acme\\\\On\\\\Model\\\\Widget',";

#[test]
fn a_missing_target_in_a_folder_that_is_no_module_does_not_stop_the_compile() {
    if !has_php() {
        return;
    }
    let (_store, output) = compile(&[(
        "app/code/Acme/NotAModule/etc/di.xml",
        &preference("Acme\\NotAModule\\Model\\DoesNotExist"),
    )]);
    assert_compiles(&output);
}

#[test]
fn a_preference_in_a_folder_that_is_no_module_is_not_applied() {
    if !has_php() {
        return;
    }
    let (store, output) = compile(&[
        // Its own registrar, as a connector framework has: Magento does not know the folder.
        (
            "app/code/Acme/NotAModule/registration.php",
            "<?php\n\\Other\\Registrar::register('Acme_NotAModule', __DIR__);\n",
        ),
        (
            "app/code/Acme/NotAModule/etc/di.xml",
            &preference("Acme\\On\\Model\\Widget"),
        ),
    ]);
    assert_compiles(&output);
    assert_eq!(gadget_preference(&store), None);
}

#[test]
fn a_vendor_package_without_a_module_adds_no_di_xml() {
    if !has_php() {
        return;
    }
    let (store, output) = compile(&[
        (
            "vendor/acme/library/registration.php",
            "<?php\n\\Magento\\Framework\\Component\\ComponentRegistrar::register(\
             \\Magento\\Framework\\Component\\ComponentRegistrar::LIBRARY, 'acme/library', __DIR__);\n",
        ),
        (
            "vendor/acme/library/etc/di.xml",
            &preference("Acme\\On\\Model\\Widget"),
        ),
    ]);
    assert_compiles(&output);
    assert_eq!(gadget_preference(&store), None);
}

#[test]
fn a_module_that_config_php_does_not_list_adds_no_di_xml() {
    if !has_php() {
        return;
    }
    let (store, output) = compile(&[
        ("app/code/Acme/Off/registration.php", "<?php\n"),
        (
            "app/code/Acme/Off/etc/module.xml",
            "<config><module name=\"Acme_Off\"/></config>\n",
        ),
        (
            "app/code/Acme/Off/etc/di.xml",
            &preference("Acme\\On\\Model\\Widget"),
        ),
    ]);
    assert_compiles(&output);
    assert_eq!(gadget_preference(&store), None);
}

#[test]
fn the_di_xml_of_an_enabled_module_and_of_app_etc_still_apply() {
    if !has_php() {
        return;
    }
    for path in [
        "app/code/Acme/On/etc/di.xml",
        "app/code/Acme/On/etc/frontend/di.xml",
        "app/etc/di.xml",
        "app/etc/local/di.xml",
    ] {
        let (store, output) = compile(&[(path, &preference("Acme\\On\\Model\\Widget"))]);
        assert_compiles(&output);
        let metadata = if path.contains("frontend") {
            "frontend.php"
        } else {
            "global.php"
        };
        let area =
            fs::read_to_string(store.path().join("generated/metadata").join(metadata)).unwrap();
        assert!(area.contains(WIDGET), "{path}: no preference in {metadata}");
    }
}

/// `extension_attributes.xml` comes through `Module\Dir\Reader` too. Read, this one would stop
/// the compile.
#[test]
fn an_extension_attributes_xml_in_a_folder_that_is_no_module_is_not_read() {
    if !has_php() {
        return;
    }
    let (_store, output) = compile(&[
        ("app/code/Acme/NotAModule/registration.php", "<?php\n"),
        (
            "app/code/Acme/NotAModule/etc/extension_attributes.xml",
            "<config><extension_attributes for=\"Acme\\On\\Api\\Data\\GadgetInterface\">",
        ),
    ]);
    assert_compiles(&output);
}
