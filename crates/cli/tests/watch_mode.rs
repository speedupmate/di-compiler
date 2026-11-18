//! `--watch` compiles again after a change that can change the result, never because of its own
//! output, and stops cleanly on SIGINT.
#![cfg(unix)]

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Writes through a temporary file and a rename, as Composer and many editors save.
fn replace(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    let temporary = path.with_extension("swap-in");
    fs::write(&temporary, content).unwrap();
    fs::rename(&temporary, &path).unwrap();
}

fn has_php() -> bool {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return false;
    }
    true
}

const MODEL: &str = "<?php\nnamespace Acme\\On\\Model;\n\nclass {NAME}\n{\n    public function run()\n    {\n    }\n}\n";

fn plugin_on(classes: &[&str]) -> String {
    let types: String = classes
        .iter()
        .map(|class| {
            format!("<type name=\"Acme\\On\\Model\\{class}\"><plugin name=\"p\" type=\"Acme\\On\\Plugin\\Watch\"/></type>")
        })
        .collect();
    format!("<config>{types}</config>\n")
}

struct Store {
    dir: tempfile::TempDir,
    temp: tempfile::TempDir,
}

impl Store {
    fn new() -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let root = dir.path();
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
             foreach (['/app/code/', '/vendor/acme/extra/src/'] as $base) {\n        \
             $file = dirname(__DIR__) . $base . str_replace('\\\\', '/', $class) . '.php';\n        \
             if (is_file($file)) {\n            require $file;\n            return;\n        }\n    }\n});\n",
        );
        write(
            root,
            "vendor/composer/installed.json",
            "{\"packages\": []}\n",
        );
        for class in ["Gadget", "Widget"] {
            write(
                root,
                &format!("app/code/Acme/On/Model/{class}.php"),
                &MODEL.replace("{NAME}", class),
            );
        }
        write(
            root,
            "app/code/Acme/On/Plugin/Watch.php",
            "<?php\nnamespace Acme\\On\\Plugin;\n\nclass Watch\n{\n    public function beforeRun($subject)\n    {\n    }\n}\n",
        );
        write(root, "app/code/Acme/On/etc/di.xml", "<config/>\n");
        Store { dir, temp }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn watch(&self, extra: &[&str]) -> Watcher {
        self.watch_with_env(extra, &[])
    }

    fn watch_with_env(&self, extra: &[&str], env: &[(&str, &str)]) -> Watcher {
        let mut command = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"));
        command
            .arg("--magento-root")
            .arg(self.root())
            .args(extra)
            .env("TMPDIR", self.temp.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        for (key, value) in env {
            command.env(key, value);
        }
        let mut child = command.spawn().unwrap();
        let (sender, lines) = mpsc::channel();
        let stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut watcher = Watcher {
            child,
            lines,
            seen: Vec::new(),
        };
        watcher.next_status(Duration::from_secs(30));
        watcher
    }

    fn interceptor(&self, class: &str) -> PathBuf {
        self.root().join(format!(
            "generated/code/Acme/On/Model/{class}/Interceptor.php"
        ))
    }

    fn metadata(&self) -> PathBuf {
        self.root().join("generated/metadata/global.php")
    }

    fn leftovers(&self) -> Vec<String> {
        fs::read_dir(self.temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("fast-di-"))
            .collect()
    }
}

struct Watcher {
    child: Child,
    lines: mpsc::Receiver<String>,
    seen: Vec<String>,
}

impl Watcher {
    /// The next status line (`✓` or `✗`), waiting up to `timeout`.
    fn next_status(&mut self, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    self.seen.push(line.clone());
                    if line.contains(" ✓ ") || line.contains(" ✗ ") {
                        return line;
                    }
                }
                Err(_) => panic!(
                    "no status line within {timeout:?}; output so far:\n{}",
                    self.seen.join("\n")
                ),
            }
        }
    }

    /// True when no status line arrives within `quiet`.
    fn stays_quiet(&mut self, quiet: Duration) -> bool {
        let deadline = Instant::now() + quiet;
        while let Ok(line) = self
            .lines
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            self.seen.push(line.clone());
            if line.contains(" ✓ ") || line.contains(" ✗ ") {
                return false;
            }
        }
        true
    }

    fn signal(&self, signal: libc::c_int) {
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, signal);
        }
    }

    fn wait(mut self, timeout: Duration) -> (Option<i32>, Vec<String>) {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                while let Ok(line) = self.lines.recv_timeout(Duration::from_millis(200)) {
                    self.seen.push(line);
                }
                return (status.code(), std::mem::take(&mut self.seen));
            }
            assert!(
                Instant::now() < deadline,
                "the watcher did not exit:\n{}",
                self.seen.join("\n")
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const WAIT: Duration = Duration::from_secs(20);

#[test]
fn a_changed_di_xml_compiles_again() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let mut watcher = store.watch(&["--watch"]);
    assert!(!store.interceptor("Gadget").exists());
    write(
        store.root(),
        "app/code/Acme/On/etc/di.xml",
        &plugin_on(&["Gadget"]),
    );
    let status = watcher.next_status(WAIT);
    assert!(status.contains("✓ compiled"), "{status}");
    assert!(status.contains("app/code/Acme/On/etc/di.xml"), "{status}");
    assert!(store.interceptor("Gadget").exists());
}

