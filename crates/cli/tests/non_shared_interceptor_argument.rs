//! `ArgumentsResolver::getConfiguredInstanceArgument()` in setup:di:compile calls
//! `Config::isShared()` on the object argument exactly as di.xml names it, unless the
//! argument has its own `shared` attribute. Preferences, the Interceptor among them, are
//! applied after that by `InterceptorSubstitution` and `PreferencesResolving`. A non-shared
//! class with a plugin stays `_ins_` and points at its Interceptor, and an interface that
//! is shared stays `_i_` even when its preference points at a non-shared class.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn class(root: &Path, name: &str, body: &str) {
    write(
        root,
        &format!("app/code/Acme/On/Model/{name}.php"),
        &format!("<?php\nnamespace Acme\\On\\Model;\n\n{body}\n"),
    );
}

/// A store with one enabled module. Compiles without PHP, so the test only depends on the static scan.
fn compile(root: &Path, di_xml: &str) -> String {
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
    class(
        root,
        "Iterator",
        "class Iterator\n{\n    public function __construct(Element $element, ?Element $explicit = null, ?ElementInterface $byInterface = null, ?PlainInterface $plain = null)\n    {\n    }\n}",
    );
    class(
        root,
        "ElementInterface",
        "interface ElementInterface\n{\n    public function run();\n}",
    );
    class(
        root,
        "Element",
        "class Element implements ElementInterface\n{\n    public function run()\n    {\n    }\n}",
    );
    class(root, "PlainInterface", "interface PlainInterface\n{\n}");
    class(root, "Plain", "class Plain implements PlainInterface\n{\n}");
    class(
        root,
        "ElementPlugin",
        "class ElementPlugin\n{\n    public function afterRun($subject, $result)\n    {\n        return $result;\n    }\n}",
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
    fs::read_to_string(root.join("generated/metadata/global.php")).unwrap()
}

const ELEMENT: &str = "<preference for=\"Acme\\On\\Model\\ElementInterface\" type=\"Acme\\On\\Model\\Element\"/>\n<preference for=\"Acme\\On\\Model\\PlainInterface\" type=\"Acme\\On\\Model\\Plain\"/>\n<type name=\"Acme\\On\\Model\\Plain\" shared=\"false\"/>\n<type name=\"Acme\\On\\Model\\Element\" shared=\"false\"><plugin name=\"acme\" type=\"Acme\\On\\Model\\ElementPlugin\"/></type>";

const ARGUMENTS: &str = "<arguments>\n<argument name=\"element\" xsi:type=\"object\">Acme\\On\\Model\\Element</argument>\n<argument name=\"explicit\" xsi:type=\"object\" shared=\"true\">Acme\\On\\Model\\Element</argument>\n<argument name=\"byInterface\" xsi:type=\"object\">Acme\\On\\Model\\ElementInterface</argument>\n<argument name=\"plain\" xsi:type=\"object\">Acme\\On\\Model\\PlainInterface</argument>\n</arguments>";

/// The PHP array that `generated/metadata` holds for one argument of one type.
fn argument<'a>(metadata: &'a str, owner: &str, name: &str) -> &'a str {
    let owner = metadata
        .find(&format!("'{owner}' => "))
        .unwrap_or_else(|| panic!("{owner} missing in:\n{metadata}"));
    let rest = &metadata[owner..];
    let start = rest
        .find(&format!("'{name}' => "))
        .unwrap_or_else(|| panic!("{name} missing in:\n{rest}"));
    let rest = &rest[start..];
    &rest[..rest.find("),").unwrap()]
}

fn assert_arguments(metadata: &str, owner: &str) {
    let interceptor = "'Acme\\\\On\\\\Model\\\\Element\\\\Interceptor'";
    let element = argument(metadata, owner, "element");
    assert!(
        element.contains(&format!("'_ins_' => {interceptor}")),
        "{owner} element:\n{element}"
    );
    let explicit = argument(metadata, owner, "explicit");
    assert!(
        explicit.contains(&format!("'_i_' => {interceptor}")),
        "{owner} explicit:\n{explicit}"
    );
    let by_interface = argument(metadata, owner, "byInterface");
    assert!(
        by_interface.contains(&format!("'_i_' => {interceptor}")),
        "{owner} byInterface:\n{by_interface}"
    );
    let plain = argument(metadata, owner, "plain");
    assert!(
        plain.contains("'_i_' => 'Acme\\\\On\\\\Model\\\\Plain'"),
        "{owner} plain:\n{plain}"
    );
}

#[test]
fn non_shared_intercepted_argument_of_a_type_stays_non_shared() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    let metadata = compile(
        root,
        &format!("{ELEMENT}\n<type name=\"Acme\\On\\Model\\Iterator\">{ARGUMENTS}</type>"),
    );
    assert_arguments(&metadata, "Acme\\\\On\\\\Model\\\\Iterator");
}

#[test]
fn non_shared_intercepted_argument_of_a_virtual_type_stays_non_shared() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    let metadata = compile(
        root,
        &format!("{ELEMENT}\n<virtualType name=\"Acme\\On\\Model\\Virtual\\Iterator\" type=\"Acme\\On\\Model\\Iterator\" shared=\"false\">{ARGUMENTS}</virtualType>"),
    );
    assert_arguments(&metadata, "Acme\\\\On\\\\Model\\\\Virtual\\\\Iterator");
}
