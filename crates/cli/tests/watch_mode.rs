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

    /// True when a line containing `text` was printed, waiting up to `timeout` for it.
    fn wait_for(&mut self, text: &str, timeout: Duration) -> bool {
        if self.seen.iter().any(|line| line.contains(text)) {
            return true;
        }
        let deadline = Instant::now() + timeout;
        while let Ok(line) = self
            .lines
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            self.seen.push(line.clone());
            if line.contains(text) {
                return true;
            }
        }
        false
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

#[test]
fn a_module_copied_in_is_watched_down_to_its_deepest_folder() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let mut watcher = store.watch(&["--watch"]);
    // Created in one go, as a copy or a checkout does.
    let module = store.root().join("app/code/Acme/Two");
    fs::create_dir_all(module.join("etc/frontend")).unwrap();
    fs::write(module.join("registration.php"), "<?php\n").unwrap();
    fs::write(
        module.join("etc/module.xml"),
        "<config><module name=\"Acme_Two\"/></config>\n",
    )
    .unwrap();
    fs::write(module.join("etc/frontend/di.xml"), "<config/>\n").unwrap();
    let mut status = watcher.next_status(WAIT);
    while !status.contains("app/code/Acme/Two") {
        status = watcher.next_status(WAIT);
    }
    std::thread::sleep(Duration::from_millis(200));
    fs::write(
        module.join("etc/frontend/di.xml"),
        "<config><!-- changed --></config>\n",
    )
    .unwrap();
    let status = watcher.next_status(WAIT);
    assert!(
        status.contains("app/code/Acme/Two/etc/frontend/di.xml"),
        "{status}"
    );
}

#[test]
fn two_signals_close_together_stop_the_compile_gracefully() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let watcher = store.watch(&["--watch"]);
    slow_class(&store);
    wait_for_worker(&store);
    // Ctrl+C reaches the watcher from the terminal and once more through bin/magento.
    watcher.signal(libc::SIGINT);
    std::thread::sleep(Duration::from_millis(100));
    watcher.signal(libc::SIGINT);
    let (code, output) = watcher.wait(Duration::from_secs(10));
    assert_eq!(code, Some(130), "{}", output.join("\n"));
    assert!(
        !output.join("\n").contains("was killed"),
        "{}",
        output.join("\n")
    );
    assert!(store.leftovers().is_empty(), "{:?}", store.leftovers());
}

#[test]
fn the_watcher_stops_when_its_parent_ends() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let log = store.temp.path().join("watch.log");
    // A parent that starts the watcher, waits for its first compile, then ends without passing
    // on any signal, as bin/magento does on PHP without pcntl.
    let script = format!(
        "{bin} --magento-root {root} --watch --watch-exit-with-parent > {log} 2>&1 & echo $!; \
         for i in $(seq 1 100); do grep -q compiled {log} && exit 0; sleep 0.1; done",
        bin = env!("CARGO_BIN_EXE_fast-di-compile"),
        root = store.root().display(),
        log = log.display(),
    );
    let output = Command::new("sh")
        .arg("-c")
        .arg(script)
        .env("TMPDIR", store.temp.path())
        .output()
        .unwrap();
    let pid: libc::pid_t = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(
            Instant::now() < deadline,
            "the watcher kept running:\n{}",
            fs::read_to_string(&log).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(store.metadata().exists());
    assert!(
        store.leftovers().iter().all(|name| name == "watch.log"),
        "{:?}",
        store.leftovers()
    );
}

#[test]
fn a_di_xml_in_an_app_etc_folder_compiles_again() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let mut watcher = store.watch(&["--watch"]);
    write(
        store.root(),
        "app/etc/local/di.xml",
        &plugin_on(&["Gadget"]),
    );
    let mut status = watcher.next_status(WAIT);
    while !status.contains("app/etc/local") {
        status = watcher.next_status(WAIT);
    }
    assert!(status.contains("✓ compiled"), "{status}");
    assert!(store.interceptor("Gadget").exists());
    write(
        store.root(),
        "app/etc/local/di.xml",
        &plugin_on(&["Gadget", "Widget"]),
    );
    assert!(watcher.next_status(WAIT).contains("app/etc/local/di.xml"));
    assert!(store.interceptor("Widget").exists());
}

#[test]
fn a_compile_that_stops_early_still_removes_the_metadata() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    let mut watcher = store.watch(&["--watch"]);
    assert!(store.metadata().exists());
    // An incompatible module stops the compile before it touches its output.
    replace(
        store.root(),
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n        'Creatuity_Interception' => 1,\n    ],\n];\n",
    );
    let status = watcher.next_status(WAIT);
    assert!(status.contains("✗ failed"), "{status}");
    assert!(!store.metadata().exists());
    assert!(watcher.wait_for("store runs without compiled metadata", WAIT));

    replace(
        store.root(),
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n    ],\n];\n",
    );
    assert!(watcher.next_status(WAIT).contains("✓ compiled"));
    assert!(store.metadata().exists());
}

/// A `bin/magento` that logs each run to `var/clean.log` and behaves as `bin/clean-mode` says.
fn fake_magento(store: &Store, mode: &str) {
    write(
        store.root(),
        "bin/magento",
        "<?php\n$root = dirname(__DIR__);\n\
         file_put_contents($root . '/var/clean.log', implode(' ', array_slice($argv, 1)) . \"\\n\", FILE_APPEND);\n\
         $mode = @file_get_contents(__DIR__ . '/clean-mode');\n\
         if ($mode === 'fail') {\n    fwrite(STDERR, \"cache backend down\\n\");\n    exit(1);\n}\n\
         if ($mode === 'slow') {\n    sleep(30);\n}\n\
         if ($mode === 'linger') {\n    exec((is_executable('/usr/bin/setsid') ? 'setsid ' : '') . 'sleep 10 > /dev/null &');\n}\nexit(0);\n",
    );
    write(store.root(), "bin/clean-mode", mode);
    fs::create_dir_all(store.root().join("var")).unwrap();
}

