# 26: Interceptor Namespace and Structure Parity

- Category: Correctness
- Status: Planned
- Implementation Phase: 08-parity-closure
- Owner: Unassigned
- Feature ID: `interceptor-namespace-structure-parity`
- Suggested Dependencies: 14-interceptor-codegen, 20-validator-harness

## Intent

Make generated interceptor class namespace/path/class structure exactly match Magento output.

## Current Gap

- Interceptor namespace generation currently drops the target class segment while file paths include it.

## Implementation Steps

1. Fix namespace derivation for interceptor generation.
2. Validate path-to-FQCN consistency for all generated interceptors.
3. Re-verify constructor/docblock/class declaration parity against Magento output.

## Test Plan

1. Add golden-file test for nested target class interceptor namespace.
2. Compare representative generated interceptors vs archived `_code`.
3. Run full validator diff.

## Acceptance Criteria

- No namespace/path mismatch for interceptor files
- Interceptor files loadable by expected FQCN mapping
- Interceptor diff count decreases from baseline
