# 00: Ground Truth Capture

- Category: Analysis
- Status: Planned
- Implementation Phase: 00-analysis
- Owner: Unassigned
- Feature ID: `ground-truth-capture`
- Suggested Dependencies: None

## Intent

Execute all 11 phases of `.plans/01-magento-di-analysis.md` against this Magento install.
Record all results in the deliverables and coverage gap tables. Move  the `generated/{subfolder}` to  `generated/_{subfolder}`.
This data is the foundation for every design decision in Phases 01–07.

## User Behavior

1. Operator runs each bash block in `01-magento-di-analysis.md` phases 1–11
2. Results are recorded directly in that file's deliverables table
3. PHP-generated output preserved in-project by renaming subfolders with `_` prefix:
   - `generated/code/` → `generated/_code/`
   - `generated/metadata/` → `generated/_metadata/`

## Core State and Actions

1. `generated/` is clean (delete before run)
2. `bin/magento setup:di:compile` runs to completion
3. Rename: `mv generated/code generated/_code && mv generated/metadata generated/_metadata`
4. All counts (files, directives, generated classes) recorded
5. Coverage gap table (section 10f) fully filled

## Acceptance Criteria

- Deliverables table in `01-magento-di-analysis.md` has no empty cells
- Coverage gap table (10f) has no empty cells
- `generated/_code/` and `generated/_metadata/` exist with PHP compiler output
- Exact byte format of `generated/_metadata/global.php` documented (needed by TKT-016)
- Union/intersection type hint counts known (determines Tier 2 threshold)
- Constructor promotion count known (determines lexer complexity)
