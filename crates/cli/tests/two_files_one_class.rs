//! `ClassesScanner::includeClass()` asks the autoloader for a class, so of two files that declare
//! one class, the file the autoloader finds gives the constructor. The compiler must not depend on
//! the order in which it reads files, and must not use the old copy.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn the_file_named_after_the_class_gives_the_constructor() {
    for old_copy in ["Thing_old.php", "AThing.php", "Zthing_2.4.2.php"] {
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
            "<?php\nnamespace Acme\\On\\Model;\n\nclass Thing\n{\n    public function __construct(Thing $real)\n    {\n    }\n}\n",
        );
        write(
            root,
            &format!("app/code/Acme/On/Model/{old_copy}"),
            "<?php\nnamespace Acme\\On\\Model;\n\nclass Thing\n{\n    public function __construct(Thing $old)\n    {\n    }\n}\n",
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
        let metadata = fs::read_to_string(root.join("generated/metadata/global.php")).unwrap();
        assert!(metadata.contains("'real' =>"), "{old_copy}: {metadata}");
        assert!(!metadata.contains("'old' =>"), "{old_copy}: {metadata}");
    }
}
