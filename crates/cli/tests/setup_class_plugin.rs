//! `Operation\Interception` reads `intercepted_paths`, which are the application, library and
//! generated code paths and not `setup/src`. A plugin on a setup class therefore gives no
//! Interceptor, unless a class on those paths inherits from it. `PluginList::_inheritPlugins`
//! then also walks that parent.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn compile(root: &Path) -> String {
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
        "app/code/Acme/On/etc/di.xml",
        "<config>\n    <type name=\"Magento\\Setup\\Model\\Tool\">\n        <plugin name=\"acme\" type=\"Acme\\On\\Plugin\\Watch\"/>\n    </type>\n</config>\n",
    );
    write(
        root,
        "app/code/Acme/On/Plugin/Watch.php",
        "<?php\nnamespace Acme\\On\\Plugin;\n\nclass Watch\n{\n    public function beforeRun($subject)\n    {\n    }\n}\n",
    );
    write(
        root,
        "setup/src/Magento/Setup/Model/Tool.php",
        "<?php\nnamespace Magento\\Setup\\Model;\n\nclass Tool\n{\n    public function run()\n    {\n    }\n}\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        // No PHP on purpose, so this test only depends on the static scan.
        .arg("--fallback-php")
        .arg(root.join("no-php-here"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::read_to_string(root.join("generated/metadata/global.php")).unwrap()
}

#[test]
fn plugin_on_a_setup_class_gives_no_interceptor() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    let metadata = compile(root);
    assert!(!root
        .join("generated/code/Magento/Setup/Model/Tool/Interceptor.php")
        .exists());
    assert!(!metadata.contains("Magento\\\\Setup\\\\Model\\\\Tool\\\\Interceptor"));
}

#[test]
fn setup_class_with_a_child_in_a_module_keeps_its_interceptor() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/code/Acme/On/Model/Child.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Child extends \\Magento\\Setup\\Model\\Tool\n{\n}\n",
    );
    compile(root);
    assert!(root
        .join("generated/code/Magento/Setup/Model/Tool/Interceptor.php")
        .exists());
}
