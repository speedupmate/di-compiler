//! `ObjectManager\Config\Mapper\Dom::convert()` evaluates every argument after `Config\Dom` merged
//! the files of one scope. `Interpreter\Composite::evaluate()` stops the compile when the merged
//! node has no `xsi:type`, or one it does not know. The type can come from any file of the scope,
//! but not from another scope.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const MISSING: &str = "Value for key \"xsi:type\" is missing in the argument data";

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn compile_with(files: &[(&str, &str)]) -> Option<(tempfile::TempDir, Output)> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return None;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_First' => 1,\n        'Acme_Second' => 1,\n    ],\n];\n",
    );
    for module in ["First", "Second"] {
        write(
            root,
            &format!("app/code/Acme/{module}/registration.php"),
            "<?php\n",
        );
        write(
            root,
            &format!("app/code/Acme/{module}/etc/module.xml"),
            &format!("<config><module name=\"Acme_{module}\"/></config>\n"),
        );
    }
    write(
        root,
        "vendor/autoload.php",
        "<?php\nspl_autoload_register(function ($class) {\n    \
         $file = dirname(__DIR__) . '/app/code/' . str_replace('\\\\', '/', $class) . '.php';\n    \
         if (is_file($file)) {\n        require $file;\n    }\n});\n",
    );
    write(
        root,
        "app/code/Acme/First/Model/Thing.php",
        "<?php\nnamespace Acme\\First\\Model;\n\nclass Thing\n{\n    public function __construct($value = null)\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/First/Model/Gadget.php",
        "<?php\nnamespace Acme\\First\\Model;\n\nclass Gadget\n{\n}\n",
    );
    for (relative, content) in files {
        write(root, relative, content);
    }
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .output()
        .unwrap();
    Some((tmp, output))
}

fn thing(arguments: &str) -> String {
    format!(
        "<config xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><type name=\"Acme\\First\\Model\\Thing\"><arguments>{arguments}</arguments></type></config>\n"
    )
}

const FIRST: &str = "app/code/Acme/First/etc/di.xml";
const SECOND: &str = "app/code/Acme/Second/etc/di.xml";
const SECOND_AREA: &str = "app/code/Acme/Second/etc/adminhtml/di.xml";

fn assert_stops(files: &[(&str, &str)], message: &str, argument: &str) {
    let Some((_tmp, output)) = compile_with(files) else {
        return;
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains(message), "{stderr}");
    assert!(stderr.contains("Acme\\First\\Model\\Thing"), "{stderr}");
    assert!(stderr.contains(argument), "{stderr}");
}

fn compiled_thing(files: &[(&str, &str)]) -> Option<String> {
    let (tmp, output) = compile_with(files)?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata = fs::read_to_string(tmp.path().join("generated/metadata/global.php")).unwrap();
    let after = metadata
        .split("'Acme\\\\First\\\\Model\\\\Thing' =>")
        .nth(1)
        .expect("the class has arguments");
    Some(after.split("'preferences'").next().unwrap().to_string())
}

#[test]
fn text_without_a_type_stops_the_compile() {
    let first = thing("<argument name=\"value\">text</argument>");
    assert_stops(&[(FIRST, &first)], MISSING, "value");
}

#[test]
fn empty_node_without_a_type_stops_the_compile() {
    let first = thing("<argument name=\"value\"/>");
    assert_stops(&[(FIRST, &first)], MISSING, "value");
}

#[test]
fn items_without_a_type_stop_the_compile() {
    let first =
        thing("<argument name=\"value\"><item name=\"a\" xsi:type=\"string\">b</item></argument>");
    assert_stops(&[(FIRST, &first)], MISSING, "value");
}

#[test]
fn item_without_a_type_stops_the_compile() {
    let first =
        thing("<argument name=\"value\" xsi:type=\"array\"><item name=\"a\">b</item></argument>");
    assert_stops(&[(FIRST, &first)], MISSING, "value/a");
}

#[test]
fn type_attribute_without_the_xsi_prefix_is_no_type() {
    let first = thing("<argument name=\"value\" type=\"string\">text</argument>");
    assert_stops(&[(FIRST, &first)], MISSING, "value");
}

#[test]
fn unknown_type_stops_the_compile() {
    let first = thing("<argument name=\"value\" xsi:type=\"url\">x</argument>");
    assert_stops(
        &[(FIRST, &first)],
        "Argument interpreter named 'url' has not been defined",
        "value",
    );
}

