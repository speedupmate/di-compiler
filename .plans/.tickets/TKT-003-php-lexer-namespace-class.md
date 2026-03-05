---
id: TKT-003
title: PHP lexer — namespace + class header extraction
phase: 01-php-extractor
feature: php-lexer-tier1
owner: Unassigned
status: Ready
estimate: L
depends_on: [TKT-001]
touches:
  - rust/di-compiler/crates/php-extractor/src/lexer.rs
acceptance:
  - Extracts namespace, class name, kind, extends, implements from fixture files
  - Handles both "namespace Foo;" and "namespace Foo { }" syntax
  - Returns LexError::Unsupported for intersection types
  - Does not panic on any PHP file in tests/fixtures/
---

# TKT-003: PHP Lexer — Namespace + Class Header

## Scope

State-machine lexer: `START → AFTER_NAMESPACE → CLASS_HEADER → INSIDE_CLASS`.
Extract namespace, class name, kind (Class/AbstractClass/Interface/Trait), extends, implements.
Stop at the opening `{` of the class body (don't read the body yet — that's TKT-004/005).

## Implementation Notes

- Operate on raw `&[u8]` — use `memmap2` for zero-copy reads
- Keyword scanning: scan for `<?php`, then state machine on byte slices
- Handle `abstract class` as a two-keyword sequence
- `extends` and `implements` may have FQN with leading `\` — normalize to no-leading-backslash
- `implements` is comma-separated — collect all into `Vec<String>`
- Handle both `namespace Foo\Bar;` (terminated by `;`) and `namespace Foo\Bar { }` (terminated by `{`)
- Enum keyword check: if file has `enum ClassName` pattern → return `LexResult::Enum`

## Risks

- String literals and comments containing `class` keyword — must be skipped
- Use simple heuristic: only match `class` at start of a token (preceded by whitespace or `;`)
