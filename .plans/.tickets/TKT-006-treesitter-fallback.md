---
id: TKT-006
title: tree-sitter-php Tier 2 fallback extractor
phase: 01-php-extractor
feature: php-treesitter-tier2
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-001]
touches:
  - rust/di-compiler/crates/php-extractor/src/tier2_treesitter.rs
acceptance:
  - Correctly extracts ClassInfo for intersection_type.php fixture (which Tier 1 cannot handle)
  - Returns same ClassInfo structure as Tier 1
  - Falls through to PhpFallbackFailed if tree-sitter parse fails
---

# TKT-006: tree-sitter-php Tier 2 Fallback

## Scope

Implement `extract_with_treesitter(source: &[u8]) -> Result<Option<ClassInfo>, ExtractError>`.

## Implementation Notes

```rust
use tree_sitter::{Parser, Language};
extern "C" { fn tree_sitter_php() -> Language; }

pub fn extract_with_treesitter(source: &[u8]) -> Result<Option<ClassInfo>, ExtractError> {
    let mut parser = Parser::new();
    parser.set_language(unsafe { tree_sitter_php() })?;
    let tree = parser.parse(source, None).ok_or(ExtractError::ParseFailed)?;

    // Query for namespace_definition, class_declaration, constructor_declaration
    // Walk syntax tree nodes by kind
    // Build ClassInfo from matched nodes
}
```

Key tree-sitter node types:
- `program → namespace_definition → namespace_name`
- `program → (class_declaration|interface_declaration|trait_declaration)`
- `class_declaration → class_body → method_declaration[name=__construct]`
- `method_declaration → formal_parameters → (simple_parameter|variadic_parameter|promoted_parameter)`

## Risks

- `tree-sitter-php` C bindings require a C compiler in the build environment
- Grammar version must match `tree-sitter` crate version exactly