#[test]
fn type_in_another_scope_does_not_count() {
    let first = thing("<argument name=\"value\" xsi:type=\"string\">a</argument>");
    let area = thing("<argument name=\"value\">b</argument>");
    assert_stops(&[(FIRST, &first), (SECOND_AREA, &area)], MISSING, "value");
}

#[test]
fn type_in_an_earlier_file_of_the_scope_stays() {
    let first =
        thing("<argument name=\"value\" xsi:type=\"object\">Acme\\First\\Model\\Thing</argument>");
    let second = thing("<argument name=\"value\">Acme\\First\\Model\\Gadget</argument>");
    let Some(thing) = compiled_thing(&[(FIRST, &first), (SECOND, &second)]) else {
        return;
    };
    assert!(
        thing.contains("'instance' => 'Acme\\\\First\\\\Model\\\\Gadget'"),
        "{thing}"
    );
}

#[test]
fn type_in_a_later_file_of_the_scope_counts() {
    let first = thing("<argument name=\"value\">a</argument>");
    let second = thing("<argument name=\"value\" xsi:type=\"string\">b</argument>");
    let Some(thing) = compiled_thing(&[(FIRST, &first), (SECOND, &second)]) else {
        return;
    };
    assert!(thing.contains("'_v_' => 'b'"), "{thing}");
}

#[test]
fn items_merge_with_a_type_in_an_earlier_or_later_file() {
    for (typed, untyped) in [(FIRST, SECOND), (SECOND, FIRST)] {
        let with_type = thing(
            "<argument name=\"value\" xsi:type=\"array\"><item name=\"typed\" xsi:type=\"string\">a</item></argument>",
        );
        let without = thing(
            "<argument name=\"value\"><item name=\"plain\" xsi:type=\"string\">b</item></argument>",
        );
        let Some(thing) = compiled_thing(&[(typed, &with_type), (untyped, &without)]) else {
            return;
        };
        assert!(thing.contains("'typed'"), "{thing}");
        assert!(thing.contains("'plain'"), "{thing}");
    }
}

#[test]
fn number_that_is_not_numeric_stops_the_compile() {
    let first = thing("<argument name=\"value\" xsi:type=\"number\">abc</argument>");
    assert_stops(&[(FIRST, &first)], "Numeric value is expected", "value");
}

#[test]
fn numeric_forms_of_php_compile() {
    let first = thing("<argument name=\"value\" xsi:type=\"array\"><item name=\"a\" xsi:type=\"number\">1.5e3</item><item name=\"b\" xsi:type=\"number\">-.5</item></argument>");
    assert!(compiled_thing(&[(FIRST, &first)]).is_none_or(|thing| thing.contains("1.5e3")));
}

#[test]
fn boolean_that_is_not_a_boolean_stops_the_compile() {
    let first = thing("<argument name=\"value\" xsi:type=\"boolean\">maybe</argument>");
    assert_stops(&[(FIRST, &first)], "Boolean value is expected", "value");
}

#[test]
fn invalid_value_replaced_in_a_later_file_compiles() {
    let first = thing("<argument name=\"value\" xsi:type=\"boolean\">maybe</argument>");
    let second = thing("<argument name=\"value\" xsi:type=\"boolean\">true</argument>");
    let Some(thing) = compiled_thing(&[(FIRST, &first), (SECOND, &second)]) else {
        return;
    };
    assert!(thing.contains("'_v_' => true"), "{thing}");
}

#[test]
fn object_without_a_class_stops_the_compile() {
    let first = thing("<argument name=\"value\" xsi:type=\"object\"></argument>");
    assert_stops(&[(FIRST, &first)], "has no class name", "value");
}

#[test]
fn later_empty_object_node_keeps_the_class_of_the_earlier_file() {
    let first =
        thing("<argument name=\"value\" xsi:type=\"object\">Acme\\First\\Model\\Gadget</argument>");
    let second = thing("<argument name=\"value\" xsi:type=\"object\" shared=\"false\"/>");
    let Some(thing) = compiled_thing(&[(FIRST, &first), (SECOND, &second)]) else {
        return;
    };
    assert!(
        thing.contains("Acme\\\\First\\\\Model\\\\Gadget"),
        "{thing}"
    );
}
