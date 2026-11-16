//! `bin/magento setup:di:compile` is a console command. Before it runs,
//! `Cli::getVendorCommands()` creates every command list that a file in
//! `vendor/composer/autoload_files.php` registers with `CommandLocator::register()`, and every
//! command such a list returns. Metadata is not compiled yet, so Magento's generator writes the
//! interceptors and factories those objects ask for, and the compile keeps them. A list can sit
//! in a disabled module, which Magento 2.4.9 does not scan. Without those files the commands
//! fail in every later `bin/magento` run.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
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

fn write_module(root: &Path, directory: &str, module: &str) {
    write(root, &format!("{directory}/registration.php"), "<?php\n");
    write(
        root,
        &format!("{directory}/etc/module.xml"),
        &format!("<config><module name=\"{module}\"/></config>\n"),
    );
}

/// Written from scratch: only the lines that decide which modules `execute()` compiles.
const COMPILE_COMMAND: &str = r#"<?php
namespace Magento\Setup\Console\Command;

class DiCompileCommand
{
    protected function execute()
    {
        $modulePaths = $this->componentRegistrar->getPaths(ComponentRegistrar::MODULE);
        $moduleStatuses = $this->deploymentConfig->get(ConfigOptionsListConstants::KEY_MODULES);
        $modulePathsEnabled = array_filter($modulePaths, function ($path, $module) use ($moduleStatuses) {
            return ($moduleStatuses[$module] ?? 0) === 1;
        }, ARRAY_FILTER_USE_BOTH);
        $compiledPathsList = [
            'application' => $modulePathsEnabled,
        ];
    }
}
"#;

fn write_fixture(root: &Path) {
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n        'Acme_Off' => 0,\n    ],\n];\n",
    );
    write(
        root,
        "setup/src/Magento/Setup/Console/Command/DiCompileCommand.php",
        COMPILE_COMMAND,
    );

    let on = "app/code/Acme/On";
    write_module(root, on, "Acme_On");
    write(
        root,
        &format!("{on}/etc/di.xml"),
        r#"<config>
    <type name="Symfony\Component\Console\Command\Command">
        <plugin name="on_command" type="Acme\On\Plugin\CommandPlugin"/>
    </type>
</config>
"#,
    );
    write_class(root, on, "Acme\\On\\Model\\Widget", "class Widget\n{\n}");
    write_class(
        root,
        on,
        "Acme\\On\\Plugin\\CommandPlugin",
        "class CommandPlugin\n{\n    public function beforeRun($subject, $input, $output)\n    {\n    }\n}",
    );

    let off = "app/code/Acme/Off";
    write_module(root, off, "Acme_Off");
    write(
        root,
        &format!("{off}/cli_commands.php"),
        "<?php\nif (PHP_SAPI == 'cli') {\n    \\Magento\\Framework\\Console\\CommandLocator::register(\\Acme\\Off\\Console\\CommandList::class);\n}\n",
    );
    write_class(
        root,
        off,
        "Acme\\Off\\Console\\CommandList",
        "use Acme\\Off\\Console\\Command\\Deploy;\n\nclass CommandList\n{\n    public function getCommandsClasses()\n    {\n        return [Deploy::class];\n    }\n}",
    );
    write_class(
        root,
        off,
        "Acme\\Off\\Console\\Command\\Deploy",
        "class Deploy extends \\Symfony\\Component\\Console\\Command\\Command\n{\n    public function __construct(\\Acme\\Off\\Model\\Helper $helper)\n    {\n    }\n}",
    );
    write_class(
        root,
        off,
        "Acme\\Off\\Console\\Command\\Unlisted",
        "class Unlisted extends \\Symfony\\Component\\Console\\Command\\Command\n{\n    public function __construct(\\Acme\\On\\Model\\GadgetFactory $factory)\n    {\n    }\n}",
    );
    write_class(
        root,
        off,
        "Acme\\Off\\Model\\Helper",
        "class Helper\n{\n    public function __construct(\\Acme\\On\\Model\\WidgetFactory $widgetFactory, $optional = null)\n    {\n    }\n}",
    );
    write_class(root, on, "Acme\\On\\Model\\Gadget", "class Gadget\n{\n}");

    write_class(
        root,
        "vendor/symfony/console",
        "Symfony\\Component\\Console\\Command\\Command",
        "class Command\n{\n    public function __construct(?string $name = null)\n    {\n    }\n\n    public function run($input, $output)\n    {\n    }\n}",
    );
    write(
        root,
        "vendor/composer/installed.json",
        r#"{"packages": [
    {"name": "acme/off", "install-path": "../../app/code/Acme/Off", "autoload": {"psr-4": {"Acme\\Off\\": ""}}},
    {"name": "symfony/console", "install-path": "../symfony/console", "autoload": {"psr-4": {"Symfony\\Component\\Console\\": ""}}}
]}"#,
    );
    write(
        root,
        "vendor/composer/autoload_files.php",
        "<?php\n\n$vendorDir = dirname(__DIR__);\n$baseDir = dirname($vendorDir);\n\nreturn array(\n    'a1' => $baseDir . '/app/code/Acme/Off/cli_commands.php',\n);\n",
    );
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

fn read(root: &Path, relative: &str) -> String {
    fs::read_to_string(root.join(relative)).unwrap_or_else(|e| panic!("{relative}: {e}"))
}

/// The PHP array key for a class name, as the metadata files write it.
fn key(class: &str) -> String {
    format!("'{}' =>", class.replace('\\', "\\\\"))
}

#[test]
fn command_of_a_registered_list_gets_its_interceptor() {
    let tmp = compiled_fixture();
    let root = tmp.path();

    assert!(root
        .join("generated/code/Acme/Off/Console/Command/Deploy/Interceptor.php")
        .exists());
    let global = read(root, "generated/metadata/global.php");
    let (arguments, preferences) = global.split_once("'preferences' =>").unwrap();
    assert!(preferences.contains(&format!(
        "{} '{}'",
        key("Acme\\Off\\Console\\Command\\Deploy"),
        "Acme\\\\Off\\\\Console\\\\Command\\\\Deploy\\\\Interceptor"
    )));
    // ClassesScanner does not list the module, so Magento resolves the arguments at runtime.
    assert!(!arguments.contains(&key("Acme\\Off\\Console\\Command\\Deploy")));
    assert!(!arguments.contains(&key("Acme\\Off\\Console\\Command\\Deploy\\Interceptor")));
    // The compile finds the generated file, not the class it extends.
    let interception = read(root, "generated/metadata/interception.php");
    assert!(interception.contains(&key("Acme\\Off\\Console\\Command\\Deploy\\Interceptor")));
    assert!(!interception.contains(&key("Acme\\Off\\Console\\Command\\Deploy")));
}

#[test]
fn factory_a_command_needs_is_generated() {
    let tmp = compiled_fixture();
    let root = tmp.path();

    assert!(root
        .join("generated/code/Acme/On/Model/WidgetFactory.php")
        .exists());
    let global = read(root, "generated/metadata/global.php");
    let (arguments, _) = global.split_once("'preferences' =>").unwrap();
    assert!(arguments.contains(&key("Acme\\On\\Model\\WidgetFactory")));
    assert!(!arguments.contains(&key("Acme\\Off\\Model\\Helper")));
}

#[test]
fn command_that_no_list_names_stays_out() {
    let tmp = compiled_fixture();
    let root = tmp.path();

    assert!(!root
        .join("generated/code/Acme/Off/Console/Command/Unlisted/Interceptor.php")
        .exists());
    assert!(!root
        .join("generated/code/Acme/On/Model/GadgetFactory.php")
        .exists());
}
