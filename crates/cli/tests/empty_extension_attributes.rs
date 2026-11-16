//! `ServiceDataAttributesScanner::collectEntities()` reads the `for` of every `extension_attributes`
//! node, also one without attributes. A class that asks for such an Extension at runtime would
//! otherwise depend on writing to `generated/`.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn extension_attributes_node_without_attributes_generates_the_extension() {
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
        "app/code/Acme/On/etc/extension_attributes.xml",
        "<config>\n    <extension_attributes for=\"Acme\\On\\Api\\Data\\ThingInterface\"/>\n</config>\n",
    );
    write(
        root,
        "app/code/Acme/On/Api/Data/ThingInterface.php",
        "<?php\nnamespace Acme\\On\\Api\\Data;\n\ninterface ThingInterface\n{\n}\n",
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
    let data = root.join("generated/code/Acme/On/Api/Data");
    assert!(data.join("ThingExtension.php").exists());
    assert!(data.join("ThingExtensionInterface.php").exists());
    assert!(!data.join("ThingExtensionFactory.php").exists());
}
