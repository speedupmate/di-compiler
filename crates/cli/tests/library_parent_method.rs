//! `InterceptionConfigurationBuilder::getInterceptedMethods()` compares the plugin methods with
//! `get_class_methods()` of the class, so a method of a parent in a library counts. Without an
//! override in the Interceptor, the plugin on that method never runs.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn plugin_on_a_method_of_a_library_parent_gets_an_override() {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: this test needs php on the PATH");
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
        "vendor/autoload.php",
        "<?php\nspl_autoload_register(function ($class) {\n    foreach (['app/code', 'lib'] as $dir) {\n        $file = dirname(__DIR__) . '/' . $dir . '/' . str_replace('\\\\', '/', $class) . '.php';\n        if (is_file($file)) {\n            require $file;\n        }\n    }\n});\n",
    );
    write(
        root,
        "lib/Library/Base.php",
        "<?php\nnamespace Library;\n\nclass Base\n{\n    public function send()\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/Middle.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Middle extends \\Library\\Base\n{\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/Child.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Child extends Middle\n{\n    public function own()\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Plugin/Watch.php",
        "<?php\nnamespace Acme\\On\\Plugin;\n\nclass Watch\n{\n    public function aroundSend($subject, $next)\n    {\n        return $next();\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/etc/di.xml",
        "<config>\n    <type name=\"Acme\\On\\Model\\Middle\">\n        <plugin name=\"watch\" type=\"Acme\\On\\Plugin\\Watch\"/>\n    </type>\n</config>\n",
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
    let child = fs::read_to_string(root.join("generated/code/Acme/On/Model/Child/Interceptor.php"))
        .unwrap();
    assert!(child.contains("public function send()"), "{child}");
    assert!(!child.contains("public function own()"), "{child}");
}
