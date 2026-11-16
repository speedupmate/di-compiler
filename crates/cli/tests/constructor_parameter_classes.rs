//! `ClassReader::getConstructor()` reflects the class of every constructor parameter of every
//! scanned class. A class it cannot find stops the compile with "Impossible to process
//! constructor argument". PHP sees `self` there as a class named `self`, so that fails too.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn compile_with(files: &[(&str, &str)]) -> Option<Output> {
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
        "app/code/Acme/On/Model/Gadget.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Gadget\n{\n}\n",
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

fn assert_stops(output: &Output, class: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("Impossible to process constructor argument"),
        "{stderr}"
    );
    assert!(stderr.contains(class), "{stderr}");
}

fn assert_compiles(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn thing(body: &str) -> String {
    format!("<?php\nnamespace Acme\\On\\Model;\n\n{body}\n")
}

const THING: &str = "app/code/Acme/On/Model/Thing.php";

#[test]
fn missing_parameter_class_stops_the_compile() {
    let source = thing("class Thing { public function __construct(Gone $gone) {} }");
    let Some(output) = compile_with(&[(THING, &source)]) else {
        return;
    };
    assert_stops(&output, "Acme\\On\\Model\\Thing");
}

#[test]
fn factory_of_a_missing_class_stops_the_compile() {
    let source = thing("class Thing { public function __construct(GoneFactory $gone) {} }");
    let Some(output) = compile_with(&[(THING, &source)]) else {
        return;
    };
    assert_stops(&output, "Acme\\On\\Model\\Thing");
}

#[test]
fn parameter_typed_self_stops_the_compile() {
    let source = thing("class Thing { public function __construct(self $other) {} }");
    let Some(output) = compile_with(&[(THING, &source)]) else {
        return;
    };
    assert_stops(&output, "Acme\\On\\Model\\Thing");
}

#[test]
fn missing_class_in_an_inherited_constructor_stops_the_compile() {
    let parent = thing("class Base { public function __construct(Gone $gone) {} }");
    let child = thing("class Thing extends Base {}");
    let Some(output) = compile_with(&[
        ("app/code/Acme/On/Model/Base.php", &parent),
        (THING, &child),
    ]) else {
        return;
    };
    assert_stops(&output, "Acme\\On\\Model\\");
}

#[test]
fn factory_and_proxy_of_an_existing_class_compile() {
    let source = thing(
        "class Thing { public function __construct(GadgetFactory $f, Gadget\\Proxy $p, ?Gadget $g = null, int $n = 1, $any = null) {} }",
    );
    let Some(output) = compile_with(&[(THING, &source)]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn union_type_with_a_missing_class_compiles_because_magento_skips_unions() {
    let source = thing("class Thing { public function __construct(Gone|Gadget $either) {} }");
    let Some(output) = compile_with(&[(THING, &source)]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn missing_class_in_a_plain_method_compiles() {
    let source = thing("class Thing { public function run(Gone $gone) {} }");
    let Some(output) = compile_with(&[(THING, &source)]) else {
        return;
    };
    assert_compiles(&output);
}

#[test]
fn class_inside_if_compiles_because_magento_does_not_scan_it() {
    let source = thing("if (class_exists(\\Acme\\Gone::class)) {\n    class Thing {}\n}");
    let Some(output) = compile_with(&[(THING, &source)]) else {
        return;
    };
    assert_compiles(&output);
}

/// PHP reads a reserved type name in any case. `ReflectionNamedType::isBuiltin()` is true for
/// `Array`, so `getParameterClass()` returns null and the metadata names no class.
#[test]
fn builtin_type_in_another_case_is_not_a_class() {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return;
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
        "app/code/Acme/On/etc/di.xml",
        "<config><type name=\"Acme\\On\\Model\\Upper\"><plugin name=\"watch\" type=\"Acme\\On\\Plugin\\Watch\"/></type></config>\n",
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
        "<?php\nnamespace Acme\\On\\Plugin;\n\nclass Watch\n{\n    public function beforeRun($subject, $list) {}\n}\n",
    );
    // `Boolean` is not reserved in PHP, so it stays a class of this namespace.
    write(
        root,
        "app/code/Acme/On/Model/Boolean.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Boolean\n{\n}\n",
    );
    let class = |name: &str, types: [&str; 4]| {
        format!(
            "<?php\nnamespace Acme\\On\\Model;\n\nclass {name}\n{{\n    public function __construct({} $list, {} $thing, {} $text, {} $maybe = null, Boolean $flag = null)\n    {{\n    }}\n\n    public function run({} $list): {}\n    {{\n        return $list;\n    }}\n}}\n",
            types[0], types[1], types[2], types[3], types[0], types[0],
        )
    };
    write(
        root,
        "app/code/Acme/On/Model/Lower.php",
        &class("Lower", ["array", "object", "string", "?array"]),
    );
    write(
        root,
        "app/code/Acme/On/Model/Upper.php",
        &class("Upper", ["Array", "OBJECT", "String", "?Array"]),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .output()
        .unwrap();
    assert_compiles(&output);
    let metadata = fs::read_to_string(root.join("generated/metadata/global.php")).unwrap();
    let arguments_of = |class: &str| {
        let after = metadata
            .split(&format!("'Acme\\\\On\\\\Model\\\\{class}' =>"))
            .nth(1)
            .expect("the class has arguments")
            .to_string();
        after.split("'Acme\\\\On").next().unwrap().to_string()
    };
    assert_eq!(
        arguments_of("Upper\\\\Interceptor"),
        arguments_of("Lower"),
        "{metadata}"
    );
    assert!(!metadata.contains("Model\\\\Array"), "{metadata}");
    assert!(!metadata.contains("Model\\\\OBJECT"), "{metadata}");
    let interceptor =
        fs::read_to_string(root.join("generated/code/Acme/On/Model/Upper/Interceptor.php"))
            .unwrap();
    assert!(
        interceptor.contains("public function run(array $list) : array"),
        "{interceptor}"
    );
}
