//! `PluginListGenerator::inheritPlugins()` throws "Plugin class X doesn't exist" for a plugin
//! that is not disabled and whose class fails `class_exists()`. That stops the compile. A disabled
//! plugin of a scanned concrete class stops it later, in `Definition\Runtime::getMethodList()`.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn di_xml(body: &str) -> String {
    format!("<config xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n{body}\n</config>\n")
}

fn plugin_on_gadget(attributes: &str) -> String {
    di_xml(&format!(
        "<type name=\"Acme\\On\\Model\\Gadget\"><plugin name=\"p\" {attributes}/></type>"
    ))
}

fn write_store(root: &Path) {
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n        'Acme_Off' => 0,\n    ],\n];\n",
    );
    write(root, "app/code/Acme/On/registration.php", "<?php\n");
    write(
        root,
        "app/code/Acme/On/etc/module.xml",
        "<config><module name=\"Acme_On\"/></config>\n",
    );
    write(root, "app/code/Acme/Off/registration.php", "<?php\n");
    write(
        root,
        "app/code/Acme/Off/etc/module.xml",
        "<config><module name=\"Acme_Off\"/></config>\n",
    );
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
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Gadget\n{\n    public function run()\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Plugin/Watch.php",
        "<?php\nnamespace Acme\\On\\Plugin;\n\nclass Watch\n{\n    public function beforeRun($subject)\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Plugin/Contract.php",
        "<?php\nnamespace Acme\\On\\Plugin;\n\ninterface Contract\n{\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Test/Stub/Watch.php",
        "<?php\nnamespace Acme\\On\\Test\\Stub;\n\nclass Watch\n{\n    public function beforeRun($subject)\n    {\n    }\n}\n",
    );
}

fn compile_with(files: &[(&str, String)]) -> Option<Output> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return None;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    write_store(tmp.path());
    for (relative, content) in files {
        write(tmp.path(), relative, content);
    }
    Some(
        Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
            .arg("--magento-root")
            .arg(tmp.path())
            .output()
            .unwrap(),
    )
}

fn assert_stops_on(output: &Output, class: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains(&format!("Plugin class {class} doesn't exist")),
        "{stderr}"
    );
}

