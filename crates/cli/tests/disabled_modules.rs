//! Magento compiles only enabled modules: `DiCompileCommand::execute()` keeps a
//! module path only when its status in `app/etc/config.php` is 1. Classes and
//! `extension_attributes.xml` from any other module must not reach the output,
//! but enabled code can still extend or name a class from a disabled module.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn write_module(root: &Path, directory: &str, module: &str) {
    write(
        root,
        &format!("{directory}/registration.php"),
        "<?php\n// The compiler reads the module name from etc/module.xml.\n",
    );
    write(
        root,
        &format!("{directory}/etc/module.xml"),
        &format!("<config><module name=\"{module}\"/></config>\n"),
    );
}

fn write_class(root: &Path, directory: &str, class: &str, source: &str) {
    let (namespace, _) = class.rsplit_once('\\').unwrap();
    let relative = class.split('\\').skip(2).collect::<Vec<_>>().join("/");
    write(
        root,
        &format!("{directory}/{relative}.php"),
        &format!("<?php\nnamespace {namespace};\n\n{source}\n"),
    );
}

fn extension_attribute_xml(code: &str) -> String {
    format!(
        "<config><extension_attributes for=\"Acme\\On\\Api\\Data\\ThingInterface\">\
         <attribute code=\"{code}\" type=\"string\"/>\
         </extension_attributes></config>\n"
    )
}

/// One enabled module, one module set to 0, one module that config.php does not
/// list and one library. Magento treats a missing status as disabled.
fn write_fixture(root: &Path) {
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n        'Acme_Off' => 0,\n    ],\n];\n",
    );

    let on = "app/code/Acme/On";
    write_module(root, on, "Acme_On");
    write(
        root,
        &format!("{on}/etc/extension_attributes.xml"),
        &extension_attribute_xml("from_enabled"),
    );
    write(
        root,
        &format!("{on}/etc/di.xml"),
        r#"<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <type name="Acme\Tools\Service">
        <plugin name="on_service" type="Acme\On\Plugin\ServicePlugin"/>
    </type>
    <type name="Acme\Off\Model\Owned">
        <plugin name="on_owned" type="Acme\On\Plugin\ServicePlugin"/>
    </type>
    <type name="Acme\On\Model\Keeper">
        <arguments>
            <argument name="gateway" xsi:type="object">Acme\Off\Api\GatewayInterface\Proxy</argument>
        </arguments>
    </type>
</config>
"#,
    );
    write_class(
        root,
        on,
        "Acme\\On\\Api\\Data\\ThingInterface",
        "interface ThingInterface\n{\n    /** @return \\Acme\\On\\Api\\Data\\ThingExtensionInterface|null */\n    public function getExtensionAttributes();\n}",
    );
    write_class(
        root,
        on,
        "Acme\\On\\Controller\\Index\\Index",
        "class Index\n{\n}",
    );
    write_class(root, on, "Acme\\On\\Model\\Enabled", "class Enabled\n{\n}");
    write_class(
        root,
        on,
        "Acme\\On\\Model\\Keeper",
        "class Keeper\n{\n    public function __construct(\\Acme\\On\\Model\\EnabledFactory $factory, $gateway = null)\n    {\n    }\n}",
    );
    write_class(
        root,
        on,
        "Acme\\On\\Model\\Child",
        "class Child extends \\Acme\\Off\\Model\\Base\n{\n}",
    );
    write_class(
        root,
        on,
        "Acme\\On\\Plugin\\ServicePlugin",
        "class ServicePlugin\n{\n    public function beforeRun($subject)\n    {\n    }\n}",
    );

    for (directory, module, namespace) in [
        ("app/code/Acme/Off", "Acme_Off", "Acme\\Off"),
        ("app/code/Acme/Unlisted", "Acme_Unlisted", "Acme\\Unlisted"),
    ] {
        write_module(root, directory, module);
        write(
            root,
            &format!("{directory}/etc/extension_attributes.xml"),
            &extension_attribute_xml("from_disabled"),
        );
        write_class(
            root,
            directory,
            &format!("{namespace}\\Controller\\Index\\Index"),
            "class Index\n{\n}",
        );
        write_class(
            root,
            directory,
            &format!("{namespace}\\Model\\Hidden"),
            &format!(
                "class Hidden\n{{\n    public function __construct(\\{namespace}\\Model\\DisabledFactory $factory)\n    {{\n    }}\n}}"
            ),
        );
    }
    write_class(
        root,
        "app/code/Acme/Off",
        "Acme\\Off\\Model\\Base",
        "class Base extends \\Acme\\Tools\\Service\n{\n}",
    );
    write_class(
        root,
        "app/code/Acme/Off",
        "Acme\\Off\\Model\\Owned",
        "class Owned\n{\n    public function run()\n    {\n    }\n}",
    );
    write_class(
        root,
        "app/code/Acme/Off",
        "Acme\\Off\\Api\\GatewayInterface",
        "interface GatewayInterface\n{\n    public function send();\n}",
    );

    // A library has no status, even when the package also ships etc/module.xml.
    write(
        root,
        "vendor/acme/tools/registration.php",
        "<?php\n\\Magento\\Framework\\Component\\ComponentRegistrar::register(\n    \\Magento\\Framework\\Component\\ComponentRegistrar::LIBRARY,\n    'Acme_Tools',\n    __DIR__\n);\n",
    );
    write(
        root,
        "vendor/acme/tools/etc/module.xml",
        "<config><module name=\"Acme_Tools\"/></config>\n",
    );
    write_class(
        root,
        "vendor/acme/tools",
        "Acme\\Tools\\Service",
        "class Service\n{\n    public function run()\n    {\n    }\n}",
    );

    // The disabled module stays loadable through Composer, as in a real install.
    write(
        root,
        "vendor/composer/installed.json",
        r#"{"packages": [{"name": "acme/off", "install-path": "../../app/code/Acme/Off", "autoload": {"psr-4": {"Acme\\Off\\": ""}}}]}"#,
    );
}

