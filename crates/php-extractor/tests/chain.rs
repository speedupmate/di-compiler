//! The extraction chain with Tier 3 configured: a file Tiers 1 and 2 cannot read is read by PHP,
//! with its scope.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use php_extractor::tier3::{configure, Tier3Config};
use php_extractor::{extract_file_with_class_check, ExtractResult};

/// Property hooks and `I&J` parameters: Tier 1 rejects the intersection, and tree-sitter-php
/// does not know property hooks.
const HOOKED: &str = "class Hooked implements \\Countable {\n    public string $label { get => 'x'; }\n    public function __construct(private A&B $both, int $size = 3) {}\n    public function count(): int { return 0; }\n}";

/// Configures Tier 3 once for this test binary, with a store that only has an autoloader. The
/// store lives in Cargo's folder for test files, so it outlives the tests that use it.
fn store() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let version = Command::new("php")
            .args(["-r", "echo PHP_VERSION_ID, ' ', PHP_VERSION;"])
            .output()
            .expect("these tests need php 8.4 or later on the PATH");
        let version = String::from_utf8_lossy(&version.stdout).into_owned();
        let (id, text) = version.split_once(' ').unwrap();
        assert!(
            id.parse::<u32>().unwrap() >= 80400,
            "these tests need PHP 8.4 or later, found {text}"
        );
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("tier3-chain");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("vendor")).unwrap();
        std::fs::write(root.join("vendor/autoload.php"), "<?php\n").unwrap();
        configure(Tier3Config::new("php", &root, &root.join("generated/code")))
            .expect("configured once");
        root
    })
}

fn file(name: &str, source: &str) -> PathBuf {
    let path = store().join("files").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, source).unwrap();
    path
}

#[test]
fn a_file_tiers_1_and_2_cannot_read_is_read_by_php() {
    let path = file("Hooked.php", &format!("<?php\nnamespace Acme;\n{HOOKED}\n"));
    let (result, facts) = extract_file_with_class_check(&path);
    let ExtractResult::Ok(info) = result else {
        panic!("expected a class, got {result:?}");
    };
    assert_eq!(info.fqcn, "Acme\\Hooked");
    assert_eq!(info.implements, ["Countable"]);
    let params = &info.constructor.as_ref().unwrap().params;
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].type_hint.as_deref(), Some("Acme\\A&Acme\\B"));
    assert!(params[0].is_promoted);
    assert_eq!(params[1].default_value.as_deref(), Some("3"));
    let methods: Vec<&str> = info
        .public_methods
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    assert_eq!(methods, ["count"]);
    assert!(!facts.inside_block);
}

#[test]
fn the_same_class_inside_if_is_read_and_reported_inside_a_block() {
    let path = file(
        "GuardedHooked.php",
        &format!("<?php\nnamespace Acme\\Guarded;\nif (true) {{\n{HOOKED}\n}}\n"),
    );
    let (result, facts) = extract_file_with_class_check(&path);
    let ExtractResult::Ok(info) = result else {
        panic!("expected a class, got {result:?}");
    };
    assert_eq!(info.fqcn, "Acme\\Guarded\\Hooked");
    assert!(facts.inside_block);
}
