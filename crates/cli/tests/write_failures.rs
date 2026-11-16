//! `Console\GenerationDirectoryAccess::check()` stops `bin/magento` when a generated folder is not
//! writable, and Magento's writers throw on any failed write. Nothing may hide a write failure.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

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
        "app/code/Acme/On/Model/Keeper.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Keeper\n{\n    public function __construct(GadgetFactory $factory)\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/Gadget.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Gadget\n{\n}\n",
    );
    tmp
}

fn compile(root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .output()
        .unwrap()
}

const NO_ACCESS: &str = "does not have read and write permissions on generated directory";

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn assert_stops(output: &Output, message: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains(message), "{stderr}");
}

#[test]
fn generated_folder_that_is_not_writable_stops_the_compile() {
    let tmp = store();
    let generated = tmp.path().join("generated");
    fs::create_dir(&generated).unwrap();
    set_mode(&generated, 0o555);
    let output = compile(tmp.path());
    set_mode(&generated, 0o755);
    assert_stops(&output, NO_ACCESS);
}

#[test]
fn metadata_folder_that_is_not_writable_stops_the_compile() {
    let tmp = store();
    let metadata = tmp.path().join("generated/metadata");
    fs::create_dir_all(&metadata).unwrap();
    set_mode(&metadata, 0o555);
    let output = compile(tmp.path());
    set_mode(&metadata, 0o755);
    assert_stops(&output, NO_ACCESS);
}

#[test]
fn code_folder_that_is_not_writable_stops_the_compile() {
    let tmp = store();
    let code = tmp.path().join("generated/code");
    fs::create_dir_all(&code).unwrap();
    set_mode(&code, 0o555);
    let output = compile(tmp.path());
    set_mode(&code, 0o755);
    assert_stops(&output, NO_ACCESS);
}

#[test]
fn metadata_file_that_cannot_be_written_stops_the_compile_and_names_it() {
    let tmp = store();
    fs::create_dir_all(tmp.path().join("generated/metadata/global.php")).unwrap();
    assert_stops(&compile(tmp.path()), "generated/metadata/global.php");
}

#[test]
fn code_file_that_cannot_be_written_stops_the_compile_and_names_it() {
    let tmp = store();
    let factory = "generated/code/Acme/On/Model/GadgetFactory.php";
    fs::create_dir_all(tmp.path().join(factory)).unwrap();
    assert_stops(&compile(tmp.path()), factory);
}

#[test]
fn missing_generated_folders_are_created() {
    let tmp = store();
    let output = compile(tmp.path());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(tmp.path().join("generated/metadata/global.php").is_file());
}
