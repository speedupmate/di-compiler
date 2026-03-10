---
id: TKT-001
title: Cargo workspace scaffold
phase: 01-php-extractor
feature: workspace-scaffold
owner: Unassigned
status: Done
estimate: S
depends_on: []
touches:
  - rust/di-compiler/Cargo.toml
  - rust/di-compiler/crates/
  - rust/di-compiler/Makefile
acceptance:
  - cargo build --workspace succeeds
  - cargo test --workspace succeeds (no tests yet)
  - cargo clippy --workspace clean
---

# TKT-001: Cargo Workspace Scaffold

## Scope

Create the full workspace skeleton. No business logic.

## Implementation Notes

- `rust/di-compiler/Cargo.toml` — workspace with `members = ["crates/*"]`, `[workspace.dependencies]` for shared dep versions
- Create 6 crate dirs: `php-extractor`, `di-xml-reader`, `di-resolver`, `code-generator`, `validator`, `cli`
- Each has `Cargo.toml` with correct dependencies (see feature spec 01) and stub `src/lib.rs` or `src/main.rs`
- `Makefile`: `build`, `test`, `check`, `bench`, `fmt`, `clippy`, `clean`
- `.gitignore`: `target/`, `tests/corpus/` (symlink placeholder)
- `tests/fixtures/`, `tests/snapshots/`, `benches/` directories with `.gitkeep`

## Risks

- Ensure `tree-sitter-php` C bindings compile (needs `cc` crate, may need `libclang`)
