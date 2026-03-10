# 32: Runtime Map Generator Coverage

- Category: Correctness
- Status: In Progress
- Implementation Phase: 08-parity-closure
- Owner: Unassigned
- Feature ID: `runtime-map-generator-coverage`
- Suggested Dependencies: 28-scanner-parity-xml-php, 29-extension-attributes-generation

## Intent

Align Rust-supported generated entity types with Magento runtime generator map required by this codebase.

## Current Gap

- Rust currently focuses on interceptor/factory/proxy and leaves additional runtime-map entities unimplemented or stubbed.

## Implementation Steps

1. Enumerate required entity types from Magento runtime map for this install.
2. Implement required generators or explicitly gate unsupported ones behind documented compatibility flags.
3. Eliminate remaining missing files tied to unimplemented generator types.

## Test Plan

1. Map missing files to runtime-map entity categories.
2. Add per-entity generator tests for representative outputs.
3. Run full diff harness and confirm closure of mapped gaps.

## Acceptance Criteria

- Missing files caused by runtime-map generator coverage trend to zero
- Remaining unsupported generator types are explicit, tested, and documented
