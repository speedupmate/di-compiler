//! `ClassesScanner::includeClass()` loads every scanned class during compile. A file PHP cannot
//! load stops `setup:di:compile`: a syntax error, a missing parent, interface or trait, or a
//! fatal error while linking the class.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn write_store(root: &Path) {
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n        'Acme_Off' => 0,\n    ],\n];\n",
    );
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
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Gadget\n{\n    public function run(int $a)\n    {\n    }\n}\n",
    );
}

fn compile_with(files: &[(&str, &str)]) -> Option<Output> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return None;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    write_store(tmp.path());
    for (relative, content) in files {
        write(tmp.path(), relative, content);
    }
    Some(
        Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
            .arg("--magento-root")
            .arg(tmp.path())
            .output()
            .unwrap(),
    )
}

fn assert_stops(output: &Output, needles: &[&str]) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    for needle in needles {
        assert!(stderr.contains(needle), "missing {needle} in {stderr}");
    }
}

fn assert_compiles(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn model(body: &str) -> String {
    format!("<?php\nnamespace Acme\\On\\Model;\n\n{body}\n")
}

const THING: &str = "app/code/Acme/On/Model/Thing.php";

#[test]
fn syntax_error_stops_the_compile_and_names_the_file() {
    let Some(output) = compile_with(&[(THING, &model("class Thing { public function ( }"))]) else {
        return;
    };
    assert_stops(&output, &["Model/Thing.php", "syntax error"]);
}

#[test]
fn missing_parent_stops_the_compile_and_names_the_class() {
    let Some(output) = compile_with(&[(THING, &model("class Thing extends Gone {}"))]) else {
        return;
    };
    assert_stops(&output, &["Model/Thing.php", "Acme\\On\\Model\\Gone"]);
}

#[test]
fn missing_interface_stops_the_compile_and_names_the_interface() {
    let Some(output) = compile_with(&[(THING, &model("class Thing implements Gone {}"))]) else {
        return;
    };
    assert_stops(&output, &["Model/Thing.php", "Acme\\On\\Model\\Gone"]);
}

#[test]
fn missing_trait_stops_the_compile() {
    let Some(output) = compile_with(&[(THING, &model("class Thing { use Gone; }"))]) else {
        return;
    };
    assert_stops(&output, &["Acme\\On\\Model\\Gone"]);
}

#[test]
fn fatal_error_while_linking_a_class_stops_the_compile_and_names_the_file() {
    let Some(output) = compile_with(&[(
        THING,
        &model("class Thing extends Gadget { public function run(string $a, $b) {} }"),
    )]) else {
        return;
    };
    assert_stops(&output, &["Model/Thing.php", "must be compatible"]);
}

#[test]
fn a_file_that_ends_the_process_stops_the_compile_and_names_the_file() {
    let Some(output) = compile_with(&[(THING, &model("exit(0);\nclass Thing {}"))]) else {
        return;
    };
    assert_stops(&output, &["Model/Thing.php"]);
}

#[test]
fn a_file_that_throws_while_it_loads_stops_the_compile() {
    let Some(output) = compile_with(&[(
        THING,
        &model("throw new \\RuntimeException('stop here');\nclass Thing {}"),
    )]) else {
        return;
    };
    assert_stops(&output, &["Model/Thing.php", "stop here"]);
}

#[test]
fn a_fatal_error_is_reported_when_a_later_class_also_fails() {
    let Some(output) = compile_with(&[
        (
            "app/code/Acme/On/Model/Fatal.php",
            &model("class Fatal extends Gadget { public function run(string $a, $b) {} }"),
        ),
        (
            "app/code/Acme/On/Model/Later.php",
            &model("class Later extends Gone {}"),
        ),
    ]) else {
        return;
    };
    assert_stops(&output, &["Model/Fatal.php"]);
}

#[test]
fn broken_file_in_a_disabled_module_does_not_stop_the_compile() {
    let Some(output) = compile_with(&[(
        "app/code/Acme/Off/Model/Thing.php",
        "<?php\nnamespace Acme\\Off\\Model;\n\nclass Thing extends Gone {}\n",
    )]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn broken_file_in_a_disabled_module_stops_when_the_store_compiles_all_modules() {
    let Some(output) = compile_with(&[
        (
            "setup/src/Magento/Setup/Console/Command/DiCompileCommand.php",
            "<?php\n$modulePaths = $this->componentRegistrar->getPaths(ComponentRegistrar::MODULE);\n\
             $list = ['application' => $modulePaths];\n",
        ),
        (
            "app/code/Acme/Off/Model/Thing.php",
            "<?php\nnamespace Acme\\Off\\Model;\n\nclass Thing extends Gone {}\n",
        ),
    ]) else {
        return;
    };
    assert_stops(&output, &["Off/Model/Thing.php", "Acme\\Off\\Model\\Gone"]);
}

#[test]
fn broken_file_under_test_does_not_stop_the_compile() {
    let Some(output) = compile_with(&[(
        "app/code/Acme/On/Test/Thing.php",
        "<?php\nnamespace Acme\\On\\Test;\n\nclass Thing extends Gone {}\n",
    )]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn parent_that_magento_would_generate_does_not_stop_the_compile() {
    let Some(output) = compile_with(&[
        (
            "app/code/Magento/Framework/ObjectManagerInterface.php",
            "<?php\nnamespace Magento\\Framework;\n\ninterface ObjectManagerInterface\n{\n}\n",
        ),
        (THING, &model("class Thing extends GadgetFactory {}")),
        (
            "app/code/Acme/On/Model/Other.php",
            &model("class Other extends Gadget\\Proxy {}"),
        ),
        (
            "app/code/Acme/On/Model/Third.php",
            &model("class Third implements GadgetExtensionInterface {}"),
        ),
    ]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn parent_declared_in_another_scanned_file_does_not_stop_the_compile() {
    // Whether this works in Magento depends on the order of the folder walk. So it must not stop.
    let Some(output) = compile_with(&[
        (
            "app/code/Acme/On/Model/Aaa.php",
            &model("class Aaa extends NotAtItsOwnPath {}"),
        ),
        (
            "app/code/Acme/On/Model/Zzz.php",
            &model("class NotAtItsOwnPath {}"),
        ),
    ]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn broken_file_of_a_component_magento_does_not_register_does_not_stop_the_compile() {
    let registrar = "<?php\nnamespace Magento\\Framework\\Component;\n\nclass ComponentRegistrar\n{\n    \
         public function getPaths($type)\n    {\n        \
         return $type === 'module' ? ['Acme_On' => dirname(__DIR__, 5) . '/app/code/Acme/On'] : [];\n    \
         }\n}\n";
    let Some(output) = compile_with(&[
        (
            "app/code/Magento/Framework/Component/ComponentRegistrar.php",
            registrar,
        ),
        ("vendor/acme/unregistered/registration.php", "<?php\n"),
        (
            "vendor/acme/unregistered/Model/Thing.php",
            "<?php\nnamespace Acme\\Unregistered\\Model;\n\nclass Thing extends Gone {}\n",
        ),
    ]) else {
        return;
    };
    assert_compiles(&output);
}

const ERROR_HANDLER: &str =
    "<?php\nnamespace Magento\\Framework\\App;\n\nclass ErrorHandler\n{\n    \
     public function handler($number, $text, $file, $line)\n    {\n        \
     if (($number & error_reporting()) === 0) {\n            return false;\n        }\n        \
     throw new \\Exception($text . ' in ' . $file);\n    }\n}\n";

#[test]
fn warning_while_a_class_file_loads_stops_the_compile_as_bin_magento_turns_it_into_an_exception() {
    let Some(output) = compile_with(&[
        (
            "app/code/Magento/Framework/App/ErrorHandler.php",
            ERROR_HANDLER,
        ),
        (
            THING,
            &model("trigger_error('careful', E_USER_WARNING);\nclass Thing {}"),
        ),
    ]) else {
        return;
    };
    assert_stops(&output, &["Model/Thing.php", "careful"]);
}

#[test]
fn deprecation_while_a_class_file_compiles_stops_the_compile() {
    let Some(output) = compile_with(&[
        (
            "app/code/Magento/Framework/App/ErrorHandler.php",
            ERROR_HANDLER,
        ),
        (
            THING,
            &model("class Thing { public function run(Gadget $g = null, $x) {} }"),
        ),
    ]) else {
        return;
    };
    assert_stops(&output, &["Model/Thing.php"]);
}

#[test]
fn suppressed_warning_in_a_class_file_compiles() {
    let Some(output) = compile_with(&[
        (
            "app/code/Magento/Framework/App/ErrorHandler.php",
            ERROR_HANDLER,
        ),
        (
            THING,
            &model("@trigger_error('careful', E_USER_WARNING);\nclass Thing {}"),
        ),
    ]) else {
        return;
    };
    assert_compiles(&output);
}
