//! Magento knows a module because its `registration.php` ran. For a Composer package that
//! happens through `vendor/composer/autoload_files.php`, whatever the folder is called.
//! The search for `registration.php` skips folders such as `Test` and `dev`, so a module
//! that Composer loads from such a folder has to be picked up from that list. A module
//! nobody loads, such as a test fixture, stays out.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn write_module(root: &Path, dir: &str, module: &str, class: &str) {
    write(root, &format!("{dir}/registration.php"), "<?php\n");
    write(
        root,
        &format!("{dir}/etc/module.xml"),
        &format!("<config><module name=\"{module}\"/></config>\n"),
    );
    write(
        root,
        &format!("{dir}/etc/di.xml"),
        &format!(
            r#"<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <type name="Acme\Tools\{class}">
        <plugin name="watch" type="Acme\Tools\Watch"/>
    </type>
</config>
"#
        ),
    );
    write(
        root,
        &format!("{dir}/{class}.php"),
        &format!(
            "<?php\nnamespace Acme\\Tools;\n\nclass {class}\n{{\n    public function __construct(Watch $watch)\n    {{\n    }}\n    public function run()\n    {{\n    }}\n}}\n"
        ),
    );
}

fn write_fixture(root: &Path) {
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_Listed' => 1,\n        'Acme_Fixture' => 1,\n    ],\n];\n",
    );
    write_module(
        root,
        "vendor/acme/tools/dev/listed",
        "Acme_Listed",
        "Listed",
    );
    write_module(
        root,
        "vendor/acme/tools/Test/_files/fixture",
        "Acme_Fixture",
        "Fixture",
    );
    write(
        root,
        "vendor/acme/tools/dev/listed/Watch.php",
        "<?php\nnamespace Acme\\Tools;\n\nclass Watch\n{\n    public function beforeRun($subject)\n    {\n    }\n}\n",
    );
    write(
        root,
        "vendor/composer/autoload_files.php",
        "<?php\n\n$vendorDir = dirname(__DIR__);\n$baseDir = dirname($vendorDir);\n\nreturn array(\n    \
         'abc' => $vendorDir . '/acme/tools/dev/listed/registration.php',\n    \
         'def' => $vendorDir . '/acme/tools/functions.php',\n    \
         'ghi' => $baseDir . '/app/etc/NonComposerComponentRegistration.php',\n);\n",
    );
}

#[test]
fn module_composer_loads_from_a_folder_named_dev_is_compiled() {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: this test needs php on the PATH");
        return;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    write_fixture(tmp.path());
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "compiler failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let code = tmp.path().join("generated/code/Acme/Tools");
    assert!(code.join("Listed/Interceptor.php").exists());
    assert!(!code.join("Fixture/Interceptor.php").exists());
}