#[test]
fn a_new_area_di_xml_compiles_again() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let mut watcher = store.watch(&["--watch"]);
    write(
        store.root(),
        "app/code/Acme/On/etc/frontend/di.xml",
        &plugin_on(&["Widget"]),
    );
    assert!(watcher.next_status(WAIT).contains("✓ compiled"));
    assert!(store.interceptor("Widget").exists());
}

#[test]
fn a_di_xml_replaced_by_rename_compiles_again() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let mut watcher = store.watch(&["--watch"]);
    replace(
        store.root(),
        "app/code/Acme/On/etc/di.xml",
        &plugin_on(&["Gadget"]),
    );
    assert!(watcher.next_status(WAIT).contains("✓ compiled"));
    assert!(store.interceptor("Gadget").exists());
    replace(
        store.root(),
        "app/code/Acme/On/etc/di.xml",
        &plugin_on(&["Gadget", "Widget"]),
    );
    assert!(watcher.next_status(WAIT).contains("✓ compiled"));
    assert!(store.interceptor("Widget").exists());
}

#[test]
fn a_composer_change_watches_the_new_module() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let mut watcher = store.watch(&["--watch"]);
    let root = store.root();
    write(root, "vendor/acme/extra/registration.php", "<?php\n");
    write(
        root,
        "vendor/acme/extra/etc/module.xml",
        "<config><module name=\"Acme_Extra\"/></config>\n",
    );
    write(root, "vendor/acme/extra/etc/di.xml", "<config/>\n");
    write(
        root,
        "vendor/acme/extra/src/Acme/Extra/Model/Tool.php",
        "<?php\nnamespace Acme\\Extra\\Model;\n\nclass Tool\n{\n    public function run()\n    {\n    }\n}\n",
    );
    replace(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n        'Acme_Extra' => 1,\n    ],\n];\n",
    );
    replace(
        root,
        "vendor/composer/autoload_files.php",
        "<?php\nreturn array(\n    'x' => $vendorDir . '/acme/extra/registration.php',\n);\n",
    );
    replace(
        root,
        "vendor/composer/installed.json",
        "{\"packages\": [{\"name\": \"acme/extra\"}]}\n",
    );
    // The three writes can land in more than one compile when the machine is busy.
    let mut status = watcher.next_status(WAIT);
    while !status.contains("✓ compiled") {
        status = watcher.next_status(WAIT);
    }
    // Only watched after the watch set was resolved again.
    write(
        root,
        "vendor/acme/extra/etc/di.xml",
        "<config><type name=\"Acme\\Extra\\Model\\Tool\"><plugin name=\"p\" type=\"Acme\\On\\Plugin\\Watch\"/></type></config>\n",
    );
    let status = watcher.next_status(WAIT);
    assert!(status.contains("vendor/acme/extra/etc/di.xml"), "{status}");
    assert!(root
        .join("generated/code/Acme/Extra/Model/Tool/Interceptor.php")
        .exists());
}

#[test]
fn a_changed_constructor_changes_the_arguments() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let mut watcher = store.watch(&["--watch"]);
    write(
        store.root(),
        "app/code/Acme/On/Model/Widget.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Widget\n{\n    public function __construct(Gadget $gadget, int $size = 3)\n    {\n    }\n}\n",
    );
    assert!(watcher.next_status(WAIT).contains("✓ compiled"));
    let metadata = fs::read_to_string(store.metadata()).unwrap();
    assert!(metadata.contains("'size'"), "{metadata}");
}

