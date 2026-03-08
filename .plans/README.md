# rust/di-compiler — Planning Index

Rust replacement for `bin/magento setup:di:compile`.

Canonical compare baseline for this repo is:
- `generated/_code/`
- `generated/_metadata/`

## Docs

- [000-product-prd.md](000-product-prd.md) — product goals, constraints, release gates
- [phases/](phases/) — phase PRDs (execution order)
- [.features/](.features/) — one spec per feature
- [.tickets/](.tickets/) — execution-sized ticket packs
- [.skills/](.skills/) — reusable project-specific agent skills
- [.debugging/](.debugging/) — deep-dive debugging notes and parity investigations

## Phase Order

| # | Phase | Gate |
|---|-------|------|
| 00 | [Analysis](phases/00-analysis.md) | Must run before implementation |
| 01 | [PHP Extractor](phases/01-php-extractor.md) | 00 complete |
| 02 | [DI XML Reader](phases/02-di-xml-reader.md) | 00 complete (parallel with 01) |
| 03 | [DI Resolver](phases/03-di-resolver.md) | 01 + 02 complete |
| 04 | [Code Generator](phases/04-code-generator.md) | 03 complete |
| 05 | [Validator](phases/05-validator.md) | 04 complete |
| 06 | [CLI](phases/06-cli.md) | 01–05 complete |
| 07 | [Performance](phases/07-performance.md) | Validator stable, no correctness regressions |
| 08 | [Parity Closure](phases/08-parity-closure.md) | Required before production adoption |
| 09 | [Performance Hardening](phases/09-performance-hardening.md) | Active optimization stream while parity closure remains in progress |

## Current Planning Assumptions

- Phase terminology remains `phases/`.
- Existing `.plans` structure is preserved and updated in place.
- External tracker mirrors local tickets; technical source-of-truth remains in repo docs.
- Phase 09 is now active for hot-path performance hardening work (reflection + discovery caching).
