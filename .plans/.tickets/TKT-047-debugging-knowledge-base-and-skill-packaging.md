---
id: TKT-047
title: Debugging knowledge base relocation and skill packaging
phase: 09-performance-hardening
feature: debugging-knowledge-operations
owner: Unassigned
status: Done
estimate: S
depends_on: [TKT-037]
touches:
  - rust/di-compiler/.plans/README.md
  - rust/di-compiler/.plans/.debugging/debugging_draft.md
  - rust/di-compiler/.plans/.debugging/firebear_dependency_injection_article_summary.md
  - rust/di-compiler/.plans/.debugging/interface_arg_inheritance.md
  - rust/di-compiler/.plans/.debugging/metadata_diff_and_reflection_oracle.md
  - rust/di-compiler/.plans/.skills/di-parity-debugging/SKILL.md
  - rust/di-compiler/README.md
  - rust/di-compiler/.plans/.tickets/README.md
acceptance:
  - Existing parity/debugging notes are relocated from `.planning/debugging` to `.plans/.debugging`
  - A reusable debugging skill exists under `.plans/.skills/di-parity-debugging/SKILL.md`
  - The root README documents baseline bootstrap when `_code` and `_metadata` are missing
  - Skill instructions include the same baseline bootstrap flow and refer to the relocated debugging notes
test_plan:
  - Verify `.plans/.debugging/` contains all prior debugging markdown files
  - Verify `.plans/.skills/di-parity-debugging/SKILL.md` exists with baseline and triage workflow
  - Verify `README.md` includes the bootstrap section with `setup:di:compile` + rename steps
---

# TKT-047: Debugging knowledge base relocation and skill packaging

## Scope

Consolidate debugging knowledge under `.plans` and package it as a reusable skill for parity triage sessions, while aligning root documentation with baseline bootstrap requirements.

## Implementation Notes

- Moved debugging notes from `.planning/debugging` to `.plans/.debugging`.
- Added `.plans/.skills/di-parity-debugging/SKILL.md` with:
  - baseline assumptions (`generated/_code`, `generated/_metadata`)
  - compile/compare triage loop
  - runtime safety checks
  - links to deep-dive debugging notes
- Updated root `README.md` with explicit bootstrap instructions when `_code`/`_metadata` are absent:
  - `bin/magento setup:di:compile`
  - rename `generated/code` -> `generated/_code`
  - rename `generated/metadata` -> `generated/_metadata`

## Risks

- If folder conventions change again, the skill links can drift.
- Debugging notes can become stale if parity strategies evolve but docs are not refreshed.

## Implementation Update (2026-03-08)

- Completed relocation to `.plans/.debugging`.
- Added project-local debugging skill under `.plans/.skills`.
- Added baseline bootstrap workflow to root README and skill instructions.