fn compile(root: &Path) {
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
}

fn read(root: &Path, relative: &str) -> String {
    fs::read_to_string(root.join(relative)).unwrap_or_else(|e| panic!("{relative}: {e}"))
}

/// The PHP array key for a class name, as the metadata files write it.
fn key(class: &str) -> String {
    format!("'{}' =>", class.replace('\\', "\\\\"))
}

fn compiled_fixture() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    write_fixture(tmp.path());
    compile(tmp.path());
    tmp
}

#[test]
fn classes_of_disabled_modules_are_not_compiled() {
    let tmp = compiled_fixture();
    let root = tmp.path();

    let interception = read(root, "generated/metadata/interception.php");
    let global = read(root, "generated/metadata/global.php");
    for class in ["Acme\\On\\Model\\Keeper", "Acme\\Tools\\Service"] {
        assert!(interception.contains(&key(class)), "{class} is not scanned");
    }
    for class in [
        "Acme\\Off\\Model\\Hidden",
        "Acme\\Off\\Controller\\Index\\Index",
        "Acme\\Unlisted\\Model\\Hidden",
        "Acme\\Unlisted\\Controller\\Index\\Index",
    ] {
        assert!(!interception.contains(&key(class)), "{class} is scanned");
        assert!(!global.contains(&key(class)), "{class} has arguments");
    }

    assert!(root
        .join("generated/code/Acme/On/Model/EnabledFactory.php")
        .exists());
    assert!(!root
        .join("generated/code/Acme/Off/Model/DisabledFactory.php")
        .exists());
    assert!(!root.join("generated/code/Acme/Unlisted").exists());
}

#[test]
fn actions_of_disabled_modules_are_not_listed() {
    let tmp = compiled_fixture();

    let actions = read(tmp.path(), "generated/metadata/app_action_list.php");
    assert!(actions.contains("acme\\\\on\\\\controller\\\\index\\\\index"));
    assert!(!actions.contains("acme\\\\off\\\\"));
    assert!(!actions.contains("acme\\\\unlisted\\\\"));
}

#[test]
fn extension_attributes_of_disabled_modules_are_ignored() {
    let tmp = compiled_fixture();

    let extension = read(
        tmp.path(),
        "generated/code/Acme/On/Api/Data/ThingExtension.php",
    );
    assert!(extension.contains("getFromEnabled"));
    assert!(!extension.contains("getFromDisabled"));
}

#[test]
fn disabled_parent_of_an_intercepted_class_gets_an_interceptor() {
    let tmp = compiled_fixture();
    let root = tmp.path();

    assert!(root
        .join("generated/code/Acme/On/Model/Child/Interceptor.php")
        .exists());
    assert!(root
        .join("generated/code/Acme/Off/Model/Base/Interceptor.php")
        .exists());

    assert!(
        read(root, "generated/metadata/interception.php").contains(&key("Acme\\Off\\Model\\Base"))
    );
    let global = read(root, "generated/metadata/global.php");
    let (arguments, preferences) = global.split_once("'preferences' =>").unwrap();
    assert!(arguments.contains(&key("Acme\\On\\Model\\Child")));
    assert!(!arguments.contains(&key("Acme\\Off\\Model\\Base")));
    assert!(!arguments.contains(&key("Acme\\Off\\Model\\Base\\Interceptor")));
    assert!(preferences.contains(&key("Acme\\Off\\Model\\Base")));
}

#[test]
fn plugin_on_a_disabled_class_gives_no_interceptor() {
    let tmp = compiled_fixture();
    let root = tmp.path();

    assert!(!root
        .join("generated/code/Acme/Off/Model/Owned/Interceptor.php")
        .exists());
    assert!(
        read(root, "generated/metadata/interception.php").contains(&key("Acme\\Off\\Model\\Owned"))
    );
    assert!(!read(root, "generated/metadata/global.php").contains(&key("Acme\\Off\\Model\\Owned")));
}

