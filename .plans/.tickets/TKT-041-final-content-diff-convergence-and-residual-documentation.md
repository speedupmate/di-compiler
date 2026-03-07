---
id: TKT-041
title: Final content diff convergence and residual documentation
phase: 08-parity-closure
feature: code-content-parity-closure
owner: Unassigned
status: In Progress
estimate: S
depends_on: [TKT-040, TKT-044, TKT-045, TKT-046]
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

## Current Snapshot (2026-03-06)

- Active residual set from archive compare:
  - code missing `0`, extra `39`, changed `32`
  - metadata missing `0`, extra `0`, changed `16`
- Completed: TKT-038, TKT-039
- Open blockers: TKT-040, TKT-044, TKT-045, TKT-046

## Risks

- Residual set can drift over time without automated guardrails.
- Documentation may lag if compare output schema changes.
