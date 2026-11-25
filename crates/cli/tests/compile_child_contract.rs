//! What a caller such as watch mode relies on: `--quiet` keeps the log to warnings and errors,
//! `--summary-json` prints one JSON line with the result, and SIGTERM ends the compile the way a
//! failed compile ends.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

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

fn store(root: &Path, sealed: bool) {
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
         $file = dirname(__DIR__) . '/app/code/' . str_replace('\\\\', '/', $class) . '.php';\n    \
         if (is_file($file)) {\n        require $file;\n    }\n});\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/Gadget.php",
        &format!(
            "<?php\nnamespace Acme\\On\\Model;\n\n{}class Gadget\n{{\n    public function run()\n    {{\n    }}\n}}\n",
            if sealed { "final " } else { "" }
        ),
    );
    write(
        root,
        "app/code/Acme/On/Plugin/Watch.php",
        "<?php\nnamespace Acme\\On\\Plugin;\n\nclass Watch\n{\n    public function beforeRun($subject)\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/etc/di.xml",
        "<config><type name=\"Acme\\On\\Model\\Gadget\"><plugin name=\"p\" type=\"Acme\\On\\Plugin\\Watch\"/></type></config>\n",
    );
}

fn compile(root: &Path, temp: &Path, extra: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"));
    command
        .arg("--magento-root")
        .arg(root)
        .args(extra)
        .env("TMPDIR", temp);
    command
}

fn summary(output: &Output) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().last().unwrap_or_else(|| {
        panic!(
            "no summary line; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}"))
}

fn leftovers(temp: &Path) -> Vec<String> {
    fs::read_dir(temp)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("fast-di-"))
        .collect()
}

#[test]
fn a_passing_compile_prints_a_summary_and_only_warnings() {
    if !has_php() {
        return;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let temp = tempfile::TempDir::new().unwrap();
    store(tmp.path(), false);
    let output = compile(tmp.path(), temp.path(), &["--quiet", "--summary-json"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains(" INFO "), "{stderr}");
    let line = summary(&output);
    assert_eq!(line["ok"], true, "{line}");
    assert!(line["files_written"].as_u64().unwrap() > 0, "{line}");
    assert!(line.get("error").is_none(), "{line}");

    let again = compile(tmp.path(), temp.path(), &["--quiet", "--summary-json"])
        .output()
        .unwrap();
    let line = summary(&again);
    assert_eq!(line["files_written"], 0, "{line}");
    assert!(line["files_unchanged"].as_u64().unwrap() > 0, "{line}");
    assert!(
        leftovers(temp.path()).is_empty(),
        "{:?}",
        leftovers(temp.path())
    );
}

#[test]
fn a_failed_compile_prints_its_error_in_the_summary() {
    if !has_php() {
        return;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let temp = tempfile::TempDir::new().unwrap();
    store(tmp.path(), true);
    let output = compile(tmp.path(), temp.path(), &["--quiet", "--summary-json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let line = summary(&output);
    assert_eq!(line["ok"], false, "{line}");
    assert!(
        line["error"]
            .as_str()
            .unwrap()
            .contains("cannot extend final class Acme\\On\\Model\\Gadget"),
        "{line}"
    );
    assert!(!tmp.path().join("generated/metadata").exists());
    assert!(
        leftovers(temp.path()).is_empty(),
        "{:?}",
        leftovers(temp.path())
    );
}

#[test]
fn without_the_new_flags_stdout_stays_empty() {
    if !has_php() {
        return;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let temp = tempfile::TempDir::new().unwrap();
    store(tmp.path(), false);
    let output = compile(tmp.path(), temp.path(), &[]).output().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("finished successfully"));
}

#[cfg(unix)]
#[test]
fn sigterm_during_a_compile_cleans_up_like_a_failed_compile() {
    if !has_php() {
        return;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let temp = tempfile::TempDir::new().unwrap();
    store(tmp.path(), false);
    // A class file that takes a while to load keeps the PHP class checks busy.
    write(
        tmp.path(),
        "app/code/Acme/On/Model/Slow.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nsleep(20);\n\nclass Slow\n{\n}\n",
    );
    fs::create_dir_all(tmp.path().join("generated/metadata")).unwrap();
    fs::write(
        tmp.path().join("generated/metadata/global.php"),
        "<?php return [];",
    )
    .unwrap();

    let child = compile(tmp.path(), temp.path(), &["--quiet", "--summary-json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id() as libc::pid_t;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !temp
        .path()
        .join(format!("fast-di-worker-{pid}.php"))
        .exists()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the compile never started its PHP worker"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    let started = std::time::Instant::now();
    let output = child.wait_with_output().unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(
        output.status.code(),
        Some(143),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let line = summary(&output);
    assert_eq!(line["ok"], false, "{line}");
    assert_eq!(line["error"], "interrupted", "{line}");
    assert!(!tmp.path().join("generated/metadata").exists());
    assert!(
        leftovers(temp.path()).is_empty(),
        "{:?}",
        leftovers(temp.path())
    );
}
