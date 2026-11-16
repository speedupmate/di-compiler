//! `Scanner\PhpScanner` collects every `X\Proxy` that a constructor names, and the generator needs
//! a source class. That source can be a generated class: a factory exists once it is generated.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn proxy_of_a_generated_factory_is_generated() {
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
        "app/code/Acme/On/Model/Thing.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Thing\n{\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/User.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass User\n{\n    public function __construct(ThingFactory\\Proxy $things)\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/Other.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Other\n{\n    public function __construct(ThingFactory $things)\n    {\n    }\n}\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        // No PHP on purpose, so this test only depends on the static scan.
        .arg("--fallback-php")
        .arg(root.join("no-php-here"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root
        .join("generated/code/Acme/On/Model/ThingFactory.php")
        .exists());
    assert!(root
        .join("generated/code/Acme/On/Model/ThingFactory/Proxy.php")
        .exists());
}

#[test]
fn factory_only_named_through_its_proxy_is_generated() {
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
        "app/code/Acme/On/Model/Thing.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Thing\n{\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/User.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass User\n{\n    public function __construct(ThingFactory\\Proxy $things)\n    {\n    }\n}\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        // No PHP on purpose, so this test only depends on the static scan.
        .arg("--fallback-php")
        .arg(root.join("no-php-here"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root
        .join("generated/code/Acme/On/Model/ThingFactory.php")
        .exists());
    assert!(root
        .join("generated/code/Acme/On/Model/ThingFactory/Proxy.php")
        .exists());
}
