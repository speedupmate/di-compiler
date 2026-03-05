# 27: Interceptor Method and Signature Parity

- Category: Correctness
- Status: Planned
- Implementation Phase: 08-parity-closure
- Owner: Unassigned
- Feature ID: `interceptor-method-signature-parity`
- Suggested Dependencies: 14-interceptor-codegen, 10-interceptor-detection

## Intent

Align interceptor method selection and signature rendering with Magento compile-time interception behavior.

## Current Gap

- Rust currently wraps broad public method sets and loses nullable method-parameter type hints.
- Magento compile-time interception limits wrappers to intercepted methods and specific method filters.

## Implementation Steps

1. Mirror Magento intercepted-method filtering (`__sleep`, `__wakeup`, `__clone`, `_resetState`, static/final/etc).
2. Preserve nullable/union/ref signatures in extracted method params and generated wrappers.
3. Align method-set source with plugin-method interception rules used during compile.

## Test Plan

1. Add extractor tests for nullable and union method params.
2. Add interceptor codegen tests for filtered methods and signature parity.
3. Validate against archived `_code` interceptors for representative modules.

## Acceptance Criteria

- Interceptor wrapper method list matches Magento output for sampled classes
- Nullable signatures preserved where expected
- Interceptor content diffs decrease materially
