//! `InterceptionConfigurationBuilder::getInterceptedMethods()` makes one method list per class
//! from all its plugins, including those of its parents. `Interception\Definition\Runtime` reads
//! a plugin with `get_class_methods()`, so inherited plugin methods count. A method that the list
//! leaves out of the Interceptor never fires its plugins.

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
         $file = dirname(__DIR__) . '/app/code/' . str_replace('\\\\', '/', $class) . '.php';\n    \
         if (is_file($file)) {\n        require $file;\n    }\n});\n",
    );
    write(
        root,
        "app/code/Acme/On/etc/di.xml",
        r#"<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <type name="Acme\On\Model\Block">
        <plugin name="on_block" type="Acme\On\Plugin\OnBlock"/>
    </type>
    <type name="Acme\On\Model\Tree">
        <plugin name="on_tree" type="Acme\On\Plugin\OnTree"/>
    </type>
    <type name="Acme\On\Model\Saver">
        <plugin name="inherited" type="Acme\On\Plugin\ChildOfBase"/>
    </type>
    <type name="Acme\On\Model\Bare">
        <plugin name="on_bare" type="Acme\On\Plugin\OnBare"/>
    </type>
</config>
"#,
    );

    write_class(
        root,
        "Acme\\On\\Model\\Block",
        "class Block\n{\n    public function toHtml()\n    {\n    }\n}",
    );
    write_class(
        root,
        "Acme\\On\\Model\\Tree",
        "class Tree extends Block\n{\n    public function nodes()\n    {\n    }\n}",
    );
    write_class(
        root,
        "Acme\\On\\Plugin\\OnBlock",
        "class OnBlock\n{\n    public function beforeToHtml($subject)\n    {\n    }\n}",
    );
    write_class(
        root,
        "Acme\\On\\Plugin\\OnTree",
        "class OnTree\n{\n    public function beforeNodes($subject)\n    {\n    }\n    \
         public function afterOnlyInAnotherTree($subject, $result)\n    {\n        return $result;\n    }\n}",
    );
    write_class(
        root,
        "Acme\\On\\Model\\Saver",
        "class Saver\n{\n    public function save()\n    {\n    }\n    public function other()\n    {\n    }\n}",
    );
    write_class(
        root,
        "Acme\\On\\Plugin\\BasePlugin",
        "class BasePlugin\n{\n    public function beforeSave($subject)\n    {\n    }\n}",
    );
    write_class(
        root,
        "Acme\\On\\Plugin\\ChildOfBase",
        "class ChildOfBase extends BasePlugin\n{\n}",
    );
    write_class(
        root,
        "Acme\\On\\Model\\Bare",
        "class Bare\n{\n    function noKeyword()\n    {\n    }\n}",
    );
    write_class(
        root,
        "Acme\\On\\Plugin\\OnBare",
        "class OnBare\n{\n    function beforeNoKeyword($subject)\n    {\n    }\n}",
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

fn has_method(root: &Path, class: &str, method: &str) -> bool {
    let path = root
        .join("generated/code")
        .join(class.replace('\\', "/"))
        .join("Interceptor.php");
    fs::read_to_string(path)
        .unwrap()
        .contains(&format!("function {method}("))
}

#[test]
fn plugin_on_a_method_the_class_lacks_keeps_methods_of_parent_plugins() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    assert!(has_method(tmp.path(), "Acme\\On\\Model\\Tree", "nodes"));
    assert!(has_method(tmp.path(), "Acme\\On\\Model\\Tree", "toHtml"));
}

#[test]
fn plugin_method_inherited_by_the_plugin_class_counts() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    assert!(has_method(tmp.path(), "Acme\\On\\Model\\Saver", "save"));
    assert!(!has_method(tmp.path(), "Acme\\On\\Model\\Saver", "other"));
}

#[test]
fn method_without_a_visibility_keyword_is_public() {
    let Some(tmp) = compiled_fixture() else {
        return;
    };
    assert!(has_method(tmp.path(), "Acme\\On\\Model\\Bare", "noKeyword"));
}
