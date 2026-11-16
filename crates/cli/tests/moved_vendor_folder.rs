//! PHP loads a class through `autoload_psr4.php`, not through `install-path` in `installed.json`.
//! A vendor folder that was moved keeps working, so the compiler must still find its classes.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn install_path_that_leads_nowhere_still_finds_the_package() {
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
        "app/code/Acme/On/Model/User.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass User\n{\n    public function __construct(\\Library\\ThingFactory $things)\n    {\n    }\n}\n",
    );
    write(
        root,
        "vendor/acme/lib/src/ThingFactory.php",
        "<?php\nnamespace Library;\n\nclass ThingFactory\n{\n}\n",
    );
    write(
        root,
        "vendor/composer/installed.json",
        "{\"packages\": [{\"name\": \"acme/lib\", \"install-path\": \"../../../gone/vendor/acme/lib\", \"autoload\": {\"psr-4\": {\"Library\\\\\": \"src/\"}}}]}\n",
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
    // The library has this class, so it must not be generated.
    assert!(!root
        .join("generated/code/Library/ThingFactory.php")
        .exists());
}
