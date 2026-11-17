//! `Config\Dom` merges di.xml files node by node and keeps the attributes of an earlier file, and
//! `ArgumentParser` reads `shared` on an argument and on an item. `shared="false"` gives `_ins_`,
//! a new object for each request. With `_i_` all users share one object, such as one collection.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// A store with one enabled module. Compiles without PHP, so a test only depends on the static scan.
fn compile(root: &Path, di_xml: &str) {
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
        &format!("<config>\n{di_xml}\n</config>\n"),
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
}

fn class(root: &Path, name: &str, body: &str) {
    write(
        root,
        &format!("app/code/Acme/On/Model/{name}.php"),
        &format!("<?php\nnamespace Acme\\On\\Model;\n\n{body}\n"),
    );
}

fn metadata(root: &Path) -> String {
    fs::read_to_string(root.join("generated/metadata/global.php")).unwrap()
}

const TAKES: &str = "class User\n{\n    public function __construct(array $list = [], ?Thing $one = null)\n    {\n    }\n}";

#[test]
fn item_with_shared_false_is_not_shared() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    class(root, "User", TAKES);
    class(root, "Thing", "class Thing\n{\n}");
    compile(
        root,
        "<type name=\"Acme\\On\\Model\\User\"><arguments><argument name=\"list\" xsi:type=\"array\">\n<item name=\"a\" xsi:type=\"object\" shared=\"false\">Acme\\On\\Model\\Thing</item>\n<item name=\"b\" xsi:type=\"object\">Acme\\On\\Model\\Thing</item>\n</argument></arguments></type>",
    );
    let metadata = metadata(root);
    let a = metadata.find("'a' =>").unwrap();
    let b = metadata.find("'b' =>").unwrap();
    assert!(metadata[a..b].contains("'_ins_'"), "{metadata}");
    assert!(
        metadata[b..].contains("'_i_' => 'Acme\\\\On\\\\Model\\\\Thing'"),
        "{metadata}"
    );
}

#[test]
fn later_file_without_shared_keeps_shared_false_of_an_earlier_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    class(root, "User", TAKES);
    class(root, "Thing", "class Thing\n{\n}");
    class(root, "Other", "class Other extends Thing\n{\n}");
    write(root, "app/code/Acme/Zed/registration.php", "<?php\n");
    write(
        root,
        "app/code/Acme/Zed/etc/module.xml",
        "<config><module name=\"Acme_Zed\"/></config>\n",
    );
    write(
        root,
        "app/code/Acme/Zed/etc/di.xml",
        "<config><type name=\"Acme\\On\\Model\\User\"><arguments><argument name=\"one\" xsi:type=\"object\">Acme\\On\\Model\\Other</argument></arguments></type></config>\n",
    );
    write(root, "app/etc/config.php", "");
    compile_two_modules(root);
    let metadata = metadata(root);
    assert!(
        metadata.contains("'_ins_' => 'Acme\\\\On\\\\Model\\\\Other'"),
        "{metadata}"
    );
}

fn compile_two_modules(root: &Path) {
    compile(
        root,
        "<type name=\"Acme\\On\\Model\\User\"><arguments><argument name=\"one\" xsi:type=\"object\" shared=\"false\">Acme\\On\\Model\\Thing</argument></arguments></type>",
    );
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n        'Acme_Zed' => 1,\n    ],\n];\n",
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
}
