//! `Compiler\Config\Reader::generateCachePerScope()` checks the target of every preference, per
//! area. A target that fails `class_exists()` stops the compile, and so does a target that the
//! scan does not list. A virtual type and a class from a PHP extension pass.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn compile_with(files: &[(&str, &str)]) -> Option<Output> {
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
    for (class, keyword) in [
        ("On/Model/Gadget", "class"),
        ("On/Model/Other", "class"),
        ("On/Model/Contract", "interface"),
        ("On/Test/Stub", "class"),
        ("Off/Model/Hidden", "class"),
    ] {
        let (folder, name) = class.rsplit_once('/').unwrap();
        write(
            root,
            &format!("app/code/Acme/{class}.php"),
            &format!(
                "<?php\nnamespace Acme\\{};\n\n{keyword} {name}\n{{\n}}\n",
                folder.replace('/', "\\")
            ),
        );
    }
    write(
        root,
        "app/code/Magento/Framework/ObjectManagerInterface.php",
        "<?php\nnamespace Magento\\Framework;\n\ninterface ObjectManagerInterface\n{\n}\n",
    );
    for (relative, content) in files {
        write(root, relative, content);
    }
    Some(
        Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
            .arg("--magento-root")
            .arg(root)
            .output()
            .unwrap(),
    )
}

fn di_xml(body: &str) -> String {
    format!("<config xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n{body}\n</config>\n")
}

fn preference(to: &str) -> String {
    di_xml(&format!(
        "<preference for=\"Acme\\On\\Model\\Gadget\" type=\"{to}\"/>"
    ))
}

const MODULE_DI: &str = "app/code/Acme/On/etc/di.xml";

fn assert_stops(output: &Output, message: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains(message), "{stderr}");
}

fn assert_compiles(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

const MISSING: &str =
    "Preference declared for \"Acme\\On\\Model\\Gadget\" as \"Acme\\On\\Model\\Gone\", but the latter does not exist.";

#[test]
fn missing_target_stops_the_compile() {
    let xml = preference("Acme\\On\\Model\\Gone");
    let Some(output) = compile_with(&[(MODULE_DI, &xml)]) else {
        return;
    };
    assert_stops(&output, MISSING);
}

#[test]
fn missing_target_in_an_area_file_stops_the_compile() {
    let xml = preference("Acme\\On\\Model\\Gone");
    let Some(output) = compile_with(&[("app/code/Acme/On/etc/frontend/di.xml", &xml)]) else {
        return;
    };
    assert_stops(&output, MISSING);
}

#[test]
fn interface_as_target_stops_the_compile_because_class_exists_says_no() {
    let xml = preference("Acme\\On\\Model\\Contract");
    let Some(output) = compile_with(&[(MODULE_DI, &xml)]) else {
        return;
    };
    assert_stops(&output, "but the latter does not exist.");
}

#[test]
fn target_under_test_stops_the_compile() {
    let xml = preference("Acme\\On\\Test\\Stub");
    let Some(output) = compile_with(&[(MODULE_DI, &xml)]) else {
        return;
    };
    assert_stops(
        &output,
        "as \"Acme\\On\\Test\\Stub\", but the latter has not been included in dependency injection compilation.",
    );
}

#[test]
fn target_in_a_disabled_module_stops_the_compile() {
    let xml = preference("Acme\\Off\\Model\\Hidden");
    let Some(output) = compile_with(&[(MODULE_DI, &xml)]) else {
        return;
    };
    assert_stops(&output, "has not been included in dependency injection");
}

#[test]
fn circular_preference_stops_the_compile() {
    let xml = di_xml(
        "<preference for=\"Acme\\On\\Model\\Gadget\" type=\"Acme\\On\\Model\\Other\"/>\n\
         <preference for=\"Acme\\On\\Model\\Other\" type=\"Acme\\On\\Model\\Gadget\"/>",
    );
    let Some(output) = compile_with(&[(MODULE_DI, &xml)]) else {
        return;
    };
    assert_stops(&output, "Circular type preference");
}

#[test]
fn the_end_of_a_chain_is_what_gets_checked() {
    let xml = di_xml(
        "<preference for=\"Acme\\On\\Model\\Contract\" type=\"Acme\\On\\Model\\Gadget\"/>\n\
         <preference for=\"Acme\\On\\Model\\Gadget\" type=\"Acme\\On\\Model\\Other\"/>",
    );
    let Some(output) = compile_with(&[(MODULE_DI, &xml)]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn virtual_type_as_target_compiles() {
    let xml = di_xml(
        "<virtualType name=\"Acme\\On\\Model\\Virtual\" type=\"Acme\\On\\Model\\Other\"/>\n\
         <preference for=\"Acme\\On\\Model\\Gadget\" type=\"Acme\\On\\Model\\Virtual\"/>",
    );
    let Some(output) = compile_with(&[(MODULE_DI, &xml)]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn class_of_a_php_extension_as_target_compiles() {
    let xml = preference("ArrayObject");
    let Some(output) = compile_with(&[(MODULE_DI, &xml)]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn generated_class_as_target_compiles() {
    let xml = preference("Acme\\On\\Model\\OtherFactory");
    let Some(output) = compile_with(&[(MODULE_DI, &xml)]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn preference_in_a_disabled_module_does_not_stop_the_compile() {
    let xml = preference("Acme\\On\\Model\\Gone");
    let Some(output) = compile_with(&[("app/code/Acme/Off/etc/di.xml", &xml)]) else {
        return;
    };
    assert_compiles(&output);
}
