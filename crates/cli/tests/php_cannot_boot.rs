//! `bin/magento` loads `app/bootstrap.php` first, so a store whose autoloader fails stops there.
//! The compiler stops at once too, with PHP's own message, and does not start PHP again and again.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn compile_with(files: &[(&str, &str)]) -> Option<Output> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return None;
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
        "app/code/Acme/On/Model/Thing.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Thing\n{\n}\n",
    );
    for (relative, content) in files {
        write(root, relative, content);
    }
    Some(
        Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
            .arg("--magento-root")
            .arg(root)
            .output()
            .unwrap(),
    )
}

fn assert_stops(output: &Output, needles: &[&str]) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    for needle in needles {
        assert!(stderr.contains(needle), "missing {needle} in {stderr}");
    }
}

#[test]
fn autoloader_with_a_fatal_error_stops_with_the_message_of_php() {
    let Some(output) = compile_with(&[(
        "vendor/autoload.php",
        "<?php\nrequire __DIR__ . '/gone/autoload_real.php';\n",
    )]) else {
        return;
    };
    assert_stops(
        &output,
        &["PHP cannot start this store", "autoload_real.php"],
    );
}

#[test]
fn autoloader_that_throws_stops_with_the_message_of_php() {
    let Some(output) = compile_with(&[(
        "vendor/autoload.php",
        "<?php\nthrow new \\RuntimeException('Boom from the autoloader');\n",
    )]) else {
        return;
    };
    assert_stops(
        &output,
        &["PHP cannot start this store", "Boom from the autoloader"],
    );
}

#[test]
fn bootstrap_that_exits_stops_with_what_it_printed() {
    let Some(output) = compile_with(&[
        ("vendor/autoload.php", "<?php\n"),
        (
            "app/bootstrap.php",
            "<?php\necho 'This store needs another PHP version';\nexit(1);\n",
        ),
    ]) else {
        return;
    };
    assert_stops(
        &output,
        &["PHP cannot start this store", "another PHP version"],
    );
}

#[test]
fn working_autoloader_that_prints_a_notice_compiles() {
    let Some(output) = compile_with(&[(
        "vendor/autoload.php",
        "<?php\ntrigger_error('Only a notice', E_USER_NOTICE);\n",
    )]) else {
        return;
    };
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
