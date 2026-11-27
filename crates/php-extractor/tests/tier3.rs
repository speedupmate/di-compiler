//! Tier 3 against a small store on disk: loading context, runner failures, and the comparison
//! with the tier that reads each fixture today.

use std::path::{Path, PathBuf};
use std::time::Duration;

use php_extractor::lexer::Lexer;
use php_extractor::tier3::{reflect, reflect_with_facts, Tier3Config};
use php_extractor::types::optional_in_php;
use php_extractor::types::MethodParam;
use php_extractor::{
    apply_traits, extract_file, ClassInfo, ClassKind, ConstructorParam, ExtractResult,
    MethodSignature,
};
use rustc_hash::FxHashMap;
use tempfile::TempDir;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn php_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Names PHP needs to declare the fixtures that no fixture declares.
const FIXTURE_STUBS: &[&str] = &[
    "Magento\\Framework\\Model\\ResourceModel\\Db\\ObjectRelationInterface",
    "Magento\\Framework\\DataObject\\IdentityInterface",
    "Magento\\Framework\\Data\\SearchResultInterface",
];

/// Files of the test store beside the fixtures, by class.
const STORE_CLASSES: &[(&str, &str)] = &[
    ("Acme\\Data\\Item", "<?php\nnamespace Acme\\Data;\nclass Item {}\n"),
    (
        "Magento\\Framework\\ObjectManagerInterface",
        "<?php\nnamespace Magento\\Framework;\ninterface ObjectManagerInterface {\n    public function create($type, array $arguments = []);\n    public function get($type);\n    public function configure(array $configuration);\n}\n",
    ),
    (
        "Magento\\Framework\\App\\ObjectManager",
        "<?php\nnamespace Magento\\Framework\\App;\nclass ObjectManager {\n    private static $instance;\n    public static function setInstance(\\Magento\\Framework\\ObjectManagerInterface $instance) { self::$instance = $instance; }\n    public static function getInstance() { return self::$instance; }\n}\n",
    ),
    (
        "Magento\\Framework\\App\\Bootstrap",
        "<?php\nnamespace Magento\\Framework\\App;\nclass Bootstrap {\n    public static function create($root, array $params) { return new self(); }\n    public function getObjectManager() {\n        return new class implements \\Magento\\Framework\\ObjectManagerInterface {\n            public function create($type, array $arguments = []) { return new \\stdClass(); }\n            public function get($type) { return new \\stdClass(); }\n            public function configure(array $configuration) {}\n        };\n    }\n}\n",
    ),
];

const SCANNER_CLASS: &str = "Magento\\Setup\\Module\\Di\\Code\\Reader\\FileClassScanner";

/// Answers what a `// scanner: <answer>` comment in the file says, and no class without one.
const SCANNER: &str = "<?php\nnamespace Magento\\Setup\\Module\\Di\\Code\\Reader;\nclass FileClassScanner {\n    public function __construct(private string $file) {}\n    public function getClassName(): string {\n        return preg_match('~// scanner: (\\S*)~', file_get_contents($this->file), $m) ? $m[1] : '';\n    }\n}\n";

struct Store {
    dir: TempDir,
    config: Tier3Config,
}

impl Store {
    /// A store whose autoloader knows every fixture class, the stubs and `STORE_CLASSES`, and
    /// prints while it loads, as some stores do.
    fn new() -> Self {
        Self::build(false)
    }

    /// A store with a stand-in for Magento's `FileClassScanner` (`SCANNER`).
    fn with_scanner() -> Self {
        Self::build(true)
    }

    fn build(scanner: bool) -> Self {
        require_php_8_4();
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let mut classmap: Vec<(String, PathBuf)> = Vec::new();
        for fixture in fixture_files() {
            if let ExtractResult::Ok(info) = extract_file(&fixture) {
                classmap.push((info.fqcn, fixture));
            }
        }
        std::fs::create_dir_all(root.join("store")).unwrap();
        let scanner_class = [(SCANNER_CLASS, SCANNER)];
        let extra: &[(&str, &str)] = if scanner { &scanner_class } else { &[] };
        for (class, source) in STORE_CLASSES.iter().chain(extra) {
            let file = root
                .join("store")
                .join(format!("{}.php", class.replace('\\', "_")));
            std::fs::write(&file, source).unwrap();
            classmap.push((class.to_string(), file));
        }
        let mut autoload = String::from("<?php\necho \"store autoload output\\n\";\n$map = [\n");
        for (class, file) in &classmap {
            autoload.push_str(&format!(
                "    {} => {},\n",
                php_string(&class.to_lowercase()),
                php_string(&file.to_string_lossy())
            ));
        }
        autoload.push_str("];\n$stubs = [\n");
        for stub in FIXTURE_STUBS {
            autoload.push_str(&format!("    {},\n", php_string(&stub.to_lowercase())));
        }
        autoload.push_str(
            r#"];
spl_autoload_register(static function (string $class) use ($map, $stubs): void {
    $key = strtolower(ltrim($class, '\\'));
    if (isset($map[$key])) {
        require_once $map[$key];
        return;
    }
    if (in_array($key, $stubs, true)) {
        $split = strrpos($class, '\\');
        eval(($split === false ? '' : 'namespace ' . substr($class, 0, $split) . '; ')
            . 'interface ' . substr($class, $split === false ? 0 : $split + 1) . ' {}');
    }
});
"#,
        );
        std::fs::create_dir_all(root.join("vendor")).unwrap();
        std::fs::write(root.join("vendor/autoload.php"), autoload).unwrap();
        let generated = root.join("generated/code");
        std::fs::create_dir_all(generated.join("Acme/Generated")).unwrap();
        std::fs::write(
            generated.join("Acme/Generated/Base.php"),
            "<?php\nnamespace Acme\\Generated;\nclass Base {}\n",
        )
        .unwrap();
        let mut config = Tier3Config::new("php", root, &generated);
        config.scratch = root.join("scratch");
        std::fs::create_dir_all(&config.scratch).unwrap();
        Store { dir, config }
    }

