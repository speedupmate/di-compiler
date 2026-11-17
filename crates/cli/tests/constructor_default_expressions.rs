//! Magento reads constructor defaults through PHP reflection
//! (`Code\Reader\ClassReader::getConstructor()`), so the compiled metadata holds the value
//! of a default and never its source text. Text in place of a value is a `TypeError` for a
//! typed parameter and a wrong value for any other one.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn write_module(root: &Path, di_xml: &str) {
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
    write(root, "app/code/Acme/On/etc/di.xml", di_xml);
    // Stands in for Composer: like a Composer autoloader it does not know `generated/code`.
    write(
        root,
        "vendor/autoload.php",
        r#"<?php
spl_autoload_register(function (string $class): void {
    $file = dirname(__DIR__) . '/app/code/' . str_replace('\\', '/', $class) . '.php';
    if (is_file($file)) {
        require $file;
    }
});
"#,
    );
}

fn write_defaults_fixture(root: &Path) {
    write_module(root, "<config/>\n");
    write(
        root,
        "app/code/Acme/On/Model/Status.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nenum Status: string\n{\n    case Open = 'open';\n}\n",
    );
    write(
        root,
        "app/code/Acme/On/Model/Clock.php",
        r#"<?php
namespace Acme\On\Model;

class Clock
{
    public function __construct(public int $tick = 0)
    {
    }

    public static function __set_state(array $state): self
    {
        return new self($state['tick']);
    }
}
"#,
    );
    write(
        root,
        "app/code/Acme/On/Api/Paths.php",
        "<?php\nnamespace Acme\\On\\Api;\n\ninterface Paths\n{\n    public const ENGINE = 'catalog/search/engine';\n}\n",
    );
    // One class per form: a class with one `::` default was already reflected as a whole,
    // which hides what happens to the other parameters.
    for (param, declaration) in FORMS {
        write(
            root,
            &format!("app/code/Acme/On/Model/Form/{}.php", class_name(param)),
            &format!(
                "<?php\nnamespace Acme\\On\\Model\\Form;\n\nuse Acme\\On\\Api\\Paths;\nuse Acme\\On\\Model\\Clock;\nuse Acme\\On\\Model\\Status;\n\nclass {}\n{{\n    public const NAME = 'own';\n\n    public function __construct({declaration})\n    {{\n    }}\n}}\n",
                class_name(param)
            ),
        );
    }
}

const FORMS: &[(&str, &str)] = &[
    ("product", "int $product = 2 * 1024 * 1024"),
    ("imported", "string $imported = Paths::ENGINE"),
    ("own", "string $own = self::NAME"),
    ("joined", "string $joined = 'left' . '-right'"),
    ("global", "int $global = E_WARNING"),
    ("magic", "string $magic = __CLASS__"),
    ("tab", "string $tab = \"a\\tb\""),
    ("octal", "int $octal = 010"),
    ("hex", "int $hex = 0x10"),
    ("exponent", "float $exponent = 1e3"),
    (
        "heredoc",
        "string $heredoc = <<<TEXT\n        it's, here\n        TEXT",
    ),
    ("case", "Status $case = Status::Open"),
    ("clock", "Clock $clock = new Clock(5)"),
    (
        "expressions",
        "array $expressions = ['size' => 2 * 8, 'path' => 'x' . 'y']",
    ),
    ("cases", "array $cases = ['first' => Status::Open]"),
    ("sparse", "array $sparse = [5 => 'a', 'b']"),
    ("slashes", "array $slashes = ['Foo\\Bar']"),
    ("literal", "string $literal = 'plain'"),
    ("number", "int $number = 42"),
    ("list", "array $list = ['a', 'b']"),
];

fn class_name(param: &str) -> String {
    let mut name = param.to_string();
    name[..1].make_ascii_uppercase();
    format!("Form{name}")
}

fn php() -> String {
    std::env::var("PHP_BIN").unwrap_or_else(|_| "php".to_string())
}

