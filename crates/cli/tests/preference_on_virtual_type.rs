//! `InterceptorSubstitution::modify()` resolves every preference to its Interceptor, whatever else
//! uses that name. The object manager follows one preference only, so a preference that stops at
//! the plain class skips the plugins.

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

#[test]
fn preference_on_a_name_that_is_also_a_virtual_type_gets_the_interceptor() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    class(
        root,
        "Impl",
        "class Impl implements Api\n{\n    public function run()\n    {\n    }\n}",
    );
    class(
        root,
        "Api",
        "interface Api\n{\n    public function run();\n}",
    );
    class(root, "Other", "class Other\n{\n}");
    class(
        root,
        "Watch",
        "class Watch\n{\n    public function beforeRun($subject)\n    {\n    }\n}",
    );
    compile(
        root,
        "<preference for=\"Acme\\On\\Model\\Api\" type=\"Acme\\On\\Model\\Impl\"/>\n<virtualType name=\"Acme\\On\\Model\\Api\" type=\"Acme\\On\\Model\\Other\"/>\n<type name=\"Acme\\On\\Model\\Impl\"><plugin name=\"watch\" type=\"Acme\\On\\Model\\Watch\"/></type>",
    );
    assert!(
        metadata(root).contains(
            "'Acme\\\\On\\\\Model\\\\Api' => 'Acme\\\\On\\\\Model\\\\Impl\\\\Interceptor'"
        ),
        "{}",
        metadata(root)
    );
}
