//! Tier 3 against a small store on disk: loading context, runner failures, and the comparison
//! with the tier that reads each fixture today.

use std::path::{Path, PathBuf};
use std::time::Duration;

use php_extractor::lexer::Lexer;
use php_extractor::tier3::{reflect, Tier3Config};
use php_extractor::{extract_file, ClassInfo, ClassKind, ExtractResult};
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

struct Store {
    dir: TempDir,
    config: Tier3Config,
}

impl Store {
    /// A store whose autoloader knows every fixture class, the stubs and `STORE_CLASSES`, and
    /// prints while it loads, as some stores do.
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let mut classmap: Vec<(String, PathBuf)> = Vec::new();
        for fixture in fixture_files() {
            if let ExtractResult::Ok(info) = extract_file(&fixture) {
                classmap.push((info.fqcn, fixture));
            }
        }
        std::fs::create_dir_all(root.join("store")).unwrap();
        for (class, source) in STORE_CLASSES {
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

fn compare_fixtures<T: PartialEq + std::fmt::Debug>(fields: impl Fn(&ClassInfo) -> T) {
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
        let want = reference(&path, *expected);
        let got = reflect(&store.config, &path);
        match (want, got) {
            (None, ExtractResult::NoClass) => {}
            (Some(want), ExtractResult::Ok(got)) => {
                if fields(&want) != fields(&got) {
                    mismatches.push(format!(
                        "{name}:\n  want {:?}\n  got  {:?}",
                        fields(&want),
                        fields(&got)
                    ));
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
    compare_fixtures(identity);
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
    let class_json =
        r#"{"result":"class","kind":"class","namespace":"","name":"Any","fqcn":"Any"}"#;

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
