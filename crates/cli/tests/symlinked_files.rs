//! `ClassesScanner::getList()` walks with `FilesystemIterator::FOLLOW_SYMLINKS`, so classes behind
//! a symlink inside a module are compiled like any other.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn store() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n        'Acme_Linked' => 1,\n    ],\n];\n",
    );
    write(root, "app/code/Acme/On/registration.php", "<?php\n");
    write(
        root,
        "app/code/Acme/On/etc/module.xml",
        "<config><module name=\"Acme_On\"/></config>\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/Gadget.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Gadget\n{\n}\n",
    );
    tmp
}

fn compiled_global(root: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .arg("--fallback-php")
        .arg(root.join("no-php-here"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::read_to_string(root.join("generated/metadata/global.php")).unwrap()
}

const WITH_CONSTRUCTOR: &str =
    "\n{\n    public function __construct(\\Acme\\On\\Model\\Gadget $g)\n    {\n    }\n}\n";

#[test]
fn class_in_a_symlinked_folder_is_compiled() {
    let tmp = store();
    let root = tmp.path();
    write(
        root,
        "shared/Thing.php",
        &format!("<?php\nnamespace Acme\\On\\Linked;\n\nclass Thing{WITH_CONSTRUCTOR}"),
    );
    symlink(root.join("shared"), root.join("app/code/Acme/On/Linked")).unwrap();
    assert!(compiled_global(root).contains("'Acme\\\\On\\\\Linked\\\\Thing' =>"));
}

#[test]
fn symlinked_class_file_is_compiled() {
    let tmp = store();
    let root = tmp.path();
    write(
        root,
        "shared/Single.php",
        &format!("<?php\nnamespace Acme\\On\\Model;\n\nclass Single{WITH_CONSTRUCTOR}"),
    );
    symlink(
        root.join("shared/Single.php"),
        root.join("app/code/Acme/On/Model/Single.php"),
    )
    .unwrap();
    assert!(compiled_global(root).contains("'Acme\\\\On\\\\Model\\\\Single' =>"));
}

#[test]
fn symlink_loop_does_not_hang_the_compile() {
    let tmp = store();
    let root = tmp.path();
    symlink(
        root.join("app/code/Acme/On"),
        root.join("app/code/Acme/On/Model/Loop"),
    )
    .unwrap();
    assert!(compiled_global(root).contains("'Acme\\\\On\\\\Model\\\\Gadget' =>"));
}

#[test]
fn symlinked_vendor_package_is_found_with_its_di_xml() {
    let tmp = store();
    let root = tmp.path();
    let package = root.join("packages/linked");
    write(root, "packages/linked/registration.php", "<?php\n");
    write(
        root,
        "packages/linked/etc/module.xml",
        "<config><module name=\"Acme_Linked\"/></config>\n",
    );
    write(
        root,
        "packages/linked/etc/di.xml",
        "<config><type name=\"Acme\\On\\Model\\Gadget\"><plugin name=\"p\" type=\"Acme\\Linked\\Model\\Watch\"/></type></config>\n",
    );
    write(
        root,
        "packages/linked/Model/Watch.php",
        "<?php\nnamespace Acme\\Linked\\Model;\n\nclass Watch\n{\n    public function beforeRun($s)\n    {\n    }\n}\n",
    );
    fs::create_dir_all(root.join("vendor/acme")).unwrap();
    symlink(&package, root.join("vendor/acme/linked")).unwrap();
    compiled_global(root);
    assert!(root
        .join("generated/code/Acme/On/Model/Gadget/Interceptor.php")
        .exists());
}
