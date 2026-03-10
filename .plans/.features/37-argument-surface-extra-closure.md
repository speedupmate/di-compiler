# 37: Argument Surface Extra Closure

- Category: Correctness
- Status: Planned
- Implementation Phase: 08-parity-closure
- Owner: Unassigned
- Feature ID: `argument-surface-extra-closure`
- Suggested Dependencies: 36-di-merge-order-and-null-surface

## Intent

Preserve a focused follow-up plan to close the remaining `arguments extra=40`
surface drift after `missing=0` and `mismatches=0` were achieved.

## Current Decision

This feature is intentionally deferred for now. Current parity milestone treats
the `extra=40` set as acceptable residual while prioritizing missing/mismatch
closure and runtime correctness.

## Current Gap

- all areas: `arguments missing=0`, `arguments mismatches=0`
- all areas: `arguments extra=40`

The same 40 keys repeat across all areas, indicating shared type-universe
inclusion behavior rather than area-specific merge issues.

## Implementation Steps

1. Split extras into NULL-only and non-NULL payload groups.
2. Trace each extra key back to `build_argument_type_names` and area post-filters.
3. Constrain argument key-surface to match Magento while preserving zero missing/mismatch.
4. Add regression tests to keep key-surface stable.

## Test Plan

1. Run `compare_metadata_parity.php --max-samples=100` before/after.
2. Ensure `missing` and `mismatches` remain zero.
3. Track archive compare impact (`metadata_changed` trend).

## Acceptance Criteria

- Argument extras reduce materially from 40 (target zero unless explicitly documented).
- No regression in missing/mismatch counts.
- Deferred status is removed only when this work is actively scheduled.
