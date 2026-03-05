//! Fixture snapshot tests (TKT-009).
//!
//! Each test extracts a PHP fixture file and compares against an insta snapshot.
//! Run `cargo test` then `cargo insta review` to accept new snapshots.
//! Run with `INSTA_UPDATE=always` to regenerate all snapshots.

use php_extractor::{extract_file, ExtractResult};
use std::path::Path;

fn fixture(name: &str) -> std::path::PathBuf {
    // Path relative to workspace root
    let manifest = env!("CARGO_MANIFEST_DIR");
    let root = Path::new(manifest)
        .parent() // crates/php-extractor → crates
        .unwrap()
        .parent() // crates → di-compiler root
        .unwrap();
    root.join("tests/fixtures").join(name)
}

fn extract_ok(name: &str) -> php_extractor::ClassInfo {
    match extract_file(&fixture(name)) {
        ExtractResult::Ok(info) => info,
        other => panic!("Expected Ok for {name}, got: {other:?}"),
    }
}

fn assert_no_class(name: &str) {
    match extract_file(&fixture(name)) {
        ExtractResult::NoClass => {}
        other => panic!("Expected NoClass for {name}, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Basic structural tests (no insta — deterministic)
// ---------------------------------------------------------------------------

#[test]
fn fixture_simple_class_fqcn() {
    let info = extract_ok("simple_class.php");
    assert_eq!(info.fqcn, "Magento\\Framework\\App\\Application");
    assert_eq!(info.namespace, "Magento\\Framework\\App");
    assert_eq!(info.name, "Application");
    assert!(!info.is_abstract);
    assert!(!info.is_final);
}

#[test]
fn fixture_simple_class_constructor_params() {
    let info = extract_ok("simple_class.php");
    let ctor = info.constructor.unwrap();
    assert_eq!(ctor.params.len(), 2);
    assert_eq!(
        ctor.params[0].type_hint.as_deref(),
        Some("Magento\\Framework\\AppInterface")
    );
    assert_eq!(
        ctor.params[1].type_hint.as_deref(),
        Some("Magento\\Framework\\App\\State")
    );
}

#[test]
fn fixture_abstract_class() {
    let info = extract_ok("abstract_class.php");
    assert!(info.is_abstract);
    assert_eq!(info.fqcn, "Magento\\Framework\\App\\Action\\AbstractAction");
    let ctor = info.constructor.unwrap();
    assert_eq!(ctor.params.len(), 2);
    assert!(ctor.params[0].is_promoted);
    assert!(ctor.params[1].is_promoted);
}

#[test]
fn fixture_interface() {
    let info = extract_ok("interface.php");
    assert!(matches!(info.kind, php_extractor::ClassKind::Interface));
    assert_eq!(info.fqcn, "Magento\\Framework\\App\\ActionInterface");
    assert!(info.constructor.is_none());
}

#[test]
fn fixture_trait() {
    let info = extract_ok("trait.php");
    assert!(matches!(info.kind, php_extractor::ClassKind::Trait));
    assert_eq!(info.fqcn, "Magento\\Framework\\DataObject\\IdentityTrait");
}

#[test]
fn fixture_enum_is_no_class() {
    assert_no_class("enum_class.php");
}

#[test]
fn fixture_constructor_promotion() {
    let info = extract_ok("constructor_promotion.php");
    let ctor = info.constructor.unwrap();
    assert_eq!(ctor.params.len(), 4);
    assert!(ctor.params[0].is_promoted); // public readonly
    assert!(ctor.params[1].is_promoted); // protected
    assert!(ctor.params[2].is_promoted); // private string
    assert!(ctor.params[3].is_promoted); // public int
    assert!(ctor.params[2].is_optional); // has default 'default'
    assert_eq!(ctor.params[2].type_hint.as_deref(), Some("string"));
    assert!(ctor.params[2].is_primitive);
}

#[test]
fn fixture_nullable_union() {
    let info = extract_ok("nullable_union.php");
    let ctor = info.constructor.unwrap();
    assert_eq!(ctor.params.len(), 3);
    // ?RequestInterface preserved as nullable type
    assert!(ctor.params[0].is_optional);
    assert_eq!(
        ctor.params[0].type_hint.as_deref(),
        Some("?Magento\\Framework\\App\\RequestInterface")
    );
    // string|int|null preserved
    assert_eq!(ctor.params[1].type_hint.as_deref(), Some("string|int|null"));
    assert!(ctor.params[1].is_optional);
}

#[test]
fn fixture_variadic() {
    let info = extract_ok("variadic.php");
    let ctor = info.constructor.unwrap();
    assert_eq!(ctor.params.len(), 3);
    assert!(ctor.params[2].is_variadic);
    assert_eq!(
        ctor.params[2].type_hint.as_deref(),
        Some("Magento\\Framework\\Logger\\Handler")
    );
}

#[test]
fn fixture_intersection_type_extracted_by_tier2() {
    // Tier 1 rejects, Tier 2 extracts
    let info = extract_ok("intersection_type.php");
    assert_eq!(info.fqcn, "Magento\\Framework\\IntersectionExample");
    let ctor = info.constructor.unwrap();
    assert_eq!(ctor.params.len(), 1);
    assert_eq!(ctor.params[0].name, "collection");
}

#[test]
fn fixture_final_class() {
    let info = extract_ok("final_class.php");
    assert!(info.is_final);
    assert_eq!(info.fqcn, "Magento\\Framework\\App\\Registry");
}

#[test]
fn fixture_no_namespace() {
    let info = extract_ok("no_namespace.php");
    assert_eq!(info.namespace, "");
    assert_eq!(info.fqcn, "LegacyClass");
}

#[test]
fn fixture_extends_implements() {
    let info = extract_ok("extends_implements.php");
    assert_eq!(
        info.extends.as_deref(),
        Some("Magento\\Framework\\DataObject")
    );
    assert_eq!(info.implements.len(), 2);
    assert!(info
        .implements
        .iter()
        .any(|i| i.contains("ObjectRelationInterface")));
    assert!(info
        .implements
        .iter()
        .any(|i| i.contains("IdentityInterface")));
}

#[test]
fn fixture_public_methods() {
    let info = extract_ok("public_methods.php");
    let method_names: Vec<&str> = info
        .public_methods
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    // getData and processRequest and static create should be included
    assert!(
        method_names.contains(&"getData"),
        "missing getData: {method_names:?}"
    );
    assert!(
        method_names.contains(&"create"),
        "missing create: {method_names:?}"
    );
    assert!(
        method_names.contains(&"processRequest"),
        "missing processRequest: {method_names:?}"
    );
    // final getVersion should be excluded
    assert!(
        !method_names.contains(&"getVersion"),
        "getVersion should be excluded"
    );
    // private/protected excluded
    assert!(!method_names.contains(&"internalMethod"));
    assert!(!method_names.contains(&"protectedMethod"));
}

#[test]
fn fixture_comment_with_class_keyword() {
    let info = extract_ok("comment_with_class.php");
    assert_eq!(info.name, "CommentTest");
    assert_eq!(info.fqcn, "Magento\\Framework\\CommentTest");
}

#[test]
fn fixture_readonly_class() {
    let info = extract_ok("readonly_class.php");
    assert_eq!(info.fqcn, "Magento\\Framework\\ValueObject");
}

#[test]
fn fixture_string_with_keywords() {
    let info = extract_ok("string_with_keywords.php");
    assert_eq!(info.name, "StringEdgeCase");
}

#[test]
fn fixture_complex_defaults() {
    let info = extract_ok("complex_defaults.php");
    let ctor = info.constructor.unwrap();
    // All 5 params should be extracted
    assert_eq!(ctor.params.len(), 5);
    assert_eq!(ctor.params[0].name, "service");
    assert_eq!(ctor.params[1].name, "config");
    assert!(ctor.params[1].is_optional); // has default
    assert_eq!(ctor.params[2].name, "name");
    assert!(ctor.params[3].is_optional); // = null
    assert_eq!(ctor.params[4].name, "timeout");
    assert!(ctor.params[4].is_optional);
}

#[test]
fn fixture_interface_extends() {
    let info = extract_ok("interface_extends.php");
    assert!(matches!(info.kind, php_extractor::ClassKind::Interface));
    assert_eq!(info.fqcn, "Magento\\Framework\\Data\\CollectionInterface");
    // extends multiple → additional ones go into implements
    assert!(!info.implements.is_empty());
}
