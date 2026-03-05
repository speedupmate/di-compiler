---
id: TKT-017
title: Interceptor PHP code generator
phase: 04-code-generator
feature: interceptor-codegen
owner: Unassigned
status: Ready
estimate: L
depends_on: [TKT-013]
touches:
  - rust/di-compiler/crates/code-generator/src/interceptor.rs
acceptance:
  - Generated Interceptor.php files match PHP INTERCEPTION output byte-for-byte
  - All public non-final methods wrapped
  - Constructor calls ___init() then parent::__construct
---

# TKT-017: Interceptor Code Generator

## Scope

Generate `*Interceptor.php` files from `InterceptorSpec`.

## Implementation Notes

```rust
pub fn generate_interceptor(spec: &InterceptorSpec, class_info: &ClassInfo) -> String
```

Template (implement as format string or a simple template engine):
- Namespace = same as target class namespace
- Class name = `Interceptor` (always)
- `extends \{OriginalFQCN}`
- `implements \Magento\Framework\Interception\InterceptorInterface`
- `use \Magento\Framework\Interception\Interceptor;`
- Constructor: same params as original, calls `$this->___init()` then `parent::__construct(...)`
- One method wrapper per public method in `spec.public_methods`

Output path: `generated/code/{Vendor}/{Module}/{SubPath}/Interceptor.php`
where path is derived from `spec.fqcn` by replacing `\` with `/`.

Compare template against a sample Interceptor from Phase 00 ground truth before implementing.

## Risks

- Method param formatting must match PHP exactly (spacing, typehints)
- `return` vs no-return for void methods — check PHP generated output
