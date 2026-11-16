//! `app/etc/registration_globlist.php` registers `lib/internal/*/*/registration.php` and
//! `lib/internal/*/*/*/registration.php`. `DiCompileCommand` scans those libraries too.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn class_of_a_library_in_lib_internal_is_compiled() {
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
        "app/code/Acme/On/Model/Gadget.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Gadget\n{\n}\n",
    );
    write(
        root,
        "lib/internal/Acme/Lib/registration.php",
        "<?php\n\\Magento\\Framework\\Component\\ComponentRegistrar::register(\\Magento\\Framework\\Component\\ComponentRegistrar::LIBRARY, 'acme/lib', __DIR__);\n",
    );
    let class = "<?php\nnamespace Acme\\Lib;\n\nclass Thing\n{\n    public function __construct(\\Acme\\On\\Model\\Gadget $g)\n    {\n    }\n}\n";
    write(root, "lib/internal/Acme/Lib/Thing.php", class);
    write(
        root,
        "lib/internal/Acme/Lib/Test/Stub.php",
        &class
            .replace("Acme\\Lib;", "Acme\\Lib\\Test;")
            .replace("class Thing", "class Stub"),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .arg("--fallback-php")
        .arg(root.join("no-php-here"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let global = fs::read_to_string(root.join("generated/metadata/global.php")).unwrap();
    assert!(global.contains("'Acme\\\\Lib\\\\Thing' =>"));
    assert!(!global.contains("Acme\\\\Lib\\\\Test\\\\Stub"));
}

#[test]
fn folder_whose_registration_php_registers_nothing_is_not_compiled() {
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
        "lib/internal/Acme/Loader/registration.php",
        "<?php\n$loader = new AcmeLoader('Acme', __DIR__);\n$loader->register();\n",
    );
    write(
        root,
        "lib/internal/Acme/Loader/Thing.php",
        "<?php\nnamespace Acme\\Loader;\n\nclass Thing\n{\n    public function __construct(\\Acme\\On\\Model\\Gadget $g)\n    {\n    }\n}\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .arg("--fallback-php")
        .arg(root.join("no-php-here"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let global = fs::read_to_string(root.join("generated/metadata/global.php")).unwrap();
    assert!(!global.contains("Acme\\\\Loader\\\\Thing"));
}
