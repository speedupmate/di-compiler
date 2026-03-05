---
id: TKT-004
title: PHP lexer — constructor parameter extraction
phase: 01-php-extractor
feature: php-lexer-tier1
owner: Unassigned
status: Ready
estimate: L
depends_on: [TKT-003]
touches:
  - rust/di-compiler/crates/php-extractor/src/lexer.rs
acceptance:
  - Extracts all constructor params from constructor_promotion.php fixture
  - Handles nullable ?Foo, variadic ..., optional (has =)
  - Handles constructor promotion (public/private/protected + optional readonly)
  - Union type Foo|Bar → extract first type, log warning
  - Intersection type Foo&Bar → return LexError::Unsupported
---

# TKT-004: PHP Lexer — Constructor Parameter Extraction

## Scope

Extend the lexer from TKT-003: after reaching `INSIDE_CLASS`, find `__construct` and
parse its parameter list.

## Implementation Notes

Inside class body (depth tracking with `{`/`}`):
- Skip non-constructor functions by scanning to matching `}` (count braces)
- On `__construct(`: parse param list until `)` at paren-depth 0

Per param in the list:
1. Optional visibility modifier: `public` / `private` / `protected` (constructor promotion)
2. Optional `readonly` keyword
3. Optional `?` (nullable)
4. Type hint: bare word, `\`-prefixed FQN, or `Namespace\Class` — normalize
5. Optional `...` (variadic)
6. `$name`
7. Optional `=` → set is_optional = true (don't parse default value, just flag it)

Union type `Foo|Bar`: split on `|`, take first non-`null` type, set `is_optional = true` if `null` present.
Intersection type `Foo&Bar`: return `LexError::Unsupported("intersection_type")`.

## Risks

- Nested generics or complex defaults (arrays, new expressions) after `=` — just flag is_optional, skip value
