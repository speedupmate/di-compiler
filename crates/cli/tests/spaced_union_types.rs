//! PHP allows whitespace and comments around the `|` of a union type. An interceptor keeps every
//! member of a union return type and every constructor parameter, in order, however the union
//! is written.

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

const RUNNER: &str = "<?php\nnamespace Acme\\On\\Model;\n\ninterface RunnerInterface\n{\n    public function run();\n}\n";

/// A store with `Acme\On\Model\<class>` for each source, and a plugin on `RunnerInterface`.
fn compile(classes: &[(&str, &str)]) -> tempfile::TempDir {
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
    write(root, "app/code/Acme/On/Model/RunnerInterface.php", RUNNER);
    for name in ["Gadget", "Widget"] {
        write(
            root,
            &format!("app/code/Acme/On/Model/{name}.php"),
            &format!("<?php\nnamespace Acme\\On\\Model;\n\nclass {name} implements \\Countable\n{{\n    public function count(): int\n    {{\n        return 0;\n    }}\n}}\n"),
        );
    }
    for (name, source) in classes {
        write(root, &format!("app/code/Acme/On/Model/{name}.php"), source);
    }
    write(
        root,
        "app/code/Acme/On/Plugin/Watch.php",
        "<?php\nnamespace Acme\\On\\Plugin;\n\nclass Watch\n{\n    public function beforeRun($subject)\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/etc/di.xml",
        "<config><type name=\"Acme\\On\\Model\\RunnerInterface\"><plugin name=\"p\" type=\"Acme\\On\\Plugin\\Watch\"/></type></config>\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_lints(&tmp);
    tmp
}

fn interceptor(store: &tempfile::TempDir, class: &str) -> String {
    fs::read_to_string(store.path().join(format!(
        "generated/code/Acme/On/Model/{class}/Interceptor.php"
    )))
    .unwrap()
}

fn line_with<'a>(source: &'a str, text: &str) -> &'a str {
    source
        .lines()
        .find(|line| line.contains(text))
        .unwrap_or_else(|| panic!("no line with {text} in\n{source}"))
        .trim()
}

const MODEL: &str = "\\Acme\\On\\Model";

#[test]
fn a_spaced_union_return_type_keeps_every_member() {
    if !has_php() {
        return;
    }
    let store = compile(&[(
        "Add",
        "<?php\ndeclare(strict_types=1);\nnamespace Acme\\On\\Model;\n\nclass Add implements RunnerInterface\n{\n    \
         public function run(): Gadget | Widget\n    {\n        return new Widget();\n    }\n}\n",
    )]);
    assert_eq!(
        line_with(&interceptor(&store, "Add"), "function run"),
        format!("public function run() : {MODEL}\\Gadget|{MODEL}\\Widget")
    );
}

#[test]
fn spaced_union_parameters_are_kept_in_their_place() {
    if !has_php() {
        return;
    }
    let store = compile(&[(
        "View",
        "<?php\ndeclare(strict_types=1);\nnamespace Acme\\On\\Model;\n\nclass View implements RunnerInterface\n{\n    \
         public function __construct(\n        Gadget $first,\n        Widget | null $middle,\n        \
         \\Acme\\On\\Model\\Gadget /* main */ | /* other */ Widget $commented,\n        Gadget $last,\n        \
         private Widget | null $tail = null\n    ) {\n    }\n\n    public function run()\n    {\n    }\n}\n",
    )]);
    let code = interceptor(&store, "View");
    assert_eq!(
        line_with(&code, "function __construct"),
        format!(
            "public function __construct({MODEL}\\Gadget $first, ?{MODEL}\\Widget $middle, \
             {MODEL}\\Gadget|{MODEL}\\Widget $commented, {MODEL}\\Gadget $last, ?{MODEL}\\Widget $tail = null)"
        )
    );
    assert_eq!(
        line_with(&code, "parent::__construct"),
        "parent::__construct($first, $middle, $commented, $last, $tail);"
    );
    let metadata = fs::read_to_string(store.path().join("generated/metadata/global.php")).unwrap();
    let start = metadata
        .find("'Acme\\\\On\\\\Model\\\\View\\\\Interceptor' => ")
        .expect("the interceptor has an arguments entry");
    let entry = &metadata[start..];
    let entry = &entry[..entry[1..]
        .find("\n    '")
        .map_or(entry.len(), |end| end + 1)];
    let names: Vec<&str> = entry
        .lines()
        .skip(1)
        .filter_map(|line| line.trim().strip_prefix('\'')?.strip_suffix("' =>"))
        .filter(|name| !name.starts_with('_'))
        .collect();
    assert_eq!(
        names,
        ["first", "middle", "commented", "last", "tail"],
        "{entry}"
    );
}

#[test]
fn a_dnf_parameter_is_kept_in_its_place() {
    if !has_php() {
        return;
    }
    let store = compile(&[(
        "Dnf",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Dnf implements RunnerInterface\n{\n    \
         public function __construct(Gadget $first, (Gadget&\\Countable)|null $either, Widget $last)\n    {\n    }\n\n    \
         public function run(Widget|(Gadget&\\Countable) $value = null): (Gadget&\\Countable)|null\n    {\n        return null;\n    }\n}\n",
    )]);
    let code = interceptor(&store, "Dnf");
    assert_eq!(
        line_with(&code, "function __construct"),
        format!(
            "public function __construct({MODEL}\\Gadget $first, ({MODEL}\\Gadget&\\Countable)|null $either, {MODEL}\\Widget $last)"
        )
    );
    assert_eq!(
        line_with(&code, "parent::__construct"),
        "parent::__construct($first, $either, $last);"
    );
    assert!(
        line_with(&code, "function run").contains(&format!(": ({MODEL}\\Gadget&\\Countable)|null")),
        "{code}"
    );
}

/// Every generated interceptor is valid PHP.
fn assert_lints(store: &tempfile::TempDir) {
    let code = store.path().join("generated/code");
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
