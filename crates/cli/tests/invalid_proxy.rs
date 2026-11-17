//! `Scanner\XmlScanner::_filterEntities()` logs "Invalid proxy class" for a proxy named in di.xml
//! whose source class does not exist, and `Log::report()` then stops the compile.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn compile_with(relative: &str, body: &str) -> Option<Output> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return None;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n        'Acme_Off' => 0,\n    ],\n];\n",
    );
    for module in ["On", "Off"] {
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
        "app/code/Acme/On/Model/Gadget.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Gadget\n{\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/Contract.php",
        "<?php\nnamespace Acme\\On\\Model;\n\ninterface Contract\n{\n}\n",
    );
    write(
        root,
        relative,
        &format!(
            "<config xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n{body}\n</config>\n"
        ),
    );
    Some(
        Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
            .arg("--magento-root")
            .arg(root)
            .output()
            .unwrap(),
    )
}

const ON: &str = "app/code/Acme/On/etc/di.xml";

fn argument(value: &str) -> String {
    format!(
        "<type name=\"Acme\\On\\Model\\Gadget\"><arguments><argument name=\"x\" xsi:type=\"object\">{value}</argument></arguments></type>"
    )
}

fn assert_stops(output: &Output, class: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("Invalid proxy class"), "{stderr}");
    assert!(stderr.contains(class), "{stderr}");
}

fn assert_compiles(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn argument_stops_the_compile() {
    let Some(output) = compile_with(ON, &argument("Acme\\On\\Model\\Gone\\Proxy")) else {
        return;
    };
    assert_stops(&output, "Acme\\On\\Model\\Gone");
}

#[test]
fn array_item_stops_the_compile() {
    let body = "<type name=\"Acme\\On\\Model\\Gadget\"><arguments><argument name=\"x\" xsi:type=\"array\">\
        <item name=\"a\" xsi:type=\"object\">Acme\\On\\Model\\Gone\\Proxy</item></argument></arguments></type>";
    let Some(output) = compile_with(ON, body) else {
        return;
    };
    assert_stops(&output, "Acme\\On\\Model\\Gone");
}

#[test]
fn preference_stops_the_compile() {
    let body =
        "<preference for=\"Acme\\On\\Model\\Contract\" type=\"Acme\\On\\Model\\Gone\\Proxy\"/>";
    let Some(output) = compile_with(ON, body) else {
        return;
    };
    assert_stops(&output, "Acme\\On\\Model\\Gone");
}

#[test]
fn virtual_type_stops_the_compile() {
    let body =
        "<virtualType name=\"Acme\\On\\Model\\Virtual\" type=\"Acme\\On\\Model\\Gone\\Proxy\"/>";
    let Some(output) = compile_with(ON, body) else {
        return;
    };
    assert_stops(&output, "Acme\\On\\Model\\Gone");
}

#[test]
fn area_file_stops_the_compile() {
    let Some(output) = compile_with(
        "app/code/Acme/On/etc/frontend/di.xml",
        &argument("Acme\\On\\Model\\Gone\\Proxy"),
    ) else {
        return;
    };
    assert_stops(&output, "Acme\\On\\Model\\Gone");
}

#[test]
fn name_that_only_ends_in_proxy_stops_the_compile_as_magentos_pattern_matches_it() {
    let Some(output) = compile_with(ON, &argument("Acme\\On\\Model\\GoneProxy")) else {
        return;
    };
    assert_stops(&output, "Acme\\On\\Model\\GoneProxy");
}

#[test]
fn proxy_of_an_existing_class_or_interface_compiles() {
    let body = format!(
        "{}\n<type name=\"Acme\\On\\Model\\Contract\"><arguments><argument name=\"x\" xsi:type=\"object\">Acme\\On\\Model\\Contract\\Proxy</argument></arguments></type>",
        argument("Acme\\On\\Model\\Gadget\\Proxy")
    );
    let Some(output) = compile_with(ON, &body) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn argument_of_type_string_compiles() {
    let body = "<type name=\"Acme\\On\\Model\\Gadget\"><arguments><argument name=\"x\" xsi:type=\"string\">Acme\\On\\Model\\Gone\\Proxy</argument></arguments></type>";
    let Some(output) = compile_with(ON, body) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn disabled_module_does_not_stop_the_compile() {
    let Some(output) = compile_with(
        "app/code/Acme/Off/etc/di.xml",
        &argument("Acme\\On\\Model\\Gone\\Proxy"),
    ) else {
        return;
    };
    assert_compiles(&output);
}
