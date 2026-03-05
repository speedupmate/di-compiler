---
id: TKT-020
title: Repository code generator
phase: 04-code-generator
feature: repository-codegen
owner: Unassigned
status: Ready
estimate: M
depends_on: [TKT-008, TKT-012]
touches:
  - rust/di-compiler/crates/code-generator/src/repository.rs
acceptance:
  - Generated Repository files match PHP REPOSITORY_GENERATOR output
---

# TKT-020: Repository Code Generator

## Scope

Generate repository classes for REPOSITORY_GENERATOR operation.

## Implementation Notes

**Trigger condition must be confirmed in Phase 00.** Expected: classes with `@api` annotation
that implement a repository interface (has `getById`, `getList`, `save`, `delete` methods).

Examine 3+ generated repository files from Phase 00 ground truth and implement the template.

## Risks

- Scope may be narrower than expected — Phase 00 analysis required before implementing
