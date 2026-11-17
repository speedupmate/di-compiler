//! `FileClassScanner::extract()` only takes a class outside any block, so `ClassesScanner` never
//! lists a class that a file declares inside `if`. Such a class has no arguments entry, and the
//! object manager reflects it at runtime, after PHP has chosen the branch.

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
        "vendor/composer/installed.json",
        "{\"packages\": []}\n",
    );
    write(
        root,
        "app/code/Acme/On/etc/module.xml",
        "<config><module name=\"Acme_On\"/></config>\n",
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
fn class_inside_if_has_no_arguments_entry() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/code/Acme/On/Model/Base.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Base\n{\n    public function __construct(Dependency $dependency)\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/Dependency.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Dependency\n{\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/Chosen.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nif (Checker::enabled()) {\n    class Chosen extends Base\n    {\n    }\n} else {\n    class Chosen\n    {\n    }\n}\n",
    );
    let metadata = compile(root);
    assert!(metadata.contains("'Acme\\\\On\\\\Model\\\\Base' =>"));
    assert!(
        !metadata.contains("'Acme\\\\On\\\\Model\\\\Chosen' =>"),
        "{metadata}"
    );
}

#[test]
fn class_inside_if_can_still_be_named_by_a_constructor() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/code/Acme/On/Model/Chosen.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nif (Checker::enabled()) {\n    class Chosen\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/User.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass User\n{\n    public function __construct(ChosenFactory $chosen, Chosen\\Proxy $proxy)\n    {\n    }\n}\n",
    );
    let metadata = compile(root);
    assert!(metadata.contains("'Acme\\\\On\\\\Model\\\\User' =>"));
    assert!(root
        .join("generated/code/Acme/On/Model/ChosenFactory.php")
        .exists());
    assert!(root
        .join("generated/code/Acme/On/Model/Chosen/Proxy.php")
        .exists());
}

#[test]
fn class_in_a_braced_namespace_is_scanned() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/code/Acme/On/Model/Braced.php",
        "<?php\nnamespace Acme\\On\\Model {\n    class Braced\n    {\n        public function __construct(Braced $self)\n        {\n        }\n    }\n}\n",
    );
    let metadata = compile(root);
    assert!(
        metadata.contains("'Acme\\\\On\\\\Model\\\\Braced' =>"),
        "{metadata}"
    );
}
