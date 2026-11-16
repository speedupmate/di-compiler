//! `PluginList` only reads a compiled plugin list when the file has the name the store's own
//! `PluginListGenerator` gives it. A composer patch can change that name, so the framework version
//! says nothing about it. The compiler has to use the name the store's own class produces.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// A `PluginListGenerator` written for this test, with the same collaborators as Magento's. The
/// arguments are PHP code: how to name the file, and what to do before writing it.
fn generator(cache_id: &str, before_write: &str) -> String {
    format!(
        r#"<?php
declare(strict_types=1);

namespace Magento\Framework\Interception;

class PluginListGenerator
{{
    private $directoryList;
    private \Acme\Unknown\ScopeInterface $scopeConfig;
    private string $cacheId = 'plugin-list';
    private array $pluginData = [];
    private array $scopePriorityScheme = ['global'];

    public function __construct(
        private readonly \Acme\Unknown\ReaderInterface $reader
    ) {{
    }}

    public function write(array $scopes): void
    {{
        foreach ($scopes as $scope) {{
            $this->scopeConfig->setCurrentScope($scope);
            $this->reader->read($scope);
            array_keys($this->pluginData);
            $this->scopePriorityScheme[] = $scope;
            $scheme = array_values(array_unique($this->scopePriorityScheme));
            {before_write}
            $cacheId = {cache_id};
            file_put_contents($this->directoryList->getPath('metadata') . '/' . $cacheId . '.php', '<?php return [];');
            if (count($this->scopePriorityScheme) > 2) {{
                array_pop($this->scopePriorityScheme);
            }}
        }}
    }}
}}
"#
    )
}

/// Compiles a small store and returns the sorted names of its plugin list files.
fn plugin_list_files(generator_php: Option<&str>) -> Option<Vec<String>> {
    if Command::new("php").arg("-v").output().is_err() {
        eprintln!("skipped: these tests need php on the PATH");
        return None;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "app/etc/config.php",
        "<?php return ['modules' => ['Acme_On' => 1]];\n",
    );
    write(root, "app/code/Acme/On/registration.php", "<?php\n");
    write(
        root,
        "app/code/Acme/On/etc/module.xml",
        "<config><module name=\"Acme_On\"/></config>\n",
    );
    if let Some(php) = generator_php {
        write(
            root,
            "vendor/magento/framework/Interception/PluginListGenerator.php",
            php,
        );
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
    let mut names: Vec<String> = fs::read_dir(root.join("generated/metadata"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("plugin-list"))
        .collect();
    names.sort();
    Some(names)
}

fn expect(names: &[&str]) -> Option<Vec<String>> {
    let mut names: Vec<String> = names.iter().map(|name| name.to_string()).collect();
    names.sort();
    Some(names)
}

#[test]
fn generator_that_sorts_the_scopes_decides_the_names() {
    let php = generator(
        "implode('|', $sorted) . '|' . $this->cacheId",
        "$sorted = $scheme; sort($sorted);",
    );
    let names = plugin_list_files(Some(&php));
    if names.is_none() {
        return;
    }
    assert_eq!(
        names,
        expect(&[
            "adminhtml|global|primary|plugin-list.php",
            "crontab|global|primary|plugin-list.php",
            "frontend|global|primary|plugin-list.php",
            "global|graphql|primary|plugin-list.php",
            "global|primary|plugin-list.php",
            "global|primary|webapi_rest|plugin-list.php",
            "global|primary|webapi_soap|plugin-list.php",
        ])
    );
}

#[test]
fn generator_with_another_separator_decides_the_names() {
    let php = generator("implode('-', $scheme) . '-' . $this->cacheId", "");
    let names = plugin_list_files(Some(&php));
    if names.is_none() {
        return;
    }
    assert_eq!(
        names,
        expect(&[
            "primary-global-adminhtml-plugin-list.php",
            "primary-global-crontab-plugin-list.php",
            "primary-global-frontend-plugin-list.php",
            "primary-global-graphql-plugin-list.php",
            "primary-global-plugin-list.php",
            "primary-global-webapi_rest-plugin-list.php",
            "primary-global-webapi_soap-plugin-list.php",
        ])
    );
}

fn default_names() -> Option<Vec<String>> {
    expect(&[
        "primary|global|adminhtml|plugin-list.php",
        "primary|global|crontab|plugin-list.php",
        "primary|global|frontend|plugin-list.php",
        "primary|global|graphql|plugin-list.php",
        "primary|global|plugin-list.php",
        "primary|global|webapi_rest|plugin-list.php",
        "primary|global|webapi_soap|plugin-list.php",
    ])
}

#[test]
fn generator_without_a_patch_keeps_the_default_names() {
    let php = generator("implode('|', $scheme) . '|' . $this->cacheId", "");
    let names = plugin_list_files(Some(&php));
    if names.is_none() {
        return;
    }
    assert_eq!(names, default_names());
}

#[test]
fn store_without_a_generator_class_gets_the_default_names() {
    let names = plugin_list_files(None);
    if names.is_none() {
        return;
    }
    assert_eq!(names, default_names());
}

#[test]
fn generator_that_cannot_answer_gets_the_default_names() {
    let php = generator(
        "implode('|', $scheme) . '|' . $this->cacheId",
        "throw new \\RuntimeException('no answer');",
    );
    let names = plugin_list_files(Some(&php));
    if names.is_none() {
        return;
    }
    assert_eq!(names, default_names());
}
