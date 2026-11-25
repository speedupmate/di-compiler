//! The compiler skips writing the metadata only when the inputs, the build and the metadata
//! files are all what the last compile saw.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn has_php() -> bool {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return false;
    }
    true
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
        "app/code/Acme/On/Model/Gadget.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Gadget\n{\n}\n",
    );
    tmp
}

/// Whether the compile wrote the metadata.
fn compile_with(binary: &Path, root: &Path) -> bool {
    let output = Command::new(binary)
        .arg("--magento-root")
        .arg(root)
        .env("RUST_LOG", "info")
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{log}");
    !log.contains("Phase 7 skipped")
}

fn compile(root: &Path) -> bool {
    compile_with(Path::new(env!("CARGO_BIN_EXE_fast-di-compile")), root)
}

#[test]
fn an_unchanged_store_skips_the_metadata() {
    if !has_php() {
        return;
    }
    let store = store();
    assert!(compile(store.path()));
    assert!(!compile(store.path()));
}

#[test]
fn metadata_rewritten_by_another_compiler_is_written_again() {
    if !has_php() {
        return;
    }
    let store = store();
    assert!(compile(store.path()));
    let global = store.path().join("generated/metadata/global.php");
    let ours = fs::read_to_string(&global).unwrap();
    fs::write(&global, "<?php\nreturn [];\n").unwrap();
    assert!(compile(store.path()));
    assert_eq!(fs::read_to_string(&global).unwrap(), ours);
}

/// A rebuilt compiler, here the same executable with one more byte, writes its own metadata.
#[cfg(target_os = "linux")]
#[test]
fn another_build_of_the_compiler_writes_the_metadata_again() {
    use std::os::unix::fs::PermissionsExt;
    if !has_php() {
        return;
    }
    let store = store();
    assert!(compile(store.path()));
    let bin = tempfile::TempDir::new().unwrap();
    let rebuilt = bin.path().join("fast-di-compile");
    let mut executable = fs::read(env!("CARGO_BIN_EXE_fast-di-compile")).unwrap();
    executable.push(0);
    fs::write(&rebuilt, executable).unwrap();
    fs::set_permissions(&rebuilt, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(compile_with(&rebuilt, store.path()));
    assert!(!compile_with(&rebuilt, store.path()));
}
