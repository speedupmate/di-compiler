//! `DiCompileCommand::getExcludedModulePaths()` excludes `<module>/Test` and `<module>/tests`.
//! A directory with that name deeper in a module is normal code, for example an admin
//! controller at `Controller/Adminhtml/Test/Run.php`. Unknown actions give a 404.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn write_fixture(root: &Path) {
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
        r#"<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <type name="Acme\On\Service\Test">
        <plugin name="watch" type="Acme\On\Plugin\Watch"/>
    </type>
</config>
"#,
    );
    write(
        root,
        "app/code/Acme/On/Controller/Adminhtml/Test/Run.php",
        "<?php\nnamespace Acme\\On\\Controller\\Adminhtml\\Test;\n\nclass Run\n{\n    \
         public function __construct(\\Acme\\On\\Model\\Gadget $gadget)\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/Gadget.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Gadget\n{\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Service/Test.php",
        "<?php\nnamespace Acme\\On\\Service;\n\nclass Test\n{\n    \
         public function __construct(\\Acme\\On\\Model\\Gadget $gadget)\n    {\n    }\n    \
         public function run()\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Plugin/Watch.php",
        "<?php\nnamespace Acme\\On\\Plugin;\n\nclass Watch\n{\n    \
         public function beforeRun($subject)\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Test/Unit/FooTest.php",
        "<?php\nnamespace Acme\\On\\Test\\Unit;\n\nclass FooTest\n{\n    \
         public function __construct(\\Acme\\On\\Model\\Gadget $gadget)\n    {\n    }\n}\n",
    );
}

fn compile(root: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .output()
        .unwrap()
}

fn compiled_fixture() -> Option<tempfile::TempDir> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return None;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    write_fixture(tmp.path());
    let output = compile(tmp.path());
    assert!(
        output.status.success(),
        "compiler failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(tmp)
}

fn metadata(root: &Path, file: &str) -> String {
    fs::read_to_string(root.join("generated/metadata").join(file)).unwrap()
}

#[test]
fn controller_in_a_directory_named_test_is_a_known_action() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let actions = metadata(tmp.path(), "app_action_list.php");
    assert!(
        actions.contains("acme\\\\on\\\\controller\\\\adminhtml\\\\test\\\\run"),
        "{actions}"
    );
}

#[test]
fn interceptor_of_a_class_named_test_gets_arguments() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let global = metadata(tmp.path(), "global.php");
    assert!(
        global.contains("'Acme\\\\On\\\\Service\\\\Test\\\\Interceptor' =>"),
        "{global}"
    );
}

#[test]
fn test_directory_at_module_root_is_still_skipped() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let global = metadata(tmp.path(), "global.php");
    assert!(!global.contains("FooTest"), "{global}");
}
