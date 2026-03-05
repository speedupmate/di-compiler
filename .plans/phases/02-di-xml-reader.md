# Phase 02: DI XML Reader

## Purpose

Build the `di-xml-reader` crate. Parse all `di.xml` files and merge them into a single
`DiConfig` struct per area, following Magento's id-attribute merge rules and load order.

## Gate To Enter

Phase 00 complete (area-specific di.xml coverage confirmed, xsi:type varieties known).

## Gate To Complete

- All `xsi:type` argument varieties parsed correctly
- Merge produces same preferences/plugins/virtualTypes as PHP DomMapper output
- Area-specific files (`frontend/di.xml`, `adminhtml/di.xml`, etc.) loaded correctly

## Features In This Phase

| Feature | Deps |
|---------|------|
| [07-di-xml-parser](./.features/07-di-xml-parser.md) | TKT-001 |
| [08-di-xml-merger](./.features/08-di-xml-merger.md) | 07 |
| [09-di-config-model](./.features/09-di-config-model.md) | 08 |

## Merge Rules

```
id-attributes:
  preference        → @for
  type/virtualType  → @name
  plugin            → @name (under type)
  argument          → @name (under type/arguments)

load order (later overrides earlier for preferences; plugins accumulate):
  vendor/magento/*/etc/di.xml
  vendor/*/etc/di.xml
  app/etc/di.xml
  app/code/*/etc/di.xml
  + area-specific: same dirs under etc/{area}/di.xml
```

## Tickets In This Phase

TKT-010 through TKT-012
