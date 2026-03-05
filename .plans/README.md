# rust/di-compiler — Planning Index

Rust replacement for `bin/magento setup:di:compile`.

## Docs

- [000-product-prd.md](000-product-prd.md) — goals, constraints, success criteria
- [phases/](phases/) — phase PRDs (logical execution order)
- [.features/](.features/) — one spec per feature
- [.tickets/](.tickets/) — execution-sized ticket packs

## Phase Order

| # | Phase | Gate |
|---|-------|------|
| 00 | [Analysis](phases/00-analysis.md) | Must run before writing Rust |
| 01 | [PHP Extractor](phases/01-php-extractor.md) | Phase 00 complete |
| 02 | [DI XML Reader](phases/02-di-xml-reader.md) | — (parallel with 01) |
| 03 | [DI Resolver](phases/03-di-resolver.md) | 01 + 02 complete |
| 04 | [Code Generator](phases/04-code-generator.md) | 03 complete |
| 05 | [Validator](phases/05-validator.md) | 04 complete |
| 06 | [CLI](phases/06-cli.md) | All complete |
| 07 | [Performance](phases/07-performance.md) | Validator green (TKT-023) |

## External Reference Docs

- `.plans/01-magento-di-analysis.md` — ground truth capture commands
- `.plans/02-magento-di-rust-implementation.md` — architecture, structs, deps
