//! Tier 2 reads a file as Tier 1 does. Tier 2 only gets the files Tier 1 rejects, so these tests
//! give it files both can read and compare every field.

use std::path::{Path, PathBuf};

use php_extractor::lexer::Lexer;
use php_extractor::tier2::extract_tier2;
use php_extractor::{ClassInfo, ExtractResult};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn tier2(path: &Path) -> ClassInfo {
    match extract_tier2(path) {
        ExtractResult::Ok(info) => info,
        other => panic!("Tier 2 does not read {}: {other:?}", path.display()),
    }
}

/// Every field but the path, field by field, so a failure names the field.
fn differences(tier1: &ClassInfo, tier2: &ClassInfo) -> Vec<String> {
    let mut out = Vec::new();
    macro_rules! compare {
        ($($field:ident),*) => {$(
            if tier1.$field != tier2.$field {
                out.push(format!(
                    "{}:\n    tier 1 {:?}\n    tier 2 {:?}",
                    stringify!($field),
                    tier1.$field,
                    tier2.$field
                ));
            }
        )*};
    }
    compare!(
        namespace,
        name,
        fqcn,
        kind,
        extends,
        implements,
        is_abstract,
        is_final,
        constructor,
        public_methods,
        traits
    );
    out
}

fn assert_same(path: &Path) {
    let tier1 = Lexer::extract(path).unwrap();
    let found = differences(&tier1, &tier2(path));
    assert!(
        found.is_empty(),
        "{}\n  {}",
        path.display(),
        found.join("\n  ")
    );
}

#[test]
fn every_fixture_tier1_reads_is_read_the_same_by_tier2() {
    let mut files: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "php"))
        .collect();
    files.sort();
    let mut compared = 0;
    for file in files {
        if Lexer::extract(&file).is_ok() {
            assert_same(&file);
            compared += 1;
        }
    }
    assert!(compared >= 15, "only {compared} fixtures compared");
}

/// Forms the fixtures do not have, all of which Tier 1 reads.
const SOURCES: &[(&str, &str)] = &[
    (
        "Names.php",
        "<?php\nnamespace Acme\\Shop;\n\nuse Lib\\Thing;\nuse Lib\\Other as O, Lib\\Contracts;\nuse Lib\\Group\\{First, Second as Two};\n\n\
         class Names extends \\Lib\\Base implements Contracts\\Marker, \\Countable, namespace\\Local\n{\n    \
         public function __construct(\n        \\Foo\\Bar $absolute,\n        Thing $imported,\n        O $aliased,\n        \
         Local $relative,\n        namespace\\Sub\\Here $keyword,\n        First $grouped,\n        Two $groupAlias,\n        \
         ?\\Foo\\Bar $nullable = null,\n        \\Foo\\Bar | /* or */ Thing | null $union = null,\n        \
         (\\Countable&Thing)|null $dnf = null,\n        INT $builtin = 1\n    ) {\n    }\n\n    \
         public function run(\\Foo\\Bar $a, ?Thing $b = null): \\Foo\\Bar|Thing\n    {\n    }\n\n    \
         public function same(): ?self\n    {\n    }\n\n    \
         public static function make(): static\n    {\n    }\n}\n",
    ),
    (
        "ImportForms.php",
        "<?php\nnamespace Acme\\Shop;\n\nuse Lib\\Other AS O;\nuse Lib\\Thing /* the thing */ as T;\nuse \\Lib\\Group\\{Inner As I};\n\n\
         class ImportForms\n{\n    \
         public function __construct(O $other, T $thing, I $inner)\n    {\n    }\n\n    \
         public function run(): O\n    {\n    }\n}\n",
    ),
    (
        "Parents.php",
        "<?php\nnamespace Acme;\n\ninterface Parents extends \\Countable, Local, \\IteratorAggregate\n{\n    \
         public function items(): array;\n}\n",
    ),
    (
        "AbstractReadonly.php",
        "<?php\nnamespace Acme;\n\nabstract readonly class AbstractReadonly implements \\Stringable\n{\n    \
         public function __toString(): string\n    {\n        return '';\n    }\n}\n",
    ),
    (
        "ReadonlyAbstract.php",
        "<?php\nnamespace Acme;\n\nreadonly abstract class ReadonlyAbstract\n{\n}\n",
    ),
    (
        "FinalReadonly.php",
        "<?php\nnamespace Acme;\n\nfinal readonly class FinalReadonly extends Base\n{\n}\n",
    ),
    (
        "UsesTraits.php",
        // tree-sitter-php 0.22 sees a syntax error in a fully qualified name inside the rules,
        // and such a file goes on to Tier 3.
        "<?php\nnamespace Acme;\n\nuse Lib\\Greets;\nuse Lib\\Waves;\n\nclass UsesTraits\n{\n    \
         use Greets, Waves, \\Lib\\Third {\n        Greets::hello insteadof Waves;\n        Waves::hello as protected wave;\n    }\n}\n",
    ),
];

