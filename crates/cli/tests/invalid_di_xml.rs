//! `Config\Reader\Filesystem::_readFiles()` reads each di.xml through `Config\Dom::_initDom()`.
//! When `DOMDocument::loadXML()` fails, the compile stops with "The XML in file ... is invalid".

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

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
    write(root, "vendor/autoload.php", "<?php\n");
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

fn assert_stops_on_file(output: &Output, relative: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("The XML in file"), "{stderr}");
    assert!(stderr.contains(relative), "{stderr}");
    assert!(stderr.contains("is invalid"), "{stderr}");
}

const BROKEN: &str = "<?xml version=\"1.0\"?>\n<config><type name=\"Acme\\On\\Thing\"></config>\n";

#[test]
fn mismatched_tag_in_a_module_di_xml_stops_the_compile() {
    let Some((_tmp, output)) = compile_with(&[("app/code/Acme/On/etc/di.xml", BROKEN)]) else {
        return;
    };
    assert_stops_on_file(&output, "app/code/Acme/On/etc/di.xml");
}

#[test]
fn broken_area_di_xml_stops_the_compile() {
    let Some((_tmp, output)) = compile_with(&[("app/code/Acme/On/etc/frontend/di.xml", BROKEN)])
    else {
        return;
    };
    assert_stops_on_file(&output, "app/code/Acme/On/etc/frontend/di.xml");
}

#[test]
fn unclosed_root_stops_the_compile() {
    let Some((_tmp, output)) = compile_with(&[(
        "app/code/Acme/On/etc/di.xml",
        "<?xml version=\"1.0\"?>\n<config>\n",
    )]) else {
        return;
    };
    assert_stops_on_file(&output, "app/code/Acme/On/etc/di.xml");
}

#[test]
fn bare_ampersand_stops_the_compile() {
    let Some((_tmp, output)) = compile_with(&[(
        "app/code/Acme/On/etc/di.xml",
        "<config><a>b & c</a></config>\n",
    )]) else {
        return;
    };
    assert_stops_on_file(&output, "app/code/Acme/On/etc/di.xml");
}

#[test]
fn empty_di_xml_stops_the_compile() {
    let Some((_tmp, output)) = compile_with(&[("app/code/Acme/On/etc/di.xml", "")]) else {
        return;
    };
    assert_stops_on_file(&output, "app/code/Acme/On/etc/di.xml");
}

#[test]
fn undeclared_namespace_prefix_does_not_stop_because_load_xml_accepts_it() {
    let Some((_tmp, output)) = compile_with(&[(
        "app/code/Acme/On/etc/di.xml",
        "<config><type name=\"Acme\\On\\Thing\"><arguments><argument name=\"a\" xsi:type=\"string\">b</argument></arguments></type></config>\n",
    )]) else {
        return;
    };
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn broken_di_xml_of_a_disabled_module_does_not_stop_the_compile() {
    let Some((_tmp, output)) = compile_with(&[("app/code/Acme/Off/etc/di.xml", BROKEN)]) else {
        return;
    };
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn invalid_extension_attributes_xml_stops_the_compile_and_names_the_file() {
    let Some((_tmp, output)) = compile_with(&[(
        "app/code/Acme/On/etc/extension_attributes.xml",
        "<?xml version=\"1.0\"?>\n<config><extension_attributes for=\"Acme\\Thing\"></config>\n",
    )]) else {
        return;
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("etc/extension_attributes.xml"), "{stderr}");
}

#[test]
fn invalid_extension_attributes_xml_of_a_disabled_module_compiles() {
    let Some((_tmp, output)) = compile_with(&[(
        "app/code/Acme/Off/etc/extension_attributes.xml",
        "<config><extension_attributes for=\"Acme\\Thing\"></config>\n",
    )]) else {
        return;
    };
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
