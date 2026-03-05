# Phase 04: Code Generator

## Purpose

Build the `code-generator` crate. Consume `ResolvedGraph` and write all PHP class files
to `generated/code/` and metadata files to `generated/metadata/`. Output must be
byte-for-byte identical to PHP compiler output.

## Gate To Enter

Phase 03 complete (`ResolvedGraph` validated).

## Gate To Complete

- TKT-016 green: `generated/metadata/` diff clean vs ground truth
- All interceptor/factory/proxy/repository PHP files generated correctly
- Content-addressed writes in place (no unnecessary FS writes)

## Features In This Phase

| Feature | Deps |
|---------|------|
| [14-interceptor-codegen](./.features/14-interceptor-codegen.md) | 10, 13 |
| [15-factory-codegen](./.features/15-factory-codegen.md) | 11 |
| [16-proxy-codegen](./.features/16-proxy-codegen.md) | 12 |
| [17-repository-codegen](./.features/17-repository-codegen.md) | 06, 09 |
| [18-area-config-codegen](./.features/18-area-config-codegen.md) | 13 |
| [19-metadata-php-serializer](./.features/19-metadata-php-serializer.md) | 13 |

## PHP Operations Covered

| Magento Op | Feature |
|-----------|---------|
| PROXY_GENERATOR | 16-proxy-codegen |
| REPOSITORY_GENERATOR | 17-repository-codegen |
| DATA_ATTRIBUTES_GENERATOR | (confirm scope in Phase 00) |
| APPLICATION_CODE_GENERATOR | 15-factory-codegen |
| INTERCEPTION | 14-interceptor-codegen |
| AREA_CONFIG_GENERATOR | 18-area-config-codegen |
| INTERCEPTION_CACHE + PLUGIN_LIST | 19-metadata-php-serializer |

## Tickets In This Phase

TKT-016 through TKT-022
