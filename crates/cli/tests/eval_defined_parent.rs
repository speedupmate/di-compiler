//! Some vendors ship a class only as an encoded string that the file passes to `eval()`, so no
//! scan of the source finds it. Magento loads such a class when a scanned child asks for it and
//! reads constructor and methods through reflection. The compiler has to do the same through PHP.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn php(args: &[&str]) -> String {
    let output = Command::new("php").args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

const BASE: &str = "namespace Acme\\On\\Helper; class Base { \
    public function __construct(\\Acme\\On\\Model\\Gadget $gadget, $limit = 5) {} \
    public function send() { return 1; } }";

/// A store with `Base` only inside `eval()`, a `Package` and `Datalayer` without a constructor
/// of their own, and a `Data` with its own constructor and a plugin on `send()`.
fn compile_store(root: &Path) -> Output {
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
    let encoded = php(&["-r", "echo base64_encode($argv[1]);", "--", BASE]);
    write(
        root,
        "app/code/Acme/On/Helper/Base.php",
        &format!("<?php\n$b = 'base' . '64_' . 'decode';\neval($b('{encoded}'));\n"),
    );
    write(
        root,
        "app/code/Acme/On/Helper/Package.php",
        "<?php\nnamespace Acme\\On\\Helper;\n\nclass Package extends Base\n{\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Helper/Datalayer.php",
        "<?php\nnamespace Acme\\On\\Helper;\n\nclass Datalayer extends Package\n{\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Helper/Data.php",
        "<?php\nnamespace Acme\\On\\Helper;\n\nclass Data extends Package\n{\n    \
         public function __construct(\\Acme\\On\\Model\\Gadget $gadget, Datalayer $layer)\n    {\n        \
         parent::__construct($gadget);\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Plugin/Watch.php",
        "<?php\nnamespace Acme\\On\\Plugin;\n\nclass Watch\n{\n    \
         public function aroundSend($subject, $next)\n    {\n        return $next();\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/etc/di.xml",
        "<config>\n    <type name=\"Acme\\On\\Helper\\Data\">\n        \
         <plugin name=\"watch\" type=\"Acme\\On\\Plugin\\Watch\"/>\n    </type>\n</config>\n",
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
    output
}

fn global_arguments(root: &Path, class: &str) -> String {
    let metadata = root.join("generated/metadata/global.php");
    php(&[
        "-r",
        "$m = include $argv[1]; echo json_encode($m['arguments'][$argv[2]] ?? null, JSON_UNESCAPED_SLASHES);",
        "--",
        metadata.to_str().unwrap(),
        class,
    ])
}

fn has_php() -> bool {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return false;
    }
    true
}

#[test]
fn child_without_constructor_takes_the_constructor_of_an_eval_defined_parent() {
    if !has_php() {
        return;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    compile_store(tmp.path());

    let expected = r#"{"gadget":{"_i_":"Acme\\On\\Model\\Gadget"},"limit":{"_v_":5}}"#;
    for class in ["Acme\\On\\Helper\\Package", "Acme\\On\\Helper\\Datalayer"] {
        assert_eq!(global_arguments(tmp.path(), class), expected, "{class}");
    }
}

#[test]
fn plugin_on_a_method_of_an_eval_defined_parent_gets_an_override() {
    if !has_php() {
        return;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    compile_store(tmp.path());

    let interceptor = fs::read_to_string(
        tmp.path()
            .join("generated/code/Acme/On/Helper/Data/Interceptor.php"),
    )
    .unwrap();
    assert!(
        interceptor.contains("public function send()"),
        "{interceptor}"
    );
    assert_eq!(
        global_arguments(tmp.path(), "Acme\\On\\Helper\\Data\\Interceptor"),
        r#"{"gadget":{"_i_":"Acme\\On\\Model\\Gadget"},"layer":{"_i_":"Acme\\On\\Helper\\Datalayer"}}"#
    );
}