#[test]
fn a_failed_compile_removes_metadata_without_compiling_again() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let mut watcher = store.watch(&["--watch"]);
    write(
        store.root(),
        "app/code/Acme/On/Model/Gadget.php",
        &MODEL.replace("class {NAME}", "final class Gadget"),
    );
    write(
        store.root(),
        "app/code/Acme/On/etc/di.xml",
        &plugin_on(&["Gadget"]),
    );
    // The two writes can land in two compiles when the machine is busy; the second one fails.
    let mut status = watcher.next_status(WAIT);
    for _ in 0..2 {
        if status.contains("✗ failed") {
            break;
        }
        status = watcher.next_status(WAIT);
    }
    assert!(status.contains("✗ failed"), "{status}");
    assert!(!store.metadata().exists());
    // The metadata removal of the failed compile itself triggers nothing.
    assert!(
        watcher.stays_quiet(Duration::from_secs(4)),
        "{}",
        watcher.seen.join("\n")
    );
    let output = watcher.seen.join("\n");
    assert!(
        output.contains("cannot extend final class Acme\\On\\Model\\Gadget"),
        "{output}"
    );
    assert!(
        output.contains("store runs without compiled metadata"),
        "{output}"
    );

    write(
        store.root(),
        "app/code/Acme/On/Model/Gadget.php",
        &MODEL.replace("{NAME}", "Gadget"),
    );
    assert!(watcher.next_status(WAIT).contains("✓ compiled"));
    assert!(store.metadata().exists());
}

#[test]
fn metadata_removed_from_outside_compiles_again() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let mut watcher = store.watch(&["--watch"]);
    fs::remove_dir_all(store.root().join("generated/metadata")).unwrap();
    let status = watcher.next_status(WAIT);
    assert!(status.contains("generated/metadata removed"), "{status}");
    assert!(store.metadata().exists());
}

#[test]
fn sigint_while_idle_exits_and_keeps_the_metadata() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let watcher = store.watch(&["--watch"]);
    watcher.signal(libc::SIGINT);
    let (code, _) = watcher.wait(Duration::from_secs(5));
    assert_eq!(code, Some(130));
    assert!(store.metadata().exists());
}

fn slow_class(store: &Store) {
    write(
        store.root(),
        "app/code/Acme/On/Model/Slow.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nsleep(30);\n\nclass Slow\n{\n}\n",
    );
}

fn wait_for_worker(store: &Store) {
    let deadline = Instant::now() + WAIT;
    while !store
        .leftovers()
        .iter()
        .any(|name| name.starts_with("fast-di-worker-"))
    {
        assert!(Instant::now() < deadline, "no compile started");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(500));
}

#[test]
fn sigint_during_a_compile_stops_it_and_cleans_up() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let watcher = store.watch(&["--watch"]);
    slow_class(&store);
    wait_for_worker(&store);
    let started = Instant::now();
    watcher.signal(libc::SIGINT);
    let (code, output) = watcher.wait(Duration::from_secs(10));
    assert_eq!(code, Some(130), "{}", output.join("\n"));
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
    assert!(store.leftovers().is_empty(), "{:?}", store.leftovers());
    assert!(!store.metadata().exists());
}

#[test]
fn a_compile_that_ignores_sigterm_is_killed_and_cleaned_up() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let watcher = store.watch_with_env(
        &["--watch"],
        &[("FAST_DI_COMPILE_TEST_IGNORE_SIGTERM", "1")],
    );
    slow_class(&store);
    wait_for_worker(&store);
    let started = Instant::now();
    watcher.signal(libc::SIGINT);
    let (code, output) = watcher.wait(Duration::from_secs(15));
    assert_eq!(code, Some(130), "{}", output.join("\n"));
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_secs(4) && waited < Duration::from_secs(9),
        "{waited:?}"
    );
    assert!(
        output.join("\n").contains("was killed"),
        "{}",
        output.join("\n")
    );
    assert!(store.leftovers().is_empty(), "{:?}", store.leftovers());
    assert!(!store.metadata().exists());
}

#[test]
fn polling_finds_changed_and_new_files() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let mut watcher = store.watch(&["--watch-poll=200"]);
    write(
        store.root(),
        "app/code/Acme/On/etc/di.xml",
        &plugin_on(&["Gadget"]),
    );
    assert!(watcher.next_status(WAIT).contains("✓ compiled"));
    assert!(store.interceptor("Gadget").exists());
    write(
        store.root(),
        "app/code/Acme/On/etc/frontend/di.xml",
        &plugin_on(&["Widget"]),
    );
    assert!(watcher.next_status(WAIT).contains("✓ compiled"));
    assert!(store.interceptor("Widget").exists());
}

#[test]
fn watch_refuses_one_off_modes() {
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .args(["--watch", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--watch cannot be combined"));
}
