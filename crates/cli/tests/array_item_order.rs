//! `Data\Argument\Interpreter\ArrayType::evaluate()` sorts the items of an array when it reads the
//! merged di.xml of one scope. `Config::_collectConfiguration()` then merges parent and child with
//! `array_replace_recursive`, so new items of the child go last, and calls `SortItems::sortItems()`
//! after each merge. That call only sorts when the last item of the last array argument has a
//! `sortOrder`. Only object items keep that value, because `Interpreter\DataObject` copies it.

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

fn item_order(metadata: &str, owner: &str, items: &[&str]) -> Vec<usize> {
    let start = metadata
        .find(&format!("'{owner}' =>"))
        .unwrap_or_else(|| panic!("no entry for {owner}: {metadata}"));
    items
        .iter()
        .map(|item| {
            start
                + metadata[start..]
                    .find(&format!("'{item}' =>"))
                    .unwrap_or_else(|| panic!("no item {item}: {metadata}"))
        })
        .collect()
}

fn assert_in_order(positions: Vec<usize>) {
    let mut sorted = positions.clone();
    sorted.sort_unstable();
    assert_eq!(positions, sorted);
}

const TAKES_ITEMS: &str =
    "class Base\n{\n    public function __construct(array $things = [])\n    {\n    }\n}";

#[test]
fn item_of_a_child_without_sort_order_comes_after_the_sorted_items_of_the_parent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    class(root, "Base", TAKES_ITEMS);
    class(root, "Child", "class Child extends Base\n{\n}");
    compile(
        root,
        "<type name=\"Acme\\On\\Model\\Base\"><arguments><argument name=\"things\" xsi:type=\"array\">\n<item name=\"late\" sortOrder=\"20\" xsi:type=\"string\">l</item>\n<item name=\"early\" sortOrder=\"10\" xsi:type=\"string\">e</item>\n</argument></arguments></type>\n<type name=\"Acme\\On\\Model\\Child\"><arguments><argument name=\"things\" xsi:type=\"array\">\n<item name=\"own\" xsi:type=\"string\">o</item>\n</argument></arguments></type>",
    );
    assert_in_order(item_order(
        &metadata(root),
        "Acme\\\\On\\\\Model\\\\Child",
        &["early", "late", "own"],
    ));
}

#[test]
fn new_item_of_a_virtual_type_comes_last_whatever_its_sort_order() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    class(root, "Base", TAKES_ITEMS);
    compile(
        root,
        "<type name=\"Acme\\On\\Model\\Base\"><arguments><argument name=\"things\" xsi:type=\"array\">\n<item name=\"last\" sortOrder=\"100\" xsi:type=\"string\">l</item>\n<item name=\"first\" sortOrder=\"1\" xsi:type=\"string\">f</item>\n</argument></arguments></type>\n<virtualType name=\"Acme\\On\\Model\\Virtual\" type=\"Acme\\On\\Model\\Base\"><arguments><argument name=\"things\" xsi:type=\"array\">\n<item name=\"first\" sortOrder=\"1\" xsi:type=\"string\">f</item>\n<item name=\"middle\" sortOrder=\"4\" xsi:type=\"string\">m</item>\n</argument></arguments></virtualType>",
    );
    assert_in_order(item_order(
        &metadata(root),
        "Acme\\\\On\\\\Model\\\\Virtual",
        &["first", "last", "middle"],
    ));
}

#[test]
fn items_of_one_type_are_sorted() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    class(root, "Base", TAKES_ITEMS);
    compile(
        root,
        "<type name=\"Acme\\On\\Model\\Base\"><arguments><argument name=\"things\" xsi:type=\"array\">\n<item name=\"none\" xsi:type=\"string\">n</item>\n<item name=\"late\" sortOrder=\"20\" xsi:type=\"string\">l</item>\n<item name=\"early\" sortOrder=\"10\" xsi:type=\"string\">e</item>\n</argument></arguments></type>",
    );
    assert_in_order(item_order(
        &metadata(root),
        "Acme\\\\On\\\\Model\\\\Base",
        &["none", "early", "late"],
    ));
}