#[test]
fn names_types_headers_and_modifiers_are_read_the_same_by_tier2() {
    let dir = tempfile::TempDir::new().unwrap();
    for (name, source) in SOURCES {
        let path = dir.path().join(name);
        std::fs::write(&path, source).unwrap();
        assert_same(&path);
    }
}

#[test]
fn intersection_types_resolve_member_by_member() {
    // Tier 1 rejects intersections, so Tier 2 is the reference here.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("Both.php");
    std::fs::write(
        &path,
        "<?php\nnamespace Acme\\Shop;\n\nuse Lib\\Thing;\n\nclass Both\n{\n    \
         public function __construct(\\Countable&\\Iterator $plain, Thing & Local $spaced)\n    {\n    }\n\n    \
         public function run(\\Countable&Thing $items): \\Countable&Thing\n    {\n    }\n}\n",
    )
    .unwrap();
    assert!(Lexer::extract(&path).is_err(), "Tier 1 reads it now");
    let info = tier2(&path);
    let types: Vec<Option<&str>> = info
        .constructor
        .as_ref()
        .unwrap()
        .params
        .iter()
        .map(|p| p.type_hint.as_deref())
        .collect();
    assert_eq!(
        types,
        [
            Some("Countable&Iterator"),
            Some("Lib\\Thing&Acme\\Shop\\Local")
        ]
    );
    let run = &info.public_methods[0];
    assert_eq!(
        run.params[0].type_hint.as_deref(),
        Some("Countable&Lib\\Thing")
    );
    assert_eq!(run.return_type.as_deref(), Some("Countable&Lib\\Thing"));
}

#[test]
fn a_reference_parameter_with_an_intersection_type_is_by_reference() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("ByReference.php");
    std::fs::write(
        &path,
        "<?php\nnamespace Acme;\n\nclass ByReference\n{\n    \
         public function take(\\Countable&\\Iterator & $items, \\Countable&\\Iterator $plain, int &$count, &...$rest): void\n    {\n    }\n}\n",
    )
    .unwrap();
    let info = tier2(&path);
    let by_ref: Vec<bool> = info.public_methods[0]
        .params
        .iter()
        .map(|p| p.is_by_ref)
        .collect();
    assert_eq!(by_ref, [true, false, true, true]);
}

#[test]
fn imports_are_read_whatever_the_keyword_case_or_comments() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("ImportForms.php");
    let source = SOURCES
        .iter()
        .find(|(name, _)| *name == "ImportForms.php")
        .unwrap()
        .1;
    std::fs::write(&path, source).unwrap();
    let info = tier2(&path);
    let types: Vec<Option<&str>> = info
        .constructor
        .as_ref()
        .unwrap()
        .params
        .iter()
        .map(|p| p.type_hint.as_deref())
        .collect();
    assert_eq!(
        types,
        [
            Some("Lib\\Other"),
            Some("Lib\\Thing"),
            Some("Lib\\Group\\Inner")
        ]
    );
    assert_eq!(
        info.public_methods[0].return_type.as_deref(),
        Some("Lib\\Other")
    );
}