    fn file(&self, name: &str, source: &str) -> PathBuf {
        let path = self.dir.path().join("files").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, source).unwrap();
        path
    }

    fn reflect(&self, name: &str, source: &str) -> ExtractResult {
        reflect(&self.config, &self.file(name, source))
    }

    fn scratch_is_empty(&self) -> bool {
        std::fs::read_dir(&self.config.scratch)
            .unwrap()
            .next()
            .is_none()
    }
}

/// Fails with the version found when `php` is older than 8.4, the version these tests pin.
fn require_php_8_4() {
    static CHECKED: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    let checked = CHECKED.get_or_init(|| {
        let output = std::process::Command::new("php")
            .args(["-r", "echo PHP_VERSION_ID, ' ', PHP_VERSION;"])
            .output()
            .map_err(|e| format!("these tests need php 8.4 or later on the PATH: {e}"))?;
        let version = String::from_utf8_lossy(&output.stdout).into_owned();
        match version.split_once(' ') {
            Some((id, _)) if id.parse::<u32>().is_ok_and(|id| id >= 80400) => Ok(()),
            _ => Err(format!(
                "these tests need PHP 8.4 or later, found {}",
                version
                    .split_once(' ')
                    .map_or(version.as_str(), |(_, text)| text)
            )),
        }
    });
    if let Err(reason) = checked {
        panic!("{reason}");
    }
}

fn fixture_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "php"))
        .collect();
    files.sort();
    files
}

fn class(result: ExtractResult) -> ClassInfo {
    match result {
        ExtractResult::Ok(info) => info,
        other => panic!("expected a class, got {other:?}"),
    }
}