fn touch_module_list(store: &Store) {
    replace(
        store.root(),
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n    ],\n];\n// touched\n",
    );
}

#[test]
fn a_failed_cache_clean_is_tried_again_without_a_new_change() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    fake_magento(&store, "fail");
    let mut watcher = store.watch(&["--watch"]);
    touch_module_list(&store);
    assert!(watcher.next_status(WAIT).contains("✓ compiled"));
    assert!(
        watcher.wait_for("will be tried again in 2s", WAIT),
        "{}",
        watcher.seen.join("\n")
    );
    assert!(watcher.seen.join("\n").contains("cache backend down"));

    write(store.root(), "bin/clean-mode", "ok");
    assert!(
        watcher.wait_for("(module list changed)", Duration::from_secs(10)),
        "{}",
        watcher.seen.join("\n")
    );
    let runs = fs::read_to_string(store.root().join("var/clean.log")).unwrap();
    assert!(runs.lines().count() >= 2, "{runs}");
    assert!(
        runs.lines()
            .all(|line| line == "cache:clean config compiled_config --quiet"),
        "{runs}"
    );
}

#[test]
fn sigint_during_a_cache_clean_stops_at_once() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    fake_magento(&store, "slow");
    let mut watcher = store.watch(&["--watch"]);
    touch_module_list(&store);
    assert!(watcher.next_status(WAIT).contains("✓ compiled"));
    let deadline = Instant::now() + WAIT;
    while !store.root().join("var/clean.log").exists() {
        assert!(Instant::now() < deadline, "the cache clean never started");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    let started = Instant::now();
    watcher.signal(libc::SIGINT);
    let (code, output) = watcher.wait(Duration::from_secs(10));
    assert_eq!(code, Some(130), "{}", output.join("\n"));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_process_left_by_the_cache_clean_does_not_hold_up_the_watcher() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    // The process keeps the stderr of the cache clean open after `bin/magento` ends.
    fake_magento(&store, "linger");
    let mut watcher = store.watch(&["--watch"]);
    touch_module_list(&store);
    assert!(watcher.next_status(WAIT).contains("✓ compiled"));
    assert!(
        watcher.wait_for("(module list changed)", Duration::from_secs(5)),
        "{}",
        watcher.seen.join("\n")
    );
    let started = Instant::now();
    watcher.signal(libc::SIGTERM);
    let (code, output) = watcher.wait(Duration::from_secs(15));
    assert_eq!(code, Some(143), "{}", output.join("\n"));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn metadata_that_cannot_be_removed_is_reported_as_still_in_use() {
    if !has_php() {
        return;
    }
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipped: root can remove files from a read-only folder");
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let store = Store::new();
    let mut watcher = store.watch(&["--watch"]);
    let metadata = store.root().join("generated/metadata");
    fs::set_permissions(&metadata, fs::Permissions::from_mode(0o555)).unwrap();
    replace(
        store.root(),
        "app/code/Acme/On/etc/di.xml",
        &plugin_on(&["Widget"]),
    );
    let status = watcher.next_status(WAIT);
    let reported = watcher.wait_for("still runs on the metadata of an earlier compile", WAIT);
    fs::set_permissions(&metadata, fs::Permissions::from_mode(0o755)).unwrap();
    let output = watcher.seen.join("\n");
    assert!(status.contains("✗ failed"), "{output}");
    assert!(store.metadata().exists(), "{output}");
    assert!(reported, "{output}");
    assert!(
        !output.contains("store runs without compiled metadata"),
        "{output}"
    );
}

#[cfg(target_os = "linux")]
fn threads_and_descriptors(pid: u32) -> (usize, usize) {
    let count = |folder: &str| {
        fs::read_dir(format!("/proc/{pid}/{folder}"))
            .unwrap()
            .count()
    };
    (count("task"), count("fd"))
}

#[cfg(target_os = "linux")]
#[test]
fn a_process_left_by_the_cache_clean_takes_no_thread_or_descriptor_of_the_watcher() {
    if !has_php() {
        return;
    }
    let store = Store::new();
    fake_magento(&store, "linger");
    let mut watcher = store.watch(&["--watch"]);
    let before = threads_and_descriptors(watcher.child.id());
    for round in 1..=3 {
        replace(
            store.root(),
            "app/etc/config.php",
            &format!(
                "<?php\nreturn [\n    'modules' => [\n        'Acme_On' => 1,\n    ],\n];\n// {round}\n"
            ),
        );
        assert!(watcher.next_status(WAIT).contains("✓ compiled"));
        let cleans = || {
            fs::read_to_string(store.root().join("var/clean.log"))
                .unwrap_or_default()
                .lines()
                .count()
        };
        let deadline = Instant::now() + WAIT;
        while cleans() < round {
            assert!(Instant::now() < deadline, "the cache clean never ran");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    // Past the drain time of the last clean; the lingering processes still run.
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(
        threads_and_descriptors(watcher.child.id()),
        before,
        "{}",
        watcher.seen.join("\n")
    );
}
