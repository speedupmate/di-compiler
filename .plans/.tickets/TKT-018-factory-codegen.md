---
id: TKT-018
title: Factory PHP code generator
phase: 04-code-generator
feature: factory-codegen
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-014]
touches:
  - rust/di-compiler/crates/code-generator/src/factory.rs
acceptance:
  - Generated Factory.php files match PHP APPLICATION_CODE_GENERATOR output byte-for-byte
---

# TKT-018: Factory Code Generator

## Scope

Generate `*Factory.php` files from `FactorySpec`.

## Implementation Notes

```rust
pub fn generate_factory(spec: &FactorySpec) -> String
```

Template from feature spec 15. Compare against a sample Factory from Phase 00 ground truth.

Output path: `generated/code/{FQN path}/Factory.php` (NOT `{FQN}Factory.php` — the Factory suffix is part of the class name, not added to the path).

## Risks

- Template must match PHP output exactly — examine 3+ real generated factories in Phase 00
