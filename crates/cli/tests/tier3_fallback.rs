//! A class Tiers 1 and 2 cannot read is read by PHP (Tier 3) and compiled like any other: its
//! constructor arguments, an interceptor for a plugin on it, and the plugin order and argument
//! inheritance of its interfaces. The same declaration inside `if` is read but not compiled,
//! because Magento's scanner does not see it.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Skips without php, as the other tests here do, and fails on a php older than 8.4: property
/// hooks need 8.4.
fn has_php_8_4() -> bool {
    let Ok(output) = Command::new("php")
        .args(["-r", "echo PHP_VERSION_ID, ' ', PHP_VERSION;"])
        .output()
    else {
        eprintln!("skipped: these tests need php on the PATH");
        return false;
    };
    let version = String::from_utf8_lossy(&output.stdout).into_owned();
    let (id, text) = version.split_once(' ').unwrap_or(("0", version.as_str()));
    assert!(
        id.parse::<u32>().unwrap_or(0) >= 80400,
        "these tests need PHP 8.4 or later, found {text}"
    );
    true
}

/// A store with module `Acme_On`, the given files under `app/code/Acme/On`, and `di.xml`.
fn compile(files: &[(&str, &str)], di_xml: &str) -> (tempfile::TempDir, String) {
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
         $file = dirname(__DIR__) . '/app/code/' . str_replace('\\\\', '/', $class) . '.php';\n    \
         if (is_file($file)) {\n        require $file;\n    }\n});\n",
    );
    for (relative, source) in files {
        write(root, &format!("app/code/Acme/On/{relative}"), source);
    }
    write(root, "app/code/Acme/On/etc/di.xml", di_xml);
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{log}");
    assert_lints(&tmp);
    (tmp, log)
}

fn read_by_tier3(log: &str, class: &str) -> bool {
    log.lines()
        .any(|line| line.contains("with PHP (Tier 3)") && line.ends_with(&format!(": {class}")))
}

/// The entries of a compiled metadata file whose key names `class`, at any depth, as JSON.
fn entries_for(file: &Path, class: &str) -> String {
    let output = Command::new("php")
        .arg("-r")
        .arg(
            r#"function pick($data, $name) {
    $out = [];
    foreach ((array) $data as $key => $value) {
        if (is_string($key) && stripos($key, $name) !== false) {
            $out[$key] = $value;
        } elseif (is_array($value) && ($inner = pick($value, $name)) !== []) {
            $out[$key] = $inner;
        }
    }
    return $out;
}
echo json_encode(pick(require $argv[1], $argv[2]), JSON_UNESCAPED_SLASHES);"#,
        )
        .arg(file)
        .arg(class)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

const RUNNER: (&str, &str) = (
    "Model/RunnerInterface.php",
    "<?php\nnamespace Acme\\On\\Model;\n\ninterface RunnerInterface\n{\n    public function run();\n}\n",
);
const GADGET: (&str, &str) = (
    "Model/Gadget.php",
    "<?php\nnamespace Acme\\On\\Model;\n\ninterface Gadget\n{\n}\n",
);
const WATCH: (&str, &str) = (
    "Plugin/Watch.php",
    "<?php\nnamespace Acme\\On\\Plugin;\n\nclass Watch\n{\n    public function beforeRun($subject)\n    {\n    }\n}\n",
);
const WATCH_RUNNERS: &str = "<config><type name=\"Acme\\On\\Model\\RunnerInterface\"><plugin name=\"p\" type=\"Acme\\On\\Plugin\\Watch\"/></type></config>\n";

/// Tier 1 rejects the intersection type; tree-sitter-php does not know property hooks.
const HOOKED_BODY: &str = "class Hooked implements RunnerInterface\n{\n    \
     public string $label { get => 'hooked'; }\n\n    \
     public function __construct(private Gadget&\\Countable $gadget, private int $size = 3)\n    {\n    }\n\n    \
     public function run()\n    {\n    }\n}\n";

#[test]
fn a_class_only_php_can_read_gets_its_arguments_and_interceptor() {
    if !has_php_8_4() {
        return;
    }
    let hooked = format!("<?php\nnamespace Acme\\On\\Model;\n\n{HOOKED_BODY}");
    let (store, log) = compile(
        &[RUNNER, GADGET, WATCH, ("Model/Hooked.php", &hooked)],
        WATCH_RUNNERS,
    );
    assert!(read_by_tier3(&log, "Acme\\On\\Model\\Hooked"), "{log}");

    let interceptor = fs::read_to_string(
        store
            .path()
            .join("generated/code/Acme/On/Model/Hooked/Interceptor.php"),
    )
    .expect("an interceptor for the plugin on RunnerInterface");
    assert!(
        interceptor.contains("public function run()"),
        "{interceptor}"
    );
    assert!(
        interceptor.contains("\\Acme\\On\\Model\\Gadget&\\Countable $gadget, int $size = 3"),
        "{interceptor}"
    );

    let arguments = entries_for(
        &store.path().join("generated/metadata/global.php"),
        "Acme\\On\\Model\\Hooked",
    );
    // An intersection names no single class, so the argument is a required value, as for a
    // class Tier 1 or 2 reads.
    assert_eq!(
        arguments,
        r#"{"arguments":{"Acme\\On\\Model\\Hooked\\Interceptor":{"gadget":{"_vn_":true},"size":{"_v_":3}}},"preferences":{"Acme\\On\\Model\\Hooked":"Acme\\On\\Model\\Hooked\\Interceptor"}}"#
    );
}

