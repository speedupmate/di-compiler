# 29: Extension Attributes Generation

- Category: Correctness
- Status: Planned
- Implementation Phase: 08-parity-closure
- Owner: Unassigned
- Feature ID: `extension-attributes-generation`
- Suggested Dependencies: 28-scanner-parity-xml-php

## Intent

Add compile-time generation support for extension attribute interfaces/classes/factories currently missing from Rust output.

## Current Gap

- Missing `*Extension.php`, `*ExtensionInterface.php`, and related factory outputs are a major share of current code misses.

## Implementation Steps

1. Implement extension attribute class discovery from `extension_attributes.xml` and PHP interface patterns.
2. Generate extension interface/class/factory artifacts with Magento-compatible naming and output.
3. Integrate into compile pipeline ordering with explicit dependency on scanner outputs.

## Test Plan

1. Fixture tests for extension attribute naming and generation triggers.
2. Module-level regression on Magento API/Data interfaces.
3. Full diff run against archived `_code`.

## Acceptance Criteria

- Extension-attribute artifact classes are generated where expected
- Missing file count for Extension* classes trends to zero
