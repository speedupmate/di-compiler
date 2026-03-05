---
id: TKT-010
title: di.xml SAX parser
phase: 02-di-xml-reader
feature: di-xml-parser
owner: Unassigned
status: Ready
estimate: L
depends_on: [TKT-001]
touches:
  - rust/di-compiler/crates/di-xml-reader/src/parser.rs
acceptance:
  - Parses all di.xml files in this Magento install without errors
  - All 8 xsi:type argument variants parsed correctly
  - disabled="true" plugins parsed
  - Nested array items parsed recursively
---

# TKT-010: di.xml SAX Parser

## Scope

Parse a single `di.xml` file into `PartialDiConfig` using `quick-xml` event-based parsing.

## Implementation Notes

```rust
pub fn parse_di_xml(path: &Path) -> Result<PartialDiConfig, DiXmlError>
```

Use a stack-based state machine with `quick-xml::Reader`:
- Push state on `Start` events, pop on `End` events
- Track current path: `config > type|virtualType > plugins|arguments > plugin|argument`
- On `argument`: read `xsi:type` attribute → parse inner text or nested `item` elements accordingly

Handle all argument types per feature spec 07.

## Risks

- `xsi:type` attribute uses XML namespace prefix — must handle `xsi:type` regardless of namespace declaration position
- Array items can be nested arrays — recursive parsing needed
