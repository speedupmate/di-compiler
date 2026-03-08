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

## Current Status (2026-03-08)

### Runtime Behavior

`bin/magento list` reports **177 commands in compiled mode = 177 in developer mode** —
runtime behavior parity achieved for command dispatch. This was validated after two
argument-resolver fixes (interface arg inheritance + recursive array merge). Multiple
classes verified against PHP runtime via `$config->getArguments(...)` — metadata
semantics match PHP exactly.

PHP runtime (`$config->getArguments()`) is now the primary correctness ground truth.
Comparison against the archived baseline (`generated/_code/`, `generated/_metadata/`) is
a secondary signal for byte-level file format work.

### Code File Parity (vs Archive Baseline)

Content diffs in interceptors, factories, and proxies remain. The archived baseline
numbers from 2026-03-05 (723 missing / 13 extra / 4219 content-different code files) are
no longer the active tracking metric — runtime semantic parity is.

### Metadata File Parity (vs Archive Baseline)

Metadata runtime semantics match PHP runtime. Remaining archive diffs are key-ordering
artifacts (BTreeMap vs PHP insertion order) and minor serialization formatting, not
semantic gaps.

### Primary Remaining Gaps

- Code file byte-exact content (interceptors, factories, proxies): namespace structure, method signatures, scanner trigger/skip rules
- Missing generated entity types: plugin-list metadata, app action list metadata, extension attributes/service data
- Metadata key-ordering vs PHP insertion order (non-semantic, archive diff only)
- Metadata syntax: area files may still contain invalid numeric literals (`php -n -l`)

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
