# 17: Repository Code Generator

- Category: Code Generation
- Status: Planned
- Implementation Phase: 04-code-generator
- Owner: Unassigned
- Feature ID: `repository-codegen`
- Suggested Dependencies: 06-extract-result-type, 09-di-config-model

## Intent

Generate repository classes for the REPOSITORY_GENERATOR operation.
Scope to be confirmed during Phase 00 analysis (what triggers repository generation?).

## Acceptance Criteria

- Output matches PHP REPOSITORY_GENERATOR output byte-for-byte
- Written to `generated/code/` under correct namespace path

## Note

Confirm exact trigger condition during Phase 00. In PHP, this generates
`*Repository.php` classes for interfaces with `@api` annotation + specific method patterns.
