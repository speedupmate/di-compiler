# 28: Scanner Parity (XML + PHP)

- Category: Correctness
- Status: Planned
- Implementation Phase: 08-parity-closure
- Owner: Unassigned
- Feature ID: `scanner-parity-xml-php`
- Suggested Dependencies: 07-di-xml-parser, 11-factory-detection, 12-proxy-detection

## Intent

Mirror Magento scanner triggers used by compile operations so generated class candidate sets match.

## Current Gap

- Rust factory/proxy detection is narrower than Magento XML/PHP scanners.
- Scanner-level class-existence and skip validation rules are incomplete.

## Implementation Steps

1. Port XML scanner trigger coverage: preferences, virtual types, argument/item object references.
2. Port PHP scanner trigger coverage for missing factories and extension-attribute related classes.
3. Implement should-generate validations equivalent to Magento scanner behavior.

## Test Plan

1. Add scanner fixture tests for XML-triggered proxy/factory cases.
2. Add PHP scanner fixtures for extension-attribute interface patterns.
3. Compare candidate counts and generated outputs against archived baseline.

## Acceptance Criteria

- Scanner candidate sets align with Magento for sampled modules
- Missing/extra generated file counts drop from baseline
