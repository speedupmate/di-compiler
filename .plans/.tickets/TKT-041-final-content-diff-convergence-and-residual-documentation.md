---
id: TKT-041
title: Final content diff convergence and residual documentation
phase: 08-parity-closure
feature: code-content-parity-closure
owner: Unassigned
status: In Progress
estimate: S
depends_on: [TKT-040, TKT-045]
touches:
  - rust/di-compiler/crates/cli/src/main.rs
  - rust/di-compiler/README.md
  - rust/di-compiler/.plans/.tickets/TKT-041-final-content-diff-convergence-and-residual-documentation.md
acceptance:
  - Full archive compare runs produce categorized residual changed sets with documented rationale
  - Any intentionally unresolved content differences are explicitly listed in repo docs
  - Compare report output remains stable and actionable for CI/manual review
test_plan:
  - Run compile + archive compare on clean output root
  - Verify summary and changed reports match documented residual categories
  - Review README parity section against latest report format
---

# TKT-041: Final content diff convergence and residual documentation

## Scope

Complete the final content-parity convergence pass and document residual differences that remain by design or dependency constraints.

## Current Snapshot (2026-03-07, updated)

- Active residual set (clean output directory, fresh compile):
  - code missing `0`, extra `0`, changed `0`
  - metadata missing `0`, extra `0`, changed `8` (ordering noise only — equal ±lines in all 7 area configs; `interception.php` identical)
- Completed: TKT-038, TKT-039, TKT-040, TKT-042, TKT-043, TKT-044, TKT-046
- Open blockers: TKT-045

### What TKT-044 fixed

- **Virtual types in arguments**: Non-intercepted virtual types now correctly appear
  in the `arguments` section. Previously all virtual types were filtered out.
- **Virtual types in preferences**: Virtual types whose concrete class is intercepted
  (e.g. a payment gateway facade virtual type → `Concrete\Interceptor`) now correctly
  appear in the `preferences` section instead of being silently dropped.
- **instanceTypes unchanged**: PHP uses the bare concrete class name in `instanceTypes`;
  the Interceptor form is exclusive to `preferences`. Code reflects this correctly.
- **`interception.php`**: Stale Interceptor entries from previous runs (for uninstalled
  EE modules referenced by bridge di.xml files) were mistaken for a live bug. On a clean
  output directory the file is byte-for-byte identical to PHP.

## Known Deferred: Metadata key ordering gap

Metadata files (`global.php`, `frontend.php`, plugin-list files, etc.) differ from the PHP
archive in **key ordering only** for ~79% of their diff lines. PHP's `setup:di:compile`
produces keys in filesystem traversal order (`RecursiveDirectoryIterator` inode order),
which is non-deterministic and cannot be reproduced without mirroring the exact directory
scan sequence. Rust uses `BTreeMap` (alphabetical). PHP array key order has no runtime
impact — Magento accesses all metadata via `isset()`/`array_key_exists()` and direct key
lookup. The remaining `metadata changed 16` count is therefore inflated by this ordering
noise. This gap has **not** been closed and may warrant further analysis if exact byte-for-byte
archive parity becomes a requirement.

## Risks

- Residual set can drift over time without automated guardrails.
- Documentation may lag if compare output schema changes.
- Metadata ordering gap inflates `changed` count; actual content bugs are a smaller subset.
