# 03: PHP Lexer — Tier 1 (Custom State Machine)

- Category: Extraction
- Status: Planned
- Implementation Phase: 01-php-extractor
- Owner: Unassigned
- Feature ID: `php-lexer-tier1`
- Suggested Dependencies: 01-workspace-scaffold

## Intent

A fast, targeted state-machine lexer that extracts exactly what DI needs from a PHP file.
Handles ~99% of Magento PHP files without tree-sitter or PHP. Never enters method bodies.

## State Machine

```
START
  → "namespace" keyword → capture namespace string → AFTER_NAMESPACE
AFTER_NAMESPACE
  → "class" / "abstract class" / "interface" / "trait" keyword → capture name, kind → CLASS_HEADER
CLASS_HEADER
  → "extends" → capture parent FQN → CLASS_HEADER
  → "implements" → capture comma-separated list → CLASS_HEADER
  → "{" → INSIDE_CLASS (depth = 1)
INSIDE_CLASS
  → "public function" / "protected function" (not final) → METHOD_HEADER
  → "__construct" → CONSTRUCTOR_PARAMS
  → "function" (other) → skip body (count braces) → INSIDE_CLASS
  → "{" → depth++ → INSIDE_CLASS
  → "}" depth-- → if depth == 0 → DONE
METHOD_HEADER
  → capture name, "final" flag → METHOD_PARAMS → INSIDE_CLASS
CONSTRUCTOR_PARAMS / METHOD_PARAMS
  → parse param list until ")" at depth 0
  → for each param: visibility (promotion), nullable "?", type hint, "...", "$name", "="
  → → INSIDE_CLASS (for methods) or DONE (for constructor)
```

## What to Extract

```rust
pub struct ClassInfo {
    pub path: PathBuf,
    pub namespace: String,
    pub name: String,
    pub fqcn: String,
    pub kind: ClassKind,
    pub extends: Option<String>,
    pub implements: Vec<String>,
    pub constructor: Option<Constructor>,
    pub is_abstract: bool,
    pub public_methods: Vec<MethodSignature>,  // for InterceptorSpec
}

pub enum ClassKind { Class, AbstractClass, Interface, Trait }
```

## Edge Cases (from 02-magento-di-rust-implementation.md)

| Case | Handling |
|------|----------|
| Constructor promotion `public readonly Foo $foo` | Extract type hint, set is_promoted |
| Nullable `?Foo` | Extract Foo, set is_optional |
| Union `Foo\|Bar` | Extract first non-null type, log warning |
| Intersection `Foo&Bar` | Return `Err(LexError::Unsupported)` → Tier 2 |
| Variadic `...$args` | Set is_variadic, type hint if present |
| Enum class | Return `ExtractResult::NoClass` |
| Anonymous class | Ignore, continue |
| Multiple classes in one file | Extract first, log warning |
| No namespace | namespace = "" |
| Abstract class | kind = AbstractClass, is_abstract = true |
| Interface | kind = Interface, no constructor |
| Trait | kind = Trait |

## Acceptance Criteria

- Passes all fixture snapshot tests (TKT-009)
- Does not enter method bodies (verify with brace-depth tracking)
- Returns `Err(LexError::Unsupported)` for intersection types, not a panic
- Handles files with Windows line endings (CRLF)
