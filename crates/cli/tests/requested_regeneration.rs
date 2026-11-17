//! `composer install` leaves `var/.regenerate`, and Magento's boot before `setup:di:compile`
//! deletes it with `generated/code`, `generated/metadata` and `var/cache`. The compiler does the
//! same before it compiles. Otherwise the first PHP process that boots the store after the compile
//! finds the flag and deletes the fresh output, while Composer's class map still lists its files.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// A store with one module, which compiles.
fn store() -> Option<tempfile::TempDir> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return None;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn ['modules' => ['Acme_On' => 1]];\n",
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
    Some(tmp)
}

fn compile(root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .output()
        .unwrap()
}

fn assert_compiled(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_flag_is_removed_with_the_folders_magento_cleans() {
    let Some(tmp) = store() else {
        return;
    };
    let root = tmp.path();
    write(root, "var/.regenerate", "");
    write(root, "var/cache/mage--0/stale", "cached");
    write(root, "generated/code/Acme/Gone/Proxy.php", "<?php\n");
    write(root, "var/log/system.log", "kept");

    assert_compiled(&compile(root));

    assert!(!root.join("var/.regenerate").exists());
    assert!(!root.join("var/cache").exists());
    assert!(!root.join("generated/code/Acme/Gone/Proxy.php").exists());
    assert!(root.join("generated/metadata/global.php").is_file());
    assert!(root.join("var/log/system.log").is_file());
}

#[test]
fn without_the_flag_var_cache_stays() {
    let Some(tmp) = store() else {
        return;
    };
    let root = tmp.path();
    write(root, "var/cache/mage--0/kept", "cached");

    assert_compiled(&compile(root));

    assert!(root.join("var/cache/mage--0/kept").is_file());
    assert!(root.join("generated/metadata/global.php").is_file());
}
