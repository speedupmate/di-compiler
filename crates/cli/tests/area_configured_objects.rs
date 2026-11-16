//! Magento builds each area file from scratch: `Compiler\Config\Reader::generateCachePerScope()`
//! resolves every type against the merged area config, and `Chain\PreferencesResolving`
//! then replaces every `_i_` and `_ins_` value at any depth. An area preference or `shared`
//! setting must therefore reach objects configured in the global di.xml, also when the type
//! that holds them has no area config of its own.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn write_class(root: &Path, class: &str, source: &str) {
    let (namespace, _) = class.rsplit_once('\\').unwrap();
    let relative = class.split('\\').skip(2).collect::<Vec<_>>().join("/");
    write(
        root,
        &format!("app/code/Acme/On/{relative}.php"),
        &format!("<?php\nnamespace {namespace};\n\n{source}\n"),
    );
}

fn write_empty_class(root: &Path, class: &str) {
    let name = class.rsplit('\\').next().unwrap();
    write_class(root, class, &format!("class {name}\n{{\n}}"));
}

fn write_fixture(root: &Path) {
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n    ],\n];\n",
    );
    write(
        root,
        "app/code/Acme/On/registration.php",
        "<?php\n// The compiler reads the module name from etc/module.xml.\n",
    );
    write(
        root,
        "app/code/Acme/On/etc/module.xml",
        "<config><module name=\"Acme_On\"/></config>\n",
    );
    write(
        root,
        "app/code/Acme/On/etc/di.xml",
        r#"<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <preference for="Acme\On\Api\RuleInterface" type="Acme\On\Model\StoreRule"/>
    <virtualType name="GateAlias" type="Acme\On\Model\StoreGate"/>
    <virtualType name="GateChain" type="Acme\On\Model\Chain">
        <arguments>
            <argument name="gates" xsi:type="array">
                <item name="first" xsi:type="object">GateAlias</item>
                <item name="nested" xsi:type="array">
                    <item name="deep" xsi:type="object">Acme\On\Model\StoreDeep</item>
                </item>
            </argument>
        </arguments>
    </virtualType>
    <type name="Acme\On\Model\Keeper">
        <arguments>
            <argument name="tool" xsi:type="object">Acme\On\Model\Tool</argument>
        </arguments>
    </type>
</config>
"#,
    );
    write(
        root,
        "app/code/Acme/On/etc/adminhtml/di.xml",
        r#"<config>
    <preference for="GateAlias" type="Acme\On\Model\AdminGate"/>
    <preference for="Acme\On\Model\StoreDeep" type="Acme\On\Model\AdminDeep"/>
    <preference for="Acme\On\Model\StoreRule" type="Acme\On\Model\AdminRule"/>
    <type name="Acme\On\Model\Tool" shared="false"/>
</config>
"#,
    );

    write_class(
        root,
        "Acme\\On\\Api\\RuleInterface",
        "interface RuleInterface\n{\n}",
    );
    write_class(
        root,
        "Acme\\On\\Api\\ToolInterface",
        "interface ToolInterface\n{\n}",
    );
    write_class(
        root,
        "Acme\\On\\Model\\Chain",
        "class Chain\n{\n    public function __construct(array $gates = [])\n    {\n    }\n}",
    );
    write_class(
        root,
        "Acme\\On\\Model\\Keeper",
        "class Keeper\n{\n    public function __construct(\\Acme\\On\\Api\\ToolInterface $tool)\n    {\n    }\n}",
    );
    write_class(
        root,
        "Acme\\On\\Model\\RuleUser",
        "class RuleUser\n{\n    public function __construct(\\Acme\\On\\Api\\RuleInterface $rule)\n    {\n    }\n}",
    );
    for class in [
        "Acme\\On\\Model\\AdminDeep",
        "Acme\\On\\Model\\AdminGate",
        "Acme\\On\\Model\\AdminRule",
        "Acme\\On\\Model\\StoreDeep",
        "Acme\\On\\Model\\StoreGate",
        "Acme\\On\\Model\\StoreRule",
        "Acme\\On\\Model\\Tool",
    ] {
        write_empty_class(root, class);
    }
}

fn compiled_fixture() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    write_fixture(tmp.path());
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(tmp.path())
        // No PHP on purpose, so this test only depends on the static scan.
        .arg("--fallback-php")
        .arg(tmp.path().join("no-php-here"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "compiler failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    tmp
}

fn metadata(root: &Path, area: &str) -> String {
    fs::read_to_string(root.join(format!("generated/metadata/{area}.php"))).unwrap()
}

/// The argument entries of one type in an area file.
fn arguments_of(file: &str, type_name: &str) -> String {
    let key = format!("    '{}' => \n", type_name.replace('\\', "\\\\"));
    let start = file
        .find(&key)
        .unwrap_or_else(|| panic!("{type_name} has no arguments in:\n{file}"));
    let rest = &file[start..];
    let end = rest.find("\n    ),").expect("unterminated entry");
    rest[..end].to_string()
}

fn instance(kind: &str, class: &str) -> String {
    format!("'{kind}' => '{}'", class.replace('\\', "\\\\"))
}

#[test]
fn area_preference_replaces_object_in_a_global_array_argument() {
    let tmp = compiled_fixture();

    let admin = arguments_of(&metadata(tmp.path(), "adminhtml"), "GateChain");
    assert!(
        admin.contains(&instance("_i_", "Acme\\On\\Model\\AdminGate")),
        "{admin}"
    );
    assert!(
        admin.contains(&instance("_i_", "Acme\\On\\Model\\AdminDeep")),
        "{admin}"
    );

    let frontend = arguments_of(&metadata(tmp.path(), "frontend"), "GateChain");
    assert!(
        frontend.contains(&instance("_i_", "GateAlias")),
        "{frontend}"
    );
    assert!(
        frontend.contains(&instance("_i_", "Acme\\On\\Model\\StoreDeep")),
        "{frontend}"
    );
}

#[test]
fn area_shared_setting_reaches_a_global_object_argument() {
    let tmp = compiled_fixture();

    let admin = arguments_of(
        &metadata(tmp.path(), "adminhtml"),
        "Acme\\On\\Model\\Keeper",
    );
    assert!(
        admin.contains(&instance("_ins_", "Acme\\On\\Model\\Tool")),
        "{admin}"
    );

    let frontend = arguments_of(&metadata(tmp.path(), "frontend"), "Acme\\On\\Model\\Keeper");
    assert!(
        frontend.contains(&instance("_i_", "Acme\\On\\Model\\Tool")),
        "{frontend}"
    );
}

#[test]
fn area_preference_on_the_target_of_a_global_preference_is_followed() {
    let tmp = compiled_fixture();

    let admin = arguments_of(
        &metadata(tmp.path(), "adminhtml"),
        "Acme\\On\\Model\\RuleUser",
    );
    assert!(
        admin.contains(&instance("_i_", "Acme\\On\\Model\\AdminRule")),
        "{admin}"
    );

    let frontend = arguments_of(
        &metadata(tmp.path(), "frontend"),
        "Acme\\On\\Model\\RuleUser",
    );
    assert!(
        frontend.contains(&instance("_i_", "Acme\\On\\Model\\StoreRule")),
        "{frontend}"
    );
}
