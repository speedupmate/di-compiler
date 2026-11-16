//! `PluginList::_loadScopedData()` runs `_inheritPlugins()` on every virtual type, which goes on to
//! the class of that type. `InterceptionConfigurationBuilder` then gives that class an Interceptor,
//! also when the scan does not list it: a generated proxy, or a class of a disabled module.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn compile(root: &Path, di_xml: &str) -> String {
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n        'Acme_Off' => 0,\n    ],\n];\n",
    );
    write(
        root,
        "vendor/composer/installed.json",
        "{\"packages\": []}\n",
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
        "app/code/Acme/On/etc/di.xml",
        &format!("<config>\n{di_xml}\n</config>\n"),
    );
    write(
        root,
        "app/code/Acme/On/Plugin/Watch.php",
        "<?php\nnamespace Acme\\On\\Plugin;\n\nclass Watch\n{\n    public function beforeRun($subject)\n    {\n    }\n}\n",
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

const WATCH: &str = "<plugin name=\"watch\" type=\"Acme\\On\\Plugin\\Watch\"/>";

#[test]
fn generated_proxy_as_target_of_a_virtual_type_gets_an_interceptor() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/code/Acme/On/Model/Helper.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Helper\n{\n    public function run()\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/User.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass User\n{\n    public function __construct(Helper\\Proxy $helper)\n    {\n    }\n}\n",
    );
    let metadata = compile(
        root,
        &format!("<virtualType name=\"LazyHelper\" type=\"Acme\\On\\Model\\Helper\\Proxy\"/>\n<type name=\"Acme\\On\\Model\\Helper\">{WATCH}</type>"),
    );
    let interceptor =
        fs::read_to_string(root.join("generated/code/Acme/On/Model/Helper/Proxy/Interceptor.php"))
            .unwrap();
    assert!(
        interceptor.contains("public function run()"),
        "{interceptor}"
    );
    assert!(
        interceptor.contains("$instanceName = '\\\\Acme\\\\On\\\\Model\\\\Helper', $shared = true"),
        "{interceptor}"
    );
    assert!(
        metadata
            .contains("'LazyHelper' => 'Acme\\\\On\\\\Model\\\\Helper\\\\Proxy\\\\Interceptor'"),
        "{metadata}"
    );
}

#[test]
fn proxy_without_a_virtual_type_gets_no_interceptor() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/code/Acme/On/Model/Helper.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Helper\n{\n    public function run()\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/User.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass User\n{\n    public function __construct(Helper\\Proxy $helper)\n    {\n    }\n}\n",
    );
    compile(
        root,
        &format!("<type name=\"Acme\\On\\Model\\Helper\">{WATCH}</type>"),
    );
    assert!(!root
        .join("generated/code/Acme/On/Model/Helper/Proxy/Interceptor.php")
        .exists());
}

#[test]
fn class_of_a_disabled_module_as_target_of_a_virtual_type_gets_an_interceptor() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/code/Acme/Off/Model/Sorter.php",
        "<?php\nnamespace Acme\\Off\\Model;\n\nclass Sorter\n{\n    public function run()\n    {\n    }\n}\n",
    );
    let metadata = compile(
        root,
        &format!("<virtualType name=\"PriceSorter\" type=\"Acme\\Off\\Model\\Sorter\"/>\n<type name=\"Acme\\Off\\Model\\Sorter\">{WATCH}</type>"),
    );
    assert!(root
        .join("generated/code/Acme/Off/Model/Sorter/Interceptor.php")
        .exists());
    assert!(
        metadata.contains("'PriceSorter' => 'Acme\\\\Off\\\\Model\\\\Sorter\\\\Interceptor'"),
        "{metadata}"
    );
}

#[test]
fn plugin_on_an_interface_of_a_disabled_module_reaches_the_target_of_a_virtual_type() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/code/Acme/Off/Model/SorterInterface.php",
        "<?php\nnamespace Acme\\Off\\Model;\n\ninterface SorterInterface\n{\n    public function run();\n}\n",
    );
    write(
        root,
        "app/code/Acme/Off/Model/Sorter.php",
        "<?php\nnamespace Acme\\Off\\Model;\n\nclass Sorter implements SorterInterface\n{\n    public function run()\n    {\n    }\n}\n",
    );
    compile(
        root,
        &format!("<virtualType name=\"PriceSorter\" type=\"Acme\\Off\\Model\\Sorter\"/>\n<type name=\"Acme\\Off\\Model\\SorterInterface\">{WATCH}</type>"),
    );
    let interceptor =
        fs::read_to_string(root.join("generated/code/Acme/Off/Model/Sorter/Interceptor.php"))
            .unwrap();
    assert!(
        interceptor.contains("public function run()"),
        "{interceptor}"
    );
}
