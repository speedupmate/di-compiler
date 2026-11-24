//! PHP treats a parameter with a default as required when a required parameter follows it,
//! `Type $param = null` included. `ClassReader::getConstructor()` reads that from reflection, so
//! `ArgumentsResolver` gives such a parameter an instance and not its default. It skips a
//! configured `xsi:type="null"` argument, because `isset()` is false for the null it interprets to.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn global_metadata_for(constructor: &str) -> Option<String> {
    global_metadata_with_di_xml(constructor, None)
}

fn global_metadata_with_di_xml(constructor: &str, di_xml: Option<&str>) -> Option<String> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return None;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/etc/config.php",
        "<?php\nreturn [\n    'modules' => [\n        'Acme_First' => 1,\n    ],\n];\n",
    );
    write(root, "app/code/Acme/First/registration.php", "<?php\n");
    write(
        root,
        "app/code/Acme/First/etc/module.xml",
        "<config><module name=\"Acme_First\"/></config>\n",
    );
    write(
        root,
        "vendor/autoload.php",
        "<?php\nspl_autoload_register(function ($class) {\n    \
         $file = dirname(__DIR__) . '/app/code/' . str_replace('\\\\', '/', $class) . '.php';\n    \
         if (is_file($file)) {\n        require $file;\n    }\n});\n",
    );
    for name in ["Gadget", "Widget"] {
        write(
            root,
            &format!("app/code/Acme/First/Model/{name}.php"),
            &format!("<?php\nnamespace Acme\\First\\Model;\n\nclass {name}\n{{\n}}\n"),
        );
    }
    write(
        root,
        "app/code/Acme/First/Model/Thing.php",
        &format!(
            "<?php\nnamespace Acme\\First\\Model;\n\nclass Thing\n{{\n    public function __construct({constructor})\n    {{\n    }}\n}}\n"
        ),
    );
    if let Some(di_xml) = di_xml {
        write(root, "app/code/Acme/First/etc/di.xml", di_xml);
    }
    let output = Command::new(env!("CARGO_BIN_EXE_fast-di-compile"))
        .arg("--magento-root")
        .arg(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(fs::read_to_string(root.join("generated/metadata/global.php")).unwrap())
}

fn thing_arguments(metadata: &str) -> String {
    let start = metadata
        .find("'Acme\\\\First\\\\Model\\\\Thing' => ")
        .expect("Thing has an arguments entry");
    let rest = &metadata[start..];
    let end = rest[1..].find("\n    '").map_or(rest.len(), |end| end + 1);
    rest[..end].to_string()
}

#[test]
fn implicitly_nullable_parameter_before_a_required_one_gets_an_instance() {
    let Some(metadata) = global_metadata_for("Gadget $gadget = null, Widget $widget") else {
        return;
    };
    let arguments = thing_arguments(&metadata);
    assert!(
        arguments.contains("'_i_' => 'Acme\\\\First\\\\Model\\\\Gadget'"),
        "{arguments}"
    );
}

#[test]
fn nullable_parameter_before_a_required_one_gets_an_instance() {
    let Some(metadata) = global_metadata_for("?Gadget $gadget = null, Widget $widget") else {
        return;
    };
    let arguments = thing_arguments(&metadata);
    assert!(
        arguments.contains("'_i_' => 'Acme\\\\First\\\\Model\\\\Gadget'"),
        "{arguments}"
    );
}

#[test]
fn nullable_parameter_before_only_optional_ones_keeps_its_default() {
    let Some(metadata) =
        global_metadata_for("?Gadget $gadget = null, ?Widget $widget = null, ...$rest")
    else {
        return;
    };
    let arguments = thing_arguments(&metadata);
    assert!(
        !arguments.contains("'_i_' => 'Acme\\\\First\\\\Model\\\\Gadget'"),
        "{arguments}"
    );
    assert!(arguments.contains("'_vn_' => true"), "{arguments}");
}

#[test]
fn nullable_parameter_without_a_default_gets_an_instance() {
    let Some(metadata) = global_metadata_for(
        "?Gadget $gadget, // @deprecated\n        private readonly Widget $widget,\n        array $data = [],\n        private ?Widget $other = null",
    ) else {
        return;
    };
    let arguments = thing_arguments(&metadata);
    assert!(
        arguments.contains("'_i_' => 'Acme\\\\First\\\\Model\\\\Gadget'"),
        "{arguments}"
    );
}

const NULL_GADGET_DI_XML: &str = "<config xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
    <type name=\"Acme\\First\\Model\\Thing\"><arguments>\
    <argument name=\"gadget\" xsi:type=\"null\"/>\
    </arguments></type></config>\n";

#[test]
fn configured_null_for_a_required_parameter_gives_an_instance() {
    let Some(metadata) =
        global_metadata_with_di_xml("?Gadget $gadget, Widget $widget", Some(NULL_GADGET_DI_XML))
    else {
        return;
    };
    let arguments = thing_arguments(&metadata);
    assert!(
        arguments.contains("'_i_' => 'Acme\\\\First\\\\Model\\\\Gadget'"),
        "{arguments}"
    );
}

#[test]
fn configured_null_for_an_optional_parameter_gives_its_default() {
    let Some(metadata) =
        global_metadata_with_di_xml("Widget $widget, $gadget = 5", Some(NULL_GADGET_DI_XML))
    else {
        return;
    };
    let arguments = thing_arguments(&metadata);
    assert!(arguments.contains("'_v_' => 5"), "{arguments}");
}
