---
id: TKT-019
title: Proxy PHP code generator
phase: 04-code-generator
feature: proxy-codegen
owner: Unassigned
status: Ready
estimate: M
depends_on: [TKT-014]
touches:
  - rust/di-compiler/crates/code-generator/src/proxy.rs
acceptance:
  - Generated Proxy.php files match PHP PROXY_GENERATOR output byte-for-byte
  - __sleep/__wakeup included
  - All public methods proxied
---

# TKT-019: Proxy Code Generator

## Scope

Generate `*Proxy.php` files from `ProxySpec`.

## Implementation Notes

```rust
pub fn generate_proxy(spec: &ProxySpec, class_info: &ClassInfo) -> String
```

Compare against 3+ sample Proxy files from Phase 00 ground truth before implementing.
Template is more complex than Factory — includes `__sleep`, `__wakeup`, `_getSubject()`,
and a wrapper per public method that delegates to the subject.

Output path: `generated/code/{FQN path}/Proxy.php`.