#[test]
fn the_same_class_inside_if_is_read_but_not_compiled() {
    if !has_php_8_4() {
        return;
    }
    let guarded = format!("<?php\nnamespace Acme\\On\\Model;\n\nif (true) {{\n{HOOKED_BODY}}}\n");
    let (store, log) = compile(
        &[RUNNER, GADGET, WATCH, ("Model/Hooked.php", &guarded)],
        WATCH_RUNNERS,
    );
    assert!(read_by_tier3(&log, "Acme\\On\\Model\\Hooked"), "{log}");
    assert!(!store
        .path()
        .join("generated/code/Acme/On/Model/Hooked")
        .exists());
    assert_eq!(
        entries_for(
            &store.path().join("generated/metadata/global.php"),
            "Acme\\On\\Model\\Hooked"
        ),
        "[]"
    );
}

/// `Tier1Twin` and `Tier3Twin` both `extend Base implements Second, First`, and `Base`
/// implements `First`. A static method with an intersection type, which Tier 1 rejects, and a
/// property hook, which tree-sitter-php does not know, send `Tier3Twin` to Tier 3. Neither
/// changes the metadata.
fn twin(name: &str, hooked: bool) -> String {
    format!(
        "<?php\nnamespace Acme\\On\\Model;\n\nclass {name} extends Base implements Second, First\n{{\n    \
         {}public function __construct(private string $label = 'own')\n    {{\n    }}\n\n    \
         public function run()\n    {{\n    }}\n}}\n",
        if hooked {
            "public string $shown { get => $this->label; }\n\n    \
             public static function accepts(\\Countable&\\Traversable $items): bool\n    {\n        return true;\n    }\n\n    "
        } else {
            ""
        }
    )
}

#[test]
fn interface_order_and_inherited_arguments_are_the_same_through_tier1_and_tier3() {
    if !has_php_8_4() {
        return;
    }
    let tier1 = twin("Tier1Twin", false);
    let tier3 = twin("Tier3Twin", true);
    let interface = |name: &str| {
        format!("<?php\nnamespace Acme\\On\\Model;\n\ninterface {name}\n{{\n    public function run();\n}}\n")
    };
    let plugin = |name: &str| {
        format!("<?php\nnamespace Acme\\On\\Plugin;\n\nclass {name}\n{{\n    public function beforeRun($subject)\n    {{\n    }}\n}}\n")
    };
    let (first, second) = (interface("First"), interface("Second"));
    let (on_first, on_second) = (plugin("OnFirst"), plugin("OnSecond"));
    let (store, log) = compile(
        &[
            ("Model/First.php", &first),
            ("Model/Second.php", &second),
            (
                "Model/Base.php",
                "<?php\nnamespace Acme\\On\\Model;\n\nabstract class Base implements First\n{\n}\n",
            ),
            ("Model/Tier1Twin.php", &tier1),
            ("Model/Tier3Twin.php", &tier3),
            ("Plugin/OnFirst.php", &on_first),
            ("Plugin/OnSecond.php", &on_second),
        ],
        "<config>\
         <type name=\"Acme\\On\\Model\\First\"><plugin name=\"first\" type=\"Acme\\On\\Plugin\\OnFirst\"/>\
         <arguments><argument name=\"label\" xsi:type=\"string\">from-first</argument></arguments></type>\
         <type name=\"Acme\\On\\Model\\Second\"><plugin name=\"second\" type=\"Acme\\On\\Plugin\\OnSecond\"/></type>\
         <type name=\"Acme\\On\\Model\\Tier1Twin\"><plugin name=\"own\" type=\"Acme\\On\\Plugin\\OnFirst\"/></type>\
         <type name=\"Acme\\On\\Model\\Tier3Twin\"><plugin name=\"own\" type=\"Acme\\On\\Plugin\\OnFirst\"/></type>\
         </config>\n",
    );
    assert!(read_by_tier3(&log, "Acme\\On\\Model\\Tier3Twin"), "{log}");
    assert!(!read_by_tier3(&log, "Acme\\On\\Model\\Tier1Twin"), "{log}");

    let metadata = store.path().join("generated/metadata");
    let mut compared = 0;
    for entry in fs::read_dir(&metadata).unwrap().flatten() {
        let file = entry.path();
        let via_tier1 = entries_for(&file, "Tier1Twin");
        let via_tier3 = entries_for(&file, "Tier3Twin").replace("Tier3Twin", "Tier1Twin");
        assert_eq!(via_tier3, via_tier1, "{}", file.display());
        if via_tier1 != "[]" {
            compared += 1;
        }
    }
    // The twin's plugins after inheritance: from `Base` (which has `First`), then `Second`, the
    // interface `Base` does not have, then its own.
    let order = entries_for(
        &metadata.join("primary|global|plugin-list.php"),
        "Tier1Twin",
    );
    assert!(
        order.contains(r#""Acme\\On\\Model\\Tier1Twin_run___self":{"1":["first","second","own"]}"#),
        "{order}"
    );
    let arguments = entries_for(&metadata.join("global.php"), "Tier1Twin");
    assert!(arguments.contains("from-first"), "{arguments}");
    assert!(compared >= 2, "nothing compared");
}

fn assert_lints(store: &tempfile::TempDir) {
    let code = store.path().join("generated/code");
    if !code.exists() {
        return;
    }
    for entry in walk(&code) {
        let output = Command::new("php").arg("-l").arg(&entry).output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            fs::read_to_string(&entry).unwrap()
        );
    }
}

fn walk(folder: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(folder).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else if path.extension().is_some_and(|extension| extension == "php") {
            files.push(path);
        }
    }
    files
}
