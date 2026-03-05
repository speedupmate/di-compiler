# 01: Workspace Scaffold

- Category: Infrastructure
- Status: Planned
- Implementation Phase: 01-php-extractor
- Owner: Unassigned
- Feature ID: `workspace-scaffold`
- Suggested Dependencies: None

## Intent

Create the Cargo workspace with all six crate skeletons, a `Makefile`, and CI configuration.
No business logic — just the skeleton that all subsequent tickets build on.

## Core State and Actions

1. `rust/di-compiler/Cargo.toml` — workspace with `members = ["crates/*"]`
2. Six crates created: `php-extractor`, `di-xml-reader`, `di-resolver`, `code-generator`, `validator`, `cli`
3. Each crate has: `Cargo.toml` with correct deps, `src/lib.rs` (or `src/main.rs` for cli) with stub
4. `Makefile` with targets: `build`, `test`, `check`, `bench`, `fmt`, `clippy`
5. `.github/workflows/ci.yml` (or equivalent): `cargo check`, `cargo test`, `cargo clippy`

## Dependencies (Cargo)

```toml
# php-extractor
tree-sitter = "0.22"
tree-sitter-php = "0.22"
rayon = "1.10"
ignore = "0.4"
memmap2 = "0.9"
thiserror = "1"
log = "0.4"

# di-xml-reader
quick-xml = "0.36"
thiserror = "1"

# di-resolver
thiserror = "1"

# code-generator
rayon = "1.10"
rustc-hash = "2"
thiserror = "1"

# validator
thiserror = "1"

# cli
clap = { version = "4", features = ["derive"] }
env_logger = "0.11"
indicatif = "0.17"

# workspace dev-dependencies
insta = "1"
criterion = "0.5"
tempfile = "3"
```

## Acceptance Criteria

- `cargo build --workspace` succeeds with no errors
- `cargo test --workspace` succeeds (no tests yet, but compiles)
- `cargo clippy --workspace` clean