fn compile(root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .arg("--fallback-php")
        .arg(php())
        .output()
        .unwrap()
}

fn compiled(root: &Path) {
    let output = compile(root);
    assert!(
        output.status.success(),
        "compiler failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Loads the area file in PHP, as the store does, and exports one argument entry.
fn argument(root: &Path, type_name: &str, param: &str) -> String {
    let script = r#"
require $argv[1] . '/vendor/autoload.php';
$file = require $argv[1] . '/generated/metadata/global.php';
echo preg_replace('/\s+/', '', var_export($file['arguments'][$argv[2]][$argv[3]], true));
"#;
    let output = Command::new(php())
        .args(["-d", "display_errors=1", "-r", script, "--"])
        .arg(root)
        .arg(type_name)
        .arg(param)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(output.status.success(), "php failed: {stdout}");
    stdout
}

fn assert_values(expected: &[(&str, &str)]) {
    let tmp = tempfile::tempdir().unwrap();
    write_defaults_fixture(tmp.path());
    compiled(tmp.path());
    for (param, value) in expected {
        assert_eq!(
            argument(
                tmp.path(),
                &format!("Acme\\On\\Model\\Form\\{}", class_name(param)),
                param
            ),
            format!("array('_v_'=>{value},)"),
            "parameter {param}"
        );
    }
}

#[test]
fn arithmetic_default_is_written_as_its_value() {
    assert_values(&[("product", "2097152")]);
}

#[test]
fn class_constant_defaults_are_written_as_their_value() {
    assert_values(&[("imported", "'catalog/search/engine'"), ("own", "'own'")]);
}

#[test]
fn concatenated_default_is_written_as_its_value() {
    assert_values(&[("joined", "'left-right'")]);
}

#[test]
fn global_constant_default_keeps_its_type() {
    assert_values(&[("global", "2")]);
}

#[test]
fn magic_constant_default_is_written_as_its_value() {
    assert_values(&[("magic", "'Acme\\\\On\\\\Model\\\\Form\\\\FormMagic'")]);
}

#[test]
fn double_quoted_escape_is_written_as_its_value() {
    assert_values(
        &[("tab", "'a\tb'")]
            .map(|(p, v)| (p, v))
            .map(|(p, _)| (p, "'ab'")),
    );
}

#[test]
fn number_notations_are_written_as_their_value() {
    assert_values(&[("octal", "8"), ("hex", "16"), ("exponent", "1000.0")]);
}

#[test]
fn heredoc_default_is_written_as_its_value() {
    assert_values(&[("heredoc", "'it\\'s,here'")]);
}

#[test]
fn enum_case_default_is_written_as_the_case() {
    assert_values(&[("case", "\\Acme\\On\\Model\\Status::Open")]);
}

#[test]
fn new_in_initializer_is_written_as_the_object() {
    assert_values(&[(
        "clock",
        "\\Acme\\On\\Model\\Clock::__set_state(array('tick'=>5,))",
    )]);
}

#[test]
fn array_holding_expressions_is_written_with_values() {
    assert_values(&[
        ("expressions", "array('size'=>16,'path'=>'xy',)"),
        ("cases", "array('first'=>\\Acme\\On\\Model\\Status::Open,)"),
        ("sparse", "array(5=>'a',6=>'b',)"),
        ("slashes", "array(0=>'Foo\\\\Bar',)"),
    ]);
}

#[test]
fn plain_literals_stay_correct() {
    assert_values(&[
        ("literal", "'plain'"),
        ("number", "42"),
        ("list", "array(0=>'a',1=>'b',)"),
    ]);
}

#[test]
fn default_of_a_class_extending_a_generated_factory_is_written_as_its_value() {
    let tmp = tempfile::tempdir().unwrap();
    write_module(tmp.path(), "<config/>\n");
    write(
        tmp.path(),
        "app/code/Acme/On/Model/Thing.php",
        "<?php\nnamespace Acme\\On\\Model;\n\nclass Thing\n{\n}\n",
    );
    write(
        tmp.path(),
        "app/code/Acme/On/Model/Own/ThingFactory.php",
        r#"<?php
namespace Acme\On\Model\Own;

class ThingFactory extends \Acme\On\Model\ThingFactory
{
    public const PATH = 'catalog/search/engine';

    public function __construct(string $configPath = self::PATH, int $size = 2 * 8)
    {
    }
}
"#,
    );
    compiled(tmp.path());

    let class = "Acme\\On\\Model\\Own\\ThingFactory";
    assert_eq!(
        argument(tmp.path(), class, "configPath"),
        "array('_v_'=>'catalog/search/engine',)"
    );
    assert_eq!(argument(tmp.path(), class, "size"), "array('_v_'=>16,)");
}

#[test]
fn interceptor_of_a_class_with_a_relative_default_does_not_copy_the_text() {
    let tmp = tempfile::tempdir().unwrap();
    write_module(
        tmp.path(),
        r#"<config>
    <type name="Acme\On\Model\Own\Target">
        <plugin name="p" type="Acme\On\Plugin\TargetPlugin"/>
    </type>
</config>
"#,
    );
    write(
        tmp.path(),
        "app/code/Acme/On/Plugin/TargetPlugin.php",
        "<?php\nnamespace Acme\\On\\Plugin;\n\nclass TargetPlugin\n{\n    public function beforeRun($subject, $mode = null)\n    {\n        return [$mode];\n    }\n}\n",
    );
    write(
        tmp.path(),
        "app/code/Acme/On/Model/Own/Target.php",
        r#"<?php
namespace Acme\On\Model\Own;

const LOCAL = 'local-value';

class Target
{
    public function __construct(public string $start = LOCAL . '-start')
    {
    }

    public function run(string $mode = LOCAL): string
    {
        return $mode;
    }
}
"#,
    );
    compiled(tmp.path());

    let code = fs::read_to_string(
        tmp.path()
            .join("generated/code/Acme/On/Model/Own/Target/Interceptor.php"),
    )
    .unwrap();
    assert!(code.contains("'local-value-start'"), "{code}");
    assert!(code.contains("$mode = 'local-value'"), "{code}");
}

#[test]
fn compile_stops_when_php_cannot_load_a_class_with_an_expression_default() {
    let tmp = tempfile::tempdir().unwrap();
    write_module(tmp.path(), "<config/>\n");
    write(
        tmp.path(),
        "app/code/Acme/On/Model/Broken.php",
        r#"<?php
namespace Acme\On\Model;

class Broken extends \Acme\Missing\Base
{
    public function __construct(int $size = 2 * 8)
    {
    }
}
"#,
    );

    let output = compile(tmp.path());

    assert!(!output.status.success(), "compile must not succeed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Acme\\On\\Model\\Broken"), "{stderr}");
}

#[test]
fn compile_stops_on_an_object_default_that_needs_generated_code() {
    let tmp = tempfile::tempdir().unwrap();
    write_module(
        tmp.path(),
        r#"<config>
    <type name="Acme\On\Model\Target">
        <plugin name="p" type="Acme\On\Plugin\TargetPlugin"/>
    </type>
</config>
"#,
    );
    write(
        tmp.path(),
        "app/code/Acme/On/Plugin/TargetPlugin.php",
        "<?php\nnamespace Acme\\On\\Plugin;\n\nclass TargetPlugin\n{\n    public function beforeRun($subject)\n    {\n    }\n}\n",
    );
    write(
        tmp.path(),
        "app/code/Acme/On/Model/Target.php",
        r#"<?php
namespace Acme\On\Model;

class Target
{
    public function __construct(\stdClass $options = new \stdClass())
    {
    }

    public function run(): void
    {
    }
}
"#,
    );

    let output = compile(tmp.path());

    assert!(!output.status.success(), "compile must not succeed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Acme\\On\\Model\\Target"), "{stderr}");
}
