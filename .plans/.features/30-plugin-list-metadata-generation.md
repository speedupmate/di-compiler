# 30: Plugin List Metadata Generation

- Category: Correctness
- Status: Planned
- Implementation Phase: 08-parity-closure
- Owner: Unassigned
- Feature ID: `plugin-list-metadata-generation`
- Suggested Dependencies: 18-area-config-codegen, 19-metadata-php-serializer

## Intent

Generate compiled plugin-list metadata files per scope to match Magento setup compile outputs.

## Current Gap

- Rust currently emits only `{area}.php` and `interception.php` metadata files.
- Missing `primary|global|...|plugin-list.php` files are blocking parity.

## Implementation Steps

1. Implement plugin list inheritance/merge processing equivalent to Magento compile pipeline.
2. Write scope-specific plugin-list metadata files under metadata output.
3. Ensure deterministic ordering and stable cache-id naming.

## Test Plan

1. Validate expected plugin-list file set exists.
2. Compare selected plugin chains and processed maps with archived metadata.
3. Run full metadata diff.

## Acceptance Criteria

- All expected plugin-list metadata files are emitted
- Plugin-list output format and content match archived baseline
