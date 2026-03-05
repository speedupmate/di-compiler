# 04: PHP Extractor — Tier 2 (tree-sitter-php)

- Category: Extraction
- Status: Planned
- Implementation Phase: 01-php-extractor
- Owner: Unassigned
- Feature ID: `php-treesitter-tier2`
- Suggested Dependencies: 01-workspace-scaffold

## Intent

Fallback extractor using `tree-sitter-php` for files where Tier 1 returns
`Err(LexError::Unsupported)`. Produces the same `ClassInfo` output type.

## Core State and Actions

```rust
pub fn extract_with_treesitter(source: &[u8]) -> Result<Option<ClassInfo>, ExtractError> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(tree_sitter_php::language_php())?;
    let tree = parser.parse(source, None).ok_or(ExtractError::ParseFailed)?;
    // Walk tree for: namespace_definition, class_declaration,
    //   constructor_declaration, method_declaration
    todo!()
}
```

## Tree-sitter Node Types to Query

- `namespace_definition` → namespace string
- `class_declaration` / `interface_declaration` / `trait_declaration` → name, extends, implements
- `constructor_declaration` → parameter list
- `method_declaration` → name, visibility, final modifier

## Acceptance Criteria

- Handles all cases Tier 1 returns Unsupported for (intersection types, unusual syntax)
- Returns same `ClassInfo` struct shape as Tier 1
- Falls through to Tier 3 (`PhpFallbackFailed`) if tree-sitter parse fails
