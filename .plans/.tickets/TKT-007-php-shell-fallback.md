---
id: TKT-007
title: PHP shell Tier 3 fallback extractor
phase: 01-php-extractor
feature: php-fallback-tier3
owner: Unassigned
status: Done
estimate: S
depends_on: [TKT-001]
touches:
  - rust/di-compiler/crates/php-extractor/src/tier3_php.rs
acceptance:
  - Returns valid ClassInfo via PHP reflection for a class Tier 1+2 fail on
  - Logs warning with file path when invoked
  - Returns PhpFallbackFailed if PHP not available or exits non-zero
---

# TKT-007: PHP Shell Tier 3 Fallback

## Scope

Shell out to `php -r` to use reflection as last resort.

## Implementation Notes

Write a PHP script as a string constant that:
1. Requires the target file
2. Uses `ReflectionClass` on the first declared class
3. Outputs JSON to stdout: `{ fqcn, namespace, name, kind, extends, implements, constructor, public_methods }`

```rust
pub fn extract_with_php(path: &Path) -> Result<Option<ClassInfo>, ExtractError> {
    let script = PHP_REFLECTION_SCRIPT.replace("__FILE__", &path.to_string_lossy());
    let output = std::process::Command::new("php")
        .args(["-r", &script])
        .output()
        .map_err(|_| ExtractError::PhpNotAvailable)?;
    if !output.status.success() {
        return Err(ExtractError::PhpFailed(String::from_utf8_lossy(&output.stderr).into()));
    }
    let info: Option<ClassInfo> = serde_json::from_slice(&output.stdout)?;
    Ok(info)
}
```

## Risks

- PHP script must handle all class kinds (enums → return null, not error)
- Slow: each invocation starts a new PHP process — acceptable for < 0.5% of files
