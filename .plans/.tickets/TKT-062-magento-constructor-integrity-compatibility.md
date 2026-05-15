---
id: TKT-062
title: Fail fast on Magento-incompatible constructor pseudo-type hints
phase: 08-parity-closure
feature: 39-magento-constructor-integrity-compatibility
owner: Unassigned
status: Done
estimate: S
depends_on: [TKT-015, TKT-024]
touches:
  - rust/di-compiler/crates/cli/src/main.rs
  - rust/di-compiler/crates/di-resolver/src/arguments.rs
acceptance:
  - `object` and `iterable` constructor type hints are namespace-resolved like Magento, not treated as safe primitives
  - `mixed` remains allowed because Magento ScalarTypesProvider includes it
  - CLI exits non-zero by default when constructor integrity violations are detected
  - Error output lists Magento-supported scalar/pseudo constructor type hints
  - `--ignore-constructor-integrity` logs violations and continues
test_plan:
  - `cargo test -p fast-di-compile`
  - `cargo test -p di-resolver`
  - Dry-run probe module without ignore flag and confirm non-zero exit
  - Dry-run probe module with `--ignore-constructor-integrity` and confirm success
---

# TKT-062: Magento Constructor Integrity Compatibility

## Scope

Magento's constructor integrity validator does not treat every PHP pseudo-type as a supported non-class type. It only allows the scalar/pseudo list returned by `Magento\Framework\Code\Reader\ScalarTypesProvider::getTypes()`.

This ticket closes the gap where Rust was more permissive than Magento for constructor hints such as `object` and `iterable`.

## Implementation Notes

- Add CLI constructor integrity validation after class extraction.
- Fail by default when a parent constructor has a Magento-unsupported pseudo-type and a child forwards an incompatible concrete/interface type to `parent::__construct(...)`.
- Add `--ignore-constructor-integrity` as an explicit escape hatch.
- Keep the supported scalar/pseudo allowlist in one helper, with a source comment pointing to `vendor/magento/framework/Code/Reader/ScalarTypesProvider.php`.
- Update argument resolution so unsupported pseudo-types namespace-resolve like Magento.

Implemented in commit `9a9aeb0` (`Align DI metadata parity with Magento compiler`).

## Risks

- Disabled-but-registered Magento modules are still scanned by Magento compile paths and by Rust. This means constructor integrity failures can still come from modules disabled in `app/etc/config.php` if their `registration.php` remains present.
- The validator is intentionally narrow to avoid broad false positives. Wider constructor compatibility validation should be ticketed separately if needed.
