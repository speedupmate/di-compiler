//! Magento learns about a package through the `registration.php` that Composer loads, as listed in
//! `vendor/composer/autoload_files.php`. A package can ship a file with that name that Composer
//! does not load. `ComponentRegistrar` never hears about it, so `setup:di:compile` skips the package.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn package(root: &Path, package: &str, class: &str) {
    write(
        root,
        &format!("vendor/acme/{package}/src/registration.php"),
        "<?php\n",
    );
    write(
        root,
        &format!("vendor/acme/{package}/src/{class}.php"),
        &format!("<?php\nnamespace Acme;\n\nclass {class}\n{{\n    public function __construct({class} $self)\n    {{\n    }}\n}}\n"),
    );
}

#[test]
fn only_the_registration_files_composer_loads_count() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [],\n];\n",
    );
    package(root, "loaded", "Loaded");
    package(root, "ignored", "Ignored");
    write(
        root,
        "vendor/composer/autoload_files.php",
        "<?php\n$vendorDir = dirname(__DIR__);\nreturn array(\n    'a' => $vendorDir . '/acme/loaded/src/registration.php',\n    'b' => $vendorDir . '/acme/ignored/src/polyfills.php',\n);\n",
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
    assert!(metadata.contains("'Acme\\\\Loaded' =>"), "{metadata}");
    assert!(!metadata.contains("'Acme\\\\Ignored' =>"), "{metadata}");
}
