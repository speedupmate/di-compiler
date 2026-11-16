//! `Operation\Area::doOperation()` writes one metadata file per code of `AreaList::getCodes()`, and
//! a module can add an area through the `areas` argument of `Magento\Framework\App\AreaList`.
//! Without its file, a request to that area is a TypeError in `ObjectManager::configure()`.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn write_module(root: &Path, name: &str) {
    write(
        root,
        &format!("app/code/Acme/{name}/registration.php"),
        "<?php\n",
    );
    write(
        root,
        &format!("app/code/Acme/{name}/etc/module.xml"),
        &format!("<config><module name=\"Acme_{name}\"/></config>\n"),
    );
}

fn area_list(area: &str) -> String {
    format!(
        r#"<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <type name="Magento\Framework\App\AreaList">
        <arguments>
            <argument name="areas" xsi:type="array">
                <item name="{area}" xsi:type="array">
                    <item name="frontName" xsi:type="string">{area}</item>
                </item>
            </argument>
        </arguments>
    </type>
</config>
"#
    )
}

fn compiled_fixture() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n        'Acme_Off' => 0,\n    ],\n];\n",
    );
    write_module(root, "On");
    write_module(root, "Off");
    write(
        root,
        "app/code/Acme/On/etc/di.xml",
        &area_list("custom_api"),
    );
    write(root, "app/code/Acme/Off/etc/di.xml", &area_list("off_api"));
    write(
        root,
        "app/code/Acme/On/etc/custom_api/di.xml",
        r#"<config>
    <preference for="Acme\On\Api\RuleInterface" type="Acme\On\Model\Rule"/>
    <type name="Acme\On\Model\Gadget">
        <plugin name="only_in_custom" type="Acme\On\Plugin\GadgetPlugin"/>
    </type>
</config>
"#,
    );
    write(
        root,
        "app/code/Acme/On/etc/off_api/di.xml",
        r#"<config>
    <preference for="Acme\On\Api\RuleInterface" type="Acme\On\Model\Rule"/>
</config>
"#,
    );
    for (file, source) in [
        ("Api/RuleInterface", "namespace Acme\\On\\Api;\n\ninterface RuleInterface\n{\n}"),
        ("Model/Rule", "namespace Acme\\On\\Model;\n\nclass Rule implements \\Acme\\On\\Api\\RuleInterface\n{\n}"),
        ("Model/Gadget", "namespace Acme\\On\\Model;\n\nclass Gadget\n{\n    public function run()\n    {\n    }\n}"),
        ("Plugin/GadgetPlugin", "namespace Acme\\On\\Plugin;\n\nclass GadgetPlugin\n{\n    public function beforeRun($subject)\n    {\n    }\n}"),
    ] {
        write(root, &format!("app/code/Acme/On/{file}.php"), &format!("<?php\n{source}\n"));
    }
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
        "compiler failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    tmp
}

#[test]
fn area_from_the_area_list_gets_a_metadata_file_with_its_own_config() {
    let tmp = compiled_fixture();

    let area = fs::read_to_string(tmp.path().join("generated/metadata/custom_api.php")).unwrap();
    assert!(
        area.contains("Acme\\\\On\\\\Api\\\\RuleInterface"),
        "{area}"
    );
    let global = fs::read_to_string(tmp.path().join("generated/metadata/global.php")).unwrap();
    assert!(
        !global.contains("Acme\\\\On\\\\Api\\\\RuleInterface' =>"),
        "{global}"
    );
}

#[test]
fn area_from_the_area_list_gets_a_plugin_list_file() {
    let tmp = compiled_fixture();

    let list = fs::read_to_string(
        tmp.path()
            .join("generated/metadata/primary|global|custom_api|plugin-list.php"),
    )
    .unwrap();
    assert!(list.contains("only_in_custom"), "{list}");
}

#[test]
fn plugin_only_in_a_custom_area_gives_an_interceptor() {
    let tmp = compiled_fixture();

    assert!(tmp
        .path()
        .join("generated/code/Acme/On/Model/Gadget/Interceptor.php")
        .exists());
}

#[test]
fn area_of_a_disabled_module_gets_no_file() {
    let tmp = compiled_fixture();

    assert!(!tmp.path().join("generated/metadata/off_api.php").exists());
    assert!(tmp.path().join("generated/metadata/frontend.php").exists());
}
