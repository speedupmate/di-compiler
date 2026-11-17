//! `self` and `parent` in a method signature mean the class that declares the method.
//! `getReturnTypeValue()` in Magento's `GetReflectionMethodReturnTypeValueTrait` uses
//! `getDeclaringClass()` for this. `static` stays `static`, and a class name in its place
//! is a fatal error when PHP loads the Interceptor.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn write_model(root: &Path, name: &str, source: &str) {
    write(
        root,
        &format!("app/code/Acme/On/Model/{name}.php"),
        &format!("<?php\nnamespace Acme\\On\\Model;\n\n{source}\n"),
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
         foreach (['app/code', 'generated/code'] as $dir) {\n        \
         $file = dirname(__DIR__) . '/' . $dir . '/' . str_replace('\\\\', '/', $class) . '.php';\n        \
         if (is_file($file)) {\n            require $file;\n            return;\n        }\n    }\n});\n",
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
        "app/code/Magento/Framework/ObjectManager/NoninterceptableInterface.php",
        "<?php\nnamespace Magento\\Framework\\ObjectManager;\n\ninterface NoninterceptableInterface\n{\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/etc/di.xml",
        r#"<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <type name="Acme\On\Model\Child">
        <plugin name="watch" type="Acme\On\Plugin\Watch"/>
    </type>
    <type name="Acme\On\Model\User">
        <arguments>
            <argument name="child" xsi:type="object">Acme\On\Model\Child\Proxy</argument>
        </arguments>
    </type>
</config>
"#,
    );
    write_model(root, "Grand", "class Grand\n{\n}");
    write_model(
        root,
        "Shared",
        "trait Shared\n{\n    public function shared(): self\n    {\n        return new self();\n    }\n}",
    );
    write_model(
        root,
        "Base",
        "class Base extends Grand\n{\n    use Shared;\n\n    \
         public function fresh(): self\n    {\n        return new self();\n    }\n    \
         public function maybe(): ?self\n    {\n        return null;\n    }\n    \
         public function either(): self|array\n    {\n        return [];\n    }\n    \
         public function own(): static\n    {\n        return $this;\n    }\n    \
         public function up(): parent\n    {\n        return new Grand();\n    }\n    \
         public function take(self $other): void\n    {\n    }\n}",
    );
    write_model(root, "Child", "class Child extends Base\n{\n}");
    write_model(
        root,
        "User",
        "class User\n{\n    public function __construct(Child $child)\n    {\n    }\n}",
    );
    write(
        root,
        "app/code/Acme/On/Plugin/Watch.php",
        "<?php\nnamespace Acme\\On\\Plugin;\n\nclass Watch\n{\n    \
         public function beforeFresh($subject)\n    {\n    }\n    \
         public function beforeMaybe($subject)\n    {\n    }\n    \
         public function beforeEither($subject)\n    {\n    }\n    \
         public function beforeOwn($subject)\n    {\n    }\n    \
         public function beforeUp($subject)\n    {\n    }\n    \
         public function beforeTake($subject, $other)\n    {\n    }\n    \
         public function beforeShared($subject)\n    {\n    }\n}\n",
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

fn generated(root: &Path, kind: &str) -> String {
    fs::read_to_string(root.join(format!("generated/code/Acme/On/Model/Child/{kind}.php"))).unwrap()
}

fn signature(code: &str, method: &str) -> String {
    code.lines()
        .find(|line| line.contains(&format!("function {method}(")))
        .unwrap_or_else(|| panic!("no method {method} in\n{code}"))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn self_and_parent_mean_the_declaring_class_in_both_generators() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    for kind in ["Interceptor", "Proxy"] {
        let code = generated(tmp.path(), kind);
        for (method, expected) in [
            ("fresh", "fresh() : \\Acme\\On\\Model\\Base"),
            ("maybe", "maybe() : ?\\Acme\\On\\Model\\Base"),
            ("either", "\\Acme\\On\\Model\\Base"),
            ("up", "up() : \\Acme\\On\\Model\\Grand"),
            ("take", "take(\\Acme\\On\\Model\\Base $other)"),
            ("shared", "shared() : \\Acme\\On\\Model\\Base"),
            ("own", "own() : static"),
        ] {
            let line = signature(&code, method);
            assert!(line.contains(expected), "{kind}: {line}");
        }
    }
}

#[test]
fn generated_classes_load_in_php() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    let output = Command::new("php")
        .arg("-r")
        .arg(
            "require 'vendor/autoload.php'; \
             class_exists('Acme\\On\\Model\\Child\\Interceptor'); \
             class_exists('Acme\\On\\Model\\Child\\Proxy'); echo 'loaded';",
        )
        .current_dir(tmp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.ends_with("loaded"), "php said: {stdout}");
}
