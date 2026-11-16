//! `bin/magento` creates the real ObjectManager before `setup:di:compile` loads any class, so a
//! class file can use `ObjectManager::getInstance()` while it loads. A failure in that code still
//! stops the compile. Everything Magento-shaped here is a small stand-in written for this test.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// `get()` on this ObjectManager knows one service. `started` shows how often the store booted.
fn write_framework(root: &Path, bootstrap_create: &str) {
    write(
        root,
        "lib/Magento/Framework/ObjectManagerInterface.php",
        "<?php\nnamespace Magento\\Framework;\n\ninterface ObjectManagerInterface\n{\n    public function create($type, array $arguments = []);\n    public function get($type);\n    public function configure(array $configuration);\n}\n",
    );
    write(
        root,
        "lib/Magento/Framework/App/ObjectManager.php",
        r#"<?php
namespace Magento\Framework\App;

class ObjectManager implements \Magento\Framework\ObjectManagerInterface
{
    protected static $_instance;

    public static function getInstance()
    {
        if (!self::$_instance instanceof \Magento\Framework\ObjectManagerInterface) {
            throw new \RuntimeException('ObjectManager is not started');
        }
        return self::$_instance;
    }

    public static function setInstance(\Magento\Framework\ObjectManagerInterface $objectManager)
    {
        self::$_instance = $objectManager;
    }

    public function create($type, array $arguments = [])
    {
        return $this->get($type);
    }

    public function get($type)
    {
        if ($type !== 'Known') {
            throw new \DomainException('No such service: ' . $type);
        }
        return new class {
            public function answer()
            {
                return 'yes';
            }
        };
    }

    public function configure(array $configuration)
    {
    }
}
"#,
    );
    write(
        root,
        "lib/Magento/Framework/App/Bootstrap.php",
        &format!(
            r#"<?php
namespace Magento\Framework\App;

class Bootstrap
{{
    const PARAM_REQUIRE_MAINTENANCE = 'MAGE_REQUIRE_MAINTENANCE';
    const INIT_PARAM_FILESYSTEM_DIR_PATHS = 'MAGE_DIRS';

    public static function create($rootDir, array $initParams)
    {{
        {bootstrap_create}
        return new self();
    }}

    public function getObjectManager()
    {{
        $objectManager = new ObjectManager();
        ObjectManager::setInstance($objectManager);
        return $objectManager;
    }}
}}
"#
        ),
    );
}

fn write_store(root: &Path) {
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn ['modules' => ['Acme_On' => 1]];\n",
    );
    write(root, "app/code/Acme/On/registration.php", "<?php\n");
    write(
        root,
        "app/code/Acme/On/etc/module.xml",
        "<config><module name=\"Acme_On\"/></config>\n",
    );
    write(
        root,
        "vendor/autoload.php",
        "<?php\nspl_autoload_register(function ($class) {\n    \
         $path = str_replace('\\\\', '/', $class) . '.php';\n    \
         foreach (['/app/code/', '/lib/'] as $base) {\n        \
         if (is_file(dirname(__DIR__) . $base . $path)) {\n            \
         require dirname(__DIR__) . $base . $path;\n            return;\n        }\n    }\n});\n",
    );
}

fn compile(
    files: &[(&str, &str)],
    bootstrap_create: Option<&str>,
) -> Option<(Output, tempfile::TempDir)> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return None;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    write_store(tmp.path());
    if let Some(bootstrap_create) = bootstrap_create {
        write_framework(tmp.path(), bootstrap_create);
    }
    for (relative, content) in files {
        write(tmp.path(), relative, content);
    }
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(tmp.path())
        .output()
        .unwrap();
    Some((output, tmp))
}

fn assert_compiles(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_stops(output: &Output, needles: &[&str]) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    for needle in needles {
        assert!(stderr.contains(needle), "missing {needle} in {stderr}");
    }
}

const THING: &str = "app/code/Acme/On/Model/Thing.php";

