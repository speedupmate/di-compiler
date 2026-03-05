# Phase 01: PHP Extractor

## Purpose

Build the `php-extractor` crate. Given a set of PHP file paths, extract `ClassInfo` structs
(namespace, FQN, kind, extends, implements, constructor params, public methods) using a
three-tier extraction strategy. Never panic. Log all failures.

## Gate To Enter

Phase 00 complete — coverage gap table filled, constructor promotion count known.

## Gate To Complete

- TKT-009 green: all fixture snapshot tests pass
- Extraction fallback rate on real Magento < 0.5% (Tier 2), 0% failures (Tier 3)
- `ClassInfo` structs match PHP reflection output for 100-class validation sample

## Features In This Phase

| Feature | Deps |
|---------|------|
| [01-workspace-scaffold](./.features/01-workspace-scaffold.md) | none |
| [02-php-file-walker](./.features/02-php-file-walker.md) | 01 |
| [03-php-lexer-tier1](./.features/03-php-lexer-tier1.md) | 01 |
| [04-php-treesitter-tier2](./.features/04-php-treesitter-tier2.md) | 01 |
| [05-php-fallback-tier3](./.features/05-php-fallback-tier3.md) | 01 |
| [06-extract-result-type](./.features/06-extract-result-type.md) | 03–05 |
| [22-snapshot-test-corpus](./.features/22-snapshot-test-corpus.md) | 06 |

## Tickets In This Phase

TKT-001 through TKT-009
