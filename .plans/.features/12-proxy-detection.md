# 12: Proxy Detection

- Category: Resolution
- Status: Planned
- Implementation Phase: 03-di-resolver
- Owner: Unassigned
- Feature ID: `proxy-detection`
- Suggested Dependencies: 06-extract-result-type, 09-di-config-model

## Intent

Determine which Proxy classes need to be generated.

## Detection Rules

A Proxy needs generating if:
- A constructor param type hint ends in `\Proxy` (e.g. `Some\Class\Proxy`)
- OR a `di.xml` `<argument xsi:type="object">` value ends in `\Proxy`
- AND that Proxy class FQN does not already exist in `class_map`

## Core State and Actions

```rust
pub fn detect_proxies(
    class_map: &HashMap<String, ClassInfo>,
    config: &DiConfig,
) -> Vec<ProxySpec>

pub struct ProxySpec {
    pub target_fqcn: String,   // the class being proxied (strip \Proxy suffix)
    pub proxy_fqcn: String,    // the full FQN of the proxy class to generate
}
```

## Acceptance Criteria

- Proxy list matches PHP PROXY_GENERATOR output
- Already-existing Proxy classes not re-generated
- Both constructor-param and di.xml-argument sources detected
