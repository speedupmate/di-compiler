---
id: TKT-054
title: Bootstrap const_map from PHP runtime get_defined_constants
phase: 08-parity-closure
feature: 36-di-merge-order-and-null-surface
owner: Unassigned
status: Done
estimate: S
depends_on: []
touches:
  - rust/di-compiler/crates/cli/src/main.rs
acceptance:
  - MCRYPT_BLOWFISH resolves to "blowfish" in area config output
  - MCRYPT_MODE_ECB resolves to "ecb"
  - All PHP extension constants (openssl, curl, intl, etc.) are available for resolution without hardcoding
  - Falls back gracefully (empty map) if PHP is not available or returns error
test_plan:
  - Unit test: bootstrap_php_constants with a mock php binary that returns known JSON → correct HashMap
  - Integration: compile; check Magento\Framework\Encryption\Adapter\Mcrypt args in global.php
---

# TKT-054: Bootstrap const_map from PHP runtime constants

## Scope

PHP extension constants (`MCRYPT_BLOWFISH` → `"blowfish"`, `MCRYPT_MODE_ECB` → `"ecb"`, etc.) are not defined in any Magento PHP file, so they never enter the `const_map` built by source scanning. They're emitted verbatim as literal strings in area config output, causing 4 known mismatches.

## Implementation Notes

Add a `bootstrap_php_constants` function called once at startup before source-scan constants are merged in. Source-scan constants added after this call win on name collision.

```rust
fn bootstrap_php_constants(php_bin: &str) -> HashMap<String, String> {
    let script = concat!(
        "$c=get_defined_constants(true);",
        "$o=[];",
        "foreach($c as $items){",
        "  foreach($items as $k=>$v){",
        "    if(is_scalar($v)) $o[$k]=(string)$v;",
        "  }",
        "}",
        "echo json_encode($o);"
    );
    let output = std::process::Command::new(php_bin)
        .args(["-r", script])
        .output();
    let Ok(out) = output else { return HashMap::new() };
    if !out.status.success() { return HashMap::new() }
    serde_json::from_slice(&out.stdout).unwrap_or_default()
}

// In main(), before source const_map is built:
let mut const_map = bootstrap_php_constants(&args.fallback_php);
// Source-scan constants override builtins:
const_map.extend(source_const_map);
```

The PHP script dumps ALL scalar-valued constants from all categories (`user`, `Core`, `pcre`, `mcrypt`, `openssl`, `curl`, etc.) as a JSON object. This is a one-time ~50ms call at startup with a warm PHP process. The existing `PhpWorkerPool` is for per-class reflection; this is a separate one-shot call.

## Risks

- If `php_bin` path is wrong or PHP is not installed, we fall back silently to an empty map — const_map has no PHP constants but does not crash.
- Very large constant tables could slow JSON decode; in practice `get_defined_constants` returns ~2,000 entries and decodes in < 1ms.