fn failure(result: ExtractResult) -> String {
    match result {
        ExtractResult::PhpFallbackFailed(reason) => reason,
        other => panic!("expected PhpFallbackFailed, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Comparison harness
// ---------------------------------------------------------------------------

/// Which tier reads a fixture today, and so what Tier 3 must match.
#[derive(Clone, Copy, Debug)]
enum Reference {
    Tier1,
    /// Tier 1 rejects the file; the chain's answer, from Tier 2.
    Tier2,
    NoClass,
}

const FIXTURE_TABLE: &[(&str, Reference)] = &[
    ("abstract_class.php", Reference::Tier1),
    ("comment_with_class.php", Reference::Tier1),
    ("complex_defaults.php", Reference::Tier1),
    ("constructor_promotion.php", Reference::Tier1),
    ("enum_class.php", Reference::NoClass),
    ("extends_implements.php", Reference::Tier1),
    ("final_class.php", Reference::Tier1),
    ("interface.php", Reference::Tier1),
    ("interface_extends.php", Reference::Tier1),
    ("intersection_type.php", Reference::Tier2),
    ("no_namespace.php", Reference::Tier1),
    ("nullable_union.php", Reference::Tier1),
    ("public_methods.php", Reference::Tier1),
    ("readonly_class.php", Reference::Tier1),
    ("simple_class.php", Reference::Tier1),
    ("string_with_keywords.php", Reference::Tier1),
    ("trait.php", Reference::Tier1),
    ("variadic.php", Reference::Tier1),
];

/// `info` after `apply_traits`, with traits read by Tier 1 from `files`.
fn with_traits(info: ClassInfo, files: &[PathBuf]) -> ClassInfo {
    let fqcn = info.fqcn.clone();
    let mut map = FxHashMap::default();
    map.insert(fqcn.clone(), info);
    apply_traits(&mut map, &|name| {
        files.iter().find_map(|file| match extract_file(file) {
            ExtractResult::Ok(found) if found.fqcn.eq_ignore_ascii_case(name) => Some(found),
            _ => None,
        })
    });
    map.remove(&fqcn).unwrap()
}

fn reference(path: &Path, expected: Reference) -> Option<ClassInfo> {
    match expected {
        Reference::Tier1 => Some(
            Lexer::extract(path)
                .unwrap_or_else(|e| panic!("Tier 1 no longer reads {}: {e}", path.display())),
        ),
        Reference::Tier2 => {
            assert!(
                Lexer::extract(path).is_err(),
                "Tier 1 reads {} now; update the table",
                path.display()
            );
            Some(class(extract_file(path)))
        }
        Reference::NoClass => {
            assert!(matches!(extract_file(path), ExtractResult::NoClass));
            None
        }
    }
}

/// The fields this harness compares so far.
#[derive(Debug, PartialEq)]
struct Identity {
    namespace: String,
    name: String,
    fqcn: String,
    kind: ClassKind,
}

fn identity(info: &ClassInfo) -> Identity {
    Identity {
        namespace: info.namespace.clone(),
        name: info.name.clone(),
        fqcn: info.fqcn.clone(),
        kind: info.kind.clone(),
    }
}

/// Defaults the reference keeps as source text the resolver cannot evaluate, by fixture and
/// parameter, with the value PHP gives them.
const DEFAULT_VALUES: &[(&str, &str, &str)] = &[
    ("complex_defaults.php", "name", "'default'"),
    ("complex_defaults.php", "timeout", "1800"),
];

/// The header of a class and its constructor, after the normalization of the comparison.
#[derive(Debug, PartialEq)]
struct Header {
    extends: Option<String>,
    implements: Vec<String>,
    is_abstract: bool,
    is_final: bool,
    constructor: Option<Vec<Param>>,
}

#[derive(Debug, PartialEq)]
struct Param {
    name: String,
    type_hint: Option<String>,
    optional_in_php: bool,
    /// What the resolver makes of the default, for a parameter PHP calls optional.
    default: Option<String>,
    is_primitive: bool,
    is_variadic: bool,
    is_promoted: bool,
}

const BUILTIN_TYPES: &[&str] = &[
    "int", "float", "string", "bool", "array", "callable", "iterable", "void", "null", "mixed",
    "never", "true", "false", "object", "static", "parent",
];

/// Builtin names in lowercase and `self` as the class name; class names as they are.
fn normalize_type(type_hint: &str, class: &str) -> String {
    let mut out = String::new();
    let mut name = String::new();
    let flush = |name: &mut String, out: &mut String| {
        let lower = name.to_ascii_lowercase();
        if lower == "self" {
            out.push_str(class);
        } else if BUILTIN_TYPES.contains(&lower.as_str()) {
            out.push_str(&lower);
        } else {
            out.push_str(name);
        }
        name.clear();
    };
    for c in type_hint.chars() {
        if matches!(c, '?' | '|' | '&' | '(' | ')') {
            flush(&mut name, &mut out);
            out.push(c);
        } else {
            name.push(c);
        }
    }
    flush(&mut name, &mut out);
    out
}

/// `reference` is whether `info` is the reference's reading, which the tables correct.
fn header_of(fixture: &str, info: &ClassInfo, reference: bool) -> Header {
    let const_map = FxHashMap::default();
    Header {
        extends: info.extends.clone(),
        implements: info.implements.clone(),
        is_abstract: info.is_abstract,
        is_final: info.is_final,
        constructor: info.constructor.as_ref().map(|ctor| {
            ctor.params
                .iter()
                .zip(optional_in_php(&ctor.params))
                .map(|(param, optional)| {
                    let listed = |table: &[(&str, &str, &'static str)]| {
                        table
                            .iter()
                            .find(|(file, name, _)| {
                                reference && *file == fixture && *name == param.name
                            })
                            .map(|(_, _, value)| *value)
                    };
                    Param {
                        name: param.name.clone(),
                        type_hint: param
                            .type_hint
                            .as_deref()
                            .map(|t| normalize_type(t, &info.fqcn)),
                        optional_in_php: optional,
                        default: optional.then(|| {
                            format!(
                                "{:?}",
                                di_resolver::arguments::resolve_non_object_default(
                                    listed(DEFAULT_VALUES).or(param.default_value.as_deref()),
                                    &const_map
                                )
                            )
                        }),
                        is_primitive: param.is_primitive,
                        is_variadic: param.is_variadic,
                        is_promoted: param.is_promoted,
                    }
                })
                .collect()
        }),
    }
}

/// The methods of a class, after the normalization of the comparison.
#[derive(Debug, PartialEq)]
struct Methods {
    public_methods: Vec<Method>,
    hidden_methods: Vec<String>,
    /// For a trait: every method, with public, final and abstract.
    trait_methods: Vec<(Method, bool, bool, bool)>,
}

#[derive(Debug, PartialEq)]
struct Method {
    name: String,
    params: Vec<MethodParamFields>,
    return_type: Option<String>,
    is_static: bool,
    returns_reference: bool,
}

#[derive(Debug, PartialEq)]
struct MethodParamFields {
    name: String,
    type_hint: Option<String>,
    optional_in_php: bool,
    default: Option<String>,
    is_variadic: bool,
    is_by_ref: bool,
}

fn method_of(signature: &MethodSignature, class: &str) -> Method {
    let const_map = FxHashMap::default();
    // `optional_in_php` reads a constructor parameter; a method parameter has the same rule.
    let as_constructor: Vec<ConstructorParam> = signature
        .params
        .iter()
        .map(|p| ConstructorParam {
            name: p.name.clone(),
            type_hint: None,
            is_optional: p.has_default,
            default_value: None,
            is_primitive: false,
            is_variadic: p.is_variadic,
            is_promoted: false,
        })
        .collect();
    Method {
        name: signature.name.clone(),
        params: signature
            .params
            .iter()
            .zip(optional_in_php(&as_constructor))
            .map(|(p, optional): (&MethodParam, bool)| MethodParamFields {
                name: p.name.clone(),
                type_hint: p.type_hint.as_deref().map(|t| normalize_type(t, class)),
                optional_in_php: optional,
                default: optional.then(|| {
                    format!(
                        "{:?}",
                        di_resolver::arguments::resolve_non_object_default(
                            p.default_value.as_deref(),
                            &const_map
                        )
                    )
                }),
                is_variadic: p.is_variadic,
                is_by_ref: p.is_by_ref,
            })
            .collect(),
        return_type: signature
            .return_type
            .as_deref()
            .map(|t| normalize_type(t, class)),
        is_static: signature.is_static,
        returns_reference: signature.returns_reference,
    }
}

fn methods_of(info: &ClassInfo) -> Methods {
    Methods {
        public_methods: info
            .public_methods
            .iter()
            .map(|m| method_of(m, &info.fqcn))
            .collect(),
        hidden_methods: info.traits.hidden_methods.clone(),
        trait_methods: info
            .traits
            .methods
            .iter()
            .map(|m| {
                (
                    method_of(&m.signature, &info.fqcn),
                    m.is_public,
                    m.is_final,
                    m.is_abstract,
                )
            })
            .collect(),
    }
}

/// Compares `fields` of Tier 3 and of the reference for every fixture. `fields` gets the
/// fixture's name and whether the class is the reference's.
fn compare_fixtures<T: PartialEq + std::fmt::Debug>(fields: impl Fn(&str, &ClassInfo, bool) -> T) {
    let listed: Vec<PathBuf> = FIXTURE_TABLE
        .iter()
        .map(|(name, _)| fixtures().join(name))
        .collect();
    assert_eq!(
        listed
            .iter()
            .map(|p| p.canonicalize().unwrap())
            .collect::<Vec<_>>(),
        fixture_files()
            .iter()
            .map(|p| p.canonicalize().unwrap())
            .collect::<Vec<_>>(),
        "every fixture needs a row in FIXTURE_TABLE"
    );
    let store = Store::new();
    let mut mismatches = Vec::new();
    for (name, expected) in FIXTURE_TABLE {
        let path = fixtures().join(name);
        let want = reference(&path, *expected).map(|info| with_traits(info, &fixture_files()));
        let got = match reflect(&store.config, &path) {
            ExtractResult::Ok(info) => ExtractResult::Ok(with_traits(info, &[])),
            other => other,
        };
        match (want, got) {
            (None, ExtractResult::NoClass) => {}
            (Some(want), ExtractResult::Ok(got)) => {
                let (want, got) = (fields(name, &want, true), fields(name, &got, false));
                if want != got {
                    mismatches.push(format!("{name}:\n  want {want:?}\n  got  {got:?}"));
                }
            }
            (want, got) => mismatches.push(format!(
                "{name}: want {}, got {got:?}",
                if want.is_some() { "a class" } else { "NoClass" }
            )),
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    assert!(store.scratch_is_empty());
}

#[test]
fn every_fixture_has_the_identity_of_its_reference() {
    compare_fixtures(|_, info, _| identity(info));
}

#[test]
fn every_fixture_has_the_header_and_constructor_of_its_reference() {
    compare_fixtures(header_of);
}

#[test]
fn every_fixture_has_the_methods_of_its_reference() {
    compare_fixtures(|_, info, _| methods_of(info));
}

// ---------------------------------------------------------------------------
// Header and constructor
// ---------------------------------------------------------------------------

fn params(info: &ClassInfo) -> &[php_extractor::ConstructorParam] {
    &info.constructor.as_ref().expect("a constructor").params
}

#[test]
fn explicit_interfaces_stay_in_source_order_when_the_parent_has_them() {
    let store = Store::new();
    store.file(
        "Base.php",
        "<?php\nnamespace Acme\\Order;\ninterface A {}\ninterface B {}\nclass P implements A {}\n",
    );
    let info = class(store.reflect(
        "X.php",
        &format!(
            "<?php\nnamespace Acme\\Order;\nrequire_once {};\nclass X extends P implements B, A {{}}\n",
            php_string(&store.dir.path().join("files/Base.php").to_string_lossy())
        ),
    ));
    assert_eq!(info.extends.as_deref(), Some("Acme\\Order\\P"));
    assert_eq!(info.implements, ["Acme\\Order\\B", "Acme\\Order\\A"]);
}

#[test]
fn class_flags_follow_the_declaration() {
    let store = Store::new();
    let abstract_class = class(store.reflect(
        "Shape.php",
        "<?php\nnamespace Acme;\nabstract class Shape { abstract public function area(): float; }\n",
    ));
    assert_eq!(abstract_class.kind, ClassKind::AbstractClass);
    assert!(abstract_class.is_abstract);
    assert!(!abstract_class.is_final);

    let value = class(store.reflect(
        "Money.php",
        "<?php\nnamespace Acme;\nfinal readonly class Money { public function __construct(public int $amount) {} }\n",
    ));
    assert_eq!(value.kind, ClassKind::Class);
    assert!(value.is_final);
    assert!(!value.is_abstract);
    assert!(params(&value)[0].is_promoted);

    for (name, source) in [
        (
            "AbstractReadonly.php",
            "<?php\nnamespace Acme;\nabstract readonly class AbstractReadonly {}\n",
        ),
        (
            "ReadonlyAbstract.php",
            "<?php\nnamespace Acme;\nreadonly abstract class ReadonlyAbstract {}\n",
        ),
    ] {
        let path = store.file(name, source);
        let info = class(reflect(&store.config, &path));
        assert_eq!(info.kind, ClassKind::AbstractClass, "{name}");
        assert!(info.is_abstract, "{name}");
        let tier1 = Lexer::extract(&path).unwrap();
        assert_eq!(
            (tier1.kind, tier1.is_abstract),
            (info.kind, info.is_abstract),
            "{name}: Tier 1"
        );
    }
}

#[test]
fn an_interface_extending_two_interfaces_is_read_as_tier1() {
    let store = Store::new();
    store.file(
        "Parents.php",
        "<?php\nnamespace Acme\\Two;\ninterface Readable {}\ninterface Writable {}\n",
    );
    let path = store.file(
        "Stream.php",
        &format!(
            "<?php\nnamespace Acme\\Two;\nrequire_once {};\ninterface Stream extends Readable, \\Countable {{}}\n",
            php_string(&store.dir.path().join("files/Parents.php").to_string_lossy())
        ),
    );
    let tier3 = class(reflect(&store.config, &path));
    assert_eq!(tier3.kind, ClassKind::Interface);
    assert_eq!(tier3.extends.as_deref(), Some("Acme\\Two\\Readable"));
    assert_eq!(tier3.implements, ["Countable"]);
    let tier1 = Lexer::extract(&path).unwrap();
    assert_eq!(
        (tier3.extends, tier3.implements),
        (tier1.extends, tier1.implements)
    );
}

fn contracts(store: &Store) -> String {
    store.file(
        "Contracts.php",
        "<?php\nnamespace Acme\\Contracts;\ninterface First {}\ninterface Second {}\ninterface Third {}\nclass Base {}\n",
    );
    php_string(
        &store
            .dir
            .path()
            .join("files/Contracts.php")
            .to_string_lossy(),
    )
}

#[test]
fn header_names_resolve_through_imports_as_tier1() {
    let store = Store::new();
    let contracts = contracts(&store);
    let path = store.file(
        "Imports.php",
        &format!(
            "<?php\nnamespace Acme\\Shop;\nuse Acme\\Contracts\\{{First, Second as Two}};\nuse Acme\\Contracts\\Base as Root;\nuse Acme\\Contracts;\nuse function strlen;\nrequire_once {contracts};\n$closure = function () use ($contracts) {{}};\nclass Cart extends Root implements First, Two, Contracts\\Third {{}}\n"
        ),
    );
    let tier3 = class(reflect(&store.config, &path));
    assert_eq!(tier3.extends.as_deref(), Some("Acme\\Contracts\\Base"));
    assert_eq!(
        tier3.implements,
        [
            "Acme\\Contracts\\First",
            "Acme\\Contracts\\Second",
            "Acme\\Contracts\\Third"
        ]
    );
    let tier1 = Lexer::extract(&path).unwrap();
    assert_eq!(
        (tier3.extends, tier3.implements),
        (tier1.extends, tier1.implements)
    );
}

#[test]
fn every_import_of_a_comma_list_counts() {
    let store = Store::new();
    let contracts = contracts(&store);
    let path = store.file(
        "ImportList.php",
        &format!(
            "<?php\nnamespace Acme\\Shop;\nuse Acme\\Contracts\\Base as Root, Acme\\Contracts;\nrequire_once {contracts};\nclass Order extends Root implements Contracts\\Third {{}}\n"
        ),
    );
    let info = class(reflect(&store.config, &path));
    assert_eq!(info.extends.as_deref(), Some("Acme\\Contracts\\Base"));
    assert_eq!(info.implements, ["Acme\\Contracts\\Third"]);
    let tier1 = Lexer::extract(&path).unwrap();
    assert_eq!(
        (info.extends, info.implements),
        (tier1.extends, tier1.implements)
    );
}

#[test]
fn a_relative_name_resolves_in_the_namespace() {
    let store = Store::new();
    store.file(
        "Local.php",
        "<?php\nnamespace Acme\\Local;\ninterface Marker {}\n",
    );
    let path = store.file(
        "Relative.php",
        &format!(
            "<?php\nnamespace Acme\\Local;\nrequire_once {};\nclass Relative implements namespace\\Marker {{}}\n",
            php_string(&store.dir.path().join("files/Local.php").to_string_lossy())
        ),
    );
    let info = class(reflect(&store.config, &path));
    assert_eq!(info.implements, ["Acme\\Local\\Marker"]);
    assert_eq!(info.implements, Lexer::extract(&path).unwrap().implements);
}

#[test]
fn a_header_reflection_does_not_confirm_fails() {
    let store = Store::new();
    // An alias: the source names `Alias`, PHP has the interface as `Acme\Real`.
    let reason = failure(store.reflect(
        "Aliased.php",
        "<?php\nnamespace Acme;\neval('namespace Acme; interface Real {}');\nclass_alias('Acme\\\\Real', 'Acme\\\\Alias');\nclass Aliased implements Alias {}\n",
    ));
    assert!(reason.contains("does not match reflection"), "{reason}");
    assert!(reason.contains("Acme\\Alias"), "{reason}");
}

#[test]
fn a_constructor_counts_only_when_the_class_or_its_traits_declare_it() {
    let store = Store::new();
    store.file(
        "Parts.php",
        "<?php\nnamespace Acme\\Ctor;\nclass Parent_ { public function __construct(int $size) {} }\ntrait Builds { public function __construct(string $name = 'x') {} }\n",
    );
    let parts = php_string(&store.dir.path().join("files/Parts.php").to_string_lossy());
    let inherited = class(store.reflect(
        "Child.php",
        &format!("<?php\nnamespace Acme\\Ctor;\nrequire_once {parts};\nclass Child extends Parent_ {{}}\n"),
    ));
    assert_eq!(inherited.constructor, None);
    let from_trait = class(store.reflect(
        "Built.php",
        &format!("<?php\nnamespace Acme\\Ctor;\nrequire_once {parts};\nclass Built extends Parent_ {{ use Builds; }}\n"),
    ));
    let params = params(&from_trait);
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "name");
    assert_eq!(params[0].default_value.as_deref(), Some("'x'"));
}

#[test]
fn constructor_parameters_as_reflection_reports_them() {
    let store = Store::new();
    let class_file = store.file(
        "ParamsClass.php",
        "<?php\nnamespace Acme\\Params;\nuse Acme\\Data\\Item;\nclass Params {\n    const SIZE = 10;\n    public function __construct(\n        $first = 1,\n        Item $item,\n        protected int $size = self::SIZE,\n        (\\Countable&\\Traversable)|null $both = null,\n        ?Item $maybe = null,\n        string ...$rest\n    ) {}\n}\n",
    );
    let info = class(reflect(&store.config, &class_file));
    let params = params(&info);
    let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["first", "item", "size", "both", "maybe", "rest"]);

    // A default before a required parameter: PHP reports none, Tier 1 keeps the source's.
    assert!(!params[0].is_optional);
    assert_eq!(params[0].default_value, None);
    let tier1 = Lexer::extract(&class_file).unwrap();
    let tier1_params = &tier1.constructor.as_ref().unwrap().params;
    assert!(tier1_params[0].is_optional);
    assert_eq!(optional_in_php(params), optional_in_php(tier1_params));

    assert_eq!(params[1].type_hint.as_deref(), Some("Acme\\Data\\Item"));
    assert!(!params[1].is_primitive);

    // A promoted parameter with a constant default: PHP evaluates the constant.
    assert!(params[2].is_promoted);
    assert_eq!(params[2].type_hint.as_deref(), Some("int"));
    assert_eq!(params[2].default_value.as_deref(), Some("10"));
    assert!(params[2].is_primitive);

    assert_eq!(
        params[3].type_hint.as_deref(),
        Some("(Countable&Traversable)|null")
    );
    assert!(!params[3].is_primitive);
    assert_eq!(params[3].default_value.as_deref(), Some("NULL"));

    assert_eq!(params[4].type_hint.as_deref(), Some("?Acme\\Data\\Item"));

    // A variadic parameter has no default and is not optional in the source's sense.
    assert!(params[5].is_variadic);
    assert!(!params[5].is_optional);
    assert_eq!(params[5].default_value, None);
}

#[test]
fn a_default_php_cannot_evaluate_fails() {
    let store = Store::new();
    let reason = failure(store.reflect(
        "Undefined.php",
        "<?php\nnamespace Acme;\nclass Undefined { public function __construct(int $size = \\ACME_UNDEFINED_SIZE) {} }\n",
    ));
    assert!(
        reason.contains("constructor of Acme\\Undefined"),
        "{reason}"
    );
    assert!(reason.contains("ACME_UNDEFINED_SIZE"), "{reason}");
}

// ---------------------------------------------------------------------------
// Methods and traits
// ---------------------------------------------------------------------------

/// Two traits in files of their own, so Tier 1 can read each of them.
fn trait_files(store: &Store) -> Vec<PathBuf> {
    vec![
        store.file(
            "Greets.php",
            "<?php\nnamespace Acme\\Mix;\ntrait Greets {\n    public function hello(): string { return ''; }\n    final public function bye(): void {}\n    protected function secret(self $other) {}\n    abstract public function name(): string;\n}\n",
        ),
        store.file(
            "Waves.php",
            "<?php\nnamespace Acme\\Mix;\ntrait Waves {\n    public function hello(): int { return 1; }\n    public function wave(?string $how = null, int ...$times): static { return $this; }\n}\n",
        ),
    ]
}

fn requires(files: &[PathBuf]) -> String {
    files
        .iter()
        .map(|file| format!("require_once {};\n", php_string(&file.to_string_lossy())))
        .collect()
}

#[test]
fn trait_rules_give_the_methods_of_tier1_after_apply_traits() {
    let store = Store::new();
    let traits = trait_files(&store);
    let path = store.file(
        "Person.php",
        &format!(
            "<?php\nnamespace Acme\\Mix;\n{}class Person {{\n    use Greets, Waves {{\n        Greets::hello insteadof Waves;\n        Waves::hello as waveHello;\n        wave as protected;\n        secret as public reveal;\n    }}\n    public function name(): string {{ return 'p'; }}\n    private function mine() {{}}\n}}\n",
            requires(&traits)
        ),
    );
    let tier3 = class(reflect(&store.config, &path));
    let tier1 = with_traits(Lexer::extract(&path).unwrap(), &traits);
    assert_eq!(methods_of(&tier3), methods_of(&tier1));
    assert!(tier3.traits.uses.is_empty() && tier3.traits.rules.is_empty());

    let public: Vec<&str> = tier3
        .public_methods
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    // The class's own methods first, then each trait's, an alias before its method.
    assert_eq!(public, ["name", "hello", "reveal", "waveHello"]);
    let hello = &tier3.public_methods[1];
    assert_eq!(hello.return_type.as_deref(), Some("string"));
    assert_eq!(
        tier3.traits.hidden_methods,
        ["mine", "bye", "secret", "wave"]
    );
}

#[test]
fn a_trait_lists_every_method_with_its_flags() {
    let store = Store::new();
    let traits = trait_files(&store);
    let tier3 = class(reflect(&store.config, &traits[0]));
    assert_eq!(tier3.kind, ClassKind::Trait);
    let flags: Vec<(&str, bool, bool, bool)> = tier3
        .traits
        .methods
        .iter()
        .map(|m| {
            (
                m.signature.name.as_str(),
                m.is_public,
                m.is_final,
                m.is_abstract,
            )
        })
        .collect();
    assert_eq!(
        flags,
        [
            ("hello", true, false, false),
            ("bye", true, true, false),
            ("secret", false, false, false),
            ("name", true, false, true),
        ]
    );
    let tier1 = Lexer::extract(&traits[0]).unwrap();
    assert_eq!(methods_of(&tier3), methods_of(&tier1));
}

#[test]
fn an_abstract_constructor_of_a_trait_is_no_constructor() {
    let store = Store::new();
    let path = store.file(
        "NeedsCtor.php",
        "<?php\nnamespace Acme;\ntrait NeedsCtor { abstract public function __construct(int $size); }\n",
    );
    let tier3 = class(reflect(&store.config, &path));
    assert_eq!(tier3.constructor, None);
    assert!(tier3.traits.methods.is_empty());
    assert_eq!(
        tier3.constructor,
        Lexer::extract(&path).unwrap().constructor
    );
}

#[test]
fn inherited_methods_belong_to_the_parent() {
    let store = Store::new();
    let parent = store.file(
        "Elder.php",
        "<?php\nnamespace Acme\\Family;\nclass Elder { public function old() {} protected function older() {} }\n",
    );
    let info = class(store.reflect(
        "Younger.php",
        &format!(
            "<?php\nnamespace Acme\\Family;\n{}class Younger extends Elder {{ public function young() {{}} }}\n",
            requires(&[parent])
        ),
    ));
    let public: Vec<&str> = info
        .public_methods
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    assert_eq!(public, ["young"]);
    assert!(info.traits.hidden_methods.is_empty());
}

#[test]
fn method_parameters_compare_with_tier1_after_normalization() {
    let store = Store::new();
    let path = store.file(
        "Calls.php",
        "<?php\nnamespace Acme;\nuse Acme\\Data\\Item;\nclass Calls {\n    const LIMIT = 5;\n    public function run($first = 1, string $second, int ...$rest): ?self { return null; }\n    public function &many(array $list = [], ?Item $item = null, &$out = null, int $limit = self::LIMIT): array { return $list; }\n    public static function make(): static { return new static(); }\n}\n",
    );
    let tier3 = class(reflect(&store.config, &path));
    let mut tier1 = Lexer::extract(&path).unwrap();
    // The resolver cannot evaluate Tier 1's `self::LIMIT`; PHP gives its value.
    tier1.public_methods[1].params[3].default_value = Some("5".into());
    assert_eq!(methods_of(&tier3), methods_of(&tier1));

    let run = &tier3.public_methods[0].params;
    // PHP reports no default before a required parameter; Tier 1 keeps the source's.
    assert!(!run[0].has_default);
    assert!(tier1.public_methods[0].params[0].has_default);
    assert!(run[2].is_variadic && !run[2].has_default);

    let many = &tier3.public_methods[1];
    assert!(many.returns_reference);
    assert_eq!(many.params[0].default_value.as_deref(), Some("__json__:[]"));
    assert!(many.params[2].is_by_ref);
    assert_eq!(many.params[3].default_value.as_deref(), Some("5"));
    assert!(tier3.public_methods[2].is_static);
}

#[test]
fn a_method_default_php_cannot_evaluate_fails() {
    let store = Store::new();
    let reason = failure(store.reflect(
        "Broken.php",
        "<?php\nnamespace Acme;\nclass Broken { public function run(int $size = \\ACME_UNDEFINED_LIMIT) {} }\n",
    ));
    assert!(reason.contains("methods of Acme\\Broken"), "{reason}");
}

// ---------------------------------------------------------------------------
// Scan scope
// ---------------------------------------------------------------------------

fn inside_block(store: &Store, name: &str, source: &str) -> bool {
    let path = store.file(name, source);
    match reflect_with_facts(&store.config, &path) {
        (ExtractResult::Ok(_), facts) => {
            assert!(!facts.class_check);
            facts.inside_block
        }
        (other, _) => panic!("{name}: expected a class, got {other:?}"),
    }
}

#[test]
fn a_declaration_inside_a_block_is_reported_as_tier1_does() {
    let store = Store::new();
    for (name, source, expected) in [
        ("Top.php", "<?php\nnamespace Acme;\nclass Top {}\n", false),
        (
            "Braced.php",
            "<?php\nnamespace Acme\\Braced {\n    class Braced {}\n}\n",
            false,
        ),
        (
            "Guarded.php",
            "<?php\nnamespace Acme;\nif (true) {\n    class Guarded {}\n}\n",
            true,
        ),
        (
            "GuardedInterface.php",
            "<?php\nnamespace Acme;\nif (!interface_exists('Acme\\\\GuardedInterface', false)) {\n    interface GuardedInterface {}\n}\n",
            true,
        ),
        (
            "AfterFunction.php",
            "<?php\nnamespace Acme;\nfunction helper() { return function () {}; }\nclass AfterFunction {}\n",
            false,
        ),
    ] {
        assert_eq!(inside_block(&store, name, source), expected, "{name}");
        let tier1 = Lexer::extract_with_facts(&store.file(name, source)).unwrap().1;
        assert_eq!(tier1.inside_block, expected, "{name}: Tier 1");
    }
}

#[test]
fn magento_s_scanner_must_agree_for_classes_and_traits() {
    let store = Store::with_scanner();
    assert!(!inside_block(
        &store,
        "Seen.php",
        "<?php\n// scanner: Acme\\Seen\nnamespace Acme;\nclass Seen {}\n"
    ));
    assert!(inside_block(
        &store,
        "Hidden.php",
        "<?php\nnamespace Acme;\nif (true) {\n    class Hidden {}\n}\n"
    ));
    // The scanner reads no interfaces, so it is not asked for one.
    assert!(!inside_block(
        &store,
        "Asked.php",
        "<?php\nnamespace Acme;\ninterface Asked {}\n"
    ));
    let reason = failure(store.reflect(
        "Disagrees.php",
        "<?php\nnamespace Acme;\nclass Disagrees {}\n",
    ));
    assert!(
        reason.contains("FileClassScanner gives no class"),
        "{reason}"
    );
    assert!(reason.contains("at the top level"), "{reason}");
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

#[test]
fn missing_php_binary_fails_with_the_binary() {
    let mut store = Store::new();
    store.config.php = "/nonexistent/php".into();
    let reason = failure(store.reflect("Plain.php", "<?php\nclass Plain {}\n"));
    assert!(
        reason.starts_with("Could not start /nonexistent/php"),
        "{reason}"
    );
    assert!(store.scratch_is_empty());
}

#[test]
fn fatal_error_while_loading_is_the_reason() {
    let store = Store::new();
    let reason = failure(store.reflect(
        "Redeclares.php",
        "<?php\nfunction strlen() {}\nclass Redeclares {}\n",
    ));
    assert!(reason.contains("Cannot redeclare"), "{reason}");
    assert!(reason.contains("Redeclares.php"), "{reason}");
}

#[test]
fn missing_parent_is_the_reason() {
    let store = Store::new();
    let reason = failure(store.reflect(
        "Orphan.php",
        "<?php\nnamespace Acme;\nclass Orphan extends \\Acme\\Missing\\Base {}\n",
    ));
    assert!(reason.contains("Acme\\Missing\\Base"), "{reason}");
    assert!(reason.contains("not found"), "{reason}");
}

#[test]
fn enum_and_anonymous_only_files_have_no_class() {
    let store = Store::new();
    assert!(matches!(
        store.reflect(
            "Suit.php",
            "<?php\nnamespace Acme;\nenum Suit { case Hearts; }\n"
        ),
        ExtractResult::NoClass
    ));
    assert!(matches!(
        store.reflect("Anonymous.php", "<?php\nreturn new class {};\n"),
        ExtractResult::NoClass
    ));
}

#[test]
fn first_declaration_is_the_class_of_the_file() {
    let store = Store::new();
    let info = class(store.reflect(
        "Two.php",
        "<?php\nnamespace Acme;\ninterface First {}\nclass Second implements First {}\n",
    ));
    assert_eq!(info.fqcn, "Acme\\First");
    assert_eq!(info.kind, ClassKind::Interface);
}

#[test]
fn declarations_on_one_line_are_taken_in_source_order_as_tier1() {
    let store = Store::new();
    for (name, source) in [
        (
            "OneLine.php",
            "<?php\nnamespace Acme;\ninterface First {} class Second implements First {}\n",
        ),
        (
            "OneLineReversed.php",
            "<?php\nnamespace Acme;\nclass Second {} interface First {}\n",
        ),
        (
            "OneLineTrait.php",
            "<?php\nnamespace Acme;\ntrait Mixin {} final class User { use Mixin; }\n",
        ),
    ] {
        let path = store.file(name, source);
        let tier1 = Lexer::extract(&path).unwrap();
        let tier3 = class(reflect(&store.config, &path));
        assert_eq!(identity(&tier3), identity(&tier1), "{name}");
    }
}

#[test]
fn class_constants_and_anonymous_classes_are_no_declarations() {
    let store = Store::new();
    let info = class(store.reflect(
        "Real.php",
        "<?php\nnamespace Acme\\Deep;\n$name = \\stdClass::class; $object = new class {}; $other = new readonly class {};\nfinal class Real {}\n",
    ));
    assert_eq!(info.fqcn, "Acme\\Deep\\Real");
}

#[test]
fn a_declaration_php_did_not_run_fails() {
    let store = Store::new();
    let reason = failure(store.reflect(
        "Never.php",
        "<?php\nnamespace Acme;\nif (false) { class Skipped {} }\nclass Later {}\n",
    ));
    assert!(reason.contains("Acme\\Skipped"), "{reason}");
    assert!(reason.contains("PHP did not declare it"), "{reason}");
}

/// A stand-in PHP binary: writes `json` as the result (argument 7) and exits with `code`.
fn fake_php(store: &Store, name: &str, json: Option<&str>, code: i32) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = store.dir.path().join(name);
    let write = json.map_or(String::new(), |json| {
        format!("printf '%s' '{json}' > \"$7\"\n")
    });
    std::fs::write(&path, format!("#!/bin/sh\n{write}exit {code}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn a_result_counts_only_with_exit_code_zero() {
    let mut store = Store::new();
    let file = store.file("Any.php", "<?php\nclass Any {}\n");
    let class_json = r#"{"result":"class","kind":"class","namespace":"","name":"Any","fqcn":"Any","is_final":false,"extends":null,"implements":[],"constructor":null,"methods":[],"inside_block":false}"#;

    store.config.php = fake_php(
        &store,
        "no-class-exit-7",
        Some(r#"{"result":"no_class"}"#),
        7,
    );
    let reason = failure(reflect(&store.config, &file));
    assert!(reason.contains("code 7"), "{reason}");

    store.config.php = fake_php(&store, "class-exit-1", Some(class_json), 1);
    let reason = failure(reflect(&store.config, &file));
    assert!(reason.contains("code 1"), "{reason}");

    store.config.php = fake_php(&store, "nothing-exit-0", None, 0);
    let reason = failure(reflect(&store.config, &file));
    assert!(
        reason.contains("code 0 without a usable result"),
        "{reason}"
    );

    // A fatal error exits 255 and its reason comes from the result.
    store.config.php = fake_php(
        &store,
        "fatal-exit-255",
        Some(r#"{"result":"error","message":"There is an error in Any.php at line: 2: boom"}"#),
        255,
    );
    assert_eq!(
        failure(reflect(&store.config, &file)),
        "There is an error in Any.php at line: 2: boom"
    );

    store.config.php = fake_php(&store, "class-exit-0", Some(class_json), 0);
    assert_eq!(class(reflect(&store.config, &file)).fqcn, "Any");
}

#[test]
fn folders_already_in_scratch_are_left_alone() {
    let store = Store::new();
    // Names the first runner used, and one any program could have made.
    let mut existing = Vec::new();
    for run in 0..512 {
        existing.push(format!("fast-di-tier3-{}-{run}", std::process::id()));
    }
    existing.push("fast-di-tier3-shared".to_string());
    for name in &existing {
        let folder = store.config.scratch.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("sentinel"), name).unwrap();
    }
    for run in 0..4 {
        class(store.reflect(
            &format!("Kept{run}.php"),
            &format!("<?php\nclass Kept{run} {{}}\n"),
        ));
    }
    let mut left: Vec<String> = std::fs::read_dir(&store.config.scratch)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    existing.sort();
    assert_eq!(left, existing);
    for name in &existing {
        assert_eq!(
            std::fs::read_to_string(store.config.scratch.join(name).join("sentinel")).unwrap(),
            *name
        );
    }
}

#[test]
fn output_of_the_store_and_the_class_file_is_not_the_result() {
    // The store's autoloader prints as well (`Store::new`).
    let store = Store::new();
    let info = class(store.reflect(
        "Noisy.php",
        "<?php\necho \"before\\n\";\nclass Noisy {}\necho '{\"result\":\"no_class\"}';\n",
    ));
    assert_eq!(info.fqcn, "Noisy");
}

#[test]
fn generated_classes_load_from_generated_code_and_as_stand_ins() {
    let store = Store::new();
    let from_disk = class(store.reflect(
        "UsesGenerated.php",
        "<?php\nnamespace Acme;\nclass UsesGenerated extends \\Acme\\Generated\\Base {}\n",
    ));
    assert_eq!(from_disk.fqcn, "Acme\\UsesGenerated");
    let stand_in = class(store.reflect(
        "UsesExtension.php",
        "<?php\nnamespace Acme;\nclass UsesExtension implements \\Acme\\Data\\ItemExtensionInterface {}\n",
    ));
    assert_eq!(stand_in.fqcn, "Acme\\UsesExtension");
}

#[test]
fn object_manager_is_available_while_the_class_file_loads() {
    let store = Store::new();
    let info = class(store.reflect(
        "UsesObjectManager.php",
        "<?php\nnamespace Acme;\n\\Magento\\Framework\\App\\ObjectManager::getInstance()->get('Acme\\Thing');\nclass UsesObjectManager {}\n",
    ));
    assert_eq!(info.fqcn, "Acme\\UsesObjectManager");
}

#[test]
fn a_run_past_the_timeout_is_stopped_and_cleaned_up() {
    let mut store = Store::new();
    store.config.timeout = Duration::from_secs(1);
    let reason = failure(store.reflect("Slow.php", "<?php\nsleep(30);\nclass Slow {}\n"));
    assert!(reason.starts_with("timeout"), "{reason}");
    assert!(store.scratch_is_empty());
    #[cfg(target_os = "linux")]
    {
        let scratch = store.config.scratch.to_string_lossy().into_owned();
        let still_running: Vec<String> = std::fs::read_dir("/proc")
            .unwrap()
            .filter_map(|entry| std::fs::read(entry.ok()?.path().join("cmdline")).ok())
            .map(|cmdline| String::from_utf8_lossy(&cmdline).replace('\0', " "))
            .filter(|cmdline| cmdline.contains(&scratch))
            .collect();
        assert!(still_running.is_empty(), "{still_running:?}");
    }
}
