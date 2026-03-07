---
id: TKT-045
title: Plugin-list key-space and scope parity
phase: 08-parity-closure
feature: runtime-map-generator-coverage
owner: Unassigned
status: Done
estimate: L
depends_on: [TKT-043, TKT-044]
touches:
  - rust/di-compiler/crates/code-generator/src/plugin_list.rs
  - rust/di-compiler/crates/cli/src/main.rs
acceptance:
  - Plugin-list sections (0/1/2) converge toward baseline key-space per scope
  - `_execute___self`/`___self` key inflation is removed to baseline-equivalent behavior
  - Scope-specific plugin-list generation uses the correct class-definition/key universe for that scope
test_plan:
  - Key-set diff by section for all plugin-list metadata files
  - Targeted assertions for known drift patterns (`_execute___self`, inherited section expansion)
  - Full archive compare and residual categorization
---

# TKT-045: Plugin-list key-space and scope parity

## Scope

Close plugin-list metadata drift that remains after universe/coverage fixes:

- over-generated processed keys (notably `_execute___self`)
- inherited/processed section explosion
- scope-specific input mismatch

## Implementation Notes

- Re-validate listener key construction rules against Magento output semantics.
- Keep section ordering and serialization stable while shrinking key-space to expected shape.

## Risks

- Tightening key generation can accidentally drop valid plugin chains for edge modules.
- Scope leakage (global entries appearing in non-global scope files) can persist if input filtering is incomplete.

## Resolution (2026-03-07)

Code parity gap closed. Remaining delta vs PHP archive:

| Category | Count | Root cause |
|---|---|---|
| code missing | 1 | `StructureLazy/Interceptor.php` — stale baseline (NoninterceptableInterface, PHP also skips) |
| code extra | 2 | `StockItemImporter/Interceptor.php` (archive incomplete, correct); `ReadFactory.php` cap-S (setup typo) |
| metadata changed | 16 | Pure key-ordering noise — BTreeMap vs PHP insertion order. No content bugs. |

Fixes committed:
- Disabled-module di.xml filtering (config.php `=> 0` entries)
- NoninterceptableInterface check in interceptor detection (Phase 1 + Phase 2)
- Setup interceptor filter refined: suppress inherited-only, keep directly-plugged
- Root-namespace factory guard (lexer use-import resolution bug)
