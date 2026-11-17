//! `DeploymentConfig` includes `app/etc/config.php`, so any valid PHP counts. Magento only reads
//! a module as enabled when its value is exactly 1. These forms must keep the plugins of a module.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn interceptor_is_generated(config_php: &str) -> Option<bool> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return None;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "app/etc/config.php", config_php);
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
        "<?php\nnamespace Acme\\On\\Plugin;\n\nclass Watch\n{\n    public function beforeRun($s)\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/etc/di.xml",
        "<config><type name=\"Acme\\On\\Model\\Gadget\"><plugin name=\"p\" type=\"Acme\\On\\Plugin\\Watch\"/></type></config>\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(
        root.join("generated/code/Acme/On/Model/Gadget/Interceptor.php")
            .exists(),
    )
}

#[test]
fn compact_form_on_one_line_keeps_the_plugin() {
    assert_ne!(
        interceptor_is_generated("<?php return ['modules'=>['Acme_On'=>1,'Acme_Off'=>0]];\n"),
        Some(false)
    );
}

#[test]
fn compact_form_on_several_lines_keeps_the_plugin() {
    assert_ne!(
        interceptor_is_generated(
            "<?php\nreturn [\n'modules'=>[\n'Acme_On'=>1,\n'Acme_Off'=>0,\n],\n];\n"
        ),
        Some(false)
    );
}

#[test]
fn array_syntax_keeps_the_plugin() {
    assert_ne!(
        interceptor_is_generated(
            "<?php\nreturn array(\n  'modules' => array(\n    \"Acme_On\" => 1,\n  ),\n);\n"
        ),
        Some(false)
    );
}

#[test]
fn module_with_value_0_is_disabled_in_compact_form() {
    assert_ne!(
        interceptor_is_generated("<?php return ['modules'=>['Acme_Off'=>1,'Acme_On'=>0]];\n"),
        Some(true)
    );
}

#[test]
fn a_value_that_is_not_exactly_1_is_disabled() {
    assert_ne!(
        interceptor_is_generated(
            "<?php\nreturn [\n'modules' => [\n'Acme_Off' => 1,\n'Acme_On' => '1',\n],\n];\n"
        ),
        Some(true)
    );
}
