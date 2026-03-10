---
id: TKT-056
title: Replace std HashMap/HashSet with FxHashMap/FxHashSet across all crates
phase: 09-performance-hardening
feature: 38-low-level-performance-hardening
owner: Unassigned
status: Done
estimate: S
depends_on: []
touches:
  - rust/di-compiler/crates/di-resolver/src/interceptor.rs
  - rust/di-compiler/crates/di-resolver/src/arguments.rs
  - rust/di-compiler/crates/di-resolver/src/graph.rs
  - rust/di-compiler/crates/di-resolver/src/factory.rs
  - rust/di-compiler/crates/di-resolver/src/proxy.rs
  - rust/di-compiler/crates/di-xml-reader/src/model.rs
  - rust/di-compiler/crates/di-xml-reader/src/parser.rs
  - rust/di-compiler/crates/di-xml-reader/src/config.rs
  - rust/di-compiler/crates/php-extractor/src/constants.rs
  - rust/di-compiler/crates/php-extractor/src/lexer.rs
  - rust/di-compiler/crates/php-extractor/src/tier2.rs
  - rust/di-compiler/crates/code-generator/src/metadata.rs
  - rust/di-compiler/crates/code-generator/src/area_config.rs
  - rust/di-compiler/crates/code-generator/src/app_action_list.rs
  - rust/di-compiler/crates/code-generator/src/plugin_list.rs
  - rust/di-compiler/crates/cli/src/main.rs
  - rust/di-compiler/crates/cli/Cargo.toml
  - rust/di-compiler/crates/di-resolver/Cargo.toml
  - rust/di-compiler/crates/di-xml-reader/Cargo.toml
  - rust/di-compiler/crates/php-extractor/Cargo.toml
acceptance:
  - All HashMap/HashSet usages replaced with FxHashMap/FxHashSet in the 5 non-cli crates and cli
  - rustc-hash added to Cargo.toml for crates that lacked it (cli, di-resolver, di-xml-reader, php-extractor)
  - cargo build --release succeeds with zero warnings
  - cargo test passes (all pre-existing tests green)
test_plan:
  - cargo build --release
  - cargo test
---

# TKT-056: Replace std HashMap/HashSet with FxHashMap/FxHashSet across all crates

## Scope

`rustc-hash` was already a workspace dependency used only in `code-generator`. Extend
it to all six crates: `di-resolver`, `di-xml-reader`, `php-extractor`, `cli`, and
`code-generator` (already done). Replace every `std::collections::HashMap` and
`HashSet` at the source level.

## Implementation Notes

- `rustc_hash::FxHashMap` / `FxHashSet` are drop-in replacements; only imports and
  constructor calls change (`HashMap::new()` → `FxHashMap::default()`).
- Where types appear in public function signatures or struct fields, the substitution
  is transparent because the external interface (crate boundaries) all use concrete
  owned maps, not trait objects.
- Test modules that construct maps and pass them to functions must also import
  `rustc_hash::FxHashMap`.

## Status

Implemented. Build clean, all tests pass.
