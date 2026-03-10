# 000: Product PRD — Magento DI Compiler (Rust)

## Product Goal

A single static Rust binary (`fast-di-compile`) that replaces `bin/magento setup:di:compile`.
Reads PHP source files and `di.xml` configuration, writes byte-for-byte identical output to `generated/`.
For this repo, parity is measured against archived PHP ground truth in `generated/_code/` and `generated/_metadata/`.
Correctness first. Speed second. Never silent failures.

## Primary User Outcomes

- Run `fast-di-compile --magento-root /var/www/application` and get identical `generated/` output to `bin/magento setup:di:compile`
- DI compile time drops from ~85 s to < 5 s on a warm filesystem cache
- Any parse failure is loud and logged — never silently wrong output

## Non-Goals

- Full PHP parser (we tokenize only what DI needs — FQN, constructor, public methods)
- Magento upgrade detection or schema migration
- Supporting Magento versions before 2.4
- Replacing other `bin/magento` commands

## Product Constraints

- Output must be byte-for-byte identical (or semantically equivalent) to PHP compiler output
- Validated by diff harness before any release (TKT-023)
- No PHP runtime dependency at compile time (PHP fallback is opt-in, not required)

## Current Status (2026-03-10)

### Metadata Semantic Parity (Area Files)

Latest `compare_metadata_parity.php` snapshot:

- `global`: missing `0`, extra `40`, mismatches `0`
- `frontend`: missing `0`, extra `40`, mismatches `0`
- `adminhtml`: missing `0`, extra `40`, mismatches `0`
- `crontab`: missing `0`, extra `40`, mismatches `0`
- `webapi_rest`: missing `0`, extra `40`, mismatches `0`
- `webapi_soap`: missing `0`, extra `40`, mismatches `0`
- `graphql`: missing `0`, extra `40`, mismatches `0`

`preferences` and `instanceTypes` are now at zero drift in all areas.
Remaining semantic gap in area files is concentrated in `arguments` key-surface extras,
which are currently accepted as deferred residual for this milestone.

### Archive Compare Snapshot (Generated vs _Generated)

From `generated/diff/summary.json`:

- `code_missing`: `0`
- `code_extra`: `3`
- `code_changed`: `0`
- `metadata_missing`: `0`
- `metadata_extra`: `0`
- `metadata_changed`: `16`

### Performance Snapshot

Latest full run with `--compare-archive`:

- Total: `23.885s`
- Phase 5 (argument resolution): `0.672s`
- Phase 7 (metadata generation): `4.142s`
- Archive compare step: `16.121s`

The prior Phase 5 regression (~103s) is closed.

### Primary Remaining Gaps

- Area-config argument key-surface extras (`40` keys repeated across all areas, deferred)
- Plugin-list metadata parity (still significant drift in comparable reports)
- Archive compare closure for `code_extra=3` and `metadata_changed=16`

## Technical Guardrails

- Use `rayon` for parallelism — not `tokio` (CPU+IO bound, not async network)
- Use `ignore` crate for file walking (respects `.gitignore`, parallel)
- Use `quick-xml` for di.xml SAX parsing
- Three-tier PHP extraction: custom state-machine lexer → `tree-sitter-php` → PHP shell
- Content-addressed writes: hash before writing, skip unchanged files
- All errors use `thiserror` — no `unwrap()` in library crates

## Workspace Layout

```
rust/di-compiler/
├── Cargo.toml                   # workspace, members = ["crates/*"]
├── crates/
│   ├── php-extractor/
│   ├── di-xml-reader/
│   ├── di-resolver/
│   ├── code-generator/
│   ├── validator/
│   └── cli/
├── tests/fixtures/              # .php edge-case files
├── tests/corpus/                # symlink to vendor/ for CI
├── tests/snapshots/             # insta snapshot expectations
└── benches/full_compile.rs
```

## Crate Dependencies

```
cli
 ├── php-extractor
 ├── di-xml-reader
 ├── di-resolver
 ├── code-generator
 └── validator  (dev/CI only)
```

## Release Logic

- Phase 00 (analysis) must complete before any crate work starts
- Phase 01–02 can proceed in parallel after Phase 00
- Phase 03 gates on Phase 01 + 02
- Phase 04 gates on Phase 03
- Phases 05–06 gate on Phase 04
- Phase 07 (performance) is optimization-only and must not change correctness
- Phase 08 (parity closure) is mandatory before production adoption
- Final release gate: zero diff vs archived PHP baseline + valid PHP syntax for all generated metadata files

## Success Criteria

| Metric | Target |
|--------|--------|
| Output identical to PHP compiler | 100% file match |
| Wall clock (warm cache) | < 5 s |
| Wall clock (cold cache) | < 15 s |
| Files falling back to PHP | < 0.5% |
| Files failing entirely | 0 |
| Generated metadata syntax | 100% `php -n -l` pass |
| CI green on clean Magento 2.4.x | ✅ |

## Local Planning Layout

- `phases/`: phase-level PRDs
- `.features/`: one spec per feature
- `.tickets/`: execution-sized ticket packs

## External Tracker Policy

- Link tracker issues back to local spec or ticket files
- Technical contracts live here, not in issue comments
