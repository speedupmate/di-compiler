---
id: TKT-005
title: PHP lexer — public method signature extraction
phase: 01-php-extractor
feature: php-lexer-tier1
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-004]
touches:
  - rust/di-compiler/crates/php-extractor/src/lexer.rs
acceptance:
  - Extracts all public non-final methods from a sample interceptor-eligible class
  - final public methods excluded
  - private/protected methods excluded
  - Returns MethodSignature { name, params, return_type }
---

# TKT-005: PHP Lexer — Public Method Signature Extraction

## Scope

After extracting the constructor, continue scanning class body to collect all
`public function` declarations (excluding `final` and `__construct`).
Needed by `InterceptorSpec.public_methods`.

## Implementation Notes

In `INSIDE_CLASS` state:
- On `public function` (preceded by optional `final`):
  - If `final`: skip to `}` — do not add to list
  - Otherwise: parse method name + param types (same param parsing as TKT-004) + return type
- Skip method bodies by counting braces
- Stop at depth-0 `}` (end of class)

```rust
pub struct MethodSignature {
    pub name: String,
    pub params: Vec<ConstructorParam>,   // reuse same type
    pub return_type: Option<String>,
    pub is_static: bool,
}
```

## Risks

- `abstract public function` — include in list (interceptors still need to handle these)
- Magic methods (`__get`, `__set`, etc.) — include (PHP interceptors handle them)
