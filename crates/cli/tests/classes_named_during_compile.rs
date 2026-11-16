//! Magento generates a Factory or Proxy the moment compile asks for its name:
//! `Compiler\Config\Reader::getConfigForScope()` reflects every virtual type
//! target, `ClassReader::getConstructor()` reflects every constructor type, and
//! `ClassesScanner::includeClass()` runs the file-level code of every class file.
//! That code decides at compile time which classes get generated and which
//! declaration of a class counts, so these tests run real PHP.

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
    let relative = class.replace('\\', "/");
    write(
        root,
        &format!("app/code/{relative}.php"),
        &format!("<?php\nnamespace {namespace};\n\n{source}\n"),
    );
}

fn write_fixture(root: &Path) {
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
        "app/code/Acme/On/etc/di.xml",
        r#"<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <virtualType name="widgetFactory" type="Acme\On\Model\WidgetFactory"/>
    <virtualType name="goneFactory" type="Acme\On\Model\GoneFactory"/>
    <virtualType name="Acme\On\Model\AliasFactory" type="Acme\On\Model\Widget"/>
    <virtualType name="viaAlias" type="Acme\On\Model\AliasFactory"/>
</config>
"#,
    );

    write_class(root, "Acme\\On\\Model\\Widget", "class Widget\n{\n}");
    write_class(root, "Acme\\On\\Model\\Gadget", "class Gadget\n{\n}");
    write_class(
        root,
        "Acme\\On\\Model\\Keeper",
        "use Acme\\On\\Model\\Gadget\\Proxy as Gadget;\n\nclass Keeper\n{\n    \
         public function __construct(Gadget $gadget)\n    {\n    }\n}",
    );

    for name in ["Present", "Checked", "Computed"] {
        write_class(
            root,
            &format!("Acme\\On\\Model\\{name}"),
            &format!("class {name}\n{{\n}}"),
        );
    }
    write_class(
        root,
        "Acme\\On\\Compat\\Present",
        "use Acme\\On\\Model\\PresentFactory as Dep;\n\n\
         if (class_exists(Dep::class)):\n    class Present\n    {\n        \
         public function __construct(Dep $dep)\n        {\n        }\n    }\n\
         else:\n    class Present\n    {\n    }\nendif;",
    );
    write_class(
        root,
        "Acme\\On\\Compat\\Absent",
        "use Acme\\Gone\\ThingFactory as Dep;\n\n\
         if (class_exists(Dep::class)):\n    class Absent\n    {\n        \
         public function __construct(Dep $dep)\n        {\n        }\n    }\n\
         else:\n    class Absent\n    {\n        \
         public function __construct(\\Acme\\On\\Model\\Widget $widget)\n        {\n        }\n    }\nendif;",
    );
    write_class(
        root,
        "Acme\\On\\Compat\\CheckOnly",
        "if (class_exists(\\Acme\\On\\Model\\CheckedFactory::class)) {\n    \\strlen('x');\n}\n\n\
         class CheckOnly\n{\n}",
    );
    write_class(
        root,
        "Acme\\On\\Compat\\ComputedName",
        "$name = 'Acme\\\\On\\\\Model\\\\' . 'Computed' . 'Factory';\n\
         if (class_exists($name)) {\n    \\strlen('x');\n}\n\nclass ComputedName\n{\n}",
    );
}

/// The arguments of one class in a metadata file, or `None` if it has no entry.
fn arguments_of(metadata: &str, name: &str) -> Option<String> {
    let (arguments, _) = metadata.split_once("'preferences' =>").unwrap();
    let start = arguments.find(&format!("'{}' =>", name.replace('\\', "\\\\")))?;
    let rest = &arguments[start..];
    let end = rest[1..].find("\n    '").map_or(rest.len(), |at| at + 1);
    Some(rest[..end].to_string())
}

fn compiled_fixture() -> Option<tempfile::TempDir> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return None;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    write_fixture(tmp.path());
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "compiler failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(tmp)
}

fn generated(root: &Path, class: &str) -> bool {
    root.join("generated/code")
        .join(format!("{}.php", class.replace('\\', "/")))
        .exists()
}

fn global_metadata(root: &Path) -> String {
    fs::read_to_string(root.join("generated/metadata/global.php")).unwrap()
}

#[test]
fn factory_named_only_as_a_virtual_type_target_is_generated() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let root = tmp.path();

    assert!(generated(root, "Acme\\On\\Model\\WidgetFactory"));
    let arguments = arguments_of(&global_metadata(root), "widgetFactory").unwrap();
    assert!(arguments.contains("'objectManager'"), "{arguments}");
    assert!(arguments.contains("'instanceName'"), "{arguments}");
}

#[test]
fn virtual_type_target_is_not_generated_without_its_source_class() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let root = tmp.path();

    assert!(!generated(root, "Acme\\On\\Model\\GoneFactory"));
}

#[test]
fn virtual_type_target_that_is_a_virtual_type_is_not_generated() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };

    assert!(!generated(tmp.path(), "Acme\\On\\Model\\AliasFactory"));
}

#[test]
fn proxy_named_only_as_a_constructor_type_is_generated() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };

    assert!(generated(tmp.path(), "Acme\\On\\Model\\Gadget\\Proxy"));
}

#[test]
fn file_level_check_generates_the_class_and_keeps_the_branch_php_takes() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let root = tmp.path();

    assert!(generated(root, "Acme\\On\\Model\\PresentFactory"));
    let arguments = arguments_of(&global_metadata(root), "Acme\\On\\Compat\\Present").unwrap();
    assert!(arguments.contains("PresentFactory"), "{arguments}");
}

#[test]
fn file_level_check_on_a_class_without_source_takes_the_other_branch() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let root = tmp.path();

    assert!(!generated(root, "Acme\\Gone\\ThingFactory"));
    let arguments = arguments_of(&global_metadata(root), "Acme\\On\\Compat\\Absent").unwrap();
    assert!(arguments.contains("'widget'"), "{arguments}");
    assert!(!arguments.contains("ThingFactory"), "{arguments}");
}

#[test]
fn file_level_check_alone_generates_the_class() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };

    assert!(generated(tmp.path(), "Acme\\On\\Model\\CheckedFactory"));
}

#[test]
fn file_level_check_on_a_computed_name_generates_the_class() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };

    assert!(generated(tmp.path(), "Acme\\On\\Model\\ComputedFactory"));
}
