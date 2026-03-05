# 05: PHP Extractor — Tier 3 (PHP Shell Fallback)

- Category: Extraction
- Status: Planned
- Implementation Phase: 01-php-extractor
- Owner: Unassigned
- Feature ID: `php-fallback-tier3`
- Suggested Dependencies: 01-workspace-scaffold

## Intent

Last-resort extraction by shelling out to `php -r` with reflection.
Used only when Tiers 1 and 2 both fail. Enabled via `--fallback-php` CLI flag (default: on).
Logs a warning for every file that reaches this tier.

## Core State and Actions

```rust
pub fn extract_with_php(path: &Path) -> Result<Option<ClassInfo>, ExtractError> {
    let script = format!(
        "require '{}'; /* reflection logic → JSON to stdout */",
        path.display()
    );
    let output = std::process::Command::new("php")
        .arg("-r")
        .arg(&script)
        .output()?;
    // parse JSON from stdout into ClassInfo
    todo!()
}
```

PHP script outputs JSON:
```json
{
  "fqcn": "Magento\\Catalog\\Model\\Product",
  "namespace": "Magento\\Catalog\\Model",
  "name": "Product",
  "kind": "class",
  "extends": null,
  "implements": [],
  "constructor": { "params": [...] },
  "public_methods": [...]
}
```

## Acceptance Criteria

- Only invoked when `--fallback-php` is set (default: on)
- Logs a warning with file path and reason when invoked
- If PHP is not available or exits non-zero → `ExtractResult::PhpFallbackFailed`
- Target: < 0.5% of files reach this tier