#[test]
fn later_file_without_sort_order_keeps_the_sort_order_of_an_earlier_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    class(root, "Base", TAKES_ITEMS);
    write(root, "app/code/Acme/Zed/registration.php", "<?php\n");
    write(
        root,
        "app/code/Acme/Zed/etc/module.xml",
        "<config><module name=\"Acme_Zed\"/></config>\n",
    );
    write(
        root,
        "app/code/Acme/Zed/etc/di.xml",
        "<config><type name=\"Acme\\On\\Model\\Base\"><arguments><argument name=\"things\" xsi:type=\"array\">\n<item name=\"moved\" xsi:type=\"string\">m2</item>\n</argument></arguments></type></config>\n",
    );
    compile(
        root,
        "<type name=\"Acme\\On\\Model\\Base\"><arguments><argument name=\"things\" xsi:type=\"array\">\n<item name=\"moved\" sortOrder=\"20\" xsi:type=\"string\">m</item>\n<item name=\"stays\" sortOrder=\"10\" xsi:type=\"string\">s</item>\n</argument></arguments></type>",
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
    assert!(output.status.success());
    assert_in_order(item_order(
        &metadata(root),
        "Acme\\\\On\\\\Model\\\\Base",
        &["stays", "moved"],
    ));
}

const TAKES_TWO: &str = "class Base\n{\n    public function __construct(array $things = [], array $helpers = [])\n    {\n    }\n}";

const PARENT_WITH_TWO: &str = "<type name=\"Acme\\On\\Model\\Base\"><arguments><argument name=\"things\" xsi:type=\"array\">\n<item name=\"late\" sortOrder=\"900\" xsi:type=\"object\">Acme\\On\\Model\\Base</item>\n</argument>\n<argument name=\"helpers\" xsi:type=\"array\">\n<item name=\"helper\" sortOrder=\"10\" xsi:type=\"object\">Acme\\On\\Model\\Base</item>\n</argument></arguments></type>";

#[test]
fn child_items_with_a_sort_order_sort_with_the_parent_when_the_last_item_has_one() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    class(root, "Base", TAKES_TWO);
    class(root, "Child", "class Child extends Base\n{\n}");
    compile(
        root,
        &format!("{PARENT_WITH_TWO}\n<type name=\"Acme\\On\\Model\\Child\"><arguments><argument name=\"things\" xsi:type=\"array\">\n<item name=\"early\" sortOrder=\"200\" xsi:type=\"object\">Acme\\On\\Model\\Base</item>\n</argument></arguments></type>"),
    );
    assert_in_order(item_order(
        &metadata(root),
        "Acme\\\\On\\\\Model\\\\Child",
        &["early", "late"],
    ));
}

#[test]
fn child_item_without_a_sort_order_stays_last_when_it_is_the_last_item() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    class(root, "Base", TAKES_TWO);
    class(root, "Child", "class Child extends Base\n{\n}");
    compile(
        root,
        &format!("{PARENT_WITH_TWO}\n<type name=\"Acme\\On\\Model\\Child\"><arguments><argument name=\"things\" xsi:type=\"array\">\n<item name=\"own\" xsi:type=\"object\">Acme\\On\\Model\\Base</item>\n</argument></arguments></type>"),
    );
    assert_in_order(item_order(
        &metadata(root),
        "Acme\\\\On\\\\Model\\\\Child",
        &["late", "own"],
    ));
}

#[test]
fn later_file_with_another_item_type_drops_the_sort_order_of_an_earlier_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    class(root, "Base", TAKES_ITEMS);
    write(root, "app/code/Acme/Zed/registration.php", "<?php\n");
    write(
        root,
        "app/code/Acme/Zed/etc/module.xml",
        "<config><module name=\"Acme_Zed\"/></config>\n",
    );
    write(
        root,
        "app/code/Acme/Zed/etc/di.xml",
        "<config><type name=\"Acme\\On\\Model\\Base\"><arguments><argument name=\"things\" xsi:type=\"array\">\n<item name=\"replaced\" xsi:type=\"null\"/>\n</argument></arguments></type></config>\n",
    );
    compile(
        root,
        "<type name=\"Acme\\On\\Model\\Base\"><arguments><argument name=\"things\" xsi:type=\"array\">\n<item name=\"replaced\" sortOrder=\"20\" xsi:type=\"string\">r</item>\n<item name=\"none\" xsi:type=\"string\">n</item>\n</argument></arguments></type>",
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
    assert!(output.status.success());
    assert_in_order(item_order(
        &metadata(root),
        "Acme\\\\On\\\\Model\\\\Base",
        &["replaced", "none"],
    ));
}
