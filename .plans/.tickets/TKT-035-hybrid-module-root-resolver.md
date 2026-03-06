---
id: TKT-035
title: Hybrid module-root resolver (Composer seed + registration fallback)
phase: 09-performance-hardening
feature: module-root-discovery-hybrid
owner: Unassigned
status: Ready
estimate: M
depends_on: [TKT-034]
touches:
  - rust/di-compiler/crates/php-extractor/src/walker.rs
  - rust/di-compiler/crates/di-xml-reader/src/config.rs
  - rust/di-compiler/crates/cli/src/main.rs
acceptance:
  - Module-root coverage matches current registration-path discovery on this repo
  - `app/code` modules and `/setup` are still included
  - Vendor edge cases with nested module roots remain covered
  - No generated `_code`/`_metadata` regression in validator output
  - Module-root discovery time does not regress; target is improved wall clock
test_plan:
  - Compare discovered module roots before/after against a golden snapshot
  - Full compile to temp output and validator diff against baseline archives
  - Explicit checks for known edge paths (e.g. nested `src/*/registration.php`)
---

# TKT-035: Hybrid module-root resolver (Composer seed + registration fallback)

## Scope

Introduce a single shared module-root resolver used by both PHP extraction and DI XML discovery:

- Fast seed from Composer runtime metadata (`vendor/composer/autoload_files.php` registration entries).
- Explicit include for `app/code/*/*/registration.php`.
- Explicit include for `/setup`.
- Bounded registration-path fallback for vendor packages not covered by the Composer seed.

## Implementation Notes

- Avoid duplicating root-discovery logic in `php-extractor` and `di-xml-reader`.
- Canonicalize duplicate roots (e.g. package root and nested `src` root) and keep deterministic ordering.
- Preserve current behavior for module layouts that do not rely on Composer autoload declarations.

## Risks

- Aggressive canonicalization can drop valid module roots in multi-module packages.
- Composer metadata shape differences (`installed.json` variants) can cause environment-specific drift.