const ASKS_AT_FILE_LEVEL: &str = "<?php\nnamespace Acme\\On\\Model;\n\n$answer = \\Magento\\Framework\\App\\ObjectManager::getInstance()->get('Known')->answer();\n\nclass Thing\n{\n    public function run()\n    {\n    }\n}\n";

#[test]
fn class_file_that_uses_the_object_manager_while_it_loads_compiles() {
    let Some((output, _tmp)) = compile(&[(THING, ASKS_AT_FILE_LEVEL)], Some("")) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn failure_inside_the_object_manager_call_still_stops_the_compile() {
    let source = ASKS_AT_FILE_LEVEL.replace("'Known'", "'Unknown'");
    let Some((output, _tmp)) = compile(&[(THING, &source)], Some("")) else {
        return;
    };
    assert_stops(
        &output,
        &["Acme\\On\\Model\\Thing", "No such service: Unknown"],
    );
}

#[test]
fn store_whose_object_manager_cannot_start_stops_the_compile_with_the_reason() {
    let Some((output, _tmp)) = compile(
        &[(THING, ASKS_AT_FILE_LEVEL)],
        Some("throw new \\LogicException('Boot went wrong');"),
    ) else {
        return;
    };
    assert_stops(&output, &["Acme\\On\\Model\\Thing", "Boot went wrong"]);
}

#[test]
fn store_is_not_started_when_no_class_file_asks_for_the_object_manager() {
    let marker = "fopen(dirname(__DIR__, 4) . '/booted', 'w');";
    let Some((output, tmp)) = compile(
        &[(
            THING,
            "<?php\nnamespace Acme\\On\\Model;\n\nclass Thing\n{\n}\n",
        )],
        Some(marker),
    ) else {
        return;
    };
    assert_compiles(&output);
    assert!(!tmp.path().join("booted").exists());
}

#[test]
fn plugin_class_that_uses_the_object_manager_while_it_loads_compiles() {
    let plugin = ASKS_AT_FILE_LEVEL.replace("class Thing", "class ThingPlugin");
    let Some((output, tmp)) = compile(
        &[
            (
                "app/code/Acme/On/Model/Gadget.php",
                "<?php\nnamespace Acme\\On\\Model;\n\nclass Gadget\n{\n    public function run()\n    {\n    }\n}\n",
            ),
            ("app/code/Acme/On/Model/ThingPlugin.php", &plugin),
            (
                "app/code/Acme/On/etc/di.xml",
                "<config><type name=\"Acme\\On\\Model\\Gadget\"><plugin name=\"p\" type=\"Acme\\On\\Model\\ThingPlugin\"/></type></config>\n",
            ),
        ],
        Some(""),
    ) else {
        return;
    };
    assert_compiles(&output);
    assert!(tmp
        .path()
        .join("generated/code/Acme/On/Model/Gadget/Interceptor.php")
        .exists());
}

#[test]
fn class_file_with_a_class_check_declares_the_branch_the_object_manager_picks() {
    let source = "<?php\nnamespace Acme\\On\\Model;\n\nif (class_exists('Acme\\\\On\\\\Model\\\\Nothing') || \\Magento\\Framework\\App\\ObjectManager::getInstance()->get('Known')->answer() !== 'yes'):\n    class Thing\n    {\n        public function __construct(int $first)\n        {\n        }\n    }\nelse:\n    class Thing\n    {\n        public function __construct(string $second = 'x')\n        {\n        }\n    }\nendif;\n";
    let Some((output, tmp)) = compile(&[(THING, source)], Some("")) else {
        return;
    };
    assert_compiles(&output);
    let global = fs::read_to_string(tmp.path().join("generated/metadata/global.php")).unwrap();
    assert!(global.contains("second"), "{global}");
    assert!(!global.contains("first"), "{global}");
}

#[test]
fn store_without_the_framework_keeps_the_php_error() {
    let Some((output, _tmp)) = compile(&[(THING, ASKS_AT_FILE_LEVEL)], None) else {
        return;
    };
    assert_stops(
        &output,
        &[
            "Acme\\On\\Model\\Thing",
            "Magento\\Framework\\App\\ObjectManager",
        ],
    );
}