fn assert_compiles(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

const MODULE_DI: &str = "app/code/Acme/On/etc/di.xml";

#[test]
fn missing_plugin_class_stops_the_compile() {
    let Some(output) = compile_with(&[(
        MODULE_DI,
        plugin_on_gadget("type=\"Acme\\On\\Plugin\\Gone\""),
    )]) else {
        return;
    };
    assert_stops_on(&output, "Acme\\On\\Plugin\\Gone");
}

#[test]
fn missing_plugin_class_in_an_area_file_stops_the_compile() {
    let Some(output) = compile_with(&[(
        "app/code/Acme/On/etc/frontend/di.xml",
        plugin_on_gadget("type=\"Acme\\On\\Plugin\\Gone\""),
    )]) else {
        return;
    };
    assert_stops_on(&output, "Acme\\On\\Plugin\\Gone");
}

#[test]
fn interface_as_plugin_stops_the_compile_because_class_exists_says_no() {
    let Some(output) = compile_with(&[(
        MODULE_DI,
        plugin_on_gadget("type=\"Acme\\On\\Plugin\\Contract\""),
    )]) else {
        return;
    };
    assert_stops_on(&output, "Acme\\On\\Plugin\\Contract");
}

#[test]
fn virtual_type_as_plugin_is_checked_by_its_real_class() {
    let Some(output) = compile_with(&[(
        MODULE_DI,
        di_xml(
            "<virtualType name=\"Acme\\On\\Plugin\\Virtual\" type=\"Acme\\On\\Plugin\\Gone\"/>\n\
             <type name=\"Acme\\On\\Model\\Gadget\"><plugin name=\"p\" type=\"Acme\\On\\Plugin\\Virtual\"/></type>",
        ),
    )]) else {
        return;
    };
    assert_stops_on(&output, "Acme\\On\\Plugin\\Gone");
}

#[test]
fn disabled_plugin_with_a_missing_class_on_a_scanned_class_stops_the_compile() {
    // `getInterceptedMethods()` calls `get_class_methods()` on every plugin in the inherited
    // list, and disabled plugins stay in that list.
    let Some(output) = compile_with(&[(
        MODULE_DI,
        plugin_on_gadget("type=\"Acme\\On\\Plugin\\Gone\" disabled=\"true\""),
    )]) else {
        return;
    };
    assert_stops_on(&output, "Acme\\On\\Plugin\\Gone");
}

#[test]
fn disabled_plugin_with_a_missing_class_on_an_interface_only_compiles() {
    let Some(output) = compile_with(&[(
        MODULE_DI,
        di_xml(
            "<type name=\"Acme\\On\\Plugin\\Contract\"><plugin name=\"p\" type=\"Acme\\On\\Plugin\\Gone\" disabled=\"true\"/></type>",
        ),
    )]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn disabled_plugin_with_a_missing_class_on_a_class_outside_the_scan_compiles() {
    let Some(output) = compile_with(&[(
        MODULE_DI,
        di_xml(
            "<type name=\"Acme\\Elsewhere\\Thing\"><plugin name=\"p\" type=\"Acme\\On\\Plugin\\Gone\" disabled=\"true\"/></type>",
        ),
    )]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn disabled_plugin_whose_class_is_an_interface_compiles_because_get_class_methods_accepts_it() {
    let Some(output) = compile_with(&[(
        MODULE_DI,
        plugin_on_gadget("type=\"Acme\\On\\Plugin\\Contract\" disabled=\"true\""),
    )]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn disabling_a_plugin_without_naming_a_class_compiles() {
    let Some(output) = compile_with(&[(MODULE_DI, plugin_on_gadget("disabled=\"true\""))]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn plugin_disabled_in_one_area_still_stops_on_the_scopes_where_it_runs() {
    let Some(output) = compile_with(&[
        (
            MODULE_DI,
            plugin_on_gadget("type=\"Acme\\On\\Plugin\\Gone\""),
        ),
        (
            "app/code/Acme/On/etc/frontend/di.xml",
            plugin_on_gadget("disabled=\"true\""),
        ),
    ]) else {
        return;
    };
    assert_stops_on(&output, "Acme\\On\\Plugin\\Gone");
}

#[test]
fn plugin_disabled_in_the_global_file_by_another_module_still_stops_the_compile() {
    let Some(output) = compile_with(&[
        (
            MODULE_DI,
            plugin_on_gadget("type=\"Acme\\On\\Plugin\\Gone\" disabled=\"true\""),
        ),
        (
            "app/code/Acme/On/etc/frontend/di.xml",
            plugin_on_gadget("sortOrder=\"5\""),
        ),
    ]) else {
        return;
    };
    assert_stops_on(&output, "Acme\\On\\Plugin\\Gone");
}

#[test]
fn disabled_module_does_not_stop_the_compile() {
    let Some(output) = compile_with(&[(
        "app/code/Acme/Off/etc/di.xml",
        plugin_on_gadget("type=\"Acme\\Off\\Plugin\\Gone\""),
    )]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn plugin_without_a_type_does_not_stop_the_compile() {
    let Some(output) = compile_with(&[(MODULE_DI, plugin_on_gadget("sortOrder=\"10\""))]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn plugin_class_the_scan_skips_but_the_autoloader_finds_does_not_stop_the_compile() {
    let Some(output) = compile_with(&[(
        MODULE_DI,
        plugin_on_gadget("type=\"Acme\\On\\Test\\Stub\\Watch\""),
    )]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn generated_plugin_class_whose_source_exists_does_not_stop_the_compile() {
    let Some(output) = compile_with(&[(
        MODULE_DI,
        plugin_on_gadget("type=\"Acme\\On\\Plugin\\Watch\\Proxy\""),
    )]) else {
        return;
    };
    assert_compiles(&output);
}
