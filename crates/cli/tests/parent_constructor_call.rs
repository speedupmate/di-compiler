//! `Compiler\Decorator\Interceptions::getList()` runs `Validator\ConstructorIntegrity` on every
//! scanned class outside `setup/`, and the compile stops on what it reports. The fixture has its
//! own small validator, because the real one lives in Magento: these tests prove that the
//! compiler calls it, on the right classes. Only the real one decides what counts.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

const VALIDATOR: &str = "<?php\nnamespace Magento\\Framework\\Code\\Validator;\n\nclass ConstructorIntegrity\n{\n    \
     public function validate($class)\n    {\n        \
     if (str_ends_with($class, 'Bad')) {\n            \
     throw new \\RuntimeException('Missed required argument second in parent::__construct call.');\n        \
     }\n        return true;\n    }\n}\n";

fn compile_with(files: &[(&str, &str)], flags: &[&str]) -> Option<Output> {
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
        "vendor/autoload.php",
        "<?php\nspl_autoload_register(function ($class) {\n    \
         foreach (['/app/code/', '/setup/src/'] as $base) {\n        \
         $file = dirname(__DIR__) . $base . str_replace('\\\\', '/', $class) . '.php';\n        \
         if (is_file($file)) {\n            require $file;\n            return;\n        }\n    }\n});\n",
    );
    write(
        root,
        "app/code/Magento/Framework/Code/Validator/ConstructorIntegrity.php",
        VALIDATOR,
    );
    for (relative, content) in files {
        write(root, relative, content);
    }
    Some(
        Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
            .arg("--magento-root")
            .arg(root)
            .args(flags)
            .output()
            .unwrap(),
    )
}

const BAD: &str = "app/code/Acme/On/Model/Bad.php";
const BAD_SOURCE: &str = "<?php\nnamespace Acme\\On\\Model;\n\nclass Bad\n{\n}\n";

#[test]
fn what_the_validator_reports_stops_the_compile() {
    let Some(output) = compile_with(&[(BAD, BAD_SOURCE)], &[]) else {
        return;
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("Acme\\On\\Model\\Bad"), "{stderr}");
    assert!(
        stderr.contains("Missed required argument second"),
        "{stderr}"
    );
}

#[test]
fn the_flag_ignores_what_the_validator_reports() {
    let Some(output) = compile_with(&[(BAD, BAD_SOURCE)], &["--ignore-constructor-integrity"])
    else {
        return;
    };
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn setup_classes_are_not_validated() {
    let Some(output) = compile_with(
        &[(
            "setup/src/Acme/Setup/Bad.php",
            "<?php\nnamespace Acme\\Setup;\n\nclass Bad\n{\n}\n",
        )],
        &[],
    ) else {
        return;
    };
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
