# Tickets

Execution-sized slices. One agent per ticket. Dependencies are explicit.

| TKT | Title | Phase | Status | Deps |
|-----|-------|-------|--------|------|
| [001](TKT-001-workspace-scaffold.md) | Workspace scaffold | 01 | Ready | — |
| [002](TKT-002-php-file-walker.md) | PHP file walker | 01 | Ready | 001 |
| [003](TKT-003-php-lexer-namespace-class.md) | PHP lexer: namespace + class header | 01 | Ready | 001 |
| [004](TKT-004-php-lexer-constructor-params.md) | PHP lexer: constructor params | 01 | Ready | 003 |
| [005](TKT-005-php-lexer-public-methods.md) | PHP lexer: public method signatures | 01 | Ready | 004 |
| [006](TKT-006-treesitter-fallback.md) | tree-sitter Tier 2 fallback | 01 | Ready | 001 |
| [007](TKT-007-php-shell-fallback.md) | PHP shell Tier 3 fallback | 01 | Ready | 001 |
| [008](TKT-008-extract-result-orchestrator.md) | Extract result orchestrator | 01 | Ready | 003–007 |
| [009](TKT-009-fixture-snapshot-tests.md) | Fixture + snapshot tests | 01 | Ready | 008 |
| [010](TKT-010-di-xml-sax-parser.md) | di.xml SAX parser | 02 | Ready | 001 |
| [011](TKT-011-di-xml-merger.md) | di.xml merger | 02 | Ready | 010 |
| [012](TKT-012-di-config-struct-queries.md) | DiConfig struct + query methods | 02 | Ready | 011 |
| [013](TKT-013-interceptor-detection.md) | Interceptor detection | 03 | Ready | 008, 012 |
| [014](TKT-014-factory-proxy-detection.md) | Factory + Proxy detection | 03 | Ready | 008, 012 |
| [015](TKT-015-arguments-resolver.md) | Arguments resolver | 03 | Ready | 012, 008 |
| [016](TKT-016-metadata-php-serializer.md) | Metadata PHP serializer | 04 | Ready | 015 |
| [017](TKT-017-interceptor-codegen.md) | Interceptor code generator | 04 | Ready | 013 |
| [018](TKT-018-factory-codegen.md) | Factory code generator | 04 | Ready | 014 |
| [019](TKT-019-proxy-codegen.md) | Proxy code generator | 04 | Ready | 014 |
| [020](TKT-020-repository-codegen.md) | Repository code generator | 04 | Ready | 008, 012 |
| [021](TKT-021-area-config-codegen.md) | Area config code generator | 04 | Ready | 015 |
| [022](TKT-022-content-addressed-writes.md) | Content-addressed writes | 04 | Ready | 017–021 |
| [023](TKT-023-validator-diff-harness.md) | Validator diff harness | 05 | Ready | 016–022 |
| [024](TKT-024-cli-binary.md) | CLI binary | 06 | Ready | all |
| [025](TKT-025-rayon-parallel-parse.md) | rayon parallel parse | 07 | Ready | 008, 022 |
| [026](TKT-026-incremental-fingerprinting.md) | Incremental fingerprinting | 07 | Ready | 025 |
| [027](TKT-027-metadata-syntax-and-scalar-parity.md) | Metadata syntax and scalar parity | 08 | Ready | 016, 023 |
| [028](TKT-028-interceptor-namespace-structure-parity.md) | Interceptor namespace and structure parity | 08 | Ready | 017 |
| [029](TKT-029-interceptor-method-and-signature-parity.md) | Interceptor method and signature parity | 08 | Ready | 028 |
| [030](TKT-030-scanner-parity-xml-php.md) | Scanner parity for XML and PHP triggers | 08 | Ready | 014 |
| [031](TKT-031-extension-attributes-and-service-data-generation.md) | Extension attributes and service data generation | 08 | Ready | 030 |
| [032](TKT-032-plugin-list-metadata-generation.md) | Plugin-list metadata generation | 08 | Ready | 027, 030 |
| [033](TKT-033-app-action-list-and-archive-layout-parity.md) | App action list and archive layout parity | 08 | Ready | 032 |
| [034](TKT-034-runtime-map-coverage-and-final-diff-closure.md) | Runtime-map coverage and final diff closure | 08 | Ready | 031, 032, 033 |
