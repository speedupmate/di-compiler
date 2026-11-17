//! `ComponentRegistrar::register()` takes the path of a component, and that does not have to be
//! the folder of `registration.php`. Magento reads the component from that path.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn store() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n        'Acme_Red' => 1,\n    ],\n];\n",
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
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Gadget\n{\n    public function run()\n    {\n    }\n}\n",
    );
    tmp
}

fn compile(root: &Path) {
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
}

fn write_redirected_module(root: &Path, package: &str, registration: &str) {
    write(root, &format!("{package}/registration.php"), registration);
    write(
        root,
        &format!("{package}/src/etc/module.xml"),
        "<config><module name=\"Acme_Red\"/></config>\n",
    );
    write(
        root,
        &format!("{package}/src/etc/di.xml"),
        "<config><type name=\"Acme\\On\\Model\\Gadget\"><plugin name=\"red\" type=\"Acme\\Red\\Plugin\\Watch\"/></type></config>\n",
    );
    write(
        root,
        &format!("{package}/src/Plugin/Watch.php"),
        "<?php\nnamespace Acme\\Red\\Plugin;\n\nclass Watch\n{\n    public function beforeRun($s)\n    {\n    }\n\n    public function __construct(\\Acme\\On\\Model\\Gadget $g)\n    {\n    }\n}\n",
    );
    write(
        root,
        &format!("{package}/src/Test/Stub.php"),
        "<?php\nnamespace Acme\\Red\\Test;\n\nclass Stub\n{\n    public function __construct(\\Acme\\On\\Model\\Gadget $g)\n    {\n    }\n}\n",
    );
}

fn assert_module_is_compiled(root: &Path) {
    compile(root);
    let plugin_list = fs::read_dir(root.join("generated/metadata"))
        .unwrap()
        .flatten()
        .map(|entry| fs::read_to_string(entry.path()).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plugin_list.contains("'red'"),
        "plugin of the module is lost"
    );
    let global = fs::read_to_string(root.join("generated/metadata/global.php")).unwrap();
    assert!(global.contains("Acme\\\\Red\\\\Plugin\\\\Watch"));
    assert!(!global.contains("Acme\\\\Red\\\\Test\\\\Stub"));
    assert!(root
        .join("generated/code/Acme/On/Model/Gadget/Interceptor.php")
        .exists());
}

const REGISTRATION: &str = "<?php\n\\Magento\\Framework\\Component\\ComponentRegistrar::register(\n    \\Magento\\Framework\\Component\\ComponentRegistrar::MODULE,\n    'Acme_Red',\n    __DIR__ . '/src'\n);\n";

#[test]
fn vendor_module_registered_in_a_subfolder_is_compiled() {
    let tmp = store();
    write_redirected_module(tmp.path(), "vendor/acme/red", REGISTRATION);
    assert_module_is_compiled(tmp.path());
}

#[test]
fn app_code_module_registered_in_a_subfolder_is_compiled() {
    let tmp = store();
    write_redirected_module(tmp.path(), "app/code/Acme/Red", REGISTRATION);
    assert_module_is_compiled(tmp.path());
}

#[test]
fn double_quotes_and_no_spaces_work_too() {
    let tmp = store();
    write_redirected_module(
        tmp.path(),
        "vendor/acme/red",
        "<?php\nuse Magento\\Framework\\Component\\ComponentRegistrar;\nComponentRegistrar::register(ComponentRegistrar::MODULE,'Acme_Red',__DIR__.\"/src\");\n",
    );
    assert_module_is_compiled(tmp.path());
}
