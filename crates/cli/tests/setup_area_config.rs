//! Magento 2.4.8 passes `setup/src` to `AREA_CONFIG_GENERATOR`, and 2.4.7 does not. The classes
//! of `setup/src` then have arguments in `generated/metadata` on 2.4.8 only, so the compiler
//! follows the store's own `DiCompileCommand`.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn has_php() -> bool {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return false;
    }
    true
}

const COMPILE_COMMAND: &str = "setup/src/Magento/Setup/Console/Command/DiCompileCommand.php";

/// Written from scratch: the module list of `execute()` and the paths of the area configuration.
fn compile_command(area_config_paths: &str) -> String {
    format!(
        r#"<?php
namespace Magento\Setup\Console\Command;

class DiCompileCommand
{{
    protected function execute()
    {{
        $modulePaths = $this->componentRegistrar->getPaths(ComponentRegistrar::MODULE);
        $moduleStatuses = $this->deploymentConfig->get(ConfigOptionsListConstants::KEY_MODULES);
        $modulePathsEnabled = array_filter($modulePaths, function ($path, $module) use ($moduleStatuses) {{
            return ($moduleStatuses[$module] ?? 0) === 1;
        }}, ARRAY_FILTER_USE_BOTH);
        $compiledPathsList = [
            'application' => $modulePathsEnabled,
            'library' => $libraryPaths,
            'setup' => $setupPath,
        ];
    }}

    private function getOperationsConfiguration(array $compiledPathsList)
    {{
        return [
            OperationFactory::AREA_CONFIG_GENERATOR => [
{area_config_paths}
            ],
            OperationFactory::INTERCEPTION_CACHE => [
                $compiledPathsList['application'],
                $compiledPathsList['setup'],
            ],
        ];
    }}
}}
"#
    )
}

const PATHS_2_4_7: &str = "                $compiledPathsList['application'],
                $compiledPathsList['library'],
                $compiledPathsList['generated_helpers'],";

const PATHS_2_4_8: &str = "                $compiledPathsList['application'],
                $compiledPathsList['library'],
                $compiledPathsList['setup'],
                $compiledPathsList['generated_helpers'],";

/// `global.php` of a store with the given `AREA_CONFIG_GENERATOR` paths.
fn global_metadata(area_config_paths: &str) -> Option<String> {
    if !has_php() {
        return None;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
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
        "app/code/Acme/On/Model/Gadget.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Gadget\n{\n}\n",
    );
    write(
        root,
        "vendor/autoload.php",
        "<?php\nspl_autoload_register(function ($class) {\n    \
         foreach (['/app/code/', '/setup/src/'] as $base) {\n        \
         $file = dirname(__DIR__) . $base . str_replace('\\\\', '/', $class) . '.php';\n        \
         if (is_file($file)) {\n            require $file;\n            return;\n        }\n    }\n});\n",
    );
    write(root, COMPILE_COMMAND, &compile_command(area_config_paths));
    write(
        root,
        "setup/src/Magento/Setup/Acme/Tool.php",
        "<?php\nnamespace Magento\\Setup\\Acme;\n\nclass Tool\n{\n    \
         public function __construct(\\Acme\\On\\Model\\Gadget $gadget)\n    {\n    }\n}\n",
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
    Some(fs::read_to_string(root.join("generated/metadata/global.php")).unwrap())
}

const TOOL: &str = "'Magento\\\\Setup\\\\Acme\\\\Tool' => ";
const GADGET: &str = "'Acme\\\\On\\\\Model\\\\Gadget' => ";

#[test]
fn magento_2_4_7_writes_no_arguments_for_setup_classes() {
    let Some(global) = global_metadata(PATHS_2_4_7) else {
        return;
    };
    assert!(!global.contains(TOOL), "{global}");
    assert!(global.contains(GADGET), "{global}");
}

#[test]
fn magento_2_4_8_writes_arguments_for_setup_classes() {
    let Some(global) = global_metadata(PATHS_2_4_8) else {
        return;
    };
    assert!(global.contains(TOOL), "{global}");
    assert!(global.contains(GADGET), "{global}");
}
