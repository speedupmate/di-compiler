//! Magento reads methods through `ReflectionClass`, so a method from a trait counts as a
//! method of the class that uses it. It wins over a method of the parent class. The
//! Interceptor has to repeat its signature or PHP refuses to load the Interceptor.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn write_class(root: &Path, class: &str, source: &str) {
    let (namespace, _) = class.rsplit_once('\\').unwrap();
    let relative = class.replace('\\', "/");
    write(
        root,
        &format!("app/code/{relative}.php"),
        &format!("<?php\nnamespace {namespace};\n\n{source}\n"),
    );
}

fn write_fixture(root: &Path) {
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
         foreach (['app/code', 'generated/code', 'vendor/acme/lib/src'] as $dir) {\n        \
         $file = dirname(__DIR__) . '/' . $dir . '/' . str_replace('\\\\', '/', $class) . '.php';\n        \
         if (is_file($file)) {\n            require $file;\n            return;\n        }\n    }\n});\n",
    );
    write(
        root,
        "vendor/composer/installed.json",
        r#"{"packages": [{"name": "acme/lib", "install-path": "../acme/lib", "autoload": {"psr-4": {"AcmeLib\\": "src/AcmeLib/"}}}]}"#,
    );
    write(
        root,
        "vendor/acme/lib/src/AcmeLib/Shared.php",
        "<?php\nnamespace AcmeLib;\n\ntrait Shared\n{\n    public function shared(): int\n    {\n        return 1;\n    }\n}\n",
    );
    write(
        root,
        "app/code/Magento/Framework/Interception/Interceptor.php",
        "<?php\nnamespace Magento\\Framework\\Interception;\n\ntrait Interceptor\n{\n    \
         public function ___init()\n    {\n    }\n    \
         public function ___callPlugins($method, array $arguments, array $pluginInfo)\n    {\n    }\n}\n",
    );
    write(
        root,
        "app/code/Magento/Framework/Interception/InterceptorInterface.php",
        "<?php\nnamespace Magento\\Framework\\Interception;\n\ninterface InterceptorInterface\n{\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/etc/di.xml",
        r#"<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <type name="Acme\On\Model\Base">
        <plugin name="watch" type="Acme\On\Plugin\Watch"/>
    </type>
    <type name="Acme\On\Model\Viewed">
        <plugin name="trait_plugin" type="Acme\On\Plugin\FromTrait"/>
    </type>
</config>
"#,
    );

    write_class(
        root,
        "Acme\\On\\Model\\Base",
        "class Base\n{\n    public function __construct(\\Acme\\On\\Model\\Viewed $viewed)\n    {\n    }\n    \
         public function run()\n    {\n    }\n    public function shared()\n    {\n    }\n    \
         public function sealed()\n    {\n    }\n    public function kept(): string\n    {\n        return '';\n    }\n}",
    );
    write_class(
        root,
        "Acme\\On\\Model\\Runs",
        "trait Runs\n{\n    public function __construct(\\Acme\\On\\Model\\Gadget $gadget)\n    {\n    }\n    \
         public function run(): void\n    {\n    }\n    final public function sealed()\n    {\n    }\n    \
         abstract public function kept(): string;\n}",
    );
    write_class(
        root,
        "Acme\\On\\Model\\Outer",
        "trait Outer\n{\n    use Runs;\n}",
    );
    write_class(
        root,
        "Acme\\On\\Model\\Child",
        "class Child extends Base\n{\n    use Outer, \\AcmeLib\\Shared;\n}",
    );
    write_class(root, "Acme\\On\\Model\\Gadget", "class Gadget\n{\n}");
    write_class(
        root,
        "Acme\\On\\Model\\Viewed",
        "class Viewed\n{\n    use ViewedTrait;\n}",
    );
    write_class(
        root,
        "Acme\\On\\Model\\ViewedTrait",
        "trait ViewedTrait\n{\n    public function view(int $id): ?string\n    {\n        return null;\n    }\n}",
    );
    write_class(
        root,
        "Acme\\On\\Plugin\\Watch",
        "class Watch\n{\n    public function beforeRun($subject)\n    {\n    }\n    \
         public function beforeShared($subject)\n    {\n    }\n    public function beforeSealed($subject)\n    {\n    }\n    \
         public function beforeKept($subject)\n    {\n    }\n}",
    );
    write_class(
        root,
        "Acme\\On\\Plugin\\Listens",
        "trait Listens\n{\n    public function afterView($subject, $result)\n    {\n        return $result;\n    }\n}",
    );
    write_class(
        root,
        "Acme\\On\\Plugin\\FromTrait",
        "class FromTrait\n{\n    use Listens;\n}",
    );
}

