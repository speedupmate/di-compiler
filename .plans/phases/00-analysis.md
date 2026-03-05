# Phase 00: Analysis

## Purpose

Run all 11 phases of `.plans/01-magento-di-analysis.md` against this Magento install.
Record results. Fill the deliverables table and coverage gap table (section 10f).
Capture the ground truth `generated/` archive.

This data drives architecture decisions in every subsequent phase — do not skip it.

## Gate To Enter

None. This is the first phase.

## Gate To Complete

- Deliverables table in `01-magento-di-analysis.md` fully populated
- Coverage gap table (section 10f) filled
- `generated/_code/` and `generated/_metadata/` exist with PHP compiler output (renamed after compile)
- Exact whitespace format of `generated/_metadata/global.php` documented (needed by TKT-016)

## Key Questions This Phase Answers

| Question | Why It Matters |
|----------|----------------|
| Does Magento use constructor promotion? | Determines lexer complexity |
| Count of union/intersection type hints? | Sets threshold for tree-sitter fallback (Tier 2) |
| Exact `generated/metadata/global.php` whitespace? | Gates metadata serializer (TKT-016) |
| Are area-specific di.xml loaded during compile? | Gates area config codegen (TKT-021) |
| Generated file types beyond Interceptor/Factory/Proxy? | Gates TKT-020 |
| I/O bound or CPU bound? | Informs parallelism strategy |
| Total PHP files scanned? | Sets benchmark corpus size |

## Features In This Phase

- [00-ground-truth-capture](./.features/00-ground-truth-capture.md) — deps: none

## Recommended External Tracker Shape

1. One epic: "Phase 00 — Analysis"
2. One ticket: run analysis and fill deliverables table
