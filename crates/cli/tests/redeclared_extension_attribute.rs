//! Magento merges every `extension_attributes.xml` into one DOM in module load order.
//! `Magento\Framework\Config\Dom::_mergeNode()` finds an attribute that a second module
//! declares again by its `code`, then `_mergeAttributes()` copies the new `type` onto the
//! node that is already there. The attribute keeps the place of its first declaration and
//! takes the type of the last one. `ExtensionAttributesGenerator` writes the methods in
//! that DOM order.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn write_module(root: &Path, directory: &str, module: &str, attributes: &str) {
    write(root, &format!("{directory}/registration.php"), "<?php\n");
    write(
        root,
        &format!("{directory}/etc/module.xml"),
        &format!("<config><module name=\"{module}\"/></config>\n"),
    );
    write(
        root,
        &format!("{directory}/etc/extension_attributes.xml"),
        &format!(
            "<config>\n    <extension_attributes for=\"Acme\\Base\\Api\\Data\\ThingInterface\">\n{attributes}    </extension_attributes>\n</config>\n"
        ),
    );
}

fn method_positions(source: &str, names: &[&str]) -> Vec<usize> {
    names
        .iter()
        .map(|name| {
            source
                .find(&format!("function {name}("))
                .unwrap_or_else(|| panic!("{name} missing in:\n{source}"))
        })
        .collect()
}

#[test]
fn redeclared_extension_attribute_keeps_its_first_place_and_takes_the_last_type() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_Base' => 1,\n        'Acme_Second' => 1,\n    ],\n];\n",
    );
    write_module(
        root,
        "app/code/Acme/Base",
        "Acme_Base",
        "        <attribute code=\"first_code\" type=\"string\"/>\n        <attribute code=\"shared_ids\" type=\"int[]\"/>\n        <attribute code=\"last_code\" type=\"string\"/>\n",
    );
    write_module(
        root,
        "app/code/Acme/Second",
        "Acme_Second",
        "        <attribute code=\"shared_ids\" type=\"mixed\"/>\n        <attribute code=\"added_code\" type=\"string\"/>\n",
    );
    write(
        root,
        "app/code/Acme/Base/Api/Data/ThingInterface.php",
        "<?php\nnamespace Acme\\Base\\Api\\Data;\n\ninterface ThingInterface\n{\n}\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        // No PHP on purpose, so this test only depends on the static scan.
        .arg("--fallback-php")
        .arg(root.join("no-php-here"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let data = root.join("generated/code/Acme/Base/Api/Data");
    let expected = [
        "getFirstCode",
        "getSharedIds",
        "getLastCode",
        "getAddedCode",
    ];
    for file in ["ThingExtension.php", "ThingExtensionInterface.php"] {
        let source = fs::read_to_string(data.join(file)).unwrap();
        let positions = method_positions(&source, &expected);
        let mut sorted = positions.clone();
        sorted.sort();
        assert_eq!(positions, sorted, "method order in {file}:\n{source}");
        assert_eq!(
            source.matches("function getSharedIds(").count(),
            1,
            "{file}"
        );
        assert!(
            source.contains("@return mixed|null"),
            "{file} must take the last declared type:\n{source}"
        );
        assert!(!source.contains("int[]"), "{file}:\n{source}");
    }
}
