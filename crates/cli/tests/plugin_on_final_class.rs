//! `Operation\Interception` generates an interceptor for every scanned concrete class with plugins,
//! its own or inherited, and loads it. For a final class that is a PHP fatal error, which ends
//! `setup:di:compile`.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn compile_with(files: &[(&str, String)]) -> Option<Output> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
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
        "vendor/autoload.php",
        "<?php\nspl_autoload_register(function ($class) {\n    \
         $file = dirname(__DIR__) . '/app/code/' . str_replace('\\\\', '/', $class) . '.php';\n    \
         if (is_file($file)) {\n        require $file;\n    }\n});\n",
    );
    write(
        root,
        "app/code/Acme/On/Plugin/Watch.php",
        "<?php\nnamespace Acme\\On\\Plugin;\n\nclass Watch\n{\n    public function beforeRun($subject)\n    {\n    }\n}\n",
    );
    for (relative, content) in files {
        write(root, relative, content);
    }
    Some(
        Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
            .arg("--magento-root")
            .arg(root)
            .output()
            .unwrap(),
    )
}

fn model(name: &str, source: &str) -> (String, String) {
    (
        format!("app/code/Acme/On/Model/{name}.php"),
        format!("<?php\nnamespace Acme\\On\\Model;\n\n{source}\n"),
    )
}

fn plugin_on(owner: &str, attributes: &str) -> (String, String) {
    (
        "app/code/Acme/On/etc/di.xml".to_string(),
        format!(
            "<config><type name=\"Acme\\On\\Model\\{owner}\"><plugin name=\"p\" type=\"Acme\\On\\Plugin\\Watch\"{attributes}/></type></config>\n"
        ),
    )
}

fn run(files: Vec<(String, String)>) -> Option<Output> {
    let files: Vec<(&str, String)> = files
        .iter()
        .map(|(path, content)| (path.as_str(), content.clone()))
        .collect();
    compile_with(&files)
}

fn assert_stops(output: &Output, class: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains(&format!("cannot extend final class {class}")),
        "{stderr}"
    );
}

fn assert_compiles(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn plugin_on_a_final_class_stops_the_compile() {
    let Some(output) = run(vec![
        model(
            "Sealed",
            "final class Sealed\n{\n    public function run()\n    {\n    }\n}",
        ),
        plugin_on("Sealed", ""),
    ]) else {
        return;
    };
    assert_stops(&output, "Acme\\On\\Model\\Sealed");
}

#[test]
fn disabled_plugin_on_a_final_class_stops_the_compile() {
    let Some(output) = run(vec![
        model("Sealed", "final class Sealed\n{\n}"),
        plugin_on("Sealed", " disabled=\"true\""),
    ]) else {
        return;
    };
    assert_stops(&output, "Acme\\On\\Model\\Sealed");
}

#[test]
fn plugin_on_the_parent_of_a_final_class_stops_the_compile() {
    let Some(output) = run(vec![
        model("Base", "class Base\n{\n}"),
        model("Sealed", "final class Sealed extends Base\n{\n}"),
        plugin_on("Base", ""),
    ]) else {
        return;
    };
    assert_stops(&output, "Acme\\On\\Model\\Sealed");
}

#[test]
fn plugin_on_an_interface_of_a_final_class_stops_the_compile() {
    let Some(output) = run(vec![
        model("Contract", "interface Contract\n{\n}"),
        model("Sealed", "final class Sealed implements Contract\n{\n}"),
        plugin_on("Contract", ""),
    ]) else {
        return;
    };
    assert_stops(&output, "Acme\\On\\Model\\Sealed");
}

#[test]
fn final_class_without_plugins_compiles() {
    let Some(output) = run(vec![
        model("Open", "class Open\n{\n}"),
        model("Sealed", "final class Sealed\n{\n}"),
        plugin_on("Open", ""),
    ]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn final_class_that_is_not_interceptable_compiles() {
    let Some(output) = run(vec![
        (
            "app/code/Magento/Framework/ObjectManager/NoninterceptableInterface.php".to_string(),
            "<?php\nnamespace Magento\\Framework\\ObjectManager;\n\ninterface NoninterceptableInterface\n{\n}\n".to_string(),
        ),
        model(
            "Sealed",
            "final class Sealed implements \\Magento\\Framework\\ObjectManager\\NoninterceptableInterface\n{\n}",
        ),
        plugin_on("Sealed", ""),
    ]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn plugin_on_a_readonly_class_stops_the_compile() {
    let Some(output) = run(vec![
        model(
            "Locked",
            "readonly class Locked\n{\n    public function run()\n    {\n    }\n}",
        ),
        plugin_on("Locked", ""),
    ]) else {
        return;
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("cannot extend readonly class Acme\\On\\Model\\Locked"),
        "{stderr}"
    );
}

#[test]
fn the_word_readonly_in_a_comment_compiles() {
    let Some(output) = run(vec![
        model(
            "Open",
            "// A readonly class would break here.\nclass Open\n{\n    public function run()\n    {\n    }\n}",
        ),
        plugin_on("Open", ""),
    ]) else {
        return;
    };
    assert_compiles(&output);
}