fn compiled_fixture() -> Option<tempfile::TempDir> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return None;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    write_fixture(tmp.path());
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "compiler failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(tmp)
}

fn interceptor(root: &Path, class: &str) -> String {
    fs::read_to_string(
        root.join("generated/code")
            .join(class.replace('\\', "/"))
            .join("Interceptor.php"),
    )
    .unwrap()
}

/// The cache file of one scope holds a serialized copy of the plugin list.
fn plugin_list(root: &Path) -> String {
    let mut text = String::new();
    for entry in fs::read_dir(root.join("generated/metadata")).unwrap() {
        let path = entry.unwrap().path();
        if path.to_string_lossy().contains("plugin") {
            text.push_str(&fs::read_to_string(path).unwrap());
        }
    }
    text
}

fn method_line<'a>(code: &'a str, method: &str) -> Option<&'a str> {
    code.lines()
        .find(|line| line.contains(&format!("function {method}(")))
}

#[test]
fn interceptor_of_a_class_with_a_trait_method_loads_in_php() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let output = Command::new("php")
        .arg("-r")
        .arg(
            "require 'vendor/autoload.php'; \
             class_exists('Acme\\On\\Model\\Child\\Interceptor'); echo 'loaded';",
        )
        .current_dir(tmp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.ends_with("loaded"), "php said: {stdout}");
}

#[test]
fn trait_method_signature_wins_over_the_parent_method() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let code = interceptor(tmp.path(), "Acme\\On\\Model\\Child");
    let line = method_line(&code, "run").expect("run is intercepted");
    assert!(line.contains(": void"), "{line}");
}

#[test]
fn trait_from_a_composer_library_counts() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let code = interceptor(tmp.path(), "Acme\\On\\Model\\Child");
    let line = method_line(&code, "shared").expect("shared is intercepted");
    assert!(line.contains(": int"), "{line}");
}

#[test]
fn abstract_trait_method_does_not_replace_the_parent_method() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let code = interceptor(tmp.path(), "Acme\\On\\Model\\Child");
    assert!(method_line(&code, "kept").is_some());
}

#[test]
fn final_trait_method_is_not_intercepted() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let code = interceptor(tmp.path(), "Acme\\On\\Model\\Child");
    assert!(method_line(&code, "sealed").is_none());
}

#[test]
fn method_that_only_a_trait_declares_is_intercepted() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let code = interceptor(tmp.path(), "Acme\\On\\Model\\Viewed");
    let line = method_line(&code, "view").expect("view is intercepted");
    assert!(
        line.contains("int $id") && line.contains("?string"),
        "{line}"
    );
}

#[test]
fn plugin_method_from_a_trait_is_in_the_plugin_list() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let plugin_list = plugin_list(tmp.path());
    assert!(
        plugin_list.contains("Acme\\\\On\\\\Model\\\\Viewed_view___self"),
        "{plugin_list}"
    );
}

#[test]
fn trait_constructor_wins_over_the_parent_constructor() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let metadata = fs::read_to_string(tmp.path().join("generated/metadata/global.php")).unwrap();
    let arguments = &metadata[metadata.find("'arguments' =>").unwrap()..];
    let start = arguments
        .find("'Acme\\\\On\\\\Model\\\\Child\\\\Interceptor' =>")
        .or_else(|| arguments.find("'Acme\\\\On\\\\Model\\\\Child' =>"))
        .expect("Child has an arguments entry");
    let entry = &arguments[start..];
    let entry = &entry[..entry[1..].find("\n    '").map_or(entry.len(), |at| at + 1)];
    assert!(entry.contains("Gadget"), "{entry}");
    assert!(!entry.contains("Viewed"), "{entry}");
}