#[test]
fn proxy_named_by_enabled_code_is_generated_for_a_disabled_class() {
    let tmp = compiled_fixture();

    assert!(tmp
        .path()
        .join("generated/code/Acme/Off/Api/GatewayInterface/Proxy.php")
        .exists());
}

const COMPILE_COMMAND: &str = "setup/src/Magento/Setup/Console/Command/DiCompileCommand.php";

/// Written from scratch: only the lines that decide which modules `execute()` compiles.
fn compile_command(execute_body: &str) -> String {
    format!(
        "<?php\nnamespace Magento\\Setup\\Console\\Command;\n\nclass DiCompileCommand\n{{\n    protected function execute()\n    {{\n{execute_body}\n    }}\n}}\n"
    )
}

const COMMAND_WITH_ALL_MODULES: &str = r#"        $modulePaths = $this->componentRegistrar->getPaths(ComponentRegistrar::MODULE);
        $compiledPathsList = [
            'application' => $modulePaths,
            'library' => $libraryPaths,
        ];"#;

const COMMAND_WITH_ENABLED_MODULES: &str = r#"        $modulePaths = $this->componentRegistrar->getPaths(ComponentRegistrar::MODULE);
        $moduleStatuses = $this->deploymentConfig->get(ConfigOptionsListConstants::KEY_MODULES);
        $modulePathsEnabled = array_filter($modulePaths, function ($path, $module) use ($moduleStatuses) {
            return ($moduleStatuses[$module] ?? 0) === 1;
        }, ARRAY_FILTER_USE_BOTH);
        $compiledPathsList = [
            'application' => $modulePathsEnabled,
            'library' => $libraryPaths,
        ];"#;

fn compiled_with_command(execute_body: &str) -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    write_fixture(tmp.path());
    write(tmp.path(), COMPILE_COMMAND, &compile_command(execute_body));
    compile(tmp.path());
    tmp
}

#[test]
fn store_whose_command_compiles_all_modules_scans_disabled_classes() {
    let tmp = compiled_with_command(COMMAND_WITH_ALL_MODULES);
    let root = tmp.path();

    let global = read(root, "generated/metadata/global.php");
    let (arguments, _) = global.split_once("'preferences' =>").unwrap();
    assert!(arguments.contains(&key("Acme\\Off\\Model\\Hidden")));
    assert!(root
        .join("generated/code/Acme/Off/Model/DisabledFactory.php")
        .exists());
    assert!(root
        .join("generated/code/Acme/Unlisted/Model/DisabledFactory.php")
        .exists());
}

#[test]
fn store_whose_command_compiles_all_modules_gives_a_disabled_class_its_interceptor() {
    let tmp = compiled_with_command(COMMAND_WITH_ALL_MODULES);
    let root = tmp.path();

    assert!(root
        .join("generated/code/Acme/Off/Model/Owned/Interceptor.php")
        .exists());
    let global = read(root, "generated/metadata/global.php");
    let (_, preferences) = global.split_once("'preferences' =>").unwrap();
    assert!(preferences.contains(&key("Acme\\Off\\Model\\Owned")));
}

#[test]
fn store_whose_command_compiles_all_modules_still_reads_only_enabled_module_config() {
    let tmp = compiled_with_command(COMMAND_WITH_ALL_MODULES);
    let root = tmp.path();

    // `Dir\Reader::getActionFiles()` and the extension attributes reader use the module list.
    let actions = read(root, "generated/metadata/app_action_list.php");
    assert!(actions.contains("acme\\\\on\\\\controller\\\\index\\\\index"));
    assert!(!actions.contains("acme\\\\off\\\\"));
    assert!(!actions.contains("acme\\\\unlisted\\\\"));
    let extension = read(root, "generated/code/Acme/On/Api/Data/ThingExtension.php");
    assert!(!extension.contains("getFromDisabled"));
}

#[test]
fn store_whose_command_filters_modules_does_not_scan_disabled_classes() {
    let tmp = compiled_with_command(COMMAND_WITH_ENABLED_MODULES);
    let root = tmp.path();

    let global = read(root, "generated/metadata/global.php");
    assert!(!global.contains(&key("Acme\\Off\\Model\\Hidden")));
    assert!(!root
        .join("generated/code/Acme/Off/Model/Owned/Interceptor.php")
        .exists());
}

#[test]
fn commented_out_line_in_the_command_does_not_count() {
    let body = format!(
        "        // 'application' => $modulePathsEnabled,\n        /* 'application' => $other, */\n{COMMAND_WITH_ALL_MODULES}"
    );
    let tmp = compiled_with_command(&body);

    assert!(tmp
        .path()
        .join("generated/code/Acme/Off/Model/Owned/Interceptor.php")
        .exists());
}

#[test]
fn command_of_an_unknown_shape_stops_the_compile() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_fixture(tmp.path());
    write(
        tmp.path(),
        COMPILE_COMMAND,
        &compile_command("        $compiledPathsList = $this->somethingNew();"),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(tmp.path())
        .arg("--fallback-php")
        .arg(tmp.path().join("no-php-here"))
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("DiCompileCommand.php"), "{stderr}");
}
